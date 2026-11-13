#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PKG="$ROOT/packages/leptoskit-pod-kotlin"

if ! command -v java >/dev/null 2>&1; then
	echo "SKIP: java not found — install JDK 17+ to run leptoskit-pod-kotlin conformance"
	exit 0
fi

GRADLE=(gradle)
if [[ -x "$PKG/gradlew" ]]; then
	GRADLE=("$PKG/gradlew")
elif ! command -v gradle >/dev/null 2>&1; then
	echo "SKIP: gradle not found — install Gradle 8+ or add a wrapper under packages/leptoskit-pod-kotlin"
	exit 0
fi

cd "$ROOT"
"${GRADLE[@]}" -p "$PKG" test
