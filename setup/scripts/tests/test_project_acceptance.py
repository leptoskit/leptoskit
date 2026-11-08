"""Exercise the real acceptance wrapper with controlled executable boundaries."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


REPO = Path(__file__).resolve().parents[3]
SCRIPT = REPO / "setup/scripts/project-acceptance.sh"
BASH = shutil.which("bash")
COMMAND_TIMEOUT_SECONDS = 15

TOOL = '''#!{python}
import json, os, sys
from pathlib import Path
name = Path(sys.argv[0]).name
args = sys.argv[1:]
if name == "rustc":
    print(os.environ["TEST_TARGET_LIBDIR"])
elif name == "node":
    sys.exit(int(os.environ.get("TEST_NODE_FAILURE", "0")))
elif name == "python3":
    sys.exit(int(os.environ.get("TEST_SQLITE_FAILURE", "0")))
elif name == "cargo" and args[:2] == ["nextest", "--version"]:
    sys.exit(int(os.environ.get("TEST_NEXTEST_FAILURE", "0")))
elif name == "cargo":
    log = Path(os.environ["TEST_CALLS"])
    calls = json.loads(log.read_text()) if log.exists() else []
    calls.append({{"args": args,
        "required": os.environ.get("LEPTOSKIT_REQUIRE_SDK_TOOLCHAINS"),
        "target": os.environ.get("LEPTOSKIT_E2E_TARGET_DIR"),
        "jobs": os.environ.get("CARGO_BUILD_JOBS")}})
    log.write_text(json.dumps(calls))
    missing = os.environ.get("TEST_MISSING_SELECTOR")
    if missing and "test(=" + missing + ")" in args:
        sys.exit(4 if "--no-tests=fail" in args else 0)
    if int(os.environ.get("TEST_FAIL_CALL", "0")) == len(calls):
        sys.exit(37)
'''


class ProjectAcceptance(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory(prefix="lk-project-gate-")
        self.addCleanup(self.scratch.cleanup)
        self.root = Path(self.scratch.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()

        for name in ["cargo", "rustc", "node", "wasm-opt", "python3"]:
            path = self.bin / name
            path.write_text(TOOL.format(python=sys.executable))
            path.chmod(0o755)

        for name in ["dirname", "mkdir"]:
            (self.bin / name).symlink_to(shutil.which(name))

        self.libdir = self.root / "wasm std"
        self.libdir.mkdir()
        (self.libdir / "libstd-fixture.rlib").touch()
        self.log = self.root / "calls.json"
        self.env = {
            **os.environ,
            "PATH": str(self.bin),
            "TEST_TARGET_LIBDIR": str(self.libdir),
            "TEST_CALLS": str(self.log),
            "CARGO_TARGET_DIR": str(self.root / "parent target"),
            "LEPTOSKIT_E2E_TARGET_DIR": str(self.root / "generated target"),
            "LEPTOSKIT_REQUIRE_SDK_TOOLCHAINS": "0",
            "CARGO_BUILD_JOBS": "2",
        }

    def run_gate(self, script=SCRIPT, **env):
        return subprocess.run(
            [BASH, str(script)], env={**self.env, **env},
            capture_output=True, text=True, timeout=COMMAND_TIMEOUT_SECONDS,
        )

    def calls(self):
        return json.loads(self.log.read_text()) if self.log.exists() else []

    def test_each_missing_tool_fails_before_running_a_journey(self):
        for tool in ["cargo", "rustc", "node", "wasm-opt", "python3"]:
            with self.subTest(tool=tool):
                path = self.bin / tool
                saved = path.read_bytes()
                path.unlink()

                result = self.run_gate()
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(f"requires {tool}", result.stderr)
                self.assertEqual(self.calls(), [])

                path.write_bytes(saved)
                path.chmod(0o755)

    def test_unusable_nextest_or_node_fails_before_builds(self):
        for setting in ["TEST_NEXTEST_FAILURE", "TEST_NODE_FAILURE", "TEST_SQLITE_FAILURE"]:
            with self.subTest(setting=setting):
                result = self.run_gate(**{setting: "12"})
                self.assertEqual(result.returncode, 12)
                self.assertEqual(self.calls(), [])

    def test_missing_wasm_standard_library_fails_before_builds(self):
        (self.libdir / "libstd-fixture.rlib").unlink()
        result = self.run_gate()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("wasm32-unknown-unknown", result.stderr)
        self.assertEqual(self.calls(), [])

    def test_all_required_selectors_run_serially_with_no_optional_skips(self):
        result = self.run_gate()
        self.assertEqual(result.returncode, 0, result.stderr)
        cases = self.calls()
        self.assertEqual(len(cases), 3)
        expected = [
            ("db", "the_database_scaffold_migrates_and_serves"),
            ("db", "the_auth_scaffold_signs_people_in"),
            ("universal", "generated_application_shares_behavior_and_selects_platform_overrides"),
        ]

        for call, (binary, name) in zip(cases, expected):
            args = call["args"]
            self.assertEqual(args[args.index("--test") + 1], binary)
            self.assertEqual(args[args.index("-E") + 1], f"test(={name})")
            self.assertIn("--no-tests=fail", args)
            self.assertIn("--ignore-default-filter", args)
            self.assertEqual(args[args.index("--run-ignored") + 1], "ignored-only")
            self.assertIn("--test-threads=1", args)

            self.assertEqual(call["required"], "1")
            self.assertEqual(call["jobs"], "2")
            self.assertEqual(Path(call["target"]), (self.root / "generated target").resolve())

    def test_failed_journey_prevents_later_journeys(self):
        result = self.run_gate(TEST_FAIL_CALL="2")
        self.assertEqual(result.returncode, 37)
        self.assertEqual(len(self.calls()), 2)

    def test_missing_single_selector_fails_even_if_other_cases_exist(self):
        result = self.run_gate(TEST_MISSING_SELECTOR="the_auth_scaffold_signs_people_in")
        self.assertEqual(result.returncode, 4)
        self.assertEqual(len(self.calls()), 2)

    def test_same_target_and_symlink_alias_are_rejected_before_builds(self):
        parent = self.root / "parent target"
        parent.mkdir()
        alias = self.root / "target alias"
        alias.symlink_to(parent, target_is_directory=True)

        for path in [parent, alias]:
            with self.subTest(path=path):
                result = self.run_gate(LEPTOSKIT_E2E_TARGET_DIR=str(path))
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("distinct", result.stderr)
                self.assertEqual(self.calls(), [])


if __name__ == "__main__":
    unittest.main()
