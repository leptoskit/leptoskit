---
"leptoskit_cloud": minor
"leptoskit_rpc": minor
---

# Limit request rates and estimate a `[cloud]` table

The LeptosKit Cloud control plane now counts every operation against who asked: a person, an API token and its organisation's tokens together, a worker, or the address of a request that is nobody's. A caller past their rate is answered `429` (`rate_limited`) until the window ends. `ControlConfig` gains `rate_limits` and `client_ip_header`, so code that builds one by hand must set them; the server reads the header's name from `LEPTOSKIT_CLOUD_CLIENT_IP_HEADER`. An organisation's checkout and portal pages have a rate of their own.

`leptoskit_cloud::billing` adds `estimate`: what a month of a whole `[cloud]` table is expected to cost on a plan, before anything is deployed.

A backup's dump has a size limit, `ShippingSettings::backup_cap`. Workers are told it in `BackupDatabaseInput::max_bytes` and `RestoreDatabaseInput::max_bytes`, and a dump a worker reports as larger is a backup that failed.

`leptoskit_rpc::Error` gains `TooMany`, answered as `429`, for an operation or an `Identify` that refuses a caller who asks too often. `Identify` gains `admit`, asked before each subscription, mutation, action and stream a live connection sends, so a limit on how often a caller asks covers a connection that is identified once. It admits everything unless implemented.

A `[cloud]` table with more than 50 environments, 50 services or 50 hostnames in an environment is now refused as `invalid_config` before it is resolved.
