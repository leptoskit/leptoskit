#!/usr/bin/env bash
# One command for local checks, pull requests, and publication. Every SDK
# must run its real live-server journey; selecting no tests is a failure.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

for tool in cargo deno node uv dart go; do
	if ! command -v "$tool" >/dev/null 2>&1; then
		echo "SDK acceptance requires $tool; run through devenv shell test:sdk" >&2
		exit 1
	fi
done
# Node needs native TypeScript stripping and its built-in WebSocket.
node -e 'if (Number(process.versions.node.split(".")[0]) < 22) { throw new Error("SDK acceptance requires Node 22 or newer"); }'

# The generated Rust client and CLI share their own cache, outside the
# parent Cargo target: Cargo must not wait on a lock held by its test runner.
export LEPTOSKIT_SDK_TARGET_DIR="${LEPTOSKIT_SDK_TARGET_DIR:-$repo_root/target/sdk_live}"
mkdir -p "$LEPTOSKIT_SDK_TARGET_DIR"
LEPTOSKIT_SDK_TARGET_DIR="$(cd "$LEPTOSKIT_SDK_TARGET_DIR" && pwd)"
export LEPTOSKIT_SDK_TARGET_DIR

cargo nextest run --locked --package leptoskit_rpc --all-features \
	--test sdk_live --run-ignored all --no-tests=fail --test-threads=1

cargo nextest run --locked -p leptoskit_rpc_codegen --features generators \
	--test dart_presence --run-ignored all --no-tests=fail --test-threads=1
