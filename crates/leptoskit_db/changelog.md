# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-21)

### Features

#### Control-plane scaling, HA sweeper, CA TLS, and billing

_Owner:_ Ifiok Jr. · _Introduced in:_ [713fddf](https://github.com/leptoskit/leptoskit/commit/713fddf6095efdf17fbbbbd7e6ea87d9c2d586a6)

- `leptoskit_db` now supports custom Postgres CA PEM bundles via `sslrootcert=` or `PGSSLROOTCERT`, and adds session-scoped `Db::try_advisory_lock` for HA leader coordination.
- `update_environment` enforces plan scale limits (`max_size`, `max_replicas`, `require_sleep`, also exposed on `PlanLimits` and enforced on `rollback`) and queues an immediate live-scale cluster release when the caller holds `Permission::Deploy` (allowing scale-down even over spend cap, skipping no-op rollouts, honoring cluster facts, and preserving `schema_uncertain`). Forward deploys clear `scale_differs` once applied.
- Added `cluster_environments` worker RPC so restarted cluster workers immediately re-hydrate tracked environments for status, logs, and compute metering.
- `ShippingSettings` gains O(1)-tracked `organization_runtime_log_cap` (via `log_usage.runtime_bytes`), `run_sweeper` (`LEPTOSKIT_CLOUD_SWEEPER`), and Postgres advisory-lock sweeper leader election across HA replicas.
- Hardened `S3Store` (disabled redirects, bounded streaming hash concurrency, `400`/`403`/`501` checksum fallback, content-length check), `Stripe` (sanitized error telemetry and key length check), `subscriptions` (concurrent checkout customer conflict handling and overdue sweep rotation), and `bootstrap_operator` (concurrency-safe, checks disabled accounts, only runs when no operators exist).
- `leptoskit_cloud_app` adds a `production = ["ssr", "postgres"]` feature flag.

### Fixes

#### Refuse ambiguous consolidated rollback without panicking

_Owner:_ Ifiok Jr. · _Introduced in:_ [8051706](https://github.com/leptoskit/leptoskit/commit/8051706d7c9f9c655f5dc639b8acff2da3c2d47c)

Rollback now returns a migration-history error when an adopted consolidation leaves more applied history rows than the current registry contains. It stops before reverse SQL or history changes instead of indexing beyond the registry. Ordinary rollback of an intact registry prefix is unchanged.

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

### Breaking changes

#### leptoskit_db: tables as Rust structs

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_db` is leptoskit's database layer: tables declared as Rust structs, typed IDs, client-safe public projections that leave out private and secret columns, typed queries and writes, links, access policies, and migrations.

Enable `db` on `leptoskit` for the client side, which has no database code, and `db-postgres` or `db-sqlite` on the server; `db-geo` and `db-vector` add geospatial and vector columns. `leptoskit db` diffs the tables and writes migrations, and `leptoskit db studio` browses the data in development. Auth and IDP build their schema packs on it.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
