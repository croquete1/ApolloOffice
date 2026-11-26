"""Use real Git trees and mocked HTTP for missing dependency evidence."""
import io
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch
import preflight


class MissingDependencyTests(unittest.TestCase):
    def setUp(self):
        env = {key: value for key, value in os.environ.items() if not key.startswith('GIT_')}
        env.update(GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL=os.devnull, GIT_TERMINAL_PROMPT='0')
        isolated = patch.dict(os.environ, env, clear=True)
        isolated.start()
        self.addCleanup(isolated.stop)
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        previous = Path.cwd()
        os.chdir(temp.name)
        self.addCleanup(os.chdir, previous)
        self.git('init', '-q')
        self.git('config', 'user.name', 'Fixture')
        self.git('config', 'user.email', 'fixture@localhost')
        self.git('commit', '--allow-empty', '-qm', 'dependency')
        self.revision = self.git('rev-parse', 'HEAD')

    def git(self, *args):
        return subprocess.check_output(['git', *args], text=True).strip()

    def link(self, path):
        self.git('update-index', '--add', '--cacheinfo', f'160000,{self.revision},{path}')

    def scan(self, config=None, response=None):
        if config is not None:
            Path('.gitmodules').write_bytes(config)
            self.git('add', '.gitmodules')
        self.git('commit', '-qm', 'root')
        with patch.object(preflight.shutil, 'which', return_value=None), \
             patch.object(preflight, 'open_pinned', return_value=response) as remote:
            facts = preflight.scan(self.git('rev-parse', 'HEAD'), '')
        return facts, remote

    def assert_unresolved(self, facts, path):
        self.assertTrue(facts['inventory_truncated'])
        self.assertTrue(any(item.get('path') == path and item.get('revision') == self.revision
                            for item in facts['unresolved_dependencies']))
        self.assertFalse(any(f['path'] == f'{path}/constants.gradle' for f in facts['files']))

    def test_gitlink_without_configuration(self):
        self.link('deps/missing')
        facts, remote = self.scan()
        self.assert_unresolved(facts, 'deps/missing')
        remote.assert_not_called()

    def test_unsupported_url(self):
        self.link('deps/first')
        facts, remote = self.scan(b'[submodule "first"]\npath = deps/first\nurl = ../first.git\n')
        self.assert_unresolved(facts, 'deps/first')
        remote.assert_not_called()

    def test_truncated_config_is_not_used_as_authority(self):
        self.link('deps/first')
        config = b'[submodule "first"]\npath = deps/first\nurl = https://github.com/example/first.git\n' + b'#' * 4100
        facts, remote = self.scan(config, io.BytesIO(b'minSdkVersion 21'))
        self.assert_unresolved(facts, 'deps/first')
        self.assertEqual(facts['min_sdk'], [])
        remote.assert_not_called()

    def test_unreadable_remote_response_marks_inventory_incomplete(self):
        self.link('deps/first')
        config = b'[submodule "first"]\npath = deps/first\nurl = https://github.com/example/first.git\n'
        facts, remote = self.scan(config, io.BytesIO(b'\xff'))
        self.assert_unresolved(facts, 'deps/first')
        remote.assert_called_once()

    def test_missing_diagnostics_are_bounded(self):
        for i in range(35):
            self.link(f'deps/{i:02d}')
        facts, remote = self.scan()
        self.assertTrue(facts['inventory_truncated'])
        self.assertEqual(len(facts['unresolved_dependencies']), 33)
        self.assertEqual(facts['unresolved_dependencies'][-1]['omitted_count'], 3)
        remote.assert_not_called()

    def test_conflicting_duplicate_paths_are_refused_before_io(self):
        self.link('deps/first')
        config = (b'[submodule "first"]\npath = deps/first\nurl = https://github.com/example/first.git\n'
                  b'[submodule "other"]\npath = deps/first\nurl = https://github.com/example/other.git\n')
        facts, remote = self.scan(config, io.BytesIO(b'minSdkVersion 21'))
        self.assert_unresolved(facts, 'deps/first')
        self.assertEqual(len(facts['unresolved_dependencies']), 1)
        self.assertIn('ambiguous', facts['unresolved_dependencies'][0]['reason'])
        self.assertEqual(facts['min_sdk'], [])
        remote.assert_not_called()

    def test_identical_duplicate_paths_are_refused_before_io(self):
        self.link('deps/first')
        config = (b'[submodule "first"]\npath = deps/first\nurl = https://github.com/example/first.git\n'
                  b'[submodule "other"]\npath = deps/first\nurl = https://github.com/example/first.git\n')
        facts, remote = self.scan(config, io.BytesIO(b'minSdkVersion 21'))
        self.assert_unresolved(facts, 'deps/first')
        self.assertEqual(len(facts['unresolved_dependencies']), 1)
        remote.assert_not_called()

    def test_inherited_default_url_is_not_dependency_evidence(self):
        self.link('deps/first')
        config = (b'[DEFAULT]\nurl = https://github.com/example/first.git\n'
                  b'[submodule "first"]\npath = deps/first\n')
        facts, remote = self.scan(config, io.BytesIO(b'minSdkVersion 21'))
        self.assert_unresolved(facts, 'deps/first')
        remote.assert_not_called()

    def test_ambiguity_does_not_discard_independent_valid_dependency(self):
        self.link('deps/first')
        self.link('deps/second')
        config = (b'[submodule "first"]\npath = deps/first\nurl = https://github.com/example/first.git\n'
                  b'[submodule "second"]\npath = deps/second\nurl = https://github.com/example/second.git\n'
                  b'[submodule "other"]\npath = deps/first\nurl = https://github.com/example/other.git\n')
        facts, remote = self.scan(config, io.BytesIO(b'minSdkVersion 22'))
        self.assert_unresolved(facts, 'deps/first')
        self.assertEqual(facts['min_sdk'], [{'value': 22, 'path': 'deps/second/constants.gradle', 'revision': self.revision}])
        remote.assert_called_once()
        self.assertIn('/example/second/', remote.call_args.args[0].full_url)

    def test_unrelated_section_is_not_submodule_evidence(self):
        self.link('deps/first')
        config = b'[unrelated]\npath = deps/first\nurl = https://github.com/example/first.git\n'
        facts, remote = self.scan(config, io.BytesIO(b'minSdkVersion 21'))
        self.assert_unresolved(facts, 'deps/first')
        self.assertEqual(facts['min_sdk'], [])
        remote.assert_not_called()

    def test_native_git_syntax_preserves_valid_dependency(self):
        self.link('deps/first')
        config = (b'[Submodule "first.with.dot"]\npath = "deps/first"\n'
                  b'url = "https://github.com/example/first.git" # comment\n')
        facts, remote = self.scan(config, io.BytesIO(b'minSdkVersion 21'))
        self.assertFalse(facts['inventory_truncated'])
        self.assertEqual(facts['min_sdk'], [{'value': 21, 'path': 'deps/first/constants.gradle', 'revision': self.revision}])
        remote.assert_called_once()

    def test_duplicate_native_keys_fail_closed(self):
        self.link('deps/first')
        config = (b'[submodule "first"]\npath = deps/first\nurl = https://github.com/example/first.git\n'
                  b'[Submodule "first"]\nurl = https://github.com/example/other.git\n')
        facts, remote = self.scan(config, io.BytesIO(b'minSdkVersion 21'))
        self.assert_unresolved(facts, 'deps/first')
        remote.assert_not_called()

    def test_invalid_git_config_syntax_is_not_evidence(self):
        self.link('deps/first')
        config = b'[submodule "first"]\npath: deps/first\nurl: https://github.com/example/first.git\n'
        facts, remote = self.scan(config, io.BytesIO(b'minSdkVersion 21'))
        self.assert_unresolved(facts, 'deps/first')
        remote.assert_not_called()

    def test_valid_dependency_retains_pinned_evidence(self):
        self.link('deps/first')
        config = b'[submodule "first"]\npath = deps/first\nurl = https://github.com/example/first.git\n'
        facts, remote = self.scan(config, io.BytesIO(b'minSdkVersion 21'))
        self.assertFalse(facts['inventory_truncated'])
        self.assertEqual(facts['unresolved_dependencies'], [])
        self.assertEqual(facts['min_sdk'], [{'value': 21, 'path': 'deps/first/constants.gradle', 'revision': self.revision}])
        remote.assert_called_once()


if __name__ == '__main__':
    unittest.main()
