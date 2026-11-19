---
"leptoskit_auth": minor
"leptoskit_cloud": minor
---

# Fix control-plane scaling, plan scale limits, S3 checksum fallback, and operator bootstrap

- `update_environment` now enforces plan scale limits (`max_size`, `max_replicas`, `require_sleep`) and immediately queues a cluster `Release` job to scale live deployments instead of leaving live pods unchanged. Subsequent deploys clear `scale_differs` so future `leptoskit.toml` changes are not permanently locked out.
- `ShippingSettings` gains `organization_runtime_log_cap` (default 512 MiB) enforced in `report_status` without interrupting status or compute metering, plus `run_sweeper` (`LEPTOSKIT_CLOUD_SWEEPER`) for multi-replica control planes. Expired backup sweeping now runs inside serializable transactions.
- `S3Config` gains `checksum_headers` (`LEPTOSKIT_CLOUD_S3_CHECKSUM_HEADERS`) and `S3Store::verify` falls back to plain `HEAD` + streaming SHA-256 verification when an S3-compatible store answers `400` or `501` to `x-amz-checksum-mode: ENABLED`.
- `leptoskit_cloud::server::bootstrap_operator` (backed by `leptoskit_auth::ensure_verified_email_account`) enables non-interactive operator bootstrapping via `LEPTOSKIT_CLOUD_BOOTSTRAP_OPERATOR_EMAIL` during `migrate` and `serve`.
- `Stripe::send` logs HTTP status and redacted Stripe error type/code on non-2xx responses.
- `leptoskit_cloud_app` adds a `production = ["ssr", "postgres"]` feature convenience flag.
