#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if ! command -v cargo-mutants >/dev/null 2>&1 && ! cargo mutants --version >/dev/null 2>&1; then
	echo "Skip: cargo-mutants not installed (cargo install cargo-mutants)"
	exit 0
fi

TIMEOUT_SEC="${POD_MUTANTS_TIMEOUT_SEC:-900}"
echo "==> cargo-mutants: leptoskit_pod (timeout ${TIMEOUT_SEC}s)"

MUTANTS_CMD=(cargo mutants -p leptoskit_pod --no-shuffle --emit-json)

if command -v timeout >/dev/null 2>&1; then
	timeout --signal=TERM --kill-after=60 "$TIMEOUT_SEC" "${MUTANTS_CMD[@]}"
else
	"${MUTANTS_CMD[@]}"
fi
