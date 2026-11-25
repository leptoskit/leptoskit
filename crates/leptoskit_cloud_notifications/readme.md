# leptoskit_cloud_notifications

`LeptosKit` Cloud's notifications hub (`leptoskit-cloud-notifications`). It serves the hosted notifications of every environment on one cluster from a single process.

An app that declares `[services.notifications]` with the registry crate `leptoskit_notifications` runs no process of its own for the service in the cloud. Its environment becomes a tenant of its cluster's hub instead. Each tenant has its own database on the cluster's Postgres server, made by the cluster's worker, and a token that only the app and the hub hold. Its push settings are the environment's `LEPTOSKIT_NOTIFICATIONS_*` secrets and variables, and only the hub is given them.

## Pairing

The hub is a worker of the `notifications` role. An operator issues a pairing code for the cluster with `issue_pairing` (`{ "role": "notifications", "cluster": "eu-one" }`). The hub is then paired with the worker's own command:

```sh
LEPTOSKIT_CLOUD_WORKER_PAIRING_CODE=lkp_… leptoskit-cloud-worker pair \
	--url https://cloud.example.com \
	--name eu-one-notifications \
	--state /var/lib/leptoskit-cloud-notifications
```

## Running

```sh
leptoskit-cloud-notifications --state /var/lib/leptoskit-cloud-notifications
```

| Option            | Variable                                      | What it sets                                                                                                                                                                |
| ----------------- | --------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--state`         | `LEPTOSKIT_CLOUD_NOTIFICATIONS_STATE`         | The paired state directory. A worker paired for another role is refused.                                                                                                    |
| `--listen`        | `LEPTOSKIT_CLOUD_NOTIFICATIONS_LISTEN`        | The address it serves on. `0.0.0.0:8080` when not set. The cluster's `notifications` facts and the cluster worker's `--notifications-port` name the Service in front of it. |
| `--every-seconds` | `LEPTOSKIT_CLOUD_NOTIFICATIONS_EVERY_SECONDS` | How often it asks what changed and reports. 30 when not set.                                                                                                                |
| `--connections`   | `LEPTOSKIT_CLOUD_NOTIFICATIONS_CONNECTIONS`   | The most connections it holds to each tenant's database, 1 to 20. 4 when not set.                                                                                           |

Every round, the hub asks the control plane's `notification_tenants` what changed among its cluster's tenants, sending the tenants it holds and their versions. For each tenant it is given, it reads that tenant's settings, opens and migrates its database, and starts the service's routes and job workers with the tenant's token. A newer version replaces the one before it, reusing its database pool when the address is the same, and the old version's live connections are told to reconnect. A tenant it is told to drop stops being served, and its connections are closed. Then it tells `report_notifications` how each tenant is doing, and how many pushes each one's providers accepted.

A tenant whose settings or database fail is served by no one. Its reason is reported, and is shown to the environment's owners by the `notifications` query. The hub tries again when the tenant changes, or after five minutes. Opening a tenant that takes more than two minutes counts as a failure. A reason never shows the tenant's token, its database's address or password, or its settings' values: each is replaced by `[redacted]`.

When a worker that saw a tenant's values is revoked, the tenant is given a new database password and token. Until a deployment applies them, the hub is given both sets of values. It accepts either token, and it reaches the database with whichever password the database still takes.

## Routing

- `/_leptoskit/services/notifications/…`, with a tenant's hostname as `Host` (or, over HTTP/2, as the request's authority), goes to that tenant. The edge sends these from each app hostname and verified domain. If two tenants claim one hostname, which happens only while the control plane moves it, the one opened more recently has it.
- `/t/<tenant>/_leptoskit/services/notifications/…` goes to that tenant. These are the app's own calls from inside the cluster, as `LEPTOSKIT_SERVICE_NOTIFICATIONS_URL` names it.
- `/healthz` and `/readyz` answer `200`.
- Anything else, or a tenant the hub doesn't serve, is `404`.

A tenant's routes take its own app's token, or a person its app forwarded with a key derived from that token. A forged `Host` or path reaches another tenant's routes, which refuse it as they would on the internet.

## Usage

A push counts when its provider accepts it. The service writes each accepted push to its `push_acceptances` ledger in the same transaction, naming no person, so forgetting a person or pruning deliveries never removes it; the ledger keeps a row for 90 days. The hub counts each tenant's ledger rows from the cursor the control plane holds up to a minute ago. A delivery still being written is therefore counted in the next round. The control plane counts a range only once, whichever hub reports it.

## Limits

- Every tenant runs in this one process. A tenant that sends a great deal slows the others, and the process's memory grows with the number of tenants.
- Each tenant's job workers poll its database every second.
- The process's in-process RPC server is the last tenant's opened. Only its service name, `notifications`, is read from it, and that is every tenant's.
