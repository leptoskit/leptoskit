---
"leptoskit_config": minor
"leptoskit_cli": minor
---

# Add cloud launch/deploy, storage config, and MCP tools

- Adds `[cloud.storage.buckets.<name>]` configuration (`public`, `max_size_gb`) and updates the published `leptoskit.toml` JSON schemas.
- Adds `lk cloud launch`, `lk cloud deploy`, `lk cloud status`, `lk cloud logs [--follow]`, `lk cloud estimate`, and `lk cloud open`, wires `lk deploy` to dispatch to LeptosKit Cloud whenever `[cloud].project` is set, and adds Cargo-style external subcommand dispatch (`lk cloud <name>` → `leptoskit-cloud-<name>` on `PATH`, enabling `lk cloud platform`, `lk cloud worker`, and `lk cloud control`).
- Adds `--web-only` and `--cloud` flags to `lk new` and `lk generate app`, allows `--frontends` to combine with `--db`, `--auth`, `--web-only`, and `--cloud`, and falls back with a clear warning instead of hard-failing `lk dev` when `monosecret` is not installed.
- Exposes six cloud and database tools on `lk mcp` (`cloud_estimate`, `cloud_deploy`, `cloud_status`, `cloud_logs`, `cloud_secrets_list`, `db_check`) without ever returning secret values.
