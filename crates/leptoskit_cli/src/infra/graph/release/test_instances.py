"""Production release state machine with deterministic application processes.

This suite proves transaction behavior, not Linux/systemd/ELF acceptance.
"""
import base64
import copy
import json
import os
import pathlib
import tempfile
import unittest

from host import Machine, atomic, canonical, digest, pointer
from instances import render_edge
from test_host import Interrupted, Runtime


class ProcessRuntime:
    def __init__(self):
        self.running, self.enabled, self.events = {}, set(), []
        self.rejected, self.reject_stop = set(), set()

    def check(self, release, contract):
        pass  # Instances checks actual stored bytes; proc/account are separate tests.

    def observe(self, release, contract):
        if release.name in self.rejected or self.running.get(release) != contract:
            raise ValueError("application readiness failed")
        self.events.append(("observe", release))

        return {"kind": "fixture_local_observation", "attestation": "absent"}

    def start(self, release, contract, write):
        for other, value in self.running.items():
            if other != release and value["address"] == contract["address"]:
                raise ValueError("port occupied")
        self.running[release] = copy.deepcopy(contract)
        self.events.append(("start", release))

        return self.observe(release, contract)

    def enable(self, release, contract):
        result = self.observe(release, contract)
        self.enabled.add(release)
        self.events.append(("enable", release))

        return result

    def stop_if_installed(self, release, contract):
        if release.name in self.reject_stop:
            raise ValueError("cannot stop process")
        self.running.pop(release, None)
        self.enabled.discard(release)
        self.events.append(("stop", release))


class ExecutableReleases(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(dir=os.getcwd())
        self.root = pathlib.Path(self.directory.name)
        self.runtime = Runtime()
        self.runtime.processes = ProcessRuntime()
        self.machine = Machine(self.runtime)

    def tearDown(self):
        self.directory.cleanup()

    def release(self, version, unit="services.compiler"):
        return self.root / "units" / unit / "releases" / version

    def request(self, version="v1", body=b"binary one", unit="services.compiler"):
        contract = {"account": "compiler", "address": "127.0.0.1:9001", "address_env": "PLAY_BIND",
                    "environment": {"PLAY_BIND": "127.0.0.1:9001"},
                    "health": "/healthz", "ready": "/readyz", "runtime": None,
                    "backend": {"sha256": digest(body), "size": len(body), "mode": 0o755}}
        host = {"root": str(self.root), "units": {unit: {}}, "executables": {unit: contract},
                "edge": {"routes": [{"handler": "reverse_proxy", "upstreams": [{"dial": "127.0.0.1:9001"}]}]},
                "routes": {unit: {"domain": "play-api.example", "mount": "/"}},
                "caddy_sha256": digest(b"caddy")}

        return {"action": "prepare", "operation": digest(canonical([version, host])), "version": version,
                "host": host, "files": {}, "backends": {unit: base64.b64encode(body).decode()},
                "caddy": base64.b64encode(b"caddy").decode(), "rollback": False}

    def deploy(self, request):
        self.machine.run(request)

        return self.machine.run({**request, "action": "activate"})

    def journal(self):
        return json.loads((self.root / "journal.json").read_bytes())

    def test_candidate_coexists_until_fresh_activation_then_old_instance_retires(self):
        first, second = self.request(), self.request("v2", b"binary two")
        self.deploy(first)
        self.machine.run(second)
        processes = self.runtime.processes
        self.assertEqual(set(processes.running), {self.release("v1"), self.release("v2")})
        self.assertEqual(processes.enabled, {self.release("v1")})
        self.assertNotEqual(processes.running[self.release("v1")]["address"],
                            processes.running[self.release("v2")]["address"])
        self.assertEqual(pointer(self.root / "units/services.compiler/current"), "releases/v1")

        result = self.machine.run({**second, "action": "activate"})
        self.assertEqual(result["phase"], "complete")
        self.assertEqual(set(processes.running), {self.release("v2")})
        self.assertEqual(processes.enabled, {self.release("v2")})
        config = json.loads((self.root / "edge/current/config.json").read_bytes())
        self.assertEqual(config["routes"][0]["upstreams"][0]["dial"], processes.running[self.release("v2")]["address"])
        self.assertEqual(result["observations"]["services.compiler"]["attestation"], "absent")

    def test_stopped_old_instance_is_restarted_for_checked_rollback(self):
        first = self.request()
        self.deploy(first)
        self.deploy(self.request("v2", b"binary two"))
        self.deploy({**first, "rollback": True})
        self.assertEqual(set(self.runtime.processes.running), {self.release("v1")})
        self.assertEqual(self.runtime.processes.enabled, {self.release("v1")})
        self.deploy({**first, "rollback": True})
        self.assertEqual(set(self.runtime.processes.running), {self.release("v1")})

    def test_bad_payload_has_no_started_process_or_success_record(self):
        request = self.request()
        request["backends"]["services.compiler"] = base64.b64encode(b"different").decode()
        with self.assertRaisesRegex(ValueError, "copied executable differs"):
            self.deploy(request)
        self.assertFalse(self.runtime.processes.running)
        self.assertEqual(self.journal()["phase"], "reverted")

    def test_partial_candidate_start_is_cleaned_without_stopping_previous(self):
        self.deploy(self.request())
        self.runtime.processes.rejected.add("v2")
        with self.assertRaises(ValueError):
            self.deploy(self.request("v2", b"bad runtime"))
        self.assertEqual(set(self.runtime.processes.running), {self.release("v1")})
        self.assertEqual(self.runtime.processes.enabled, {self.release("v1")})
        self.assertEqual(self.journal()["phase"], "reverted")

    def test_route_failure_restores_old_process_and_stops_candidate(self):
        self.deploy(self.request())
        self.runtime.rejected_versions.add("v2")
        with self.assertRaises(ValueError):
            self.deploy(self.request("v2", b"binary two"))
        self.assertEqual(set(self.runtime.processes.running), {self.release("v1")})
        self.assertEqual(self.runtime.processes.enabled, {self.release("v1")})
        self.assertEqual(self.journal()["phase"], "reverted")

    def test_failed_cleanup_retains_operation_ownership(self):
        self.runtime.processes.rejected.add("v1")
        self.runtime.processes.reject_stop.add("v1")
        first = self.request()
        with self.assertRaises(ValueError):
            self.deploy(first)
        self.assertNotIn(self.journal()["phase"], ("complete", "reverted"))
        with self.assertRaisesRegex(ValueError, "another operation owns"):
            self.deploy(self.request("v2"))

        self.runtime.processes.rejected.clear()
        self.runtime.processes.reject_stop.clear()
        self.deploy(first)
        self.assertEqual(self.journal()["phase"], "complete")

    def test_same_version_different_contract_never_starts(self):
        self.deploy(self.request())
        request = self.request()
        request["host"]["executables"]["services.compiler"]["environment"]["FEATURE"] = "changed"
        request["operation"] = digest(canonical(request["host"]))
        before = list(self.runtime.processes.events)
        with self.assertRaisesRegex(ValueError, "process request differs"):
            self.deploy(request)
        self.assertEqual(before, self.runtime.processes.events)

    def test_changed_process_and_edge_after_prepare_are_not_reauthorized(self):
        request = self.request()
        self.machine.run(request)
        path = self.release("v1") / "process.json"
        contract = json.loads(path.read_bytes())
        contract["address"] = "127.0.0.1:9999"
        contract["environment"]["PLAY_BIND"] = contract["address"]
        atomic(path, canonical(contract))
        edge = self.root / "edge/releases" / self.journal()["transaction"] / "config.json"

        value = json.loads(edge.read_bytes())
        value["routes"][0]["upstreams"][0]["dial"] = contract["address"]
        atomic(edge, canonical(value), 0o644)
        with self.assertRaisesRegex(ValueError, "changed after preparation"):
            self.machine.run({**request, "action": "activate"})
        self.assertIsNone(pointer(self.root / "units/services.compiler/current"))

    def test_process_receipt_and_executable_modes_are_checked(self):
        request = self.request()
        self.machine.run(request)

        for name in ("backend", "process.json", "process-request.json"):
            path = self.release("v1") / name
            mode = path.stat().st_mode & 0o777
            path.chmod(0o644)
            with self.assertRaises(ValueError):
                self.machine.run({**request, "action": "activate"})
            path.chmod(mode)

    def test_docs_only_update_preserves_application_pid_contract_and_binding(self):
        first = self.request()
        self.deploy(first)
        before = copy.deepcopy(self.runtime.processes.running)
        events = list(self.runtime.processes.events)
        history = (self.root / "units/services.compiler/journal.json").read_bytes()
        docs = copy.deepcopy(first)
        docs["host"]["units"] = {"frontends.docs": {}}
        docs["host"]["executables"] = {}

        docs["host"]["routes"] = {}
        docs["version"] = "docs2"
        docs["operation"] = digest(canonical(docs["host"]))
        self.deploy(docs)
        self.assertEqual(before, self.runtime.processes.running)
        self.assertEqual(events, self.runtime.processes.events)
        self.assertEqual(history, (self.root / "units/services.compiler/journal.json").read_bytes())
        config = json.loads((self.root / "edge/current/config.json").read_bytes())
        self.assertEqual(config["routes"][0]["upstreams"][0]["dial"], before[self.release("v1")]["address"])

    def test_interrupted_promotion_or_retirement_restores_before_retry(self):
        # Every fault gets an independent root so a previous successful recovery
        # cannot hide stale instance state at a later transition.
        for point in ("installing", "installed:services.compiler", "candidates_started", "prepared",
                      "before_units", "switched:services.compiler", "before_edge", "after_edge",
                      "after_restart", "before_retirement", "after_retirement", "complete"):
            with self.subTest(point=point):
                root = self.root / point.replace(":", "-")
                root.mkdir()
                first, second = self.request(), self.request("v2", b"second")

                for request in (first, second):
                    request["host"]["root"] = str(root)
                    request["operation"] = digest(canonical([request["version"], request["host"]]))
                self.deploy(first)
                def fault(at):
                    if at == point:
                        raise Interrupted()
                machine = Machine(self.runtime, fault)
                with self.assertRaises(Interrupted):
                    machine.run(second)
                    machine.run({**second, "action": "activate"})
                journal = json.loads((root / "journal.json").read_bytes())

                if point != "complete":
                    self.assertNotEqual(journal["phase"], "complete")
                self.deploy(second)
                new = root / "units/services.compiler/releases/v2"
                old = root / "units/services.compiler/releases/v1"
                self.assertIn(new, self.runtime.processes.running)
                self.assertIn(new, self.runtime.processes.enabled)
                self.assertNotIn(old, self.runtime.processes.running)

                self.assertNotIn(old, self.runtime.processes.enabled)

    def test_one_service_update_and_rollback_preserve_other_instance_and_history(self):
        first = self.request()
        other = copy.deepcopy(first["host"]["executables"]["services.compiler"])
        other["address"] = "127.0.0.1:9002"
        other["environment"][other["address_env"]] = other["address"]
        first["host"]["executables"]["services.accounts"] = other
        first["host"]["units"]["services.accounts"] = {}
        first["host"]["routes"]["services.accounts"] = {"domain": "accounts.example", "mount": "/"}
        first["host"]["edge"]["routes"].append({"handler": "reverse_proxy", "upstreams": [{"dial": other["address"]}]})

        first["backends"]["services.accounts"] = first["backends"]["services.compiler"]
        first["operation"] = digest(canonical(first["host"]))
        self.deploy(first)
        process = self.release("v1", "services.accounts")
        account = copy.deepcopy(self.runtime.processes.running[process])
        history = (self.root / "units/services.accounts/journal.json").read_bytes()
        before_events = len(self.runtime.processes.events)
        second = self.request("v2", b"second compiler")
        second["host"]["edge"] = copy.deepcopy(first["host"]["edge"])
        second["operation"] = digest(canonical(second["host"]))
        self.deploy(second)
        rollback = copy.deepcopy(first)

        for key in ("executables", "units", "routes"):
            rollback["host"][key].pop("services.accounts")
        rollback["rollback"] = True
        rollback["operation"] = digest(canonical(rollback["host"]))
        self.deploy(rollback)
        self.assertEqual(self.runtime.processes.running[process], account)
        self.assertEqual((self.root / "units/services.accounts/journal.json").read_bytes(), history)

        self.assertFalse(any(path == process for _, path in self.runtime.processes.events[before_events:]))
        config = json.loads((self.root / "edge/current/config.json").read_bytes())
        self.assertEqual(config["routes"][1]["upstreams"][0]["dial"], account["address"])

    def test_old_contract_substitution_blocks_recovery_without_success(self):
        self.deploy(self.request())
        second = self.request("v2", b"new")
        def interrupt(point):
            if point == "after_retirement":
                raise Interrupted()
        with self.assertRaises(Interrupted):
            machine = Machine(self.runtime, interrupt)
            machine.run(second)
            machine.run({**second, "action": "activate"})
        path = self.release("v1") / "process.json"
        old = path.read_bytes()

        contract = json.loads(old)
        contract["address"] = "127.0.0.1:9999"
        contract["environment"]["PLAY_BIND"] = contract["address"]
        atomic(path, canonical(contract))
        with self.assertRaisesRegex(ValueError, "previous process contract changed"):
            self.machine.run(second)
        self.assertEqual(self.journal()["phase"], "retiring")
        atomic(path, old)
        self.deploy(second)
        self.assertEqual(self.journal()["phase"], "complete")

    def test_static_ssr_assets_and_backend_share_a_release(self):
        request = self.request(unit="frontends.site")
        data = b"asset"
        request["host"]["units"]["frontends.site"] = {
            "index.html": {"kind": "file", "mode": 0o644, "size": len(data), "sha256": digest(data)}}
        request["files"]["frontends.site/index.html"] = base64.b64encode(data).decode()
        self.deploy(request)
        self.assertEqual((self.release("v1", "frontends.site") / "assets/index.html").read_bytes(), data)
        self.assertIn(self.release("v1", "frontends.site"), self.runtime.processes.running)

    def test_edge_materialization_cannot_change_routes_or_tls_policy(self):
        template = self.request()["host"]["edge"]
        previous = copy.deepcopy(template)
        previous["routes"][0]["upstreams"][0]["dial"] = "127.0.0.1:49123"
        self.assertEqual(render_edge(template, previous, {}), previous)
        previous["routes"][0]["new_policy"] = True
        with self.assertRaisesRegex(ValueError, "changed beyond"):
            render_edge(template, previous, {})


if __name__ == "__main__":
    unittest.main()
