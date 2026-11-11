"""Acceptance-harness regressions; these tests do not run Cargo builds."""

import importlib.util
import io
import json
from pathlib import Path
import sys
import tarfile
import tempfile
import tomllib
import unittest
from unittest.mock import patch

from model import PRESETS, inspect_graph, write_consumer
from packages import load_packages


class Packages(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.archives = self.root / "archives"
        self.archives.mkdir()
        self.metadata = {"packages": [{"name": "leptoskit", "version": "1.0.0", "publish": None}]}

    def archive(self, entries=None, revision="commit", version="1.0.0"):
        files = {
            "leptoskit-1.0.0/Cargo.toml": f'[package]\nname="leptoskit"\nversion="{version}"\n',
            "leptoskit-1.0.0/.cargo_vcs_info.json": json.dumps({"git": {"sha1": revision}}),
            "leptoskit-1.0.0/src/lib.rs": "pub fn exported() {}",
        }
        files.update(entries or {})
        with tarfile.open(self.archives / "leptoskit-1.0.0.crate", "w:gz") as archive:
            for name, content in files.items():
                entry = tarfile.TarInfo(name)
                data = content.encode()
                entry.size = len(data)
                archive.addfile(entry, io.BytesIO(data))

    def load(self):
        return load_packages(self.metadata, self.archives, self.root / "payloads", "commit")

    def test_extracts_package_identity_and_hash(self):
        self.archive()
        packages = self.load()
        self.assertEqual(packages["leptoskit"]["version"], "1.0.0")
        self.assertEqual(len(packages["leptoskit"]["sha256"]), 64)
        self.assertTrue((packages["leptoskit"]["path"] / "src/lib.rs").is_file())

    def test_missing_archive_cannot_fall_back_to_registry(self):
        with self.assertRaisesRegex(ValueError, "missing package archive"):
            self.load()

    def test_version_mismatch_is_rejected(self):
        self.archive(version="0.9.0")
        with self.assertRaisesRegex(ValueError, "identity mismatch"):
            self.load()

    def test_stale_revision_is_rejected(self):
        self.archive(revision="previous-commit")
        with self.assertRaisesRegex(ValueError, "different source revision"):
            self.load()

    def test_archive_member_cannot_escape_output(self):
        self.archive({"leptoskit-1.0.0/../../outside": "bad"})
        with self.assertRaisesRegex(ValueError, "unsupported archive member"):
            self.load()
        self.assertFalse((self.root / "outside").exists())

    def test_case_aliases_are_rejected_on_every_filesystem(self):
        self.archive({"leptoskit-1.0.0/README.md": "upper", "leptoskit-1.0.0/readme.md": "lower"})
        with self.assertRaisesRegex(ValueError, "case-insensitive path collision"):
            self.load()

    def test_directory_case_aliases_are_rejected(self):
        self.archive({"leptoskit-1.0.0/Src/other.rs": "pub fn other() {}"})
        with self.assertRaisesRegex(ValueError, "case-insensitive path collision"):
            self.load()

    def test_no_publishable_facade_fails(self):
        self.metadata["packages"][0]["publish"] = []
        with self.assertRaisesRegex(ValueError, "no publishable facade"):
            self.load()

    def test_each_consumer_has_its_own_workspace_and_explicit_features(self):
        self.archive()
        packages = self.load()
        for preset, (features, _) in PRESETS.items():
            directory = self.root / preset
            write_consumer(directory, preset, packages)
            manifest = tomllib.loads((directory / "Cargo.toml").read_text())
            self.assertEqual(manifest["workspace"], {"resolver": "2"})
            dependency = manifest["dependencies"]["leptoskit"]
            self.assertEqual(dependency["features"], features)
            self.assertFalse(dependency["default-features"])
            self.assertNotIn("path", dependency)
            self.assertFalse((directory / "Cargo.lock").exists())
            self.assertEqual(
                manifest["patch"]["crates-io"]["leptoskit"]["path"],
                str(packages["leptoskit"]["path"]),
            )


class Graph(unittest.TestCase):
    def setUp(self):
        self.path = Path("/fixture/payloads/leptoskit-1.0.0")
        self.packages = {"leptoskit": {"version": "1.0.0", "path": self.path}}
        self.metadata = {
            "packages": [{"id": "kit", "name": "leptoskit", "version": "1.0.0",
                          "source": None, "manifest_path": str(self.path / "Cargo.toml")}],
            "resolve": {"nodes": [{"id": "kit", "features": []}]},
        }

    def test_exact_payload_resolves(self):
        self.assertEqual(inspect_graph(self.metadata, self.packages, "minimal-host"), {"leptoskit": []})

    def test_registry_and_repository_fallbacks_fail(self):
        for field, value in [("source", "registry+https://index.crates.io/"),
                             ("manifest_path", "/repository/crates/leptoskit/Cargo.toml")]:
            with self.subTest(field=field):
                original = self.metadata["packages"][0][field]
                self.metadata["packages"][0][field] = value
                with self.assertRaisesRegex(ValueError, "did not resolve"):
                    inspect_graph(self.metadata, self.packages, "minimal-host")
                self.metadata["packages"][0][field] = original

    def test_server_feature_leak_is_rejected(self):
        self.metadata["resolve"]["nodes"][0]["features"] = ["server"]
        with self.assertRaisesRegex(ValueError, "server features leaked"):
            inspect_graph(self.metadata, self.packages, "minimal-host")

    def test_missing_requested_features_cannot_pass(self):
        with self.assertRaisesRegex(ValueError, "requested facade features"):
            inspect_graph(self.metadata, self.packages, "native-guest")

    def test_empty_graph_cannot_pass(self):
        self.metadata["packages"] = []
        with self.assertRaisesRegex(ValueError, "no facade"):
            inspect_graph(self.metadata, self.packages, "minimal-host")


class Orchestration(unittest.TestCase):
    def runner(self):
        script = Path(__file__).resolve().parents[1] / "packaged-consumers.py"
        spec = importlib.util.spec_from_file_location("consumer_runner", script)
        runner = importlib.util.module_from_spec(spec)
        with patch.object(sys, "path", [str(script.parent), *sys.path]):
            spec.loader.exec_module(runner)
        return script, runner

    def test_a_failed_consumer_never_writes_success_evidence(self):
        script, runner = self.runner()
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "fresh-output"
            results = iter(["nightly (override)", "wasm32-unknown-unknown", "commit", '{"packages": []}'])
            arguments = [str(script), "--archives", temporary, "--output", str(output)]
            partial = [{"preset": name, "state": "passed"} for name in list(PRESETS)[:-1]]
            with patch("sys.argv", arguments), patch.object(runner.shutil, "which", return_value="tool"), \
                    patch.object(runner, "capture", side_effect=lambda *args: next(results)), \
                    patch.object(runner, "load_packages", return_value={}), \
                    patch.object(runner, "compile_consumer", side_effect=[*partial, ValueError("compile failed")]):
                with self.assertRaisesRegex(ValueError, "compile failed"):
                    runner.main()
            self.assertFalse((output / "acceptance.json").exists())

    def test_missing_tool_cannot_be_skipped(self):
        script, runner = self.runner()
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "output"
            arguments = [str(script), "--archives", temporary, "--output", str(output)]
            for missing in ["cargo", "rustc", "rustup"]:
                with patch("sys.argv", arguments), patch.object(
                    runner.shutil, "which", side_effect=lambda tool: None if tool == missing else "tool"
                ):
                    with self.assertRaisesRegex(ValueError, f"required tool missing: {missing}"):
                        runner.main()
                self.assertFalse(output.exists())

    def test_missing_wasm_target_cannot_pass_host_only(self):
        script, runner = self.runner()
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "output"
            arguments = [str(script), "--archives", temporary, "--output", str(output)]
            with patch("sys.argv", arguments), patch.object(runner.shutil, "which", return_value="tool"), \
                    patch.object(runner, "capture", side_effect=["nightly", "aarch64-apple-darwin"]):
                with self.assertRaisesRegex(ValueError, "required target missing"):
                    runner.main()
            self.assertFalse(output.exists())

    def test_all_supported_consumers_are_mandatory(self):
        self.assertEqual(set(PRESETS), {
            "minimal-host", "browser-csr", "browser-hydrate", "native-guest",
            "server-sqlite", "server-postgres", "auth-sqlite", "auth-postgres",
        })


class PublicationReport(unittest.TestCase):
    def setUp(self):
        from packages import sha256
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.archive = self.root / "leptoskit-1.0.0.crate"
        self.archive.write_bytes(b"tested package bytes")
        self.metadata = {"packages": [{"name": "leptoskit", "version": "1.0.0", "publish": None}]}
        self.report = {
            "format_version": 1, "state": "passed", "source_revision": "commit",
            "packages": {"leptoskit": {"version": "1.0.0", "sha256": sha256(self.archive)}},
            "consumers": [{"preset": name, "state": "passed", "features": features,
                           "target": target or "host"} for name, (features, target) in PRESETS.items()],
        }

    def verify(self):
        from packages import verify_report
        verify_report(self.report, self.metadata, self.root, "commit", PRESETS)

    def test_exact_tested_archives_pass(self):
        self.verify()

    def test_changed_archive_fails(self):
        self.archive.write_bytes(b"different prepared upload")
        with self.assertRaisesRegex(ValueError, "differs from the tested payload"):
            self.verify()

    def test_different_revision_fails(self):
        self.report["source_revision"] = "different"
        with self.assertRaisesRegex(ValueError, "source revision"):
            self.verify()

    def test_partial_or_repeated_profiles_fail(self):
        original = self.report["consumers"]
        for consumers in [original[:-1], [original[0]] * len(original)]:
            self.report["consumers"] = consumers
            with self.assertRaisesRegex(ValueError, "required profiles"):
                self.verify()

    def test_wrong_features_or_target_or_failed_profile_fails(self):
        original = self.report["consumers"][0].copy()
        for field, value in [("features", ["server"]), ("target", "different"), ("state", "failed")]:
            self.report["consumers"][0] = {**original, field: value}
            with self.assertRaisesRegex(ValueError, "incomplete evidence"):
                self.verify()

    def test_missing_package_fails(self):
        self.report["packages"] = {}
        with self.assertRaisesRegex(ValueError, "package set"):
            self.verify()

    def test_untested_package_fails(self):
        self.report["packages"]["leptoskit_extra"] = {"version": "1.0.0", "sha256": "0" * 64}
        with self.assertRaisesRegex(ValueError, "package set"):
            self.verify()


if __name__ == "__main__":
    unittest.main()
