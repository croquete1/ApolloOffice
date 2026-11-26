"""Remote evidence must not follow a redirect away from its pinned URL."""
from http.server import BaseHTTPRequestHandler, HTTPServer
import threading
import unittest
from urllib.error import HTTPError
from urllib.request import Request

import preflight


class PinnedTransportTests(unittest.TestCase):
    def setUp(self):
        self.visits = []
        visits = self.visits

        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                visits.append(self.path)
                if self.path == '/redirect':
                    self.send_response(302)
                    self.send_header('Location', '/unrelated')
                    self.end_headers()
                else:
                    self.send_response(200)
                    self.end_headers()
                    self.wfile.write(b'minSdkVersion 21')

            def log_message(self, *args):
                pass

        server = HTTPServer(('127.0.0.1', 0), Handler)
        self.addCleanup(server.server_close)
        thread = threading.Thread(target=server.serve_forever, kwargs={'poll_interval': 0.01}, daemon=True)
        thread.start()
        self.addCleanup(thread.join, 5)
        self.addCleanup(server.shutdown)
        self.url = f'http://127.0.0.1:{server.server_port}'

    def test_redirect_refused_without_visiting_target(self):
        with self.assertRaises(HTTPError) as error:
            with preflight.open_pinned(Request(self.url + '/redirect'), timeout=2):
                pass
        self.assertEqual(error.exception.code, 302)
        self.assertEqual(self.visits, ['/redirect'])

    def test_direct_response_preserved(self):
        with preflight.open_pinned(Request(self.url + '/direct'), timeout=2) as response:
            self.assertEqual(response.read(32), b'minSdkVersion 21')
        self.assertEqual(self.visits, ['/direct'])


if __name__ == '__main__':
    unittest.main()
