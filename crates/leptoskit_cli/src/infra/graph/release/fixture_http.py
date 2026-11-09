"""Benign HTTP fixture that actually consumes the declared listen contract.

No compilation, containers, database or subprocess execution. Port zero is
permitted only for local tests; release contracts require an explicit port.
"""
import http.server
import os


class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200 if self.path in ("/healthz", "/readyz", "/") else 404)
        self.end_headers()
        self.wfile.write(b"benign fixture ready")

    def log_message(self, *args):
        pass


if __name__ == "__main__":
    address = os.environ["LEPTOS_SITE_ADDR"]
    host, port = address.rsplit(":", 1)
    if host != "127.0.0.1":
        raise ValueError("fixture only accepts loopback")
    server = http.server.HTTPServer((host, int(port)), Handler)
    print(server.server_port, flush=True)
    server.serve_forever()
