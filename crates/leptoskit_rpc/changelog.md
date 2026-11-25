# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-25)

### Features

#### Limit request rates and estimate a `[cloud]` table

_Owner:_ Ifiok Jr. · _Introduced in:_ [373ad29](https://github.com/leptoskit/leptoskit/commit/373ad295394a26e69e883a6b28983d35b7e3f02d)

The LeptosKit Cloud control plane now counts every operation against who asked: a person, an API token and its organisation's tokens together, a worker, or the address of a request that is nobody's. A caller past their rate is answered `429` (`rate_limited`) until the window ends. `ControlConfig` gains `rate_limits` and `client_ip_header`, so code that builds one by hand must set them; the server reads the header's name from `LEPTOSKIT_CLOUD_CLIENT_IP_HEADER`. An organisation's checkout and portal pages have a rate of their own.

`leptoskit_cloud::billing` adds `estimate`: what a month of a whole `[cloud]` table is expected to cost on a plan, before anything is deployed.

A backup's dump has a size limit, `ShippingSettings::backup_cap`. Workers are told it in `BackupDatabaseInput::max_bytes` and `RestoreDatabaseInput::max_bytes`, and a dump a worker reports as larger is a backup that failed.

`leptoskit_rpc::Error` gains `TooMany`, answered as `429`, for an operation or an `Identify` that refuses a caller who asks too often. `Identify` gains `admit`, asked before each subscription, mutation, action and stream a live connection sends, so a limit on how often a caller asks covers a connection that is identified once. It admits everything unless implemented.

A `[cloud]` table with more than 50 environments, 50 services or 50 hostnames in an environment is now refused as `invalid_config` before it is resolved.

- **JSON RPC wire and ambient context.** Queries and mutations speak JSON through serde on `POST /rpc/<name>`. `[rpc]` sets `path`, `live_path`, `transports`, and `default_transport`; the only transport is `json`. A query or mutation can omit its context argument and call `rpc::cx()` or `rpc::mutation_cx()`. _Owner:_ Ifiok Jr. · _Introduced in:_ [8bd1891](https://github.com/leptoskit/leptoskit/commit/8bd1891b8d987c9abe19128343e7107d706ecd88)
- **`AppCaller`: call a service as the app from another program.** `AppCaller::new(service, url, token)` calls one service's operations as the app, the same way `call_as_app` does, but it uses the address and token you hand it instead of reading them from the environment. It's for a separate program that acts for the app, such as its admin. `.call(op)` refuses an operation that belongs to a different service. Keep it on the server, like the token it holds. _Owner:_ Ifiok Jr. · _Introduced in:_ [60e11ae](https://github.com/leptoskit/leptoskit/commit/60e11ae75fdffff7d6da8bc30d4369aced2e9d41)
- **Configured RPC endpoints now work for clients alongside the server configuration.** The clients previously called the compile-time defaults: the app's own client posted every call to `/rpc/<op>`, the browser's server-sent events fallback opened `/live/events` and posted frames to `/live/frames`, and the exported spec described every operation at `/rpc/<op>` however the app was configured. The spec export now carries both mounts (`rpc_path` and `live_path`, spec version 4), composes every rpc operation's route from the configured prefix — a service's spec under the service's own prefix — and the foreign and generated SDK clients call those routes and open that socket, so `leptoskit rpc generate` output follows the configuration. The app's own client follows it too: `leptoskit dev` and `leptoskit build` compile the browser and native clients with `LEPTOSKIT_RPC_PATH` and `LEPTOSKIT_RPC_LIVE_PATH` from `leptoskit.toml`, and `ClientConfig::rpc_path`/`live_path` name the mounts by hand for builds that don't go through the CLI. A service's operations keep the default mounts under the service's prefix: `[rpc].path` moves the app's own. Defaults are unchanged (`/rpc`, `/live`, legacy `/api`), as are scope lookup, auth headers, service credentials, and service origin isolation. _Owner:_ Ifiok Jr. · _Introduced in:_ [8bd1891](https://github.com/leptoskit/leptoskit/commit/8bd1891b8d987c9abe19128343e7107d706ecd88)

#### `[rpc].path` and `[rpc].live_path` decide where routes serve

_Owner:_ Ifiok Jr. · _Introduced in:_ [8bd1891](https://github.com/leptoskit/leptoskit/commit/8bd1891b8d987c9abe19128343e7107d706ecd88)

`RpcConfig.path`, `live_path`, and `transports` were accepted and validated but no server read them: every app served `/rpc` and `/live` whatever the config said. Now they decide the mounts, once, at the app's entrypoint.

The generated app reads `[rpc]` from `leptoskit.toml` and applies it: operation routes serve at `path` (with the JSON routes kept on the legacy `/api` prefix while JSON is a transport), and `Server::rpc_config` carries the resolved live mount through `Server::live`. `Live` takes the same config, so a router built by hand mounts the socket at `live_path` — `/live` while the config stays at its defaults — with its server-sent events and frame posts right next to it. `path` without `live_path` moves the socket to `{path}/live`; an explicit `live_path` mounts it anywhere.

`leptoskit rpc spec` exports the configured live mount, so regenerated clients open their socket where the app serves it. Generated HTTP calls still name the compile-time `/rpc/<op>` route: keep the default `path` when generated clients call the app directly, and use a custom one behind a proxy or gateway that remounts the prefix.

#### Rate limits, timeouts, and guards for public operations

_Owner:_ Ifiok Jr. · _Introduced in:_ [4c41c6e](https://github.com/leptoskit/leptoskit/commit/4c41c6e61ebf8a586cfdd4e1e6183a9beafbffdc)

Every query, mutation, action, and stream a client can call can now be limited before its body runs, over HTTP and over a live connection alike.

```rust
use leptoskit::rpc::server::{Rate, RateLimit, Server};

let rpc = Server::new(db)
	.identify(auth)
	.client_ip_header("fly-client-ip")
	.rate_limit(RateLimit::new().anonymous(Rate::per_minute(60)))
	.guard(SignedInOnly);
```

- `Server::rate_limit(RateLimit)` counts each caller: by address when nobody is signed in (IPv6 by its /64), and by `Identity::caller` when someone is, with `RateLimit::key` to count every session of one person together. The defaults are 120 calls a minute per address and 600 per signed-in caller, refilled evenly (GCRA). Services and the app calling them aren't counted unless `RateLimit::internal` says so. A refused call is `429` with `Retry-After`, which clients already read as `FnError::Refused` with the code `rate_limited`.
- Counts live in a bounded `MemoryStore` by default. Implement `RateStore` to share them between servers, for example over Redis.
- `#[rpc::query(rate_limit = 10 / 1m, timeout = 5s, max_body = 64KiB)]`, and the same on `#[rpc::mutation]` and `#[rpc::action]`, set one operation's limits. An operation's own `rate_limit` holds even when the server sets no `RateLimit`. They are on `Registration::limits` as `OperationLimits`.
- A call past its timeout is answered `504` with the code `timed_out` (`rpc::server::TIMED_OUT`); arguments over the size limit are answered `413` with `too_large` (`rpc::server::TOO_LARGE`) instead of a `400`. `Server::timeout` and `Server::max_body` set the defaults; the body limit is still 2 MiB unless changed.
- `Server::guard(impl Guard)` runs an app's own check before every call. A `Guard` sees a `Call`: the operation's name and kind, the identity, the client's address, and whether it came over HTTP or a live connection. `Live` and `LiveStreams` take the same `guard`, `rate_limit`, and `client_ip_header`, and `Server::live()` passes its server's on.
- The HTTP routes now identify the caller and run the guards before reading the request body, so a refused caller costs no more than their headers. Calls the server makes in process while it renders a page are not checked again.

Rate limiting is opt-in for existing apps. New database and sign-in projects from `leptoskit new` start with `Server::new(db).rate_limit(RateLimit::new())` in the backend's `operations`.

`Registration` has a new `limits` field: code that builds one by hand adds `limits: OperationLimits::NONE`.

#### Let an operation refuse with a code and a wait

_Owner:_ Ifiok Jr. · _Introduced in:_ [bd68e95](https://github.com/leptoskit/leptoskit/commit/bd68e9525308319c899d7e4125b570cf2b0d905f)

A query, mutation, or stream can now say why it refused, in a form a screen can match on without comparing sentences. `rpc::Error::rate_limited(wait)` answers `429` with a `Retry-After` header, and `rpc::Error::refused("notes_full", "Your notebook is full")` answers `409` with a code the app chose. Build `rpc::Error::Refused { code, message, status }` for another status.

The caller gets the new `FnError::Refused { status, code, message, retry_after }`, with `FnError::code()` and `FnError::retry_after()` as shortcuts. It arrives the same way over a plain call, the live socket, and a stream, on every platform. A refusal is never retried.

The generated Rust, TypeScript, Dart, Go, and Python clients read it too: each gains a `refused` kind with the code and the wait, and each `LiveError` carries them over the live connection.

Things to know when upgrading:

- `leptoskit_rpc::Error` and `FnError` each gained variants, and `leptoskit_rpc::live::WireError` gained the optional fields `code` and `retry_after`. Code that matches on them without a wildcard arm, or builds a `WireError` by hand, needs updating.
- The generated Rust client's `Error` gained a `Refused` variant, and its `LiveError` two fields.
- A client built or generated before this release reads a refusal as an `other` error whose text is the raw `Refused|{…}` body. Release clients before the server starts refusing this way.

#### Server-rendered queries hydrate without a mismatch

_Owner:_ Ifiok Jr. · _Introduced in:_ [c451911](https://github.com/leptoskit/leptoskit/commit/c451911f920e3f81df7154ede1e9cc0bff52ff10)

A page that rendered a query on the server now hydrates as the same page in the browser. Before, only a query's data was carried, so a query that failed on the server started from "loading" in the browser, leptos reported a hydration mismatch, and the page stopped updating.

- `QueryScope::with_ssr_errors` (and `with_error_codec`) streams a server render's error alongside the data, so the browser shows the same error first. RPC queries and paginated queries opt in, and `FnError` is now `Serialize` and `Deserialize`.
- A query seeded from the server doesn't fetch while the page hydrates. Its mount refetch — stale data in the background, or a retry after an error — starts once hydration has finished, so the first render matches the server's HTML.
- On the server, reading a query that is still loading inside `<Suspense>` holds the boundary until it settles, as a leptos resource does, so the server renders the outcome it streams.
- Streamed payloads escape `<`, `>`, `&`, and U+2028/U+2029, so data and errors can't close the `<script>` they're embedded in.

Apps using `render = "ssr"` should read queries inside `<Suspense>`; outside a boundary the server renders whatever the query holds at that moment.

New apps put their web adapter inside that boundary, including the database and sign-in scaffolds. Existing portable apps should wrap the closure that calls `leptoskit::web::render` in the same way.

Generated browser entrypoints configure the RPC and auth clients inside the mount callback, after leptos has set up its executor and owner, so the live socket starts on the first page load. A query observer reads `enabled` without subscribing while it binds or runs its refetch interval, so an `enabled` closure that reads another query no longer logs a reactive-tracking warning on every page.

#### Serve streams from a server that has no database

_Owner:_ Ifiok Jr. · _Introduced in:_ [f7bf762](https://github.com/leptoskit/leptoskit/commit/f7bf762393787df34f5c07c85f9741e6cf6be5e9)

A `#[rpc::stream]` used to need the full live server, which reads a database's change log, so a service with something to stream and no database could not serve one.

- **`rpc::StreamCx`** is a new context for a stream's body. It holds who is calling (`get`, `require`, `signed_in`, `identity`) and no database. A stream that takes it runs on any live server. A stream that reads the database keeps `rpc::QueryCx`.
- **`rpc::server::LiveStreams`** serves streams alone, on the same routes as `Live` (the WebSocket, its server-sent events, and their frame posts) with the same limits. It takes `identify`, `origin`, `for_service`, `rpc_config`, `stall_timeout`, and `drain`. `Live::streams_only()` returns one too. A query, mutation, action, or channel sent to it is answered with a `501` that says why.
- **`live-streams`** is a new feature of `leptoskit_rpc` (`rpc-live-streams` through `leptoskit`). It needs no database driver and opens no connection. `live` now includes it.

Nothing changes for a client: `use_stream` reads either server.

Things to know when upgrading:

- A stream with no context argument no longer requires a database driver in the build. It needs `live-streams` or a driver, and `rpc::cx()` inside it works only where the server has a database.
- `StreamOperation::pump` and `StreamRegistration::open` now take the hidden `rpc::server::StreamOpen` in place of a `QueryCx`. Code generated by `#[rpc::stream]` follows by itself; a registration built by hand needs the new type.
- `LiveStreams` keeps an identity until its lease expires or the connection ends. It can't watch the rows a lease reads, because it has no change log.

- **Jobs can carry process-local typed runtime state through `Jobs::state` and read it with `JobCx::state`.** This state stays separate from the durable job owner's authorization context and is never serialized. Notifications routes and workers now share an explicit push runtime. Queued push arguments contain only send and round IDs, keeping database URLs and credentials out of job metadata and allowing pending work to recover with a fresh runtime after restart. _Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)

### Fixes

- **A stream body that omits its context and reads it with `rpc::cx()` now sees the query context the server passed in, while it is constructed and while its items are polled.** Streams ran outside the query task, so `rpc::cx()` answered "outside a query" and the stream failed; the whole body now runs inside the context's task — constructing the stream and driving its items — the same as queries and mutations, so a lazily-polled stream keeps the context too. _Owner:_ Ifiok Jr. · _Introduced in:_ [8bd1891](https://github.com/leptoskit/leptoskit/commit/8bd1891b8d987c9abe19128343e7107d706ecd88)
- **Stop live queries when their screen goes away.** Live query hooks now unsubscribe when their reactive owner is cleaned up or their arguments change. Buffered updates are cancelled too, so a removed screen cannot repopulate the query cache after an account change. _Owner:_ Ifiok Jr. · _Introduced in:_ [1bd0618](https://github.com/leptoskit/leptoskit/commit/1bd0618c284e4dff885524d3d6a23db9cfc668f8)

#### Retries stop when a host cannot schedule the wait

_Owner:_ Ifiok Jr. · _Introduced in:_ [1bd0618](https://github.com/leptoskit/leptoskit/commit/1bd0618c284e4dff885524d3d6a23db9cfc668f8)

A keyed HTTP mutation and `call_with_retry` now surface the last network error when the host has no usable timer, instead of waiting forever on a backoff that can never finish. This keeps a native app's Save button from staying on "Saving…" when its host predates the guest timer plugin or rejects the timer.

A live subscription created before the socket opens is sent once: the handshake records which connection delivered it, so the command that woke the connection no longer sends a duplicate `Sub` frame.

- **No dead-code warnings for a client-only wasm build.** A crate that turns on only `leptoskit_rpc`'s `client` feature and builds for `wasm32-unknown-unknown`, such as a shared types crate a dashboard compiles, no longer fails `clippy -D warnings` with `service_origin` and `at_service` "never used". Those helpers are now compiled only where something reaches a service: the browser (`web`), a native app (`guest`), and a server (`services`). Nothing changes for builds that use them. _Owner:_ Ifiok Jr. · _Introduced in:_ [784ba50](https://github.com/leptoskit/leptoskit/commit/784ba50c9e9009f79cbbe2b99e6d59c55c836ec8)
- **Record server argument defaults in exported schemas without executing their default functions.** Strict Go, Rust and CLI generation now reports nullable arguments whose omission invokes a server default but whose explicit null cannot be represented by the generated client. Ordinary optional arguments remain accepted. Strict generation also rejects query/form parameters containing free-form JSON, whose empty arrays and objects cannot round-trip through that codec. Use JSON input encoding for those endpoints. Typed lists and maps remain supported. _Owner:_ Ifiok Jr. · _Introduced in:_ [0857012](https://github.com/leptoskit/leptoskit/commit/0857012d89f7a81925de440a09af034c95a2a4c1)

#### Preserve null, empty collections and omitted arguments in generated SDK requests.

_Owner:_ Ifiok Jr. · _Introduced in:_ [51224bf](https://github.com/leptoskit/leptoskit/commit/51224bf974470ffc7ad6154c57e515f4f83556d3)

Query and form encoders now match the server's serde_qs 1.1 protocol: explicit null is a bare key and an empty collection is `key=`. JSON and live encoders keep explicit null, and required nullable arguments in Go and Rust keep their keys. Python optional arguments use the exported `UNSET` for omission and `None` for explicit null. Generated Rust and CLI packages need `serde_qs` 1.1.3; update an existing package's manifest when regenerating its code.

Clients, OpenAPI, AsyncAPI and compatibility checks now follow each argument's `required` flag rather than inferring it from nullability. Specs from `#[api]` functions mark `Option` arguments optional, because the server reads a missing key as `None`. The spec format is now version 4; older specs still load, and their nullable arguments stay optional. A spec written by this version needs an equally new CLI to generate clients.

In regenerated Dart clients, `Option` arguments become optional `Presence<T?>` parameters instead of required `T?` ones. Omit them, or pass `Presence.present(value)`.

Strict diagnostics report a Go or Rust optional nullable argument only when its schema declares a default, and no longer report a Dart query/form limitation that the encoder now handles. Mandatory SDK acceptance decodes generated HTTP requests with the server codec.

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

### Breaking changes

#### leptoskit_rpc: typed server operations

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_rpc` is the boundary between an app's shared screens and its server. It adds retries and error classification to leptos server functions, and declares operations (`#[query]`, `#[mutation]`, `#[action]`) that run on the database as the signed-in caller and are called the same way from SSR, the browser, and a native guest through `client::call`.

The `live` feature serves queries over a WebSocket, with a server-sent events fallback, and pushes only what changed when a write commits; screens read them with `use_live`, and join channels with presence through `use_channel`. Live connections are bounded: a per-connection frame budget, a timeout for clients that stop reading, channel rooms tied to the arguments they were authorized for, and capped page sizes. `#[api]` publishes server functions so `leptoskit rpc generate` can write TypeScript, Python, Go, Rust, Dart, and command-line clients plus an OpenAPI document. Pick a database with the `postgres` or `sqlite` feature, and spread live updates across several servers with `relay`.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
