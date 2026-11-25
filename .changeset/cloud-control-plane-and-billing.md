---
"leptoskit_auth": minor
"leptoskit_cloud": minor
"leptoskit_db": minor
---

# Control-plane scaling, HA sweeper, CA TLS, and billing

- `leptoskit_db` now supports custom Postgres CA PEM bundles via `sslrootcert=` or `PGSSLROOTCERT`, and adds session-scoped `Db::try_advisory_lock` for HA leader coordination.
- `update_environment` enforces plan scale limits (`max_size`, `max_replicas`, `require_sleep`, also exposed on `PlanLimits` and enforced on `rollback`) and queues an immediate live-scale cluster release when the caller holds `Permission::Deploy` (allowing scale-down even over spend cap, skipping no-op rollouts, honoring cluster facts, and preserving `schema_uncertain`). Forward deploys clear `scale_differs` once applied.
- Added `cluster_environments` worker RPC so restarted cluster workers immediately re-hydrate tracked environments for status, logs, and compute metering.
- `ShippingSettings` gains O(1)-tracked `organization_runtime_log_cap` (via `log_usage.runtime_bytes`), `run_sweeper` (`LEPTOSKIT_CLOUD_SWEEPER`), and Postgres advisory-lock sweeper leader election across HA replicas.
- Hardened `S3Store` (disabled redirects, bounded streaming hash concurrency, `400`/`403`/`501` checksum fallback, content-length check), `Stripe` (sanitized error telemetry and key length check), `subscriptions` (concurrent checkout customer conflict handling and overdue sweep rotation), and `bootstrap_operator` (concurrency-safe, checks disabled accounts, runs once per database and is not repeated after a revoke).
- `leptoskit_cloud_app` adds a `production = ["ssr", "postgres"]` feature flag.
