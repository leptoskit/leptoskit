# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-25)

### Fixes

#### Smaller web Wasm, lazy listeners & precompressed serving

_Owner:_ Ifiok Jr. · _Introduced in:_ [4001dbb](https://github.com/leptoskit/leptoskit/commit/4001dbb8ed91ecbdcb3caffdc86690b17607ba9c)

- Remove `sha2` from `leptoskit_native`'s `web` feature in favor of a deterministic 64-bit `ScopeHasher`, emit non-default base CSS style deltas directly with `styles::delta_for`, fuse tree pre-scans into one `scan_tree` pass, attach DOM event listeners lazily via `Either` only when handled, and make `NodePath` zero-sized without `devtools`.
- Inject `<link rel="modulepreload">` and `<link rel="preload" as="fetch" type="application/wasm" crossorigin>` hints into built `index.html` files and enable `--strip-producers --low-memory-unused` in `leptoskit_cli`'s `wasm-opt` pipeline.
- Negotiate precompressed `.br`/`.gz` `index.html` in `leptoskit_axum`'s `SpaFallback` and serve precompressed static site/docs assets with `Vary: accept-encoding` in `leptoskit_app`.

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

### Breaking changes

#### leptoskit_axum: the server toolkit

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_axum` is the Axum server toolkit every leptoskit deployment shares: static files with precompressed copies and an `index.html` fallback, `/healthz` and `/readyz` probes with graceful shutdown, code push routes, and the association files that let the native apps use the site's passkeys and sign-in links.

Apps reach it as `leptoskit::server` through the `server` feature, and mount it beside `leptos_axum`; the `analytics` feature adds request measurements. It re-exports `axum`, `tokio`, `tower`, and `tower_http`, so a deployment depends on one crate, and it stays leptos-free so the toolkit composes with any router.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
