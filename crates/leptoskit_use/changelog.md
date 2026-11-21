# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-21)

### Features

- **Dual-pane and tabletop layouts for foldables.** Fold posture now carries each hinge's `separating` flag (Android `FoldingFeature.isSeparating`), so a fully open continuous foldable stays one pane. `Posture::is_dual_pane`, `layout_mode`, and `use_posture` expose dual-pane, tabletop, book, and single layouts. `DevicePosture::watch` remains how you react to fold-driven configuration changes while the Android activity stays alive. The guide and Android platform page show concrete dual-pane and tabletop examples. _Owner:_ Ifiok Jr. · _Introduced in:_ [930b24e](https://github.com/leptoskit/leptoskit/commit/930b24e989f9e15a809201ea1baf21913c8cd596)

#### Add `use_clock`, a clock that advances by itself

_Owner:_ Ifiok Jr. · _Introduced in:_ [95d9169](https://github.com/leptoskit/leptoskit/commit/95d9169b4306a60430916d2d39ab8d4f4afe6785)

`leptoskit_use::use_clock(interval)` returns a `Clock` signal holding the time since it was called, updated about once per `interval` on the platform's own timer: `setTimeout` in the browser, the timer plugin in a native app, and tokio or a timer thread elsewhere. Pass it to `use_timeout_fn`, `use_interval_fn`, `signal_debounced` and the other hooks that take a clock, or read it in a `Text` closure for a countdown. Until now a host had to advance a clock signal itself.

The time is measured from the wall clock and never goes back, so a tab that slept catches up in one step. The timer stops when the owner the clock was made under is cleaned up.

It is behind a new `clock` feature, because it is the one hook in `leptoskit_use` that needs a task executor and depends on `leptoskit_query`. In `leptoskit`, the `clock` feature turns on `use` and `query` too.

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

### Breaking changes

#### leptoskit_use: cross-platform hooks

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_use` provides cross-platform reactive primitives inspired by leptos-use: state, derived values, watchers, timers, storage, and viewport breakpoints without a DOM or executor.

The hooks read leptos signals and accept host services explicitly, so the same code runs in a browser and in a native guest. The `local-storage` feature adds persistent JSON storage in native app directories and the browser's local storage, and the `web` feature adds browser-only hooks such as media queries and event listeners. It is the utility layer beneath app screens, not a UI framework.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
