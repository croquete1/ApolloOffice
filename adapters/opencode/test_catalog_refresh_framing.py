"""Require complete, matching verbose model records before catalog publication."""
import contextlib
import io
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import refresh_catalog


class CatalogRefreshFramingTests(unittest.TestCase):
    def record(self, ident='free', header=None):
        row = {'id': ident, 'providerID': 'opencode', 'status': 'active',
               'cost': {'input': 0, 'output': 0}}
        return (header if header is not None else 'opencode/' + ident) + '\n' + json.dumps(row, indent=2) + '\n'

    def test_mismatched_header_rejected(self):
        with self.assertRaises(ValueError):
            refresh_catalog.parse_models(self.record(header='opencode/paid'))

    def test_incomplete_inventory_rejected(self):
        for tail in ('opencode/paid\n', 'unexpected output\n', 'opencode/paid\n[]\n'):
            with self.subTest(tail=tail), self.assertRaises(ValueError):
                refresh_catalog.parse_models(self.record() + tail)

    def test_invalid_framing_preserves_previous_catalog(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'catalog.json'
            original = b'{"previous":"catalog"}'
            path.write_bytes(original)
            raw = self.record() + 'opencode/paid\n'
            with patch.object(refresh_catalog, 'CATALOG', path), \
                 patch.object(refresh_catalog.subprocess, 'run', return_value=SimpleNamespace(stdout=raw)), \
                 contextlib.redirect_stdout(io.StringIO()):
                with self.assertRaises(ValueError):
                    refresh_catalog.main()
            self.assertEqual(path.read_bytes(), original)
            self.assertEqual(list(Path(directory).iterdir()), [path])

    def test_multiline_records_with_crlf_and_blank_lines(self):
        raw = '\n' + self.record() + '\n' + self.record('second') + '\n'
        rows = refresh_catalog.parse_models(raw.replace('\n', '\r\n'))
        self.assertEqual([row['full_id'] for row in rows], ['opencode/free', 'opencode/second'])


if __name__ == '__main__':
    unittest.main()
