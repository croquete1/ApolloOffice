"""Exercise the shared inspection budget with real gitlinks and bounded HTTP fixtures."""
import hashlib
import importlib.util
import io
import os
import pathlib
import subprocess
import tempfile
import unittest
from unittest.mock import patch


HERE = pathlib.Path(__file__).resolve().parent


class DependencyBudgetTests(unittest.TestCase):
    def setUp(self):
        env = {key: value for key, value in os.environ.items() if not key.startswith('GIT_')}
        env.update(GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL=os.devnull,
                   GIT_TERMINAL_PROMPT='0')
        isolated = patch.dict(os.environ, env, clear=True)
        isolated.start()
        self.addCleanup(isolated.stop)
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        previous = pathlib.Path.cwd()
        os.chdir(temp.name)
        self.addCleanup(os.chdir, previous)
        self.git('init', '-q')
        self.git('config', 'user.name', 'Fixture')
        self.git('config', 'user.email', 'fixture@localhost')
        self.git('commit', '--allow-empty', '-qm', 'dependency')
        self.revision = self.git('rev-parse', 'HEAD')
        spec = importlib.util.spec_from_file_location('preflight', HERE / 'preflight.py')
        self.preflight = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.preflight)

    def git(self, *args):
        return subprocess.check_output(['git', *args], text=True).strip()

    def modules(self, *names):
        config = ''.join(f'[submodule "{name}"]\n\tpath = deps/{name}\n\turl = https://github.com/example/{name}.git\n' for name in names)
        pathlib.Path('.gitmodules').write_text(config)
        for name in names:
            self.git('update-index', '--add', '--cacheinfo', f'160000,{self.revision},deps/{name}')

    def manifest(self, index, size):
        path = pathlib.Path(f'local-{index:02d}/README.md')
        path.parent.mkdir()
        path.write_bytes(b' ' * size)

    def fill_to(self, target):
        remaining = target - pathlib.Path('.gitmodules').stat().st_size
        for index in range((remaining + 3999) // 4000):
            size = min(4000, remaining)
            self.manifest(index, size)
            remaining -= size
        self.assertEqual(remaining, 0)

    def scan(self, *responses):
        self.git('add', '.gitmodules')
        for directory in pathlib.Path('.').glob('local-*'):
            self.git('add', str(directory))
        self.git('commit', '-qm', 'root')
        base = self.git('rev-parse', 'HEAD')
        with patch.object(self.preflight.shutil, 'which', return_value=None), \
             patch.object(self.preflight.urllib.request, 'urlopen',
                          side_effect=[io.BytesIO(data) for data in responses]) as remote:
            facts = self.preflight.scan(base, '')
        return facts, remote

    def assert_bounded(self, facts):
        self.assertLessEqual(len(facts['files']), 32)
        self.assertLessEqual(sum(len(f['content'].encode('utf-8')) for f in facts['files']), 56000)

    def assert_omitted(self, facts, name):
        self.assertTrue(facts['inventory_truncated'])
        self.assertTrue(any(item.get('path') == f'deps/{name}' and item.get('revision') == self.revision
                            and 'budget' in item['reason'] for item in facts['unresolved_dependencies']))
        self.assertFalse(any(f['path'] == f'deps/{name}/constants.gradle' for f in facts['files']))

    def test_file_limit_prevents_remote_request(self):
        self.modules('first')
        for index in range(31):
            self.manifest(index, 1)
        facts, remote = self.scan(b'minSdkVersion 21')
        self.assert_bounded(facts)
        self.assert_omitted(facts, 'first')
        remote.assert_not_called()

    def test_remote_blob_exceeding_remaining_bytes_is_omitted(self):
        self.modules('first')
        self.fill_to(55997)
        facts, remote = self.scan(b'minSdkVersion 21')
        self.assert_bounded(facts)
        self.assert_omitted(facts, 'first')
        remote.assert_called_once()

    def test_two_dependencies_share_one_byte_budget(self):
        self.modules('first', 'second')
        self.fill_to(50000)
        raw = b'minSdkVersion 21\n' + b' ' * 3983
        facts, remote = self.scan(raw, raw)
        self.assert_bounded(facts)
        self.assert_omitted(facts, 'second')
        self.assertEqual(remote.call_count, 2)
        first = next(f for f in facts['files'] if f['path'] == 'deps/first/constants.gradle')
        self.assertEqual(first['dependency_revision'], self.revision)
        self.assertEqual(first['sha256'], hashlib.sha256(raw).hexdigest())
        self.assertEqual(first['content'], raw.decode())

    def test_small_dependency_fits_and_retains_identity(self):
        self.modules('first')
        raw = 'minSdkVersion 21\n// café'.encode()
        facts, remote = self.scan(raw)
        self.assert_bounded(facts)
        self.assertFalse(facts['inventory_truncated'])
        self.assertEqual(facts['unresolved_dependencies'], [])
        self.assertEqual(facts['min_sdk'], [{'value': 21, 'path': 'deps/first/constants.gradle', 'revision': self.revision}])
        self.assertEqual(remote.call_args.args[0].full_url, f'https://raw.githubusercontent.com/example/first/{self.revision}/constants.gradle')


if __name__ == '__main__':
    unittest.main()
