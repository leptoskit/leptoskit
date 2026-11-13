# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-13)

### Features

#### Server-rendered queries hydrate without a mismatch

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #756](https://github.com/leptoskit/leptoskit/pull/756) · _Closed issues:_ [#722](https://github.com/leptoskit/leptoskit/issues/722)

A page that rendered a query on the server now hydrates as the same page in the browser. Before, only a query's data was carried, so a query that failed on the server started from "loading" in the browser, leptos reported a hydration mismatch, and the page stopped updating.

- `QueryScope::with_ssr_errors` (and `with_error_codec`) streams a server render's error alongside the data, so the browser shows the same error first. RPC queries and paginated queries opt in, and `FnError` is now `Serialize` and `Deserialize`.
- A query seeded from the server doesn't fetch while the page hydrates. Its mount refetch — stale data in the background, or a retry after an error — starts once hydration has finished, so the first render matches the server's HTML.
- On the server, reading a query that is still loading inside `<Suspense>` holds the boundary until it settles, as a leptos resource does, so the server renders the outcome it streams.
- Streamed payloads escape `<`, `>`, `&`, and U+2028/U+2029, so data and errors can't close the `<script>` they're embedded in.

Apps using `render = "ssr"` should read queries inside `<Suspense>`; outside a boundary the server renders whatever the query holds at that moment.

New apps put their web adapter inside that boundary, including the database and sign-in scaffolds. Existing portable apps should wrap the closure that calls `leptoskit::web::render` in the same way.

Generated browser entrypoints configure the RPC and auth clients inside the mount callback, after leptos has set up its executor and owner, so the live socket starts on the first page load. A query observer reads `enabled` without subscribing while it binds or runs its refetch interval, so an `enabled` closure that reads another query no longer logs a reactive-tracking warning on every page.

### Fixes

- **Stop live queries when their screen goes away.** Live query hooks now unsubscribe when their reactive owner is cleaned up or their arguments change. Buffered updates are cancelled too, so a removed screen cannot repopulate the query cache after an account change. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #609](https://github.com/leptoskit/leptoskit/pull/609)

#### Retries stop when a host cannot schedule the wait

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #609](https://github.com/leptoskit/leptoskit/pull/609)

A keyed HTTP mutation and `call_with_retry` now surface the last network error when the host has no usable timer, instead of waiting forever on a backoff that can never finish. This keeps a native app's Save button from staying on "Saving…" when its host predates the guest timer plugin or rejects the timer.

A live subscription created before the socket opens is sent once: the handshake records which connection delivered it, so the command that woke the connection no longer sends a duplicate `Sub` frame.

#### Preserve null, empty collections and omitted arguments in generated SDK requests.

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #669](https://github.com/leptoskit/leptoskit/pull/669)

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
