"""Literal environment contracts, encoding and immutable release consumption.

These tests do not execute systemd; real Linux parser/process acceptance is a
separate required gate. Transaction tests use the production worker with a
deterministic process adapter.
"""
import copy
import json
import pathlib
import unittest

import test_instances
from host import canonical, digest
from process import Processes, environment_line, environment_valid


class Environment(unittest.TestCase):
    def test_literal_encodings_do_not_expand_specifiers_or_shell_variables(self):
        cases = {
            "": 'Environment="VALUE="\n',
            "hello world": 'Environment="VALUE=hello world"\n',
            'a"b\\c': 'Environment="VALUE=a\\"b\\\\c"\n',
            "%n %% %u": 'Environment="VALUE=%%n %%%% %%u"\n',
            "$HOME ${NAME} $(echo nope)": 'Environment="VALUE=$HOME ${NAME} $(echo nope)"\n',
            "世界 🌱": 'Environment="VALUE=世界 🌱"\n',
            "'quoted'=value": 'Environment="VALUE=\'quoted\'=value"\n',
        }

        for value, expected in cases.items():
            with self.subTest(value=value):
                environment_valid({"VALUE": value})
                self.assertEqual(environment_line("VALUE", value), expected)

    def test_legacy_simple_lines_remain_byte_identical(self):
        for value in ("127.0.0.1:9001", "/srv/app", "production", "https://one,https://two"):
            self.assertEqual(environment_line("VALUE", value), f"Environment=VALUE={value}\n")

    def test_invalid_values_and_names_fail_without_value_disclosure(self):
        for value in ("secret\nUser=root", "\x00", "\x7f", "\x85", "\ud800", "\ufeff", "\ufdd0", "\U0010ffff"):
            with self.subTest(code=repr(value)):
                with self.assertRaisesRegex(ValueError, "printable Unicode") as error:
                    environment_valid({"VALUE": value})
                self.assertNotIn(value, str(error.exception))

        for values in ({"BAD-NAME": "x"}, {"lower": "x"}, {"1NAME": "x"}, {"N" * 129: "x"}, []):
            with self.assertRaises(ValueError):
                environment_valid(values)

    def test_byte_and_entry_limits_are_enforced(self):
        environment_valid({"VALUE": "🌱" * 2048})

        for values in ({"VALUE": "🌱" * 2049}, {f"V{i}": "" for i in range(129)},
                       {f"V{i}": "x" * 8192 for i in range(5)}):
            with self.assertRaises(ValueError):
                environment_valid(values)


class EnvironmentLifecycle(unittest.TestCase):
    def setUp(self):
        self.fixture = test_instances.ExecutableReleases()
        self.fixture.setUp()
        self.addCleanup(self.fixture.tearDown)

    def request(self, version, label):
        request = self.fixture.request(version)
        request["host"]["executables"]["services.compiler"]["environment"].update(
            {"LABEL": label, "EMPTY": ""})
        request["operation"] = digest(canonical([version, request["host"]]))

        return request

    def test_settings_survive_port_allocation_update_and_rollback(self):
        first = self.request("v1", "first 世界 %n $HOME")
        second = self.request("v2", "second")
        self.fixture.deploy(first)
        self.fixture.deploy(second)
        self.fixture.deploy({**first, "rollback": True})
        release = self.fixture.release("v1")
        contract = self.fixture.runtime.processes.running[release]
        self.assertEqual(contract["environment"]["LABEL"], "first 世界 %n $HOME")
        self.assertEqual(contract["environment"]["EMPTY"], "")
        self.assertEqual(contract["environment"]["PLAY_BIND"], contract["address"])

        unit = Processes().unit(pathlib.Path("/srv/release"), contract).decode()
        self.assertIn('Environment="LABEL=first 世界 %%n $HOME"\n', unit)
        self.assertIn('Environment="EMPTY="\n', unit)

    def test_substituted_installed_settings_cannot_activate(self):
        request = self.request("v1", "pinned")
        self.fixture.machine.run(request)
        path = self.fixture.release("v1") / "process.json"
        contract = json.loads(path.read_bytes())
        contract["environment"]["LABEL"] = "changed"
        path.write_bytes(canonical(contract))

        with self.assertRaisesRegex(ValueError, "installed process configuration differs"):
            self.fixture.machine.run({**request, "action": "activate"})

    def test_consumption_independently_rechecks_installed_settings(self):
        request = self.request("v1", "pinned")
        self.fixture.machine.run(request)
        release = self.fixture.release("v1")
        path = release / "process.json"
        contract = json.loads(path.read_bytes())
        contract["environment"]["LABEL"] = "changed"
        path.write_bytes(canonical(contract))
        requested = request["host"]["executables"]["services.compiler"]

        with self.assertRaisesRegex(ValueError, "installed process configuration differs"):
            self.fixture.machine.instances.check(release, requested)

    def test_altered_request_cannot_clean_up_prepared_settings(self):
        request = self.request("v1", "pinned")
        self.fixture.machine.run(request)
        journal = (self.fixture.root / "journal.json").read_bytes()
        running = copy.deepcopy(self.fixture.runtime.processes.running)
        changed = copy.deepcopy(request)
        changed["host"]["executables"]["services.compiler"]["environment"]["LABEL"] = "changed"

        with self.assertRaisesRegex(ValueError, "operation context differs"):
            self.fixture.machine.run(changed)
        self.assertEqual((self.fixture.root / "journal.json").read_bytes(), journal)
        self.assertEqual(self.fixture.runtime.processes.running, running)
