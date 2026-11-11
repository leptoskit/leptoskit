"""Cheap deterministic tests: executes the production host state machine."""
import base64
import copy
import fcntl
import json
import os
import pathlib
import tempfile
import unittest

from host import Linux, Machine, canonical, digest, inventory, pointer
from process import masked_dropins
from unittest import mock


class Interrupted(BaseException):
    pass


class Runtime:
    def __init__(self):
        self.ready = True
        self.rejected_versions = set()
        self.pinned = True
        self.restarts = 0

    def preflight(self, host):
        if not self.pinned:
            raise ValueError("runtime/TLS prerequisites changed")

    def candidate(self, host, edge, releases):
        if not self.ready:
            raise ValueError("candidate readiness failed")

    def restart(self, host):
        self.restarts += 1

    def active(self, host, releases):
        if any(release.name in self.rejected_versions for release in releases.values()):
            raise ValueError("active route failed")

    def stop(self, host):
        pass


class Releases(unittest.TestCase):
    def setUp(self):
        # Avoid world-writable ancestors: this is also required of Linux roots.
        self.directory = tempfile.TemporaryDirectory(dir=os.getcwd())
        self.root = pathlib.Path(self.directory.name)
        self.runtime = Runtime()
        self.machine = Machine(self.runtime)

    def tearDown(self):
        self.directory.cleanup()

    def request(self, version="v1", content=b"first"):
        tree = {"index.html": {"kind": "file", "mode": 0o644, "size": len(content), "sha256": digest(content)}}
        host = {"root": str(self.root), "units": {"frontends.docs": tree},
                "edge": {"test": "generated routes"}, "caddy_sha256": digest(b"caddy")}

        return {"action": "prepare", "operation": digest(canonical([version, host])),
                "version": version, "host": host, "files": {"frontends.docs/index.html": base64.b64encode(content).decode()},
                "caddy": base64.b64encode(b"caddy").decode(), "rollback": False}

    def deploy(self, request):
        self.machine.run(request)

        return self.machine.run({**request, "action": "activate"})

    def current(self):
        return pointer(self.root / "units/frontends.docs/current")

    def test_install_activate_retry_and_rollback(self):
        first = self.request()
        self.assertEqual(self.deploy(first)["phase"], "complete")
        second = self.request("v2", b"second")
        self.deploy(second)
        self.assertEqual(self.current(), "releases/v2")
        rollback = {**first, "rollback": True}
        self.deploy(rollback)
        self.assertEqual(self.current(), "releases/v1")

        self.deploy(rollback)
        self.assertEqual(self.current(), "releases/v1")

    def test_restrictive_installer_umask_preserves_declared_public_modes(self):
        request = self.request()
        tree = request["host"]["units"]["frontends.docs"]
        tree["nested"] = {"kind": "directory", "mode": 0o755}
        tree["nested/page.html"] = tree["index.html"].copy()
        request["files"]["frontends.docs/nested/page.html"] = request["files"]["frontends.docs/index.html"]
        previous = os.umask(0o077)
        try:
            self.deploy(request)

        finally:
            os.umask(previous)
        for path in ("units", "units/frontends.docs", "units/frontends.docs/releases",
                     "units/frontends.docs/releases/v1/assets", "units/frontends.docs/releases/v1/assets/nested"):
            self.assertEqual((self.root / path).stat().st_mode & 0o777, 0o755)

    def test_readiness_failure_keeps_previous(self):
        self.deploy(self.request())
        self.runtime.ready = False
        with self.assertRaises(ValueError):
            self.deploy(self.request("v2", b"second"))
        self.assertEqual(self.current(), "releases/v1")
        self.assertEqual(json.loads((self.root / "journal.json").read_bytes())["phase"], "reverted")
        self.runtime.ready = True
        self.deploy(self.request("v3", b"corrected"))

        self.assertEqual(self.current(), "releases/v3")

    def test_active_failure_restores_previous(self):
        self.deploy(self.request())
        self.runtime.rejected_versions = {"v2"}
        with self.assertRaises(ValueError):
            self.deploy(self.request("v2", b"second"))
        self.assertEqual(self.current(), "releases/v1")
        self.assertEqual(json.loads((self.root / "journal.json").read_bytes())["phase"], "reverted")

    def test_failed_restoration_retains_nonterminal_journal(self):
        self.deploy(self.request())
        self.runtime.rejected_versions = {"v1", "v2"}
        second = self.request("v2", b"second")
        with self.assertRaises(ValueError):
            self.deploy(second)
        self.assertEqual(self.current(), "releases/v1")
        state = json.loads((self.root / "journal.json").read_bytes())
        self.assertNotIn(state["phase"], ("complete", "reverted"))

        with self.assertRaises(ValueError):
            self.deploy(self.request("v3", b"third"))
        self.runtime.rejected_versions.clear()
        self.deploy(second)
        self.assertEqual(self.current(), "releases/v2")

    def test_same_version_different_bytes(self):
        self.deploy(self.request())
        with self.assertRaises(ValueError):
            self.deploy(self.request(content=b"substituted"))
        self.assertEqual(self.current(), "releases/v1")

    def test_substituted_copy_never_activates(self):
        request = self.request()
        request["files"]["frontends.docs/index.html"] = base64.b64encode(b"substituted").decode()
        with self.assertRaises(ValueError):
            self.deploy(request)
        self.assertIsNone(self.current())
        self.deploy(self.request("v2", b"corrected"))
        self.assertEqual(self.current(), "releases/v2")

    def test_changed_installed_bytes_blocks_activation_and_rollback(self):
        first = self.request()
        self.deploy(first)
        self.deploy(self.request("v2", b"second"))
        (self.root / "units/frontends.docs/releases/v1/assets/index.html").write_bytes(b"bad")
        with self.assertRaises(ValueError):
            self.deploy({**first, "rollback": True})
        self.assertEqual(self.current(), "releases/v2")

    def test_extra_member_and_mode_change(self):
        for change in ("extra", "mode"):
            with self.subTest(change=change):
                request = self.request()
                self.machine.run(request)
                assets = self.root / "units/frontends.docs/releases/v1/assets"

                if change == "extra":
                    (assets / "extra").write_text("extra")
                else:
                    (assets / "index.html").chmod(0o600)
                with self.assertRaises(ValueError):
                    self.machine.run({**request, "action": "activate"})

                if change == "extra":
                    (assets / "extra").unlink()
                else:
                    (assets / "index.html").chmod(0o644)

    def test_runtime_prerequisites_fail_before_mutation(self):
        self.runtime.pinned = False
        with self.assertRaises(ValueError):
            self.machine.run(self.request())
        self.assertEqual(list(self.root.iterdir()), [])

    def test_foreign_operation_and_concurrent_lock(self):
        request = self.request()
        self.machine.run(request)
        with self.assertRaises(ValueError):
            self.machine.run(self.request("v2"))
        with open(self.root / "operation.lock", "r+") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            with self.assertRaises(BlockingIOError):
                self.machine.run(request)

    def test_docs_only_preserves_unselected_service_tree_and_history(self):
        service = self.root / "units/services.compiler"
        service.mkdir(parents=True)
        (service / "journal.json").write_bytes(b"untouched history")
        (service / "process-state").write_bytes(b"running")

        before = {p.name: p.read_bytes() for p in service.iterdir()}
        self.deploy(self.request())

        self.assertEqual(before, {p.name: p.read_bytes() for p in service.iterdir()})

    def test_interruptions_recover_without_false_history(self):
        for point in ("installing", "installed:frontends.docs", "prepared", "before_units",
                      "switched:frontends.docs", "before_edge", "after_edge", "after_restart", "complete"):
            with self.subTest(point=point):
                # Distinct roots make every point an independent transaction.
                target = self.root / point.replace(":", "-")
                target.mkdir()
                first = self.request()
                first["host"]["root"] = str(target)
                first["operation"] = digest(canonical(first["host"]))
                self.deploy(first)

                second = self.request("v2", b"second")
                second["host"]["root"] = str(target)
                second["operation"] = digest(canonical(second["host"]))
                def interrupt(at):
                    if at == point:
                        raise Interrupted()
                machine = Machine(self.runtime, interrupt)
                with self.assertRaises(Interrupted):
                    machine.run(second)
                    machine.run({**second, "action": "activate"})
                journal = json.loads((target / "journal.json").read_bytes())

                if point != "complete":
                    self.assertNotEqual((journal["version"], journal["phase"]), ("v2", "complete"))
                self.deploy(second)
                self.assertEqual(pointer(target / "units/frontends.docs/current"), "releases/v2")

    def test_changed_runtime_edge_and_manifest_bindings(self):
        request = self.request()
        self.machine.run(request)
        for field, value in (("edge", {"changed": True}), ("caddy_sha256", "0" * 64),
                             ("trust", {"host_key": "changed"})):
            changed = copy.deepcopy(request)
            changed["host"][field] = value
            with self.assertRaises(ValueError):
                self.machine.run({**changed, "action": "activate"})
        self.assertIsNone(self.current())


class SystemdMasks(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(dir=os.getcwd())
        self.addCleanup(self.directory.cleanup)
        self.root = pathlib.Path(self.directory.name)
        self.mask = self.root / "10-timeout-abort.conf"
        self.mask.symlink_to("/dev/null")

    def test_empty_and_protected_absolute_null_masks_are_inert(self):
        second = self.root / "20-disabled.conf"
        second.symlink_to("/dev/null")
        self.assertTrue(masked_dropins(b""))
        self.assertTrue(masked_dropins(f"{self.mask} {second}\n".encode()))

    def test_active_missing_relative_and_non_null_dropins_are_rejected(self):
        active = self.root / "active.conf"
        active.write_text("[Service]\nUser=root\n")
        alias = self.root / "alias.conf"
        alias.symlink_to(active)
        relative = self.root / "relative.conf"
        relative.symlink_to("../../../dev/null")
        for path in (active, alias, relative, self.root / "missing.conf", pathlib.Path("relative.conf")):
            with self.subTest(path=path):
                self.assertFalse(masked_dropins(str(path).encode()))
        self.assertFalse(masked_dropins(f"{self.mask} {active}".encode()))
        self.assertFalse(masked_dropins(b"\xff"))

    def test_masks_reject_writable_or_aliased_parents_and_foreign_ownership(self):
        self.root.chmod(0o777)
        self.assertFalse(masked_dropins(str(self.mask).encode()))
        self.root.chmod(0o700)
        alias = self.root / "alias"
        alias.symlink_to(self.root, target_is_directory=True)
        self.assertFalse(masked_dropins(str(alias / self.mask.name).encode()))
        with mock.patch("process.os.geteuid", return_value=os.geteuid() + 1):
            self.assertFalse(masked_dropins(str(self.mask).encode()))

    def test_first_release_recovery_can_stop_a_unit_with_a_verified_mask(self):
        runtime = Linux()
        host = {"root": "/var/lib/leptoskit/example", "edge": {}}
        unit = runtime.service(host)
        (self.root / unit).write_bytes(runtime.unit(host))
        commands = []
        def command(args):
            commands.append(args)
            return str(self.mask).encode() if "DropInPaths" in args else b""
        runtime.command = command
        runtime.stop(host, self.root)
        self.assertEqual(commands[-2:], [["systemctl", "stop", unit], ["systemctl", "disable", unit]])


class ServiceLifecycle(unittest.TestCase):
    def test_service_is_enabled_for_boot_and_disabled_without_previous_release(self):
        runtime = Linux()
        commands = []
        def command(args):
            commands.append(args)

            return b""
        runtime.command = command
        runtime.install_service = lambda host: None
        host = {"root": "/var/lib/leptoskit/example", "edge": {}}
        runtime.restart(host)
        unit = "leptoskit-edge-example.service"
        self.assertEqual(commands, [["systemctl", "enable", unit],
                                    ["systemctl", "restart", unit],
                                    ["systemctl", "is-active", "--quiet", unit]])

        commands.clear()
        with tempfile.TemporaryDirectory() as temporary:
            system = pathlib.Path(temporary)
            runtime.stop(host, system)
            self.assertEqual(commands, [], "uninstalled edge must not block first-release recovery")
            path = system / unit
            path.write_bytes(runtime.unit(host))
            runtime.stop(host, system)
            self.assertEqual(commands, [["systemctl", "show", "-p", "DropInPaths", "--value", unit],
                                        ["systemctl", "stop", unit], ["systemctl", "disable", unit]])
            commands.clear()
            path.write_bytes(b"foreign unit")
            with self.assertRaisesRegex(ValueError, "retiring edge service differs"):
                runtime.stop(host, system)
            self.assertEqual(commands, [])


if __name__ == "__main__":
    unittest.main()
