# Packaged consumer acceptance

Workspace tests can enable features through unrelated members. A successful default `cargo package` build also leaves browser, guest, database and auth combinations untested. The publication job now checks eight separate consumers after package verification and before requesting registry credentials.

`setup/scripts/packaged-consumers.py` reads the exact `.crate` payloads produced by Cargo. It checks each package's name, version and recorded source revision, then extracts into a new directory outside the checkout. Each consumer has its own workspace and new lockfile, an exact registry-version dependency on the facade, and patches pointing only to those extracted payloads. It neither copies the monorepo lockfile nor patches to repository source. Cargo metadata must confirm every resolved leptoskit crate comes from the expected payload. Client graphs must not enable server or SQL-driver features.

The eight profiles cover the minimal facade on the host, CSR and hydration on Wasm, a native Wasm guest, SQLite and Postgres servers, and auth on each database. Browser clients enable `rpc`; `rpc-live` is a server feature and would introduce the server/database implementation into their graph. The fixtures compile the portable builder surface, browser rendering or guest mounting, and server RPC/database entry points as applicable. These are compilation and feature-isolation checks, not runtime application journeys or device evidence.

## Running it

First create the package archives with the same `cargo package --workspace --locked --allow-dirty` command and unpublished-package exclusions used by the publication workflow. Run in the pinned devenv toolchain, then:

```sh
python3 setup/scripts/packaged-consumers.py \
  --archives "$CARGO_TARGET_DIR/package" \
  --output /absolute/new/path/outside/the/checkout
```

The output directory must not already exist. All eight profiles are mandatory; absent tools, a missing Wasm target, missing/stale packages, registry fallback, feature leakage or a failed build fail the gate. A final `acceptance.json` appears only after all profiles pass. It records archive hashes, the toolchain, each lockfile hash, and the resolved feature sets. Per-profile logs and metadata are retained on failure. Publication uploads these small evidence files without uploading build targets or package trees.

The gate shares a target directory across the serial consumers to conserve space. Separate Cargo invocations and independent workspace manifests still resolve their feature graphs independently. The recorded source revision is package metadata; it is not a cryptographic proof that those bytes came from that revision. Exact archive hashes identify what the gate actually tested. The workflow still performs its existing crate packaging verification before this step.

## Evidence boundaries

The Python harness tests exercise extraction, identity/revision rejection, required package presence, separate manifest construction, graph-source checks, feature leakage, and failure without successful evidence. A normal CLI integration test runs those tests, so they are not silently skipped in the workspace gate.

The source implementation's harness tests have run locally. The eight real Rust consumer builds, the Rust wrapper and full normal push gate remain pending initial acceptance. Before merging, generate actual candidate archives, execute every profile without skips, retain hashes and logs, and fix any missing packaged files or feature dependencies. Existing generated-app/native/server SDK journeys remain separate requirements. This gate does not claim registry-download delivery, installed CLI behavior, native widget support, cloud deployment, or runtime provenance.

The publication workflow's existing `dry_run` condition skips the entire crate-publishing job, including this gate. Run the local command for a package-only rehearsal; do not report a workflow dry run as packaged-consumer acceptance.
