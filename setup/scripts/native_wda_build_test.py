"""Exercise the real WDA helper with deterministic Xcode/CoreSimulator commands."""

import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


DEVICE = "D9DC1465-992F-41FE-A718-70CD3A9EFF23"
SCRIPT = Path(
    os.environ.get("WDA_BUILD_SCRIPT", Path(__file__).with_name("native_wda_build.sh"))
).resolve()
COMMAND = r'''
import json, os, sys
from pathlib import Path
device = os.environ["TEST_DEVICE"]
args = sys.argv[1:]
if Path(sys.argv[0]).name == "xcrun":
    if args[:3] == ["simctl", "list", "devices"]:
        print(json.dumps({"devices": {"runtime-ios": [{"udid": device}]}}))
    elif args[:3] == ["simctl", "list", "runtimes"]:
        print(json.dumps({"runtimes": [{"identifier": "runtime-ios", "version": "26.5"}]}))
    else:
        raise SystemExit("unexpected xcrun command: " + repr(args))
    raise SystemExit(0)
state_file = Path(os.environ["TEST_STATE"])
state = json.loads(state_file.read_text()) if state_file.exists() else {"probes": 0, "builds": []}
scenario = os.environ["TEST_SCENARIO"]
if args == ["-version"]:
    print("Xcode 26.6")
elif "-showdestinations" in args:
    state["probes"] += 1
    if scenario == "diagnostic":
        print("error: could not resolve id:" + device + ", for this scheme")
    else:
        print('Available destinations for the "WebDriverAgentRunner" scheme:')
        if scenario == "ineligible":
            print("{ platform:iOS Simulator, id:placeholder, name:Any iOS Simulator Device }")
            print('Ineligible destinations for the "WebDriverAgentRunner" scheme:')
        platform = "macOS" if scenario == "mac" else "iOS Simulator"
        identifier = device + "-other" if scenario == "prefix" else device
        error = ", error:Runtime not ready" if scenario == "ineligible" else ""
        print("{ platform:" + platform + ", arch:arm64, id:" + identifier + ", name:CI" + error + " }")
elif args[0] == "build-for-testing":
    state["builds"].append(args)
    state_file.write_text(json.dumps(state))
    if scenario == "compile":
        print("error: undefined symbols for architecture arm64")
        raise SystemExit(65)
    if scenario == "persistent" or (scenario == "transient" and len(state["builds"]) == 1):
        print("xcodebuild: error: Unable to find a device matching the provided destination specifier:")
        raise SystemExit(70)
    print("** TEST BUILD SUCCEEDED **")
else:
    raise SystemExit("unexpected xcodebuild command: " + repr(args))
state_file.write_text(json.dumps(state))
'''


class DestinationTests(unittest.TestCase):
    def run_case(self, scenario, timeout=0):
        with tempfile.TemporaryDirectory(prefix="wda-readiness-") as temporary:
            root = Path(temporary)
            project = (
                root / "setup/native-ui/node_modules/appium-xcuitest-driver"
                / "node_modules/appium-webdriveragent/WebDriverAgent.xcodeproj"
            )
            project.mkdir(parents=True)
            tools = root / "bin"
            tools.mkdir()
            for name in ("xcrun", "xcodebuild"):
                command = tools / name
                command.write_text("#!" + sys.executable + "\n" + COMMAND)
                command.chmod(0o755)
            sleep = tools / "sleep"
            sleep.write_text("#!/bin/sh\nexit 0\n")
            sleep.chmod(0o755)
            state = root / "state.json"
            env = dict(
                os.environ,
                PATH=str(tools) + os.pathsep + os.environ["PATH"],
                TEST_DEVICE=DEVICE,
                TEST_STATE=str(state),
                TEST_SCENARIO=scenario,
                NATIVE_WDA_DESTINATION_TIMEOUT=str(timeout),
            )
            result = subprocess.run(
                ["bash", str(SCRIPT), DEVICE, str(root / "derived data")],
                cwd=root, env=env, capture_output=True, text=True, timeout=5,
            )
            return result, json.loads(state.read_text())

    def test_available_simulator_preserves_build_arguments(self):
        result, state = self.run_case("ready")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(state["builds"]), 1)
        args = state["builds"][0]
        self.assertEqual(args[args.index("-destination") + 1], "id=" + DEVICE)
        self.assertIn("IPHONEOS_DEPLOYMENT_TARGET=26.5", args)
        self.assertIn("COMPILER_INDEX_STORE_ENABLE=NO", args)

    def test_ineligible_udid_never_starts_a_build(self):
        result, state = self.run_case("ineligible")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(state["builds"], [])

    def test_mac_destination_never_starts_a_simulator_build(self):
        result, state = self.run_case("mac")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(state["builds"], [])

    def test_diagnostic_mention_is_not_a_destination(self):
        result, state = self.run_case("diagnostic")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(state["builds"], [])

    def test_udid_prefix_does_not_match_another_device(self):
        result, state = self.run_case("prefix")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(state["builds"], [])

    def test_destination_lost_after_readiness_is_rechecked(self):
        result, state = self.run_case("transient", timeout=10)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(len(state["builds"]), 2)
        self.assertEqual(state["probes"], 2)

    def test_persistent_destination_failure_obeys_deadline(self):
        result, state = self.run_case("persistent")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(len(state["builds"]), 1)
        self.assertIn("could not use simulator", result.stderr)

    def test_compiler_failure_is_not_retried(self):
        result, state = self.run_case("compile", timeout=10)
        self.assertEqual(result.returncode, 65)
        self.assertEqual(len(state["builds"]), 1)
        self.assertEqual(state["probes"], 1)


if __name__ == "__main__":
    unittest.main()
