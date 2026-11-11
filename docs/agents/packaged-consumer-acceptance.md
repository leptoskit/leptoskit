# Packaged consumer acceptance

Workspace tests can enable features through unrelated members. A successful default `cargo package` build also leaves browser, guest, database and auth combinations untested. A separate read-only validation job now checks eight consumers after package verification. It runs during release dry runs too, without entering the publisher environment or requesting registry credentials.

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

The gate leaves target management to `mbx` instead of setting `CARGO_TARGET_DIR`. Separate Cargo invocations and independent workspace manifests resolve their feature graphs independently while compatible compiled dependencies can use the shared cache. The recorded source revision is package metadata; it is not a cryptographic proof that those bytes came from that revision. Exact archive hashes identify what the gate actually tested. The workflow still performs its existing crate packaging verification before this step.

## Evidence boundaries

The Python harness tests exercise extraction, identity/revision rejection, required package presence, separate manifest construction, graph-source checks, feature leakage, and failure without successful evidence. A normal CLI integration test runs those tests, so they are not silently skipped in the workspace gate.

The source implementation's harness tests have run locally. The eight real Rust consumer builds, the Rust wrapper and full normal push gate remain pending initial acceptance. Before merging, generate actual candidate archives, execute every profile without skips, retain hashes and logs, and fix any missing packaged files or feature dependencies. Existing generated-app/native/server SDK journeys remain separate requirements. This gate does not claim registry-download delivery, installed CLI behavior, native widget support, cloud deployment, or runtime provenance.

The publisher depends on successful package validation, binary builds, and attestations. It checks out the same validated revision, restores the same interface hash, prepares archives without repeating compilation, downloads the successful consumer report, and compares every prepared archive's SHA-256 with the tested payload before requesting a registry credential:

```sh
python3 setup/scripts/packaged-consumers.py \
  --archives "$CARGO_TARGET_DIR/package" \
  --verify-report /path/to/consumer-evidence/acceptance.json
```

Verification rejects a different revision, missing or repeated profiles, changed features or targets, failed profiles, a different package set, and changed archive bytes. A dry run executes validation but skips the publisher and public-release steps. The workflow wiring and these rejection cases have local checks; actual package generation in two fresh checkouts and equality of their archive hashes still need acceptance. A successful local harness run alone does not establish that the workflow's full dry run has executed.
