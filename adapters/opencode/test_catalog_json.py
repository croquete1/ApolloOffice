"""Reject ambiguous JSON and nonzero prices rounded down to zero."""
from pathlib import Path
import tempfile
import unittest

from opencode_bridge import allowed_models


class CatalogJsonTests(unittest.TestCase):
    def allowed(self, raw):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'catalog.json'
            path.write_text(raw, encoding='utf-8')
            return allowed_models(path, now=100.0)

    def catalog(self, cost):
        return ('{"updated_at":100,"models":[{"full_id":"opencode/free",'
                '"providerID":"opencode","status":"active","cost":' + cost + '}]}')

    def test_duplicate_price_keys_are_rejected(self):
        for cost in ('{"input":1,"input":0,"output":0}',
                     '{"input":0,"output":0,"cache":{"read":1,"read":0}}',
                     '{"input":1,"\\u0069nput":0,"output":0}'):
            with self.subTest(cost=cost), self.assertRaises(ValueError):
                self.allowed(self.catalog(cost))

    def test_duplicate_inventory_key_is_rejected(self):
        raw = self.catalog('{"input":0,"output":0}').replace('"models":', '"models":[],"models":', 1)
        with self.assertRaises(ValueError):
            self.allowed(raw)

    def test_nonzero_underflow_is_rejected(self):
        for price in ('1e-9999', '-1e-9999', '0.0001e-9999'):
            with self.subTest(price=price), self.assertRaises(ValueError):
                self.allowed(self.catalog('{"input":' + price + ',"output":0}'))

    def test_explicit_zero_formats_remain_free(self):
        for price in ('0', '0.0', '-0.0', '0e-9999', '0.000e+10'):
            with self.subTest(price=price):
                self.assertEqual(self.allowed(self.catalog('{"input":' + price + ',"output":0}')),
                                 {'opencode/free'})

    def test_representable_nonzero_price_remains_excluded(self):
        self.assertEqual(self.allowed(self.catalog('{"input":1e-300,"output":0}')), set())


if __name__ == '__main__':
    unittest.main()
