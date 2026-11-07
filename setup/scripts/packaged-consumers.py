#!/usr/bin/env python3
"""Compile all supported consumer feature sets using only Cargo package payloads."""

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

from consumer_acceptance.model import PRESETS, inspect_graph, write_consumer
from consumer_acceptance.packages import load_packages, sha256


def capture(arguments, directory, environment):
    return subprocess.check_output(arguments, cwd=directory, env=environment, text=True).strip()


def run_logged(arguments, directory, environment, path):
    with path.open("w") as log:
        subprocess.run(arguments, cwd=directory, env=environment, stdout=log, stderr=log, check=True)


def resolve_consumer(directory, environment):
    metadata = directory / "metadata.json"
    with metadata.open("w") as output, (directory / "metadata.log").open("w") as log:
        subprocess.run(
            ["cargo", "metadata", "--locked", "--format-version", "1"],
            cwd=directory, env=environment, stdout=output, stderr=log, check=True,
        )
    return json.loads(metadata.read_text())


def compile_consumer(preset, packages, output, environment):
    directory = output / preset
    write_consumer(directory, preset, packages)
    run_logged(["cargo", "generate-lockfile"], directory, environment, directory / "resolve.log")
    metadata = resolve_consumer(directory, environment)
    graph = inspect_graph(metadata, packages, preset)
    _, target = PRESETS[preset]
    arguments = ["cargo", "check", "--locked", "--lib"]
    if target:
        arguments += ["--target", target]
    run_logged(arguments, directory, environment, directory / "check.log")
    return {
        "preset": preset, "target": target or "host", "state": "passed",
        "features": PRESETS[preset][0], "resolved_features": graph,
        "lock_sha256": sha256(directory / "Cargo.lock"),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archives", type=Path, required=True, help="cargo package output directory")
    parser.add_argument("--output", type=Path, required=True, help="new directory outside the checkout")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[2]
    output = args.output.resolve()
    if output.is_relative_to(root):
        raise ValueError("consumer output must be outside the repository")
    for tool in ["cargo", "rustc", "rustup"]:
        if shutil.which(tool) is None:
            raise ValueError(f"required tool missing: {tool}")
    environment = os.environ.copy()
    for key in ["RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_BUILD_RUSTFLAGS"]:
        environment.pop(key, None)
    environment["RUSTUP_TOOLCHAIN"] = capture(
        ["rustup", "show", "active-toolchain"], root, environment
    ).split()[0]
    targets = capture(["rustup", "target", "list", "--installed"], root, environment).splitlines()
    if "wasm32-unknown-unknown" not in targets:
        raise ValueError("required target missing: wasm32-unknown-unknown")
    # Reusing a previous directory could leave stale successful evidence.
    output.mkdir(parents=True, exist_ok=False)
    environment["CARGO_TARGET_DIR"] = str(output / "target")
    revision = capture(["git", "rev-parse", "HEAD"], root, environment)
    metadata = json.loads(capture(
        ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"], root, environment
    ))
    packages = load_packages(metadata, args.archives.resolve(), output / "packages", revision)
    results = []
    for preset in PRESETS:
        print(f"Checking packaged consumer: {preset}", flush=True)
        results.append(compile_consumer(preset, packages, output, environment))
    report = {
        "format_version": 1, "state": "passed", "source_revision": revision,
        "toolchain": capture(["rustc", "--version", "--verbose"], root, environment),
        "packages": {name: {key: value for key, value in package.items() if key != "path"}
                     for name, package in packages.items()},
        "consumers": results,
        "scope": "packaged source compilation and feature isolation; not registry delivery or runtime behavior",
    }
    (output / "acceptance.json").write_text(json.dumps(report, indent=2) + "\n")
    print(f"All {len(results)} packaged consumers passed; evidence: {output / 'acceptance.json'}")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, KeyError, subprocess.CalledProcessError) as error:
        print(f"Packaged consumer acceptance failed: {error}", file=sys.stderr)
        sys.exit(1)
