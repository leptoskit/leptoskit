# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-25)

### Features

#### Publish cloud CLI crates and prebuilt release binaries

_Owner:_ Ifiok Jr. · _Introduced in:_ [e47b3de](https://github.com/leptoskit/leptoskit/commit/e47b3de2192792651b71741c9094b14cf49492be)

- Registers `leptoskit_cloud_platform` (`leptoskit-cloud-platform`), `leptoskit_cloud_worker` (`leptoskit-cloud-worker`), and `leptoskit_cloud_app` (`leptoskit-cloud-control`) in `monochange.toml` and removes `publish = false` so all three crates version and publish with the main release group.
- Adds `[package.metadata.binstall]` to all three crates and extends `.github/workflows/publish.yml` to build, archive, upload, and provenance-attest prebuilt binaries for all release targets alongside `leptoskit` and `lk`.
- Bundles the console's two Latin font subsets inside `crates/leptoskit_cloud_app/assets/fonts/` so `leptoskit_cloud_app` packages cleanly for crates.io.
- Updates `lk cloud <name>` missing-binary hints to recommend `cargo binstall` and `cargo install`.

### Fixes

- **Promote the bootstrap operator once per database.** With `LEPTOSKIT_CLOUD_BOOTSTRAP_OPERATOR_EMAIL` set, `leptoskit-cloud-control` makes that address an operator while the database has never had one. Before, it did so again at every start or `migrate` that found no operator, so revoking that address was undone by the next start. The promotion now stops once the audit log holds an `operator.bootstrap` or `operator.grant` entry, so `operator revoke` stays final. `operator revoke` says so when it removes the configured address and no operator remains. _Owner:_ Ifiok Jr. · _Introduced in:_ [a53a454](https://github.com/leptoskit/leptoskit/commit/a53a4542f7ffd915ae725ede071581ebb9891300)

#### The control plane's sweeper runs with several replicas

_Owner:_ Ifiok Jr. · _Introduced in:_ [1dc5cd4](https://github.com/leptoskit/leptoskit/commit/1dc5cd4261a10990a6b1a49726698cca5f276ed6)

A control plane generated with two or more `[control_plane] replicas` was deployed with `LEPTOSKIT_CLOUD_SWEEPER=false`. A Deployment gives that value to every replica, so none of them ran the background sweeper: a worker's expired lease was not taken back, and expired archives and logs were not deleted. The generated manifest now sets the variable to `true` whatever the replica count. The sweeper already lets one replica sweep at a time with a Postgres advisory lock, and `LEPTOSKIT_CLOUD_SWEEPER=false` still switches it off.

The documentation of `LEPTOSKIT_CLOUD_SWEEPER` in `leptoskit_cloud_app` no longer says to set it to `false` on secondary replicas. The `run_sweeper` setting's documentation in `leptoskit_cloud` no longer tells operators to set it to `false` on secondary replicas.
