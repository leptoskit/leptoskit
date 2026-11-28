# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-28)

### Features

- **JSON RPC wire and ambient context.** Queries and mutations speak JSON through serde on `POST /rpc/<name>`. `[rpc]` sets `path`, `live_path`, `transports`, and `default_transport`; the only transport is `json`. A query or mutation can omit its context argument and call `rpc::cx()` or `rpc::mutation_cx()`. _Owner:_ Ifiok Jr. · _Introduced in:_ [8bd1891](https://github.com/leptoskit/leptoskit/commit/8bd1891b8d987c9abe19128343e7107d706ecd88)

#### Keep a crate's operations behind its own server feature

_Owner:_ Ifiok Jr. · _Introduced in:_ [784ba50](https://github.com/leptoskit/leptoskit/commit/784ba50c9e9009f79cbbe2b99e6d59c55c836ec8)

An API crate that other crates link for its client, such as a CLI calling a hosted service, can now keep its operations' bodies and registrations behind one of its own features:

```toml
[package.metadata.leptoskit.rpc]
server-feature = "server"

[features]
server = ["leptoskit_rpc/server"]
```

Built without that feature, the crate's `#[rpc::query]`, `#[rpc::mutation]`, job, and stream operations keep their argument structs and an `Operation` impl that fails with a message naming the feature. Nothing registers them, so a server in a build that links the crate for its client neither compiles their bodies nor serves them, even when `leptoskit_rpc/server` is on for another crate's sake. `#[api]` describes them in the spec only where they are served. A `server-feature` that names a feature the crate doesn't declare is a compile error. Cargo doesn't rebuild a crate when only its metadata changes, so run `cargo clean -p <crate>` after adding or removing the key.

`#[api(scope = …)]` on an operation now reaches its description. Before, operations were registered with no scopes, so `leptoskit_idp` refused every third-party access token on them.

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

### Fixes

- **A stream body that omits its context and reads it with `rpc::cx()` now sees the query context the server passed in, while it is constructed and while its items are polled.** Streams ran outside the query task, so `rpc::cx()` answered "outside a query" and the stream failed; the whole body now runs inside the context's task — constructing the stream and driving its items — the same as queries and mutations, so a lazily-polled stream keeps the context too. _Owner:_ Ifiok Jr. · _Introduced in:_ [8bd1891](https://github.com/leptoskit/leptoskit/commit/8bd1891b8d987c9abe19128343e7107d706ecd88)
- **Configured RPC endpoints now work for clients alongside the server configuration.** The clients previously called the compile-time defaults: the app's own client posted every call to `/rpc/<op>`, the browser's server-sent events fallback opened `/live/events` and posted frames to `/live/frames`, and the exported spec described every operation at `/rpc/<op>` however the app was configured. The spec export now carries both mounts (`rpc_path` and `live_path`, spec version 4), composes every rpc operation's route from the configured prefix — a service's spec under the service's own prefix — and the foreign and generated SDK clients call those routes and open that socket, so `leptoskit rpc generate` output follows the configuration. The app's own client follows it too: `leptoskit dev` and `leptoskit build` compile the browser and native clients with `LEPTOSKIT_RPC_PATH` and `LEPTOSKIT_RPC_LIVE_PATH` from `leptoskit.toml`, and `ClientConfig::rpc_path`/`live_path` name the mounts by hand for builds that don't go through the CLI. A service's operations keep the default mounts under the service's prefix: `[rpc].path` moves the app's own. Defaults are unchanged (`/rpc`, `/live`, legacy `/api`), as are scope lookup, auth headers, service credentials, and service origin isolation. _Owner:_ Ifiok Jr. · _Introduced in:_ [8bd1891](https://github.com/leptoskit/leptoskit/commit/8bd1891b8d987c9abe19128343e7107d706ecd88)
- **No `service` cfg warning in apps that call a service.** `#[rpc::query(service = "...")]`, and the other attributes that take `service`, no longer make Cargo warn `` unexpected `cfg` condition value: `service` `` in a crate that calls a service without declaring a `service` feature, such as an app. The attributes read whether the crate declares the feature: a service's own crate keeps the body behind it as before, and every other crate gets the typed stub directly, which is what it got before. _Owner:_ Ifiok Jr. · _Introduced in:_ [b4efffc](https://github.com/leptoskit/leptoskit/commit/b4efffc4eb47c86b523454bfc2508c0fcd97a1e1)

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

### Breaking changes

#### leptoskit_rpc_macros: RPC attributes

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_rpc_macros` provides the attributes behind `leptoskit_rpc`: `#[query]`, `#[mutation]`, and `#[action]` declare operations; `#[stream]`, `#[channel]`, `#[job]`, and `#[cron]` declare live streams, channels, background jobs, and scheduled jobs; and `#[api]` publishes a leptos server function to generated clients.

Use them through `leptoskit_rpc`, which re-exports them. Every target gets the operation's typed argument struct so any client can call it; only a server build keeps the function body and registers its route.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
