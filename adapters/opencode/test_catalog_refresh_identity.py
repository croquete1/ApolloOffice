"""An incomplete model identity must not disappear from a catalog refresh."""
import contextlib
import io
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import refresh_catalog


class CatalogRefreshIdentityTests(unittest.TestCase):
    def metadata(self, **overrides):
        row = {'id': 'free', 'providerID': 'opencode', 'status': 'active',
               'cost': {'input': 0, 'output': 0}}
        row.update(overrides)
        return f"{row['providerID']}/{row['id']}\n" + json.dumps(row) + '\n'

    def test_invalid_identity_rejects_entire_inventory(self):
        for overrides in ({'id': None}, {'providerID': None}, {'id': ''},
                          {'providerID': ''}, {'id': ' '}, {'providerID': ' '}):
            with self.subTest(overrides=overrides), self.assertRaises(ValueError):
                refresh_catalog.parse_models(self.metadata() + self.metadata(**overrides))

    def test_invalid_identity_preserves_previous_catalog(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'catalog.json'
            original = b'{"previous":"catalog"}'
            path.write_bytes(original)
            raw = self.metadata() + self.metadata(id=None, cost={'input': 1, 'output': 1})
            with patch.object(refresh_catalog, 'CATALOG', path), \
                 patch.object(refresh_catalog.subprocess, 'run', return_value=SimpleNamespace(stdout=raw)), \
                 contextlib.redirect_stdout(io.StringIO()):
                with self.assertRaises(ValueError):
                    refresh_catalog.main()
            self.assertEqual(path.read_bytes(), original)
            self.assertEqual(list(Path(directory).iterdir()), [path])

    def test_complete_rows_preserved(self):
        rows = refresh_catalog.parse_models(self.metadata() + self.metadata(id='paid', cost={'input': 1, 'output': 1}))
        self.assertEqual([row['full_id'] for row in rows], ['opencode/free', 'opencode/paid'])
        self.assertEqual(rows[1]['cost']['input'], 1)


if __name__ == '__main__':
    unittest.main()
