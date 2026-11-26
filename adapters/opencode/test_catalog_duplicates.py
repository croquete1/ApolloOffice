"""Fail closed on ambiguous pricing metadata, without calling a provider."""
import json
from pathlib import Path
import tempfile
import unittest

from opencode_bridge import allowed_models


class CatalogDuplicateTests(unittest.TestCase):
    def allowed(self, *rows):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'catalog.json'
            path.write_text(json.dumps({'updated_at': 100, 'models': rows}))
            return allowed_models(path, now=100)

    def row(self, model, price=0, status='active'):
        return {'full_id': 'opencode/' + model, 'providerID': 'opencode',
                'status': status, 'cost': {'input': price, 'output': price}}

    def test_unique_free_model_is_allowed(self):
        self.assertEqual(self.allowed(self.row('free'), self.row('paid', 1)), {'opencode/free'})

    def test_conflicting_prices_are_rejected_in_both_orders(self):
        free, paid = self.row('ambiguous'), self.row('ambiguous', 1)
        for first, second in ((free, paid), (paid, free)):
            with self.subTest(first=first):
                self.assertEqual(self.allowed(first, second, self.row('other')), {'opencode/other'})

    def test_conflicting_status_is_rejected(self):
        self.assertEqual(self.allowed(self.row('ambiguous'), self.row('ambiguous', status='deprecated')), set())

    def test_identical_duplicate_is_rejected(self):
        free = self.row('ambiguous')
        self.assertEqual(self.allowed(free, free), set())


if __name__ == '__main__':
    unittest.main()
