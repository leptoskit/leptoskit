"""Exercise the actual remote command and bounded local process transport."""
import contextlib
import io
import json
import pathlib
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

from host import canonical, digest
import test_host
import transport


class Transport(unittest.TestCase):
    def envelope(self, directory, worker):
        identity = pathlib.Path(directory) / "identity with spaces"
        identity.write_bytes(b"test credential placeholder")
        identity.chmod(0o600)
        return {"host": {"trust": {"identity": str(identity), "ipv4": "192.0.2.1",
                                  "port": 2222, "host_key": "ssh-ed25519 dGVzdA=="}},
                "worker": worker, "request": {"action": "activate", "operation": "test"}}

    def run_main(self, envelope, exchange):
        output = io.StringIO()
        with patch.object(sys, "stdin", io.StringIO(json.dumps(envelope))), \
                patch.object(transport, "exchange", exchange), contextlib.redirect_stdout(output):
            transport.main()
        return json.loads(output.getvalue())

    def test_exact_worker_runs_with_request_and_pinned_ssh_options(self):
        worker = "import json,sys\nr=json.load(sys.stdin)\nprint(json.dumps({'phase':'complete','operation':r['operation'],'quoted':\"it's exact\"}))"
        exchange = transport.exchange
        with tempfile.TemporaryDirectory() as directory:
            envelope = self.envelope(directory, worker)

            def local_ssh(command, request):
                self.assertEqual(command[0:3], ["ssh", "-F", "/dev/null"])
                for option in ("BatchMode=yes", "StrictHostKeyChecking=yes", "IdentityAgent=none",
                               "ForwardAgent=no", "IdentitiesOnly=yes", "GlobalKnownHostsFile=/dev/null"):
                    self.assertIn(option, command)
                known = next(arg.split("=", 1)[1] for arg in command if arg.startswith("UserKnownHostsFile="))
                self.assertEqual(pathlib.Path(known).read_text(),
                                 "[192.0.2.1]:2222 ssh-ed25519 dGVzdA==\n192.0.2.1 ssh-ed25519 dGVzdA==\n")
                self.assertEqual(command[-5:-1], ["-p", "2222", "--", "root@192.0.2.1"])
                # Execute exactly the shell command SSH would receive, without a network host.
                return exchange(["/bin/sh", "-c", command[-1]], request)

            result = self.run_main(envelope, local_ssh)
            self.assertEqual(result["quoted"], "it's exact")

    def test_incomplete_wrong_operation_and_error_never_report_success(self):
        with tempfile.TemporaryDirectory() as directory:
            envelope = self.envelope(directory, "")
            for value in ({"phase": "reverted", "operation": "test"},
                          {"phase": "complete", "operation": "other"},
                          {"error": "failed"}, []):
                with self.subTest(value=value), self.assertRaises(RuntimeError):
                    self.run_main(envelope, lambda *_: json.dumps(value).encode())

    def test_private_identity_required_before_starting_ssh(self):
        with tempfile.TemporaryDirectory() as directory:
            envelope = self.envelope(directory, "")
            pathlib.Path(envelope["host"]["trust"]["identity"]).chmod(0o644)
            with patch.object(transport, "exchange") as exchange, self.assertRaises(ValueError):
                self.run_main(envelope, exchange)
            exchange.assert_not_called()

    def test_upload_and_reply_can_exceed_pipe_capacity(self):
        data = b"x" * (256 * 1024)
        script = "import sys;sys.stdout.buffer.write(b'y'*131072);sys.stdout.flush();data=sys.stdin.buffer.read();sys.stdout.buffer.write(data)"
        self.assertEqual(transport.exchange([sys.executable, "-c", script], data), b"y" * 131072 + data)

    def test_output_limit_applies_while_child_is_running(self):
        script = "import os\nwhile True: os.write(1,b'x'*65536)"
        with self.assertRaisesRegex(ValueError, "output exceeds"):
            transport.exchange([sys.executable, "-c", script], b"", timeout=5, output_limit=1024)

    def test_timeout_and_nonzero_exit_fail(self):
        with self.assertRaises((TimeoutError, subprocess.TimeoutExpired)):
            transport.exchange([sys.executable, "-c", "import time;time.sleep(5)"], b"", timeout=0.05)
        with self.assertRaisesRegex(RuntimeError, "SSH failed"):
            transport.exchange([sys.executable, "-c", "raise SystemExit(7)"], b"")

    def test_two_large_asset_releases_fit_the_actual_transport_bound(self):
        fixture = test_host.Releases()
        fixture.setUp()
        self.addCleanup(fixture.tearDown)
        module_path = str(pathlib.Path(__file__).resolve().parent)
        worker = (f"import sys;sys.path.insert(0,{module_path!r})\n"
                  "import json\nfrom host import Machine\nfrom test_host import Runtime\n"
                  "print(json.dumps(Machine(Runtime()).run(json.load(sys.stdin))))")

        def exchange(request):
            output = transport.exchange([sys.executable, "-c", worker], canonical(request))
            self.assertLess(len(output), 64 * 1024)
            return json.loads(output)

        def release(version, content):
            request = fixture.request(version, content)
            tree = request["host"]["units"]["frontends.docs"]
            for index in range(10000):
                name = f"page-{index:05d}.txt"
                tree[name] = {"kind": "file", "mode": 0o644, "size": 1, "sha256": digest(b"x")}
                request["files"]["frontends.docs/" + name] = "eA=="
            request["operation"] = digest(canonical([version, request["host"]]))
            return request

        for version, content in [("v1", b"first"), ("v2", b"second")]:
            request = release(version, content)
            for _ in range(2):
                prepared = exchange(request)
                self.assertEqual((prepared["operation"], prepared["phase"]),
                                 (request["operation"], "prepared"))
            for _ in range(2):
                self.assertEqual(exchange({**request, "action": "activate"})["phase"], "complete")
            status = exchange({**request, "action": "status"})
            self.assertEqual(status["journal"]["version"], version)
            self.assertEqual(status["current"]["frontends.docs"], "releases/" + version)

        journal = fixture.root / "journal.json"
        self.assertGreater(journal.stat().st_size, 1024 * 1024)
        self.assertEqual(len(json.loads(journal.read_bytes())["previous_pins"]["frontends.docs"]), 10001)


if __name__ == "__main__":
    unittest.main()
