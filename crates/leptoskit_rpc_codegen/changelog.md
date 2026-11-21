# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-21)

### Features

- **Configured RPC endpoints now work for clients alongside the server configuration.** The clients previously called the compile-time defaults: the app's own client posted every call to `/rpc/<op>`, the browser's server-sent events fallback opened `/live/events` and posted frames to `/live/frames`, and the exported spec described every operation at `/rpc/<op>` however the app was configured. The spec export now carries both mounts (`rpc_path` and `live_path`, spec version 4), composes every rpc operation's route from the configured prefix — a service's spec under the service's own prefix — and the foreign and generated SDK clients call those routes and open that socket, so `leptoskit rpc generate` output follows the configuration. The app's own client follows it too: `leptoskit dev` and `leptoskit build` compile the browser and native clients with `LEPTOSKIT_RPC_PATH` and `LEPTOSKIT_RPC_LIVE_PATH` from `leptoskit.toml`, and `ClientConfig::rpc_path`/`live_path` name the mounts by hand for builds that don't go through the CLI. A service's operations keep the default mounts under the service's prefix: `[rpc].path` moves the app's own. Defaults are unchanged (`/rpc`, `/live`, legacy `/api`), as are scope lookup, auth headers, service credentials, and service origin isolation. _Owner:_ Ifiok Jr. · _Introduced in:_ [8bd1891](https://github.com/leptoskit/leptoskit/commit/8bd1891b8d987c9abe19128343e7107d706ecd88)

#### Let an operation refuse with a code and a wait

_Owner:_ Ifiok Jr. · _Introduced in:_ [bd68e95](https://github.com/leptoskit/leptoskit/commit/bd68e9525308319c899d7e4125b570cf2b0d905f)

A query, mutation, or stream can now say why it refused, in a form a screen can match on without comparing sentences. `rpc::Error::rate_limited(wait)` answers `429` with a `Retry-After` header, and `rpc::Error::refused("notes_full", "Your notebook is full")` answers `409` with a code the app chose. Build `rpc::Error::Refused { code, message, status }` for another status.

The caller gets the new `FnError::Refused { status, code, message, retry_after }`, with `FnError::code()` and `FnError::retry_after()` as shortcuts. It arrives the same way over a plain call, the live socket, and a stream, on every platform. A refusal is never retried.

The generated Rust, TypeScript, Dart, Go, and Python clients read it too: each gains a `refused` kind with the code and the wait, and each `LiveError` carries them over the live connection.

Things to know when upgrading:

- `leptoskit_rpc::Error` and `FnError` each gained variants, and `leptoskit_rpc::live::WireError` gained the optional fields `code` and `retry_after`. Code that matches on them without a wildcard arm, or builds a `WireError` by hand, needs updating.
- The generated Rust client's `Error` gained a `Refused` variant, and its `LiveError` two fields.
- A client built or generated before this release reads a refusal as an `other` error whose text is the raw `Refused|{…}` body. Release clients before the server starts refusing this way.

#### Preserve null, empty collections and omitted arguments in generated SDK requests.

_Owner:_ Ifiok Jr. · _Introduced in:_ [51224bf](https://github.com/leptoskit/leptoskit/commit/51224bf974470ffc7ad6154c57e515f4f83556d3)

Query and form encoders now match the server's serde_qs 1.1 protocol: explicit null is a bare key and an empty collection is `key=`. JSON and live encoders keep explicit null, and required nullable arguments in Go and Rust keep their keys. Python optional arguments use the exported `UNSET` for omission and `None` for explicit null. Generated Rust and CLI packages need `serde_qs` 1.1.3; update an existing package's manifest when regenerating its code.

Clients, OpenAPI, AsyncAPI and compatibility checks now follow each argument's `required` flag rather than inferring it from nullability. Specs from `#[api]` functions mark `Option` arguments optional, because the server reads a missing key as `None`. The spec format is now version 4; older specs still load, and their nullable arguments stay optional. A spec written by this version needs an equally new CLI to generate clients.

In regenerated Dart clients, `Option` arguments become optional `Presence<T?>` parameters instead of required `T?` ones. Omit them, or pass `Presence.present(value)`.

Strict diagnostics report a Go or Rust optional nullable argument only when its schema declares a default, and no longer report a Dart query/form limitation that the encoder now handles. Mandatory SDK acceptance decodes generated HTTP requests with the server codec.

### Fixes

- **Record server argument defaults in exported schemas without executing their default functions.** Strict Go, Rust and CLI generation now reports nullable arguments whose omission invokes a server default but whose explicit null cannot be represented by the generated client. Ordinary optional arguments remain accepted. Strict generation also rejects query/form parameters containing free-form JSON, whose empty arrays and objects cannot round-trip through that codec. Use JSON input encoding for those endpoints. Typed lists and maps remain supported. _Owner:_ Ifiok Jr. · _Introduced in:_ [0857012](https://github.com/leptoskit/leptoskit/commit/0857012d89f7a81925de440a09af034c95a2a4c1)

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

### Breaking changes

#### leptoskit_rpc_codegen: the RPC specification and clients

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_rpc_codegen` holds the RPC specification behind leptoskit server functions and generates typed clients from it for TypeScript, Python, Go, Rust, Dart, and the command line.

The server writes the specification; the CLI reads it back and generates clients that speak the RPC's own protocol, plus an OpenAPI 3.1 document. The generators sit behind a default feature, so the server binary can depend on the types alone.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
