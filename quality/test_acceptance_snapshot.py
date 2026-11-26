"""Regression tests for the independent acceptance snapshot boundary."""
import base64
import importlib.util
import json
import os
import pathlib
import subprocess
import tempfile
import unittest


HERE = pathlib.Path(__file__).resolve().parent


class AcceptanceSnapshotTests(unittest.TestCase):
    def setUp(self):
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

    @unittest.skipUnless(hasattr(os, 'mkfifo'), 'FIFO support required')
    def test_fifo_creation_is_reported_without_reading_it(self):
        self.assertFalse(self.run_script('os.mkfifo(root / "unexpected-fifo")')['source_clean'])


if __name__ == '__main__':
    unittest.main()
