"""Reject malformed free-model catalogs with a controlled policy error."""
import io
import json
from pathlib import Path
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import Mock

from opencode_bridge import Bridge, Handler, allowed_models


class CatalogStructureTests(unittest.TestCase):
    def allowed(self, catalog):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'catalog.json'
            path.write_text(json.dumps(catalog))
            return allowed_models(path, now=100.0)

    def row(self, ident='opencode/free'):
        return {'full_id': ident, 'providerID': 'opencode', 'status': 'active',
                'cost': {'input': 0, 'output': 0}}

    def test_valid_catalog_remains_available(self):
        self.assertEqual(self.allowed({'updated_at': 100, 'models': [self.row()]}),
                         {'opencode/free'})

    def test_non_object_catalog_rejected(self):
        for catalog in (None, [], 'invalid'):
            with self.subTest(catalog=catalog), self.assertRaises(ValueError):
                self.allowed(catalog)

    def test_invalid_timestamp_rejected(self):
        for timestamp in (True, None, '100'):
            with self.subTest(timestamp=timestamp), self.assertRaises(ValueError):
                self.allowed({'updated_at': timestamp, 'models': [self.row()]})

    def test_out_of_range_timestamps_rejected(self):
        for timestamp in (10**400, -10**400, float('nan'), float('inf'),
                          float('-inf'), 100.01, -86300.01):
            with self.subTest(timestamp=timestamp), self.assertRaises(ValueError):
                self.allowed({'updated_at': timestamp, 'models': [self.row()]})

    def test_freshness_boundaries_are_inclusive(self):
        for timestamp in (100, -86300):
            with self.subTest(timestamp=timestamp):
                self.assertEqual(self.allowed({'updated_at': timestamp, 'models': [self.row()]}),
                                 {'opencode/free'})

    def test_non_list_inventory_rejected(self):
        for rows in (None, {}, 'invalid'):
            with self.subTest(rows=rows), self.assertRaises(ValueError):
                self.allowed({'updated_at': 100, 'models': rows})

    def test_malformed_row_rejects_entire_catalog(self):
        for row in (None, 1, 'invalid', [], {}, self.row([]), self.row('')):
            with self.subTest(row=row), self.assertRaises(ValueError):
                self.allowed({'updated_at': 100, 'models': [self.row(), row]})

    def test_wrong_provider_prefix_does_not_authorize_model(self):
        for ident in ('other/free', 'opencode/', 'free'):
            with self.subTest(ident=ident):
                self.assertEqual(self.allowed({'updated_at': 100, 'models': [self.row(ident)]}), set())


class CatalogAvailabilityTests(unittest.TestCase):
    def test_unreadable_catalog_returns_service_unavailable(self):
        with tempfile.TemporaryDirectory() as directory:
            for path in (Path(directory) / 'missing.json', Path(directory)):
                with self.subTest(path=path):
                    factory = Mock(side_effect=AssertionError('Provider must not start'))
                    bridge = Bridge(factory=factory, catalog=path)
                    handler = Handler.__new__(Handler)
                    handler.server = SimpleNamespace(bridge=bridge)
                    handler.path = '/chat/completions'
                    payload = json.dumps({'model': 'opencode/free',
                                          'messages': [{'role': 'user', 'content': 'test'}]}).encode()
                    handler.headers = {'Content-Length': str(len(payload))}
                    handler.rfile = io.BytesIO(payload)
                    handler.send_json = Mock()
                    handler.do_POST()
                    handler.send_json.assert_called_once_with(
                        503, {'error': {'message': 'Free-model catalog unavailable'}}, None)
                    factory.assert_not_called()
                    self.assertIsNone(bridge.session)
                    self.assertTrue(bridge.busy.acquire(blocking=False))
                    bridge.busy.release()


if __name__ == '__main__':
    unittest.main()
