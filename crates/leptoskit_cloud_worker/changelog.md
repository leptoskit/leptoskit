# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-28)

### Features

#### Cluster worker makes notifications databases

_Owner:_ Ifiok Jr. · _Introduced in:_ [22c2e0e](https://github.com/leptoskit/leptoskit/commit/22c2e0e2a9fe938b58fe8cc9b66c1c0702a55d49)

A cluster's worker now makes sure of an environment's hosted notifications database, `ntf_` and the environment's ID, with each release that uses its cluster's notifications hub, and drops it with the environment. The database and its role are made and checked as an app's managed database is. The password, leased as `LEPTOSKIT_NOTIFICATIONS_DATABASE_URL`, is used by the worker alone and never given to the release's processes or written into a Secret.

A release bridged to a platform service is refused unless that service is the hub the new `--notifications-namespace`, `--notifications-host` and `--notifications-port` options name, exactly. Without them, every release that names a platform service is refused.

#### Publish cloud CLI crates and prebuilt release binaries

_Owner:_ Ifiok Jr. · _Introduced in:_ [e47b3de](https://github.com/leptoskit/leptoskit/commit/e47b3de2192792651b71741c9094b14cf49492be)

- Registers `leptoskit_cloud_platform` (`leptoskit-cloud-platform`), `leptoskit_cloud_worker` (`leptoskit-cloud-worker`), and `leptoskit_cloud_app` (`leptoskit-cloud-control`) in `monochange.toml` and removes `publish = false` so all three crates version and publish with the main release group.
- Adds `[package.metadata.binstall]` to all three crates and extends `.github/workflows/publish.yml` to build, archive, upload, and provenance-attest prebuilt binaries for all release targets alongside `leptoskit` and `lk`.
- Bundles the console's two Latin font subsets inside `crates/leptoskit_cloud_app/assets/fonts/` so `leptoskit_cloud_app` packages cleanly for crates.io.
- Updates `lk cloud <name>` missing-binary hints to recommend `cargo binstall` and `cargo install`.

### Fixes

#### Cloud serves notifications from a shared hub

_Owner:_ Ifiok Jr. · _Introduced in:_ [78c0bf4](https://github.com/leptoskit/leptoskit/commit/78c0bf47afd1e2e6c741d5d3cb79f0692ef0b985)

An app that declares `[services.notifications]` with `crate = "leptoskit_notifications"` runs no process for it in LeptosKit Cloud. Its environment becomes a tenant of its cluster's notifications hub, a worker of the new role `notifications`:

- `lk cloud deploy` reports the service as `hosted = "notifications"`, with whether the app forwards identity. The build worker builds nothing for it.
- A deployment makes the tenant once: a database of its own, `ntf_` and the environment's ID, on the cluster's Postgres server, and a token the app and the hub share, both sealed in the vault. A cluster without a hub or a Postgres server refuses the deployment as `notifications_unavailable`.
- The release bridges to the hub as a platform service, routes `/_leptoskit/services/notifications/` on every hostname to it when the service is public, and gives the app `LEPTOSKIT_SERVICE_NOTIFICATIONS_URL`, `_IDENTITY` and, leased with the release, `_TOKEN`. The cluster's worker is leased the database's address as `LEPTOSKIT_NOTIFICATIONS_DATABASE_URL`.
- Push credentials and settings are the service's own `LEPTOSKIT_NOTIFICATIONS_*` names, set as secrets or variables. They are exempt from the reserved prefix, never given to the app's processes, and changing one reloads the tenant on the hub without a deployment.
- The hub asks for its cluster's tenants with `notification_tenants`, is given each changed one's values (recorded in `notification_leases` first), and reports how each is doing and how many pushes each sent with `report_notifications`. Pushes are the new meter `notification_pushes`.
- `notifications` tells anyone who can view the project the tenant's state, which providers have settings, and the project's pushes this month.

#### Add the cloud notifications hub

_Owner:_ Ifiok Jr. · _Introduced in:_ [abe023c](https://github.com/leptoskit/leptoskit/commit/abe023cd7fd875aff9cb3098439359bed61bd26e)

`leptoskit-cloud-notifications` serves the hosted notifications of every environment on one cluster. It runs from a worker paired for the `notifications` role. Each round, it asks the control plane which tenants changed, and opens each tenant's database, migrated, with that tenant's token and push settings. It drops the tenants it is told to, and reports how each tenant is doing and how many pushes each one's providers accepted.

The hub serves a tenant's `/_leptoskit/services/notifications/` routes in two ways: by the tenant's hostname, for the edge, and under `/t/<tenant>/`, for the app's own calls.

The worker's control-plane client gains `notification_tenants` and `report_notifications`.

The notifications service keeps a ledger of accepted pushes, `push_acceptances`, written with each acceptance. It names no person, so `forget_person` and retention leave it, and the retention sweep deletes its rows after 90 days. A host that meters pushes counts it. Run the new migration before deploying. The service's settings wrapper now passes the callers' socket leases and limits through, so a lease the app's callers set expires a socket as it does without settings.

#### Run the notifications hub on platform clusters

_Owner:_ Ifiok Jr. · _Introduced in:_ [d4f1c23](https://github.com/leptoskit/leptoskit/commit/d4f1c236b3130b7c7a9266215fc44b66fdc4054a)

`[workers] notifications_image` makes `leptoskit-cloud-platform` run each cluster's notifications hub: the `leptoskit-notifications` namespace with `Deployment/hub` and `Service/hub` on port 8080, a policy that admits the apps' namespaces on that port only, and the cluster worker told where the hub is.

The hub pairs itself from `LEPTOSKIT_CLOUD_NOTIFICATIONS_PAIRING_CODE` with `--url` and `--name` while its state isn't paired. Its image, `leptoskit-cloud-notifications`, is built from the worker's Dockerfile, and its binary ships with each release.

#### Rotate notifications credentials when a worker is revoked

_Owner:_ Ifiok Jr. · _Introduced in:_ [188e2b9](https://github.com/leptoskit/leptoskit/commit/188e2b93aa068084af3ef2791751026d83ee9465)

Revoking a cluster's worker or its notifications hub now gives every hosted notifications tenant on that cluster a new database password and token. The next deployment applies them: until then, the hub accepts the old and the new values, and the `notifications` summary reports `rotating`.

`ServiceCallers::forwarded_identity` can now be called more than once, so a service accepts people forwarded with either token while its token is being replaced.

When the cluster's worker sets a notifications tenant's database password, it now ends that role's other sessions, so a connection made with the old password doesn't outlive it. Its Postgres administrator needs superuser access or the `pg_signal_backend` role.

#### A build worker's shared build cache is bounded

_Owner:_ Ifiok Jr. · _Introduced in:_ [e5fee56](https://github.com/leptoskit/leptoskit/commit/e5fee56762346d7604212ec733e30dcd273bca21)

A build worker keeps each project's build cache on one volume that every organisation whose builds run on the worker shares. Nothing capped a cache and nothing removed another project's, so one build with a large layer could fill the volume, and every organisation's next build then failed for want of space until someone deleted directories by hand.

Now a cache export larger than `--build-cache-max-mib` is removed once its build has succeeded. Before each build, the caches of other projects that no build has used for `--build-cache-idle-days` are removed, then the least recently used until no more than `--build-cache-max-used-percent` of the volume is taken. A build never removes the cache of the project it builds, and no build's result changes: a build whose cache was removed is only slower next time.
