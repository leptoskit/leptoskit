#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if ! command -v cargo-kani >/dev/null 2>&1 && ! cargo kani --version >/dev/null 2>&1; then
	echo "Skip: cargo-kani not installed (see https://model-checking.github.io/kani/install-kani.html)"
	exit 0
fi

echo "==> Kani proofs: leptoskit_pod"
CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}" cargo kani -p leptoskit_pod
