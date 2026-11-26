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
4. The hub: `crates/leptoskit_cloud_notifications`. 4b. New values for a cluster's tenants when a worker that saw them is revoked.
5. `lk cloud notifications`, the console's section, the platform's manifests for the hub, and the guide.

## 8. Decisions for the owner

- **Price.** Decided by the owner on 26 November 2026: a push accepted by a provider is the unit, 10,000 a month are included on every plan, and paid plans pay $0.10 per 1,000 beyond that. Hobby stops sending pushes at its allowance and keeps the inbox working. The prices are catalogue data; stopping Hobby is code, described in "Pricing as built" below.

## 9. As built

### Platform services in the runtime as built (slice 1)

`leptoskit_cloud_runtime::PlatformService` is a name (a DNS label no process or other platform service has, and never `http-interceptor`), a namespace, a host and a port. `ReleaseSpec.platform` lists them; `#[serde(default)]` keeps every stored spec readable, and an empty list isn't written, so a spec without one serializes byte for byte as before. `Route.process` may name one. `ReleaseSpec::validate` checks them after the quota and before the routes: `PlatformName`, `PlatformNamespace`, `PlatformHost`, `InvalidPort`, and `DuplicateProcess` for a name a process or another platform service has.

The manifests add, for each, an `ExternalName` Service of that name in the environment's namespace pointing at the host on the port (after the processes' Services), a `NetworkPolicy` `allow-platform-<name>` opening egress to the service's namespace on that port alone, and, for a route that names it, an Ingress path whose backend is the bridge on that port. Nothing else changes: the processes keep their own Services, a sleeping app still routes through the interceptor, and the hub's own namespace decides who it admits.

`tests/manifests.rs` holds the bridge, the door and the route (`a_route_to_a_platform_service_crosses_one_bridge`) and an old spec reading and writing as before; `spec.rs` holds each refusal.

Not yet: nothing makes a spec with a platform service. The cluster's worker doesn't yet refuse a platform service it wasn't told about, as it refuses a database server that isn't its cluster's; that comes with its slice, before anything uses one. `an_app_with_services` in `tests/manifests.rs` had failed on `main` since #960 added the ACME solver's policy for an issued certificate: its count and snapshot now include it.

### The control plane as built (slice 2)

**Detection.** `ServiceFacts` gains `hosted` (`HostedService::Notifications` for a registry `crate = "leptoskit_notifications"` that isn't remote, Git, a path or a workspace package) and `forward_identity` (as `ResolvedService::merge` settles it). An older CLI sends neither, and its app is planned as before. `release::app_config` gives a hosted service a placeholder `url`, so `[cloud]` resolves it as one that runs elsewhere: nothing to size, nothing counted in `estimate` or the plan's limits, and the build worker builds no process for it.

**Planning.** `release::plan` refuses a hosted service under another name than `notifications` (`hosted_service_name`: the service's routes are fixed under `/_leptoskit/services/notifications/`), a public isolated one as any other, and one on a cluster whose facts lack `notifications` or `postgres` (`notifications_unavailable`). `Plan.notifications` keeps whether it is public and forwards identity, and the fine print says it is the shared service, that its database isn't backed up on a schedule yet, and that pushes count toward `notification_pushes`. A SQLite app's hosted service has its database on Postgres, so `sqlite_shared` doesn't apply to it.

**The spec.** `release::spec` adds the `notifications` platform service at the hub's namespace, host and port, a route to it on the hostname and each verified domain when the service is public, and, on the app's process alone, `LEPTOSKIT_SERVICE_NOTIFICATIONS_URL` (`http://<hub host>:<port>/t/<tenant hex>`), `LEPTOSKIT_SERVICE_NOTIFICATIONS_IDENTITY=forward` when the app forwards identity, and `LEPTOSKIT_SERVICE_NOTIFICATIONS_TOKEN` in `secret_env`. A plan with notifications and no tenant or hub is `SpecBuildError::NoNotifications`.

**Tenants.** `notification_tenants` holds one row per environment, made by `deploy` after the deployment's row (`server::notifications::ensure`): the database's name (`ntf_<environment hex>`), a 24-byte password and a 32-byte token, each sealed with the organisation's data key to a `TenantPlace` (organisation, environment, tenant row, credential version) under its own purpose and its own label, so neither opens as the other, nor as a secret or an app database's password. `ready` is set by the first applied release that uses it, `serving` by every applied release (true when it uses the service), and `removing` by `delete_environment`, which also names the database in `DestroyEnvironmentInput.notifications`. The row goes with its environment's.

**Releases.** A release that uses the service carries `ReleaseInput.notifications`, a `DatabaseStep` of shared size. When it is claimed, `release_secrets` checks the spec's platform service is where the cluster's hub is now (otherwise "deploy again"), opens both values and leases them as `LEPTOSKIT_SERVICE_NOTIFICATIONS_TOKEN` and `LEPTOSKIT_NOTIFICATIONS_DATABASE_URL`, each a `secret_leases` row naming the tenant's row and credential version. `leased_values` opens them again to hide the token and the password in what the worker reports. Rollbacks and rescales of a release that used the service point its platform service and the app's address where the hub is now (`notifications::point`), refuse as `rollback_unavailable` when the tenant or the hub is gone, and don't ask for the token among the environment's secrets.

**Settings.** `notifications::SETTINGS` are accepted as variable and secret names despite the reserved prefix, left out of `config::for_release`, so no process starts with one, and each set or unset raises the tenant's `version`. The private keys among them (`SECRET_SETTINGS`) are refused as variables. `LEPTOSKIT_NOTIFICATIONS_WEB_PUSH_EXTRA_HOSTS` is left out on purpose: on a shared hub, which hosts it may send to is the operator's to say. So does verifying or detaching a domain, which changes the tenant's hostnames.

**The hub's operations.** Both are actions that run one unserved mutation, so neither has a receipt, and both refuse any worker but a live `notifications` one (`notifications_worker_required`); a notifications worker's `claim` is refused (`notifications_worker_takes_no_jobs`), and `issue_pairing` gives the role a cluster and no organisation.

- `notification_tenants { known }`: of its cluster's tenants that are ready, serving and not removing, those it doesn't hold at their version, at most 50 a time with `more`, and those it holds that it should drop. Each answer's values are recorded in `notification_leases` (tenant, worker, version, names) before they leave. A tenant whose values can't be opened is left out and marked failed with the reason, so `notifications` shows it.
- `report_notifications { tenants, usage }`: a tenant's state is kept only for its current version, and a failure's reason only with every one of the tenant's values and the configured redactions hidden in it, and a usage count is added to `notification_pushes` only when it starts at the tenant's cursor, which then moves to its `until`. A report about another cluster's tenant is dropped. Reports are bounded: 500 of each, reasons of 500 characters, cursors of 1 to 128 that move, 100,000,000 pushes a count.

**What people see.** `notifications { organization, project, environment }` (`view_projects`): the tenant, its database's name, its state (`pending`, `unused`, `waiting`, `serving`, `failed` with the hub's reason, `removing`), which providers have a setting, the setting names, and the project's pushes this month (usage is counted per project). `set_cluster_facts` refuses to drop a cluster's hub, or its Postgres server, while it has tenants.

**Migrations.** `20261125T185009_hosted_notifications` (SQLite) and `20261125T185047_hosted_notifications` (Postgres), generated, add both tables and the `notifications` worker role. Generating them found the snapshot of `20261119T120000_runtime_log_usage` naming a column type that doesn't exist (`int8`); it now says `int64`, which changes no SQL and no checksum.

**Tests.** `release.rs` holds the plan and the spec (`hosted_notifications_are_a_platform_service_and_no_process`, `private_hosted_notifications_have_no_route`, `hosted_notifications_need_a_hub_a_server_and_their_own_name`); `vault.rs` that a tenant's values open only as themselves; `tests/support/notifications.rs`, on SQLite and Postgres, the whole story: refused without a hub, the settings, the release and its leases, the hub's answers and reports, a setting reloading the tenant, values in no other answer or table, a release without the service stopping the hub, removal, and another cluster's hub given and keeping nothing. The authorisation matrix covers the three operations.

Not yet: the cluster's worker doesn't make or drop the tenant's database (slice 3), so nothing should set a cluster's `notifications` fact until it does. Hobby's push allowance isn't enforced anywhere; it waits on the price.

### The cluster worker as built (slice 3)

**The database.** `ClusterRole::release` takes `LEPTOSKIT_NOTIFICATIONS_DATABASE_URL` out of the claim's leased values first, whatever the spec says, so the runtime is never given it and no Secret can hold it. After the app's own database, it makes sure of `ReleaseInput.notifications` through the same `Databases::ensure` (a login role that is nothing more, a database it owns made from `template0`, closed to `PUBLIC`, each checked afterwards), with that lease's password. The step must be named `notifications_database_name` of the spec's own environment, `ntf_` and its ID in hex, for both database and role, and before either database is touched the lease must log in as that role and its password must be in none of the values left for the processes (`private_lease`), so a control plane can't hand the tenant the hub's database; otherwise, or without the lease, the release is not ready before anything is applied. `postgres::ours` accepts `ntf_` names as well as `env_` ones and nothing else, and the server's other databases are closed to `PUBLIC` as before, with `ntf_` ones counted as the environments'.

**Removal.** `destroy` checks both names in `DestroyEnvironmentInput` against the environment before anything is removed, then removes the processes, then drops the app's database and the notifications one, each with its role. Each is tried even when the other couldn't be dropped, and the outcome names every failure.

**Made closed.** Every managed database, the app's and the notifications one, is now made with `ALLOW_CONNECTIONS false` and opened only after `PUBLIC` is revoked and the check passes, so no other role can connect in between, and one left half made stays closed.

**Bounds.** `Bounds.notifications` is the cluster's hub as the operator names it (`--notifications-namespace`, `--notifications-host`, `--notifications-port`, all three or none). `Bounds::reaches` refuses a release that names any platform service that isn't exactly that hub, so the control plane can't bridge an environment to another namespace or port; without the options, every release that names one is refused.

**Tests.** `cluster.rs` holds the release making both databases with their own passwords and the runtime given the app's address and never the hub's, a step naming another environment's database or missing its lease, a bridge to another hub, and a removal dropping both or, for another environment's name, nothing. `postgres.rs` holds which names are an environment's; `cli.rs` that the hub's options go together.

**Review.** An independent security review of this slice found nothing serious. Its low findings (a partial removal leaving the notifications database, the password not checked against the app's values, the window between making a database and closing it, the hub's options unchecked at startup) are fixed as above. It also noted an unchanged limit: the worker trusts which environment a job names, so a compromised control plane can still name another environment's databases; the names only keep it to environments' databases.

**Rollout.** A cluster's `notifications` facts are set only once its worker is from this slice and has the `--notifications-*` options: an older worker would apply the release without the database, and the control plane would count the tenant ready. Rotating a tenant's values when a notifications worker is revoked comes with the hub (slice 4).

### The hub as built (slice 4)

**The crate.** `crates/leptoskit_cloud_notifications` builds `leptoskit-cloud-notifications`. It runs from a worker state directory paired for the `notifications` role with `leptoskit-cloud-worker pair`, and refuses any other role. The worker's `Control` gains `notification_tenants` and `report_notifications`. `Hub::round` takes the control plane as a `Plane` and the tenants' databases as an `Open`: `Connect` in production, an in-memory map in tests.

**A round.** The hub asks with the tenants it serves at their versions, and with those that failed in the last five minutes at theirs, so a broken tenant isn't opened every round. It asks again while `more`, up to 20 times. A removed tenant is dropped: its routes go, its job workers stop, and its live connections are sent `Reconnect` and closed (`Live::drain`). A changed tenant is opened before anything is dropped, then replaces the old one, keeping the usage cursor when the answer carries none; the old one's sockets are closed the same way, so a client reconnects to the new routes and is identified under the new settings.

To open a tenant, the hub does this:

- It keeps only the settings `notifications::is_setting` names.
- It builds `PushConfig`, `OpenTopics` and `Retention` from those settings, and the push runtime when any provider is set.
- It opens the database with a pool of `--connections`, or reuses the pool it holds when the address hasn't changed, and migrates it with the dialect's migrations.
- It starts `routes_with_settings` (`ServiceCallers` knowing the tenant's token as the app, and forwarded identities keyed from it) and `workers_with_settings`.

Opening one tenant may take two minutes, after which it counts as failed, so one slow database doesn't hold up the round. A failure becomes the tenant's `Failed` reason, with the tenant's token, database address and password, and every setting value of four characters or more replaced by `[redacted]`. A database the hub can't reach says only that, because the driver's error can repeat the address.

The service's `WithSettings` wrapper, which adds the tenant's push settings to each identity, passes `lease`, `identify_token` and `admit` through to the callers, so a lease they set still expires a socket and a limit they set still holds there.

**Routing.** `Hub::router` sends `/t/<32 lower-case hex>/_leptoskit/services/notifications/…` to that tenant, with the prefix taken off and the query kept. It sends the service's prefix on any other path to the tenant whose hostname is the `Host`, lower-cased and without its port, or over HTTP/2 the URI's authority. A hostname two tenants claim reaches the one opened most recently, and the conflict is logged: the control plane gives a hostname to one environment at a time, so the other hasn't been told it lost it yet. `/healthz` and `/readyz` are the hub's own. Everything else is `404`.

**Usage.** The service keeps a ledger, `push_acceptances`: one row per accepted delivery (its id, platform and `accepted_at`), written in the transaction that marks it accepted, unique by delivery. It names no person, so `forget_person` and the operator's retention leave it; its own stage of the retention sweep deletes rows after `ACCEPTANCES_DAYS` (90). `usage::count` counts its rows with `accepted_at` after the cursor and at or before `now - LAG` (one minute). An app can't make a push go uncounted by forgetting its person or pruning its deliveries before the hub's next round. The lag is safe because the job workers that write `accepted_at` and the hub that counts run in one process, on one clock. The cursor is written `accepted:<epoch milliseconds>`. A count of zero isn't reported. A cursor the hub can't read stops that tenant's counting, rather than counting from anywhere. The cursors the control plane answers with become the hub's.

**Tests.** `tests/hub.rs` drives a scripted control plane through these: three tenants, one with half-set APNs settings; routing by path and by host (each tenant accepts its own token and refuses another's, and an unknown host or tenant gets the hub's `404`); the failed tenant held at its version; pushes counted once across rounds with the lag; a new version reopened on the same database with new hostnames; and removal. Against a real Postgres, the database the cluster worker's `Postgres::ensure` makes is migrated and served by the hub as its plain login role, and its pushes are counted. Further steps check that a new version at the same address reuses its pool, that a hostname two tenants claim reaches the later and goes back when it lets go, and that a failure quoting a setting's value shows `[redacted]`. In the service, `tests/push.rs` checks that every accepted delivery is in the ledger, that forgetting its person leaves it, and that retention prunes it after 90 days and not before.

**Review.** An independent security review found two medium and five low findings, all fixed as above: usage counted from deliveries an app could delete first (the ledger), and `WithSettings` dropping the callers' `lease` and `admit` (pass-through, and sockets closed when a tenant changes); the low ones were a new pool on each reopen, the first claimant keeping a hostname, a slow tenant stalling the round, secrets in a reason, and the lag's clock. Recorded in [cloud.md](cloud.md).

Not yet:

- A hub says it holds at most `MAX_KNOWN_TENANTS` (10,000) tenants in one ask; past that, the control plane sends the rest again each round.
- Packaging. The hub's image, release binary and Kubernetes manifests come with slice 5.

### Rotation as built (slice 4b)

**When.** `revoke_worker` of a `cluster` or `notifications` worker calls `notifications::rotate` for its cluster. The cluster's worker leased every tenant's password and token to releases, and the hub was given them, so either may hold them. Each tenant on the cluster that isn't being removed gets a new password and token, sealed to a credential version it has never had (`next_version`, beside `next_sealed_password` and `next_sealed_token`), and its `version` goes up so the hub reloads it. A second revoke before the first rotation finishes replaces the values it was moving to, as the revoked worker may have had those too. Revoking a build worker rotates nothing. A tenant whose organisation's data key can't be had is skipped rather than holding up the revoke. Each tenant's owners get an audit event, `notifications.rotate`, or `notifications.rotate_failed` when it was skipped.

**Moving.** While a tenant is moving:

- Every release leases the new values (`credentials` and `leased` record the new version in `secret_leases`), so the cluster's worker sets the new password on the role, and the app is given the new token.
- The hub is given both (`TenantSecrets.next`, recorded in `notification_leases` as `… (next)`). It takes either token, both as the app and for forwarded identities: `ServiceCallers::forwarded_identity` may now be called more than once, and a token is checked against the key whose signature it carries. It opens the database with the current password, then the new one, and checks each with `SELECT 1`, because a pool connects only when first used. It doesn't reuse its pool, whose connections may have been made with a password the database no longer takes.
- Whenever the cluster's worker sets an `ntf_` role's password, it ends that role's other sessions (`pg_terminate_backend`), so a revoked hub's open connections don't outlive the password they were made with. This needs an administrator with superuser access or `pg_signal_backend`; without it, the release fails and says so.
- Any release that leased the new values and ends, however it ends, bumps the tenant's `version` (`release_ended`), so the hub opens the database again, since it may now take only the new password.
- `leased_value` opens either version, so a worker's report hides both.
- The `notifications` summary says `rotating: true`.

**Done.** When a release whose job and attempt leased the token at `next_version` is applied, `released` makes the new values the tenant's: `credential_version` becomes `next_version`, the sealed values move over, the `next_*` columns are cleared, and `version` goes up. From then on the hub and every release are given only the new values, and the old token is refused.

**Tests.** `tests/support/notifications.rs` `revoking_a_worker_gives_its_tenants_new_values`, on SQLite and Postgres, covers these: a build worker's revoke changes nothing; revoking the hub tells the owners and gives the new hub both sets of values; a release given the new values fails and the hub is told to reopen; an applied one finishes the move, and later releases keep the new values; and revoking the cluster's own worker starts another move. The hub's `a_tenant_being_given_new_values_takes_the_old_and_the_new` takes both tokens, falls back to the new password when the old one is refused, and refuses the old token once the move is done. Its Postgres scenario checks that setting a new password ends the role's other sessions, and that the hub then opens the tenant with the new values. `leptoskit_rpc`'s `while_a_token_is_replaced_both_forward_people` checks both keys, a stranger's, and that an expired token is refused by its own key's rule.

Not yet:

- Until a release is applied, the old values still work, so a revoked worker's copies work too. The owner is asked to deploy (`rotating`), and nothing deploys for them yet.
- Push provider keys are the owner's secrets, which the hub was given. Rotating them with the provider is the owner's job, and the summary doesn't say so yet.
- A release that ends without `release_finished` (its worker lost, or reconciled as not applied) doesn't bump the tenant, so the hub reopens only on the next change.
- A second revoke while a release that leased the first rotation's values is in flight replaces those values. If that release then sets the role's password, the database and app hold values the hub isn't given, and notifications are down until the next deployment, which leases the newest values and finishes the move.
- A tenant being removed isn't rotated; its database is dropped with the removal.
- Revoking a cluster's worker doesn't replace the Postgres administrator's own credential, which that worker held and with which it could set any role's password. The operator must replace it.
- An app's own database credentials have the same gap when a cluster worker is revoked: they aren't rotated.

### Packaging as built (slice 5a)

**The image and the binary.** The worker's Dockerfile has a `notifications` target: `leptoskit-cloud-notifications` as the entry point, with `leptoskit-cloud-worker` beside it for pairing by hand, on `debian:trixie-slim` as user 65532. `cloud-images.yml` builds and publishes it as `leptoskit-cloud-notifications` for amd64 and arm64, and rebuilds when the hub or `leptoskit_notifications` changes. `publish.yml` uploads the `leptoskit-cloud-notifications` binary with each release, and its check that every CLI binary ships counts the hub's crate.

**Pairing itself.** The hub takes `--pairing-code` (`LEPTOSKIT_CLOUD_NOTIFICATIONS_PAIRING_CODE`), `--url` and `--name`. When its state isn't paired and a code is given, it pairs with `leptoskit_cloud_worker::cli::pair`, now public, before it opens the state; a paired state leaves the code unused, so a Secret that still holds a spent code does no harm. A code without `--url` and `--name` is refused before anything is spent. `Pair.role` names the role the code must be for: the hub passes `notifications`, and a key for any other role isn't kept, though the code is spent and the worker it made must be revoked. Which cluster the code is for isn't checked.

**The platform's manifests.** `[workers] notifications_image` makes `leptoskit-cloud-platform` add, on each cluster, the `leptoskit-notifications` namespace (enforcing `restricted`), `PersistentVolumeClaim/hub-state`, `Deployment/hub` and `Service/hub` on 8080, with an entry for `hub-pairing` in `required-secrets`. The hub's pod has no service-account token, a read-only root, every capability dropped, and the platform pool's placement, which falls back to the tenant pool on a cluster without one. The namespace isn't one of the platform's, which admit each other: the hub answers tenants and the internet, so nothing else of the platform admits it. Its `hub-ingress` policy admits Traefik (`kube-system`, `app.kubernetes.io/name=traefik`) and the apps' namespaces on TCP 8080 only; `database/notifications-hub` admits it to the shared Postgres's pods on 5432, and `kube-system/notifications-dns` to the cluster's DNS. The cluster worker is run with the three `--notifications-*` options naming exactly that Service, so a release bridging anywhere else is refused.

**Tests.** `tests/addons.rs` `a_notifications_image_runs_the_clusters_hub_and_tells_its_worker` checks the namespace, the Deployment's hardening and environment (the pairing code optional), the Service's port, the worker's options, the three policies, that no platform namespace admits the hub, and the Secret's entry; `without_a_notifications_image_there_is_no_hub` that nothing names the namespace without the image, and that `:latest` is refused. The hub's `a_pairing_code_is_used_only_while_the_state_is_unpaired` covers no code, a code without an address, and a paired state ignoring the code without asking any control plane.

Not yet:

- `up` doesn't wait for the hub, since it restarts until its pairing Secret exists, and doesn't set the cluster's `notifications` facts. The operator does both, in that order.
- One replica per cluster: its state is a `ReadWriteOnce` volume, and a restart is an outage of every tenant's notifications for its length.
- On a cluster without a platform pool, such as a single launch box, the hub shares nodes with tenant pods. A cluster with hosted notifications should have a platform pool.
- The hub's egress isn't limited: it must reach the control plane and every push provider on the internet.

**Review.** An independent security review found one medium and four low findings, fixed or documented as above: the namespace was first a platform namespace, which every other admitted on every port; it enforced `baseline`; a code for another role kept its key; the tenant-pool fallback; and the image not rebuilding for `leptoskit_rpc`. Recorded in [cloud.md](cloud.md).

### Pricing as built

**The catalogue.** The price is the operator's, in the catalogue at `LEPTOSKIT_CLOUD_PRICE_CATALOGUE`. For the owner's decision, every plan lists the allowance and the meter has a price in minor units (here US cents):

```json
{
	"plans": [
		{
			"id": "hobby",
			"monthly": 0,
			"allowances": { "notification_pushes": 10000 }
		},
		{
			"id": "pro",
			"monthly": 2000,
			"allowances": { "notification_pushes": 10000 }
		}
	],
	"meters": { "notification_pushes": { "minor": 10, "per": 1000 } }
}
```

Each plan keeps its other fields and allowances; only these entries are new. Allowances are counted per organisation, over all its projects, for the calendar month (UTC), as every meter is.

**Stopping a free plan.** `billing::pushes_stopped` names the organisations, among those asked about, whose plan costs nothing, lists a `notification_pushes` allowance, and has `notification_pushes` usage this month at or above it. It reads the usage rows first and loads only the organisations at or above the smallest such allowance any free plan lists, so it costs one query when nobody is near. An unpriced organisation, a paid plan, and a free plan that lists no allowance are never stopped.

`notification_tenants` answers `paused`: every served tenant of the cluster whose organisation is stopped, in every answer, so a tenant the answer doesn't name pushes again. The hub keeps one `service::push::Paused` switch per tenant, shared by its routes and its job workers, sets it before a tenant's workers start, and sets every tenant's after each answer, so stopping and starting need no reopening and nothing else of the tenant stops. `Round.paused` says which are stopped. A paused service skips each push round's deliveries with the reason `paused` (`DeliveryReason::Paused`) and refuses `send_test` with a `400`; the notification, the inbox, live updates and every other operation are untouched. Pushes start again at the next month's first answer, or within a minute of the organisation moving to a paid plan.

**Billing.** On a free plan that lists the allowance, `bill` counts no more pushes than it, so the pushes a hub sends before its next answer (at most a minute's) are never charged and never reach the free plan's cap of zero, which would stop deploys. The usage rows still hold every push. A free plan that lists no allowance is billed as before, so a catalogue without the entries above behaves as it did.

**Seen.** `lk cloud notifications` says `Pushes are paused` when the summary's `pushes_paused` is set; `pushes_paused` is `false` from an older control plane, and `paused` empty, through `#[serde(default)]`.

**Tests.** `a_free_plan_stops_pushing_at_its_allowance` (SQLite and Postgres) runs a catalogue with a Hobby allowance of 10: under it nothing is paused, at it every answer names the tenant and the summary says so, 500 more pushes leave the bill at nothing and deploys going, and the `pro` plan starts it again and bills 500 past its allowance. The hub's `a_paused_tenant_stops_pushing_until_it_is_named_no_more` opens a tenant paused, starts it, and pauses a served one, with no reopening. The service's `a_paused_service_skips_its_pushes_and_keeps_its_inbox` sends while paused (every delivery skipped as `paused`, no provider asked, the inbox row there, `send_test` refused) and again once started.

Not yet:

- `send_test` writes no ledger row, as before, so its pushes aren't counted; while a tenant isn't paused it is an uncounted way to push to one person's devices.
- Pushes aren't stopped on a paid plan at its spend cap: the cap stops deploys only, as for every meter.
- Nothing tells the organisation by email when its pushes stop.
