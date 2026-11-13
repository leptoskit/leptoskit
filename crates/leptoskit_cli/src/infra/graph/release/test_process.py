"""Executable adapter tests: real benign HTTP, synthetic Linux proc/systemd."""
import copy
import hashlib
import json
import os
import pathlib
import selectors
import subprocess
import sys
import tempfile
import types
import unittest

from host import atomic
from process import Processes, validate


class Executables(unittest.TestCase):
    def test_embedded_worker_loads_without_a_remote_module_installation(self):
        directory = pathlib.Path(__file__).parent
        worker = "\n".join((directory / name).read_text() for name in ("process.py", "instances.py", "host.py"))
        namespace = {"__name__": "embedded_worker"}
        exec(compile(worker, "embedded_worker", "exec"), namespace)
        self.assertIsInstance(namespace["Linux"]().processes, namespace["Processes"])

    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.directory.name)
        self.release = self.root / "releases/v1"
        self.release.mkdir(parents=True)
        self.system = self.root / "systemd"
        self.system.mkdir()
        self.child = subprocess.Popen([sys.executable, str(pathlib.Path(__file__).with_name("fixture_http.py"))],
                                      env={**os.environ, "LEPTOS_SITE_ADDR": "127.0.0.1:0"},
                                      stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        self.addCleanup(self.cleanup_child)

        with selectors.DefaultSelector() as ready:
            ready.register(self.child.stdout, selectors.EVENT_READ)
            self.assertTrue(ready.select(5), "benign HTTP child did not report its binding")
        self.port = int(self.child.stdout.readline())
        data = b"fixture executable bytes"
        self.contract = {"account": "fixture", "address": f"127.0.0.1:{self.port}",
                         "address_env": "LEPTOS_SITE_ADDR",
                         "environment": {"LEPTOS_SITE_ADDR": f"127.0.0.1:{self.port}"},
                         "backend": {"sha256": hashlib.sha256(data).hexdigest(), "size": len(data), "mode": 0o755},
                         "health": "/healthz", "ready": "/readyz", "runtime": None}
        (self.release / "backend").write_bytes(data)
        self.save_contract()
        self.proc = self.root / "proc"
        (self.proc / "17/fd").mkdir(parents=True)
        (self.proc / "net").mkdir()
        (self.proc / "17/exe").symlink_to(self.release / "backend")
        (self.proc / "17/fd/3").symlink_to("socket:[1234]")
        self.stat = self.proc / "17/stat"
        self.stat.write_text("17 (fixture ) process) " + " ".join(["0"] * 19 + ["42"]))
        self.table = self.proc / "net/tcp"
        self.table.write_text(f"header\n0: 0100007F:{self.port:04X} 0 0A 0 0 0 0 0 1234\n")
        self.commands = []
        self.pid = b"17"
        self.drops = b""
        self.adapter = Processes(command=self.command, proc=self.proc,
                                 accounts=lambda name: types.SimpleNamespace(pw_uid=os.geteuid(), pw_gid=1),
                                 read=lambda path, mode=None: pathlib.Path(path).read_bytes(), pause=lambda _: None)

    def save_contract(self):
        (self.release / "process.json").write_text(json.dumps(self.contract))

    def command(self, args):
        self.commands.append(args)

        if "MainPID" in args:
            return self.pid

        if "DropInPaths" in args:
            return self.drops
        return b""

    def tearDown(self):
        self.directory.cleanup()

    def cleanup_child(self):
        # This fixture owns the child; never enumerate or kill other processes.
        self.child.terminate()
        try:
            self.child.wait(timeout=5)

        except subprocess.TimeoutExpired:
            self.child.kill()
            self.child.wait()
        self.child.stdout.close()

    def installed(self):
        path = self.system / self.adapter.name(self.release)
        path.write_bytes(self.adapter.unit(self.release, self.contract))

        return path

    def test_observation_binds_both_real_http_probes_to_process_socket_and_bytes(self):
        observation = self.adapter.observe(self.release, self.contract)
        self.assertEqual(observation["starttime"], "42")
        self.assertEqual(observation["socket_inode"], "1234")
        self.assertEqual(observation["attestation"], "absent")

    def test_start_is_idempotent_and_never_restarts_another_release(self):
        self.installed()
        self.adapter.start(self.release, self.contract, atomic, self.system)
        self.adapter.start(self.release, self.contract, atomic, self.system)

        self.assertFalse(any("restart" in args or "stop" in args for args in self.commands))
        names = {args[-1] for args in self.commands if "start" in args}
        self.assertEqual(names, {self.adapter.name(self.release)})

    def test_candidate_collision_fails_before_systemd_writes(self):
        with self.assertRaisesRegex(ValueError, "port is occupied"):
            self.adapter.start(self.release, self.contract, atomic, self.system)
        self.assertEqual(list(self.system.iterdir()), [])
        self.assertEqual(self.commands, [])

    def test_changed_executable_fails_before_systemd_calls(self):
        (self.release / "backend").write_bytes(b"changed")
        with self.assertRaisesRegex(ValueError, "bytes differ"):
            self.adapter.start(self.release, self.contract, atomic, self.system)
        self.assertEqual(self.commands, [])

    def test_substituted_process_and_socket_are_not_readiness(self):
        (self.proc / "17/exe").unlink()
        (self.proc / "17/exe").symlink_to(self.release / "process.json")
        with self.assertRaises(RuntimeError):
            self.adapter.observe(self.release, self.contract)
        (self.proc / "17/exe").unlink()
        (self.proc / "17/exe").symlink_to(self.release / "backend")
        self.table.write_text(self.table.read_text().replace("1234", "9999"))
        with self.assertRaises(RuntimeError):
            self.adapter.observe(self.release, self.contract)

    def test_wildcard_socket_is_not_private_binding(self):
        self.table.write_text(self.table.read_text().replace("0100007F", "00000000"))
        with self.assertRaises(RuntimeError):
            self.adapter.observe(self.release, self.contract)

    def test_restart_during_http_probe_rejected(self):
        original = self.adapter.probe
        def restart(port, endpoint):
            original(port, endpoint)
            self.pid = b"18"
        self.adapter.probe = restart
        with self.assertRaises(RuntimeError):
            self.adapter.observe(self.release, self.contract)

    def test_enable_requires_fresh_readiness_and_stop_is_exact(self):
        self.adapter.enable(self.release, self.contract)
        self.assertIn(["systemctl", "enable", self.adapter.name(self.release)], self.commands)
        self.commands.clear()
        self.adapter.stop(self.release)
        self.assertEqual(self.commands, [["systemctl", "stop", self.adapter.name(self.release)],
                                        ["systemctl", "disable", self.adapter.name(self.release)]])

    def test_failed_readiness_never_enables_boot(self):
        self.contract["ready"] = "/absent"
        self.save_contract()
        with self.assertRaises(RuntimeError):
            self.adapter.enable(self.release, self.contract)

        self.assertFalse(any("enable" in args for args in self.commands))

    def test_unit_and_dropin_substitution_rejected(self):
        path = self.installed()
        path.write_bytes(b"unexpected")
        with self.assertRaisesRegex(ValueError, "unit differs"):
            self.adapter.start(self.release, self.contract, atomic, self.system)
        self.installed()
        self.drops = b"/unexpected.conf"
        with self.assertRaisesRegex(ValueError, "drop-ins"):
            self.adapter.start(self.release, self.contract, atomic, self.system)

    def test_explicit_address_and_unsupported_runtime_fail_closed(self):
        for change in ({"address_env": "BAD ENV"}, {"address": "0.0.0.0:12345"},
                       {"runtime": {"socket": "/var/run/docker.sock"}}):
            contract = {**copy.deepcopy(self.contract), **change}
            with self.assertRaises(ValueError):
                validate(contract)

    def test_custom_address_environment_is_preserved_in_unit(self):
        # The actual playground uses PLAY_BIND, not a toolkit-specific name.
        self.contract["address_env"] = "PLAY_BIND"
        self.contract["environment"] = {"PLAY_BIND": self.contract["address"]}
        unit = self.adapter.unit(self.release, self.contract)
        self.assertIn(f"Environment=PLAY_BIND={self.contract['address']}\n".encode(), unit)
        self.assertNotIn(b"LEPTOS_SITE_ADDR", unit)
        self.contract["environment"]["PLAY_BIND"] = "127.0.0.1:1"
        with self.assertRaisesRegex(ValueError, "listen environment differs"):
            validate(self.contract)

    def test_retirement_requires_owned_unit_but_tolerates_unstarted_candidate(self):
        self.adapter.stop_if_installed(self.release, self.contract, self.system)
        self.assertEqual(self.commands, [])
        path = self.installed()
        path.write_bytes(b"foreign unit")
        with self.assertRaisesRegex(ValueError, "retiring application unit differs"):
            self.adapter.stop_if_installed(self.release, self.contract, self.system)
        self.assertEqual(self.commands, [])
        self.installed()

        self.adapter.stop_if_installed(self.release, self.contract, self.system)
        self.assertEqual(self.commands[-2:], [["systemctl", "stop", self.adapter.name(self.release)],
                                             ["systemctl", "disable", self.adapter.name(self.release)]])

    def test_comma_separated_origin_and_cidr_settings_remain_literal(self):
        self.contract["environment"]["PLAY_ORIGINS"] = "https://leptoskit.dev,https://play.leptoskit.dev"
        self.contract["environment"]["PLAY_CIDRS"] = "127.0.0.1/32,10.0.0.0/8"
        unit = self.adapter.unit(self.release, self.contract)
        self.assertIn(b"Environment=PLAY_CIDRS=127.0.0.1/32,10.0.0.0/8\n", unit)
        self.contract["environment"]["PLAY_ORIGINS"] = "value\nExecStart=/foreign"
        with self.assertRaisesRegex(ValueError, "printable Unicode"):
            self.adapter.unit(self.release, self.contract)

    def test_root_identity_is_never_accepted(self):
        self.adapter.accounts = lambda name: types.SimpleNamespace(pw_uid=0, pw_gid=0)
        with self.assertRaisesRegex(ValueError, "must not be root"):
            self.adapter.check(self.release, self.contract)


if __name__ == "__main__":
    unittest.main()
