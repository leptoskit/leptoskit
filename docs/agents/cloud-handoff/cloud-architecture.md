# LeptosKit Cloud: architecture contract

Written 18 November 2026 by the planning thread. This is the contract every implementation task builds against. If a task finds the contract wrong, it stops and reports; it does not quietly diverge.

Background research (read the part your task names; do not re-research):

- Infrastructure prices and unit economics: `research-infra-costs.md` (same directory as this file).
- Serverpod Cloud, Expo EAS and other deploy flows: `research-cloud-product-flows.md`.
- Store APIs and the credential-vault design: `research-store-apis.md`.
- A map of this repository's patterns (service crates, RPC wire format, auth, config, publish clients, devenv scripts): `codebase-map.md`.

## 1. What we are building

**LeptosKit Cloud** is a hosted platform that runs any leptoskit app. A developer signs up, runs one command (or presses one button), and gets their server, database, web client, over-the-air updates and store submissions running on our infrastructure, driven by the `leptoskit.toml` they already have.

It is different from the existing pieces, and builds on them:

| Existing (keep working, do not rewrite) | What it is |
| --- | --- |
| `lk infra` + `lk deploy` (`crates/leptoskit_cli/src/infra`) | The app author provisions their own VM with OpenTofu and Kamal. |
| `lk console serve` (`leptoskit_deploy_console::control`) | A self-hosted, single-owner deployment service. |
| The hosted pilot (`leptoskit_deploy_console::hosted`, the `leptoskit-cloud` binary in `leptoskit_app`) | An operator-enrolled broker: customers bring their own compute and a dedicated worker. |
| `lk publish` (`crates/leptoskit_cli/src/publish`) | Uploads to six stores from the developer's machine. |

LeptosKit Cloud adds: public signup, organisations and teams, shared multi-tenant compute on Kubernetes, cloud builds, managed Postgres, custom domains, usage metering and billing, and store publishing from cloud build machines with credentials kept in a vault. `docs/agents/deployment-control-plane.md` step 4 ("Hosted operation") said not to turn the single-owner executor into a multi-tenant one by adding an organisation ID. This is the separate design that step asked for.

### What makes it ours rather than a clone

1. **One file.** `leptoskit.toml` already describes the server, database, services, native apps and stores. `[cloud]` adds only placement and scale. Nothing is declared twice.
2. **One release, many targets.** A release is the server deployment, the over-the-air update and the store builds together, with one version and one rollback. A fingerprint of the native inputs decides whether a change can go over the air or needs a store build.
3. **Show your workings.** Every screen can reveal the config table, the Kubernetes objects, the build steps and the cost behind it (the admin's "Workings" idea, `docs/agents/admin-design.md`). Cost is shown before a deploy, with a spend cap on by default.
4. **Honest limits.** Regions, provider, cold-start time and what happens at a cap are stated. Rollback exists from day one (Serverpod Cloud has none).
5. **Sign store releases with your wallet.** The Solana dApp Store needs the publisher wallet to sign prepared transactions. The cloud prepares them and the developer's wallet signs (browser wallet, or Seed Vault in the native cloud app); we never hold the wallet.
6. **Agent-first.** Every action is a typed RPC operation with an OpenAPI export, a generated CLI, and scoped tokens.

## 2. Crates

| Crate | Published | What it holds |
| --- | --- | --- |
| `crates/leptoskit_cloud` | yes | Domain types, validation, the source archive, pricing/metering types; tables and RPC operations (bodies behind the `server` feature); the worker protocol. Client-safe without features: the wasm UI and the CLI compile it. |
| `crates/leptoskit_cloud_runtime` | yes | `ReleaseSpec` (pure data), the `Runtime` trait, Kubernetes manifests (`kubernetes` feature), the cluster adapter over `kube` (`kube` feature), a local process adapter (`local`), a recording fake (`testing`). Depends on nothing else in the cloud. |
| `crates/leptoskit_cloud_platform` | no | Operator side: the platform configuration, OpenTofu and k3s bootstrap generation, add-on manifests, and the price/cost model. |
| `crates/leptoskit_cloud_worker` | no | One binary, `leptoskit-cloud-worker`, with two roles: `build` (builds, native builds, store publishing) and `cluster` (applies releases in one cluster). Both use the same pair/claim/finish loop. |
| `crates/leptoskit_cloud_app` | no | The cloud's own leptoskit app: sign-up, the dashboard, and the control-plane server binary `leptoskit-cloud-control` (SSR plus the API). Dogfood: it is a leptoskit app with `render = "ssr"`, hydrated, with no hand-written JavaScript. |
| `crates/leptoskit_cli/src/cloud/` | (part of the CLI) | `lk cloud …`. |
| `crates/leptoskit_config/src/cloud.rs` | (part of config) | The `[cloud]` table. |
| `crates/leptoskit_cli/src/publish/<store>.rs` | (part of the CLI) | New stores. |

Dependency direction: `leptoskit_cloud_runtime` ← `leptoskit_cloud` ← { `leptoskit_cloud_worker`, `leptoskit_cloud_app`, `leptoskit_cli` }. `leptoskit_cloud_platform` depends on `leptoskit_cloud_runtime` only. No core crate (`leptoskit`, `leptoskit_native`, …) depends on any cloud crate.

The existing binary named `leptoskit-cloud` (the pilot broker in `leptoskit_app`) keeps its name. New binaries are `leptoskit-cloud-control` and `leptoskit-cloud-worker`.

## 3. The technology choices, and why

- **API:** `leptoskit_rpc` operations over `leptoskit_db`, like `leptoskit_notifications`. That gives typed operations, live queries for deploy timelines and logs, durable jobs with retries and cron for builds, metering and billing sync, idempotency keys, and an OpenAPI export. The control plane is an app, not a `[services]` service: operations are declared without `service = "…"`, and the crate's server-only code sits behind a `server` feature.
- **Database:** Postgres in production, SQLite for tests and single-node self-hosting. Both migration directories ship, as notifications does.
- **Identity:** `leptoskit_auth` for people (email codes, passkeys, GitHub and Google OAuth, Solana wallets) and `leptoskit_idp` for the CLI's device-code login. Organisations, memberships, invitations and API tokens are cloud tables; they are not added to `leptoskit_auth`.
- **Runtime:** Kubernetes. One namespace per environment. Tenant pods run under a `gvisor` RuntimeClass with a default-deny NetworkPolicy, a ResourceQuota, no service-account token and the restricted Pod Security profile. Each `[services.<name>]` is its own Deployment and Service (this is how microservices arrive). Scaling is KEDA (HTTP add-on for sleep-when-idle); apps that hold live connections keep a minimum of one replica.
- **Node network:** k3s does not support embedded etcd across its multicloud WireGuard overlay, so control-plane servers share a private network (one provider, one private network; or a single server), and workers join from anywhere over k3s's `wireguard-native` overlay on their public addresses. Launch is one bare-metal server. Growth is three small cloud VMs as the control plane on a provider private network, with bare-metal workers joining over the overlay. Each node admits only the peers and ports that topology needs, plus the administrator CIDRs on SSH and the API; application traffic arrives only through the edge. Cloud nodes get this from a provider firewall; `existing` nodes get the same rules as a generated host firewall (nftables). Because cloud nodes' addresses exist only after `tofu apply`, generation is two-phase: the OpenTofu configuration first, then node bootstrap and firewall peer lists rendered from the configuration plus OpenTofu's JSON outputs.
- **Cluster access:** the control plane never holds a cluster's credentials. A `cluster` worker runs inside each cluster with a scoped ServiceAccount, connects outward, and claims release jobs. The same mechanism attaches a customer's own cluster later.
- **Builds:** untrusted customer code never builds in the control plane or on a shared daemon. A `build` worker runs each job in a fresh sandbox (an ephemeral VM, or a gVisor pod with rootless BuildKit) and is destroyed afterwards. macOS workers are a separate pool.
- **Postgres for tenants:** CloudNativePG. The default is a database and login role per environment on a shared cluster; paid sizes get their own instance. WAL goes to object storage.
- **Edge:** Cloudflare in front. Default hostnames under a wildcard; custom domains through Cloudflare for SaaS (or cert-manager where an operator has no Cloudflare). Object storage is S3-compatible (R2 or B2) through the existing `rusty-s3` seam.
- **Money:** Stripe. A flat plan price plus metered usage through Billing Meters. Prices are operator configuration, never constants in code. A spend cap is on by default; reaching it stops new work and sleeps what can sleep, and never deletes data.
- **Secrets:** envelope encryption with AES-256-GCM-SIV (already used in `leptoskit_auth` and `leptoskit_deploy_console`). A data key per organisation, wrapped by a master key from the environment (a KMS adapter can replace it later). Organisation, project and secret name are authenticated data. Values are released only as a per-job lease to a single-tenant worker.

Substrate at launch, from the cost research: self-managed k3s on one Hetzner AX41 (about €59 a month, bare metal so `/dev/kvm` exists), gVisor, CloudNativePG, Cloudflare Free plus Cloudflare for SaaS, R2 or B2, and owned Mac minis for Apple builds. Loaded cost is about $0.15–0.30 a month for a sleeping hobby app and $3–4 for an always-on small app. None of this is hard-coded: the platform crate takes it as configuration.

## 4. Vocabulary and the domain model

- **Organisation**: the billing and membership boundary. Signing up creates a personal organisation.
- **Member**: a person in an organisation, with a role: `owner`, `admin`, `developer`, `billing`, `viewer`.
- **Project**: one leptoskit app (one `leptoskit.toml`). Slug unique within its organisation.
- **Environment**: a running copy of a project: `production`, `staging`, or a preview. Owns its database, secrets, domains and scale.
- **Deployment**: one attempt to put one build of the server into one environment. Immutable once created; has a number per environment.
- **Build**: one execution on a build worker: `server`, or a native target.
- **Release**: a version of the whole app: a deployment plus optional over-the-air update and store submissions.
- **Worker**: a `build` or `cluster` process paired to the control plane.
- **Cluster**: a place environments run; has a region.

Identifiers are UUID v7. Slugs match `[a-z][a-z0-9-]{1,38}[a-z0-9]`, with reserved words refused (`api`, `www`, `admin`, `cloud`, `console`, `app`, `status`, `docs`).

### Sizes

`Size` is `xs`, `s`, `m`, `l`, `xl`. It maps to CPU and memory in one table (operator-overridable later; the defaults are code):

| Size | CPU request / limit (millicores) | Memory (MiB) |
| --- | --- | --- |
| xs | 100 / 500 | 128 |
| s | 250 / 1000 | 256 |
| m | 500 / 2000 | 512 |
| l | 1000 / 2000 | 1024 |
| xl | 2000 / 4000 | 2048 |

`Scale { min, max }`: `min <= max`, `max <= 20`. `min = 0` means sleep when idle.

`DatabaseSize` is `shared` (a database on the shared cluster), `s`, `m`, `l` (a dedicated instance with 1, 2, 4 GiB).

### Deployment states

```text
queued → building → migrating → rolling_out → live → superseded
                 ↘        ↘            ↘
                  failed { stage, blocker }      cancelled (from queued or building)
```

- A rollback is a new deployment whose source names an earlier deployment; it skips `building`.
- `failed` carries the stage it failed in and a typed `Blocker`: `build_failed`, `builder_unavailable`, `schema_incompatible`, `migration_failed`, `not_ready`, `quota_exceeded`, `spend_cap_reached`, `payment_required`, `cluster_unavailable`, `interrupted`.
- A worker that dies mid-job leaves the deployment `failed { blocker: interrupted }` only after an operator or the reconciler has confirmed the cluster's actual state. An expired lease is not proof the old process stopped (see `docs/agents/deployment-control-plane.md`, "Durability").
- Transitions are a function on the type (`DeploymentState::can_become`), tested exhaustively. Illegal transitions are unrepresentable or refused in one place.

### Deployment sources

`upload { archive }` (a source archive digest), `git { repository, commit }`, `template { name }` (the cloud scaffolds a starter with `lk new`, for a first deploy with no local setup), `rollback { deployment }`.

### The source archive

Both the CLI (packing) and the build worker (unpacking) use `leptoskit_cloud::archive`:

- The file list is `git ls-files --cached --others --exclude-standard` when the project is a Git checkout, else a walk that honours `.gitignore`. A `.leptoskitignore` file excludes more. `target/`, `.leptoskit/`, `.git/`, `dist/` and `node_modules/` are always excluded.
- Deterministic: sorted paths, zeroed mtimes and owners, normalised modes (0644 / 0755), gzip-compressed tar. The digest is SHA-256 of the compressed bytes, written `sha256:<hex>`.
- Bounds: at most 20,000 files and 512 MiB uncompressed; a file over 64 MiB fails with its path. Symlinks that leave the root, absolute paths and `..` components are refused when packing and when unpacking.
- Secrets never ride in the archive: `.env`, `.env.*`, `*.pem`, `*.p8`, `*.p12`, `*.jks`, `*.keystore` and `credentials*.toml` are refused with a message naming the file and `lk cloud secret set`, unless `.leptoskitignore` already excludes them.

## 5. `[cloud]` in `leptoskit.toml`

```toml
[cloud]
project = "notes" # written by `lk cloud launch`
organization = "acme" # optional; default is the signed-in person's own organisation
region = "eu-central" # optional; default is the cloud's default region
url = "https://cloud.leptoskit.dev" # optional; for a self-hosted cloud

[cloud.database]
size = "shared" # shared | s | m | l
backups = "daily" # daily | continuous

[cloud.environments.production]
size = "s" # xs | s | m | l | xl
min = 1
max = 3
sleep = "never" # never | when-idle; when-idle requires min = 0
domains = ["notes.example.com"]

[cloud.environments.production.services.search]
size = "m"
min = 1
max = 2
```

- Everything is optional. With only `[cloud] project = "…"`, production runs at size `xs`, `min = 0`, `max = 1`, `sleep = "when-idle"`, on a shared database when the app has `[db]`.
- `[db]` decides whether there is a database and its dialect. A SQLite app is given a persistent volume and exactly one replica; `max > 1` with SQLite is a validation error naming Postgres.
- `[services.<name>]` decides the processes. A `[cloud.environments.<env>.services.<name>]` table for a service that does not exist is an error. Remote services (`url = …`) take no cloud settings.
- `sleep = "when-idle"` is refused when `[rpc]` enables live queries for the app, unless the table says `allow_sleep_with_live = true`; the error explains that sleeping closes live connections.
- `[cloud]` and a managed `[infra]` can coexist: `lk deploy` uses the cloud when `[cloud].project` is set, and `lk infra …` keeps addressing the app's own infrastructure. `[cloud]` together with `infra.control_url` is an error (two places claiming `lk deploy`).
- Spend limits, secrets and billing never live in the repository.

## 6. Operations (the public API)

Declared in `leptoskit_cloud` with `#[rpc::query]`, `#[rpc::mutation]`, `#[rpc::action]`. Every operation authorises on the server from the caller's identity and membership; nothing trusts an organisation or project named in arguments without checking membership. Names are stable; grouping by module:

| Module | Operations |
| --- | --- |
| `account` | `whoami`, `organizations`, `create_organization`, `members`, `invite_member`, `accept_invitation`, `set_member_role`, `remove_member` |
| `tokens` | `tokens`, `create_token` (value shown once), `revoke_token` |
| `projects` | `projects`, `project`, `create_project`, `delete_project` (type-to-confirm argument), `environments`, `create_environment`, `update_environment`, `delete_environment` |
| `deployments` | `begin_upload` (returns a presigned PUT for an archive digest), `deploy`, `deployment`, `deployments` (live), `deployment_events` (live), `cancel_deployment`, `rollback`, `runtime_logs`, `build_logs` |
| `config` | `variables`, `set_variable`, `unset_variable`, `secrets` (names only), `set_secret`, `unset_secret` |
| `domains` | `domains`, `attach_domain`, `verify_domain`, `detach_domain` |
| `databases` | `database`, `database_backups`, `create_backup`, `restore_backup` (confirm argument) |
| `billing` | `plans`, `subscription`, `checkout`, `portal`, `usage`, `estimate` (cost of a `[cloud]` table before deploying), `set_spend_cap` |
| `stores` | `store_connections`, `connect_store`, `disconnect_store`, `publish`, `store_releases` (live), `store_release`, `signing_request` / `submit_signature` (wallet-delegated signing) |
| `workers` (worker keys only) | `pair`, `claim`, `renew`, `append_log`, `finish` |
| `operator` (operator role only) | `clusters`, `register_cluster`, `workers`, `issue_pairing`, `revoke_worker`, `reconcile` |

Rules that hold across the API:

- Mutations that start work take an idempotency key and return the same result for a repeat.
- Destructive operations take an explicit confirmation argument and are refused for scoped tokens unless the scope names them.
- Errors are `rpc::Error` variants with a stable machine code in the message prefix for the typed blockers above.
- Secrets are write-only through the API. No operation returns a secret value.
- Token scopes: `read`, `deploy`, `manage` (config, domains), `publish`, `billing`, `admin`; bound to an organisation and optionally one project; hashed at rest; shown once; expiring by default.

## 7. The worker protocol

- **Pairing.** An operator (or an organisation, for its own workers later) issues a one-use pairing code that expires in ten minutes. `pair { code, role, capabilities }` returns a worker key, stored hashed. A worker belongs to a pool: the shared pool, or one organisation.
- **Claim.** `claim` long-polls and returns at most one job the worker's role, pool, region and capabilities match, with a lease. `renew` extends the lease; `append_log` streams bounded log chunks; `finish { job, outcome }` is idempotent.
- **Crash safety**, as the hosted worker already does it: the worker writes a pending marker before executing and the result before acknowledging. A marker without a result means the job's effect is unknown; the worker refuses to repeat it and the job is reconciled, never replayed blindly. Build jobs are the exception: they have no side effect outside artifacts keyed by digest, so an interrupted build is retried.
- **Jobs.** `build_server { archive, toolchain, targets }` → an image digest, web assets, migration fingerprints. `build_native { platform, … }` → an artifact. `publish { store, submit, leases }` → a "What to do next" report (`publish::steps::Report`). `release { spec: ReleaseSpec }`, `scale`, `destroy_environment`, `fetch_logs`, `sample_usage` for cluster workers.
- **Leases on secrets.** A job that needs a credential gets a lease naming the secret versions, recorded before release, delivered in the claim response over TLS, valid for that job only. Where a store's API allows it the control plane mints a short-lived token and the worker never sees the long-lived key.
- **Tenancy.** A build worker runs one job at a time in a fresh sandbox and keeps no cache that another organisation's job can read.

## 8. `ReleaseSpec` and the runtime

`leptoskit_cloud_runtime::ReleaseSpec` is the whole desired state of one environment, as data, with no reference to plans or prices:

- identity: organisation, project and environment slugs and IDs; the deployment number;
- processes: the app, optional split roles, and one per service: image digest (never a tag), command, port, health paths, environment variables, secret references, resources (CPU and memory numbers), scale (min, max, sleep), volumes;
- routes: hostnames and path prefixes to processes (including `/_leptoskit/services/<name>/` for public services, and cookie stripping for isolated ones);
- database bindings: which secret holds each process's `DATABASE_URL`;
- policy: runtime class, egress rules, quota.

`Runtime` is the adapter boundary:

```rust
pub trait Runtime {
	async fn apply(&self, spec: &ReleaseSpec) -> Result<Applied, RuntimeError>;
	async fn status(&self, environment: &EnvironmentRef) -> Result<Observed, RuntimeError>;
	async fn logs(&self, environment: &EnvironmentRef, query: &LogQuery) -> Result<Vec<LogLine>, RuntimeError>;
	async fn destroy(&self, environment: &EnvironmentRef) -> Result<(), RuntimeError>;
}
```

Adapters: Kubernetes (server-side apply, then watch the rollout to ready or failed), local processes (for development and the end-to-end test), and a recording fake. Rollouts replace pods with `maxUnavailable: 0`; a release that never becomes ready leaves the previous one serving.

## 9. The first-deploy flow

```text
lk cloud login       # device code: opens the browser, or prints a URL and code
lk cloud launch      # in a project directory
```

`launch`:

1. reads `leptoskit.toml` and prints what it found (server, database dialect, services, platforms, stores);
2. asks the API for an `estimate` and prints the monthly cost and the spend cap;
3. creates the project and its production environment (idempotent), and writes `[cloud] project = "…"`;
4. packs the source archive, uploads it by digest (skipping the upload when the cloud already has it);
5. creates the deployment and follows its events: `Uploaded`, `Built`, `Migrated`, `Live`;
6. prints the URL, the dashboard link, and a "What to do next" list (custom domain, secrets it noticed in `.env`, stores).

On the web the same thing is: sign up, press **New project**, then either **Deploy a starter** (one press; the cloud scaffolds and deploys a template), **Import from GitHub**, or copy the two commands. Later deploys are `lk deploy`. `lk cloud` also has `status`, `logs`, `open`, `env`, `secret`, `domain`, `scale`, `rollback`, `db`, `publish`, `usage`, `token`, `project`, `org`, `whoami`, `logout`. Every command takes `--json`.

Credentials: the CLI keeps its tokens in a private file under `$XDG_CONFIG_HOME/leptoskit/cloud/` (mode 0600, the pattern in `console/client.rs`), or reads `LEPTOSKIT_CLOUD_TOKEN` in CI.

## 10. Store publishing from the cloud

`lk cloud publish [stores] [--submit]` asks the cloud to build each native target on a worker and run the same store clients `lk publish` uses, with credentials from the vault. The result is the same "What to do next" report, shown in the dashboard as a checklist with `agent` and `person` steps.

New stores in `lk publish` (so they work locally too), in this order: Solana dApp Store; Huawei AppGallery for Android and HarmonyOS; Microsoft Store; Samsung Galaxy Store; Meta Horizon Store; itch.io and Steam for the game host; then Amazon Appstore and the Snap Store. Each follows the existing shape exactly: a `[publish.<store>]` table, a client over the `Http` seam or the `Runner` seam, credentials in `NEEDED`, dry-run checks, steps, and fake-server tests.

## 11. Pricing model

Prices are data in the platform crate's catalogue, with the cost basis, its source and its date beside each price, and a target gross margin. The control plane reads plan and meter definitions from operator configuration and Stripe price IDs from the environment.

Proposed defaults (the owner decides the real numbers):

| Plan | Price | Includes |
| --- | --- | --- |
| Hobby | free | one project, size `xs`, sleeps when idle, shared database up to 500 MB, 100 Linux build minutes, default hostname only, hard caps |
| Pro | $20 a month plus usage, with $20 of usage included | teams, custom domains, autoscaling, always-on, continuous backups, store publishing |
| Scale | by agreement | dedicated nodes, a customer's own cluster |

Meters: compute seconds per size, database instance hours and storage GB-months, object storage GB-months, egress GB over an allowance, Linux and macOS build minutes, custom hostnames. Suggested unit prices, from the research: Linux build minute $0.005, macOS build minute $0.04, an always-on `m` app with a 1 GB database about $12–15 a month all in.

## 12. Honest boundaries

What the code does not establish, and the docs must not claim until someone has done it for real:

- no cluster has been provisioned, no customer has been billed, and no store has received an upload from a cloud worker;
- Apple's macOS licence restricts running other people's builds on our own Macs; that needs legal review before the macOS pool is offered;
- isolation is designed around gVisor and single-tenant build sandboxes, and has not had a security review;
- the default domains (`cloud.leptoskit.dev`, `*.leptoskit.app`) are configuration defaults, not registered infrastructure.

## 13. Slices

Each slice is one pull request. Later slices state which earlier ones they need.

- **Foundation:** `leptoskit_cloud` base types, the source archive, `[cloud]` config and schema, this note in `docs/agents/cloud.md`.
- **Runtime 1:** `ReleaseSpec`, `Runtime`, Kubernetes manifests, the fake and local adapters. **Runtime 2:** the `kube` cluster adapter.
- **Platform:** platform config, OpenTofu and k3s bootstrap, add-ons, the price catalogue and cost model.
- **Control 1:** tables and migrations, auth and device login, organisations, tokens, projects, environments, the server binary. **Control 2:** uploads, deployments, the worker protocol, events and logs. **Control 3:** variables, the secrets vault, domains, databases. **Control 4:** metering, Stripe, plans, quotas and spend caps.
- **Worker 1:** the claim loop and the build role. **Worker 2:** the cluster role. **Worker 3:** native builds and store publishing.
- **CLI 1:** `login`, `launch`, `deploy`, `status`, `logs`, `open`. **CLI 2:** `env`, `secret`, `domain`, `scale`, `rollback`, `db`. **CLI 3:** `publish`, `usage`, `token`.
- **Design:** the cloud's design direction. **UI 1:** shell, sign-up and sign-in. **UI 2:** new project, deploy timeline, logs. **UI 3:** settings, domains, billing, stores.
- **Stores:** one pull request per store.
- **End to end:** a local stack (control plane, local runtime, local build worker) that `lk cloud launch` deploys an example to, as a test.
- **Docs:** the public guide chapter and the skill reference.

## 14. Amendments after the design direction

The design (`docs/agents/cloud-design.md`) makes the **release sheet** the product's central object: one sheet per release with a line per target, signed by a person or token, stamped by each target as it lands. Its §20 lists what the screens need from the API. These are now part of the contract; the control-plane slices implement them alongside §6:

- **Signer on every deployment:** who (person or token) and via what (web, CLI with the host name it reported, a token's client name). Recorded at creation, immutable.
- **Per-line progress:** `deployment_events` carry a line (server, each service, migrations, web assets, each over-the-air platform, each store), a stamp (`built`, `migrated`, `live`, `in_review`, `failed`, `unknown`), build steps with cache hits, the native-fingerprint decision, and a `reconciling` event. An outcome that cannot be confirmed is `unknown`, never guessed.
- **`deployments::preview_release`:** a release's lines, costs and fine print without shipping, for drafts, rollbacks and "ship again".
- **`deployments::in_flight`** (live, per organisation) and **`projects::environment_status`** (live: per process ready/desired, asleep, waking, restarts, out-of-memory kills, last heard).
- **`build_logs`** is a live query.
- **`create_project`** returns what was detected (targets, `.env` variable names, never values).
- **`billing::plans`** returns unit prices per size and meter; **`usage`** returns a forecast and `updated_at`; **`billing::invoices`**; what a reached cap put to sleep.
- **Scale changes are events**, and an environment records whether its scale differs from the last `leptoskit.toml` it was deployed from.
- **Config release:** changing variables or secrets creates a deployment that skips building (as a rollback does), so the change takes effect deliberately and has its own sheet.
- **Domains** return the DNS records required, what DNS answered at the last check, and certificate state. **Databases** return applied migrations with their release, backup kinds, and restore impact.
- **Invitations:** list pending, revoke.
- **Stores:** `check_store` (the store client's dry-run checks on a worker), a done state per report step and marking person steps done, signing requests prepared on demand with the publisher's balance, an expiry and decoded transactions, and a `todo` query across organisations.
- **Reserved slugs** include the console's route words (see the design's §3).
- Later, not in the first control-plane slices: GitHub (app installation, repositories, `git` sources) and request metrics.

Decisions taken on the design's open questions (the owner can overrule any of them):

1. The console may change scale; the drift from `leptoskit.toml` is shown on the process and on the next sheet.
2. Pro's spend cap starts at the usage the plan includes.
3. Variable and secret changes are config releases.
4. The native cloud apps ship before fonts reach native.
5. Prices on the public page are labelled as proposed until the owner confirms them.
