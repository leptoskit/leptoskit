# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-13)

### Features

#### File-route navigators, deep links, and system Back

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #679](https://github.com/leptoskit/leptoskit/pull/679)

Layouts can export `navigator()` returning a typed `Stack`, `Tabs`, or `Drawer`. Folder groups like `(tabs)/` stay URL-less; the navigator return value chooses the chrome. Tab and drawer roots are `impl Into<Route>`, not string names. Stacks can present matched routes as a sheet or modal above the previous screen, and optionally offer an edge hit target for Back.

`Navigation::open_url` / `open_url_with` turn incoming URLs into typed routes (custom schemes and `http(s)`). `apply_activations` drains a host deep-link queue into the router. `bind_system_back` and `leptoskit_system_back` let Android system Back pop router history when the host offers it. Generated Android activities call into that path before finishing.

[`examples/file_routes`](https://github.com/leptoskit/leptoskit/tree/main/examples/file_routes) shows Stack + Tabs + Drawer, deep links, and Back. True UIKit interactive-pop and AppKit history swipe still need host presentation; the guide names those gaps.

### Fixes

- **Generated routes also compile in private client modules.** The generated `axum_route_list()` helper is optional in client-rendered apps. An unused helper inside a private routes module no longer fails a warnings-denied build. Its documentation names the Axum method without requiring the app to depend directly on `leptos_axum`, so client-only documentation builds work too. SSR hosts can still call the same helper. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #755](https://github.com/leptoskit/leptoskit/pull/755)
- **Generate SSR paths without redundant conversions.** Generated `axum_route_list` entries now use the `String` that `Pattern::axum_path` returns directly. The redundant conversion made generated SSR routes fail clippy when warnings were denied. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #752](https://github.com/leptoskit/leptoskit/pull/752)

#### Native scenes load models on the first launch

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #734](https://github.com/leptoskit/leptoskit/pull/734)

Android, iOS and macOS install the scene asset reader before starting the guest. Models queued before the first native scene view exists now load from the app bundle on a cold launch.

Generated Axum route lists also return their owned paths directly, keeping generated apps free of redundant conversion warnings.

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

### Breaking changes

#### leptoskit_router_codegen: the routes generator

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_router_codegen` reads a leptoskit `src/routes/` directory and writes its typed `routes.rs`.

The `leptoskit routes` command and the CLI's build pipeline use it; apps never depend on it. It reports every problem with its file and line, and never overwrites a file leptoskit did not write.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
