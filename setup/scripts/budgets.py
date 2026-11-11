#!/usr/bin/env python3
"""Caps and boot, the blocking checks that fail on a count.

Exact's six checks are build, test, lint, caps, boot, and workspace. This
script runs caps and boot. They count, so a timer cannot flake them. build,
test, lint, and workspace stay the CI jobs named below; this script only
checks those jobs are still wired, because a workspace compile does not
finish in Exact's 65 second gate.
"""

from __future__ import annotations

import re
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CHECKS = ("build", "test", "lint", "caps", "boot", "workspace")
SHELLS = (
    "crates/leptoskit_cli/src/new_project/templates/app.rs",
    "crates/leptoskit_cli/src/new_project/templates/app_auth.rs",
    "crates/leptoskit_cli/src/new_project/templates/app_db.rs",
)


def main() -> int:
    started = time.perf_counter()
    problems: list[str] = []
    problems.extend(caps())
    problems.extend(boot())
    problems.extend(wired())
    elapsed_ms = (time.perf_counter() - started) * 1000
    for name in CHECKS:
        if name in ("caps", "boot"):
            state = "failed" if any(item.startswith(f"{name}:") for item in problems) else "ok"
        else:
            state = "failed" if any(item.startswith(f"{name}:") for item in problems) else "wired"
        print(f"{name}\t{state}")
    print(f"cold_start_budget_ms\t100")
    print(f"edit_to_present_budget_ms\t500")
    print(f"crate_rebuild_budget_s\t30")
    print(f"gate_ms\t{elapsed_ms:.1f}")
    for problem in problems:
        print(f"problem\t{problem}", file=sys.stderr)
    return 1 if problems else 0


def caps() -> list[str]:
    problems: list[str] = []
    budgets = read("crates/leptoskit_cli/src/budgets.rs")
    wasm = read("crates/leptoskit_cli/src/wasm.rs")
    checks = re.search(
        r"pub const CHECKS: \[&str; 6\] = \[\s*"
        r'"build",\s*"test",\s*"lint",\s*"caps",\s*"boot",\s*"workspace",?\s*'
        r"\];",
        budgets,
    )
    if checks is None:
        problems.append(
            "caps: CHECKS is not build, test, lint, caps, boot, workspace"
        )
    for line in (
        "pub const COLD_START: Duration = Duration::from_millis(100);",
        "pub const EDIT_TO_PRESENT: Duration = Duration::from_millis(500);",
        "pub const CRATE_REBUILD: Duration = Duration::from_secs(30);",
    ):
        if line not in budgets:
            problems.append(f"caps: missing `{line}`")
    body = function_body(wasm, "dev_profile_config")
    if body is None:
        problems.append("caps: dev_profile_config is missing")
        return problems
    for banned in ("lto", "codegen-units"):
        if banned in body:
            problems.append(f"caps: the dev profile contains {banned}")
    for required in (
        'inherits = \\"dev\\"',
        'debug = \\"line-tables-only\\"',
        "incremental = true",
    ):
        if required not in body:
            problems.append(f"caps: the dev profile is missing {required}")
    return problems


def boot() -> list[str]:
    problems: list[str] = []
    source = read("crates/leptoskit_cli/src/new_project.rs")
    body = function_body(source, "index_html")
    if body is None:
        problems.append("boot: index_html is missing")
    else:
        scripts = body.count("<script")
        if scripts != 1:
            problems.append(
                f"boot: web/index.html has {scripts} script tags; the budget is 1"
            )
        if 'type="module"' not in body:
            problems.append("boot: the csr script is not type=module")
        if "src=" in body.split("fn ", 1)[0]:
            problems.append("boot: the csr document loads a script by src")
    for path in SHELLS:
        shell = read(path)
        scripts = shell.count("<script")
        hydration = shell.count("HydrationScripts")
        if scripts != 0:
            problems.append(
                f"boot: {path} has {scripts} <script tags; the budget is 0"
            )
        if hydration != 1:
            problems.append(
                f"boot: {path} has {hydration} HydrationScripts; the budget is 1"
            )
    return problems


def wired() -> list[str]:
    problems: list[str] = []
    ci = read(".github/workflows/ci.yml")
    clippy = read("setup/scripts/clippy.sh")
    if "run: lint:all" not in ci:
        problems.append("lint: CI no longer runs lint:all")
    if "cargo test" not in ci:
        problems.append("test: CI no longer runs cargo test")
    if "--workspace --all-features --all-targets" not in clippy:
        problems.append("build: clippy.sh no longer checks the workspace")
        problems.append("workspace: clippy.sh no longer checks the workspace")
    return problems


def read(path: str) -> str:
    file = ROOT / path
    if not file.is_file():
        return ""
    return file.read_text()


def function_body(source: str, name: str) -> str | None:
    match = re.search(rf"fn {name}\b[^{{]*\{{", source)
    if match is None:
        return None
    start = match.end()
    depth = 1
    for index, character in enumerate(source[start:], start):
        if character == "{":
            depth += 1
        elif character == "}":
            depth -= 1
            if depth == 0:
                return source[start:index]
    return None


if __name__ == "__main__":
    sys.exit(main())
