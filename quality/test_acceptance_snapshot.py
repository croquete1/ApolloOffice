"""Regression tests for the independent acceptance snapshot boundary."""
import base64
import importlib.util
import json
import os
import pathlib
import subprocess
import tempfile
import unittest
from unittest.mock import patch


HERE = pathlib.Path(__file__).resolve().parent


class AcceptanceSnapshotTests(unittest.TestCase):
    def setUp(self):
        # Never let a caller's Git context redirect temporary fixture operations.
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
        pathlib.Path('tracked').mkdir()
        pathlib.Path('tracked/file.py').write_text('pass\n')
        self.git('add', '.')
        self.git('commit', '-qm', 'fixture')
        self.base = self.git('rev-parse', 'HEAD')
        spec = importlib.util.spec_from_file_location('run_acceptance', HERE / 'run_acceptance.py')
        self.runner = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.runner)

    def git(self, *args):
        return subprocess.check_output(['git', *args], text=True).strip()

    def run_script(self, action):
        script = ('import os, pathlib\n'
                  'root = pathlib.Path(os.environ["APOLLO_SOURCE_ROOT"])\n'
                  + action + '\nprint("APOLLO_CHECK:fixture:PASS")\n')
        encoded = base64.b64encode(json.dumps({'script': script}).encode()).decode()
        result = self.runner.run(self.base, encoded)
        self.assertEqual(result['exit'], 0, result['output'])
        self.assertIn('APOLLO_CHECK:fixture:PASS', result['output'])
        self.assertEqual(self.git('status', '--porcelain'), '')
        self.assertEqual(self.git('rev-parse', 'HEAD'), self.base)
        return result

    def test_read_only_verifier_keeps_snapshot_clean(self):
        self.assertTrue(self.run_script('assert (root / "tracked/file.py").is_file()')['source_clean'])

    def test_empty_directory_creation_is_reported(self):
        self.assertFalse(self.run_script('(root / "unexpected").mkdir()')['source_clean'])

    def test_directory_mode_change_is_reported(self):
        self.assertFalse(self.run_script('(root / "tracked").chmod(0o700 if (root / "tracked").stat().st_mode & 0o777 != 0o700 else 0o755)')['source_clean'])

    def test_root_mode_change_is_reported(self):
        self.assertFalse(self.run_script('root.chmod(0o700 if root.stat().st_mode & 0o777 != 0o700 else 0o755)')['source_clean'])

    def test_fixture_isolates_inherited_git_environment(self):
        parent = pathlib.Path.cwd()
        config = (parent / '.git/config').read_bytes()
        index = (parent / '.git/index').read_bytes()
        poisoned = {'GIT_DIR': str(parent / '.git'), 'GIT_WORK_TREE': str(parent),
                    'GIT_INDEX_FILE': str(parent / '.git/index'), 'GIT_CONFIG_COUNT': '1',
                    'GIT_CONFIG_KEY_0': 'core.bare', 'GIT_CONFIG_VALUE_0': 'true'}
        nested = type(self)('test_read_only_verifier_keeps_snapshot_clean')
        result = unittest.TestResult()
        with patch.dict(os.environ, poisoned):
            nested.run(result)
        self.assertEqual(result.testsRun, 1)
        self.assertTrue(result.wasSuccessful(), repr(result.errors + result.failures))
        self.assertEqual(pathlib.Path.cwd(), parent)
        self.assertEqual(self.git('rev-parse', 'HEAD'), self.base)
        self.assertEqual(self.git('status', '--porcelain'), '')
        self.assertEqual((parent / '.git/config').read_bytes(), config)
        self.assertEqual((parent / '.git/index').read_bytes(), index)

    @unittest.skipUnless(hasattr(os, 'mkfifo'), 'FIFO support required')
    def test_fifo_creation_is_reported_without_reading_it(self):
        self.assertFalse(self.run_script('os.mkfifo(root / "unexpected-fifo")')['source_clean'])


if __name__ == '__main__':
    unittest.main()
