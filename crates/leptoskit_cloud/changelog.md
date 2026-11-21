# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-21)

### Features

#### Accounts, tokens and projects for LeptosKit Cloud

_Owner:_ Ifiok Jr. · _Introduced in:_ [9bd21da](https://github.com/leptoskit/leptoskit/commit/9bd21da8b6fcad9589adc4968b71fb92a656c42a)

`leptoskit_cloud` now declares the control plane's first operations, so a client such as the CLI can call them without building the server: `whoami`, organisations, members and invitations, API tokens, and projects and their environments. Their tables and argument types build everywhere, including wasm; their bodies, sign-in and routes are behind the new `server` feature, with SQLite and Postgres migrations.

The server keeps tenants apart and checks every call against one permission table: an organisation or project you can't see answers "not found" whether it exists or not, an organisation always keeps an owner, and a token never does more than its scopes and its maker's current role allow, or reaches a project it isn't bound to. API tokens look like `lkc_<prefix>_<secret>`, are shown once, stored only as a hash, expire after 90 days unless asked otherwise, and stop the moment they're revoked. Invitations are mailed, single-use, expire in a week, and work only for the address they were sent to. The CLI signs in with the device flow, and every change is written to an audit log that never holds a secret.

A queued deployment can now fail before any work starts, when no builder is available, or a quota, the spend cap or a payment stops it.

#### Back up and restore a managed database

_Owner:_ Ifiok Jr. · _Introduced in:_ [dddcd97](https://github.com/leptoskit/leptoskit/commit/dddcd97281bb14d1648cfb3b77517c23e27bca13)

`leptoskit_cloud::databases` adds three operations. `create_backup` asks an environment's cluster for a dump of its Postgres database, kept for 30 days. `database_backups` lists them, with the latest restore. `restore_backup` puts one back over the database it was taken from, for owners and admins who type `<project>/<environment>` to confirm, after the cluster has taken a backup of what it replaces.

The control plane keeps a record of each backup and never its bytes. `JobKind`, `JobInput` and `JobOutcome` gain `BackupDatabase` and `RestoreDatabase`, with the contract a cluster worker must keep on `BackupDatabaseInput` and `RestoreDatabaseInput`. `deploy` and `rollback` answer `database_restoring` while a restore is under way. Backups on a schedule are not available yet.

#### Add environment variables and the secrets vault

_Owner:_ Ifiok Jr. · _Introduced in:_ [25cddbe](https://github.com/leptoskit/leptoskit/commit/25cddbed387b5b2483808d0171a5b063884ba739)

`leptoskit_cloud::config` adds `variables`, `set_variable`, `unset_variable`, `secrets`, `set_secret` and `unset_secret`, behind a new `manage_config` permission for owners, admins and developers. Variables are written into the next deployment's release; secrets are named in it and their values are sealed per organisation with AES-256-GCM-SIV, bound to where they were stored, and leave the control plane only in a cluster worker's claim of that release, with a lease row for each.

A release whose secrets can't be read fails with the new `secret_unavailable` blocker. The control plane needs `LEPTOSKIT_CLOUD_VAULT_KEYS`, and `ControlConfig` gains `vault`. An environment's variables take at most 256 KiB together and its secrets 512 KiB, and `set_secret` takes a `SecretValue`, which is never repeated in an error.

#### Give Postgres apps a managed database

_Owner:_ Ifiok Jr. · _Introduced in:_ [ac56912](https://github.com/leptoskit/leptoskit/commit/ac56912d7456e158da2a78464c5e19a4940b5ec1)

An app whose `[db]` is Postgres can now be deployed to a cluster that has a Postgres server. `deploy` gives its environment a database and a login role of its own, and the app's process starts with `DATABASE_URL` as a secret, handed by value only to the cluster worker that claims the release. `leptoskit_cloud::databases` adds `database`, which answers the database's name, size and whether a release has used it, and never its password.

`ClusterFacts` gains `postgres`, operators get `set_cluster_facts`, `ReleaseInput` and `DestroyEnvironmentInput` gain `database`, and `PlanError::PostgresUnavailable` now means the environment's cluster has no Postgres server. Dedicated instances and backups are not available yet.

#### Uploads, deployments and workers for LeptosKit Cloud

_Owner:_ Ifiok Jr. · _Introduced in:_ [f3c87dd](https://github.com/leptoskit/leptoskit/commit/f3c87dd0aa250554006aecfcdc607cd0d7bc95d1)

The control plane can now ship an app. A client uploads its source archive straight to object storage through a presigned address, which the server checks for size and SHA-256 before anything may deploy it. It then calls `deploy` and watches the deployment go from queued through building, migrating and rolling out to live, as live queries. Each deployment records who signed it off and from where, the configuration it resolved, and its source. Its sheet shows a line per part (server, services, migrations, web assets) with stamps, build steps and fine print. An environment has one deployment in flight at a time. `cancel_deployment`, `rollback` and `preview_release` (with a monthly estimate when the operator configures prices) round it out, along with paged, redacted `build_logs` and `runtime_logs` and `environment_status`.

Build and cluster workers have their own protocol and keys (`lkw_…`), paired once with a code an operator issues. Workers `claim` one job at a time with a long poll, `renew` their lease (which says when a deployment was cancelled), stream logs, and `finish` idempotently. A build whose worker disappears is retried, then fails as `builder_unavailable`. A release whose worker disappears is never replayed: it waits for the worker's saved result or an operator's `reconcile`. Operators register clusters, pair and revoke workers, and list jobs; an environment is placed on a cluster in its region at its first deployment. Deleting an environment that ran queues its teardown.

The `server` feature now needs object storage: an S3-compatible store, or a local directory for development and tests. The tables are compiled only with `server`, so a client build sees just the plain types the operations take and return, whatever database features the rest of its build turns on. A new `deploy` permission, held by owners, admins and developers and granted to tokens by the `deploy` scope, covers uploading, deploying, cancelling and rolling back.

#### Add custom domains to the control plane

_Owner:_ Ifiok Jr. · _Introduced in:_ [bbccf61](https://github.com/leptoskit/leptoskit/commit/bbccf6129b044e6d4e5221b7df8741b3cae96cb2)

`leptoskit_cloud::domains` adds `domains`, `attach_domain`, `verify_domain` and `detach_domain`. A domain is proved with a TXT record read over DNS-over-HTTPS, a verified hostname belongs to one environment in the whole cloud, and the environment's next release routes it with a certificate of its own. A detached domain keeps its hostname until a deployment without it is live, and a rollback never routes a domain the environment no longer holds.

`ControlConfig` gains `resolver`, and the control plane reads `LEPTOSKIT_CLOUD_DNS_RESOLVER`.

#### Count usage, limit plans and cap spending

_Owner:_ Ifiok Jr. · _Introduced in:_ [4a543fe](https://github.com/leptoskit/leptoskit/commit/4a543fedc532f7671c20e435a0ca95181e19447a)

`leptoskit_cloud::billing` adds `plans`, `usage` and `set_spend_cap`. The control plane now counts build minutes and compute seconds per project and calendar month, and `usage` prices a month on the organisation's plan with the operator's catalogue. Once a month's usage reaches the spend cap, `deploy` answers `spend_cap_reached`; nothing running is stopped.

A plan in the price catalogue can carry `limits` (`organizations`, `projects`, `custom_domains`) and a `spend_cap`; `create_organization`, `create_project` and `attach_domain` answer `quota_exceeded` at a limit. A control plane now refuses to start with a catalogue that has no `hobby` plan. `Permission` gains `ViewBilling` and `ManageBilling`, and `pricing::Plan` gains `limits` and `spend_cap`, so code that builds a `Plan` by hand must set them. Nothing charges anyone yet.

#### Control-plane scaling, HA sweeper, CA TLS, and billing

_Owner:_ Ifiok Jr. · _Introduced in:_ [713fddf](https://github.com/leptoskit/leptoskit/commit/713fddf6095efdf17fbbbbd7e6ea87d9c2d586a6)

- `leptoskit_db` now supports custom Postgres CA PEM bundles via `sslrootcert=` or `PGSSLROOTCERT`, and adds session-scoped `Db::try_advisory_lock` for HA leader coordination.
- `update_environment` enforces plan scale limits (`max_size`, `max_replicas`, `require_sleep`, also exposed on `PlanLimits` and enforced on `rollback`) and queues an immediate live-scale cluster release when the caller holds `Permission::Deploy` (allowing scale-down even over spend cap, skipping no-op rollouts, honoring cluster facts, and preserving `schema_uncertain`). Forward deploys clear `scale_differs` once applied.
- Added `cluster_environments` worker RPC so restarted cluster workers immediately re-hydrate tracked environments for status, logs, and compute metering.
- `ShippingSettings` gains O(1)-tracked `organization_runtime_log_cap` (via `log_usage.runtime_bytes`), `run_sweeper` (`LEPTOSKIT_CLOUD_SWEEPER`), and Postgres advisory-lock sweeper leader election across HA replicas.
- Hardened `S3Store` (disabled redirects, bounded streaming hash concurrency, `400`/`403`/`501` checksum fallback, content-length check), `Stripe` (sanitized error telemetry and key length check), `subscriptions` (concurrent checkout customer conflict handling and overdue sweep rotation), and `bootstrap_operator` (concurrency-safe, checks disabled accounts, only runs when no operators exist).
- `leptoskit_cloud_app` adds a `production = ["ssr", "postgres"]` feature flag.

#### Limit request rates and estimate a `[cloud]` table

_Owner:_ Ifiok Jr. · _Introduced in:_ [373ad29](https://github.com/leptoskit/leptoskit/commit/373ad295394a26e69e883a6b28983d35b7e3f02d)

The LeptosKit Cloud control plane now counts every operation against who asked: a person, an API token and its organisation's tokens together, a worker, or the address of a request that is nobody's. A caller past their rate is answered `429` (`rate_limited`) until the window ends. `ControlConfig` gains `rate_limits` and `client_ip_header`, so code that builds one by hand must set them; the server reads the header's name from `LEPTOSKIT_CLOUD_CLIENT_IP_HEADER`. An organisation's checkout and portal pages have a rate of their own.

`leptoskit_cloud::billing` adds `estimate`: what a month of a whole `[cloud]` table is expected to cost on a plan, before anything is deployed.

A backup's dump has a size limit, `ShippingSettings::backup_cap`. Workers are told it in `BackupDatabaseInput::max_bytes` and `RestoreDatabaseInput::max_bytes`, and a dump a worker reports as larger is a backup that failed.

`leptoskit_rpc::Error` gains `TooMany`, answered as `429`, for an operation or an `Identify` that refuses a caller who asks too often. `Identify` gains `admit`, asked before each subscription, mutation, action and stream a live connection sends, so a limit on how often a caller asks covers a connection that is identified once. It admits everything unless implemented.

A `[cloud]` table with more than 50 environments, 50 services or 50 hostnames in an environment is now refused as `invalid_config` before it is resolved.

#### Only people manage members and tokens in LeptosKit Cloud

_Owner:_ Ifiok Jr. · _Introduced in:_ [9bd21da](https://github.com/leptoskit/leptoskit/commit/9bd21da8b6fcad9589adc4968b71fb92a656c42a)

An API token can no longer invite people, revoke invitations, change a member's role, remove a member, or list, make or revoke tokens, whatever its scopes: those answer `403 person_required` to a token, and need someone signed in, in the browser or through the CLI. The `admin` scope now means deleting projects and environments, and nothing more. A leaked token can't turn itself into a membership or a longer-lived token.

`create_token` is now an action, so a token's value is never stored anywhere, not even as the remembered answer to a request with an `Idempotency-Key`; the header is ignored. Instead a live token's name is unique in its organisation, and asking again for a name a live token has answers `409 token_name_taken`, naming that token so it can be revoked, rather than quietly making a second one.

An invitation is only as good as its sender's authority when it is accepted: if they were demoted or left, it no longer works, and their pending invitations they can no longer give are revoked. Inviting an address again needs the authority to cancel each invitation it replaces, and each one cancelled is written to the audit log.

A personal organisation's name falls back to a random suffix after `ana`, `ana-2` and `ana-3`, so nobody can stop someone's first sign-in by taking every name their address suggests.

#### Sell plans through Stripe

_Owner:_ Ifiok Jr. · _Introduced in:_ [9dcd16a](https://github.com/leptoskit/leptoskit/commit/9dcd16acc5aabae6efd69e03663913cf48aed3d6)

`leptoskit_cloud::billing` adds `subscription`, `checkout` and `portal`. With a Stripe key, a webhook secret and a price ID for each plan that is sold, an organisation's owners and billing members can subscribe to a plan on Stripe Checkout and manage it on Stripe's customer portal. The plan changes only when Stripe says the subscription did: the control plane verifies each event's signature at `/webhooks/stripe`, then asks Stripe what the subscription is now.

`ControlConfig` gains `payments`, so code that builds one by hand must set it (`None` sells nothing). The `leptoskit_cloud_app` server reads `LEPTOSKIT_CLOUD_STRIPE_KEY`, `LEPTOSKIT_CLOUD_STRIPE_WEBHOOK_SECRET`, `LEPTOSKIT_CLOUD_STRIPE_PRICES`, `LEPTOSKIT_CLOUD_STRIPE_AUTOMATIC_TAX` and `LEPTOSKIT_CLOUD_STRIPE_TEST_MODE`; a Stripe test key is refused outside `--dev` without the last. A plan that is sold needs a `monthly` price and a `spend_cap` in the catalogue, and `set_spend_cap` now refuses a cap above the plan's own (`cap_above_plan`). Usage beyond what a plan includes is still not invoiced.

- **The groundwork for LeptosKit Cloud.** `leptoskit.toml` accepts a `[cloud]` table that says where LeptosKit Cloud should run your app and how big: a project, an optional region, and per-environment size, replicas, sleeping when idle and custom domains, plus the database's size and backups. Config checks it as it loads, with errors that name the table and key, such as a SQLite app asking for more than one replica, or sleeping while it serves live queries. The new `leptoskit_cloud` crate holds the shared types behind it: slugs, sizes, deployment states, the source archive a deploy will upload (deterministic, with secret files refused), and a monthly cost estimate for a `[cloud]` table. The cloud itself doesn't run yet, so nothing deploys there today. _Owner:_ Ifiok Jr. · _Introduced in:_ [403aaf0](https://github.com/leptoskit/leptoskit/commit/403aaf07e6df872150ffa904a48c1a9d55e1b6ed)

#### Publish to app and extension stores from the cloud

_Owner:_ Ifiok Jr. · _Introduced in:_ [101b2ee](https://github.com/leptoskit/leptoskit/commit/101b2ee1e7acf5a060554a6a0bd7c9f597ff7903)

`leptoskit_cloud::publishing` adds `publish`, which builds an uploaded archive for its stores and uploads it on one of the organisation's build workers, as `lk publish` does, with the environment's secrets named as `lk publish` reads them. `publication`, `publications` and `publication_log` read a publish back, with its report and its log. A publish is never run twice, since it may have reached a store. `JobKind`, `JobInput` and `JobOutcome` gain `Publish`; store credentials may now be set as secrets, and no release is given them.

`leptoskit publish` takes a keystore's `_BASE64` variable in place of its path for Google Play and the Solana dApp Store, and with `LEPTOSKIT_PUBLISH_CONTENTS_ONLY=1` refuses a credential that names a file.

### Fixes

#### Close the control plane's second security review

_Owner:_ Ifiok Jr. · _Introduced in:_ [dedf032](https://github.com/leptoskit/leptoskit/commit/dedf032eb758ec35adc6689619328df598e6c427)

An upload keeps the size it was begun with, and each archive has its own object key, so no ticket uploads more than the storage cap counted and a swept archive never takes a later upload with it. `claim` filters by role, organisation and cluster and pages through the queue, so one cluster's or organisation's backlog no longer starves the rest.

A build's log, an organisation's build logs, and the time one attempt at a build may run are all capped (`ShippingSettings::build_log_cap`, `organization_log_cap`, `build_deadline`). A build's lease past its end no longer renews, logs, or finishes. A build can only name images in the repository its job was given. `rollback` is refused while the database's migrations are uncertain. A removal that failed stays in front of operators, with its reason in `JobSummary::failure`, until it is reconciled or its worker reports it done. A status report's lines are stored in chunks small enough to read back.

Log lines hide a secret split over a chunk's lines, and write control characters out instead of passing them to a reader's terminal.

- **Keep control-plane tables and authentication dependencies behind the server feature so client types compile independently of database backend features.** Check isolated clients with no backend, SQLite, Postgres, both backends, and WebAssembly. _Owner:_ Ifiok Jr. · _Introduced in:_ [5e39638](https://github.com/leptoskit/leptoskit/commit/5e396387225c6d0f53e7762277214b6e54998208)
