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
	# devenv's python has no pip; prefer uv when present.
	if command -v uv >/dev/null 2>&1; then
		uv run --with pytest --with-editable . pytest -q
	elif python3 -c 'import pytest' 2>/dev/null; then
		python3 -m pytest -q
	else
		python3 -m pip install -q -e '.[dev]'
		python3 -m pytest -q
	fi
)

# Optional sibling runtimes (no-op when toolchain missing).
"$ROOT/scripts/pod-conformance-swift.sh"
"$ROOT/scripts/pod-conformance-kotlin.sh"
"$ROOT/scripts/pod-conformance-dart.sh"

echo "All pod conformance checks passed."
