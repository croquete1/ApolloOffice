"""Exercise catalog policy and malformed requests across a real local Unix socket."""
import http.client
import json
from pathlib import Path
import socket
import tempfile
import threading
import time
import unittest
from unittest.mock import Mock

from opencode_bridge import Bridge, Handler, Server, request_args


@unittest.skipUnless(hasattr(socket, 'AF_UNIX'), 'Unix sockets required')
class CatalogHttpTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.catalog = Path(temp.name) / 'catalog.json'
        self.address = str(Path(temp.name) / 'bridge.sock')
        self.factory = Mock(side_effect=AssertionError('Provider must not start'))
        self.bridge = Bridge(factory=self.factory, catalog=self.catalog)
        server = Server(self.address, Handler)
        self.addCleanup(server.server_close)
        server.bridge = self.bridge
        thread = threading.Thread(target=server.serve_forever, kwargs={'poll_interval': 0.01}, daemon=True)
        thread.start()
        self.addCleanup(thread.join, 5)
        self.addCleanup(server.shutdown)

    def request(self, method, path, body=b''):
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as conn:
            conn.settimeout(5)
            conn.connect(self.address)
            conn.sendall(f'{method} {path} HTTP/1.0\r\nHost: localhost\r\nContent-Length: {len(body)}\r\n\r\n'.encode() + body)
            response = http.client.HTTPResponse(conn)
            try:
                response.begin()
                self.assertEqual(response.getheader('Content-Type'), 'application/json')
                return response.status, json.loads(response.read())
            finally:
                response.close()

    def write_valid_catalog(self):
        self.catalog.write_text(json.dumps({'updated_at': time.time(), 'models': [
            {'full_id': 'opencode/free', 'providerID': 'opencode', 'status': 'active',
             'cost': {'input': 0, 'output': 0}}]}), encoding='utf-8')

    def assert_no_provider(self):
        self.factory.assert_not_called()
        self.assertIsNone(self.bridge.session)
        self.assertTrue(self.bridge.busy.acquire(blocking=False))
        self.bridge.busy.release()

    def test_catalog_unavailable_across_http(self):
        self.assertEqual(self.request('GET', '/health'), (503, {'ok': False}))
        status, result = self.request('POST', '/chat/completions', b'{}')
        self.assertEqual(status, 503)
        self.assertEqual(result['error']['message'], 'Free-model catalog unavailable')
        self.assert_no_provider()

    def test_malformed_catalog_across_http(self):
        self.catalog.write_text('{"updated_at":100,"models":[],"models":[]}', encoding='utf-8')
        self.assertEqual(self.request('GET', '/health'), (503, {'ok': False}))
        status, result = self.request('POST', '/chat/completions', b'{}')
        self.assertEqual(status, 400)
        self.assertIn('error', result)
        self.assert_no_provider()

    def test_malformed_tool_history_returns_http_error(self):
        self.write_valid_catalog()
        for calls in ([None], [1], ['bad'], {'function': {'name': 'read'}},
                      [{'function': None}], [{'function': {'name': ''}}],
                      [{'function': {'name': []}}], [{}], [{'id': ''}],
                      [{'id': []}], [{'id': 'old', 'function': None}]):
            with self.subTest(calls=calls):
                body = json.dumps({'model': 'opencode/free', 'messages': [
                    {'role': 'assistant', 'tool_calls': calls}]}).encode()
                status, result = self.request('POST', '/chat/completions', body)
                self.assertEqual(status, 400)
                self.assertIn('error', result)
                self.assert_no_provider()
        self.assertEqual(self.request('GET', '/health'), (200, {'ok': True, 'free_models': ['opencode/free']}))

    def test_valid_tool_history_is_preserved_by_request_validation(self):
        calls = [{'id': 'call-1', 'type': 'function',
                  'function': {'name': 'read', 'arguments': '{}'}}]
        for history in (calls, [{'id': 'old'}], calls + [{'id': 'old'}], [], None):
            with self.subTest(history=history):
                messages = [{'role': 'assistant', 'tool_calls': history}]
                args = request_args({'model': 'opencode/free', 'messages': messages}, {'opencode/free'})
                self.assertEqual(args['messages'], messages)

    def test_deep_request_is_rejected_and_health_remains_available(self):
        self.write_valid_catalog()
        depth = 20000
        body = b'[' * depth + b'0' + b']' * depth
        status, result = self.request('POST', '/chat/completions', body)
        self.assertEqual(status, 400)
        self.assertIn('error', result)
        self.assertEqual(self.request('GET', '/health'), (200, {'ok': True, 'free_models': ['opencode/free']}))
        self.assert_no_provider()


if __name__ == '__main__':
    unittest.main()
