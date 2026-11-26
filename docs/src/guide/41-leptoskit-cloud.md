# LeptosKit Cloud

Most full-stack Rust apps slow down at the same place: you already have a working web server, database migrations, and client builds, and now you have to write Dockerfiles, wire Postgres roles, configure TLS certificates, manage secrets, and set up zero-downtime rollouts before anyone can open a URL.

**LeptosKit Cloud** ships your app straight from the `leptoskit.toml` you already have. One command creates and links the project, previews the monthly cost, uploads a deterministic source archive, builds an OCI image, provisions managed Postgres when `[db]` is configured, and serves the app over HTTPS with optional scale-to-zero.

## Choosing a deployment track

leptoskit keeps deployment simple with two primary tracks:

<!-- {=deploymentTracks} -->

| Track                         | Best for                                                                               | Primary commands                                                                     | What it manages                                                                                                                                                       |
| ----------------------------- | -------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **LeptosKit Cloud** (managed) | Shipping fast with zero infrastructure boilerplate, including from AI agents           | `lk cloud login`, `lk cloud launch`, `lk deploy`, `lk cloud status`, `lk cloud logs` | Source archiving, OCI builds, rolling releases, scale-to-zero, managed Postgres, secrets vault, custom domains, and usage metering from `[cloud]` in `leptoskit.toml` |
| **Self-hosted** (`lk infra`)  | Running on your own Hetzner, DigitalOcean, or Linux servers with full provider control | `lk infra generate`, `lk infra plan`, `lk infra apply`, `lk deploy`                  | OpenTofu VMs/networks/Cloudflare tunnels, Kamal app + Postgres containers, and encrypted Restic backups (`infra.mode = "graph"` is experimental)                      |

`lk console` is the legacy operator-enrolled deployment dashboard and agent API; new projects should pick either **LeptosKit Cloud** (`lk cloud`) or **Self-hosted** (`lk infra`).

<!-- {/deploymentTracks} -->

If you want to run everything on your own Hetzner or DigitalOcean VMs with OpenTofu and Kamal, follow [Deploying a server](24-deploying.md). For the legacy operator-enrolled console, see [Hosted deployment accounts](29-hosted-deployments.md).

## Quickstart

Start a cloud-ready project (or use an existing leptoskit project) and ship it in four steps:

<!-- {=cloudQuickstart} -->

```sh
# 1. Create a cloud-ready app (add --web-only to skip mobile/desktop runners)
lk new notes --cloud --auth --web-only
cd notes

# 2. Sign in to LeptosKit Cloud in your browser
lk cloud login

# 3. Create & link the project, preview monthly cost, and ship the first release
lk cloud launch

# 4. Ship subsequent changes, check readiness, and stream logs
lk deploy
lk cloud status
lk cloud logs --follow
```

<!-- {/cloudQuickstart} -->

`lk new notes --cloud --auth --web-only` writes a starter `[cloud]` table, skips the mobile and desktop host folders so builds stay fast, and defaults `--auth` to Postgres instead of SQLite so your local schema matches managed cloud Postgres.

When `[cloud].project` is set in `leptoskit.toml`, plain `lk deploy` automatically ships to LeptosKit Cloud—no separate deploy flag is needed.

## Day-to-day commands

Every `lk cloud` subcommand accepts `--json`, `--org <slug>`, `--project <slug>`, and `-e / --environment <name>` (defaulting to `production`).

| Command                                                                                 | What it does                                                                                           |
| --------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| `lk cloud login [--no-browser]`                                                         | Signs in via OAuth device flow; stores credentials in your user config directory (`0600`)              |
| `lk cloud launch [--name ...] [--region ...] [-y] [--detach]`                           | Creates and links a cloud project if needed, previews cost, and ships the first deployment             |
| `lk cloud deploy [--detach]` (or `lk deploy`)                                           | Packs the project (honouring `.gitignore` and `.leptoskitignore`), uploads once, builds, and rolls out |
| `lk cloud estimate`                                                                     | Shows the resolved release plan and estimated monthly cost without deploying                           |
| `lk cloud status`                                                                       | Prints the environment's live HTTPS hostname and per-process readiness                                 |
| `lk cloud logs [--follow] [--process <name>] [--since-minutes <n>]`                     | Reads or streams recent runtime logs                                                                   |
| `lk cloud open [--print]`                                                               | Opens the environment's live HTTPS URL in your browser                                                 |
| `lk cloud var list \| set <NAME> <VALUE> \| unset <NAME>`                               | Manages non-secret environment variables                                                               |
| `lk cloud secret list \| set <NAME> [--from-env <VAR>] \| unset <NAME>`                 | Stores envelope-encrypted secrets (reads from stdin or `--from-env`, never command-line arguments)     |
| `lk cloud domain list \| add <host> \| verify <host> \| remove <host> --confirm <host>` | Attaches and verifies custom HTTPS domains                                                             |
| `lk cloud env list \| create <name> \| update <name> \| delete <name> --confirm <name>` | Creates staging/preview environments and adjusts live size, replicas, or sleep behavior                |
| `lk cloud deployments` / `lk cloud rollback <number>`                                   | Lists release history and rolls back to an earlier live image                                          |
| `lk cloud db show \| backups \| backup \| restore <id> --confirm <project>/<env>`       | Inspects the managed Postgres instance and backup history                                              |
| `lk cloud notifications`                                                                | Shows whether the shared notifications service serves the environment, and its push providers          |
| `lk cloud token list \| create <name> --scope ... \| revoke <id> --confirm <id>`        | Creates scoped CI tokens (`LEPTOSKIT_CLOUD_TOKEN`)                                                     |
| `lk cloud usage [--period YYYY-MM]`                                                     | Shows monthly compute, build, database, and transfer usage                                             |
| `lk cloud publish <store>... [--submit] [--detach]`                                     | Queues store publication on a configured build worker                                                  |

An API token needs the `publish` scope to call `lk cloud publish`; the `deploy` scope alone doesn't publish to a store. The command uploads the source archive first, so the token needs `deploy` as well, as in `lk cloud token create ci --scope deploy --scope publish`.

Like `cargo`, `lk cloud` also dispatches external subcommands: any executable named `leptoskit-cloud-<name>` installed beside `lk` or on `PATH` can be run as `lk cloud <name> [ARGS]...` (for example `lk cloud platform up ...`, `lk cloud worker run ...`, or `lk cloud control serve ...` for self-hosted cloud operators).

<Callout kind="note" title="Secrets stay out of shell history">

`lk cloud secret set` never takes a secret value as a positional argument. Pipe the value over standard input (`printf '%s' "$VALUE" | lk cloud secret set STRIPE_KEY`) or pass `--from-env STRIPE_KEY`. Secret values are never printed back in JSON or logs. Secret values of eight bytes or more are hidden from runtime logs when a process prints one. A value you replace or remove after a process has started is not hidden in that process's logs until it runs the new release.

</Callout>

## Notifications

An app that declares [the notifications service](notifications/readme.md) gets it hosted, with no process of its own to pay for or keep awake:

```toml
[services.notifications]
crate = "leptoskit_notifications"
version = "0.3"
trust = "first-party"
```

`lk cloud deploy` sees the registry crate `leptoskit_notifications` and builds no process for it. The environment becomes a tenant of its cluster's shared notifications service instead, which runs the same code with a database of the environment's own. Your app's code doesn't change: the same operations answer at `/_leptoskit/services/notifications/` on the environment's hostname and its verified domains, and the server's own calls reach the service the way they do under `lk dev`. Scheduled sends, retries and pruning keep running while a sleeping app is asleep.

Push credentials are environment secrets, under the names the service reads (the [push setup](notifications/push-setup.md) page says what each holds). The private keys can only be secrets:

```sh
lk cloud secret set LEPTOSKIT_NOTIFICATIONS_APNS_KEY_P8 < AuthKey_ABC123.p8
lk cloud var set LEPTOSKIT_NOTIFICATIONS_APNS_KEY_ID ABC123
lk cloud var set LEPTOSKIT_NOTIFICATIONS_APNS_TEAM_ID DEF456
lk cloud var set LEPTOSKIT_NOTIFICATIONS_APNS_TOPIC com.example.notes
lk cloud secret set LEPTOSKIT_NOTIFICATIONS_FCM_SERVICE_ACCOUNT < service-account.json
```

These settings go only to the notifications service, never to your app's processes, and a change needs no deployment: the service picks it up within a minute. The `secrets` list in `[services.notifications]` isn't needed in the cloud. `LEPTOSKIT_NOTIFICATIONS_WEB_PUSH_EXTRA_HOSTS` can't be set there, because the shared service decides which push hosts it sends to.

`lk cloud notifications` says how it is going:

```text
Notifications: serving
Push providers: apns, fcm
Pushes this month (whole project): 1834
Database: ntf_0a1b2c3d4e5f60718293a4b5c6d7e8f9
```

A provider that is only half set up, such as an APNs key without its team ID, shows as `failed` with the service's own message, and the service serves nothing for the environment until it is fixed, inbox included. Its database isn't backed up on a schedule yet.

Each push a provider accepts counts toward the `notification_pushes` line of `lk cloud usage`, over all of an organisation's projects. Every plan includes a monthly allowance of them, and a paid plan pays per 1,000 beyond it. On Hobby, pushes stop for the rest of the month once the allowance is used: `lk cloud notifications` says `Pushes are paused`, each push that would have gone out is kept as skipped with the reason `paused`, and everything else, the inbox and its live updates included, keeps working. Pushes start again on the first of the next month (UTC), or within a minute of the organisation moving to a paid plan. The few a service sends in the minute before it hears are never charged.

## Configuring `[cloud]` in `leptoskit.toml`

Add a `[cloud]` table when you want version-controlled control over region, sizing, autoscaling, scale-to-zero, managed Postgres, or object storage buckets:

```toml
[cloud]
project = "notes"
# organization = "acme" # omit to use your personal organisation
region = "eu-central"

[cloud.database]
size = "xs" # shared Postgres; larger sizes ("s", "m", "l") are dedicated
backups = "daily" # "off" or "daily"

[cloud.storage.buckets.uploads]
public = false
max_size_gb = 20

[cloud.environments.production]
size = "s" # "xs", "s", "m", "l", or "xl"
min = 1 # minimum replicas (0 with sleep = "when-idle")
max = 3 # horizontal autoscaling ceiling (up to 20)
sleep = "never" # "never" or "when-idle"

[cloud.environments.production.services.worker]
size = "xs"
min = 1
max = 2
sleep = "never"

[cloud.environments.staging]
size = "xs"
min = 0
max = 1
sleep = "when-idle"
```

Every key in `[cloud]` is optional. Without `[cloud.environments.production]`, new apps start with a single `xs` process. See [Configuration](03-configuration.md#cloud) for the full schema reference.

## Deploying from AI agents with `lk mcp`

`lk mcp` exposes read-safe and deploy-ready cloud tools alongside the local DevTools and device controls so coding agents can verify schema health, estimate cost, ship releases, inspect status, and diagnose runtime logs without shell scraping:

- `db_check` — checks migrations and database configuration before shipping.
- `cloud_estimate` — previews the resolved release plan and monthly cost estimate.
- `cloud_deploy` — runs the same pack, upload, build, and rollout pipeline as `lk deploy`.
- `cloud_status` — reads live hostname and process readiness.
- `cloud_logs` — fetches recent runtime logs (filterable by process, time window, and substring).
- `cloud_secrets_list` — lists configured secret names and version metadata only (never values).

## Platform support

LeptosKit Cloud hosts your Rust server, web bundle, database, and backend APIs; every leptoskit client platform connects to that live HTTPS endpoint:

| Platform                             | How it works with LeptosKit Cloud                                                                                                            |
| ------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------- |
| **Web**                              | Served directly over HTTPS (SSR, CSR, or islands) from the deployed server and precompressed static assets.                                  |
| **iOS**                              | Connects to the deployed HTTPS RPC/auth endpoints; store builds can be queued via `lk cloud publish` when a macOS worker pool is configured. |
| **Android**                          | Connects to the deployed HTTPS RPC/auth endpoints; store builds can be queued via `lk cloud publish` on a configured publish worker.         |
| **macOS**                            | Connects to the deployed HTTPS RPC/auth endpoints.                                                                                           |
| **Windows**                          | Connects to the deployed HTTPS RPC/auth endpoints.                                                                                           |
| **Linux**                            | Connects to the deployed HTTPS RPC/auth endpoints.                                                                                           |
| **The terminal**                     | Connects to the deployed HTTPS RPC/auth endpoints.                                                                                           |
| **HarmonyOS (experimental)**         | Connects to the deployed HTTPS RPC/auth endpoints; no HarmonyOS cloud build pool exists yet.                                                 |
| **Wear OS / watchOS (experimental)** | Watch surfaces run locally and can call deployed HTTPS endpoints where the host allows networking; no watch cloud build pool exists yet.     |

## Where things stand

<!-- {=cloudLimits} -->

- **Control plane & cluster packaging:** the control plane (`leptoskit-cloud-control`), cluster/build workers (`leptoskit-cloud-worker` with its built-in `build-app` OCI builder), Dockerfiles, GHCR multi-arch workflow, Kubernetes manifests, and the `leptoskit-cloud-platform up` operator orchestrator are implemented and covered by unit, HTTP, Postgres, and manifest snapshot tests.
- **Live cluster validation:** end-to-end provisioning on live Hetzner, Cloudflare, S3/R2, and Stripe accounts is still gated on operator acceptance runs (live gVisor isolation, sleep/wake traffic, WAL backup/restore, and Stripe checkout verification).
- **Databases & storage:** shared managed Postgres (`size = "xs"`) is provisioned automatically per environment; dedicated database sizes (`s`, `m`, `l`), automated cluster-worker dump/restore, and automatic bucket provisioning for `[cloud.storage.buckets.<name>]` are still in progress.

<!-- {/cloudLimits} -->
