#!/usr/bin/env bash
# Run every required scaffold separately: one missing selector must fail even
# when the other journeys still exist. No optional language-toolchain skips.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

for tool in cargo rustc node wasm-opt; do
	if ! command -v "$tool" >/dev/null 2>&1; then
		echo "Project acceptance requires $tool; run through devenv shell test:projects" >&2
		exit 1
	fi
done

cargo nextest --version >/dev/null
wasm-opt --version >/dev/null
node --experimental-strip-types --no-warnings -e '
if (Number(process.versions.node.split(".")[0]) < 22 || typeof WebSocket !== "function") {
  throw new Error("Project acceptance requires Node 22+ with WebSocket and TypeScript stripping");
}'
target_libdir="$(rustc --print target-libdir --target wasm32-unknown-unknown)"
if ! compgen -G "$target_libdir/libstd-*.rlib" >/dev/null; then
	echo "Project acceptance requires wasm32-unknown-unknown for the active Rust toolchain" >&2
	exit 1
fi

# Generated apps compile inside tests. Their Cargo lock must be distinct
# from the parent test build, and their reusable cache must not depend on
# the temporary fixture directories.
parent_target="${CARGO_TARGET_DIR:-$repo_root/target}"
mkdir -p "$parent_target"
parent_target="$(cd "$parent_target" && pwd -P)"
export LEPTOSKIT_E2E_TARGET_DIR="${LEPTOSKIT_E2E_TARGET_DIR:-$parent_target/generated-projects}"
mkdir -p "$LEPTOSKIT_E2E_TARGET_DIR"
LEPTOSKIT_E2E_TARGET_DIR="$(cd "$LEPTOSKIT_E2E_TARGET_DIR" && pwd -P)"
export LEPTOSKIT_E2E_TARGET_DIR
if [[ "$LEPTOSKIT_E2E_TARGET_DIR" == "$parent_target" ]]; then
	echo "Generated projects need a Cargo target directory distinct from the parent test build" >&2
	exit 1
fi

export LEPTOSKIT_REQUIRE_SDK_TOOLCHAINS=1
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}"

journey() {
	cargo nextest run --locked --package leptoskit_cli --test "$1" \
		--run-ignored ignored-only --no-tests=fail --test-threads=1 \
		--ignore-default-filter -E "test(=$2)"
}

journey db the_database_scaffold_migrates_and_serves
journey db the_auth_scaffold_signs_people_in
journey universal generated_application_shares_behavior_and_selects_platform_overrides
