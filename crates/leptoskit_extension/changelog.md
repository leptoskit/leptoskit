# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

### Features

- Long-lived `runtime::connect` ports with `Background::on_connect`, storage `clear` / `keys` / `get_bytes_in_use`, and permissions `get_all` / `on_added` / `on_removed`.

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-28)

### Features

#### Add extension browser detection and full side panel API

_Owner:_ Ifiok Jr. · _Introduced in:_ [c4aa7df](https://github.com/leptoskit/leptoskit/commit/c4aa7df45dd07fbead27b9b05bba1d67ecfacd7f)

- `leptoskit_extension::browser` adds runtime browser detection (`ExtensionBrowser` covering Chrome, Edge, Brave, Opera, Arc, Vivaldi, Firefox, and Safari; `BrowserFamily`; `BrowserInfo`; `BrowserSwitch`; and `with_browser` test scoping).
- `leptoskit_extension::side_panel` adds full Chromium `chrome.sidePanel` and Firefox `browser.sidebarAction` support: `open_target`, `open_for_tab`, `close`, `toggle`, `is_open`, `set_options` / `get_options`, `set_behavior` / `get_behavior`, `get_layout`, `set_title` / `get_title`, and `capabilities`.
- Adds `leptoskit_extension::windows`, `leptoskit_extension::offscreen`, `leptoskit_extension::omnibox`, and `leptoskit_extension::native_messaging`.
- Adds `[extension].optional_permissions` and `[extension.side_panel]` `title` / `open_at_install` options to `leptoskit_config` and generated MV3 manifests.

#### Service workers and stronger extension messaging

_Owner:_ Ifiok Jr. · _Introduced in:_ [363ebad](https://github.com/leptoskit/leptoskit/commit/363ebadd2effe51d9b954c31ab0b6bcbfdc46ab8)

Web apps can enable the `service-worker` feature to register a worker script, exchange messages, describe caching strategies, open Cache Storage, and manage Web Push subscriptions. Calls return `Unavailable` outside a secure browser context.

Browser extensions gain long-lived `runtime::connect` ports (with `Background::on_connect`), storage `clear` / `keys` / `get_bytes_in_use`, and permissions `get_all` / `on_added` / `on_removed`. The generated background worker keeps ports alive so the wasm side can post and disconnect by id.

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

### Breaking changes

#### leptoskit_extension: browser extensions

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_extension` is the runtime and typed WebExtensions bindings behind leptoskit browser extensions for Chromium, Firefox, and Safari.

Apps enable the `extension` feature, name entry points with `entry!`, and build with `--platform extension`. APIs return `Unavailable` instead of throwing, and listeners are removed when their handle drops.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
