"""Keep raw pricing ambiguity from being normalized during catalog refresh."""
import contextlib
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from types import SimpleNamespace

import refresh_catalog
from opencode_bridge import allowed_models


class CatalogRefreshTests(unittest.TestCase):
    def metadata(self, cost, ident='free'):
        return ('opencode/' + ident + '\n{"id":"' + ident + '",'
                '"providerID":"opencode","status":"active","cost":' + cost + '}\n')

    def test_duplicate_price_keys_rejected_before_serialization(self):
        for cost in ('{"input":1,"input":0,"output":0}',
                     '{"input":0,"output":0,"cache":{"read":1,"read":0}}'):
            with self.subTest(cost=cost), self.assertRaises(ValueError):
                refresh_catalog.parse_models(self.metadata(cost))

    def test_underflow_rejected_before_serialization(self):
        for value in ('1e-9999', '-1e-9999'):
            with self.subTest(value=value), self.assertRaises(ValueError):
                refresh_catalog.parse_models(self.metadata('{"input":' + value + ',"output":0}'))

    def test_invalid_refresh_preserves_existing_catalog(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'catalog.json'
            original = b'{"previous":"catalog"}'
            path.write_bytes(original)
            metadata = self.metadata('{"input":1e-9999,"output":0}')
            with patch.object(refresh_catalog, 'CATALOG', path), \
                 patch.object(refresh_catalog.subprocess, 'run', return_value=SimpleNamespace(stdout=metadata)), \
                 contextlib.redirect_stdout(io.StringIO()):
                with self.assertRaises(ValueError):
                    refresh_catalog.main()
            self.assertEqual(path.read_bytes(), original)
            self.assertEqual(list(Path(directory).iterdir()), [path])

    def test_valid_and_paid_rows_preserve_pricing(self):
        raw = self.metadata('{"input":0e-9999,"output":0}') + self.metadata('{"input":1e-300,"output":0}', 'paid')
        rows = refresh_catalog.parse_models(raw)
        self.assertEqual([row['full_id'] for row in rows], ['opencode/free', 'opencode/paid'])
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'catalog.json'
            path.write_text(json.dumps({'updated_at': 100, 'models': rows}), encoding='utf-8')
            self.assertEqual(allowed_models(path, now=100), {'opencode/free'})


if __name__ == '__main__':
    unittest.main()
