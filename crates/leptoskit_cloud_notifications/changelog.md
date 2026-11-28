# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-28)

### Features

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

#### A free plan's hosted pushes stop at its monthly allowance

_Owner:_ Ifiok Jr. · _Introduced in:_ [3d2887b](https://github.com/leptoskit/leptoskit/commit/3d2887b72737ef9972a8374a5a6d79b24500ff5a)

When an organisation on a plan that costs nothing has used its plan's `notification_pushes` allowance for the month, LeptosKit Cloud stops its hosted notifications from sending pushes until the next month or until it moves to a paid plan. The inbox, live updates and every other operation keep working, and `lk cloud notifications` says `Pushes are paused`. The pushes a service sends before it hears are free up to the allowance again, so they don't reach the free plan's spend cap and stop deploys; past twice the allowance they are priced and reach it, as before. A free plan that lists no `notification_pushes` allowance is billed as before. The price itself is the operator's catalogue: for example 10,000 pushes a month on every plan, then 10 cents per 1,000 on paid plans.

The control plane's `notification_tenants` answer gains `paused`, the cluster's tenants whose pushes are stopped, and the `notifications` summary gains `pushes_paused`; both default to empty and `false`. The hub stops and starts each tenant's pushes on its answer without reopening it.

`leptoskit_notifications` gains `service::push::Paused`, a switch on `service::push::Push` that a server's runner can flip while it runs. While it is on, each push is skipped with the new delivery reason `paused`, and `send_test` is refused with a `400`. Code that builds `Push` by hand adds `paused: Paused::default()`.

#### Rotate notifications credentials when a worker is revoked

_Owner:_ Ifiok Jr. · _Introduced in:_ [188e2b9](https://github.com/leptoskit/leptoskit/commit/188e2b93aa068084af3ef2791751026d83ee9465)

Revoking a cluster's worker or its notifications hub now gives every hosted notifications tenant on that cluster a new database password and token. The next deployment applies them: until then, the hub accepts the old and the new values, and the `notifications` summary reports `rotating`.

`ServiceCallers::forwarded_identity` can now be called more than once, so a service accepts people forwarded with either token while its token is being replaced.

When the cluster's worker sets a notifications tenant's database password, it now ends that role's other sessions, so a connection made with the old password doesn't outlive it. Its Postgres administrator needs superuser access or the `pg_signal_backend` role.
