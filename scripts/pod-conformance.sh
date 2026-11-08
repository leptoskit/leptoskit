#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "==> Rust leptoskit_pod conformance"
CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}" cargo test -p leptoskit_pod conformance

echo "==> TypeScript @leptoskit/pod"
(
	cd packages/leptoskit-pod-ts
	if [[ ! -d node_modules ]]; then
		npm install --no-audit --no-fund
	fi
	npm test
)

echo "==> Go leptoskit-pod-go"
(
	cd packages/leptoskit-pod-go
	go test ./...
)

echo "==> Python leptoskit_pod"
(
	cd packages/leptoskit-pod-py
	if ! python3 -c 'import pytest' 2>/dev/null; then
		python3 -m pip install -q -e '.[dev]'
	fi
	python3 -m pytest -q
)

# Optional sibling runtimes (no-op when toolchain missing).
"$ROOT/scripts/pod-conformance-swift.sh"
"$ROOT/scripts/pod-conformance-kotlin.sh"
"$ROOT/scripts/pod-conformance-dart.sh"

echo "All pod conformance checks passed."
