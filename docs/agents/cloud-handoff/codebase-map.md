  I've gone through all ten areas. Three facts change the plan the most:
  
  - **Something cloud-shaped already exists.** `crates/leptoskit_app` builds a `leptoskit-cloud` binary: a hosted broker for accounts, Stripe billing, per-account journals, encrypted OpenTofu state and the worker API. `lk console worker` already talks to it. There are no `[cloud]` table, `leptoskit_cloud` crate or `lk cloud` command yet.
  - **Plain Rust can't call an app's operations through `leptoskit_rpc` today.** The only reqwest-backed paths are for service operations (`call_as_app`, `AppCaller`). Otherwise you generate a typed reqwest client with `lk rpc generate --target rust`.
  - **The auth stack has a device-code flow but no organisations, teams, API keys or personal access tokens.**
  
  Line numbers come from the worktree as read on 2026-11-18. `ReportFindings` isn't used here because this is an exploration, not a code review.
  
  ---
  
  ## 1. Service crate template: `crates/leptoskit_notifications`
  
  **Files**
  - `src/lib.rs` (1636 lines): the shared types, tables and every operation.
  - `src/service.rs`, plus `src/service/{config,push,transport/*}.rs`: the server-only bodies and helpers.
  - `src/main.rs`: the binary.
  - Client-only modules: `src/browser.rs`, `src/native.rs`, `src/push.rs`, `src/condition.rs`.
  - `migrations/` (SQLite) and `migrations-postgres/`. Each step is a `<timestamp>_<name>/` folder holding `migration.sql`, `down.sql` and `schema.json`. Each folder also has a `registry.txt` listing the order, appended by `leptoskit db migration create`.
  
  **Cargo.toml** (`crates/leptoskit_notifications/Cargo.toml`)
  - `[[bin]] leptoskit_notifications`, path `src/main.rs`, `required-features = ["service"]` (line 14).
  - Each `[[test]]` target sets `required-features`. `service`, `live`, `topics`, `segments`, `push` and `boundary` need `service`; `postgres` needs `postgres`; `browser` needs `browser` (lines 20–64).
  - Always-on dependencies are client-safe only: `base64`, `chrono`, `leptoskit_db`, `leptoskit_rpc` (features `["client"]`), `schemars`, `serde`, `serde_json`, `url`, `uuid` (`v7`, `serde`).
  - Optional dependencies gated by `service`: `httpdate`, `hex`, `http`, `leptoskit_axum`, `leptoskit_query`, `reqwest` (`json`, `rustls-tls`, `http2`), `ring`, `sha2`, `tokio`.
  - Features (line 136 on):
    - `default = []`
    - `browser`: js-sys, `leptoskit_service_worker`, thiserror, wasm-bindgen, `leptoskit_rpc/web`
    - `native`: `leptoskit_bridge`, `leptoskit_plugin_notifications/guest`, `leptoskit_rpc/guest`
    - `service` (line 167): the deps above plus `leptoskit_db/sqlite`, `leptoskit_rpc/server`, `leptoskit_rpc/sqlite`, `leptoskit_rpc/live`
    - `postgres = ["service", "leptoskit_db/postgres", "leptoskit_rpc/postgres"]` (line 183)
  - `[package.metadata.leptoskit.service]` (line 186) is what `lk` reads to run the crate as a service: `name = "notifications"`, `port = 3120`, `public = true`, `features = ["service"]`, `database = true`. It is parsed as `ServiceManifest` in `crates/leptoskit_config/src/services.rs:135`.
  - The feature must literally be named `service`. The RPC macros check `CARGO_MANIFEST_DIR/Cargo.toml` for it (`crates/leptoskit_rpc_macros/src/manifest.rs`). When it's present they put `#[cfg(feature = "service")]` on the bodies and a stub under `not(service)` (`operation.rs:1530–1549`).
  - `tests/dependencies.rs` holds a `BANNED` list (axum, dbus, deadpool-postgres, hex, hyper, leptoskit_axum, reqwest, rusqlite, tokio, tokio-postgres). It runs `cargo tree` to check that the client feature sets never pull these in.
  
  **How `lib.rs` is split.** The module declarations are:
  ```rust
  pub mod condition;
  #[cfg(feature = "browser")] pub mod browser;
  #[cfg(feature = "native")]  pub mod native;
  #[cfg(feature = "service")] pub mod service;
  pub mod push;
  pub const SERVICE: &str = "notifications";
  ```
  Tables and operations sit at the top level of `lib.rs`, so the app compiles their argument types. Bodies call into `service::*` helpers, which only exist under `service`. The macro handles the cfg gating.
  
  **Table declaration** (`lib.rs:154–191`):
  ```rust
  #[table(name = "notifications", live, unique(send_key, person), index(person, id))]
  pub struct Notification {
  	#[db(key)] pub id: uuid::Uuid,
  	#[db(min_len = 1, max_len = 255)] pub send_key: String,
  	pub person: uuid::Uuid,
  	#[db(min_len = 1, max_len = 64)] pub category: String,
  	...
  	pub expires_at: Option<leptoskit_db::Timestamp>,
  	#[db(created_at)] pub created_at: leptoskit_db::Timestamp,
  }
  ```
  Other forms in the same file:
  - `pub id: leptoskit_db::Id<Self>` as an auto key (`TopicSubscription`, line 251).
  - `#[db(one_of("fanning_out","complete","cancelled"), default = "complete")]` (line 220).
  - `#[db(updated_at)]` (line 301).
  - `#[table(name = "category_preferences", live, unique(person, category))]` (line 270).
  - `leptoskit_db::export_schema!();` (line 353) lets `leptoskit db` read the tables.
  
  The macro also generates a `New{Name}` insert struct (`crates/leptoskit_db_macros/src/table.rs:2576`).
  
  **Query and insert** (`set_category_preference`, `lib.rs:1321–1378`):
  ```rust
  let existing = Preference::query()
  	.filter(|p| p.person.eq(me) & p.category.eq(category.clone()))
  	.first(cx.tx()).await?;
  if let Some(mut preference) = existing { preference.opted_out = opted_out; preference.save(cx.tx()).await?; ... }
  let inserted: leptoskit_db::Result<()> = NewPreference { person: me, category, opted_out }
  	.insert(cx.tx()).await.map(|_| ());
  match inserted { Err(leptoskit_db::DbError::Conflict { .. }) => Err(rpc::Error::Contended), ... }
  ```
  Updates look like this (`cancel`, lines 1159–1163):
  ```rust
  Notification::update().filter(|row| row.id.eq(notification.id)).set(|row| row.cancelled_at, Some(now)).run(cx.tx()).await?;
  ```
  Other query methods: `.all(db)`, `.count(db)`, `.is_null()`, `.is_not_null()`. Pagination uses `rpc::paginate(cx.db(), query, &page, |n| n.id.desc(), |n| n.id)` (line 1215).
  
  **Operations.** All three are excerpted, with line numbers:
  ```rust
  // lib.rs:908
  #[rpc::mutation(service = "notifications")]
  pub async fn send(cx: &rpc::MutationCx, idempotency_key: String, spec: SendSpec, target: SendTarget,
  	#[serde(default)] validate_only: bool) -> rpc::Result<SendResult> {
  	if cx.identity().caller.as_deref() != Some(rpc::server::APP_CALLER) {
  		return Err(rpc::Error::forbidden("only the app sends notifications: ..."));
  	}
  	...
  	cx.enqueue(FanoutSend { send_key: idempotency_key.clone(), step: 0 })
  		.at(leptoskit_db::now()).key(format!("fanout:{idempotency_key}:0")).await?;
  	...
  }
  // lib.rs:1050
  #[rpc::query(service = "notifications")]
  pub async fn send_status(cx: &rpc::QueryCx, idempotency_key: String) -> rpc::Result<SendProgress> {
  	if cx.identity().caller.as_deref() != Some(rpc::server::APP_CALLER) { return Err(rpc::Error::forbidden("...")); }
  	check_key(&idempotency_key).map_err(rpc::Error::invalid)?;
  	let Some(send) = service::find_send(cx.db(), &idempotency_key).await? else {
  		return Err(rpc::Error::not_found("no send with this idempotency key exists"));
  	};
  	Ok(service::progress_of(&send))
  }
  // lib.rs:1032
  #[rpc::job(service = "notifications", queue = "fanout", retries = 8, timeout = 5m)]
  pub async fn fanout_send(cx: &rpc::JobCx, send_key: String, step: u32) -> rpc::Result<()> {
  	service::fanout_step(cx, &send_key, step).await
  }
  ```
  - `#[rpc::job(service = "notifications")]` with no options: `wake_send` (line 1079).
  - The person comes from `service::me(cx.identity())?`, which reads the forwarded identity.
  - Mutations use `cx.tx()`; queries use `cx.db()`.
  - Macro keys accepted by operations/jobs (`crates/leptoskit_rpc_macros/src/operation.rs:300–470`): `name`, `service`, `crate`, `retries`, `queue`, `backoff`, `deadline`, `timeout`, `serialize_by`, `visible_to`, `on_failure`.
  - Errors, with HTTP status (`crates/leptoskit_rpc/src/operation.rs:209`): `Invalid` 400, `Unauthorized` 401, `Forbidden` 403, `NotFound` 404, `Conflict`/`Contended` 409, `Server`/`Permanent` 500, `Retry` 503.
  
  **Migrations**
  - Embedded with:
    ```rust
    #[cfg(feature = "service")] pub static MIGRATIONS: &[leptoskit_db::Migration] = leptoskit_db::migrations!("migrations");
    #[cfg(feature = "postgres")] pub static POSTGRES_MIGRATIONS: ... = leptoskit_db::migrations!("migrations-postgres");
    ```
    (lines 356–362.)
  - Applied in `main.rs` with `for name in db.migrate(migrations).await? { println!("applied migration {name}"); }`, choosing the set by whether the URL starts with `postgres`.
  - `migration.sql` files carry the header `-- @generated by leptoskit db ... -- dialect: sqlite` and are written by `lk db --service notifications migration create <name>` (`crates/leptoskit_cli/src/cli.rs:411–418`).
  
  **Binary** (`src/main.rs`, 110 lines):
  ```rust
  leptoskit_axum::service::main!(leptoskit_notifications, run);
  async fn run(env: ServiceEnv) -> Result<Router, Error> {
  	env.expect_name(SERVICE)?;
  	let url = env.database_url().ok_or("...")?;
  	let db = match env.database_pool() { Some(n) => Db::connect_with(url, PoolOptions { max_connections: n }).await?, None => Db::connect(url).await? };
  	/* migrate */
  	let (router, live) = service::routes_with_settings(db.clone(), env.public_origin(),
  		ServiceCallers::for_service(Anonymous), push.clone(), open_topics);
  	live.start().await?;
  	service::workers_with_push(db, push).start();
  	Ok(router)
  }
  ```
  - `main!` is defined at `crates/leptoskit_axum/src/service.rs:657–668`. It links the library, builds a multi-threaded runtime, reads `ServiceEnv::from_env()` and serves with graceful drain.
  - Environment variables: `LEPTOSKIT_SERVICE_{NAME,ADDR,TOKEN}`, `LEPTOSKIT_PUBLIC_ORIGIN`, `LEPTOSKIT_APP_URL`, `DATABASE_URL`, `LEPTOSKIT_SERVICE_DATABASE_POOL`, `LEPTOSKIT_SERVICE_DRAIN_MS`, `LEPTOSKIT_SERVICE_VOLUME_*` (lines 52–76).
  - The router is built in `service.rs:1702–1721`:
    ```rust
    let server = rpc::server::Server::new(db).service(SERVICE).identify(callers);
    let live = match origin { Some(o) => server.live().origin(o), None => server.live() };
    Router::new().nest(&rpc::service_prefix(SERVICE), server.router().merge(live.router()))
    ```
    This mounts at `/_leptoskit/services/notifications`.
  - Workers (`service.rs:1693`): `rpc::server::Jobs::new(db).state(push).service(Some(SERVICE)).poll(1s).retention(Some(24h))`.
  - Request-scoped settings travel through a wrapping `Identify` impl that adds to `identity.context` (`WithSettings`, lines 1755–1785).
  
  **Tests**
  - `tests/service.rs`: the main HTTP suite.
    - Fresh database: `leptoskit_db::testing::sqlite_migrated(MIGRATIONS).await`.
    - `start(db)` builds `ServiceCallers::new(Anonymous).caller(TOKEN, rpc::server::APP_CALLER).forwarded_identity(SERVICE, TOKEN)`, binds `127.0.0.1:0`, runs `axum::serve` in `tokio::spawn`, and starts workers with `.poll(50ms)`.
    - Calls go through `reqwest`: POST `http://{addr}/_leptoskit/services/notifications/rpc/{op}` with a JSON body and `bearer_auth(token)` (lines 96–115).
    - Person tokens are minted with `rpc::server::IdentityKey::new(SERVICE, TOKEN)?.mint(person, issued_at, rpc::server::IDENTITY_LIFETIME)`.
  - Other suites: `live.rs`, `topics.rs`, `segments.rs`, `push.rs` (fake push boundary), `boundary.rs` (real local HTTP servers built from hyper), `browser.rs` (wasm in Chrome), `native.rs`.
  - **Postgres is tested in two ways:**
    - `tests/postgres.rs` (feature `postgres`) uses `tests/support/postgres.rs`. That helper starts a private Postgres from local `initdb`/`pg_ctl` binaries found through `LEPTOSKIT_POSTGRES_BIN`, `PGBIN` or `PATH`, with trust auth, a temp directory and a random port. Tests are skipped when the binaries are missing, unless `LEPTOSKIT_REQUIRE_POSTGRES` is set (CI sets it at `.github/workflows/ci.yml:862`).
    - `tests/service.rs:1223` has an `#[ignore]` test, `postgres_behaves_like_sqlite`, that reads `LEPTOSKIT_NOTIFICATIONS_TEST_POSTGRES`.
  
  ---
  
  ## 2. RPC clients from plain Rust
  
  **What exists in `leptoskit_rpc`**
  - Features (`crates/leptoskit_rpc/Cargo.toml`):
    - `client`: the base.
    - `web`: browser `fetch`.
    - `guest`: the native HTTP plugin.
    - `server`: in-process calls during server rendering.
    - `services = ["client", "dep:reqwest"]`.
  - Backend choice is in `client/mod.rs:349–381`. On non-wasm without `server` plus a database, the app's own operations get `protocol::Unreachable`, which errors with *"this build can't reach the server..."* (`protocol.rs:252`). **A plain Rust program like `leptoskit_cli` cannot call an app's own operations through `rpc::client::call`.**
  - For **service** operations, non-wasm builds with `services` get a reqwest client (`src/client/reqwest.rs`). It is async, uses one shared `reqwest::Client` with a 30s timeout, and always POSTs.
    - `rpc::client::call_as_app(Op{..})` (`client/mod.rs:493`) reads `LEPTOSKIT_SERVICE_<NAME>_URL` and `_TOKEN`.
    - `rpc::AppCaller::new("analytics", "http://127.0.0.1:3110", token).call(Run { query }).await?` (`client/mod.rs:540–610`) takes the URL and token explicitly. `leptoskit_admin` uses `rpc` with the `services` feature.
  - `configure(ClientConfig::new().origin(..).credentials(impl Credentials))` sets things up. `Credentials` (`protocol.rs:69`) has `authorization()` and `refused()`, which renews once on a 401.
  
  **Wire format** (`client/protocol.rs`, `server/http.rs`)
  - `POST {origin}{rpc_path}/{name}`. `rpc_path` defaults to `/rpc` (`RPC_ROUTE_PREFIX`, `lib.rs:221`) and is configurable through `[rpc].path`. The legacy `/api` prefix is kept while `json` is in the transports.
  - A service operation goes to `{origin}/_leptoskit/services/{service}/rpc/{name}` (`service_prefix`, `service.rs:43`).
  - Headers: `accept: application/json`, `content-type: application/json`, optional `authorization: Bearer …`, and `idempotency-key` on mutations (`IDEMPOTENCY_KEY`, `server/http.rs:27`). A keyed mutation is retried up to 3 times on network errors.
  - Body: the operation's arguments as one JSON object of named parameters, for example `{"idempotency_key": "...", "spec": {...}, "target": {"recipients": [...]}}`. There is no envelope.
  - Success: 2xx, `application/json`, the bare result value. An empty body becomes `null`.
  - Error: the status from `Error::status()` with a `text/plain` body of `Kind|message`. Kind is `Args` for `Invalid`, otherwise `ServerError` (`operation.rs:248`, `server/http.rs` `failure()`). The client parses it with `FnError::from_wire` (`lib.rs:475`).
  - Body limit: 2 MiB.
  
  **Codegen and `lk rpc generate`** (`crates/leptoskit_rpc_codegen`, `crates/leptoskit_cli/src/cli.rs:44–137`)
  - Targets (`generate.rs:14–42`): `typescript` (fetch), `python` (stdlib with optional httpx), `go` (net/http), `rust` (async **reqwest**), `dart` (package:http), `cli` (clap, one command per function, with a built-in RFC 8628 device-flow `login`; template at `templates/cli_login.rs`), and `openapi` (JSON and YAML).
  - The generated Rust crate depends on neither leptos nor `leptoskit_rpc`. It has one `pub async fn <method>(&self, …)` per function, a `with_idempotency_key(HeaderValue)` copy method, and a method to set default headers such as `Authorization` (`rust.rs:853`, `1297–1440`).
  - Spec export: `lk rpc spec [--out F] [--format json|openapi|openapi-yaml]`. It builds the server, runs it with `LEPTOSKIT_RPC_SPEC_OUT=<file>`, and `leptoskit_rpc::spec::export_if_requested()` writes the file and exits.
  - Spec shape (`ApiSpec`, `leptoskit_rpc_codegen/src/spec.rs:306`): `{ leptoskit_rpc_spec: 4, title, version, description, servers, functions[], channels[], live_path, rpc_path, schemas{} }`. `#[rpc::query]`/`#[rpc::mutation]` operations are included (`spec.rs:104–112`), and a service's spec can be selected.
  - OpenAPI: `leptoskit_rpc::spec::openapi_router(&SpecOptions)` (feature `axum`, `spec.rs:737`) serves `GET /openapi.json`, `/openapi.yaml` and `/asyncapi.json`.
  - `lk rpc check --against <spec>` fails on breaking changes.
  
  ---
  
  ## 3. Does the CLI already talk to a hosted API?
  
  Yes, through `lk console`, `lk deploy` and `lk infra`, with **ureq 3** (blocking).
  
  **`crates/leptoskit_cli/src/console/client.rs`** (420 lines)
  - `Connection { url, key, project, root }`. `Connection::for_project()` returns `None` when `LEPTOSKIT_CONTROL_LOCAL=1` or when `[infra]` has no `control_url`. Otherwise it reads the managed `[infra]` (`infra.name`, `infra.control_url`).
  - Credentials:
    - The `LEPTOSKIT_CONTROL_KEY` environment variable (32–256 printable ASCII characters).
    - Otherwise a file at `$XDG_CONFIG_HOME` (or `~/.config`) `/leptoskit/control/<sha256(url/project)>.json` containing `{"key": ...}`. It must be mode 0600 (checked by trailing zeros ≥ 6).
    - **There is no OS keychain.**
    - Files are written by `private_write` (temp file plus persist, parent directory 0700).
  - `agent(origin, timeout)`: ureq with no redirects, `http_status_as_error(false)`, a global timeout, and no proxy for loopback. `origin()` accepts only HTTPS, or HTTP on localhost, with no path, credentials, query or fragment.
  - Every request sends `Authorization: Bearer <key>`. Error bodies are JSON `{"error": "<code>"}`. Known codes: `revision_conflict`, `operation_in_progress`, `plan_is_not_current`, `idempotency_conflict`, `authentication_required`, `scope_or_origin_denied`.
  - Endpoints:
    - `GET /api/v1/overview`
    - `GET /api/v1/projects/{project}/analytics`
    - `POST /api/v1/tokens` with `{scope, project}` → `{key}`
    - `POST /api/v1/projects/{project}/operations` with an `Intent { expected_revision, idempotency_key, action }`
    - `GET /api/v1/operations/{id}`, polled once a second until the phase is no longer `Queued`/`Running`
  - A pending intent is saved to `.leptoskit/control-request.json` for crash-safe resubmission. Plans are saved to `.leptoskit/control-plan.json`.
  - `connect(url)` validates the project against the overview, stores the key, and writes `infra.control_url` into `leptoskit.toml` with `toml_edit`.
  
  **`console/hosted_worker.rs`** (206 lines): the worker pairs with the hosted `leptoskit-cloud`.
  - **Pairing:** first run needs `LEPTOSKIT_PAIRING_CODE`. It calls `POST /api/v1/worker/pair {"code"}` → `{installation:{project, ownership}, key}` and stores `{url, project, key, ownership}` in `<data>/worker.json` (private). Managed ownership `leptoskit_digitalocean` requires DigitalOcean compute and a budget.
  - **Claim:** loop on `POST /api/v1/worker/claim {}` with the Bearer key. The answer is `null` (sleep 2s) or a `Job { operation, analytics_read_key, analytics_ingest_key }` (`leptoskit_deploy_console::hosted::Job`, `hosted/mod.rs:28`).
  - **Execute:** write `pending.json {"operation": id}`, then run `CommandRunner` with `hosted_state = ("{url}/api/v1/state/{project}", key)`.
  - **Ack:** write `pending.json {"operation", "result"}`, then `POST /api/v1/worker/finish {id, succeeded, message, details}`. On success, delete `pending.json`; on failure, retry every 5s.
  - A `pending.json` without a result means the worker crashed mid-execution, and manual reconciliation is required.
  - The server routes are in `leptoskit_deploy_console/src/hosted/mod.rs:56–67`:
    - `/api/v1/account`
    - `/installations`, `/installations/{p}/pair`, `/installations/{p}/revoke`
    - `/billing/{action}`
    - `/webhooks/stripe`
    - `/worker/{pair,claim,finish}`
    - `/state/{project}`
    - `/api/v1/{*path}` (proxy)
  
  **Command tree** (`crates/leptoskit_cli/src/cli.rs:153–470`). `Cli` has a global `--app-target`.
  
  | Command | Description |
  |---|---|
  | `targets` | Validate and list the destinations in this app's configuration |
  | `console <sub>` | Host or connect to the deployment dashboard and durable agent API |
  | `admin <sub>` | Run the admin: analytics, infrastructure and lessons, signed in through the main site |
  | `infra` (alias `tofu`) | Generate and manage the app's OpenTofu infrastructure |
  | `deploy` | Deploy a Git commit through its service, release a prebuilt image, or roll back |
  | `verify` | Build and reproduce backends from public commits; check signed deployment policies |
  | `migrate` | Upgrade `leptoskit.toml` to the current schema |
  | `native` | Build native hosts and generate plugin registration |
  | `test` | Run component and device tests |
  | `device` | Control attached devices, apps and websites |
  | `devtools fetch` | Manage the DevTools interface |
  | `debug` | Print a running app's tree, errors and commits |
  | `mcp` | Serve the device control API over MCP |
  | `publish` | Upload to the stores `[publish]` names |
  | `skill` | Print or install the leptoskit agent skill |
  | `rpc {spec,list,check,generate}` | Describe the RPC and generate clients |
  | `generate {app,plugin,auth,tool-plugin,deploy}` | Generators |
  | `new` | Scaffold a project |
  | `dev` | Run every platform with rebuild |
  | `game` | Build or run a game |
  | `build` | Build the app or graph units |
  | `serve` | Serve built output |
  | `services` | List the app's services |
  | `routes` | Write the typed `Route` module |
  | `l10n` | Write the translations module |
  | `db` (global `--service`) | Migrations |
  | `jobs {list,retry,cancel}` | Background jobs |
  | `auth` | Sign-in configuration, keys, providers, IdP clients |
  | `codepush` | Code push releases |
  | `doctor` | Check local tooling |
  | `budgets` | Caps and boot checks |
  | `workspace` | Validate a workspace and release apps |
  | `format` | Format Rust and `view!` markup |
  | `lint` | leptoskit rules plus clippy |
  | `completions` | Shell completions |
  
  **Declaration and dispatch for the four you asked about**
  - Declarations:
    - `Console { #[command(subcommand)] command: crate::console::ConsoleCommand }` (line 158)
    - `#[command(alias = "tofu")] Infra { #[command(subcommand)] command: crate::infra::InfraCommand }` (line 170)
    - `#[command(disable_version_flag = true)] Deploy(crate::infra::ReleaseArgs)` (line 177)
    - `Publish(crate::publish::PublishArgs)` (line 232)
  - Dispatch is in `pub fn run(cli: Cli)` (line 827):
    - `Command::Console { command } => crate::console::run(command)` (854)
    - `Command::Infra { command } => crate::console::infra(command)` (856)
    - `Command::Deploy(args) => crate::console::deploy(&args)` (857)
    - `Command::Publish(args) => crate::publish::run(&args)` (873)
  - In `console.rs`, `deploy()` (line 264) and `infra()` (line 272) send `Plan`, `Apply`, `Status` and `Analytics` to `client::*` when a `Connection` exists, and fall back to local `crate::infra::run` or `release` otherwise.
  - `ConsoleCommand` (`console.rs:23–92`): `Serve`, `Connect`, `Status`, `Worker`, `Token`, `Backup`, `Restore`, `RollbackDatabase`. `TokenScope` is `Read`, `Deploy` or `Ingest`.
  - `InfraCommand` (`infra.rs:47`): Release, Graph, …, Analytics, Plugins, Build, Generate, Plan, Apply, Prepare, Output, Releases, Status, and others.
  
  ---
  
  ## 4. Auth (`leptoskit_auth`, `leptoskit_idp`)
  
  **Sign-in methods**, routes in `leptoskit_auth/src/http.rs:219–323`:
  - **Email one-time codes** (`email.rs`; `/auth/email/send` and `/auth/email/verify`). One flow signs in or signs up. **There are no magic links**: grepping "magic link" finds nothing.
  - **Passwords** (`passwords.rs`, feature `password`): Argon2id, NIST rules, optional HIBP check, resets by emailed code.
  - **TOTP and recovery codes**, MFA and step-up (`mfa.rs`, `totp.rs`).
  - **Passkeys / WebAuthn** (`passkeys.rs`, feature `passkey`, webauthn-rs plus vendored OpenSSL).
  - **OAuth/OIDC** (`oauth.rs`, `oauth/catalog.rs`, feature `oauth`): Google, Apple, Microsoft, GitHub, Facebook, X, Discord, LinkedIn, GitLab, Slack, Twitch, Spotify, Reddit, Amazon, Dropbox, Bitbucket, Notion, Zoom, TikTok, Yahoo, and generic `Provider::oidc(id, name, issuer, client_id, secret)`. Linking, unlinking and merging are supported (`merge.rs`).
  - **Solana wallets / SIWS** (`siws.rs`, `wallets.rs`, feature `solana`, ed25519-dalek): link, fresh proof, optional sign-in.
  - **Dev sign-in**: `/auth/dev/sign-in`, enabled with `.dev_sign_in()`.
  
  **`leptoskit_idp`** is an OIDC provider over the same accounts:
  - Authorization code with PKCE S256, PAR, **device flow (RFC 8628)** (`device.rs`; `POST /oauth/device_authorization` and `GET`/`POST /oauth/device`), `client_credentials`, rotating refresh tokens, userinfo, revocation, introspection, end-session, dynamic registration, a developer portal (`/oauth/apps*`), DPoP and `private_key_jwt`.
  - Routes are in `leptoskit_idp/src/http.rs:110–200`.
  - Typed scopes: `#[api(scope = …)]`.
  - `lk generate auth --idp` writes a `/device` code-entry page (`crates/leptoskit_cli/src/auth_pages/templates/route_device.rs`).
  
  **Enabling auth on an app server** (`examples/admin_demo/src/main_site.rs:86–97`, then 130–160):
  ```rust
  let auth = Auth::builder(&db, Keys::single("main-site-demo", [5; 32]))
  	.app_name("Notes").origin(MAIN_SITE).mailer(Mailbox::new())
  	.dev_sign_in().on_sign_up::<User>().roles::<User>().build()?;
  let idp = Idp::builder(&db, &auth).sign_in_page("/sign-in").consent_page("/consent").build()?;
  let router = Router::new()./*routes*/.merge(auth.router()).merge(idp.router());
  let router = router.layer(auth.layer()).layer(idp.layer());
  ```
  - Other builder methods (`auth.rs:206–417`): `limits`, `sessions`, `codes`, `client_ip_header`, `dev_mailbox`, `mfa`, `gate`, `solana`, `on_merge`, `native_apps`, `provider`, `provider_http`, `passwords`, `breached_passwords`, `sign_in_methods`.
  - `Auth::from_config` also exists for `[auth]`.
  - Tables come from the `auth` schema pack: `leptoskit::db::packs!(leptoskit_auth, leptoskit_idp)`.
  
  **Organisations, teams, API keys**
  - **There are no organisations, teams, tenants or memberships** in auth or idp.
  - Roles are only an app-supplied hook: `trait Roles { fn roles(db, account) -> Result<Vec<String>> }` (`auth.rs:771`), used by `MfaPolicy::required_for` and released as the IdP `roles` claim. Tenancy is left to the app, through `rpc::Identity` context (example in a doc comment at `leptoskit_rpc/src/server/identity.rs:110`).
  - **There are no API keys or personal access tokens in auth or idp** (no hits for `api_key`, `personal access` or `pat`). The nearest options:
    - IdP `client_credentials` for app-as-itself tokens.
    - The device flow for CLIs.
    - The deploy console's own scoped keys (`leptoskit_deploy_console::control::Scope::{Owner, Read, Deploy, Ingest}`, `store.issue(...)`), on rusqlite and separate from leptoskit_auth.
  
  **Browser sessions vs native tokens.** Sessions are opaque 256-bit tokens; only their hashes are stored, and they have idle and absolute lifetimes.
  - In the browser (`client` + `web`), the session lives in an HttpOnly cookie and calls are same-origin `fetch`.
  - In a native Wasm guest (`guest`), there is no cookie jar. `client/native.rs` `TokenKeeper` keeps the access token in memory and the refresh token in the secure-store plugin, trades it at `/auth/refresh` 60s before expiry or after a 401, and serialises trades because refresh reuse ends the session.
  - The server layer (`http.rs:358–370`) accepts `Authorization: Bearer` or the cookie. `RequestInfo.transport` is `Bearer` or `Cookie`.
  - Third-party IdP access tokens arrive as a `Delegation { signed_in, client_id, scopes }` extension (`http.rs:126`). With the `rpc` feature, `Auth` implements `Identify`, so operations run as the signed-in person.
  
  **Email.** `trait Mailer { fn send(&self, mail: Mail) -> MailFuture<'_>; }` with `Mail { to, subject, text }`, plain text only (`mail.rs:11,42`). Implementations: `Mailbox` (in-memory, for dev and tests), `SmtpMailer::new(url, from)` (feature `smtp`, lettre), and `WebhookMailer` (feature `webhook`, a signed POST with reqwest).
  
  **How `leptoskit_admin` signs admins in** (`crates/leptoskit_admin/src/lib.rs:370–405`, `src/access.rs`):
  ```rust
  let provider = Provider::oidc(access::PROVIDER, &button, &main_site.issuer, &main_site.client_id,
  	Some(main_site.client_secret.clone()))?.userinfo(true).scopes(["openid","email","profile","roles"]);
  let mut auth = Auth::builder(&db, keys).app_name(...).origin(&origin).mailer(Mailbox::new())
  	.provider(provider).gate(access::AdminGate { roles: self.roles.clone() })
  	.roles::<access::AdminRoles>().sign_in_methods([format!("oidc:{}", access::PROVIDER)]);
  ```
  - The main site must run `leptoskit_idp` and register the admin as a `first_party` client with the `roles` scope (`main_site.rs:99–118`, `NewClientSpec { first_party: true, scopes: [... "roles"], ... }`).
  - The `roles` scope (`ROLES_SCOPE`, `leptoskit_idp/src/idp.rs:248`) releases `"roles": [..]` from the site's `Roles` hook.
  - `AdminGate` (`access.rs:117–160`) reads `SignInAttempt::roles()` (`auth.rs:800`), requires one of the configured roles (default `["admin"]`), and stores them in the `admin_roles` table. `AdminRoles` reads them back.
  
  ---
  
  ## 5. UI app structure
  
  **Builder calls vs `view!`.** `AGENTS.md:38` says crates, tools and tests use builder calls (`View().class(…).child(Text(…))`), while examples and docs use `view!`. `lint:leptoskit` enforces this per `[lint] markup = "builder" | "view" | "any"`. `leptoskit_app` and `leptoskit_deploy_console` set `markup = "builder"`; `huddle`, `commonplace` and `exact` set `"view"`.
  
  **Hydration in the newest code**
  - `leptoskit.toml` `[app] render = "csr" | "ssr" | "islands"` (`RenderMode`, `crates/leptoskit_config/src/lib.rs:669`). For SSR, the server package builds with `ssr` and the wasm client with `hydrate`.
  - The `leptoskit` crate's features: `csr`, `hydrate`, `islands`, `ssr` (`crates/leptoskit/Cargo.toml:92–95`).
  - Example: `examples/analytics/analytics_app` (`render = "ssr"`).
    - Features: `ssr = ["dep:leptoskit", "leptos/ssr", "leptos_router/ssr", "leptoskit_rpc/services"]` and `hydrate = [..., "leptos/hydrate", "leptoskit_rpc/web"]`.
    - Client entry in `src/lib.rs`:
      ```rust
      #[cfg(feature = "hydrate")] #[wasm_bindgen] pub fn hydrate() {
      	console_error_panic_hook::set_once();
      	let _ = any_spawner::Executor::init_wasm_bindgen();
      	leptoskit_rpc::client::configure(leptoskit_rpc::client::ClientConfig::new());
      	leptos::mount::hydrate_body(app::App);
      }
      ```
    - `src/main.rs` reads `get_configuration(None)`, builds `router(options).merge(readiness.routes())`, and serves with `leptoskit::server::serve_listener_with(listener, app, GracefulShutdown::new().readiness(&readiness))`.
  - `examples/islands` uses `render = "islands"`. `examples/admin_demo/project/leptoskit.toml` declares `render = "ssr"`.
  - **`leptoskit_admin` does not hydrate.**
    - It renders server-side only: builder `Element` → `leptoskit::web::render(body).to_html()` inside a hand-built document string (`src/shell.rs:782–795`).
    - **It still ships `assets/admin.mjs`** (1149 lines of hand-written JS), via `include_str!` in `src/server.rs:51` and served at `{BASE}/assets/admin.mjs` (`BASE = "/_leptoskit/admin"`).
    - Fonts are `include_bytes!` too.
    - So the "no hand-written JS" goal is not yet met there.
  - **`leptoskit_app`** uses `render = "csr"`, features `web`/`csr`/`native`/`ios`/`android`/`server`/`site`, and `crate-type = ["rlib", "cdylib"]`. `web/bootstrap.js` (44 lines) preloads `/pkg/leptoskit_app_bg.wasm` and the glue. `web/landing.mjs` and `playground.mjs` exist as well.
  
  **File routes** (`leptoskit_router`)
  - `examples/file_routes` enables the router with `[[plugins]] name = "router"`.
  - Routes live in `src/routes/` with conventions: `_layout.rs`, `index.rs`, `[param].rs`, `[[optional]]`, `[...rest].rs`, `(group)/`, `+not-found.rs`.
  - `src/routes.rs` is generated by `lk routes` (`// @generated`, `#[path]` modules).
  - A route file looks like:
    ```rust
    #[derive(Route, Clone, Debug, PartialEq)] pub struct Device { pub user_code: Option<String> }
    pub fn page(route: &Device) -> Element { ... }
    ```
    (from `crates/leptoskit_cli/src/auth_pages/templates/route_device.rs`.)
  
  **Live queries in components.** Hooks:
  - `leptoskit_rpc::use_live(|| Op { .. })` returns a handle with `.data()`.
  - `use_live_paginated`, `use_rpc_query`, `use_mutation(mutation_scope::<Op>())`, `use_action::<Op>()`, `use_inbox()`, `use_channel`, `use_live_status`, `use_stream`.
  
  Example from `examples/huddle/huddle_app/src/huddle.rs:102–135`:
  ```rust
  let me = use_live(|| huddle_api::Whoami {});
  messages: use_live(move || huddle_api::Messages { channel: channel(), limit: huddle_api::PAGE, before: None }),
  read: use_mutation(mutation_scope::<huddle_api::ReadChannel>()),
  ...
  Effect::new(move |_| { let Some(channels) = self.channels.data() else { return; }; ... });
  ```
  `analytics_app/src/app.rs:60` has `let summary = use_live(|| Summary { days: DAYS });` with `Suspense`.
  
  **Forms** (`leptoskit_form`)
  - Headless API: `Form::new("signup").with_field(FieldSpec::new("email","Email address").required().validate(|v| ...))`, `FormState::initial(&form)`, `.for_change(&form, "email", v)`, `.for_blur`, `submit(&form, &state) -> Result<Values, FormState>`, `field_wiring`, `input` (`crates/leptoskit_form/src/lib.rs:25–70`).
  - The only non-test consumer is `leptoskit_devtools/src/form.rs` (`track_form`). **No example app uses it.**
  
  **Charts.** `crates/leptoskit_admin/src/charts.rs` (396 lines): `LineChart { series, threshold, markers, x_labels, format }`, `line_chart(&LineChart, label) -> Element`, `sparkline(&[f64])`, and formatters `milliseconds`, `percent`, `count`, `bytes`. Paths are computed in Rust and drawn as `leptoskit::native::vector` `VectorPath`s. **Everything is `pub(crate)` and `mod charts` is private, so it is not reusable from another crate without moving or exposing it.**
  
  **`leptoskit_app` UI and design tokens**
  - Builder components from `leptoskit::prelude` (`ScrollView`, `View`, `Heading`, `Text`, `Pressable`, `Button`, `PasswordInput`), styled with Tailwind classes.
  - `src/screen.rs` (297 lines) is the console. It defines constants such as `BUTTON = "rounded-lg border border-slate-300 px-3 py-2 text-sm dark:border-slate-700"`, plus `CARD` and `QUIET`, using slate and dark variants. State lives in `Shell { key, account, overview, ... RwSignal }` (`lib.rs`).
  - Data comes from `client::request(key, path, body)`: `leptoskit_plugin_http` on web and native with a `Bearer` header, against `/api/v1/*` of the hosted broker. It does not use leptoskit_rpc.
  - `src/landing.rs` (1348 lines) is the public and hosted landing page: builders plus the declarative `Motion` API (`rise`, `arrive`, `wipe_on_stage`, …), with `Mode::{Public, Hosted}`.
  - Styles: `web/landing.css` (1787 lines) maps local tokens (`--paper`, `--ink`, `--ember`, `--mint`, `--display`, `--space-*`, …) onto the shared Skribble theme variables (`--sk-paper`, `--sk-ink`, `--sk-coral`, …). Those are defined in `docs/src/theme/skribble.css`, which is loaded by `web/index.html` as `/docs/theme/skribble.css` together with `/console.css`.
  - Skribble look: Skribble fonts (Noto Sketch, Mono Playful woff2), "wobbly" border radii, and generated SVG frames and masks (`--sk-frame: url("../_leptoskit/generated/frame-1-light.svg")`). These are produced by the pinned `skribble_rough` crate (a git dependency in the workspace) in `docs/site/src/ink.rs`. The admin reuses the same Skribble fonts.
  
  ---
  
  ## 6. Config (`crates/leptoskit_config/src/lib.rs`)
  
  **Top-level fields of `LeptoskitConfig`** (lines 72–154). The struct has `#[serde(deny_unknown_fields, default)]` and `#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]`.
  
  | Field | Type |
  |---|---|
  | `frontends` | `BTreeMap<String, application::FrontendConfig>` |
  | `targets` | `BTreeMap<String, application::TargetConfig>` |
  | `selected_target` | `Option<String>` (serde skip) |
  | `schema_version` | `u32` |
  | `app` | `AppConfig` |
  | `serve` | `ServeConfig` |
  | `build` | `BuildConfig` |
  | `bundle` | `BundleConfig` |
  | `native` | `NativeConfig` |
  | `codepush` | `CodePushConfig` |
  | `rpc` | `RpcConfig` |
  | `router` | `Option<RouterConfig>` |
  | `l10n` | `Option<L10nConfig>` |
  | `db` | `Option<DbConfig>` |
  | `infra` | `Option<infra::InfraConfig>` |
  | `auth` | `Option<AuthConfig>` |
  | `extension` | `Option<ExtensionConfig>` |
  | `publish` | `PublishConfig` (skip if empty) |
  | `plugins` | `Vec<PluginSpec>` |
  | `services` | `BTreeMap<String, ServiceConfig>` |
  | `artifacts` | `BTreeMap<String, deployment::Artifact>` |
  | `format` | `FormatConfig` |
  | `lint` | `LintConfig` |
  | `theme` | `Option<ThemeConfig>` |
  
  **Adding a new top-level table**
  1. Add a module (pattern: `pub mod publish;`, re-exported with `pub use publish::PublishConfig;`, lines 31 and 48).
  2. Add the field with `#[serde(skip_serializing_if = "Option::is_none")]` or `is_empty`, and update `impl Default` (156–185).
  3. Types derive `Debug, Clone, PartialEq, Serialize, Deserialize`, use `#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]` and `#[serde(deny_unknown_fields, default)]`. Doc comments become schema descriptions.
  4. Validation: add a `problem() -> Option<String>` method and call it from `LeptoskitConfig::validate()` (line 3686). The pattern is:
     ```rust
     if let Some(p) = self.infra.as_ref().and_then(infra::InfraConfig::problem) { return Err(ConfigError::InvalidInfra(p)); }
     ```
     You would also add a `ConfigError` variant. `validate` is called from `from_toml` (line 3612), which `load_from` (3590) uses.
  5. Schema: `src/schema.rs` `configuration(id)` derives from the types with `schemars`, and hand-patches conditionals in `$defs`. `devenv shell schema:update` runs `cargo run -p leptoskit_config --features schema --example schema`, plus the testing schema. That writes `schemas/leptoskit.schema.json` and `schemas/leptoskit.v0.schema.json` (`crates/leptoskit_config/examples/schema.rs:16–21`). `schema:check` runs with `--check` in `lint:all` and `lint:push`.
  6. `SCHEMA_VERSION = 0` is unstable; CONTRIBUTING.md §"Configuration schema" covers migration steps for breaking changes.
  
  **`[publish]`** (`src/publish.rs`)
  ```rust
  #[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
  #[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
  #[serde(deny_unknown_fields, default)]
  pub struct PublishConfig {
  	#[serde(skip_serializing_if = "Option::is_none")] pub chrome: Option<ChromeStore>,
  	... edge, firefox, app_store, macos, play
  }
  #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
  #[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
  #[serde(deny_unknown_fields)]
  pub struct ChromeStore { pub publisher_id: String, pub item_id: String }
  ```
  - `pub const STORES: [&str; 6] = ["chrome","edge","firefox","app_store","macos","play"]`.
  - `stores()` returns the configured ones in upload order.
  - `problem(&self, app: &Facts) -> Option<String>` (line 202) uses `Facts { has_extension, has_firefox_id, platforms, apple_team_id }`.
  - `chrome_problem` checks a 32-letter `a–p` item ID and a non-empty publisher. Schema regexes go through `#[cfg_attr(feature = "schema", schemars(regex(pattern = ...)))]` (`AppStore`).
  - Wired in through `LeptoskitConfig::publish_problem` (lines 4033–4043), giving `ConfigError::InvalidPublish`.
  
  **Cloud name search** (git grep excluding target and dist)
  - `\[cloud\]`: **0 hits**.
  - `leptoskit_cloud`: **0 hits**.
  - `lk cloud`: **0 hits**.
  - `leptoskit.app`: only `com.leptoskit.app` default package IDs (`leptoskit_config/src/lib.rs:3499–3500`, `tests/config.rs:509–511`) and the APNs topic `dev.leptoskit.app` in notifications tests and transport (`transport/mod.rs:578`, tests).
  - `leptoskit.cloud`: one hit, `crates/leptoskit_app/leptoskit.toml:22` `package_id = "dev.leptoskit.cloud"`.
  - **Related:** the `leptoskit-cloud` binary (`crates/leptoskit_app/Cargo.toml` `[[bin]] name = "leptoskit-cloud" path = "src/server.rs"`), its deploy files (`crates/leptoskit_app/deploy/leptoskit-cloud.{service,sysusers}`, `pilot/leptoskit.toml.example`), `.github/workflows/hosted-pilot.yml:136–140`, and `.templates/crates.t.md:42`.
    - The binary takes `--data .leptoskit-cloud` and `--url` and has subcommands `Serve`, `Account`, `RotateKey`, `Managed`, `Interrupt`, `Backup`, `Restore`, `RollbackSchema`.
    - It needs the env vars `LEPTOSKIT_HOSTED_KEY`, `STRIPE_SECRET_KEY`, `STRIPE_WEBHOOK_SECRET`, `STRIPE_CONTROL_PRICE` and `STRIPE_TEAM_PRICE`.
    - It uses `leptoskit_deploy_console::hosted::{Hosted, Billing, Ownership}`.
  
  ---
  
  ## 7. Publish store client shape (`crates/leptoskit_cli/src/publish/`)
  
  **There is no trait.** Each store is a concrete struct holding `&Http`, with inherent methods. For example `edge.rs`:
  ```rust
  pub const API: &str = "https://api.addons.microsoftedge.microsoft.com";
  pub const CLIENT_ID: &str = "LEPTOSKIT_PUBLISH_EDGE_CLIENT_ID";
  pub const API_KEY: &str = "LEPTOSKIT_PUBLISH_EDGE_API_KEY";
  pub const HOW: &str = "open Partner Center's Publish API page ...";
  pub struct Edge<'a> { http: &'a Http, api: String, product: String, client_id: Secret, api_key: Secret }
  impl<'a> Edge<'a> {
  	pub fn new(http: &'a Http, api: &str, product: &str, client_id: Secret, api_key: Secret) -> Self
  	pub fn upload(&self, zip: Vec<u8>) -> Result<()>
  	pub fn submit(&self, notes: &str) -> Result<()>
  }
  ```
  Sessions call it as `Session::<store>(&self) -> Result<(String /*summary*/, Vec<Step>)>`.
  
  **Every place a new store touches**
  1. `crates/leptoskit_config/src/publish.rs`: a new `Option<XStore>` field on `PublishConfig`, the store struct, a `STORES` entry (6 → 7), `stores()`, and `problem()`.
  2. `publish/<store>.rs`, plus `pub mod <store>;` in `publish/mod.rs` (lines 10–24).
  3. `publish/mod.rs`:
     - `const NEEDED: [Needed; 17]` (line 109), with `struct Needed { store, name, how }` (lines 103–107). Add entries and bump the length.
     - `EXTENSION_STORES` (line 83), if it's an extension store.
     - `PublishArgs.stores` doc (line 45).
     - A new `fn <store>(&self)` on `impl Session` (siblings at lines 411, 451, 488, 596, 815, 952).
     - The match in `Session::publish` (lines 1563–1572): `"chrome" => self.chrome()?, ... other => bail!`.
     - The `<store>_steps(...) -> Vec<Step>` helpers (pattern at lines 1751, 1856, 1917).
     - `icon_step` (1583), `write_workflow` (1610) and `list_credentials` (1647), which iterate stores.
     - API base override: `api("LEPTOSKIT_PUBLISH_<STORE>_API", <store>::API)` (function at line 207).
  4. `publish/tests.rs`.
  5. The schema, regenerated with `schema:update`.
  
  **Credentials** (`secrets.rs`)
  - `Secret(String)`: `Display` prints `[redacted]`, and `.expose()` gives the value.
  - `Credentials::load(file: Option<&Path>, root)` reads a TOML file given by `--credentials`. If the file is inside the project it must be git-ignored.
  - `get(name)` checks the environment first, then the file. `require(name, how)` errors with *"{name} isn't set: {how}. `leptoskit publish credentials` lists every one"*.
  - In `run` (lines 216–330), every `NEEDED` entry for the selected stores is required unless `--dry-run`.
  
  **HTTP seam** (`http.rs`)
  - `pub struct Http { agent: ureq::Agent }`. `Default` gives a 5-minute global timeout and `http_status_as_error(false)`.
  - `pub fn send(&self, method: &str, url: &str, headers: &[(&str,&str)], body: Option<Body>) -> Result<Reply>`, blocking, with a 64 MiB read limit.
  - `Body::{json, bytes, typed, zip, form, multipart}`.
  - `Reply { status, headers, body }` with `.json()`, `.header(name)` and `.ok(action)`.
  - `poll(what, attempts, interval, f)` uses a 5 ms interval under `cfg(test)`.
  
  **Fake-store test pattern** (`tests.rs`)
  - `store(answer: Answer) -> (String /*base*/, StoreServer)` (lines 61–118). It binds a std `TcpListener` on `127.0.0.1:0` and runs axum with a single `.fallback` on its own thread and current-thread tokio runtime. It records every request as `Seen { method, path, query, authorization, content_type, body }`. `Drop` shuts it down and joins.
  - `type Answer = Box<dyn Fn(&Seen, usize /*count*/) -> Response + Send + Sync>`.
  - Example (lines 231–280):
    ```rust
    let (base, seen) = store(Box::new(|request, count| match (request.method.as_str(), request.path.as_str()) {
    	("POST", "/v1/products/prod-1/submissions/draft/package") => accepted("op-upload"),
    	("GET", ".../operations/op-upload") => json(json!({"status": if count < 2 {"InProgress"} else {"Succeeded"}})),
    	...
    	_ => StatusCode::NOT_FOUND.into_response(),
    }));
    let http = Http::default();
    let edge = Edge::new(&http, &base, "prod-1", Secret::new("client".into()), Secret::new("key".into()));
    edge.upload(b"zip".to_vec()).unwrap();
    assert_eq!(seen.lock().unwrap()[0].authorization, "ApiKey key");
    ```
  - End-to-end session tests (`edge_session`, line 1728) set `LEPTOSKIT_PUBLISH_EDGE_API=<base>` and run the whole command with `spawn_publish`.
  
  **Runner seam** (`tools.rs`)
  - `pub struct Tool { program, args, dir, unset: Vec<&'static str>, env: Vec<(&'static str, Secret)> }` with builder `Tool::new("xcrun").arg(..)`.
  - `pub trait Runner { fn run(&self, tool: &Tool) -> Result<()>; fn output(&self, tool: &Tool) -> Result<String>; }`.
  - `System` runs real processes. `#[cfg(test)] Recorder { seen: RefCell<Vec<Tool>>, answer: Box<dyn Fn(&Tool) -> String> }` records them.
  - `Session` holds `runner: &dyn tools::Runner` (line 378).
  
  **Steps model** (`steps.rs:33–50`):
  ```rust
  pub struct Step { pub store: String, pub by: Who /*Agent|Person*/, pub what: String,
  	pub url: Option<String>, pub enter: Vec<Field /*{label,value}*/>, pub why: String, pub done_when: String }
  pub struct Status { pub store: String, pub summary: String }
  pub struct Report { pub stores: Vec<Status>, pub steps: Vec<Step> }  // .text(), .write(dir)
  ```
  The report is written to `.leptoskit/publish/` (next-steps files), with JSON output on `--json`.
  
  ---
  
  ## 8. Kubernetes and deploy generation
  
  **`crates/leptoskit_cli/src/deploy.rs`** (3299 lines) and `deploy/services.rs` (378 lines)
  - Everything is built as **string templates** (`format!`/`write!` into a `String`), not typed structs. YAML values are escaped with `yaml_string`.
  - Inputs: `pub struct Project` (lines 128–178), built only through `Project::read(root: &Path, config: &LeptoskitConfig) -> Result<Self>`, which reads Cargo metadata from disk.
    - Fields: `app`, `binary`, `port`, `output_name`, `platform`, `workspace_root`, `app_root`, `app_dir`, `context`, `dockerfile`, `target_dir`, `toolchain_file`, `rust_version`, `database: Option<Database{dialect,url_env}>`, `secrets`, `has_probes`, `urls`, `split_roles`, `services: Vec<DeployService>`.
    - Two fields are private: `ignore_file` and `ignore_contents`. **A `Project` can't be constructed outside the module.**
  - `pub enum Target { Docker, Compose, Fly, Kubernetes, Systemd }` (line 26). `Target::files()` and `next()` are private.
  - Kubernetes functions, all private: `kubernetes` (2440–2621), `kubernetes_split` (2648–2818, roles web/live/worker/capture from `SPLIT_ROLES`, line 2622), `kubernetes_app_env` (2819), `kubernetes_services` (2853–3112), `kubernetes_next` (3197).
  - Object kinds emitted:
    - `Deployment`: 2487, 2712, 2870.
    - `Service`: 2595, 2790, 3047.
    - `PodDisruptionBudget`: 2606, 2801.
    - `PersistentVolumeClaim`: 2579 (SQLite), 3030 (stateful services).
    - `Ingress`: 3067 (public services).
    - A Secret is referenced (`kubectl create secret generic …` is printed in comments) but **not emitted**.
  - Other generators: `dockerfile` (1176), `compose` (1521), `caddyfile` (2006), `fly` (2065), `systemd` (3113). `deploy/services.rs` has `pub struct DeployService`, `pub fn read(root, config) -> Result<Vec<DeployService>>` (173), `pub fn postgres_setup_sql` (268), and `pub struct Budget`.
  - **Public library entry points:**
    - `pub fn run(&cli::DeployArgs)` (490)
    - `pub fn generate(root, config, target, force) -> Result<Report>` (542)
    - `pub fn generate_with(root, config, target, force, split_roles) -> Result<Report>` (558)
    
    These write files to disk and return `Report { target, changed, kept, next }`.
  - **`leptoskit_cli` is a lib**: `src/lib.rs` has `pub mod deploy;` (plus `console`, `infra`, `publish`, …), it is a published crate in the release group, and `examples/admin_demo` already depends on it. So `leptoskit_cli::deploy::generate_with(...)` is callable. There is no pure in-memory `fn kubernetes(&Project) -> String` API; that function is private.
  
  **Workspace dependencies** (root `Cargo.toml` `[workspace.dependencies]`)
  
  | Crate | In workspace? | Version / notes |
  |---|---|---|
  | kube | no | |
  | k8s-openapi | no | |
  | reqwest | yes | `"0.12"`, `default-features = false` |
  | sqlx | no | |
  | tokio-postgres | yes | `"0.7"`, `default-features = false` (also `tokio-postgres-rustls = "0.14"`, `deadpool-postgres = "0.14"`, `postgres-protocol = "0.6"`) |
  | stripe / async-stripe | no | |
  | aws-sdk-* | no | `rusty-s3 = "0.10"` is present instead |
  | object_store | no | |
  | rustls | yes | `"0.23"`, `default-features = false` (also `webpki-roots = "1"`) |
  | jsonwebtoken | no | idp hand-rolls JWTs with `p256 0.14` and ring |
  | ring | yes | `"0.17"` |
  | aes-gcm | no | |
  | aes-gcm-siv | no | used crate-locally as `"0.11"` in `leptoskit_auth` and `leptoskit_deploy_console` |
  | age | no | |
  | ureq | yes | `"3"`, `default-features = false` |
  
  Also present: `hmac 0.12`, `sha2 0.10`, `subtle 2`, `ed25519-dalek 3`, `rusqlite 0.41.1`, `async-nats 0.51`, `postgresql_embedded 0.22`, `axum 0.8`, `tokio 1`, `leptos 0.9.0-rc0`. `argon2`, `lettre`, `webauthn-rs` and `openssl` are crate-local to `leptoskit_auth`.
  
  ---
  
  ## 9. Workspace conventions
  
  - **Members:** listed explicitly with a `"crates/*"` glob, plus named examples and benchmarks (the comment explains why there is no `examples/*` glob). `default-members = ["crates/*"]`, `resolver = "2"`. Some standalone workspaces are excluded: devtools, examples/commonplace, dev_client, file_routes, huddle/linux, opencode/tui.
  - **`[workspace.package]`:** version `0.3.0`, edition `2024`, license `Unlicense`, `rust-version = "1.95.0"`, homepage `https://leptoskit.dev/`, repository on GitHub. Crates inherit with `x.workspace = true`.
  - **Internal crates:** every publishable crate is listed in `[workspace.dependencies]` as `{ path = "crates/x", version = "0.3.0" }`, and members use `workspace = true`.
  - **Lints:** `[workspace.lints.rust]` denies `unsafe_code`, `unstable_features` and the rust_2021/2024 compatibility groups, and warns on several `unused_*`. `[workspace.lints.cargo] unused_workspace_dependencies = "allow"`. `[workspace.lints.clippy]` sets pedantic, complexity, perf, style and suspicious to warn, correctness to deny, `wildcard_dependencies` to deny, `cognitive_complexity` and `excessive_nesting` to warn, and allows a list of others. Each crate has `[lints] workspace = true`.
  - **`publish = false`:** only `crates/leptoskit_app` and `crates/leptoskit_playground_server` among crates; all examples have it. Published crates omit the key.
  - **Toolchain:** `rust-toolchain.toml` pins `channel = "nightly-2026-10-21"` with components cargo, clippy, rustfmt, llvm-tools-preview, rust-analyzer, rust-src and targets `wasm32-unknown-unknown`, `wasm32-wasip1`.
  
  **Changesets.** Example `.changeset/notifications-segments.md` (most recently added, 2026-11-17). Frontmatter in full; the body is one long paragraph, abridged here:
  ```
  ---
  "leptoskit_notifications": minor
  "leptoskit": minor
  ---
  
  Person attributes, segments, and send to everyone. The service keeps a registry of the people it knows — ... SQLite and Postgres migrations add `persons` and `person_attributes` with the registry backfill.
  ```
  The frontmatter maps crate to bump level; the body is user-facing prose.
  
  **`monochange.toml`** (990 lines)
  - `[defaults] include_private = false` means `publish = false` crates are ignored. `package_type = "cargo"`, `bump_propagation = "patch"`.
  - `[changesets] directory = ".changeset"`.
  - To register a new published crate:
    1. Add its name to `[group.release] packages = [...]` (lines 57–210; `"leptoskit_notifications"` is at line 209). The group is lockstep, `version_format = "primary"`, and `versioned_files` includes `Cargo.toml workspace.package.version`.
    2. Add a section:
       ```toml
       [package.leptoskit_notifications]
       path = "crates/leptoskit_notifications"
       type = "cargo"
       ```
       (lines 764–766).
  - `lint:releases` runs `monochange step validate` and `monochange check`.
  
  **devenv scripts** (`devenv.nix`, `scripts = {` at line 237)
  - **Fix and lint:**
    - `fix:all`: `fix:clippy` then `fix:format`.
    - `fix:clippy`: `cargo clippy --workspace --fix --allow-dirty --allow-staged --all-features --all-targets`, plus file_routes.
    - `fix:format`: `leptoskit:format`, `docs:update`, `dprint fmt`.
    - `lint:all`: lint:clippy, lint:clippy:wasm, lint:monostyle, lint:images, lint:leptoskit, lint:docs, lint:format, lint:kotlin, lint:swift (macOS), lint:releases, schema:check, deny:check.
    - `lint:push` (pre-push hook): gitleaks, the same lints, `test:cargo`, `security:zizmor`, lint:releases, schema:check, deny:check.
    - `lint:clippy`: `setup/scripts/clippy.sh` with warnings denied. `lint:clippy:wasm`: the same for `wasm32-unknown-unknown`.
    - `lint:monostyle`: floor 80 for the tree, 100 for devenv.nix.
    - `lint:images`. `lint:leptoskit`: `leptoskit-lint --deny-warnings examples`. `lint:docs`: `mdt check`. `lint:format`: `leptoskit:format --check` and `dprint check`.
    - `deny:check`: `cargo deny --workspace --all-features check`. `security:zizmor`.
  - **Tests:**
    - `test:all`: test:cargo, test:sdk, test:projects, test:harmonyos, test:docs, plus two deno tests.
    - `test:cargo`: `cargo fetch --locked`, then `cargo nextest run --workspace --all-features`, then nextest for file_routes, dev_client and devtools manifests, then `test:wire`.
    - Also `test:wire`, `test:sdk`, `test:projects`, `test:browser`, `test:plugins:*`, `test:docs`, `test:rive`.
  - **Schema:** `schema:update` and `schema:check` (above).
  - **Release:** `lint:releases`, `release:preview`, `release:publish-check`.
  - **Other:** `snapshot:accept` and `snapshot:review` (insta); `coverage:*`; `benchmark:*`; `docs:update`, `docs:build`, `docs:serve`, `docs:check`; `build:all`, `build:dist`, `build:example`; `install:*`; `setup:*`; `update:deps`; `devtools:build`; `leptoskit`, `leptoskit:dev`, `leptoskit:format`; `check:examples`.
  
  **Running one crate's tests.** There is no dedicated script. The convention (AGENTS.md:31) is to run `devenv shell -- true` once, then for example:
  ```
  CARGO_BUILD_JOBS=4 devenv shell -- cargo nextest run -p leptoskit_notifications --features service
  ```
  - Use `--features postgres` for the Postgres races, with `LEPTOSKIT_REQUIRE_POSTGRES=1` to forbid skipping.
  - `.config/nextest.toml` has a setup script, `setup/scripts/nextest_setup.sh`.
  - Builds go through the `mbx` cargo shim. Don't set `CARGO_TARGET_DIR` or delete `target/`.
  
  ---
  
  ## 10. Stripe billing (`crates/leptoskit_deploy_console/src/hosted/billing.rs`, 649 lines)
  
  - **HTTP:** async `reqwest::Client` with `https_only(true)`, no redirects and a 15s timeout. Raw form-encoded HTTP; **no Stripe SDK**. Every request sends `bearer_auth(key)` and `Stripe-Version: 2026-10-28.endive`. Responses are read in chunks, capped at 1 MiB, and non-2xx becomes `billing_provider_unavailable`.
  - **Endpoints:**
    - `POST /v1/checkout/sessions`: `mode=subscription`, `client_reference_id=<nonce>`, `customer`, `line_items[0][price]`, `[quantity]=1`, `success_url` and `cancel_url` = `{origin}/console/`, `subscription_data[metadata][leptoskit_account]`, with header `Idempotency-Key: lk-checkout-{nonce}`.
    - `POST /v1/billing_portal/sessions`.
    - `GET /v1/subscriptions/{sub_…}`.
    - Redirect URLs are host-checked against `checkout.stripe.com` and `billing.stripe.com`.
  - **Model:** two flat recurring prices (`control_price`, `team_price`, both must start with `price_`). State lives in the rusqlite `hosted_accounts` table: subscription, status, plan, `valid_until`, `synced_at`, and `checkout_*` columns. Calls are serialised by a `tokio::Mutex`.
  - **Webhook verification:** `verify()` parses the `stripe-signature` header for `t=` and `v1=`, rejects anything more than 300s old, computes HMAC-SHA256 over `"{t}.{body}"`, and compares in constant time with `verify_slice`. `webhook()` deduplicates by event ID in `hosted_billing_events`. For `customer.subscription.*` and `checkout.session.completed` it **re-fetches the subscription from Stripe** rather than trusting the payload, with replacement logic for canceled-then-rejoined accounts.
  - **Tests** (inline `#[cfg(test)] mod tests`): `fixture()` opens `Hosted` in a temp directory. `serve(&mut billing, Router)` binds an axum fake on `127.0.0.1:0` and overwrites the private `billing.api_origin` and `billing.client` (non-HTTPS). `signed()` builds a valid `stripe-signature` header. Tests cover checkout idempotency, signed-webhook dedup and tampering, and replacement after cancel.
  - **Reusability:** `pub use billing::Billing` (`hosted/mod.rs:13`), behind feature `server`.
    - Public: `Billing::new(key, webhook, control_price, team_price)` and `apply_subscription(&self, &Hosted, account, &Value)`.
    - `pub(crate)`: `checkout`, `portal`, `refresh` (used by the `/api/v1/billing/{action}` route), `verify` and `webhook`.
    - Everything is coupled to `Hosted` and its rusqlite `accounts().meta` tables.
    - **There is no usage-based or metered billing anywhere** (no hits for "meter", "usage_record" or "metered"). A metering design would need new code, though the HMAC verifier and the fake-Stripe test harness are worth copying.
