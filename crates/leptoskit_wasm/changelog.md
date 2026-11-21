# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-21)

### Features

#### Android Back goes back a route

_Owner:_ Ifiok Jr. · _Introduced in:_ [a93214e](https://github.com/leptoskit/leptoskit/commit/a93214edc8e5f6c7999c628285093f15cf188639)

The Back gesture and Back button in an Android app now go back one route through the file router, and leave the app only from the first route. Before, Back always left the app: Android 16 delivers Back to an app that targets it only through an `OnBackInvokedCallback`, which the generated activity never registered, and on every Android version the host's question to the router went nowhere.

- `leptoskit_wasm`: `WasmHost::system_back` offers Back to the guest's optional `leptoskit_system_back` export, then flushes and commits what changed. A guest without the export answers `EventOutcome::Unhandled`, and the platform leaves.
- `leptoskit_bridge_ffi`: `lk_guest_system_back` offers Back on the guest thread, and `lk_guest_take_system_back` takes the answers in order. `LeptosKitApplicationView.offerSystemBack` now takes a callback, `(Boolean) -> Unit`, that runs on the main thread with `true` when the router went back.
- `leptoskit_cli`: the generated `MainActivity.kt` registers an `OnBackInvokedCallback` on Android 13 and later and keeps `onBackPressed` for earlier versions; the generated manifest sets `android:enableOnBackInvokedCallback="true"`.

Apps scaffolded before this keep leaving on Back until they copy the new `MainActivity.kt` Back code and the manifest attribute from a new project; [the Android page](https://leptoskit.dev/docs/platforms/android.html#the-back-gesture) lists them.

#### Modal and Popover show content above the screen

_Owner:_ Ifiok Jr. · _Introduced in:_ [08a70cb](https://github.com/leptoskit/leptoskit/commit/08a70cb0dbd8a153b2b8eb285099920fb6d2252c)

`Modal(open)` blocks the screen until it is answered, and `Popover(anchor, open)` hangs from the element with that `id`. Both stay in the tree while closed. `.on_dismiss(...)` hears Escape and a press outside as a request to close, and the overlay closes when the next build says so. `.initial_focus(id)` names what takes focus on opening, and `.placement(Placement::BelowEnd)` picks a popover's side. In `view!` they are `<Modal open=…>` and `<Popover anchor=… open=…>`.

On the web a `Modal` is a `<dialog>` opened with `showModal()` and a `Popover` uses the `popover` attribute, so the browser supplies the top layer, the inert page, the focus trap and focus restore. The terminal draws overlays last, keeps Tab inside an open modal, and gives focus back on close.

macOS, iOS, Android, Linux and Windows draw an overlay as a layer in the right place for now. Nothing asks to dismiss it there, focus is not moved or kept inside, and a parent that clips or scrolls cuts it off.

For backend authors: `NativeKind` gains `Modal` and `Popover`, `NativeEvent` gains `Dismiss`, and `NativeProps` gains `overlay`. A backend that returns `true` from the new `NativeBackend::presents_overlays` is told the real kinds and given open overlays' frames in viewport coordinates. Every other backend is given a `View` with a frame relative to its parent. The guest event ABI gains kind 9 (dismiss), and the bridge event codes gain 10.

### Fixes

#### Regions committed together keep their own widgets

_Owner:_ Ifiok Jr. · _Introduced in:_ [7e61e9b](https://github.com/leptoskit/leptoskit/commit/7e61e9bceefea4db7c8e3a70fdd31deb21597898)

`NativeSurface::try_patch` applies patches in tree order, and it answered in that order too, though its contract (and the wasm host, which pairs each answer with the patch it sent) is the order given. A commit that changed two regions whose wire ids ran against the tree, such as a label above a list that gained a row, paired one region's wire ids with the other's widgets. A debug host stopped with "surface and wire tree diverged"; a release host could send a widget's events to another node. On a Solana Seeker, buying anything in Reapers stopped the app.

`try_patch` now answers in the order given. The wasm host's incremental tests check, after every commit, that each widget pairs with the wire node it was built from.

#### Hear the ⌘ key in key handlers on the web

_Owner:_ Ifiok Jr. · _Introduced in:_ [25a25a8](https://github.com/leptoskit/leptoskit/commit/25a25a8f96aa4774c9f855eff3423c0061308c6f)

`on_key` on the web now hears keys pressed with ⌘ held. Until now it skipped every one of them, so a web app could not save on ⌘+Enter or open search on ⌘+K.

`KeyPress` gains a `meta` field for the command key, with `KeyPress::meta()` to build one in a test. `KeyPress::is_primary()` is true when the platform's shortcut modifier is held: ⌘ on a Mac, an iPhone or an iPad, and Ctrl everywhere else. In a browser it follows the device the browser runs on.

```rust
View().on_key(move |key| {
	if key.is_primary() && key.key == "Enter" {
		save();
		EventOutcome::Handled
	} else {
		EventOutcome::Unhandled
	}
})
```

A ⌘ key that no handler takes is still the browser's, so ⌘+R reloads and ⌘+L goes to the address bar as before.

Two things to check in existing code. `KeyPress` has a new public field, so a `KeyPress { .. }` literal needs `meta`. And `KeyPress::is` no longer matches while ⌘ is held, which only matters on the web, where those keys never arrived before. `Tabs`, `RadioGroup` and `Menu` leave ⌘ with an arrow key alone, since that is back and forward in a browser on a Mac.

A terminal is never told about ⌘, so `meta` is always false there. The native widget backends do not deliver key presses to `on_key` yet.

#### Native props read the same whatever serde_json's features

_Owner:_ Ifiok Jr. · _Introduced in:_ [736759c](https://github.com/leptoskit/leptoskit/commit/736759cc6082c21aa05025aab6823b785a0c3a21)

An Android app that also used Solana's clients drew nothing: its first vector failed in the Wasm host with `invalid type: map, expected f32`. Those clients turn on serde_json's `arbitrary_precision` for the whole build, and under it a buffered number with a fraction is a map. The host read a node's props through `#[serde(flatten)]`, which buffers every value, and so did each animation `Keyframe`.

The host now reads a node with `WireNode::from_json`: the props and the keys beside them in two passes, with nothing buffered, and `Keyframe` lists its look's `opacity` and `transform` itself. Its JSON is unchanged.

The guest and the host now share `leptoskit_native::wire`: one table of kind codes and one `WireNode`. That fixes two more ways the sides disagreed:

- The host's own kind table stopped at `Camera`, so a `Map` or a `LivePhoto` showed as a plain view.
- An image's `source` was taken by the DevTools location, which used the same key, so native images had no source. A `DevTools` guest now sends its location as `source_location`.

`test:wire` runs the guest-to-host tests under `arbitrary_precision` too.

#### Handed-back calls reach the Rust plugin that serves them

_Owner:_ Ifiok Jr. · _Introduced in:_ [5445246](https://github.com/leptoskit/leptoskit/commit/54452466966f79486f975a84eb139e11a5b0e8ae)

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
