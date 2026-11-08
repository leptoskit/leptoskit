#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PKG="$ROOT/packages/leptoskit-pod-swift"

if ! command -v swift >/dev/null 2>&1; then
	echo "SKIP: swift not found — install Swift 5.9+ to run leptoskit-pod-swift conformance"
	exit 0
fi

cd "$PKG"
swift test
