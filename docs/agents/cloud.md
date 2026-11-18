# LeptosKit Cloud

Engineering note, 18 November 2026. LeptosKit Cloud is a hosted platform that runs any leptoskit app. A developer signs up and runs one command, or presses one button, and the cloud runs their server, database, web client, over-the-air updates and store submissions from the `leptoskit.toml` they already have.

This note is the contract every cloud pull request builds against. When a slice finds it wrong, the slice stops and the note changes first. Only the foundation is built; §12 says what exists and what each part's evidence is.

## 1. Why a separate product

The repository already deploys apps in three ways, and all three keep working unchanged:

| Existing                                                      | What it does                                                                 |
| ------------------------------------------------------------- | ---------------------------------------------------------------------------- |
| `lk infra` and `lk deploy` (`crates/leptoskit_cli/src/infra`) | The app author provisions their own VM with OpenTofu and Kamal.              |
| `lk console serve` (`leptoskit_deploy_console::control`)      | A self-hosted deployment service for one owner.                              |
| The hosted pilot (`leptoskit_deploy_console::hosted`)         | An operator-enrolled broker: customers bring their own compute and a worker. |
| `lk publish` (`crates/leptoskit_cli/src/publish`)             | Uploads to six stores from the developer's machine.                          |

The cloud adds public sign-up, organisations and teams, shared compute on Kubernetes, cloud builds, managed Postgres, custom domains, metering and billing, and store publishing from cloud build machines. [The deployment control plane note](deployment-control-plane.md) said, under "Hosted operation", not to turn the single-owner executor into a multi-tenant one by adding an organisation ID. The cloud is the separate design it asked for.

What makes it ours rather than another host:

1. **One file.** `leptoskit.toml` already describes the server, database, services, native apps and stores. `[cloud]` adds only placement and scale, so nothing is declared twice.
2. **One release, many targets.** A release is the server deployment, the over-the-air update and the store builds, with one version and one rollback. A fingerprint of the native inputs decides whether a change can go over the air or needs a store build.
3. **Show the workings.** Every screen can reveal the config table, the Kubernetes objects, the build steps and the cost behind it, as [the admin design](admin-design.md) does. The cost shows before a deploy, and a spend cap is on by default.
4. **Honest limits.** Regions, the provider, cold-start time and what happens at a cap are stated. Rollback exists from the first release; [Serverpod Cloud](https://www.serverpod.dev/cloud) has none.
5. **Store releases signed by the developer's wallet.** The Solana dApp Store needs the publisher's wallet to sign prepared transactions. The cloud prepares them and the developer signs; the cloud never holds the wallet.
6. **Agents first.** Every action is a typed RPC operation with an OpenAPI export, a generated CLI and scoped tokens.

## 2. Crates

| Crate                                  | Published | What it holds                                                                                                                                                                                     |
| -------------------------------------- | --------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `crates/leptoskit_cloud`               | yes       | The shared types, the source archive and the price estimate today; tables, operations (server bodies behind a `server` feature) and the worker protocol later. Safe for clients without features. |
| `crates/leptoskit_cloud_runtime`       | yes       | `ReleaseSpec` as plain data, the `Runtime` trait, Kubernetes manifests, the cluster adapter, a local process adapter and a recording fake. Depends on nothing else in the cloud.                  |
| `crates/leptoskit_cloud_platform`      | no        | The operator side: platform configuration, OpenTofu and k3s bootstrap, add-on manifests, the price catalogue and the cost model.                                                                  |
| `crates/leptoskit_cloud_worker`        | no        | `leptoskit-cloud-worker`, with a `build` role (builds and store publishing) and a `cluster` role (applies releases inside one cluster). Both share one pair, claim and finish loop.               |
| `crates/leptoskit_cloud_app`           | no        | The cloud's own leptoskit app: sign-up, the dashboard, and `leptoskit-cloud-control`, the server. It renders with SSR and hydrates, with no hand-written JavaScript.                              |
| `crates/leptoskit_cli/src/cloud/`      | CLI       | `lk cloud …`.                                                                                                                                                                                     |
| `crates/leptoskit_config/src/cloud.rs` | config    | The `[cloud]` table and its validation.                                                                                                                                                           |

Dependencies point one way: `leptoskit_cloud_runtime` ← `leptoskit_cloud` ← the worker, the app and the CLI. `leptoskit_cloud_platform` depends on `leptoskit_cloud_runtime` only. No core crate (`leptoskit`, `leptoskit_native`, …) depends on a cloud crate. The pilot's existing `leptoskit-cloud` binary in `leptoskit_app` keeps its name.

### Where the shared types live

`Slug`, `Size`, `Scale`, `Sleep`, `DatabaseSize`, `Backups` and `Region` are defined once, in `leptoskit_config::cloud`, and `leptoskit_cloud::slug` and `leptoskit_cloud::size` re-export them. Config has to validate `[cloud]` with these types, and `leptoskit_cloud` reads the resolved table for the estimate, so the definitions sit in the crate both already depend on. Putting them in `leptoskit_cloud` would have made config depend on a cloud crate, which the rule above forbids. Config builds for wasm with light dependencies, so the dashboard pays nothing extra.

## 3. Technology choices

- **API.** `leptoskit_rpc` operations over `leptoskit_db`, as `leptoskit_notifications` does: typed operations, live queries for deploy timelines and logs, durable jobs with retries and cron for builds and billing, idempotency keys, and an OpenAPI export. The control plane is an app, not a `[services]` service, so its operations carry no `service = "…"`.
- **Database.** Postgres in production and SQLite for tests and single-node self-hosting. Both migration directories ship.
- **Identity.** `leptoskit_auth` for people (email codes, passkeys, GitHub and Google, Solana wallets), and `leptoskit_idp` for the CLI's device-code sign-in. Organisations, memberships, invitations and API tokens are cloud tables, not additions to `leptoskit_auth`.
- **Runtime.** Kubernetes, one namespace per environment. Tenant pods run under a [gVisor](https://gvisor.dev/docs/architecture_guide/platforms/) RuntimeClass with a default-deny NetworkPolicy, a ResourceQuota, no service-account token and the restricted Pod Security profile. Each `[services.<name>]` becomes its own Deployment and Service. [KEDA](https://keda.sh/) scales, with its [HTTP add-on](https://github.com/kedacore/http-add-on) for sleeping when idle.
- **Nodes.** [k3s doesn't run embedded etcd across its multicloud WireGuard overlay](https://docs.k3s.io/networking/distributed-multicloud), so control-plane servers share one private network, and workers join from anywhere over `wireguard-native`. Launch is one bare-metal server; growth is three small VMs as the control plane with bare-metal workers. Each node admits only the peers and ports the topology needs, plus administrator addresses on SSH and the API. Cloud nodes get this from a provider firewall and existing machines from a generated nftables file. Cloud nodes' addresses exist only after `tofu apply`, so generation has two phases: the OpenTofu configuration, then bootstrap and peer lists rendered from it and OpenTofu's outputs.
- **Cluster access.** The control plane never holds a cluster's credentials. A `cluster` worker runs inside each cluster with a scoped ServiceAccount, connects outward, and claims release jobs. The same mechanism attaches a customer's own cluster later.
- **Builds.** Customer code never builds in the control plane or on a shared daemon. A `build` worker runs each job in a fresh sandbox (an ephemeral VM, or a gVisor pod with rootless BuildKit) and destroys it afterwards. macOS workers are a separate pool.
- **Tenant Postgres.** [CloudNativePG](https://cloudnative-pg.io/). By default each environment gets a database and a login role on a shared cluster; paid sizes get their own instance. WAL goes to object storage.
- **Edge.** Cloudflare in front, default hostnames under a wildcard, and custom domains through [Cloudflare for SaaS](https://developers.cloudflare.com/cloudflare-for-platforms/cloudflare-for-saas/), or cert-manager for an operator without Cloudflare. Object storage is S3-compatible (R2 or [B2](https://www.backblaze.com/cloud-storage/pricing)) through the existing `rusty-s3` seam.
- **Money.** Stripe: a flat plan price plus metered usage. Prices are operator configuration, never constants in code. Reaching the spend cap stops new work and sleeps what can sleep. It never deletes data.
- **Secrets.** Envelope encryption with AES-256-GCM-SIV, as `leptoskit_auth` and `leptoskit_deploy_console` already use. Each organisation has a data key wrapped by a master key from the environment; a KMS adapter can replace that later. Organisation, project and secret name are authenticated data. A value leaves the vault only as a lease for one job on a single-tenant worker.

The launch substrate is self-managed k3s on one [Hetzner AX41](https://www.hetzner.com/dedicated-rootserver/matrix-ax/) (about €59 a month, and bare metal, so `/dev/kvm` exists), gVisor, CloudNativePG, Cloudflare's free plan plus Cloudflare for SaaS, R2 or B2, and owned Mac minis for Apple builds. That puts a sleeping hobby app at roughly $0.15–0.30 a month and an always-on small app at $3–4. None of it is hard-coded: the platform crate reads it as configuration.

## 4. Vocabulary

- **Organisation**: the billing and membership boundary. Signing up creates a personal one.
- **Member**: a person in an organisation, with a role: `owner`, `admin`, `developer`, `billing` or `viewer`.
- **Project**: one leptoskit app, one `leptoskit.toml`. Its slug is unique within the organisation.
- **Environment**: a running copy of a project, such as `production`, `staging` or a preview. It owns its database, secrets, domains and scale.
- **Deployment**: one attempt to put one build of the server into one environment. Immutable once created, and numbered per environment.
- **Build**: one run on a build worker, for the server or a native target.
- **Release**: a version of the whole app: a deployment, plus an optional over-the-air update and store submissions.
- **Worker**: a `build` or `cluster` process paired with the control plane.
- **Cluster**: a place environments run, in a region.

Identifiers are UUID v7. A slug is 3 to 40 characters: lower-case letters, digits and single hyphens, starting with a letter and ending with a letter or digit. `api`, `www`, `admin`, `cloud`, `console`, `app`, `status` and `docs` are reserved. `Slug::suggest` turns a name such as "My Notes App" into `my-notes-app`.

### Sizes

| Size | CPU request / limit (millicores) | Memory (MiB) |
| ---- | -------------------------------- | ------------ |
| `xs` | 100 / 500                        | 128          |
| `s`  | 250 / 1000                       | 256          |
| `m`  | 500 / 2000                       | 512          |
| `l`  | 1000 / 2000                      | 1024         |
| `xl` | 2000 / 4000                      | 2048         |

These are the defaults in code, as methods on `Size`; an operator override can come later. A `Scale` holds `min` and `max` with `min ≤ max`, `1 ≤ max ≤ 20`, and `min = 0` meaning the process sleeps when idle. `DatabaseSize` is `shared` (a database on the shared cluster) or `s`, `m` or `l`, dedicated instances with 1, 2 and 4 GiB.

### Deployment states

```text
queued → building → migrating → rolling_out → live → superseded
                 ↘        ↘            ↘
                  failed { stage, blocker }      cancelled (from queued or building)
```

- A rollback is a new deployment whose source names an earlier one, and it goes from `queued` straight to `migrating`. `DeploymentState::can_become` allows that edge; `DeploymentState::advance` also checks the source, so an upload can't skip its build.
- `failed` records the stage it failed in and a `Blocker`: `build_failed`, `builder_unavailable`, `schema_incompatible`, `migration_failed`, `not_ready`, `quota_exceeded`, `spend_cap_reached`, `payment_required`, `cluster_unavailable` or `interrupted`. Each has a stable code for errors and the wire.
- A worker that dies mid-job leaves the deployment `failed` with `interrupted` only after an operator or the reconciler has confirmed what the cluster is actually running. An expired lease doesn't prove the old process stopped; see "Durability" in [the control plane note](deployment-control-plane.md).
- The diagram has no edge from `queued` to `failed`. A deployment that can't get a builder moves to `building` first and fails there with `builder_unavailable`.

### Deployment sources

`upload { archive }` (a source archive's digest), `git { repository, commit }` (an `https` address and a full commit ID), `template { name }` (the cloud scaffolds a starter with `lk new`, for a first deploy with no local setup), and `rollback { deployment }`.

### The source archive

The CLI packs and the build worker unpacks, both with `leptoskit_cloud::archive` behind its `archive` feature.

- **Which files.** `git ls-files --cached --others --exclude-standard` in a Git checkout, otherwise a walk that honours `.gitignore`. A `.leptoskitignore` at the project's root excludes more, with `.gitignore` syntax. `target/`, `.leptoskit/`, `.git/`, `dist/` and `node_modules/` are always left out, at any depth.
- **Deterministic.** Sorted paths, zero modification times and owners, modes of 0644 or 0755, and a gzip header with no time or OS. The digest is the SHA-256 of the compressed bytes, written `sha256:<hex>`. Two packs of the same files give the same bytes on the same build of the crate; a different compressor version may not.
- **Bounds.** At most 20,000 files and 512 MiB uncompressed; a file over 64 MiB fails with its path.
- **Paths.** Absolute paths, `..` components, backslashes and symlinks that leave the root are refused on both sides. A link is followed through the archive's own other links, never the filesystem, so packing and unpacking judge it the same way, and a loop is refused after 40 hops.
- **Secrets.** `.env`, `.env.*`, `*.pem`, `*.p8`, `*.p12`, `*.jks`, `*.keystore` and `credentials*.toml` are refused, naming every such file and pointing at `lk cloud secret set`, unless `.leptoskitignore` already leaves them out. A file Git ignores never reaches the list.
- **Unpacking** goes into an empty or missing directory, creates each file only if it doesn't exist, counts directories towards the file limit, checks sizes against the headers before writing, and never runs anything. Links are checked after every entry is read and created last. Hard links, devices and FIFOs are refused, as are Git submodules when packing.

## 5. `[cloud]` in `leptoskit.toml`

```toml
[cloud]
project = "notes" # written by `lk cloud launch`
organization = "acme" # optional; the signed-in person's own organisation by default
region = "eu-central" # optional; the cloud's default region by default
url = "https://cloud.leptoskit.dev" # optional; for a self-hosted cloud

[cloud.database]
size = "shared" # shared | s | m | l
backups = "daily" # daily | continuous

[cloud.environments.production]
size = "s" # xs | s | m | l | xl
min = 1
max = 3
sleep = "never" # never | when-idle; when-idle needs min = 0
domains = ["notes.example.com"]

[cloud.environments.production.services.search]
size = "m"
min = 1
max = 2
```

`CloudConfig::resolved` turns this into a `ResolvedCloud`, where every default is filled in and every rule below has held. `LeptoskitConfig::validate` runs it whenever the config loads, so every command refuses a bad table with `invalid [cloud]: …` and the table and key at fault.

- **Defaults.** With only `[cloud] project = "…"`, production runs at `xs`, `min = 0`, `max = 1`, sleeping when idle, on a shared database when the app has `[db]`. `production` always exists, even when no table names it.
- **Filling in.** Writing only `sleep` sets `min` (0 for `when-idle`, 1 for `never`); `max` defaults to `min`, or 1. `sleep = "when-idle"` with `min > 0`, `sleep = "never"` with `min = 0`, `min > max`, `max = 0` and `max > 20` are errors.
- **Databases.** `[db]` decides whether there is a database and which dialect. `[cloud.database]` without `[db]` is an error. A SQLite app gets a persistent volume and one replica, so `max > 1` is an error that names Postgres; that holds for a service with `database = true` on SQLite too. SQLite can't take a dedicated size or continuous backups.
- **Services.** `[services.<name>]` decides the processes. A cloud table for a service that doesn't exist is an error, and remote services (`url = …`) take no cloud settings. A service without a table follows the app: it sleeps when the app sleeps and keeps one replica when the app keeps any.
- **Live connections.** Sleeping closes live connections. When the app shows it serves them (`rpc.live.relay_url_env`, `rpc.live_path`, or `db.live_capture = "replication"`), the default becomes one replica that never sleeps, and an explicit `when-idle` or `min = 0` is refused unless the environment says `allow_sleep_with_live = true`. The error names the setting it found.
- **Domains.** Lower-case, fully qualified hostnames with no wildcard, port or trailing dot. The same hostname can't appear twice in one environment or in two environments. `leptoskit.app`, `leptoskit.dev` and their subdomains are the cloud's own and are refused.
- **Names.** Environment names are slugs, so `qa` is too short. `url` must be `https`, or `http` on localhost, with no path, query, fragment or credentials.
- **Alongside `[infra]`.** `[cloud]` and a managed `[infra]` can coexist: `lk deploy` uses the cloud when `[cloud].project` is set, and `lk infra …` keeps addressing the app's own infrastructure. `[cloud]` together with `infra.control_url` is an error, because both would claim `lk deploy`.
- Spend limits, secrets and billing never live in the repository.

## 6. Operations

Operations are declared in `leptoskit_cloud` with `#[rpc::query]`, `#[rpc::mutation]` and `#[rpc::action]`. Each one authorises on the server from the caller's identity and membership, and never trusts an organisation or project named in its arguments without checking.

| Module                       | Operations                                                                                                                                                                                      |
| ---------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `account`                    | `whoami`, `organizations`, `create_organization`, `members`, `invite_member`, `accept_invitation`, `set_member_role`, `remove_member`                                                           |
| `tokens`                     | `tokens`, `create_token` (the value is shown once), `revoke_token`                                                                                                                              |
| `projects`                   | `projects`, `project`, `create_project`, `delete_project` (type to confirm), `environments`, `create_environment`, `update_environment`, `delete_environment`                                   |
| `deployments`                | `begin_upload` (a presigned PUT for an archive digest), `deploy`, `deployment`, `deployments` (live), `deployment_events` (live), `cancel_deployment`, `rollback`, `runtime_logs`, `build_logs` |
| `config`                     | `variables`, `set_variable`, `unset_variable`, `secrets` (names only), `set_secret`, `unset_secret`                                                                                             |
| `domains`                    | `domains`, `attach_domain`, `verify_domain`, `detach_domain`                                                                                                                                    |
| `databases`                  | `database`, `database_backups`, `create_backup`, `restore_backup` (with confirmation)                                                                                                           |
| `billing`                    | `plans`, `subscription`, `checkout`, `portal`, `usage`, `estimate` (the cost of a `[cloud]` table before deploying), `set_spend_cap`                                                            |
| `stores`                     | `store_connections`, `connect_store`, `disconnect_store`, `publish`, `store_releases` (live), `store_release`, `signing_request` and `submit_signature` (wallet signing)                        |
| `workers` (worker keys only) | `pair`, `claim`, `renew`, `append_log`, `finish`                                                                                                                                                |
| `operator` (operators only)  | `clusters`, `register_cluster`, `workers`, `issue_pairing`, `revoke_worker`, `reconcile`                                                                                                        |

- Mutations that start work take an idempotency key and return the same result for a repeat.
- Destructive operations take an explicit confirmation argument, and scoped tokens can't call them unless the scope names them.
- Errors are `rpc::Error` variants whose message starts with the blocker's stable code.
- Secrets are write-only. No operation returns a secret's value.
- Token scopes are `read`, `deploy`, `manage` (config and domains), `publish`, `billing` and `admin`. A token belongs to an organisation and optionally one project, is hashed at rest, is shown once, and expires by default.

## 7. The worker protocol

- **Pairing.** An operator, or later an organisation for its own workers, issues a one-use pairing code that expires in ten minutes. `pair { code, role, capabilities }` returns a worker key, stored hashed. A worker belongs to the shared pool or to one organisation.
- **Claiming.** `claim` long-polls and returns at most one job matching the worker's role, pool, region and capabilities, with a lease. `renew` extends the lease, `append_log` streams bounded log chunks, and `finish { job, outcome }` is idempotent.
- **Crash safety**, as the hosted pilot's worker does it: the worker writes a pending marker before running a job and the result before acknowledging it. A marker without a result means the effect is unknown, so the worker refuses to repeat the job and it is reconciled rather than replayed. Builds are the exception: their only effect is artifacts keyed by digest, so an interrupted build is retried.
- **Jobs.** `build_server { archive, toolchain, targets }` returns an image digest, web assets and migration fingerprints. `build_native { platform, … }` returns an artifact. `publish { store, submit, leases }` returns a "What to do next" report (`publish::steps::Report`). Cluster workers take `release { spec }`, `scale`, `destroy_environment`, `fetch_logs` and `sample_usage`.
- **Secret leases.** A job that needs a credential gets a lease naming the secret versions. The lease is recorded before release, delivered in the claim response over TLS, and valid for that job only. Where a store's API allows it, the control plane mints a short-lived token so the worker never sees the long-lived key.
- **Tenancy.** A build worker runs one job at a time in a fresh sandbox and keeps no cache another organisation's job can read.

## 8. `ReleaseSpec` and the runtime

`leptoskit_cloud_runtime::ReleaseSpec` is the whole desired state of one environment as data, with no reference to plans or prices:

- identity: the organisation, project and environment slugs and IDs, and the deployment number;
- processes: the app, any split roles, and one per service, each with an image digest (never a tag), command, port, health paths, environment variables, secret references, CPU and memory, scale and volumes;
- routes: hostnames and path prefixes to processes, including `/_leptoskit/services/<name>/` for public services and cookie stripping for isolated ones;
- database bindings: which secret holds each process's `DATABASE_URL`;
- policy: the runtime class, egress rules and quota.

`Runtime` is the boundary between the cloud and wherever it runs:

```rust
pub trait Runtime {
	async fn apply(&self, spec: &ReleaseSpec) -> Result<Applied, RuntimeError>;
	async fn status(&self, environment: &EnvironmentRef) -> Result<Observed, RuntimeError>;
	async fn logs(&self, environment: &EnvironmentRef, query: &LogQuery) -> Result<Vec<LogLine>, RuntimeError>;
	async fn destroy(&self, environment: &EnvironmentRef) -> Result<(), RuntimeError>;
}
```

The adapters are Kubernetes (server-side apply, then a watch until the rollout is ready or has failed), local processes for development and the end-to-end test, and a recording fake. Rollouts replace pods with `maxUnavailable: 0`, so a release that never becomes ready leaves the previous one serving.

## 9. The first deploy

```text
lk cloud login       # device code: opens the browser, or prints a URL and code
lk cloud launch      # in a project directory
```

`launch`:

1. reads `leptoskit.toml` and prints what it found: server, database dialect, services, platforms and stores;
2. asks for an `estimate` and prints the monthly cost and the spend cap;
3. creates the project and its production environment, idempotently, and writes `[cloud] project = "…"`;
4. packs the source archive and uploads it by digest, skipping the upload when the cloud already has it;
5. creates the deployment and follows its events: `Uploaded`, `Built`, `Migrated`, `Live`;
6. prints the address, the dashboard link, and what to do next (a custom domain, secrets it noticed in `.env`, stores).

On the web the same thing is: sign up, press **New project**, then **Deploy a starter** (the cloud scaffolds and deploys a template), **Import from GitHub**, or copy the two commands. Later deploys are `lk deploy`. `lk cloud` also has `status`, `logs`, `open`, `env`, `secret`, `domain`, `scale`, `rollback`, `db`, `publish`, `usage`, `token`, `project`, `org`, `whoami` and `logout`, and every command takes `--json`.

The CLI keeps its tokens in a private file under `$XDG_CONFIG_HOME/leptoskit/cloud/` with mode 0600, as the console client does, or reads `LEPTOSKIT_CLOUD_TOKEN` in CI.

## 10. Store publishing

`lk cloud publish [stores] [--submit]` asks the cloud to build each native target on a worker and run the same store clients `lk publish` uses, with credentials from the vault. The result is the same "What to do next" report, shown in the dashboard as a checklist of `agent` and `person` steps.

New stores land in `lk publish`, so they work locally too, in this order: the Solana dApp Store; Huawei AppGallery for Android and HarmonyOS; the Microsoft Store; the Samsung Galaxy Store; the Meta Horizon Store; itch.io and Steam for the game host; then the Amazon Appstore and the Snap Store. Each follows the existing shape: a `[publish.<store>]` table, a client over the `Http` or `Runner` seam, credentials in `NEEDED`, dry-run checks, steps, and tests against a fake server.

## 11. Pricing

Prices are data. The platform crate's catalogue holds each price with its cost basis, source and date, and a target margin; the control plane reads plans and meters from operator configuration and Stripe price IDs from the environment.

`leptoskit_cloud::pricing` holds the shape and the arithmetic, and no prices:

- A `Catalogue` has a currency, plans, and a unit price per `Meter`. It is validated on load: at least one plan, unique plan slugs, a name for every plan, and no allowance for a meter without a price.
- Meters are compute seconds per size, dedicated database hours per size, database and object storage in MB-months, egress in MB, Linux and macOS build minutes, and custom hostname months. Each has a stable code such as `compute_seconds_s`.
- Amounts are integers in the currency's minor unit. A `UnitPrice` is `minor` per `per` units, so a fraction of a cent per second needs no floats. Each line rounds half up once, after the plan's allowance comes off; the total is the plan price plus the usage beyond what the plan includes. Arithmetic saturates instead of overflowing.
- `EstimateInput::from_cloud` turns a resolved `[cloud]` table into a month of usage, and writes down every assumption it made in a sentence the dashboard and CLI can show. A month is 730 hours. A sleeping process is assumed awake 10% of the time, a database to hold 500 MB, and the project to keep 1000 MB in object storage, send 10,000 MB out and build for 100 Linux minutes. Every assumption is a field the caller can change.
- `estimate` prices that usage under one plan. A meter the catalogue doesn't price costs nothing, and the estimate says so; a plan priced by agreement shows usage alone.

Proposed plans, for the owner to decide:

| Plan  | Price                                              | Includes                                                                                                                       |
| ----- | -------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| Hobby | free                                               | one project, size `xs`, sleeps when idle, a shared database up to 500 MB, 100 Linux build minutes, default hostname, hard caps |
| Pro   | $20 a month plus usage, with $20 of usage included | teams, custom domains, autoscaling, always-on, continuous backups, store publishing                                            |
| Scale | by agreement                                       | dedicated nodes, or a customer's own cluster                                                                                   |

Suggested unit prices, from comparing [GitHub's runner prices](https://docs.github.com/en/billing/reference/actions-runner-pricing) and [Expo's](https://expo.dev/pricing): $0.005 a Linux build minute and $0.04 a macOS minute. An always-on `m` app with a 1 GB database comes to about $12–15 a month in all.

## 12. Status

| Slice                                            | Status and evidence                                                                                                                                                                                                                                                                                                                                                                                                    |
| ------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Foundation: `[cloud]`, its validation and schema | built; 13 unit tests in `crates/leptoskit_config/src/cloud/` (slugs, sizes, scales, regions, domains) and 21 in `crates/leptoskit_config/tests/cloud.rs`: every rule in §5 with its message, the contract's example, unchanged serialization without `[cloud]`, and the JSON schema accepting the example and refusing bad values                                                                                      |
| Foundation: deployment states, blockers, sources | built; `crates/leptoskit_cloud/src/deployment.rs`, including a test over every pair of the 37 states (seven, plus `failed` for each of three stages and ten blockers) against the diagram                                                                                                                                                                                                                              |
| Foundation: the source archive                   | built; 8 unit tests and 14 in `crates/leptoskit_cloud/tests/archive.rs`: the walk, a real Git checkout, `.leptoskitignore`, identical bytes from two packs, the round trip with modes and links, secrets, bounds, and a hand-built hostile tar (absolute and `..` paths, escaping and chained links, a loop, writing through a link, collisions, hard links, FIFOs, lying headers). Unix only; Windows isn't exercised |
| Foundation: the price estimate                   | built; 9 tests in `crates/leptoskit_cloud/tests/pricing.rs` with made-up prices: allowances, rounding, included usage, a plan by agreement, an unpriced meter, saturation, and catalogue validation                                                                                                                                                                                                                    |
| Foundation: client safety                        | built; `crates/leptoskit_cloud/tests/dependencies.rs` reads the wasm dependency tree with and without `archive`, and clippy runs for `wasm32-unknown-unknown`                                                                                                                                                                                                                                                          |
| Platform                                         | offline configuration, OpenTofu, two-phase k3s bootstrap and Kubernetes add-on generation built in `leptoskit_cloud_platform`; deterministic launch/growth snapshots, secret-reference inventory and CLI tests. [Operator procedure](cloud-platform.md) records chart sources and unvalidated live checks. No cluster provisioned; price catalogue and cost model remain unimplemented.                                |
| Runtime 1                                        | desired-state types, validation, deterministic Kubernetes manifests and recording fake built in `leptoskit_cloud_runtime`; unit tests, manifest snapshots and the feature-free wasm dependency check. Quotas include one surge pod per rolling process (`Quota::for_processes`); tests cover minimum xs rollout capacity and Recreate. No manifests applied to a real cluster.                                         |
| Runtime 2                                        | quota correction built; cluster adapter not implemented yet. Status will report observed restart counts from the current ReplicaSet's existing pods and the latest retained OOM termination time; replaced pods take their counts with them. Durable historical totals are a later slice. Sleep routing and platform-profile corrections remain outstanding.                                                           |
| Control 1–4, Worker 1–3, CLI 1–3, Design, UI 1–3 | not started                                                                                                                                                                                                                                                                                                                                                                                                            |
| Stores, end to end, public docs                  | not started                                                                                                                                                                                                                                                                                                                                                                                                            |

The slices, one pull request each:

- **Foundation:** the shared types, the archive, `[cloud]` and its schema, and this note.
- **Runtime 1:** `ReleaseSpec`, `Runtime`, Kubernetes manifests, the fake and local adapters. **Runtime 2:** the cluster adapter over `kube`.
- **Platform:** platform configuration, OpenTofu and k3s bootstrap, add-ons, the price catalogue and the cost model.
- **Control 1:** tables and migrations, sign-in and device login, organisations, tokens, projects, environments and the server binary. **Control 2:** uploads, deployments, the worker protocol, events and logs. **Control 3:** variables, the secrets vault, domains and databases. **Control 4:** metering, Stripe, plans, quotas and spend caps.
- **Worker 1:** the claim loop and the build role. **Worker 2:** the cluster role. **Worker 3:** native builds and store publishing.
- **CLI 1:** `login`, `launch`, `deploy`, `status`, `logs`, `open`. **CLI 2:** `env`, `secret`, `domain`, `scale`, `rollback`, `db`. **CLI 3:** `publish`, `usage`, `token`.
- **Design:** the cloud's design direction. **UI 1:** the shell, sign-up and sign-in. **UI 2:** new project, the deploy timeline and logs. **UI 3:** settings, domains, billing and stores.
- **Stores:** one pull request per store.
- **End to end:** a local stack (control plane, local runtime, local build worker) that `lk cloud launch` deploys an example to, as a test.
- **Docs:** the public guide chapter and the skill reference. Until then, `docs/src` says nothing about the cloud.

## 13. Honest boundaries

What the code doesn't establish, and the docs mustn't claim until someone has done it for real:

- no cluster has been provisioned, no customer billed, and no store has received an upload from a cloud worker;
- [Apple's macOS licence](https://www.apple.com/legal/sla/docs/macOSSequoia.pdf) restricts running other people's builds on our own Macs, and needs legal review before a macOS pool is offered;
- isolation is designed around gVisor and single-tenant build sandboxes, and hasn't had a security review;
- the default domains (`cloud.leptoskit.dev`, `*.leptoskit.app`) are configuration defaults, not registered infrastructure.
