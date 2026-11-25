# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-25)

### Features

#### `Store::schema_fingerprint()` and generation integrity

_Owner:_ Ifiok Jr. · _Introduced in:_ [19fc493](https://github.com/leptoskit/leptoskit/commit/19fc493af2a2b28674f7c64e676c96f7080601fe)

`leptoskit_deploy_console::control::Store` now exposes `Store::schema_fingerprint()`: a pure, non-I/O digest of the management migrations this build applies (version and SQL, length-framed), computed from the migration list itself so there is no second list to keep in step. Anyone holding a published state contract can now refuse schema drift _before_ `Store::open` runs — `Store::open` still verifies the journal and auto-applies new migrations, so before this API a caller had no way to detect a new migration ahead of the open that would apply it.

The public demo runtime (an unreleased example) uses it for exactly that: its generation fingerprint now covers the deploy console's migrations alongside the analytics migrations and the linked SQLite schema, a published marker's seed count is treated as a historical receipt rather than a live invariant, corrupt markers fail closed instead of triggering a rebuild, retention keeps recent published generations across formats as rollback state, and publication is preceded by an explicit fsync barrier over the databases, their WALs, and the directory entries up to the volume root.

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

### Breaking changes

#### leptoskit_deploy_console: self-hosted deployments

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #513](https://github.com/leptoskit/leptoskit/pull/513)

`leptoskit_deploy_console` is the optional service behind `lk console`: one process that hosts the deployment dashboard, a durable operation queue, a scoped API for agents, and analytics history, backed by SQLite.

Run `lk console serve` on a trusted worker with Docker, OpenTofu, Kamal 2, Git, and SSH, then `lk console connect` from the app's checkout, and `lk infra plan`, `lk infra apply`, and `lk deploy` run through it. It reuses the CLI's OpenTofu and release engine, binds each apply to a reviewed plan, and keeps interrupted operations for explicit reconciliation. The `server` feature enables the service; the dashboard is a leptoskit component tree rendered on the server. It serves one owner's repositories and is not a multi-tenant executor.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
