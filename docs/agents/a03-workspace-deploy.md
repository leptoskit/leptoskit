# A03 workspace deployment acceptance

The machine-readable record is [a03-workspace-deploy.json](a03-workspace-deploy.json). Local build logs, generated fixtures, and container evidence live in `.git/a03-evidence` in the exclusive Mini02 checkout. A result marked pending is not acceptance.

## Scope

The generator resolves server and client packages through Cargo metadata. The context is the Cargo workspace, while configuration, assets, migrations, and generated site output remain relative to the application. The fixture uses a globbed member, an inherited local dependency, a path containing spaces, and an application directory without a manifest. Its build script rejects secret, database, cache, state, and custom-ignore sentinels if they reach the container.

Nested containers explicitly use `/src/target`. Host `CARGO_TARGET_DIR` does not affect generation. A standalone Cargo configuration that moves the target directory fails before writing deployment files. Existing standalone snapshots remain unchanged.

## Source and engine differences

Published CLI 0.3.0 rejects the current `build.backend` configuration field. The failed generated-image attempt is retained in `image-published.log`. Nested recipes expose `LEPTOSKIT_CLI_PATH` as an explicit build argument for a CLI crate inside the build context. The source-install step uses the copied workspace and its toolchain; it rejects a path resolving outside `/src`.

Podman 6.1.4 did not automatically select `Dockerfile.dockerignore`. Inspection of the first build stage found the nested sentinels in `/src`. Podman acceptance therefore explicitly passes the generated file with `--ignorefile`; its Compose provider can forward that option through `--podman-build-args`. This does not claim automatic Dockerfile-specific file selection was tested with Docker Engine.

Fly context and path behavior was checked against [the official monorepo documentation](https://docs.fly.io/launch/monorepo): run from the workspace root and supply workspace-relative config and Dockerfile paths. No Fly deployment or cloud apply is part of this audit.

## Resource and evidence recovery

One two-CPU, three-GiB task VM is used at a time. The first VM's identity and backing files and the host target directory disappeared during validation without a removal command from this task. Its open API connection remained usable. The published CLI layer was exported before stopping and removing that task VM. Replacement VM storage is inside `.git/a03-evidence/vm`; host build output is `.git/a03-target`. No other task's artifacts were removed.

The first focused run passed 63 tests, including actual Postgres bootstrap and archive recovery, and failed on the host target-directory leak. After the fix and a rebuild, all 67 focused tests passed. All 61 CLI infrastructure integration and snapshot tests also passed, with no snapshot updates. The final full-gate and image results belong in the JSON record.
