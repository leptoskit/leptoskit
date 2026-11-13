# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-13)

### Features

- **Gestures on a view.** A view can follow a finger. `gesture(Gesture::tap())`, `pan`, `long_press`, `pinch`, `rotation`, and `fling` recognize the pointer in one arena shared by the Android widget backend and Linux. `simultaneous`, `exclusive`, and `race` compose recognizers the way a pinch and a rotation share one gesture, or a pan wins over a tap. A drag that wins claims the pointer so the control underneath does not also click. A tap does not, so a button still presses. iOS, macOS, Windows, the web, the terminal, and the generated Swift and Kotlin renderers still deliver presses and clicks only. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #735](https://github.com/leptoskit/leptoskit/pull/735)

#### Screen state and queries survive a rebuild

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #736](https://github.com/leptoskit/leptoskit/pull/736) · _Closed issues:_ [#721](https://github.com/leptoskit/leptoskit/issues/721)

leptoskit builds a screen again whenever a signal it read changes. Two things broke under that: a `RwSignal::new(..)` inside a screen ran again with every rebuild, so a chosen filter or a typed field reset as soon as the screen redrew, and a `use_query` in a screen refetched in a loop, because the rebuild its data caused mounted the query again while the data already counted as stale.

- `leptoskit_native`: `remember(|| initial)` returns the same `RwSignal` on every build of the screen, told apart by its place in the source and by order. `remember_keyed(&key, || initial)` names the value by a key instead, for rows that move. A value is released once a whole build no longer reaches it, such as after navigating to another route. `guest::mount` keeps the values for native apps. In the browser, `web::screen(move || ..)` replaces `move || web::render(..)` and does the same. `ScreenState` is the store underneath, for hosts of their own. Both functions are in `leptoskit::prelude`, and the module is `leptoskit::remember`.
- `leptoskit_query`: a query mounted within two seconds of this client's last fetch of it takes that fetch's data or error instead of fetching again, like SWR's deduping interval. The window covers `RefetchOn::Always` and `retry_on_mount`, and not invalidation, `refetch()`, focus, reconnect, or intervals. Data from `set_query_data`, initial data, or hydration doesn't count as a fetch. Set it with `with_mount_dedupe` on `QueryDefaults`, `QueryOptions`, or `QueryObserverOptions`; `Duration::ZERO` gives TanStack's behaviour. `QueryOptions`, `QueryDefaults`, and `ResolvedQueryOptions` gain a `mount_dedupe` field, so code that builds them with struct literals needs it too.

An app with its own keyed store and a long stale time to work around this can switch to `remember` and drop both.

#### Service workers and stronger extension messaging

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #584](https://github.com/leptoskit/leptoskit/pull/584)

Web apps can enable the `service-worker` feature to register a worker script, exchange messages, describe caching strategies, open Cache Storage, and manage Web Push subscriptions. Calls return `Unavailable` outside a secure browser context.

Browser extensions gain long-lived `runtime::connect` ports (with `Background::on_connect`), storage `clear` / `keys` / `get_bytes_in_use`, and permissions `get_all` / `on_added` / `on_removed`. The generated background worker keeps ports alive so the wasm side can post and disconnect by id.

- **Ding-inspired toast pills.** Status toasts arrive as a 44px shape, spring open into a pill, morph in place when a newer message arrives, and shrink before they leave — the same phase model and spring tunings as [Ding](https://github.com/YusufCeng1z/ding). Hold a `Toaster`, call `success` / `error` / `warning` / `info` / `loading`, rebuild from `toaster.state()`, and advance phases with `poll`. `ToastViewport` draws the stack from shared builders, so every backend that already draws `View`, `Text`, `Button`, and `Vector` can show a toast. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #651](https://github.com/leptoskit/leptoskit/pull/651)

### Fixes

- **Over the air updates get their own section.** The home page and README now lead with one component tree and real native widgets. Over-the-air updates have their own section, describing code push for installed native apps and which generated hosts install releases. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #552](https://github.com/leptoskit/leptoskit/pull/552)
- **Add `leptoskit_plugin_background_task` for named OS background-task register/unregister/status/complete (`BGTaskScheduler` / `WorkManager`), distinct from `leptoskit_plugin_background` AppService keep-alive.** Guaranteed cadence and suspended Wasm execution remain later scope. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #696](https://github.com/leptoskit/leptoskit/pull/696)
- **Add `leptoskit_plugin_calendar` for calendar permission, listing calendars (max 64), and creating or reading bounded events on iOS and Android.** Desktop and browser hosts report unavailable. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #687](https://github.com/leptoskit/leptoskit/pull/687)
- **Add `leptoskit_plugin_contacts` for contacts permission, system pick-one, and a bounded query (max 32) on iOS and Android.** Desktop and browser hosts report unavailable. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #686](https://github.com/leptoskit/leptoskit/pull/686)
- **Add `leptoskit_plugin_device_motion` for fused attitude, user acceleration, and rotation-rate readings on iOS, Android, and the browser.** Desktop hosts report unavailable. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #684](https://github.com/leptoskit/leptoskit/pull/684)
- **Add `leptoskit_plugin_live_photo` with create/start/stop/status/release and a `LivePhoto` surface bound by session id (`PHLivePhotoView`).** Android and browser remain Unavailable; editing remains later scope. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #692](https://github.com/leptoskit/leptoskit/pull/692)
- **Add `leptoskit_plugin_maps` with create/camera/markers/status/release and a `Map` surface bound by session id (MapKit / Google Maps).** User location and shapes remain later scope. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #690](https://github.com/leptoskit/leptoskit/pull/690)
- **Add `leptoskit_plugin_media_library` for bounded photo-library permission, asset listing, and saving an app-owned file into the system library.** Change observation remains later scope. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #689](https://github.com/leptoskit/leptoskit/pull/689)
- **Add `leptoskit_plugin_print` for HTML and PDF URI print dialogs on iOS, Android, and macOS.** Completion means the dialog finished or the platform accepted a job, never that paper left a printer. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #683](https://github.com/leptoskit/leptoskit/pull/683)
- **Add `leptoskit_plugin_video_thumbnails` to extract one JPEG frame from an app-owned video URI at a time offset (`AVAssetImageGenerator` / `MediaMetadataRetriever`).** Multi-frame extraction remains later scope. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #694](https://github.com/leptoskit/leptoskit/pull/694)

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

### Breaking changes

#### leptoskit: the one dependency

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit` is the crate an app depends on. It re-exports the components, Tailwind styles, layout engine, renderer, and Wasm runtime, with one feature per optional part, so an app names `leptoskit` alone and never keeps the versions of its parts in step by hand.

Pick how the interface runs: `web` with `csr`, `hydrate`, `ssr`, or `islands` renders through leptos, and a backend feature (`macos`, `ios`, `android`, `linux`, `windows`, or `tui`) commits the same tree as real widgets or as terminal cells. `server` adds the Axum toolkit, and `analytics` beside it keeps request and browser measurements. Everything else is opt-in: `rpc`, `query`, `form`, `aria`, `table`, `virtual`, `l10n`, `rive`, `db` with `db-postgres` or `db-sqlite`, `auth` (passwords, passkeys, and providers through their own `auth-*` features), `idp`, `extension`, `codepush`, and `devtools`. `leptoskit::secrets` reads values that Monosecret, the default secret manager, or a trusted host supplies.

### Features

- **Theme tokens in leptoskit.toml and deeper CSS var().** `[theme.colors]` and `[theme.lengths]` in `leptoskit.toml` name design tokens that `Theme::install_maps` loads into the process-wide Theme map. Arbitrary color and length utilities accept `var(--name)`, one nested `var()` fallback, and `/opacity` on color vars. The web adapter prepends `Theme::css()` so the browser sees the same `:root` custom properties native resolves through Theme. Tailwind `@theme` / `tailwind.config` files remain unsupported. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #538](https://github.com/leptoskit/leptoskit/pull/538)

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
