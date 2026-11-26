"""Exercise exact commit identity using real Git objects and verifier runs."""
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


class AcceptanceRevisionTests(unittest.TestCase):
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
        spec = importlib.util.spec_from_file_location('run_acceptance', HERE / 'run_acceptance.py')
        self.runner = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.runner)
        marker_dir = tempfile.TemporaryDirectory()
        self.addCleanup(marker_dir.cleanup)
        self.marker = pathlib.Path(marker_dir.name) / 'verifier-ran'
        script = ('import os, pathlib\n'
                  f'pathlib.Path({str(self.marker)!r}).write_text("ran")\n'
                  'root = pathlib.Path(os.environ["APOLLO_SOURCE_ROOT"])\n'
                  'assert (root / "fixture.txt").read_text() == "fixture"\n'
                  'print("APOLLO_CHECK:revision:PASS")\n')
        self.encoded = base64.b64encode(json.dumps({'script': script}).encode()).decode()

    def git(self, *args):
        return subprocess.check_output(['git', *args], text=True).strip()

    def init_repo(self, object_format):
        self.git('init', '-q', '--object-format=' + object_format)
        self.git('config', 'user.name', 'Fixture')
        self.git('config', 'user.email', 'fixture@localhost')
        pathlib.Path('fixture.txt').write_text('fixture')
        self.git('add', '.')
        self.git('commit', '-qm', 'fixture')
        return self.git('rev-parse', 'HEAD')

    def assert_exact_commit(self, revision):
        original_head = self.git('rev-parse', 'HEAD')
        original_content = pathlib.Path('fixture.txt').read_bytes()
        result = self.runner.run(revision, self.encoded)
        self.assertEqual(result['revision'], revision)
        self.assertEqual(result['exit'], 0, result['output'])
        self.assertIn('APOLLO_CHECK:revision:PASS', result['output'])
        self.assertTrue(result['source_clean'])
        self.assertEqual(self.marker.read_text(), 'ran')
        self.assertEqual(self.git('rev-parse', 'HEAD'), original_head)
        self.assertEqual(pathlib.Path('fixture.txt').read_bytes(), original_content)
        self.assertEqual(self.git('status', '--porcelain'), '')

    def test_sha1_commit(self):
        self.assert_exact_commit(self.init_repo('sha1'))

    def test_sha256_commit(self):
        self.assert_exact_commit(self.init_repo('sha256'))

    def test_earlier_commit_is_selected_without_changing_original_head(self):
        revision = self.init_repo('sha1')
        pathlib.Path('fixture.txt').write_text('newer content')
        self.git('add', 'fixture.txt')
        self.git('commit', '-qm', 'newer fixture')
        self.assertNotEqual(self.git('rev-parse', 'HEAD'), revision)
        self.assert_exact_commit(revision)

    def test_sha256_prefix_is_not_exact_commit(self):
        revision = self.init_repo('sha256')
        self.assertEqual(len(revision), 64)
        with self.assertRaisesRegex(ValueError, 'revision mismatch'):
            self.runner.run(revision[:40], self.encoded)
        self.assertFalse(self.marker.exists())
        self.assert_exact_commit(revision)

    def test_annotated_tag_object_is_not_exact_commit(self):
        revision = self.init_repo('sha1')
        self.git('tag', '-a', 'fixture-tag', '-m', 'fixture tag')
        tag = self.git('rev-parse', 'refs/tags/fixture-tag')
        self.assertNotEqual(tag, revision)
        self.assertEqual(self.git('cat-file', '-t', tag), 'tag')
        with self.assertRaisesRegex(ValueError, 'revision mismatch'):
            self.runner.run(tag, self.encoded)
        self.assertFalse(self.marker.exists())
        self.assert_exact_commit(revision)


if __name__ == '__main__':
    unittest.main()
