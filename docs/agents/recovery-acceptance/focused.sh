#!/usr/bin/env bash
# Run inside devenv shell with an isolated CARGO_TARGET_DIR and RUSTUP_HOME.
set -euo pipefail
: "${CARGO_TARGET_DIR:?set an exclusive build directory}"
export CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0
export NEXTEST_TEST_THREADS=4 LEPTOSKIT_REQUIRE_POSTGRES=1 LEPTOSKIT_REQUIRE_RESTIC=1
unset PGBIN LEPTOSKIT_POSTGRES_BIN
cargo test -p leptoskit_cli --lib infra:: -- --test-threads=4
cargo test -p leptoskit_cli --test infra -- --test-threads=4
cargo test -p leptoskit_db --features sqlite,postgres --test migrations -- --test-threads=4
