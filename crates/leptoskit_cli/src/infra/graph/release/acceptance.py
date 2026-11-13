"""Opt-in real Linux/SSH acceptance. Does not provision a machine or build tools.

LK_GRAPH_SSH_ACCEPTANCE names a private JSON file:
{"lk": "/absolute/built/lk", "project": "/absolute/project",
 "first": [all release flags for v1], "second": [all release flags for v2],
 "url": "https://docs.example.test/docs/", "first_body": "...", "second_body": "..."}
Both bundles/independent pins, SSH host trust and Linux Caddy must already exist.
"""
import json
import os
import pathlib
import subprocess
import unittest
import urllib.request


@unittest.skipUnless(os.environ.get("LK_GRAPH_SSH_ACCEPTANCE"), "requires allocated disposable Linux/SSH host")
class Acceptance(unittest.TestCase):
    def test_real_release_status_and_rollback(self):
        spec = json.loads(pathlib.Path(os.environ["LK_GRAPH_SSH_ACCEPTANCE"]).read_text())
        def command(action, flags):
            return subprocess.run([spec["lk"], "infra", "release", action, *flags],
                                  cwd=spec["project"], check=True, timeout=240,
                                  stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        def body():
            with urllib.request.urlopen(spec["url"], timeout=10) as response:
                return response.read().decode()
        for name in ("first", "second"):
            command("plan", spec[name])
            command("apply", spec[name])
            self.assertEqual(body(), spec[name + "_body"])
            command("status", spec[name])
        command("rollback", spec["first"])
        self.assertEqual(body(), spec["first_body"])


if __name__ == "__main__":
    unittest.main()
