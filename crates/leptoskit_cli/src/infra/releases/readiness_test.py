"""Exercise the actual remote probe with local HTTP and a Docker boundary."""

import copy
import http.server
import json
import os
import pathlib
import runpy
import subprocess
import sys
import tempfile
import threading
import unittest

PROBE = pathlib.Path(__file__).with_name("readiness.py")


class ProbeTests(unittest.TestCase):
    def setUp(self):
        self.status = 200
        owner = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                self.send_response(owner.status if self.path == "/readyz" else 404)
                self.end_headers()

            def log_message(self, *_args):
                pass

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever)
        self.thread.start()
        self.directory = tempfile.TemporaryDirectory()
        self.docker = pathlib.Path(self.directory.name) / "docker.py"
        self.docker.write_text(
            "import os,sys\n"
            "print(os.environ['PROBE_ACTIVE' if sys.argv[1]=='ps' else 'PROBE_INSPECT'])\n"
        )
        self.record = {
            "Name": "/notes-web-v1",
            "State": {"Running": True, "Restarting": False},
            "Config": {"Env": ["TOKEN=fixture-current-secret", "PATH=/usr/bin"]},
            "NetworkSettings": {"Networks": {"kamal": {"IPAddress": "127.0.0.1"}}},
        }

        self.request = {
            "container": "notes-web-v1", "service": "notes", "port": self.server.server_port,
            "environment": {"TOKEN": "fixture-current-secret"},
        }

    def tearDown(self):
        self.server.shutdown()
        self.thread.join()
        self.server.server_close()
        self.directory.cleanup()

    def invoke(self, record=None, active="notes-web-v1", optimized=False):
        environment = dict(os.environ)
        environment["PROBE_ACTIVE"] = active
        environment["PROBE_INSPECT"] = json.dumps([record or self.record])
        command = [sys.executable] + (["-O"] if optimized else []) + [
            __file__, "--probe", str(self.docker), str(PROBE),
        ]
        result = subprocess.run(command, input=json.dumps(self.request), text=True,
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=environment, timeout=15)
        self.assertNotIn("fixture-current-secret", result.stdout + result.stderr)
        self.assertNotIn("fixture-rotated-secret", result.stdout + result.stderr)

        return result

    def test_current_running_ready_container_passes(self):
        result = self.invoke()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout), {"ready": True})

    def test_stopped_restarting_or_replaced_container_fails(self):
        for field, value in [("Running", False), ("Restarting", True)]:
            record = copy.deepcopy(self.record)
            record["State"][field] = value
            self.assertNotEqual(self.invoke(record).returncode, 0)
        record = copy.deepcopy(self.record)
        record["Name"] = "/notes-web-other"
        self.assertNotEqual(self.invoke(record).returncode, 0)

    def test_no_current_or_multiple_active_versions_fail(self):
        for active in ["", "notes-web-v0", "notes-web-v1\nnotes-web-v0"]:
            self.assertNotEqual(self.invoke(active=active).returncode, 0)

    def test_changed_or_missing_environment_fails_without_leaking_values(self):
        record = copy.deepcopy(self.record)

        for values in [[], ["TOKEN=fixture-rotated-secret"]]:
            record["Config"]["Env"] = values
            self.assertNotEqual(self.invoke(record).returncode, 0)

    def test_unready_http_response_fails(self):
        for status in [301, 401, 503]:
            self.status = status
            self.assertNotEqual(self.invoke().returncode, 0)

    def test_optimized_python_does_not_disable_checks(self):
        record = copy.deepcopy(self.record)
        record["State"]["Running"] = False
        self.assertNotEqual(self.invoke(record, optimized=True).returncode, 0)

    def test_invalid_network_address_is_not_contacted(self):
        record = copy.deepcopy(self.record)
        record["NetworkSettings"]["Networks"]["kamal"]["IPAddress"] = "invalid"
        self.assertNotEqual(self.invoke(record).returncode, 0)


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--probe":
        # Launch the Docker boundary through Python on every host. Windows
        # cannot execute the extensionless shebang fixture used on Unix.
        run = subprocess.run

        def docker(command, **kwargs):
            if command[0] == "docker":
                command = [sys.executable, sys.argv[2], *command[1:]]
            return run(command, **kwargs)

        subprocess.run = docker
        runpy.run_path(sys.argv[3], run_name="__main__")
    else:
        unittest.main()
