#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PKG="$ROOT/packages/leptoskit-pod-dart"

if ! command -v dart >/dev/null 2>&1; then
	echo "SKIP: dart not found — install Dart SDK 3.3+ to run leptoskit-pod-dart conformance"
	exit 0
fi

cd "$PKG"
dart pub get
dart test
