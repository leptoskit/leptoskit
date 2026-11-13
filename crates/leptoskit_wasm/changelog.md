# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-13)

### Features

#### Android Back goes back a route

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #726](https://github.com/leptoskit/leptoskit/pull/726)

The Back gesture and Back button in an Android app now go back one route through the file router, and leave the app only from the first route. Before, Back always left the app: Android 16 delivers Back to an app that targets it only through an `OnBackInvokedCallback`, which the generated activity never registered, and on every Android version the host's question to the router went nowhere.

- `leptoskit_wasm`: `WasmHost::system_back` offers Back to the guest's optional `leptoskit_system_back` export, then flushes and commits what changed. A guest without the export answers `EventOutcome::Unhandled`, and the platform leaves.
- `leptoskit_bridge_ffi`: `lk_guest_system_back` offers Back on the guest thread, and `lk_guest_take_system_back` takes the answers in order. `LeptosKitApplicationView.offerSystemBack` now takes a callback, `(Boolean) -> Unit`, that runs on the main thread with `true` when the router went back.
- `leptoskit_cli`: the generated `MainActivity.kt` registers an `OnBackInvokedCallback` on Android 13 and later and keeps `onBackPressed` for earlier versions; the generated manifest sets `android:enableOnBackInvokedCallback="true"`.

Apps scaffolded before this keep leaving on Back until they copy the new `MainActivity.kt` Back code and the manifest attribute from a new project; [the Android page](https://leptoskit.dev/docs/platforms/android.html#the-back-gesture) lists them.

### Fixes

#### Regions committed together keep their own widgets

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #733](https://github.com/leptoskit/leptoskit/pull/733) · _Related issues:_ [#2](https://github.com/leptoskit/leptoskit/issues/2)

`NativeSurface::try_patch` applies patches in tree order, and it answered in that order too, though its contract (and the wasm host, which pairs each answer with the patch it sent) is the order given. A commit that changed two regions whose wire ids ran against the tree, such as a label above a list that gained a row, paired one region's wire ids with the other's widgets. A debug host stopped with "surface and wire tree diverged"; a release host could send a widget's events to another node. On a Solana Seeker, buying anything in Reapers stopped the app.

`try_patch` now answers in the order given. The wasm host's incremental tests check, after every commit, that each widget pairs with the wire node it was built from.

#### Native props read the same whatever serde_json's features

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #730](https://github.com/leptoskit/leptoskit/pull/730)

An Android app that also used Solana's clients drew nothing: its first vector failed in the Wasm host with `invalid type: map, expected f32`. Those clients turn on serde_json's `arbitrary_precision` for the whole build, and under it a buffered number with a fraction is a map. The host read a node's props through `#[serde(flatten)]`, which buffers every value, and so did each animation `Keyframe`.

The host now reads a node with `WireNode::from_json`: the props and the keys beside them in two passes, with nothing buffered, and `Keyframe` lists its look's `opacity` and `transform` itself. Its JSON is unchanged.

The guest and the host now share `leptoskit_native::wire`: one table of kind codes and one `WireNode`. That fixes two more ways the sides disagreed:

- The host's own kind table stopped at `Camera`, so a `Map` or a `LivePhoto` showed as a plain view.
- An image's `source` was taken by the DevTools location, which used the same key, so native images had no source. A `DevTools` guest now sends its location as `source_location`.

`test:wire` runs the guest-to-host tests under `arbitrary_precision` too.

#### Handed-back calls reach the Rust plugin that serves them

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #732](https://github.com/leptoskit/leptoskit/pull/732)

An `AppService` such as Solana Kit's wallet service drains the bridge itself and hands back every call it does not own. Those calls skipped the interpreter's timer and went to the platform, which has no timer plugin: on Android every guest `sleep` failed as `Unavailable` in any app with such a service, so nothing that waited on a timer ever ran. A handed-back call now passes the interpreter first, as a queued one always did.

`WasmHost::serve` and `WasmHost::served_plugins` name what the interpreter answers itself, and `App::rust_plugins` lists every plugin answered in Rust: the timer, the frame clock, inline, worker and async plugins. Tests on the guest thread's own step find every registered plugin by id and fail when a call for a Rust plugin reaches the platform's router, so a Rust plugin added later is held to the same rule.

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

### Breaking changes

#### leptoskit_wasm: the on-device Wasm runtime

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_wasm` runs the app's interface bundle on the device in `wasmi`, an interpreter that is allowed on iOS where compiling code at runtime is not, and drives a native backend through a versioned ABI.

This is what makes the shipped app a shell: the interface arrives as Wasm, renders through the same renderer, and a code push release can replace it without a store update. Every entry point is fuel-metered and rendering work is capped, so a bad bundle cannot hang the device. The generated iOS, Android, macOS, Windows, Linux, and terminal apps embed it.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
