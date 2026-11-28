# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-28)

### Features

#### File-route navigators, deep links, and system Back

_Owner:_ Ifiok Jr. · _Introduced in:_ [bbc358b](https://github.com/leptoskit/leptoskit/commit/bbc358babf47a0c482e74527c7011408f620dfe8)

Layouts can export `navigator()` returning a typed `Stack`, `Tabs`, or `Drawer`. Folder groups like `(tabs)/` stay URL-less; the navigator return value chooses the chrome. Tab and drawer roots are `impl Into<Route>`, not string names. Stacks can present matched routes as a sheet or modal above the previous screen, and optionally offer an edge hit target for Back.

`Navigation::open_url` / `open_url_with` turn incoming URLs into typed routes (custom schemes and `http(s)`). `apply_activations` drains a host deep-link queue into the router. `bind_system_back` and `leptoskit_system_back` let Android system Back pop router history when the host offers it. Generated Android activities call into that path before finishing.

[`examples/file_routes`](https://github.com/leptoskit/leptoskit/tree/main/examples/file_routes) shows Stack + Tabs + Drawer, deep links, and Back. True UIKit interactive-pop and AppKit history swipe still need host presentation; the guide names those gaps.

- **Your own tab bar and sheet surface.** `Tabs::bar` draws the tab bar with your own components instead of the built-in row of titles. It receives a `TabBar` with each tab and whether it is selected, and `TabBar::link` wraps whatever you draw in a link that switches tabs the same way the built-in bar does. `Stack::surface` draws the layer a modal or sheet route sits on, so it can use your app's paper and corners instead of plain white. Both keep the built-in test ids. `Tabs`, `Navigator`, and `IntoNavigator` now require `'static` route types, as `Router` already did. _Owner:_ Ifiok Jr. · _Introduced in:_ [f2e45e3](https://github.com/leptoskit/leptoskit/commit/f2e45e355aedd42747a69086a8925e37380fdb9c)
- **Set the browser tab's title.** `leptoskit_router::set_title("Plans · Admin")` names what the person is looking at. On the web it sets the document's title, so the tab and the history entry follow the screen as the person moves around without a page load. A server-rendered page still writes its first title in its own `<title>`. Native windows keep the title they were opened with for now, and on a phone it does nothing. _Owner:_ Ifiok Jr. · _Introduced in:_ [edcb32f](https://github.com/leptoskit/leptoskit/commit/edcb32f5b4424abc33d17a784c8afb896ea06a14)

### Fixes

- **Keep the file router's SSR module and Axum dependencies on native targets, so enabling all features in browser and Wasm guest builds does not pull in native networking.** _Owner:_ Ifiok Jr. · _Introduced in:_ [7237130](https://github.com/leptoskit/leptoskit/commit/723713098b125431f6e762cbf4b6f3ebe5506a23)
- **SSR apps now register every file route with `leptos_axum`, and the server render reads the request URL instead of always opening on Home.** _Owner:_ Ifiok Jr. · _Introduced in:_ [363c36e](https://github.com/leptoskit/leptoskit/commit/363c36ed87c461f093d2a4c53a090ef8606f9228)

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

### Breaking changes

#### leptoskit_router: typed file routes

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_router` is the runtime behind leptoskit file routes: each file in `src/routes/` becomes a variant of one generated `Route` enum, so URLs are parsed into typed values and links are written from them, and the two cannot drift apart.

`leptoskit dev`, `leptoskit build`, and `leptoskit routes` generate the enum; the app then holds a `Router` to read the current route, `navigate`, `replace`, go `back`, and build links that render as real `<a href>` links on the web. Each page renders inside the layouts of the folders above it. Natively the history lives in memory; the `web` feature keeps the browser's address bar and back button in step. Routes match from most to least specific, and parameter structs use `#[derive(Route)]`.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
