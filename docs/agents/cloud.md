# LeptosKit Cloud

Engineering note, 18 November 2026. LeptosKit Cloud is a hosted platform that runs any leptoskit app. A developer signs up and runs one command, or presses one button, and the cloud runs their server, database, web client, over-the-air updates and store submissions from the `leptoskit.toml` they already have.

This note is the contract every cloud pull request builds against. When a slice finds it wrong, the slice stops and the note changes first. The foundation and Control 1 are built; §12 says what exists and what each part's evidence is.

## CLI accounts and projects

Tables are server-only; every client-visible type is a plain serde type, and an operation never returns a table row.

CLI 1's account slice is built and verified: Cloud all-feature tests passed 97/97 with Postgres required, including the isolated featureless-client check with no backend, SQLite, Postgres, both backends, and wasm. CLI cloud tests passed 13/13, help snapshots passed 14/14, and the control-plane app suite passed 11/11, including real-binary device login, whoami, project creation, token creation and API-token use. Cloud native and featureless wasm clippy, CLI all-target checking, and CLI and control-plane app native clippy passed with warnings denied.

The CLI reads cloud discovery for the client, scopes and RPC path, then OpenID Connect discovery for device authorization, token and revocation endpoints. The fake provider advertises different paths to verify that the CLI follows discovery. The binary installs the auth and identity-provider layers after merging readiness routes: Axum's merge replaces the default fallback that carries protocol middleware, so nothing may be merged after those final layers. `leptoskit_cloud_app/tests/binary.rs` starts the real binary and checks OIDC discovery, device authorization, Cloud discovery, readiness and health. `leptoskit_cloud_app/tests/cloud_cli.rs` verifies the end-to-end CLI journey.

The account part of CLI 1 lives in `crates/leptoskit_cli/src/cloud/`. It adds this command tree; deployment commands belong to a later slice, and `lk deploy` keeps its existing behavior:

```text
lk cloud login [--no-browser]
lk cloud logout
lk cloud whoami
lk cloud org list
lk cloud org create <name> [--slug <slug>]
lk cloud org members <org>
lk cloud org invite <org> <email> --role <role>
lk cloud org invitations <org>
lk cloud org revoke-invitation <org> <invitation-id> --confirm <invitation-id>
lk cloud org set-role <org> <account-id> --role <role>
lk cloud org remove-member <org> <account-id> --confirm <account-id>
lk cloud project list
lk cloud project create <name> [--region <region>]
lk cloud project show
lk cloud project delete <project> --confirm <project>
lk cloud env list
lk cloud env create <name> [--kind staging|preview]
lk cloud env update <name> [--size xs|s|m|l|xl] [--min <n>] [--max <n>] [--sleep never|when-idle]
lk cloud env delete <name> --confirm <name>
lk cloud token list
lk cloud token create <name> --scope <scope> [--scope <scope>] [--expires 30|90|365]
lk cloud token revoke <token-id> --confirm <token-id>
lk cloud link [--force]
```

Every command accepts `--json`, `--url`, `--org` and `--project`, before or after its subcommand. Organization and project default from `[cloud]`; without an organization link, the caller's personal organization is used. Organization operations name their organization positionally. Token creation's optional `--project` binds the token to that project; without an explicit or linked project it creates an organization token. Destructive operations always require `--confirm`, including in a terminal. There are no interactive confirmation prompts.

The cloud origin is chosen from `--url`, `LEPTOSKIT_CLOUD_URL`, `[cloud].url`, then `https://cloud.leptoskit.dev`. It must be HTTPS, or HTTP on loopback, without credentials, path, query or fragment. Requests have a 30-second timeout, follow no redirects and bypass proxies for loopback.

Login reads `/.well-known/leptoskit-cloud` for the first-party client ID, scopes and RPC path. It prints the verification URL and user code, opens a browser only in an interactive terminal unless `--no-browser` was supplied, and polls at the server's interval, adding five seconds after `slow_down`. Device codes, access tokens and refresh tokens are never displayed. The identity provider has no client-description field in device authorization, so no host name is sent. The control plane must add this field before the release sheet can say “signed from the terminal on <host>”.

Sessions are stored per normalized origin in `$XDG_CONFIG_HOME/leptoskit/cloud/<sha256-origin>.json`, falling back to `~/.config/leptoskit/cloud/`. On Unix the directory is mode 0700 and the files mode 0600. Broader permissions and symbolic links are refused with a repair command. Writes use a private temporary file, file synchronization, atomic replacement and directory synchronization. A separate `<sha256-origin>.lock` file is locked throughout refresh and storage. The session is read again after acquiring the lock, preventing two processes from reusing a rotating refresh token. Login replacement and logout use the same lock. Access tokens are refreshed 30 seconds before expiry and once after an HTTP 401. Logout removes local credentials even when revocation cannot reach the server, and reports that the remote session may remain live.

`LEPTOSKIT_CLOUD_TOKEN` supplies a CI API token instead of the stored session for that invocation and is never stored. Listing members accepts API tokens. Membership changes, invitations and token management use `Permission::requires_person` and reject this override before making a request, explaining that they need a person signed in through `lk cloud login`.

JSON output is the bare operation result, without a wrapper or terminal styling. UUIDs are strings, timestamps are RFC 3339 strings, roles and scopes use their lower-case API spellings, and optional fields are `null`. These shapes come from the client-safe types in `leptoskit_cloud`:

| Command                                                     | JSON result                                                                                                                    |
| ----------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| `login`                                                     | `{ "signed_in": true }`                                                                                                        |
| `logout`                                                    | `{ "local_credentials_removed": true, "revoked": boolean }`                                                                    |
| `whoami`                                                    | `WhoAmI`: `account`, `email`, `via`, `operator`, `organizations`, `token`                                                      |
| `org list`                                                  | Array of `OrganizationSummary`: `id`, `slug`, `name`, `plan`, `personal`, `role`                                               |
| `org create`                                                | One `OrganizationSummary`                                                                                                      |
| `org members`                                               | Array of `Member`: `account`, `email`, `role`, `joined_at`                                                                     |
| `org invite`, `org invitations`                             | One or an array of `PendingInvitation`: `id`, `email`, `role`, `invited_by`, `expires_at`, `created_at`                        |
| `project list`                                              | Array of `ProjectSummary`: `id`, `slug`, `name`, `region`, `created_at`                                                        |
| `project create`                                            | `CreatedProject`: `project`, `environment`, `detected`                                                                         |
| `project show`                                              | `ProjectDetail`: `project`, `environments`, `detected`                                                                         |
| `env list`                                                  | Array of `EnvironmentSummary`                                                                                                  |
| `env create`, `env update`                                  | `EnvironmentSummary`: `id`, `name`, `kind`, `scale`, `revision`, `scale_differs`, `created_at`, `updated_at`                   |
| `token list`                                                | Array of `TokenSummary`: `id`, `name`, `prefix`, `scopes`, `project`, `expires_at`, `last_used_at`, `created_by`, `created_at` |
| `token create`                                              | `{ "token": TokenSummary, "value": string }`; this is the only output containing a token value                                 |
| `link`                                                      | `{ "organization": string, "project": string, "url": string }`                                                                 |
| All deletions, revocations, member removal and role changes | `null`                                                                                                                         |

`scale` contains `app` and a map of `services`; each process has `size`, `min`, `max` and `sleep`. `detected` contains arrays `targets` and `variables`. `whoami.token` contains public caller-token metadata, never its secret. Token creation warns on stderr that the value is shown once, including in JSON mode. A lost response is not retried: `token_name_taken` preserves the server's explanation naming the token to revoke. Environment updates first read their revision, preserve service scale, and report `stale_revision` as a request to retry after another writer's change.

`link` checks the project through the API before editing `leptoskit.toml` with `toml_edit`. It writes `project`, omits `organization` for the personal organization, omits the default `url`, preserves unrelated comments and formatting, and requires `--force` to replace a different link.

Evidence is in `cloud/tests.rs` (fake control plane, device outcomes, refresh rotation and lock race, CI precedence, logout, permissions, command requests, error messages, help and human snapshots) and `leptoskit_cloud_app/tests/cloud_cli.rs` (real `serve --dev`, device approval, whoami, project creation and API-token creation/use).

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

| Crate                                  | Published | What it holds                                                                                                                                                                                                                                                             |
| -------------------------------------- | --------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `crates/leptoskit_cloud`               | yes       | The shared types, the source archive, the price estimate, and the control plane's tables and operations, with their bodies, identity, sign-in and routes behind a `server` feature; the worker protocol later. Safe for clients without features.                         |
| `crates/leptoskit_cloud_runtime`       | yes       | `ReleaseSpec` as plain data, the `Runtime` trait, Kubernetes manifests, the cluster adapter, a local process adapter and a recording fake. Depends on nothing else in the cloud.                                                                                          |
| `crates/leptoskit_cloud_platform`      | no        | The operator side: platform configuration, OpenTofu and k3s bootstrap, add-on manifests, the price catalogue and the cost model.                                                                                                                                          |
| `crates/leptoskit_cloud_worker`        | no        | `leptoskit-cloud-worker`, with a `build` role (builds and store publishing) and a `cluster` role (applies releases inside one cluster). Both share one pair, claim and finish loop.                                                                                       |
| `crates/leptoskit_cloud_app`           | no        | `leptoskit-cloud-control`, the server, behind a `server` feature, with its environment and its `serve`, `migrate` and `operator` commands; later the cloud's own leptoskit app (sign-up, the dashboard), rendered with SSR and hydrated, with no hand-written JavaScript. |
| `crates/leptoskit_cli/src/cloud/`      | CLI       | `lk cloud …`.                                                                                                                                                                                                                                                             |
| `crates/leptoskit_config/src/cloud.rs` | config    | The `[cloud]` table and its validation.                                                                                                                                                                                                                                   |

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
   ↘          ↘        ↘            ↘
    failed { stage, blocker }              cancelled (from queued or building)
```

- A rollback is a new deployment whose source names an earlier one, and it goes from `queued` straight to `migrating`. `DeploymentState::can_become` allows that edge; `DeploymentState::advance` also checks the source, so an upload can't skip its build.
- `failed` records the stage it failed in and a `Blocker`: `build_failed`, `builder_unavailable`, `schema_incompatible`, `migration_failed`, `not_ready`, `quota_exceeded`, `spend_cap_reached`, `payment_required`, `cluster_unavailable` or `interrupted`. Each has a stable code for errors and the wire.
- A worker that dies mid-job leaves the deployment `failed` with `interrupted` only after an operator or the reconciler has confirmed what the cluster is actually running. An expired lease doesn't prove the old process stopped; see "Durability" in [the control plane note](deployment-control-plane.md).
- A queued deployment fails, in the `queued` stage, only for a reason that stops it before any work starts: `builder_unavailable`, `quota_exceeded`, `spend_cap_reached` or `payment_required` (`Blocker::fails_queued`). Every other blocker needs the stage it names, so a deployment never fails `queued` with, say, `migration_failed`.

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
| `account`                    | `whoami`, `organizations`, `create_organization`, `members`, `invite_member`, `accept_invitation`, `invitations`, `revoke_invitation`, `set_member_role`, `remove_member`                       |
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

- Mutations that start work take an idempotency key and return the same result for a repeat. An operation whose result is a secret is an action instead, because a receipt keeps the result: see `create_token` below.
- Destructive operations take an explicit confirmation argument, and scoped tokens can't call them unless the scope names them.
- Errors are `rpc::Error` variants whose message starts with the blocker's stable code.
- Secrets are write-only. No operation returns a secret's value.
- Token scopes are `read`, `deploy`, `manage` (config and domains), `publish`, `billing` and `admin`. A token belongs to an organisation and optionally one project, is hashed at rest, is shown once, and expires by default.

### Accounts, tokens and projects as built (Control 1)

The `account`, `tokens` and `projects` operations are declared at the top of `leptoskit_cloud` (`account.rs`, `tokens.rs`, `projects.rs`), so the CLI and the dashboard call them without the server; their bodies, identity and authorisation are under `server/`. The rules below are what the code and its tests hold.

**Identity.** `server::CloudIdentify` checks a request's credentials in this order, and the first that holds decides who is calling:

1. a `leptoskit_auth` session: the browser's cookie, or a session token as a bearer;
2. the CLI's device-flow access token, which `leptoskit_idp`'s layer has already turned into a delegation, accepted only for the first-party CLI client (`server::CLI_CLIENT_NAME`, registered once at start, found again by name on every later start);
3. an API token, `lkc_<prefix>_<secret>`, as a bearer.

Anything else, including an unknown, malformed, expired or revoked credential, is anonymous, and every operation answers an anonymous caller `401 sign_in_required`. A malformed bearer that looks like a JWT is refused by `leptoskit_idp` first with `401 invalid_token`. The audit log records how a person called (web or CLI) or which token, never its value. A live socket keeps a token's identity for at most a minute before checking it again, so revocation reaches a socket that late.

**Permissions.** `access.rs` holds the one table; `server/authorize.rs` is the only code that applies it, reading the organisation, the caller's membership and the role from the database on every call. A "person" is someone signed in with a session or through the CLI's device-flow delegation; an API token is never one.

| Permission            | Level        | owner | admin | developer | billing | viewer | Token scope         |
| --------------------- | ------------ | :---: | :---: | :-------: | :-----: | :----: | ------------------- |
| `view_organization`   | organisation |   ✓   |   ✓   |     ✓     |    ✓    |   ✓    | any                 |
| `view_projects`       | project      |   ✓   |   ✓   |     ✓     |    ✓    |   ✓    | any                 |
| `manage_members`      | organisation |   ✓   |   ✓   |           |         |        | none: a person only |
| `manage_owners`       | organisation |   ✓   |       |           |         |        | none: a person only |
| `manage_tokens`       | organisation |   ✓   |   ✓   |           |         |        | none: a person only |
| `create_project`      | organisation |   ✓   |   ✓   |     ✓     |         |        | `manage`            |
| `manage_environments` | project      |   ✓   |   ✓   |     ✓     |         |        | `manage`            |
| `delete_project`      | project      |   ✓   |   ✓   |           |         |        | `admin`             |
| `delete_environment`  | project      |   ✓   |   ✓   |           |         |        | `admin`             |

`Permission::requires_person` marks the three person-only permissions, and `Access::require` refuses a token for them with `403 person_required` before it looks at the role or the scopes. Who is in an organisation, with what role, and which tokens act in it are the authority to act there: a token that could invite someone, change a role or make a token could turn itself into a membership or a credential that outlives its own scopes, expiry and revocation. So a token can't make or revoke tokens either, whatever its scopes; the `admin` scope means deleting projects and environments, and nothing else.

| Operations                                                                                                 | Needs                                                                                                                                                                            |
| ---------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `whoami`, `organizations`                                                                                  | signed in (a person or a token)                                                                                                                                                  |
| `create_organization`, `accept_invitation`                                                                 | a person, never a token                                                                                                                                                          |
| `members`, `projects`                                                                                      | `view_organization`                                                                                                                                                              |
| `invite_member`, `invitations`, `revoke_invitation`, `set_member_role`, `remove_member`                    | `manage_members`, and `manage_owners` for any change that gives or takes the owner role; a person only                                                                           |
| `tokens`, `create_token` (an action), `revoke_token`                                                       | `manage_tokens`; a person only                                                                                                                                                   |
| `project`, `environments`                                                                                  | `view_projects`                                                                                                                                                                  |
| `create_project`                                                                                           | `create_project`                                                                                                                                                                 |
| `create_environment`, `update_environment`                                                                 | `manage_environments`                                                                                                                                                            |
| `delete_project`                                                                                           | `delete_project`                                                                                                                                                                 |
| `delete_environment`                                                                                       | `delete_environment`                                                                                                                                                             |
| `rpc.inbox`, `rpc.mark_read`, `rpc.mark_all_read`, `rpc.job_status`, `rpc.cancel_job`, `rpc.service_token` | `leptoskit_rpc`'s own, served by every leptoskit server; they reach no organisation, and find nothing because the cloud writes no notices or jobs and forwards nobody's identity |

- A token has a permission when one of its scopes grants it _and_ its maker's current role does, so a demotion or removal takes effect on their tokens at once; removing a member also revokes their tokens. A token bound to a project has no organisation-level permission except `view_organization`, and every other project is "not found" to it.
- `access::OPERATIONS` lists every served operation with its requirement. `tests/authorization.rs` checks that list against everything the server's router serves (`leptoskit_rpc::server::registrations_in(None)`, nothing filtered, the `rpc.*` operations included) and that the OpenAPI document publishes nothing else. It then runs every operation as a person with each role, as that person through the CLI, as a token for each role × each scope × bound to a project or not (60 tokens; a token whose maker isn't an owner or admin is made by an owner who is then demoted, which is how one comes to exist), and as nobody, and checks each answer's status and code against the table, so an operation without a decided permission fails the build.
- Seeing comes before being allowed: an organisation or project the caller can't see is `404 organization_not_found` / `project_not_found`, worded the same whether or not it exists. Then a token asking for a person-only permission is `403 person_required`, and a missing permission is `403 insufficient_role`, `insufficient_scope` or `project_token`, in that order. Organisations, projects and environments are named by slug; the IDs that name a member, an invitation or a token are looked up only inside the organisation already authorised, so another tenant's ID is the same `404` as a made-up one.
- Deleting a project takes `confirm` equal to its slug, and an environment its name (`409 confirmation_mismatch` otherwise), plus the `admin` scope by name for a token. Production can't be deleted (`403 production`).

**Organisations and members.** The first call from a person makes their personal organisation. Its slug is tried in this order: their primary verified address's local part made a slug (`ana`), or `personal` without a usable one; then `ana-2` and `ana-3`; then `ana-` and ten random hex characters (40 bits), up to eight times. A reserved or taken slug moves on to the next. The friendly slugs stay derivable from an address, because the console shows the slug and a recognisable one is friendlier, so anyone who knows an address can take them first; that only moves its person to a random slug nobody can take in advance, so nobody can stop another person's first sign-in. Running out of random slugs means the server's randomness is broken, and is a `500`. A unique `personal_owner` column makes the organisation once however many first requests race: a request that loses looks again and finds the winner's. Slugs refuse the console's route words: `sign-in`, `device`, `welcome`, `invitations`, `todo`, `account`, `organizations` and `operator` for organisations; `new` and `settings` for projects; `releases`, `stores`, `publish`, `sign`, `settings`, `ship` and `logs` for environments. Only owners make or unmake owners, and the last owner can't be demoted or removed (`409 last_owner`). A person may remove themselves (leave) without `manage_members`, subject to that; a token removing its own maker is managing members, and refused. Concurrent changes keep an owner: the changes run serializable, so when two owners demote or remove each other at once, the one that runs second finds its caller no longer an owner.

**Invitations.** An invitation is a single-use token, 32 random bytes in hex, stored as its SHA-256 and mailed once in plain text through the auth `Mailer` after the transaction commits. It lasts seven days, and an organisation has at most 100 pending. The rules:

- Only a person invites, and only to a role they may give (`Role::may_give`): `manage_members`, and `manage_owners` for the owner role.
- Inviting an address again revokes its pending invitations, and the inviter must be allowed to give each of their roles too, so an admin can't cancel an owner's invitation by re-inviting the address as a viewer. Each revoked invitation gets its own `invitation.revoke` audit entry naming the one that replaced it.
- An invitation is only as good as its inviter's authority when it is accepted: accepting checks that the inviter is still a member whose role may give the invited role, and otherwise answers like an unknown invitation.
- Demoting or removing a member revokes, with an audit entry each, the pending invitations they sent that their new role can't give, so the list stays truthful and re-promoting them doesn't revive old invitations.
- Accepting needs a person whose verified address is the invited one. Unknown, used, revoked, expired, wrongly addressed and outranked invitations all answer `404 invitation_invalid`, so the token proves nothing. Accepting again, or as someone already a member, changes nothing and keeps their role.

**Tokens.** A token is `lkc_`, 12 lower-case hex characters that find it, `_`, and 64 that prove it. Only the SHA-256 of the secret is stored, compared in constant time, and the value is returned once, by `create_token`. Lifetimes are 30, 90 (the default) or 365 days. `last_used_at` is written at most every five minutes. Revoking is immediate. Only a person makes, lists or revokes tokens, and can't make one stronger than themselves: scopes their role can't grant are `403 token_too_strong`. An organisation has at most 200 live tokens.

`create_token` is an `#[rpc::action]`, not a mutation, because a mutation's receipt stores its whole result for a repeated `Idempotency-Key`, and this result is the secret. The action runs the work as an unregistered mutation operation through `ActionCx::mutate`, so the token row and its audit entry still commit in one serializable transaction with the usual retries, and nothing is kept for a repeat. Instead, a live token's name is unique in its organisation, checked in that transaction. A retried `create_token` (the same name, sent again because the answer was lost) is refused with `409 token_name_taken`, which names the first token's prefix, creation time and ID and says that nobody saw its value: revoke it and ask again. A retry never makes a second token its caller can't tell from the first, and the header is ignored. The other operations return no secret, so their receipts hold nothing secret: the invitation's link is only mailed, after the commit, and a receipt keeps a hash of the arguments, never the arguments.

**Projects and environments.** `create_project` takes the slug, the region (default from the operator's configuration) and the targets and variable names the CLI detected (never values), records them, and makes the `production` environment asleep. An organisation has at most 200 projects and a project 50 environments. `update_environment` takes the revision the caller read; a stale one is `409 stale_revision`, and a change of scale marks the environment `scale_differs` until a later slice applies it.

**The audit log** gets a row in the same transaction as every change: the actor (account, and token ID when a token acted), how they called, the organisation and project, the action, its target, and the time. Tokens are named by ID and prefix, invitations by ID and address; no row carries a secret. Nothing updates or deletes a row. The database can't enforce that: `leptoskit_db` lets only schema packs declare triggers, so an app's own table has no rule that refuses updates and deletes in both SQLite and Postgres, and the migrations are generated, never hand-written. The convention is held by a test instead (`server::audit::tests::only_this_module_writes_the_audit_log_and_only_inserts`): only `server/audit.rs` names the table's rows, it only inserts, and neither dialect's migrations give the table a foreign key that could delete entries with what they name.

**Operators** are granted and revoked only on the server's command line (`leptoskit-cloud-control operator …`), recorded in the audit log, and reported by `whoami`. There are no operator operations until the slice that adds clusters and workers.

**Discovery.** `GET /.well-known/leptoskit-cloud` returns the CLI client's ID, the scopes it asks for (`openid email cloud`) and the operations' path, so `lk cloud login` needs only the cloud's origin. Device approval is `POST /oauth/device` from a signed-in browser; the console's `/device` page arrives with the UI slices.

**The server.** `leptoskit-cloud-control` (`crates/leptoskit_cloud_app`) reads everything from `LEPTOSKIT_CLOUD_*` variables, listed in [its readme](../../crates/leptoskit_cloud_app/readme.md). Keys, SMTP and the origin have no defaults and it refuses to start without them; `--dev` (debug builds only, loopback only) adds dev sign-in, an in-memory mailbox, a key for the run and migration on start. `serve` refuses a database with pending or unknown migrations; `migrate` applies them.

### Security review of Control 1

An independent review of the slice before it shipped found the following. Each fix has a test that failed on the code as reviewed; the scenarios run on SQLite (`tests/control.rs`) and Postgres (`tests/postgres.rs`).

| Finding                                                                                                                                                                                                                                                 | Fix                                                                                                                                                                                                                                                                                                           | Test                                                                                                                                                                                                   |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| 1. An `admin`-scoped API token could invite members (an owner, from an owner's token), revoke invitations, change roles and remove members, so a leaked token became a membership that outlives it; and an invitation outlived its inviter's authority. | Managing members, owners and tokens requires a person (`Permission::requires_person`, applied in `Access::require` as `403 person_required`). Accepting an invitation checks the inviter still may give its role; demoting or removing a member revokes the invitations they can no longer give.              | `tokens_never_grant_membership`, `an_invitation_needs_its_inviters_authority`, `tokens_are_never_stronger_than_their_maker`, `authorization::every_token_is_allowed_what_its_maker_and_scope_both_are` |
| 2. `create_token` was a mutation, so with an `Idempotency-Key` its receipt stored the new token's whole value in `rpc.receipts`.                                                                                                                        | `create_token` is an action running an unregistered mutation operation through `ActionCx::mutate`, so it keeps no receipt and the row and audit entry still commit together; live token names are unique, so a retry is `409 token_name_taken` naming the first token, never a second, indistinguishable one. | `token_secrets_are_never_stored` (creates a token with an `Idempotency-Key`, then searches every table, the RPC ones included, for the secret, and races eight creates)                                |
| 3. Personal organisation slugs came only from `ana`, `ana-2` … `ana-50`, so anyone who knew an address could take all of them and make that person's first sign-in fail.                                                                                | After `ana`, `ana-2` and `ana-3`, slugs end in 40 random bits, which nobody can take in advance.                                                                                                                                                                                                              | `personal_organizations_cannot_be_squatted`                                                                                                                                                            |
| 4. Re-inviting an address revoked its pending invitations without checking the inviter could give their roles, and without auditing each revocation.                                                                                                    | `invite_member` checks `Role::may_give` for every invitation it replaces, and records an `invitation.revoke` entry for each.                                                                                                                                                                                  | `replacing_an_invitation_needs_authority_over_it`                                                                                                                                                      |
| Test gaps: the matrix's inventory filtered operations by tag, and covered neither the CLI, nor tokens by role and project binding; nothing raced the last owner or the caps, or checked a member's tokens after a demotion.                             | The inventory is everything the router serves; the matrix covers role × scope × binding × caller; new races and a demotion scenario.                                                                                                                                                                          | `the_table_names_every_served_operation`, the three matrix tests, `the_last_owner_stays_under_concurrent_changes`, `caps_hold_under_concurrent_creates`, `a_members_tokens_follow_their_role`          |
| The audit log was append-only only by convention.                                                                                                                                                                                                       | The database can't say so for an app table (see the audit log above), so a test holds the convention.                                                                                                                                                                                                         | `server::audit::tests::only_this_module_writes_the_audit_log_and_only_inserts`                                                                                                                         |

What `leptoskit_rpc` should offer instead of the action: a mutation whose result is never kept, such as `#[rpc::mutation(receipt = "outcome")]`. Its receipt would keep the request ID, the argument hash and whether it succeeded, but not the result, and a repeat with the same key would answer `409` ("already done; its answer isn't kept") instead of replaying. That is the same contract `create_token` now builds by hand from unique names. A field-level `#[rpc::redact]` that blanks marked result fields in the stored receipt would also work, but it is easier to forget on a new field than a per-operation switch.

Still open from the review: rate limiting for the control plane's operations (sign-in has its own) belongs to a separate slice.

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

| Slice                                                            | Status and evidence                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| ---------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Foundation: `[cloud]`, its validation and schema                 | built; 13 unit tests in `crates/leptoskit_config/src/cloud/` (slugs, sizes, scales, regions, domains) and 21 in `crates/leptoskit_config/tests/cloud.rs`: every rule in §5 with its message, the contract's example, unchanged serialization without `[cloud]`, and the JSON schema accepting the example and refusing bad values                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| Foundation: deployment states, blockers, sources                 | built; `crates/leptoskit_cloud/src/deployment.rs`, including a test over every pair of the 47 states (seven, plus `failed` for each of four stages and ten blockers) against the diagram                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| Foundation: the source archive                                   | built; 8 unit tests and 14 in `crates/leptoskit_cloud/tests/archive.rs`: the walk, a real Git checkout, `.leptoskitignore`, identical bytes from two packs, the round trip with modes and links, secrets, bounds, and a hand-built hostile tar (absolute and `..` paths, escaping and chained links, a loop, writing through a link, collisions, hard links, FIFOs, lying headers). Unix only; Windows isn't exercised                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| Foundation: the price estimate                                   | built; 9 tests in `crates/leptoskit_cloud/tests/pricing.rs` with made-up prices: allowances, rounding, included usage, a plan by agreement, an unpriced meter, saturation, and catalogue validation                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| Foundation: client safety                                        | built; `crates/leptoskit_cloud/tests/dependencies.rs` reads the wasm dependency tree with and without `archive`, and clippy runs for `wasm32-unknown-unknown`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| Control 1: accounts, organisations, tokens, projects, the server | built; HTTP tests against a served control plane in `crates/leptoskit_cloud/tests/control.rs` (20: the personal organisation made once under eight concurrent first calls and never blocked by squatted slugs, anonymous and forged credentials, tokens shown once, hashed, expiring, revoked, never stronger than their maker, never making tokens or members, and following their maker's role, a token's secret in no table even with an `Idempotency-Key` and a retry refused by name, invitations through the in-memory mailbox with the wrong address, reuse, revocation, replacement and an inviter's lost authority, keeping an owner, also when two owners change each other at once, caps under eight concurrent creates, isolation between same-named projects and by raw IDs, stale revisions, the audit log's actor with no secrets, the device flow then `whoami`, a second first-party client refused, operators, the CLI client registered once); `tests/authorization.rs` (6) checks `access::OPERATIONS` against everything the router serves and the OpenAPI document, then runs every operation as a person with each role, as each through the CLI, as 60 tokens (role × scope × project binding) and as nobody, checking each answer's status and code; `tests/postgres.rs` runs 16 of the scenarios on Postgres when `initdb` is on the path, and fails without it when `LEPTOSKIT_REQUIRE_POSTGRES` is set; `crates/leptoskit_cloud_app` has 5 configuration tests and 4 that run the binary (`--dev` refusing a non-loopback address, missing and bad keys refused without echoing secrets, `migrate` then `serve` answering `/readyz`, `operator grant` then `whoami`) |
| Platform                                                         | offline configuration, OpenTofu, two-phase k3s bootstrap and Kubernetes add-on generation built in `leptoskit_cloud_platform`; deterministic launch/growth snapshots, secret-reference inventory and CLI tests. [Operator procedure](cloud-platform.md) records chart sources and unvalidated live checks. No cluster provisioned; price catalogue and cost model remain unimplemented.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| Runtime 1                                                        | desired-state types, validation, deterministic Kubernetes manifests and recording fake built in `leptoskit_cloud_runtime`; unit tests, manifest snapshots and the feature-free wasm dependency check. Quotas include one surge pod per rolling process (`Quota::for_processes`); tests cover minimum xs rollout capacity and Recreate. No manifests applied to a real cluster.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| Runtime 2                                                        | Kubernetes adapter behind `kube`: server-side apply, safe pruning, rollout observation, status, bounded logs and namespace deletion; pure unit tests and loopback axum API tests. Sleep routing matches HTTP add-on 0.16.0; runtime/platform profiles and worker RBAC agree in generated snapshots. `observed_restarts` sums only the current ReplicaSet existing pods; `last_oom_kill` reports the latest retained OOM termination timestamp. Replaced pods take their evidence with them; neither field is cumulative history. No real-cluster apply performed; the opt-in test requires `LEPTOSKIT_TEST_KUBECONFIG`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| Control 2–4, Worker 1–3, CLI 1–3, Design, UI 1–3                 | not started                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| Stores, end to end, public docs                                  | not started                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |

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
- the control plane has run only in tests and `--dev`. Its authorisation has had one review, whose findings are fixed (§6, "Security review of Control 1"), but no external audit, and the operations have no rate limit beyond sign-in's own;
- the default domains (`cloud.leptoskit.dev`, `*.leptoskit.app`) are configuration defaults, not registered infrastructure.
