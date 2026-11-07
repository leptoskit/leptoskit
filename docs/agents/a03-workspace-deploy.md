# A03 workspace deployment acceptance

The machine-readable record is [a03-workspace-deploy.json](a03-workspace-deploy.json). Local build logs, generated fixtures, and container evidence live in `.git/a03-evidence` in the exclusive Mini02 checkout. A result marked pending is not acceptance.

## Scope

The generator resolves server and client packages through Cargo metadata. The context is the Cargo workspace, while configuration, assets, migrations, and generated site output remain relative to the application. The fixture uses a globbed member, an inherited local dependency, a path containing spaces, and an application directory without a manifest. Its build script rejects secret, database, cache, state, and custom-ignore sentinels if they reach the container.

Nested containers explicitly use `/src/target`. Host `CARGO_TARGET_DIR` does not affect generation. A standalone Cargo configuration that moves the target directory fails before writing deployment files. Existing standalone snapshots remain unchanged.

## Source and engine differences

Published CLI 0.3.0 rejects the current `build.backend` configuration field. The failed generated-image attempt is retained in `image-published.log`. Nested recipes expose `LEPTOSKIT_CLI_PATH` as an explicit build argument for a CLI crate inside the build context. The source-install step uses the copied workspace and its toolchain; it rejects a path resolving outside `/src`.

The successful source builds pass `RUST_VERSION=a03`, a local tag for image `420e8b272c61` produced by the failed published-CLI attempt's completed install stage. That retains downloaded crates across VM recovery. It is not a hand-written Dockerfile or a replacement application image: the generated source-install step compiles and replaces the CLI, then the generated application build and runtime stages run normally.

Podman 6.1.4 did not automatically select `Dockerfile.dockerignore`. Inspection of the first build stage found the nested sentinels in `/src`. Podman acceptance therefore explicitly passes the generated file with `--ignorefile`; its Compose provider can forward that option through `--podman-build-args`. This does not claim automatic Dockerfile-specific file selection was tested with Docker Engine.

Fly context and path behavior was checked against [the official monorepo documentation](https://docs.fly.io/launch/monorepo): run from the workspace root and supply workspace-relative config and Dockerfile paths. No Fly deployment or cloud apply is part of this audit.

## Resource and evidence recovery

One two-CPU, three-GiB task VM is used at a time. The first VM's identity and backing files and the host target directory disappeared during validation without a removal command from this task. Its open API connection remained usable. The published CLI layer was exported before stopping and removing that task VM. Replacement VM storage is inside `.git/a03-evidence/vm`; host build output is exposed at `.git/a03-target`, backed by `.git/a03-evidence/host-build` after another deletion interrupted schema validation. No other task's artifacts were removed.

The first focused run passed 63 tests, including actual Postgres bootstrap and archive recovery, and failed on the host target-directory leak. After the fix and a rebuild, all 67 focused tests passed. All 61 CLI infrastructure integration and snapshot tests also passed, with no snapshot updates. The final full-gate and image results belong in the JSON record.

## Real image results

The tiny source-CLI image built from the generated Dockerfile, passed the sentinel assertions, and ran with a read-only root as UID 10001. `/readyz` returned `a03-ready`; `/app/server` and `/app/dist` were present. An actual Podman Compose build of the same generated files passed using Docker image format, and the generated health check reported `healthy`.

Huddle's generated image also built successfully with `LEPTOSKIT_CLI_PATH=crates/leptoskit_cli`. The final image contains the 37 MiB `/app/server` and the separate client's JavaScript and WebAssembly files, including Brotli and gzip variants under `/app/dist/pkg`. This is build and artifact acceptance, not a claim that Huddle's authenticated runtime was exercised. No application secrets were supplied. The original `examples/huddle/deploy/` recipe was not changed or substituted.

Both accepted runtime images were exported before stopping and removing the task VM. The full gate and normal pre-push hook results are recorded separately in the JSON evidence.

The unmodified `devenv shell lint:push` completed successfully on the rebased source: 5,131 workspace tests, 16 file-routes tests, and 43 devtools tests passed with Postgres required. Clippy, format, documentation, release validation, schema checks, workflow security, and dependency policy passed. No snapshots changed.

An actual `git push` completed through the normal pre-push hook with every gate passing and Postgres required. The Keychain credential helper initially stalled; the successful invocation used the existing GitHub CLI authentication through a command-only credential helper override. Repository identity, signing, and hooks were not reconfigured. An earlier attempt passed its checks but was rejected by the hook's working-tree guard after a documentation edit during the run; the successful retry used an untouched clean tree.
