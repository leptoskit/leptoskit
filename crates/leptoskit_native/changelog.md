# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-13)

### Features

- **Gestures on a view.** A view can follow a finger. `gesture(Gesture::tap())`, `pan`, `long_press`, `pinch`, `rotation`, and `fling` recognize the pointer in one arena shared by the Android widget backend and Linux. `simultaneous`, `exclusive`, and `race` compose recognizers the way a pinch and a rotation share one gesture, or a pan wins over a tap. A drag that wins claims the pointer so the control underneath does not also click. A tap does not, so a button still presses. iOS, macOS, Windows, the web, the terminal, and the generated Swift and Kotlin renderers still deliver presses and clicks only. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #735](https://github.com/leptoskit/leptoskit/pull/735)

#### Camera sessions and Camera surface

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #688](https://github.com/leptoskit/leptoskit/pull/688)

`leptoskit_plugin_camera` is a new runtime plugin, `camera`, for host-owned camera sessions. Create or release a session for the back or front lens, set torch when the hardware supports it (otherwise `Unavailable`), take a bounded still into app cache (URI + width/height/mime/size matching `image_picker` Asset framing), and snapshot status. Show the same session with a new `Camera(session_id)` native widget kind that binds a host-owned id the way `Scene3d` and `Video` bind theirs (requires an accessible label or `aria_hidden`).

Ask for camera access through the `permissions` plugin; this plugin returns `PermissionDenied` when capture lacks a grant. iOS and macOS use `AVCaptureSession` / `AVCaptureVideoPreviewLayer`; Android uses CameraX / `PreviewView`; the browser uses `getUserMedia` into HTML `<video>`. Windows, Linux, and HarmonyOS return `Unavailable`. Video recording remains out of scope for this release.

- **Add `leptoskit_plugin_live_photo` with create/start/stop/status/release and a `LivePhoto` surface bound by session id (`PHLivePhotoView`).** Android and browser remain Unavailable; editing remains later scope. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #692](https://github.com/leptoskit/leptoskit/pull/692)
- **Add `leptoskit_plugin_maps` with create/camera/markers/status/release and a `Map` surface bound by session id (MapKit / Google Maps).** User location and shapes remain later scope. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #690](https://github.com/leptoskit/leptoskit/pull/690)

#### Screen state and queries survive a rebuild

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #736](https://github.com/leptoskit/leptoskit/pull/736) · _Closed issues:_ [#721](https://github.com/leptoskit/leptoskit/issues/721)

leptoskit builds a screen again whenever a signal it read changes. Two things broke under that: a `RwSignal::new(..)` inside a screen ran again with every rebuild, so a chosen filter or a typed field reset as soon as the screen redrew, and a `use_query` in a screen refetched in a loop, because the rebuild its data caused mounted the query again while the data already counted as stale.

- `leptoskit_native`: `remember(|| initial)` returns the same `RwSignal` on every build of the screen, told apart by its place in the source and by order. `remember_keyed(&key, || initial)` names the value by a key instead, for rows that move. A value is released once a whole build no longer reaches it, such as after navigating to another route. `guest::mount` keeps the values for native apps. In the browser, `web::screen(move || ..)` replaces `move || web::render(..)` and does the same. `ScreenState` is the store underneath, for hosts of their own. Both functions are in `leptoskit::prelude`, and the module is `leptoskit::remember`.
- `leptoskit_query`: a query mounted within two seconds of this client's last fetch of it takes that fetch's data or error instead of fetching again, like SWR's deduping interval. The window covers `RefetchOn::Always` and `retry_on_mount`, and not invalidation, `refetch()`, focus, reconnect, or intervals. Data from `set_query_data`, initial data, or hydration doesn't count as a fetch. Set it with `with_mount_dedupe` on `QueryDefaults`, `QueryOptions`, or `QueryObserverOptions`; `Duration::ZERO` gives TanStack's behaviour. `QueryOptions`, `QueryDefaults`, and `ResolvedQueryOptions` gain a `mount_dedupe` field, so code that builds them with struct literals needs it too.

An app with its own keyed store and a long stale time to work around this can switch to `remember` and drop both.

- **Ding-inspired toast pills.** Status toasts arrive as a 44px shape, spring open into a pill, morph in place when a newer message arrives, and shrink before they leave — the same phase model and spring tunings as [Ding](https://github.com/YusufCeng1z/ding). Hold a `Toaster`, call `success` / `error` / `warning` / `info` / `loading`, rebuild from `toaster.state()`, and advance phases with `poll`. `ToastViewport` draws the stack from shared builders, so every backend that already draws `View`, `Text`, `Button`, and `Vector` can show a toast. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #651](https://github.com/leptoskit/leptoskit/pull/651)

#### Video players and Video surface

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #675](https://github.com/leptoskit/leptoskit/pull/675)

`leptoskit_plugin_video` is a new runtime plugin, `video`, for host-owned URL video players. Create or replace a source, play, pause, seek, set muted/loop/rate, snapshot status, and release the player. Show the same player with a new `Video(player_id)` native widget kind that binds a host-owned id the way `Scene3d` binds a scene (requires an accessible label or `aria_hidden`).

iOS and macOS use `AVPlayer` with an `AVPlayerLayer` surface; Android uses Media3 ExoPlayer with a `PlayerView`; the browser uses an HTML `<video>` element. Windows, Linux, and HarmonyOS return `Unavailable`. Tracks, caching, AirPlay, fullscreen controls, and iOS picture-in-picture enter against an attached player remain out of scope for this release.

### Fixes

- **Keep Android builds warning-free: omit the unused stderr fallback on Logcat, simplify pointer decisions, and account for Clippy's Android TLS false positive while retaining const initialization.** _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #734](https://github.com/leptoskit/leptoskit/pull/734)
- **Hydrated web screens refresh their stylesheet when classes change.** Switching a theme or opening a panel could introduce new style scopes while leaving the initial CSS in place, so controls lost their layout and colours. The browser regression hydrates server HTML and changes padding in both directions. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #785](https://github.com/leptoskit/leptoskit/pull/785)

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

- **The web launch screen lifts once the app is drawn.** The HTML splash that `[app.launch_screen]` injects into `index.html` used to stay over the app for good: nothing called its `leptoskitDismissSplash()`. The web renderer now fades it out once the app's first render is painted, as iOS and Android end theirs at the first frame. An app that calls `leptoskit::splash::hold()` keeps it until `leptoskit::splash::dismiss()`, which now lifts it on the web too. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #717](https://github.com/leptoskit/leptoskit/pull/717)

#### Smaller server-rendered pages

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #741](https://github.com/leptoskit/leptoskit/pull/741)

Server-rendered and static pages no longer repeat leptoskit's style reset for every differently styled element. Each element's typed styles start from about sixty declarations, such as `display:flex`, `flex-shrink:0`, and zeroed margins and padding, and every distinct look carried its own copy of all of them, about a kilobyte each. `leptoskit::web::render` now writes those defaults once per stylesheet, and each style scope's sheet holds only what differs from them, often a single declaration. Elements look the same: the defaults keep the specificity and order they had, below utility classes. A list of two dozen rows that differ only in width used to carry 27 copies of the reset and now carries one, and its markup is under 9 KB. Style scopes are also 16 hex digits instead of 64, since every element carries its scope.

The root element of each render now carries `data-lk-root`, which keeps one render's defaults from resetting the elements of a render earlier in the page. `leptoskit_testing_library` leaves it out of the markup it prints, as it does `data-lk-style`.

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

### Breaking changes

#### leptoskit_native: components, Tailwind, and the renderer

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_native` is the native side of the shared tree: builders for every component, from `View`, `Text`, and `Button` to `TextInput`, `PasswordInput`, `Slider`, and `ScrollView`, the Tailwind compiler, the retained element tree, and the diff that turns a commit into a mutation batch.

Every native backend implements `NativeBackend` and consumes the mutations this crate produces; the web build uses its optional `web` feature to render the same builders through leptos. App code normally reaches it through `leptoskit::prelude`, not as a direct dependency, so one screen works on every target.

### Features

- **Theme tokens in leptoskit.toml and deeper CSS var().** `[theme.colors]` and `[theme.lengths]` in `leptoskit.toml` name design tokens that `Theme::install_maps` loads into the process-wide Theme map. Arbitrary color and length utilities accept `var(--name)`, one nested `var()` fallback, and `/opacity` on color vars. The web adapter prepends `Theme::css()` so the browser sees the same `:root` custom properties native resolves through Theme. Tailwind `@theme` / `tailwind.config` files remain unsupported. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #538](https://github.com/leptoskit/leptoskit/pull/538)
- **CSS color variables and a Theme map.** Arbitrary color utilities can use `var(--name)` or `var(--name, fallback)`. The web stylesheet keeps the CSS function. Native paint resolves the name through `Theme::set_color` (or the fallback when the theme has no entry). `Theme::css` emits a `:root` sheet of the same names for the browser. Nested `var()`, non-color variables, and opacity modifiers on `var()` stay unsupported. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #536](https://github.com/leptoskit/leptoskit/pull/536)

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
- **Paint italic text and per-edge borders on Linux and Android.** `border-t`, `border-l-2`, and the other edge border utilities now get the same default stroke color as a uniform `border`, and GTK and Android paint each edge instead of leaving those strokes undrawn. The `italic` utility also draws through GTK labels and Android text views, matching the web and the terminal. AppKit, UIKit, and Win32 still leave per-edge borders undrawn and italic upright. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #534](https://github.com/leptoskit/leptoskit/pull/534)
- **Yoga LayoutUnit rounding for Chrome grid parity.** Fractional grid tracks and percentage lengths resolve through Blink-style 1/64 CSS-pixel layout units, so Yoga frames match Chrome `getBoundingClientRect` for the shared Tailwind class trees. The grid parity probe covers nested grids, dense auto-flow, minmax, flex edges, absolute insets, percentages, and overflow. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #540](https://github.com/leptoskit/leptoskit/pull/540)
