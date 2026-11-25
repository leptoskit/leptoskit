# Hosted notifications in LeptosKit Cloud

Written 25 November 2026, against `main` at `6cb4803`. This is the contract for running [the notifications service](notifications-service.md) as part of [LeptosKit Cloud](cloud.md): one shared notifications hub per cluster, which every environment that uses notifications is a tenant of. It says what an app gets, how the control plane, the cluster's worker and the hub divide the work, and what is honestly not done. Each slice adds an "as built" section below §9.

## 1. Why a hub, and not a process per environment

An app adds notifications with one table:

```toml
[services.notifications]
crate = "leptoskit_notifications"
version = "0.3"
trust = "first-party"
```

`lk dev`, `lk serve` and `lk deploy` run that service beside the app with a database of its own. The cloud couldn't: a service with `database = true` is refused (`service_database_unavailable` beside a Postgres app, `sqlite_shared` beside a SQLite one), and a service process is never given its `LEPTOSKIT_SERVICE_*` environment. Running it per environment would also be the wrong shape for the cloud's prices:

- **It must never sleep.** Scheduled sends, recurring sends, push retries and pruning are jobs the service's own workers run every second. A hobby app sleeps when idle; its notifications can't. One always-on `xs` process per environment costs more than the app it serves.
- **The push providers want few, long connections.** APNs asks for HTTP/2 connections that are kept open and reused. One process for many apps keeps a handful open; one per environment opens one each.
- **It is our code.** The service is the same build for every tenant, so it needs none of the sandboxing tenant code gets (gVisor, its own namespace), and the operator can update it for everyone at once.

So the cloud runs the stock service as **a platform component**, like Postgres and the edge, and an environment that declares it becomes a tenant of it. Nothing changes for the app's code: the same operations at the same paths on the same hostnames, the same clients, the same `call_as_app`.

## 2. What an app gets

- **Detection.** `lk cloud deploy` reports each `[services.<name>]` to the control plane (`AppFacts`). A service whose `crate` is `leptoskit_notifications` is reported as `hosted = "notifications"`. The cloud runs no process for it. Any other service is unchanged.
- **Its own database.** Each environment's tenant has a database and a login role of its own, `ntf_` and the environment's ID in hex, on the cluster's shared Postgres server, made by the cluster's worker with the same contract as the app's database (`workers::DatabaseStep`). Tenants never share tables, so the service needs no tenant column and every isolation property the service already has holds per tenant. A SQLite app's notifications are on Postgres too.
- **The same paths.** `/_leptoskit/services/notifications/` on the environment's hostname and each of its verified custom domains is routed to the hub. The hub picks the tenant from the request's `Host`.
- **The same server wiring.** The app's process gets `LEPTOSKIT_SERVICE_NOTIFICATIONS_URL` (the hub's in-cluster address with the tenant's path, `http://<hub>:<port>/t/<tenant>`), `LEPTOSKIT_SERVICE_NOTIFICATIONS_IDENTITY=forward` unless the service is `trust = "isolated"`, and `LEPTOSKIT_SERVICE_NOTIFICATIONS_TOKEN` as a secret. The token is 32 random bytes the control plane makes for the tenant and seals in the vault; the hub is given the same token, so forwarded identities verify exactly as they do between an app and its own service.
- **Push credentials as settings.** APNs, FCM and Web Push are configured with the variables the service already reads, `LEPTOSKIT_NOTIFICATIONS_APNS_KEY_P8` and the rest, set per environment with `lk cloud secret set` or the console's variables page. These names (`notifications::SETTINGS`) are exempt from the reserved `LEPTOSKIT_` prefix, as the store credentials of a publish are, and like them they are **never given to the app's processes**: only the hub reads them. Changing one needs no deployment: the hub picks it up within a minute.
- **One more line of fine print.** A release that uses notifications says that they are served by the cluster's shared notifications service, and what it costs.

Nothing about the service's semantics changes: idempotency, the inbox, topics, segments, campaigns, retention and the push engine are the same code.

## 3. Who does what

```text
          control plane                        cluster
┌────────────────────────────────┐   ┌────────────────────────────────────────┐
│ notification_tenants (rows)    │   │ cluster worker: makes ntf_<env> on the │
│ vault: db password, token,     │──▶│   shared Postgres with each release    │
│   push credentials             │   │                                        │
│ notification_tenants op ◀──────┼───┤ hub (worker role `notifications`):     │
│ report_notifications op ◀──────┼───┤   one router + job workers per tenant, │
│ usage_totals (meter)           │   │   Host or /t/<tenant> picks the tenant │
└────────────────────────────────┘   │ edge: /_leptoskit/services/notifications│
                                     │   on each app hostname → hub           │
                                     └────────────────────────────────────────┘
```

- **The control plane never talks to the hub's tenants' databases, and never connects to the hub.** As with every worker, the hub connects outward.
- **The cluster's worker** makes and drops each tenant's database, because it is the one process with a connection that may create roles. A release carries the step as `ReleaseInput.notifications`, with the password leased under `LEPTOSKIT_NOTIFICATIONS_DATABASE_URL`, which the worker uses and never writes into the environment's Secret.
- **The hub** is `leptoskit-cloud-notifications`, a worker of a third role, `notifications`, paired to one cluster with a code from `issue_pairing`. It asks `notification_tenants` with the version of every tenant it holds; the control plane answers what changed (with its secrets, leased and recorded) and what was removed. For each tenant the hub opens the database, migrates it, and starts the service's router and job workers with that tenant's token and settings (`service::routes_with_settings`, `service::workers_with_settings`). It reports each tenant's state (serving, or failed and why) and its usage with `report_notifications`.

### Routing

The runtime gains **platform services**: `ReleaseSpec.platform`, each a name, the namespace it runs in, its host and port. A route may name one instead of a process. The manifests give each an `ExternalName` Service in the environment's namespace (the bridge the KEDA HTTP add-on already uses, which needs Traefik's `allowExternalNameServices`), route to it, and open the namespace's egress to that namespace and port and nothing more of it. Traefik passes the `Host` header through, which is how the hub knows the tenant on public paths; the app's own calls name the tenant in the path instead.

A forged `Host` or a forged path reaches another tenant's router and nothing more: every operation there wants that tenant's app token or a forwarded identity signed with a key derived from it, and anonymous calls are refused (`403`), exactly as on the internet-facing edge today.

### The tenant's lifecycle

| Event                                             | What happens                                                                                                                                                                 |
| ------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `deploy` of an app that has hosted notifications  | Refused as `notifications_unavailable` unless the cluster has a hub and a Postgres server. Otherwise the tenant's row is made once: its database's name, password and token. |
| Its release is claimed                            | The token is leased to the app's process, the database's address to the worker.                                                                                              |
| Its release is applied                            | The tenant is `ready`: the hub is given it on its next ask.                                                                                                                  |
| A notifications setting is set or unset           | The tenant's version goes up: the hub reloads that tenant.                                                                                                                   |
| A custom domain is verified or detached           | The version goes up: the hub's hostnames for the tenant change.                                                                                                              |
| A release without hosted notifications is applied | The tenant is kept, so redeploying with the service again finds its inbox. It is served only while a release that uses it is live.                                           |
| The environment is deleted                        | The tenant is `removing`: the hub drops it, and the removal job drops its database.                                                                                          |

## 4. Tables and operations

- `notification_tenants`: one row per environment (unique), with its organisation and cluster, the database's name, the sealed password and its version, the sealed token, `ready`, `serving` (a live release uses it), `removing`, `version`, and the hub's last report (`state`, `reason`, `reported_at`).
- `notification_leases`: that a hub was given a tenant's values, which version, by name, when. Kept as the record of where a value went, like `secret_leases`.
- Usage: a new meter, `notification_pushes`, counted from the hub's reports into `usage_totals`. A push is one delivery a provider accepted. What it costs is the catalogue's, as for every meter.
- `notifications { organization, project, environment }` (`view_projects`): whether the environment uses hosted notifications, the tenant's state, which providers are configured (names only), the hub's last report, and this month's pushes.
- Worker operations, for `notifications` workers only: `notification_tenants { known }` and `report_notifications { tenants, usage }`.

## 5. Security

- The hub holds every tenant's push credentials, tokens and database addresses on its cluster in memory, as the control plane's vault key can open every organisation's secrets. It runs no tenant code, its own namespace admits only the edge and tenant namespaces on its port, and its pods have no service-account token.
- A tenant's values leave the vault only for a hub paired to the tenant's own cluster, and each answer is recorded in `notification_leases` before it is sent.
- The hub sees the app's cookies on public paths, as any first-party service does. It never logs a request's headers.
- A tenant's database role can reach only its own database, as an app's can.
- Every review of a slice goes in `cloud.md` §7 under "Security review of hosted notifications …".

## 6. Honest limits

- One hub serves a cluster. Each tenant costs the hub a database pool, a live follower and job workers polling every second; a few hundred tenants per hub is the expected ceiling until tenants are sharded across hubs.
- Several hub replicas each serve every tenant (the service's jobs are claimed through the database, so they run once); a restart of a single replica is an outage of every tenant's notifications for its duration.
- The hub can't strip cookies (no plain Ingress can).
- Live connections from a custom domain: the hub doesn't check a browser's `Origin` for a tenant with custom domains, because the service takes one origin.
- A tenant whose credentials are half-configured is reported as failed with the service's own message, and serves nothing until it is fixed: the inbox goes down with push. The service refuses to start that way too.
- Notifications task J (admin compose) is not part of this.

## 7. Slices

1. This note, and platform services in the runtime (`leptoskit_cloud_runtime`).
2. The control plane: detection, tenants, the release wiring, settings, the worker operations, the meter, `notifications`.
3. The cluster worker: the tenant's database in a release and a removal.
4. The hub: `crates/leptoskit_cloud_notifications`.
5. `lk cloud notifications`, the console's section, the platform's manifests for the hub, and the guide.

## 8. Decisions for the owner

- **Price.** A push accepted by a provider is the unit. The proposal is 10,000 a month included on every plan, then $0.10 per 1,000 on paid plans; Hobby stops sending pushes at its allowance and keeps the inbox working. Prices are catalogue data, so this changes no code.

## 9. As built

### Platform services in the runtime as built (slice 1)

`leptoskit_cloud_runtime::PlatformService` is a name (a DNS label no process or other platform service has, and never `http-interceptor`), a namespace, a host and a port. `ReleaseSpec.platform` lists them; `#[serde(default)]` keeps every stored spec readable, and an empty list isn't written, so a spec without one serializes byte for byte as before. `Route.process` may name one. `ReleaseSpec::validate` checks them after the quota and before the routes: `PlatformName`, `PlatformNamespace`, `PlatformHost`, `InvalidPort`, and `DuplicateProcess` for a name a process or another platform service has.

The manifests add, for each, an `ExternalName` Service of that name in the environment's namespace pointing at the host on the port (after the processes' Services), a `NetworkPolicy` `allow-platform-<name>` opening egress to the service's namespace on that port alone, and, for a route that names it, an Ingress path whose backend is the bridge on that port. Nothing else changes: the processes keep their own Services, a sleeping app still routes through the interceptor, and the hub's own namespace decides who it admits.

`tests/manifests.rs` holds the bridge, the door and the route (`a_route_to_a_platform_service_crosses_one_bridge`) and an old spec reading and writing as before; `spec.rs` holds each refusal.

Not yet: nothing makes a spec with a platform service. The cluster's worker doesn't yet refuse a platform service it wasn't told about, as it refuses a database server that isn't its cluster's; that comes with its slice, before anything uses one. `an_app_with_services` in `tests/manifests.rs` had failed on `main` since #960 added the ACME solver's policy for an issued certificate: its count and snapshot now include it.
