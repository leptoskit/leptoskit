"""Exercise the generated CLI's argument parsing and actual HTTP requests."""
import http.server
import json
import subprocess
import sys
import threading

requests = []
class Capture(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        requests.append({"query": self.path.partition("?")[2], "body": self.rfile.read(int(self.headers.get("Content-Length", 0))).decode()})
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", "4")
        self.end_headers()
        self.wfile.write(b'"ok"')
    def log_message(self, *args):
        pass

server = http.server.HTTPServer(("127.0.0.1", 0), Capture)
worker = threading.Thread(target=server.serve_forever, daemon=True)
worker.start()
try:
    values = {"items": [], "labels": {}, "note": None, "enabled": False, "count": 0, "text": ""}
    nested = {**values, "items": [None, "a +&é", ""], "labels": {"key +&é": None, "empty": ""}, "text": "a +&é"}
    for arguments in [values, {**values, "fallback": ""}, nested]:
        command = [sys.argv[1], "--server-url", f"http://127.0.0.1:{server.server_port}", "probe"]
        for key, value in arguments.items():
            command += [f"--{key}", json.dumps(value)]
        result = subprocess.run(command, capture_output=True, text=True, timeout=20)
        if result.returncode:
            raise AssertionError(result.stderr)
finally:
    server.shutdown()
    worker.join(timeout=5)
    server.server_close()
print(json.dumps(requests))
