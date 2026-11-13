"""Exercise generated graph edges using local HTTP/TLS, without cloud access.

Only deployment bindings are substituted: listener/upstream ports, public
tree roots and ephemeral certificate paths. Routing/header rules are unchanged.
"""

import contextlib
import copy
import hashlib
import http.client
import http.server
import json
import os
from pathlib import Path
import socket
import ssl
import subprocess
import sys
import tempfile
import threading
import time


def check(condition, reason):
    if not condition:
        raise AssertionError(reason)


class Backend(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        data = json.dumps({"path": self.path, "headers": dict(self.headers)}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.send_header("Set-Cookie", "upstream=private; Path=/")
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *_args):
        pass


def request(port, host, path, method="GET", headers=None, tls=False, ca=None, sni=None):
    connection = socket.create_connection(("127.0.0.1", port), timeout=5)
    if tls:
        context = ssl.create_default_context(cafile=str(ca))
        connection = context.wrap_socket(connection, server_hostname=sni or host)
    with connection:
        fields = {"Host": host, "Connection": "close", **(headers or {})}
        encoded = "".join(f"{key}: {value}\r\n" for key, value in fields.items())
        connection.sendall(f"{method} {path} HTTP/1.1\r\n{encoded}\r\n".encode())
        response = http.client.HTTPResponse(connection, method=method)
        response.begin()
        body = response.read(1024 * 1024)
        return response.status, dict(response.getheaders()), body


def port():
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return listener.getsockname()[1]


def walk(value):
    if isinstance(value, dict):
        yield value
        for child in value.values():
            yield from walk(child)
    elif isinstance(value, list):
        for child in value:
            yield from walk(child)


def bind(config, directory, upstream, listener):
    result = copy.deepcopy(config)
    result["apps"]["http"]["servers"]["edge"]["listen"] = [f"127.0.0.1:{listener}"]
    for value in walk(result):
        if value.get("handler") == "file_server":
            unit = value["root"].split("/units/")[1].split("/")[0]
            value["root"] = str(directory / unit)
        if "dial" in value:
            check(value["dial"] in ["127.0.0.1:4702", "127.0.0.1:3000", "127.0.0.1:3001"], "unexpected production upstream")
            value["dial"] = f"127.0.0.1:{upstream}"
        if "certificate" in value and "key" in value:
            value["certificate"] = str(directory / "origin.crt")
            value["key"] = str(directory / "origin.key")
    return result


@contextlib.contextmanager
def running(caddy, config, directory, label):
    path = directory / (label + ".json")
    path.write_text(json.dumps(config))
    log = directory / (label + ".log")
    with log.open("wb") as output:
        validate = subprocess.run([caddy, "validate", "--config", str(path)], stdout=output, stderr=output)
        check(validate.returncode == 0, log.read_text())
        process = subprocess.Popen([caddy, "run", "--config", str(path)], stdout=output, stderr=output)
        try:
            address = config["apps"]["http"]["servers"]["edge"]["listen"][0]
            listener = int(address.rsplit(":", 1)[1])
            deadline = time.monotonic() + 10
            while True:
                check(process.poll() is None, log.read_text())
                try:
                    with socket.create_connection(("127.0.0.1", listener), timeout=0.1):
                        break
                except OSError:
                    check(time.monotonic() < deadline, "edge did not bind")
                    time.sleep(0.025)
            yield listener
        finally:
            process.terminate()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)


def assets(directory):
    members = {
        "frontends.site/index.html": "landing",
        "frontends.site/docs/sentinel.html": "forbidden root fallback",
        "frontends.site/docs2": "root segment",
        "frontends.site/Docs/index.html": "root case",
        "frontends.docs/index.html": "docs",
        "frontends.docs/guide/index.html": "nested guide",
        "frontends.docs/pkg/app.wasm": "wasm payload",
        "frontends.docs/café.txt": "unicode asset",
        "frontends.play/index.html": "play",
    }
    for name, body in members.items():
        path = directory / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(body)


def static_checks(listener):
    expected = {"/": "landing", "/docs/": "docs", "/docs2": "root segment", "/Docs/": "root case", "/play/": "play", "/docs/guide/": "nested guide", "/docs/caf%C3%A9.txt": "unicode asset"}
    for path, body in expected.items():
        status, _, actual = request(listener, "leptoskit.dev", path)
        check(status == 200 and actual.decode() == body, f"ownership: {path}: {status} {actual!r}")
    for path in ["/docs/sentinel.html", "/docs/missing", "/play/missing"]:
        check(request(listener, "leptoskit.dev", path)[0] == 404, f"cross-owner fallback: {path}")
    check(request(listener, "unknown.example", "/")[0] == 404, "unknown host leaked a site")
    for host in ["LEPTOSKIT.DEV", "leptoskit.dev.", "leptoskit.dev:443"]:
        status, _, body = request(listener, host, "/docs/")
        check(status == 200 and body == b"docs", f"host normalization: {host}")
    for path in ["/docs//guide", "/docs/../index.html", "/docs/%2e%2e/index.html", "/docs%2findex.html", "/docs%5cindex.html", "/docs/%00", "/docs/./index.html"]:
        check(request(listener, "leptoskit.dev", path)[0] == 400, f"noncanonical path accepted: {path}")
    for path, location in [("/docs?next=a%2Fb", "/docs/?next=a%2Fb"), ("/docs/guide?q=1", "/docs/guide/?q=1")]:
        status, headers, _ = request(listener, "leptoskit.dev", path)
        check(status == 308 and headers.get("Location") == location, f"mount redirect: {path} {headers}")
    status, headers, body = request(listener, "leptoskit.dev", "/docs/pkg/app.wasm", method="HEAD")
    check(status == 200 and not body and headers.get("Content-Type") == "application/wasm", "Wasm HEAD/MIME")
    status, headers, body = request(listener, "leptoskit.dev", "/docs/", method="POST")
    check(status == 405 and headers.get("Allow") == "GET, HEAD", "static method contract")
    status, _, body = request(listener, "leptoskit.dev", "/docs/pkg/app.wasm", headers={"Range": "bytes=0-3"})
    check(status == 206 and body == b"wasm", "asset range request")


def proxy_checks(listener, tls=False, ca=None):
    supplied = {"Authorization": "Bearer local-test", "Cookie": "app=private", "X-Forwarded-For": "192.0.2.200", "X-Forwarded-Host": "evil.example", "X-Forwarded-Proto": "http", "Forwarded": "for=192.0.2.201", "CF-Connecting-IP": "198.51.100.7"}
    status, headers, body = request(listener, "play-api.leptoskit.dev", "/compile/status?q=1", headers=supplied, tls=tls, ca=ca)
    check(status == 200, f"proxy status {status}")
    echoed = json.loads(body)
    actual = {key.lower(): value for key, value in echoed["headers"].items()}
    check(echoed["path"] == "/compile/status?q=1", "proxy changed application path/query")
    check(actual.get("host") == "play-api.leptoskit.dev", "proxy Host spoof")
    check(actual.get("x-forwarded-host") == "play-api.leptoskit.dev", "forwarded Host spoof")
    check(actual.get("x-forwarded-proto") == "https", "forwarded scheme spoof")
    check(actual.get("x-forwarded-for") == ("127.0.0.1" if tls else "198.51.100.7"), f"client IP: {actual}")
    check("cookie" not in actual and "Set-Cookie" not in headers, "isolated service leaked ambient cookies")
    check("forwarded" not in actual and "cf-connecting-ip" not in actual, "untrusted identity headers remained")
    check(actual.get("authorization") == "Bearer local-test", "explicit service credential was stripped")


def certificate(directory):
    command = ["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1", "-subj", "/CN=leptoskit.dev", "-addext", "subjectAltName=DNS:leptoskit.dev,DNS:play-api.leptoskit.dev", "-keyout", str(directory / "origin.key"), "-out", str(directory / "origin.crt")]
    subprocess.run(command, check=True, capture_output=True)
    os.chmod(directory / "origin.key", 0o600)


def ssr_checks(listener):
    for path in ["/dynamic?q=1", "/play/dynamic?q=2"]:
        status, headers, body = request(listener, "leptoskit.dev", path, headers={"Cookie": "app=session"})
        check(status == 200, "SSR proxy failed")
        echoed = json.loads(body)
        check(echoed["path"] == path, "SSR mount/query was stripped")
        check(echoed["headers"].get("Cookie") == "app=session" and "Set-Cookie" in headers, "SSR own session was stripped")
    status, _, body = request(listener, "leptoskit.dev", "/docs/")
    check(status == 200 and body == b"docs", "SSR swallowed a static sibling")
    check(request(listener, "leptoskit.dev", "/docs/missing")[0] == 404, "missing docs fell through to SSR")


def retain(directory, source, caddy, version, passed):
    evidence = os.environ.get("LEPTOSKIT_EDGE_EVIDENCE")
    if not evidence:
        return
    target = Path(evidence)
    target.mkdir(parents=True, exist_ok=True)
    for path in directory.iterdir():
        if path.suffix in [".log", ".json", ".crt"]:
            (target / path.name).write_bytes(path.read_bytes())
    # No ephemeral private key is retained.
    (target / "generated.json").write_bytes(source.read_bytes())
    result = {"caddy": version, "binary_sha256": hashlib.sha256(Path(caddy).read_bytes()).hexdigest(), "source_sha256": hashlib.sha256(source.read_bytes()).hexdigest(), "result": "passed" if passed else "failed", "providers": "not_checked", "runtime_attestation": "absent"}
    (target / "result.json").write_text(json.dumps(result, indent=2) + "\n")


def run(caddy, source):
    version = subprocess.check_output([caddy, "version"], text=True).strip()
    document = json.loads(source.read_text())
    check(version.split()[0] == "v" + document["caddy_version"], "acceptance requires the recorded Caddy version")
    with tempfile.TemporaryDirectory(prefix="lk-edge-") as temporary:
        directory = Path(temporary)
        assets(directory)
        certificate(directory)
        backend = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Backend)
        worker = threading.Thread(target=backend.serve_forever, daemon=True)
        worker.start()
        passed = False
        try:
            for host in ["web", "compiler"]:
                config = bind(document["tunnel"][host], directory, backend.server_port, port())
                with running(caddy, config, directory, "tunnel-" + host) as listener:
                    (static_checks if host == "web" else proxy_checks)(listener)
            for host in ["web", "compiler"]:
                config = bind(document["https"][host], directory, backend.server_port, port())
                with running(caddy, config, directory, "https-" + host) as listener:
                    if host == "compiler":
                        proxy_checks(listener, tls=True, ca=directory / "origin.crt")
                    else:
                        status, _, body = request(listener, "leptoskit.dev", "/docs/", tls=True, ca=directory / "origin.crt")
                        check(status == 200 and body == b"docs", "HTTPS static ownership")
                        status, _, _ = request(listener, "play-api.leptoskit.dev", "/", tls=True, ca=directory / "origin.crt", sni="leptoskit.dev")
                        check(status == 421, "TLS Host/SNI mismatch accepted")
            config = bind(document["ssr"]["web"], directory, backend.server_port, port())
            with running(caddy, config, directory, "ssr") as listener:
                ssr_checks(listener)
            config = bind(document["tunnel"]["web"], directory, backend.server_port, port())
            routes = config["apps"]["http"]["servers"]["edge"]["routes"]
            routes[:] = [route for route in routes if "/docs" not in json.dumps(route.get("match", []))]
            with running(caddy, config, directory, "negative-owner") as listener:
                status, _, body = request(listener, "leptoskit.dev", "/docs/sentinel.html")
                check(status == 200 and body == b"forbidden root fallback", "ownership negative control did not reproduce")
            passed = True
        finally:
            backend.shutdown()
            backend.server_close()
            worker.join(timeout=5)
            retain(directory, source, caddy, version, passed)
    print("real generated edge ownership, HTTP/TLS, proxy identity and negative control passed")


if __name__ == "__main__":
    run(sys.argv[1], Path(sys.argv[2]))
