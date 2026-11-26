"""Regression coverage for bounded Git inventory completeness."""
import importlib.util
import os
import pathlib
import subprocess
import tempfile
import unittest
from unittest.mock import patch


HERE = pathlib.Path(__file__).resolve().parent


class PreflightInventoryTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        env = {key: value for key, value in os.environ.items() if not key.startswith('GIT_')}
        env.update(GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL=os.devnull,
                   GIT_TERMINAL_PROMPT='0')
        isolated = patch.dict(os.environ, env, clear=True)
        isolated.start()
        self.addCleanup(isolated.stop)
        previous = pathlib.Path.cwd()
        os.chdir(temp.name)
        self.addCleanup(os.chdir, previous)
        self.git('init', '-q')
        self.git('config', 'user.name', 'Fixture')
        self.git('config', 'user.email', 'fixture@localhost')
        spec = importlib.util.spec_from_file_location('preflight', HERE / 'preflight.py')
        self.preflight = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.preflight)

    def git(self, *args):
        return subprocess.check_output(['git', *args], text=True).strip()

    def scan(self):
        self.git('add', '.')
        self.git('commit', '-qm', 'fixture')
        base = self.git('rev-parse', 'HEAD')
        with patch.object(self.preflight.shutil, 'which', return_value=None):
            return self.preflight.scan(base, '')

    def test_complete_inventory(self):
        pathlib.Path('build.gradle').write_bytes(b'minSdkVersion 17\n')
        facts = self.scan()
        self.assertFalse(facts['inventory_truncated'])
        self.assertEqual([f['value'] for f in facts['min_sdk']], [17])

    def test_oversized_manifest_marks_inventory_incomplete(self):
        pathlib.Path('build.gradle').write_bytes(b' ' * 200001 + b'minSdkVersion 17\n')
        facts = self.scan()
        self.assertTrue(facts['inventory_truncated'])
        self.assertEqual(facts['files'], [])

    def test_partial_manifest_marks_inventory_incomplete(self):
        pathlib.Path('build.gradle').write_bytes(b' ' * 4000 + b'minSdkVersion 17\n')
        facts = self.scan()
        self.assertTrue(facts['files'][0]['truncated'])
        self.assertTrue(facts['inventory_truncated'])

    def test_multibyte_boundary_respects_content_budget(self):
        pathlib.Path('build.gradle').write_bytes(b' ' * 3999 + 'é'.encode('utf-8'))
        facts = self.scan()
        self.assertLessEqual(len(facts['files'][0]['content'].encode('utf-8')), 4000)
        self.assertTrue(facts['files'][0]['truncated'])
        self.assertTrue(facts['inventory_truncated'])


if __name__ == '__main__':
    unittest.main()
