"""Exercise inventory completeness with real Git trees, without network access."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import preflight


class InventoryLimitTests(unittest.TestCase):
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

    def git(self, *args, data=None):
        return subprocess.check_output(['git', *args], input=data).decode().strip()

    def scan(self):
        self.git('commit', '-qm', 'root')
        with patch.object(preflight.shutil, 'which', return_value=None), \
             patch.object(preflight, 'open_pinned') as remote:
            facts = preflight.scan(self.git('rev-parse', 'HEAD'), '')
        remote.assert_not_called()
        return facts

    def test_selected_symlink_marks_inventory_incomplete(self):
        oid = self.git('hash-object', '-w', '--stdin', data=b'other.txt')
        self.git('update-index', '--add', '--cacheinfo', f'120000,{oid},README.md')
        facts = self.scan()
        self.assertEqual(facts['files'], [])
        self.assertTrue(facts['inventory_truncated'])

    def test_extreme_sdk_value_does_not_abort_inspection(self):
        Path('build.gradle').write_text('minSdkVersion 0\nminSdkVersion 2147483648\nminSdkVersion 21\n')
        Path('Example.java').write_text('minSdkVersion ' + '9' * 4500 + '\n')
        self.git('add', 'build.gradle', 'Example.java')
        self.git('commit', '-qm', 'root')
        with patch.object(preflight.shutil, 'which', return_value=None), \
             patch.object(preflight, 'open_pinned') as remote:
            facts = preflight.scan(self.git('rev-parse', 'HEAD'), 'Example.java')
        remote.assert_not_called()
        self.assertTrue(facts['inventory_truncated'])
        self.assertEqual(facts['min_sdk'], [
            {'value': 21, 'path': 'build.gradle', 'revision': facts['base']},
        ])

    def test_valid_sdk_values_are_retained(self):
        Path('build.gradle').write_text('minSdkVersion 21\nminSdk = 35\n')
        self.git('add', 'build.gradle')
        facts = self.scan()
        self.assertFalse(facts['inventory_truncated'])
        self.assertEqual([item['value'] for item in facts['min_sdk']], [21, 35])

    def test_untrusted_line_hint_preserves_bounded_evidence(self):
        raw = ''.join(f'line {index:04d} ' + 'x' * 100 + '\n' for index in range(1, 401))
        Path('Example.java').write_text(raw)
        self.git('add', 'Example.java')
        self.git('commit', '-qm', 'root')
        base = self.git('rev-parse', 'HEAD')
        with patch.object(preflight.shutil, 'which', return_value=None), \
             patch.object(preflight, 'open_pinned') as remote:
            for hint in ('9' * 4500, '9999999999', '401', '0'):
                with self.subTest(hint=hint[:20]):
                    facts = preflight.scan(base, 'Example.java:' + hint)
                    self.assertTrue(facts['inventory_truncated'])
                    self.assertEqual(facts['files'][0]['content'], raw[:20000])
            facts = preflight.scan(base, 'Example.java:200')
            self.assertTrue(facts['inventory_truncated'])
            self.assertEqual(facts['files'][0]['content'], ''.join(raw.splitlines(keepends=True)[135:])[:20000])
        remote.assert_not_called()

    def test_replace_ref_cannot_substitute_requested_commit(self):
        Path('build.gradle').write_text('minSdkVersion 21\n')
        self.git('add', 'build.gradle')
        self.git('commit', '-qm', 'original')
        original = self.git('rev-parse', 'HEAD')
        Path('build.gradle').write_text('minSdkVersion 35\n')
        self.git('add', 'build.gradle')
        self.git('commit', '-qm', 'replacement')
        replacement = self.git('rev-parse', 'HEAD')
        self.git('replace', original, replacement)
        # Positive control: normal Git reads the substituted tree.
        self.assertEqual(self.git('show', original + ':build.gradle'), 'minSdkVersion 35')
        with patch.object(preflight.shutil, 'which', return_value=None), \
             patch.object(preflight, 'open_pinned') as remote:
            facts = preflight.scan(original, '')
        remote.assert_not_called()
        self.assertEqual(facts['base'], original)
        self.assertEqual(facts['min_sdk'], [{'value': 21, 'path': 'build.gradle', 'revision': original}])
        self.assertEqual(facts['files'][0]['content'], 'minSdkVersion 21\n')

    def test_test_path_limit_reports_omitted_entries(self):
        directory = Path('tests')
        directory.mkdir()
        for index in range(63):
            (directory / f'test_{index:03}.py').write_text('pass\n')
        self.git('add', 'tests')
        facts = self.scan()
        self.assertEqual(facts['test_paths'], [f'tests/test_{i:03}.py' for i in range(60)])
        self.assertTrue(facts['inventory_truncated'])
        self.assertEqual(facts['test_paths_omitted'], 3)

    def test_exact_test_path_limit_is_complete(self):
        directory = Path('tests')
        directory.mkdir()
        for index in range(60):
            (directory / f'test_{index:03}.py').write_text('pass\n')
        self.git('add', 'tests')
        facts = self.scan()
        self.assertEqual(len(facts['test_paths']), 60)
        self.assertFalse(facts['inventory_truncated'])
        self.assertEqual(facts['test_paths_omitted'], 0)

    def workflows(self, count):
        directory = Path('.github/workflows')
        directory.mkdir(parents=True)
        for index in range(count):
            (directory / f'ci-{index}.yml').write_bytes(b'name: CI\n')
        self.git('add', '.github')
        return self.scan()

    def test_workflow_limit_marks_inventory_incomplete(self):
        facts = self.workflows(5)
        self.assertEqual([f['path'] for f in facts['files']],
                         [f'.github/workflows/ci-{i}.yml' for i in range(4)])
        self.assertTrue(facts['inventory_truncated'])

    def test_exact_workflow_limit_is_complete(self):
        facts = self.workflows(4)
        self.assertEqual(len(facts['files']), 4)
        self.assertFalse(facts['inventory_truncated'])


if __name__ == '__main__':
    unittest.main()
