"""Verify that repository inspection is bound to an actual full commit ID."""
import importlib.util
import os
import pathlib
import subprocess
import tempfile
import unittest
from unittest.mock import patch


HERE = pathlib.Path(__file__).resolve().parent


class PreflightRevisionTests(unittest.TestCase):
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
        spec = importlib.util.spec_from_file_location('preflight', HERE / 'preflight.py')
        self.preflight = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.preflight)

    def git(self, *args):
        return subprocess.check_output(['git', *args], text=True).strip()

    def init_repo(self, object_format):
        self.git('init', '-q', '--object-format=' + object_format)
        self.git('config', 'user.name', 'Fixture')
        self.git('config', 'user.email', 'fixture@localhost')
        pathlib.Path('build.gradle').write_text('minSdkVersion 17\n')
        self.git('add', '.')
        self.git('commit', '-qm', 'fixture')
        return self.git('rev-parse', 'HEAD')

    def scan(self, revision):
        with patch.object(self.preflight.shutil, 'which', return_value=None):
            return self.preflight.scan(revision, '')

    def assert_exact_commit(self, revision):
        facts = self.scan(revision)
        self.assertEqual(facts['base'], revision)
        self.assertEqual(facts['min_sdk'], [{'value': 17, 'path': 'build.gradle', 'revision': revision}])
        self.assertFalse(facts['inventory_truncated'])
        self.assertEqual(self.git('rev-parse', 'HEAD'), revision)
        self.assertEqual(self.git('status', '--porcelain'), '')

    def test_sha1_commit(self):
        self.assert_exact_commit(self.init_repo('sha1'))

    def test_sha256_commit(self):
        self.assert_exact_commit(self.init_repo('sha256'))

    def test_sha256_prefix_is_not_exact_commit(self):
        revision = self.init_repo('sha256')
        with self.assertRaisesRegex(ValueError, 'exact base commit required'):
            self.scan(revision[:40])
        self.assert_exact_commit(revision)

    def test_annotated_tag_object_is_not_exact_commit(self):
        revision = self.init_repo('sha1')
        self.git('tag', '-a', 'fixture-tag', '-m', 'fixture tag')
        tag = self.git('rev-parse', 'refs/tags/fixture-tag')
        self.assertNotEqual(tag, revision)
        self.assertEqual(self.git('cat-file', '-t', tag), 'tag')
        with self.assertRaisesRegex(ValueError, 'exact base commit required'):
            self.scan(tag)
        self.assert_exact_commit(revision)


if __name__ == '__main__':
    unittest.main()
