# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-13)

Grouped release for `release`.

### Features

#### Interfaces from agents

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #550](https://github.com/leptoskit/leptoskit/pull/550)

`leptoskit_agent_ui`, also `leptoskit`'s `agent-ui` feature, renders interfaces an AI agent writes as text while your app runs. The agent answers in Markdown with components in it — `<Stat>`, `<Chart>`, `<Slider>`, `<Button>`, and more — declares variables with `<Var>` and formulas with `<Let>`, and writes `{formulas}` in text that follow the user's input without a new message. `<View>`, `<Text>`, and `<Pressable>` let it build layouts the other components don't cover. `AgentUi::view` follows a streaming answer and shows a component that is still open. Button presses reach `on_action` with the user's state, and the app decides what they do.

Documents style with Tailwind classes and colour roles such as `bg-surface` and `text-muted`, never colours. Each role reads your `[theme.colors]` entry of the same name, and its `-dark` twin in dark appearance, so documents and the built-in components match your app in light and dark without the agent knowing either; `agent_ui::Theme` renames roles and adds your own.

Sliders, switches, checkboxes, radios, and spinners are the platform's own controls, updated in place so a slider keeps its drag. `Widgets::WithoutControls` draws them from buttons for macOS, iOS, and Android shells generated before 0.3, and `Widgets::Basic` also leaves out vector paths for HarmonyOS.

Documents are data, never code: formulas run on a fixed budget, classes that would lay content over the app or scroll are refused, images load only from allowed addresses, and `Limits` bound every document. `AgentUi::reference` writes the language reference for the agent from the components and colour roles registered right now, and `AgentUi::check` lists a document's problems by line and column.

- **Answer analytics batches once they're queued.** By default the analytics service answers a batch once its events are stored, so every answer means they're kept. An app that prefers speed can set `LEPTOSKIT_ANALYTICS_ACK=queued` on the service: it answers as soon as the events are queued, and a crash loses at most what was queued. The queue's limits still apply, and a full queue still answers `503`. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #786](https://github.com/leptoskit/leptoskit/pull/786)

#### Route metrics are now `leptoskit_apm`

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #620](https://github.com/leptoskit/leptoskit/pull/620)

The in-process route metrics that `leptoskit_analytics` held (minute aggregates, request correlation, the browser module at `/_leptoskit/analytics.mjs`, and OTLP export) are now the `leptoskit_apm` crate, and `leptoskit`'s `apm` feature. The name `leptoskit_analytics` is moving to the analytics service, a product-analytics service plugin.

Update your `leptoskit` dependency's features from `analytics` to `apm`, and `leptoskit::analytics::…` to `leptoskit::apm::…`. Both old names still work for this release. The endpoints, `[infra.plugins.analytics]`, and `lk infra analytics` are unchanged.

#### Record analytics events from Rust

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #777](https://github.com/leptoskit/leptoskit/pull/777)

`leptoskit_analytics::client` records events, people, and groups, the same way on every platform, and depends on nothing, so it adds no JSON, UUID, time, or compression crate to an app. It batches what's recorded, retries with backoff when the service can't be reached, sends smaller batches when one is too large, and drops the oldest events past its limits, reporting how many it dropped.

On the app's server, the `server-client` feature adds `client::server::Analytics`. It reads the service's address and token from what `leptoskit dev` and `leptoskit serve` set, records events without waiting, and sends them in the background:

```rust
let analytics = Analytics::from_env().expect("the app runs the analytics service");
analytics.capture(&user_id, "invoice_paid", props! { "amount_cents" => 1999 });
```

The browser and native transports follow in a later release.

- **Store route timings in the analytics service.** The analytics service accepts application-performance metrics at `/_leptoskit/services/analytics/otlp/v1/metrics`, in the OTLP/HTTP JSON that `leptoskit_apm`'s exporter sends, so how long each route took is kept beside the product analytics. Each minute of each route is stored once, even when the exporter retries. Only the app may send them, signed with the service's token. Metrics the service doesn't store, such as cumulative histograms or gauges, are reported back as rejected, as OTLP expects. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #787](https://github.com/leptoskit/leptoskit/pull/787)
- **Rate-limit analytics events per sender.** The analytics service limits how fast each `distinct_id` may send events: 100 a second, with bursts of 2,000, far above what a person's device sends. One stuck client or bot can no longer fill the queue for everyone else. Events over the limit are dropped, counted in the answer's `rate_limited`, and recorded as an ingestion warning. Events the app's server sends aren't limited. `IngestConfig::per_distinct_id` changes the limit, and `None` turns it off. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #781](https://github.com/leptoskit/leptoskit/pull/781)

#### An analytics service that stores events

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #776](https://github.com/leptoskit/leptoskit/pull/776)

`leptoskit_analytics` is now a service plugin: add `[services.analytics]` with `crate = "leptoskit_analytics"` to `leptoskit.toml`, and the app gets a process with its own database that records events.

Clients send events to `/_leptoskit/services/analytics/v1/batch` in a compact binary batch format, where the device's details are sent once per batch rather than with every event, or as JSON through the `capture` operation. The service answers once the events are stored. A retried batch is stored once, every refused event is reported with a reason, and a full queue answers `503` with `Retry-After` rather than dropping events.

The clients that send these batches, and the pages that read them, follow in later releases.

#### Let people choose light or dark

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #760](https://github.com/leptoskit/leptoskit/pull/760)

`dark:` utilities followed only the device's appearance, so an app could not offer "Light", "Dark", and "Match device". `leptoskit::appearance::set(Appearance::Light)` (or `Dark`, or `System` to follow the device again) now switches every `dark:` utility at once, without a reload, and `appearance::get()` reads the choice reactively.

On the web, the choice marks `<html>` with `data-lk-appearance` and sets its `color-scheme`; the generated stylesheet applies `dark:` rules under `prefers-color-scheme: dark` unless the root says `light`, and always when it says `dark`. In a native app, the root of the next render carries the choice (`NativeProps::appearance`), and `NativeSurface` resolves `dark:` against it and passes the result to the backend's `set_dark_mode`, so native controls match. Hosts can also call `NativeSurface::set_appearance`, and read `NativeSurface::appearance` and `NativeSurface::is_dark`. Older shells ignore the choice and keep following the device.

Persisting the choice is the app's job: store `Appearance::as_str()` and call `appearance::set(stored.parse()?)` before the first render.

- **Build several frontends, servers and native destinations from one application's root `leptoskit.toml` and Cargo workspace.** Named targets select a frontend and explicit server package, isolate public inputs, sidecars, outputs and build records, and preserve application identity. Use `--app-target NAME` and `leptoskit targets`; destinations not yet implemented can carry an actionable unavailability reason. Native hosts can live under `platforms/`. macOS menu-bar mode keeps a status item alive and opens its native UI only when requested. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #806](https://github.com/leptoskit/leptoskit/pull/806)
- **Require independent backend release approvals.** Managed backend deployments can require independent signatures over the complete image descriptor and a current version-two deployment policy binding its exact digest and image identity. Reuse, rollback and recovery revalidate operator-owned trust and policy replay state without trusting stored verification flags. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #585](https://github.com/leptoskit/leptoskit/pull/585)

#### Background execution for apps

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #676](https://github.com/leptoskit/leptoskit/pull/676)

`leptoskit_plugin_background` keeps a LeptosKit app working in the background. On Android it runs a foreground `Service` with a persistent, low-importance notification (its own channel) and wakes the app's Rust `AppService` on a configurable interval through the host's existing service-wake path; a WorkManager periodic fallback restarts the service every 15 minutes, and an opt-in `BOOT_COMPLETED` receiver restarts it after a reboot. The service's `foregroundServiceType` is declared in the generated manifest from the plugin's metadata, with the matching `FOREGROUND_SERVICE_*` permissions, and a guest API asks the user to exempt the app from battery optimisation. On iOS the plugin registers `BGTaskScheduler` app-refresh and processing tasks that wake the `AppService`, with `UIBackgroundModes` and `BGTaskSchedulerPermittedIdentifiers` merged into the generated `Info.plist`. macOS, desktop, and the browser report `Unavailable`.

The guest API is `guest::Background::{start, stop, status, request_ignore_battery_optimizations}`, behind the `guest` feature. `status` reports whether background execution is running, the last wake, and the battery-optimisation state. The `AppService` now receives a wake reason through a new `set_wake_reason` hook, so a service can tell a platform background tick from an ordinary app call.

#### Read every sample as markup or builder calls

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #595](https://github.com/leptoskit/leptoskit/pull/595)

Code samples on leptoskit.dev that build a component tree now have a Markup and Builder switch. Markup shows the tree as `view!` markup, as before; Builder shows the same tree as builder calls, like `View().class("p-4").child(Text("Hello"))`. One switch changes every sample on the page, and the choice is remembered on the landing page and throughout the docs. It applies before the page first draws, so a sample never flickers from one style to the other, and pages read as markup without JavaScript. Copy copies the style that is showing.

`leptoskit_docs` derives the builder form when it builds a book: each Rust sample with leptoskit `view!` markup gets both forms, laid out the way rustfmt lays out code, and leptos's own `view!` is left alone. Markup that has no builder form fails the build at its line and says why; write `markup-only` after the language, as in `` ```rust markup-only ``, to show a sample as written.

#### Dart clients keep requiredness and nullability separate

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #602](https://github.com/leptoskit/leptoskit/pull/602)

Generated Dart models and operation arguments no longer treat a nullable type as an optional one. A required nullable field or argument stays required and always writes its key, including an explicit `null`. Optional nullable values use the new three-state `Presence<T?>` wrapper: `const Presence<T?>.absent()` omits the key, `Presence<T?>.present(null)` sends `null`, and `Presence<T?>.present(value)` sends the value. Optional non-nullable values keep their `T?` shape, so `false`, `0`, empty strings, empty lists, and empty maps are still sent instead of dropped.

JSON and live payloads share the same presence-aware argument map, so HTTP calls, watched queries, live mutations, and channel join arguments round-trip missing, `null`, and real values. Query-string and form encodings cannot represent explicit `null`; `--strict` now reports that remaining encoding limitation instead of claiming the nullable field was lossless.

Regenerating changes the Dart signatures of optional nullable fields and arguments from `T?` to `Presence<T?>`, and required nullable constructor arguments become `required`. Update callers to pass `Presence.present(...)` or `Presence.absent()`, and read `.isPresent`/`.value` when consuming an optional nullable field.

The mandatory SDK acceptance gate now analyzes and runs generated Dart presence fixtures alongside every existing SDK journey. Release binary builds explicitly retain locked dependency resolution.

#### Hardware-bound signing keys gated by biometrics

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #682](https://github.com/leptoskit/leptoskit/pull/682)

`leptoskit_plugin_device_keys` is a new runtime plugin, `device-keys`, for hardware-bound P-256 signing keys that can never be copied off the device. `generate` creates a non-exportable key under an alias with a `Biometry` or `BiometryOrPasscode` policy; `sign` shows the operating system's biometric or passcode prompt and returns an ECDSA P-256 / SHA-256 signature, so a server can treat it as fresh proof that the device owner authenticated on this device just now. `public_key`, `delete`, and `available` round out the API.

Apple generates the key with `SecKeyCreateRandomKey` in the Secure Enclave (`kSecAttrTokenIDSecureEnclave`, `kSecAttrKeyTypeECSECPrimeRandom` 256-bit) with `SecAccessControlCreateWithFlags` using `.privateKeyUsage` plus `.biometryCurrentSet` or `.userPresence`, and signs with `SecKeyCreateSignature(.ecdsaSignatureMessageX962SHA256)` through an `LAContext`. Android generates in `AndroidKeyStore` with `setUserAuthenticationRequired(true)` and `setUserAuthenticationParameters(0, AUTH_BIOMETRIC_STRONG [| AUTH_DEVICE_CREDENTIAL])` for per-use authentication, `setInvalidatedByBiometricEnrollment(true)` for the `Biometry` policy, and `setIsStrongBoxBacked(true)` with a TEE fallback, then signs through a `BiometricPrompt` `CryptoObject`. A platform without the hardware, including the Secure Enclave-less simulator and browsers, reports `Unavailable`; there is no silent software-key fallback.

`sign` returns `KeyError::{NotFound, Cancelled, LockedOut, NotEnrolled, Invalidated, Unavailable, Failed}`, `generate` additionally `AlreadyExists`, and the public key is returned as both an uncompressed SEC1 point (65 bytes) and SPKI DER so a server can verify with the `p256` crate.

- **A book's logo can lead back to its site.** `Site::home` sets where the logo in a book's header leads, for a book that is one part of a larger site, such as `.home("/")` for a book under `/docs/` beside a landing page. Without it the logo still leads to the book's first page. The leptoskit.dev documentation uses it, so its logo now returns to the landing page. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #573](https://github.com/leptoskit/leptoskit/pull/573)

#### Exact's examples on leptoskit

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #769](https://github.com/leptoskit/leptoskit/pull/769)

The example apps Exact publishes — Caltrain, Shelf, the calendar, the galleries, and the rest of that list, apart from the game engine — now run as leptoskit screens. `cargo run -p exact_examples -- <name>` prints a screen, and `leptoskit dev` in `examples/exact` serves them in the browser.

`cargo run --release -p exact_bench` times the same list workloads Exact publishes: the 10,000-row lists, the coin list, the extra-heavy feed, a Markdown document, and the feed from the Dioxus and web comparisons. The timings are headless build and commit costs on the machine that runs them.

#### Explicit PostgreSQL expansion windows

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #654](https://github.com/leptoskit/leptoskit/pull/654)

Prepare a migration compatibility window with `lk db migration window --baseline <migration> --out <file>`. It checks committed schema snapshots and exact generated SQL, writes a new evidence file without overwriting an existing one, and prints its SHA-256. Preparing a window opens no database and changes no migration or checksum.

Applications that explicitly support the protocol can call `Db::migrate_in_window` with their own embedded schema and migrations. Both the old and the new build must match a checkpoint. A restarted old build accepts later history only inside that window. Unknown migrations, modified checksums, missing history, and development pushes still fail. Ordinary `Db::migrate` and reverse migrations keep their strict checks.

PostgreSQL applies all pending expansions and history entries in one transaction, with bounded lock and statement waits and a total attempt deadline. The first structural subset covers plain new tables, nullable columns without defaults, and comments. It does not certify application business logic, positional SQL, physical schema drift, or custom SQL. SQLite online migration windows are not supported.

Cancelling an ordinary PostgreSQL migration also discards its connection instead of returning a session lock or unfinished transaction to the pool. Completed operations reuse the connection only after the migration lock is released.

Generated database servers embed their latest schema checkpoint with `migration_schema!`, export their migration contract before configuration or database startup, and use `Db::migrate_startup`. Without a configured window startup remains strict. A window requires both `LEPTOSKIT_MIGRATION_WINDOW` and its independent `LEPTOSKIT_MIGRATION_WINDOW_SHA256` pin; invalid or incomplete configuration fails instead of falling back.

Verified image packaging can bind an exact window with `--migration-window` and `--migration-window-sha256`. It compares a confined export from the packaged executable against the reproduced source and includes both documents in the complete release descriptor, covered by its existing approvals and policy digest.

Managed releases deliver that approved window as a read-only bind mount with its exact startup pin. Digest-named files are never replaced; a conflicting file stops the operation. Before recording success, deployment checks the running container's environment, mount, and file digest. Automatic recovery restores the previous release's window configuration or removes the candidate's window when the previous release had none. Initial setup requires a release without a window so the baseline can be installed first.

These changes do not remove managed deployment's maintenance requirement. Automatic online admission, all-running-version coverage, and retiring compatibility windows still need the managed release workflow. A document hash is an integrity pin, not release authorization or runtime attestation.

- **Game host.** `leptoskit game` builds and runs a game: gameplay stays Rust (`Game::setup` and `Game::tick` on a fixed step), and menus are ordinary leptoskit components committed through the same native renderer as the other platforms. The world is drawn with wgpu, or on the CPU when the machine has no adapter. The sample is `game/lantern`. Enable it with the `game` feature. The host writes a PNG frame; it does not open a window or package a phone or browser app. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #768](https://github.com/leptoskit/leptoskit/pull/768)

#### Go clients preserve explicit zero values

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #572](https://github.com/leptoskit/leptoskit/pull/572)

Generated Go clients use pointers for optional arguments and model fields, including slices and maps. Passing `nil` omits a field; passing a pointer to `false`, `0`, or an empty string includes that value. JSON and live calls also preserve empty collections; form and query encoding cannot express an empty collection. Previously `omitempty` could discard those values and cause the server to apply a default instead.

Regenerating changes Go source signatures: callers must pass addresses for optional values previously passed directly. The HTTP and live payloads share this representation. Optional nullable fields still have the null-versus-absence limitation reported by strict schema diagnostics.

#### Linux apps get sign-in and the system plugins

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #664](https://github.com/leptoskit/leptoskit/pull/664)

The generated Linux app now registers `secure-store`, `web-auth`, `device`, `localization`, `network`, `battery`, and `linking`, besides the plugins written in Rust that it already ran.

- **Sign-in works on Linux.** Providers sign in through the default browser, which comes back to the app on a loopback address. The session is kept in the desktop's Secret Service keyring, such as GNOME Keyring or KWallet. Linux has no platform passkeys, so `passkeys` is left out there without a warning, and passkey calls answer `unsupported`.
- **New projects' Linux hosts get the plugins too.** `leptoskit new --auth`, `--db`, and `leptoskit generate auth` add sign-in's plugins, or the HTTP plugin, to `linux/Cargo.toml`, so a new app's Linux build reaches its server.
- **Snapshot plugins answer.** `device`, `localization`, `network`, and `battery` answer snapshots on Linux. Waiting for a battery or network change is still unavailable there. `linking` opens links in the default handler.
- **No zone is not an error.** `localization` reports no time zone, instead of failing, on a system with none configured, such as a container.

#### Generated Linux apps run runtime plugins

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #662](https://github.com/leptoskit/leptoskit/pull/662)

The generated Linux app now registers the runtime plugins in `native.plugins` that are written in Rust, as the iOS, Android, and macOS apps already did. Add the plugin crate to `linux/Cargo.toml`, list it in `leptoskit.toml`, and `leptoskit build --platform linux` installs it. Plugins that answer at once run on the GTK thread, blocking ones on worker threads, and async ones such as `http` on a background runtime. Replies reach your screens on the GTK thread without polling. Plugins that keep files use the app's own folders under `~/.local/share` and `~/.cache`, named by `native.package_id`.

Plugins with only a Swift or Kotlin implementation, and app-owned services, are still left out on Linux with a warning, as is every plugin on Windows and in the terminal.

`leptoskit_bridge_ffi::app::PluginHost` is the plugin host that makes this work, for any runner that owns its `WasmHost`. `desktop_context` gives such a runner its app folders. A Linux app created before this release keeps building without plugins; to add them, compare its `linux/src/main.rs` with a new project's.

#### Motion for the shared component tree

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #554](https://github.com/leptoskit/leptoskit/pull/554)

Elements can move. `motion(Motion)` describes the poses an element moves between, as opacity, offset, scale, rotation, clipping, and blur, and what drives the movement: appearing (`Motion::enter`), scrolling into view (`in_view`), or a named container scrolling past (`timeline` on the container and `follow` on the elements inside it). `stagger` on a container starts its children's motions one after another. Timing comes from a duration and an `Easing`, or from a `Spring`, and a new `key` plays an enter motion again.

The web renders motion as CSS animations and scroll-driven animations, with springs as `linear()` easing, so nothing runs in script. People who ask for less motion, and browsers without scroll timelines, see every element at rest at once. Native platforms show the resting pose too: motion is web-only for now, and native apps never receive it, so adding it changes nothing they build.

The leptoskit.dev landing page is redesigned around it: one component tree becomes each platform's own widgets as you scroll, with every movement written as `Motion` and the hand-written animation script removed.

#### Animations that run on the platform

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #544](https://github.com/leptoskit/leptoskit/pull/544)

Widgets can now move, scale, and rotate, and animate changes without the app computing a single frame. Tailwind's `translate-*`, `scale-*`, `rotate-*`, `transition-*`, `duration-*`, `delay-*`, `ease-*`, and `animate-spin`/`ping`/`pulse`/`bounce` classes work, as does the `starting:` variant for the look a widget enters from. For anything else, elements take `transform`, `transition`, `keyframes`, `entering`, `exiting`, and `layout_transition` values from the new `leptoskit_animation` crate (re-exported as `leptoskit::native::animation`), with springs, timing curves, and presets modelled on React Native Reanimated.

The guest describes an animation once and the platform runs it: Core Animation in the generated iOS and macOS apps, Android's animators in the generated Android app, and CSS in the browser. The web doesn't run `exiting` or `layout_transition` yet, and Windows, Linux, the terminal, and HarmonyOS ignore transforms and animations for now.

#### Bundled images on every platform, and launch screens

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #685](https://github.com/leptoskit/leptoskit/pull/685)

`Image("asset://portraits/ada.webp")` shows `assets/portraits/ada.webp` from your project on the web, iOS, Android, macOS, Windows, Linux, and HarmonyOS (experimental). The browser loads it from `/assets/`, where the web build already serves the folder, and `leptoskit build` now copies `assets/` into every native app: the iOS app bundle, the APK's assets, `Contents/Resources` on macOS, beside the Windows and Linux executable, and HarmonyOS's `rawfile`. Only changed files are copied again, deleted ones leave the app, and hidden files stay behind. Before, the native apps bundled the folder only when they showed 3D scenes. PNG, JPEG, and WebP decode natively; Linux needs the system's WebP loader for WebP. Assets ship with the store build, not with code push updates.

A missing asset stops a debug build with an error that names the source and where it looked, and a release build logs it and leaves the image empty. `leptoskit_native::asset` parses sources the same way on every host and refuses paths that leave the folder.

The audio plugin's `Source::Asset` reads the same copy, so sounds in `assets/` now play on native apps without copying them into the host project. On Android it reads `assets/<path>` inside the APK's assets, where the build puts them; a file you placed at the root of `android/app/src/main/assets/` by hand moves into an `assets/` folder there, or into the project's `assets/`.

`[app.launch_screen]` gives iOS and Android a launch screen: a `background` colour, an optional `background_dark`, and an optional square `image` (PNG or SVG, read like `app.icon`) drawn at `image_size` points or dp. iOS gets a colour set and image set in its asset catalog, named by `UILaunchScreen` in the generated `Info.plist`. Android gets a launch theme for the launcher activity, extending the application's own theme, with the splash screen attributes on Android 12 and later and a window background before; once the first screen draws, the generated app puts the theme's own background back.

#### Failures on a device are written to the platform's log

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #724](https://github.com/leptoskit/leptoskit/pull/724)

When an app failed on a phone, the platform's log often said nothing: a plugin refused, a prop failed to decode, or the interface trapped, and the screen stayed blank. The runtime now writes what happened to the platform's own log: logcat on Android, the unified log (`os_log`, subsystem `dev.leptoskit`) on iOS and macOS, standard error on Windows and Linux, and the console in a browser. It sets this up when the app starts, with no app code.

Each record is tagged with where it comes from (`leptoskit/bridge`, `leptoskit/plugin/<name>`, `leptoskit/render`, `leptoskit/wasm`, `leptoskit/gpu`, `leptoskit/panic`) and carries what the failure was about:

- a plugin that cannot register says which plugin and why, and a failed call names its plugin, method, call id, and error, whichever host it ran on;
- props that do not decode name the node, its kind, the field, and up to 512 bytes of the payload around the mistake;
- a trap names the guest function that was running and the call into the platform it was making;
- a panic on any thread is logged with its location, thread, and backtrace before earlier panic hooks run;
- Kotlin and Swift failures carry their whole cause chain;
- drawing surfaces and the GPU device log when they are created, resized, lost, and restored.

Whatever stops the interface is logged at `error` in every build. A debug build also logs the app's lifecycle at `info`: creation, each plugin, the guest's start, the first commit, and each pause and resume. A release build keeps `warn` and above. `[native] log-level` in `leptoskit.toml` sets another level, from `off` to `trace`.

Nothing is sent anywhere: there is no telemetry, and records stay in the device's own log. The new `leptoskit_log` crate holds the sinks, the panic hook, the bounded payload excerpt, and a capturing sink for tests.

#### Native guests get real, cancellable timers

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #609](https://github.com/leptoskit/leptoskit/pull/609)

A generated native app's guest now has a real timer. `leptoskit_bridge::guest::sleep`, re-exported as `leptoskit::guest::sleep`, waits on the host instead of resolving at once, and `leptoskit_query`'s `guest` feature uses it for retry backoff, debounce, and cache expiry. Dropping the future cancels the host's timer, so an abandoned retry or debounced update costs nothing until its deadline.

The timer is the reserved `leptoskit.timer` plugin on the ordinary bridge, not a new mandatory Wasm import, so a bundle still loads on an older host. That host leaves the plugin unregistered and reports `Unavailable`; the query cache's `has_timer` reports `false` there rather than pretending a sleep finished. A custom provider installed with `install_sleep` still takes precedence, and browser builds keep using `setTimeout`.

Generated macOS, iOS, Android, Windows, Linux, terminal, and experimental HarmonyOS apps enable the timer. HarmonyOS schedules one ArkTS timeout for the next host deadline and cancels it when the runtime closes. Hosts bound the timer to 256 waiting or undelivered sleeps and reject a single bridge request longer than 24 hours. Query timers split longer waits into bounded requests. The new `try_sleep` operation reports unavailable or rejected timers: request retries surface their original error, intervals stop, and cache cleanup retains entries when a wait cannot finish. The infallible `sleep` never reports a failed wait as elapsed; it stays pending until dropped.

Live RPC connections also wait after a failed handshake. If a host cannot schedule the retry, the connection becomes offline until new work or credentials request another attempt; missing or full timer queues no longer trigger automatic retry loops.

Timer request and cancellation bursts are drained in bounded batches. If work remains after the per-turn budget, the host requests another immediate turn rather than waiting for an unrelated long timer.

- **Platform extension contracts.** Adds `leptoskit_platform`: versioned manifests, capability matrices, and scaffold file lists for niche OS shells. The CLI exposes a live builtin registry through `leptoskit native platform list|capabilities|scaffold`. Concrete in-tree shells land in follow-up crates; out-of-tree sidecars will reuse the same manifest shape later. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #670](https://github.com/leptoskit/leptoskit/pull/670)
- **HarmonyOS platform extension crate.** Moves HarmonyOS ArkUI host templates and the capability manifest into `leptoskit_platform_harmonyos`. The CLI still runs prepare, NDK cross-compilation, and HAP packaging; `leptoskit native platform` lists the extension. Authors keep using `--platform harmonyos`. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #671](https://github.com/leptoskit/leptoskit/pull/671)
- **Meta Quest / Oculus platform extension.** Adds experimental `leptoskit_platform_oculus` host stubs and a capability manifest, registered with `leptoskit native platform`. Shipping headset packaging stays on first-party `quest`; this extension does not add an `oculus` `Platform` enum variant. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #672](https://github.com/leptoskit/leptoskit/pull/672)

#### Play the examples on the docs site

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #590](https://github.com/leptoskit/leptoskit/pull/590)

The screens on leptoskit.dev's examples page and home page were pictures. Each one is now the example's real web build, running in a sandboxed frame on the page: press, type, scroll, and navigate in it. The gallery's seven screens share one module, chosen with `?screen=`, so the browser downloads it once; Observatory and Postcard gained browser clients, and Field, the file routes example, plays too, with a back button that walks its own history. A player loads as it nears the screen, keeps the page from moving while it does, and switches to the screen's AppKit render, so the native views are one press away.

The gallery's dashboard range picker now chooses what the revenue chart covers: seven days, thirty days, or twelve months. The profile's Follow button follows and unfollows, and counts the follower.

- **A real Rust playground.** Add a standalone, container-isolated Rust compile service and a static playground editor with an opaque-origin browser preview. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #604](https://github.com/leptoskit/leptoskit/pull/604)

#### Plugin fixes from a line-by-line audit

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #574](https://github.com/leptoskit/leptoskit/pull/574)

- **passkeys:** Android now requires user verification on every passkey request, as iOS and macOS already did, whatever the options ask for.
- **location:** on Android, a second fix request while one is pending returns `Busy`, as the readme says. Before, it replaced the first request, whose location updates kept running and whose caller never got an answer.
- **network:** on iOS and macOS, `snapshot` works while `wait_for_change` is pending. It used to return `Busy`.
- **posture:**
  - On iOS before 27.1, `watch` waits instead of spinning.
  - A status read works while a watch is pending.
  - A fold outside the view no longer ends the watch with an invalid region.
  - On Android, closing the plugin ends a pending watch and its hinge listener.
- **screen-capture:** on iOS, opening Control Center to start a recording no longer ends a capture watch.
- **speech:** on iOS and macOS, a late cancel report from an earlier utterance can no longer stop the next one.
- **battery:** a new `State::NotCharging` reports a device that is plugged in but holding its charge. Android and macOS used to call this discharging or unknown.
- **haptics:** Android returns `Unavailable` when the system plays nothing, instead of success. Calls made off the main thread on iOS and macOS return `Native` instead of `InvalidArgument`.
- **localization:** Android reports modern language codes (`he`, `id`) that match the locale tag, not legacy ones (`iw`, `in`).
- **device:** an Android phone in a desk dock is no longer reported as a desktop, and VR headsets report `Headset`.
- **notifications:** iOS and macOS no longer ask for sound permission, which the plugin never uses.
- **clipboard:** the experimental Rust Android path reads plain text only, like the Kotlin plugin, instead of opening content URIs on the UI thread.
- **web-auth:** on desktop, a call with the wrong session ID no longer discards the real session.
- **http:** requests are limited to 128 headers, the same limit the bridge carries, and browser responses with more are refused the same way.
- Readmes for secure-store, shell, web-auth and menu, and the plugin matrix, now describe what the code does: macOS keychain accessibility, process-group containment, cancellation per platform, Windows test coverage, and shell's automatic registration on macOS.

#### Signed release builds for the stores

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #678](https://github.com/leptoskit/leptoskit/pull/678)

`leptoskit build --release --platform android`, `quest`, and `ios` now make what each store takes, signed with material read only from the environment, so a secret manager can hand it to one build: `monosecret run --profile release -- leptoskit build --release --platform android`.

- **Android and Meta Quest** sign the App Bundle and the APK with your upload key, from `LEPTOSKIT_ANDROID_KEYSTORE` (or the keystore itself, base64-encoded, in `LEPTOSKIT_ANDROID_KEYSTORE_BASE64`, which is written to a private temporary file and removed afterwards), `LEPTOSKIT_ANDROID_KEYSTORE_PASSWORD`, `LEPTOSKIT_ANDROID_KEY_ALIAS`, and `LEPTOSKIT_ANDROID_KEY_PASSWORD`. Gradle reads the passwords from its environment, never its command line. The build checks that both are signed and copies them to `.leptoskit/release/<target>/<name>-<version>.{aab,apk}`.
- **iOS** archives the app and exports `.leptoskit/release/ios/<name>-<version>.ipa` for the team in `LEPTOSKIT_IOS_TEAM_ID` (or `native.apple_team_id`). Automatic signing signs in with the App Store Connect key `leptoskit publish` uses, or the account in Xcode; manual signing names `LEPTOSKIT_IOS_CODE_SIGN_IDENTITY` and `LEPTOSKIT_IOS_PROVISIONING_PROFILE`. `[native.ios]` sets `signing-style` and `export-method` (`app-store-connect`, `ad-hoc`, or `development`), and `--export-method` overrides it for one build.
- `--build-number <n>` sets Android's `versionCode` and iOS's `CFBundleVersion`.

A release never falls back to an unsigned build or the debug key. Without its signing material it stops before compiling and names every missing variable; a keystore or key file inside the project's repository, Android's `debug.keystore`, and the `androiddebugkey` alias are refused. `--unsigned` builds an unsigned release on purpose, as earlier versions did, for measuring and testing. Debug builds and simulator builds are unchanged.

- **File routes under a sub-path.** `Router::from_browser` reads the page's `<base href>`. An app served under `/app/` declares `<base href="/app/" />`: its routes match the path after it, and every link and history entry the router writes starts with it. Without a `<base>`, the app is at the root of its site, as before. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #590](https://github.com/leptoskit/leptoskit/pull/590)

#### RPC clients report schema fidelity loss

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #567](https://github.com/leptoskit/leptoskit/pull/567)

`leptoskit rpc generate` now warns, with the schema path and the target's generated fallback, wherever a client cannot carry a schema's shape: an arbitrary union or fixed tuple that collapses to `any`, an unresolved `$ref`, a multi-type or multi-member `allOf` schema, a non-string enum, nullable fields whose explicit null becomes absence, explicitly allowed extra object properties, and wide integers outside TypeScript or Dart runtime limits. Deliberately unconstrained JSON (`true`, `{}`, an untyped map or array) is reported as intentional, not lossy.

The default output is unchanged: every warning still generates the documented fallback. `leptoskit rpc generate --strict` fails before writing any file when any requested target loses structure, so a pipeline that needs exact types can refuse the fallback instead of discovering it in the generated code. `ApiSpec::diagnostics` exposes the same per-target report to callers of `leptoskit_rpc_codegen`. OpenAPI preserves schema nodes and does not run typed-client diagnostics. Strict mode detects known representation losses; it is not a general JSON Schema validator.

#### GPU 3D scenes

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #663](https://github.com/leptoskit/leptoskit/pull/663)

The new `leptoskit_3d` crate, also `leptoskit`'s `3d` feature, draws 3D scenes on the GPU in a new `Scene3d` view: glTF models with skins, morph targets, and animation, lit by a directional light and a soft studio environment, each with a soft shadow beneath it. You build a scene with `scene3d::client::Scene` (load a model by path, spawn it, play or crossfade a clip, set morph weights and material overrides, show or hide named parts, attach one model to another's part, place the camera, and raycast), and the host animates it on the display's own clock. Your interface never re-renders to move a model, and a still scene draws nothing.

Scenes draw with WebGPU or WebGL2 in the browser, Metal in the generated iOS and macOS apps, and OpenGL ES in the generated Android apps, where `dev.leptoskit.scene3d.backend` manifest metadata chooses Vulkan instead. `leptoskit build` bundles the app's `assets/` folder into native apps that use 3D. Models may be compressed with meshopt and quantized, as `gltfpack` writes them. Windows and Linux leave an empty space for the view for now, the terminal draws a placeholder, HarmonyOS stops with an error, and the watch surfaces refuse it.

`leptoskit lint` knows `Scene3d` as a built-in component, so its `markup_style` rule holds a scene to the project's `[lint] markup` style like any other view.

The renderer also draws into any wgpu texture you hand it, from one view or several, over a camera image, and hidden behind real-world depth, for hosts that bring their own surfaces, such as augmented and virtual reality.

#### Schema packs declare their membership

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #586](https://github.com/leptoskit/leptoskit/pull/586)

`leptoskit::db::packs!` now records the schema packs an app expects, and export records each pack's identity and the tables it owns in `schema.json` (format 2). A pack that is declared but exports no tables is refused before the first migration, and a pack the latest snapshot or a development push recorded can no longer disappear as an ordinary destructive migration: `db migration create`, `db push`, `db migration consolidate`, and the development watch require `--drop-pack <name>` for each pack whose removal is intended. `--allow-destructive` does not cover a pack removal, so an omitted dependency or feature cannot silently drop a library's tables. Changes inside a pack that is still present stay ordinary migrations.

Pack crates declare their identity and tables once with `leptoskit_db::schema_pack!`, which keeps the pack's tables in the export even when the app only names the pack in `packs!`. `leptoskit_auth`, `leptoskit_idp`, and `leptoskit_rpc` already do. Existing format 1 snapshots remain readable; they carry no membership evidence, so a pack that vanished before the snapshot was written cannot be detected from it, and the next migration writes format 2.

#### Stream motion sensors, and read them all from one plugin

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #571](https://github.com/leptoskit/leptoskit/pull/571)

`leptoskit_plugin_sensors`, runtime name `sensors`, is the port of Flutter's `sensors_plus`. One plugin streams the accelerometer, acceleration with gravity removed (new on every platform), the gyroscope, the magnetometer, and the barometer:

```rust
let sensors = Sensors::new()?;
let mut tilt = sensors.accelerometer(Interval::GAME);
while let Ok(sample) = tilt.next().await { /* … */ }
```

The `accelerometer`, `gyroscope`, `magnetometer`, and `barometer` plugins gain the same `watch` stream, and in the browser `web::watch` keeps a Generic Sensor or `devicemotion` listener running. Sensor plugins used to read one sample per call and stop the hardware in between. A stream keeps the sensor running between reads and holds the newest 64 samples. `Watch::dropped` counts any samples your app read too slowly to see, and dropping the stream stops the sensor. `Interval::NORMAL`, `UI`, `GAME`, and `FASTEST` match `sensors_plus`'s presets.

On iOS, every sensor plugin now reads through one shared motion manager, as Apple recommends, so selecting several plugins no longer starts competing CoreMotion feeds.

**Behaviour change:** iOS acceleration now uses the same axis directions as Android and the browser. A phone lying face up reads about +9.8 m/s² on z everywhere; iOS used to report −9.8. If your app compensated for the iOS sign, remove that compensation.

#### Service databases in `leptoskit db`, branches, and releases

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #565](https://github.com/leptoskit/leptoskit/pull/565)

Every `leptoskit db` command takes `--service <name>` to work on a service's own database instead of the app's: `leptoskit db migration create add_referrer --service analytics` reads the tables from the service's crate and writes the migration into its `migrations/` directory, or `migrations-postgres/` when the app is on Postgres, and `migrate`, `status`, `check`, `push`, `rollback`, `reset`, `studio`, and `release record` work the same way. Without `--url`, a command uses the database `leptoskit dev` gives the service, or `LEPTOSKIT_SERVICE_<NAME>_DATABASE_URL`; the app's `DATABASE_URL` never reaches a service command. A service installed from a registry or git ships its migrations, so the commands that write migrations refuse it. Unknown, remote, and database-less services fail with an error naming `[services.<name>]`.

On the development Postgres `leptoskit` runs, a service now logs in as a role of its own, `<app>_service_<name>`, which owns its database and can't connect to any of the app's databases; its password reaches Postgres only as a SCRAM verifier. When the app's `DATABASE_URL` names another server, and always under `leptoskit serve`, `leptoskit` creates and changes nothing there: the service's database comes from `LEPTOSKIT_SERVICE_<NAME>_DATABASE_URL`. On a git branch with its own app database, the service's database starts as a copy of the main branch's, and `leptoskit db branch list` and `prune` handle it with the app's.

`leptoskit infra` releases record each service database's migrations beside the app's. A release that changes a service's tables needs `--maintenance`, a service database that drifted from the history stops the release, and `lk deploy --rollback <tag> --rollback-schema --yes` checks every database first, then stops the services and takes each service's database back to the release's tables with its down migrations. Releases recorded before this hold only the app's tables, so rolling back to one leaves the services' databases alone. Images built by `leptoskit infra build` carry an `io.leptoskit.services.migrations` label when the app has database-backed services.

#### Operations that run in a service

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #553](https://github.com/leptoskit/leptoskit/pull/553)

A query, mutation, or action can belong to a service: a separate process the app runs beside its server. Declare it with `#[rpc::query(service = "analytics")]`, in a library both the service and the app depend on, and call it the way you call the app's own operations. From the browser, `rpc::client::call` and `use_live` send it to `/_leptoskit/services/analytics/` on the app's origin. From a native app, it goes to the same path on the configured origin. From the app's server while it renders, it goes over HTTP to `LEPTOSKIT_SERVICE_ANALYTICS_URL` as an anonymous caller, so a page renders only what the browser could fetch itself. `rpc::client::call_as_app`, on the server only, signs a call with `LEPTOSKIT_SERVICE_ANALYTICS_TOKEN` so the service treats it as the app. Live queries the service owns run on the service's own live connection, over its own database. The app's own credentials, such as a native app's access token, never go to a service unless `ClientConfig::service_credentials` names it, and a service's connection doesn't share the app's offline queue.

The body is compiled only where the declaring crate's `service` feature is on, so the app builds without the service's dependencies. `Server::new(db).service("analytics")` serves only that service's operations, and the app's server never serves them, even when a workspace-wide build links their bodies into it. An app operation and a service operation may share a name. `rpc::server::ServiceCallers` tells the app and its services apart by their tokens: in a service the app is the caller `"app"`, and in the app a service is `"service:<name>"`. The app's live connection refuses a service, whether it connects with the service's token or signs in with it later, because a service reads the app's data by calling its operations.

Jobs, cron schedules, streams, and channels can belong to a service too: `#[rpc::job(service = "analytics")]`, `#[rpc::stream(service = "analytics")]`, `#[rpc::channel(..., service = "analytics")]`. Only the service's workers run and schedule its jobs (`Jobs::service`, defaulting to the installed server's service), enqueueing one from another process is an error, and only the service's live socket serves its streams and channels. A service's operation can't take a toolkit `rpc.*` name, and an app operation named `rpc.…` is no longer served by every service as if it were built in.

`SpecOptions::service` (or `LEPTOSKIT_RPC_SPEC_SERVICE`) exports a service's spec, and the app's spec leaves service operations out. The facade's `rpc-services` feature lets a server that doesn't serve operations itself call a service.

#### A runtime for services that run beside the app's server

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #553](https://github.com/leptoskit/leptoskit/pull/553)

`leptoskit::service` is what a service, a separate process the app runs beside its server, needs at runtime. `ServiceEnv::from_env()` reads what `leptoskit dev`, `leptoskit serve`, and deploys start it with: its name, the address to bind, the token its trusted callers present, the app's private URL, its own database, and its volumes. `ServiceEnv::serve` serves its router with `/healthz` and `/readyz` and drains on `SIGTERM`, and `RequireToken` keeps a route to callers holding the token.

`leptoskit::service::main!(analytics_service, run)` writes the whole `main` of a service binary. It also links the library that declares the service's operations, without which the binary would answer every call with `404`. On the app's side, `service_url` and `service_token` read where a service is and how to call it.

#### Services beside the app server

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #553](https://github.com/leptoskit/leptoskit/pull/553)

A `[services.<name>]` table runs another Rust binary beside the app's server, with its own port, its own database, and its own public paths on the app's origin. A service comes from a crate in the workspace (`package`), a crate `leptoskit` builds from a `path`, the registry (`crate`, `version`), or `git` (`rev`, `branch`, or `tag`), or it already runs elsewhere (`url`). The service crate describes itself in `[package.metadata.leptoskit.service]`: its `port`, whether it's `public`, extra `routes`, the `features` its binary needs, a `database` of its own with an optional `pool` size, `volumes`, `env`, `secrets`, and the variables it `bind`s on the app. The table in `leptoskit.toml` overrides any of them.

```toml
[db]
dialect = "sqlite"

[services.analytics]
package = "analytics_service"
```

`leptoskit build` builds the app's server and its workspace services in one cargo invocation, and installed services on their own, pinned by `services.lock`. `leptoskit dev` and `leptoskit serve` give each service its database (a SQLite file under `.leptoskit/services/`, or a database of its own on the app's Postgres server), its volumes, and a token; start each service and wait for `/readyz`; then start the app with `LEPTOSKIT_SERVICE_<NAME>_URL`, `LEPTOSKIT_SERVICE_<NAME>_TOKEN`, and the service's bindings. When a service is public, a proxy on the configured port sends `/_leptoskit/services/<name>` and its `routes` to it, streaming bodies and handing WebSocket upgrades through, and everything else to the app. A service whose `trust` is `isolated`, the default for installed and remote services, never receives the app's cookies and can't set them.

`leptoskit services` lists the resolved services, their databases, public paths, volumes, and the variables the app receives, without printing a token.

#### Services deploy beside the app

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #553](https://github.com/leptoskit/leptoskit/pull/553)

`leptoskit generate deploy` and `lk infra` now run an app's `[services]` next to its server, from the same image and release.

- **Dockerfile:** the release build copies each service's binary to `/app/services/<name>`, and the health check asks whichever port the container serves.
- **Compose:** each service is its own entry with its token, database, and volumes. A Caddy edge on the app's port sends public service paths to their service, and strips cookies for isolated ones. On Postgres, each service gets a database and role of its own on the first start.
- **Kubernetes:** each service is a Deployment and a Service with a Secret of its own, and an Ingress routes the public paths, next to `--split-roles` too. The Fly and systemd targets refuse services they can't run.
- **`lk infra`:** each service is a Kamal role on the primary host, reached privately through a network alias. Its public paths go through kamal-proxy with the prefix kept. Its secrets reach only it, through Kamal's `NAME:SECRET` aliases, and the app's environment moves to the app's own role. Releases create each service's Postgres database and role, the backups cover every service's database, and restores bring them back with the app's.
- **Connection budget:** `[infra.postgres]` sets `max_connections` and the app's `app_pool`. `lk infra plan` and `lk doctor` fail when the app's and the services' pools, counted through a deploy, don't fit.

#### Services know who is signed in

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #566](https://github.com/leptoskit/leptoskit/pull/566)

A service can now tell who is calling, so it can keep per-person data such as "my events" and per-person live queries. The app vouches for the person: its server mints a token that names them (their `Actor`, such as a `leptoskit_auth` account), lasts five minutes, and works only at the one service it's for. The key comes from the token `leptoskit` already gives the app and each service, through HKDF with its own label and the service's name, so nothing new has to be deployed.

- **Configuration.** `forward_identity` in a `[services.<name>]` table turns it on or off. It defaults to `true` for first-party services and `false` for isolated ones, and only `leptoskit.toml` can set it, never a service's manifest. `leptoskit dev`, `leptoskit serve`, Compose, Kubernetes, `leptoskit infra`, and the Docker steps give the app `LEPTOSKIT_SERVICE_<NAME>_IDENTITY=forward` for each such service, and `leptoskit services` lists it.
- **Browsers and native apps.** Before the first call to a service, the client asks the app for that service's token with the built-in action `rpc.service_token`, signed the way the app already is. It sends the token with that service's HTTP calls and on its live connection only, renews it before it runs out or when the service refuses it, and forgets it on `rpc::client::reauthenticate()`, which `leptoskit_auth`'s client calls on every sign-in and sign-out. With no token to give, calls go anonymously. A token can't be revoked once handed out: a service can keep seeing a person for up to about five and a half minutes after they sign out or their session is revoked.
- **Server rendering.** A page rendered for a signed-in person calls the service as that person, so the page holds what their browser would fetch. `call_as_app` is still the only way to call as the app.
- **In the service.** `ServiceCallers::for_service` accepts these tokens and identifies the person as the caller `person:<id>`, with their `Actor` and a `ForwardedIdentity` in the context. A token for another service, a changed one, or an expired one is refused with `401`. A live connection keeps the person until their token runs out and asks the client for a new one a minute before.

`rpc::server::service_identity_token` and `IdentityKey` mint and check tokens by hand. `examples/analytics` gains a live `mine` query of the signed-in person's own events, and records a signed-in person's events under them.

#### Flutter-style splash and icon assets

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #708](https://github.com/leptoskit/leptoskit/pull/708)

`[app.icons]` declares launcher icons the way Flutter's icons_launcher does: one default image, per-platform overrides, Android adaptive layers, a notification / status-bar icon, iOS dark and tinted appearances, and web favicon / maskable outputs. Windows gets an `.ico`, Linux gets hicolor PNGs, and free-form `[app.icons.extensions.<name>]` tables keep metadata extensible without opening unknown keys. `app.icon` still works as the simple path.

`[app.launch_screen]` (also `[app.splash]`) grows Flutter native-splash fields: branding, fullscreen, platform enable flags, an Android 12 subsection, web splash HTML/CSS (GIF-friendly), `[app.launch_screen.animated]` for Rive/GIF overlays, and `[app.launch_screen.extensions.<name>]`. `leptoskit::splash::{hold, dismiss}` keeps that overlay until the app is ready.

#### Stores for a state struct

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #615](https://github.com/leptoskit/leptoskit/pull/615)

`leptoskit::store` re-exports Leptos's `reactive_stores`: derive `Store` on a state struct and each field becomes its own signal, so reading `store.title()` subscribes a screen to that field alone and writing it never notifies a sibling. The struct stays a plain Rust value, `ArcStore` holds it for as long as a clone lives (like `ArcRwSignal`), and `Store::new` follows the reactive owner instead (like `RwSignal`). Deriving `Patch` as well adds `.patch(next)`, which applies a whole new value — a reducer's output, or fresh data from the server — and notifies only the fields that changed. The derives need `reactive_stores` in the app's `Cargo.toml`; the types come re-exported, at the version leptoskit was built against. Fields are signals like any other, so the same code runs on web, iOS, Android, macOS, Windows, Linux, the terminal, and HarmonyOS (experimental).

`leptoskit::prelude` also gains the `GetUntracked`, `Update`, `With`, and `WithUntracked` signal traits, next to the `Get` and `Set` it already carried, for reading and changing store fields and signals alike.

The `opencode` example keeps its reducer and now applies it with `patch`, and `commonplace` writes one field at a time; both notify per field on every platform.

#### New apps publish a typed RPC

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #570](https://github.com/leptoskit/leptoskit/pull/570)

A new app's API is now typed operations that every platform calls by name and every generated client describes. `leptoskit rpc spec` and `leptoskit rpc generate` work on a fresh scaffold: the server writes the published API and exits before it reads its configuration, opens the database, or loads sign-in's keys, so exporting needs no `DATABASE_URL` and no secrets, and creates no database file.

- Every new app publishes its backend's `health` check, still `GET /api/health`, as an `#[api]` server function, and `<name>_shared`'s `Health` derives `JsonSchema`.
- With `--db` or `--auth`, `<name>_shared` declares a `notes` query and an `add_note` mutation. The browser, the native apps, and the server call them by their argument structs, `Notes {}` and `AddNote { body }`; only the shared crate's `server` feature keeps their bodies, so no client links the database driver. The server serves them through `leptoskit::rpc::server::Server` and over the live socket at `/live`, with a `live` readiness check, and on `SIGTERM` it tells live sockets to reconnect before it drains. The `Note` table is `live`, and the db crate adds the `rpc` schema pack, whose receipts make a re-sent mutation run once.
- With `--auth`, operations run as whoever signed in: each note has an `owner_id`, and the table's policy shows and changes only the caller's own. The live socket follows `[auth] origin` once it's set. `/healthz` and `/readyz` sit outside the sign-in layer, `/readyz` waits for the migrations, and the server drains on shutdown like the others.

The scaffold's notes API changed shape, in new apps only: `GET /api/notes` is now `POST /api/notes` with `{}`, and `POST /api/notes` with `{"body": …}` is now `POST /api/add_note`. A failed call answers `Kind|message` text, and an empty note is `400` rather than `422`; `404`, `409`, and `500` keep their meaning, and with sign-in a signed-out call answers `401`. Apps created by an earlier leptoskit keep the handlers they have.

In a native app's guest, which has no timer, the live client no longer connects when it's configured: it waits until there's a call, a subscription, or a channel to serve, or calls in the offline queue. Before, an app that configured the client without going live, as sign-in's `configure` does once the app has operations, retried a socket it couldn't open without pause and never finished starting. `leptoskit_query::has_timer` says whether a host's `sleep` really waits.

#### Every font weight, and line height

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #760](https://github.com/leptoskit/leptoskit/pull/760)

Text takes all nine of Tailwind's weights: `font-thin`, `font-extralight`, `font-light`, `font-normal`, `font-medium`, `font-semibold`, `font-bold`, `font-extrabold`, and `font-black`. Before, anything outside `font-normal` to `font-bold` stopped the app as an unsupported utility. `FontWeight` gains the five new variants and `FontWeight::value`, the CSS number from 100 to 900. A platform whose system font lacks a weight draws the nearest one, as browsers do.

Line height is a text property now. `leading-none`, `leading-tight`, `leading-snug`, `leading-normal`, `leading-relaxed`, and `leading-loose` set it as a multiple of the font size, `leading-6` (any step on the spacing scale) as a fixed height, and `leading-[1.4]`, `leading-[150%]`, or `leading-[20px]` as you write it; `text-sm/6` sets a size and a line height together. Descendants inherit it, and a multiple follows each one's own font size. Every line takes exactly the line height with its glyphs centered, as in CSS, and layout reserves that height, on the web, macOS, iOS, Android, Linux, and HarmonyOS. Windows and the terminal keep the font's own line height, and native text fields keep their single-line height.

`NativeProps::line_height`, `LineHeight`, `Element::line_height`, and `TextStyle::line_height` carry it. A custom `TextMeasurer` whose platform cannot set line height can call `TextStyle::fit_line_height` to reserve what the renderer draws. `TextStyle` gains a field, so a struct literal needs `line_height: None` or `..TextStyle::new(…)`.

- **Reproduce website assets before verified packaging.** Add an explicit versioned website recipe that runs the production web pipeline offline in a digest-pinned builder, compares complete asset trees, and packages the checked backend and website bytes. Website descriptors bind paths, modes, hashes and build inputs to independent approvals and image-bound deployment policies; incomplete website evidence fails closed. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #597](https://github.com/leptoskit/leptoskit/pull/597)

#### Watch UI API for Wear OS and watchOS

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #589](https://github.com/leptoskit/leptoskit/pull/589)

Watch apps get their own surface vocabulary — `WatchApp`, `WatchPage`, `WatchTile`, `WatchGlance`, and `WatchComplication` — under `leptoskit::watch`, with hard layout budgets so a phone tree cannot be pasted onto a watch unnoticed. Content still uses leptoskit elements and Tailwind; the containers are watch-sized.

Android Wear OS (`wearos`) and Apple watchOS (`watchos`) are experimental platforms: config and CLI know them, `leptoskit new --platform wearos|watchos` scaffolds host stubs, and `leptoskit native prepare` writes them. Packaging and device install are not implemented yet. Enable with the `watch`, `wearos`, or `watchos` features.

- **Textured models packed by gltfpack.** Scenes read `KHR_texture_transform`, so a textured model compressed by `gltfpack` draws its textures where they belong. gltfpack quantizes texture coordinates and records the scale and offset that undo it as a texture transform; scenes used to ignore it, and every texture was sampled from its corner. The transform is applied to a primitive's texture coordinates as the model loads. A material's textures share one transform, the first found starting from the base color texture, which is what gltfpack writes. _Packages:_ 🟠 _leptoskit_3d_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #713](https://github.com/leptoskit/leptoskit/pull/713)
- **Deliver primary-pointer drags, taps and cancellations from Android, iOS and macOS scene views through `Scene::next_pointer`.** Movement uses layout units and a bounded queue coalesces drags independently of model-loading events. _Packages:_ 🟠 _leptoskit_3d_, 🟢 _leptoskit_bridge_ffi_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #734](https://github.com/leptoskit/leptoskit/pull/734)
- **Additive light for glows, flashes, and sparks.** `Scene::set_blend(node, part, Blend::Additive)` (`Op::SetBlend`) makes a part of a model add its colour to what is drawn behind it, scaled by its opacity, instead of covering it. A muzzle flash, a spark, or a soft halo then reads as light over a camera picture, and black adds nothing, so a round glow needs no cut-out. `Blend::Normal` puts it back. Additive parts draw with the transparent parts. The op is new, so batches without it encode exactly as before. _Packages:_ 🟠 _leptoskit_3d_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #714](https://github.com/leptoskit/leptoskit/pull/714)

#### Move a model's own parts from your app

_Packages:_ 🟠 _leptoskit_3d_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #712](https://github.com/leptoskit/leptoskit/pull/712)

`Scene::set_part_offset(node, part, offset, seconds)` (`Op::SetPartOffset`) moves a named part of a model instance, a bone or any node, from where its rest pose or the playing clip puts it. The offset turns the part about its own origin and moves it along its own axes, and everything under the part follows, skinned meshes included, so an app can turn a creature's head towards the camera while its walk clip plays, swing a rigid leg, or open a hinged lid by exactly as much as its own state says.

The host moves the part there over `seconds`, in a straight line from where it is now, on its own display clock. An app that works out a pose on each tick of its own clock and sends it with the tick's length is drawn smoothly between ticks. `Transform::IDENTITY` puts the part back. The op is new, so batches without it encode exactly as before.

#### 3D scenes over the live camera on iOS and Android

_Packages:_ 🟠 _leptoskit_3d_, 🟠 _leptoskit_xr_, 🟠 _leptoskit_bridge_ffi_, 🟠 _leptoskit_cli_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #681](https://github.com/leptoskit/leptoskit/pull/681)

A `Scene3d` whose backdrop is `Backdrop::Camera` now draws over the camera while an `xr` session runs on a phone. The generated iOS and Android apps hand the scene the session's newest frame: the camera picture behind the models, turned upright for the screen's rotation and cropped to fill the view, the session's own viewpoint in place of the scene's camera, and the room's measured depth hiding the parts of the scene behind real things. Start the session with `Features::CAMERA_IMAGE` (and `Features::DEPTH` for occlusion). Without a session the scene draws as before, transparent behind its models.

Android phones without ARCore, such as the Solana Seeker, now show the back camera in `Orientation` sessions too, through Camera2, with the rotation sensor turning the view and the camera's own field of view keeping the scene to the picture's scale. Such a session leaves the picture out, rather than failing, until the user allows the camera, unless it requires `CAMERA_IMAGE`.

On Android a `Scene3d` showing the camera moves below the window's own views, so the interface laid over it stays visible.

For hosts that draw scenes themselves, `leptoskit_3d::host::Hub::set_camera_feed` takes the picture a camera backdrop draws over, `View::set_rotation` the interface's rotation, and the bridge adds `lk_scene3d_view_set_rotation` and `lk_scene3d_view_shows_camera`. `leptoskit_xr`'s new `scene3d` feature makes the plugin's sessions that feed, and `leptoskit build` turns it on, with any other async plugin's `scene3d` feature, when the app shows 3D scenes. `leptoskit_xr::host::install_depth_estimator` (or `XrRuntime::set_depth_estimator`) takes a `DepthEstimator`, which turns small CPU copies of the camera picture (`CameraFrame`, with the camera's `Intrinsics`) into depth for phones that measure none.

#### Immersive sessions from Meta Quest panel apps

_Packages:_ 🟠 _leptoskit_3d_, 🟠 _leptoskit_xr_, 🟠 _leptoskit_bridge_ffi_, 🟠 _leptoskit_cli_, 🟢 _leptoskit_config_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #691](https://github.com/leptoskit/leptoskit/pull/691)

A Quest app now draws its 3D scene into the headset. Start an `ImmersiveVr` or `ImmersiveAr` session and choose the scene with the new `Session::present(Some(scene_id))`: each eye sees the scene from the player's eye, over passthrough in `ImmersiveAr`. An app built with `[native.quest] immersive = false`, a 2D panel, opens an immersive activity over the panel for the session and comes back to the panel in Horizon Home when the session ends, while its code keeps running and receiving frames.

`leptoskit build --platform quest` declares that activity in every Quest app, marks a panel app's launcher with Horizon OS's 2D category and targets Horizon OS v69, and turns on `leptoskit_xr`'s new `quest` feature (or any async plugin's `quest` feature). For hosts that draw scenes themselves, `leptoskit_3d::host::Hub::draw_scene` draws a scene on another device, such as a headset's, and `Hub::forget_device` moves scenes off it again.

None of this has run on a headset yet.

#### Estimated depth for phones that measure none

_Packages:_ 🟠 _leptoskit_xr_, 🟠 _leptoskit_cli_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #704](https://github.com/leptoskit/leptoskit/pull/704)

The `xr` plugin now gives depth to phones whose platform has none: the Solana Seeker and other phones without ARCore, ARCore phones without the Depth API, and iPhones without LiDAR. Put the `leptoskit_depth` model in the app's `assets/` folder and name it:

```toml
[native.plugin-options.xr]
depth-model = "depth/depth_anything_v2_metric_indoor_small.onnx"
```

The native app loads the model in the background when it starts and checks it against `leptoskit_depth::MODEL`; a missing or different file is logged and leaves depth off. Once it has loaded, sessions on those phones offer `Features::DEPTH`: the model reads about one camera picture a second on its own thread, and the newest reading becomes the frame's depth, so a `Scene3d` over the camera hides things behind real furniture and `DEPTH` hit tests reach the room. Phones that measure depth keep their own.

A reading can be a second old, so hit tests cast against it from where the phone was when the picture was taken (`XrFrame::depth_view`), and occlusion turns it to the current view (`XrFrame::depth_transform`, built on `hit_test::view_to_view`). The new `depth-estimation` feature adds `host::ModelEstimator` and `host::install_depth_model`, for hosts that load the model themselves; `leptoskit build` turns it on only for apps that set `depth-model`.

`DepthEstimator::latest` now returns a `DepthEstimate`, the depth with the view and time of the frame it was read from, and `CameraFrame` carries that frame's `view` in place of `pose`.

Plugin crates can declare `option-features` in their runtime metadata, such as `{ depth-model = "depth-estimation" }`: `leptoskit build` turns the feature on when the app sets the option.

Nothing here has run on a phone yet.

#### Distinguish measured and estimated depth

_Packages:_ 🟠 _leptoskit_xr_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #734](https://github.com/leptoskit/leptoskit/pull/734)

`Frame::depth` now carries `Option<DepthSource>`: `None`, `Measured`, or `Estimated`. The host tags depth from the camera model separately from platform depth, including on phones with AR tracking and planes but no Depth API or LiDAR. Model estimates can have a different scale from the tracked floor: find each picture's own floor before scaling it to the measured floor's height.

Depth hit tests report `HitKind::EstimatedDepth` for model estimates and `HitKind::Depth` for measured depth, using the source of the picture the request actually hit. Recording and playback retain the source; existing version-one recordings still load, with depth treated conservatively as estimated because they did not record its source.

Apps that checked `frame.depth` as a boolean now use `frame.depth.is_some()` or match its source. Rebuild native hosts alongside their guests.

#### Augmented and virtual reality, and Meta Quest apps

_Packages:_ 🟠 _leptoskit_xr_, 🟠 _leptoskit_config_, 🟠 _leptoskit_cli_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #659](https://github.com/leptoskit/leptoskit/pull/659)

The new `leptoskit_xr` crate and its `xr` plugin put an app in the real world. Start a session in the best mode the device has — `PhoneAr` through ARKit on iOS and ARCore on Android, `ImmersiveVr` and `ImmersiveAr` on headsets through OpenXR, WebXR in the browser, or `Orientation`, which turns the view with the phone on any device — and read a stream of frames: where the viewer is, each view's projection, the floors, tables, and walls found so far, pinned anchors, the room's light, and controllers and hands. Hit tests find where a tap or a controller's ray meets the real world, against planes and, on phones that measure it, the depth of the room. The camera and depth pictures stay in the native app for the renderer, and the frames your code receives are capped at `[native.plugin-options.xr] max-frame-rate`, always the newest. A recording replays a session exactly, for tests and simulators, with `replay = "…"`.

`leptoskit build --platform quest` builds the Android project as a Meta Quest app, with the manifest Horizon OS expects: OpenXR, head tracking, optional hand tracking and passthrough, the supported headsets, and a full-screen immersive launcher. `[native.quest]` configures it, and `leptoskit dev --platform quest` installs onto a connected headset.

With the `3d` feature, `host::scene3d::draw` draws a `leptoskit_3d` scene into a session: the camera picture behind it, real-world depth hiding what is behind real things, and one view per eye on a headset.

The backends build for their platforms but have not yet run on a device, so treat them as experimental.

#### Depth from the camera picture, for phones that measure none

_Packages:_ 🟠 _leptoskit_depth_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #677](https://github.com/leptoskit/leptoskit/pull/677)

The new `leptoskit_depth` crate reads metric depth from one camera picture on the device, for AR on phones whose platform has no depth: the Solana Seeker and other phones without ARCore, ARCore phones without the Depth API, and iPhones without LiDAR. It runs Depth Anything V2 Small, the authors' fine-tune for rooms, quantised to 27 MB, in pure Rust through tract: about a quarter of a second a picture on one core of an Apple M4, with no native library to ship.

`DepthModel::from_bytes` loads the model and `estimate` reads a `Picture` (RGB, RGBA, BGRA, or NV12 pixels, any quarter turn from upright, with the lens's intrinsics) into a `DepthMap` of metres along the camera's forward axis, laid over the picture as the camera read it, its scale corrected for the lens. `DepthWorker` runs it on its own thread about once a second for an AR runtime that hands over frames and must never wait. The model file is not in the crate: `MODEL` pins it by SHA-256 and says where to fetch it, `MODEL_LICENSE` is its Apache-2.0 licence, and an app ships both in its assets.

The `xr` plugin does not call it yet, and it has not run on a phone.

- **Dual-pane and tabletop layouts for foldables.** Fold posture now carries each hinge's `separating` flag (Android `FoldingFeature.isSeparating`), so a fully open continuous foldable stays one pane. `Posture::is_dual_pane`, `layout_mode`, and `use_posture` expose dual-pane, tabletop, book, and single layouts. `DevicePosture::watch` remains how you react to fold-driven configuration changes while the Android activity stays alive. The guide and Android platform page show concrete dual-pane and tabletop examples. _Packages:_ 🟠 _leptoskit_use_, 🟠 _leptoskit_plugin_posture_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #582](https://github.com/leptoskit/leptoskit/pull/582)
- **Gestures on a view.** A view can follow a finger. `gesture(Gesture::tap())`, `pan`, `long_press`, `pinch`, `rotation`, and `fling` recognize the pointer in one arena shared by the Android widget backend and Linux. `simultaneous`, `exclusive`, and `race` compose recognizers the way a pinch and a rotation share one gesture, or a pan wins over a tap. A drag that wins claims the pointer so the control underneath does not also click. A tap does not, so a button still presses. iOS, macOS, Windows, the web, the terminal, and the generated Swift and Kotlin renderers still deliver presses and clicks only. _Packages:_ 🟠 _leptoskit_native_, 🟠 _leptoskit_android_, 🟠 _leptoskit_gtk_, 🟠 _leptoskit_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #735](https://github.com/leptoskit/leptoskit/pull/735)

#### Camera sessions and Camera surface

_Packages:_ 🟠 _leptoskit_native_, 🟠 _leptoskit_plugin_camera_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #688](https://github.com/leptoskit/leptoskit/pull/688)

`leptoskit_plugin_camera` is a new runtime plugin, `camera`, for host-owned camera sessions. Create or release a session for the back or front lens, set torch when the hardware supports it (otherwise `Unavailable`), take a bounded still into app cache (URI + width/height/mime/size matching `image_picker` Asset framing), and snapshot status. Show the same session with a new `Camera(session_id)` native widget kind that binds a host-owned id the way `Scene3d` and `Video` bind theirs (requires an accessible label or `aria_hidden`).

Ask for camera access through the `permissions` plugin; this plugin returns `PermissionDenied` when capture lacks a grant. iOS and macOS use `AVCaptureSession` / `AVCaptureVideoPreviewLayer`; Android uses CameraX / `PreviewView`; the browser uses `getUserMedia` into HTML `<video>`. Windows, Linux, and HarmonyOS return `Unavailable`. Video recording remains out of scope for this release.

- **Add `leptoskit_plugin_live_photo` with create/start/stop/status/release and a `LivePhoto` surface bound by session id (`PHLivePhotoView`).** Android and browser remain Unavailable; editing remains later scope. _Packages:_ 🟠 _leptoskit_native_, 🟠 _leptoskit_bridge_ffi_, 🟠 _leptoskit_plugin_live_photo_, 🟢 _leptoskit_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #692](https://github.com/leptoskit/leptoskit/pull/692)
- **Add `leptoskit_plugin_maps` with create/camera/markers/status/release and a `Map` surface bound by session id (MapKit / Google Maps).** User location and shapes remain later scope. _Packages:_ 🟠 _leptoskit_native_, 🟠 _leptoskit_bridge_ffi_, 🟠 _leptoskit_plugin_maps_, 🟢 _leptoskit_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #690](https://github.com/leptoskit/leptoskit/pull/690)

#### Screen state and queries survive a rebuild

_Packages:_ 🟠 _leptoskit_native_, 🟠 _leptoskit_query_, 🟠 _leptoskit_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #736](https://github.com/leptoskit/leptoskit/pull/736) · _Closed issues:_ [#721](https://github.com/leptoskit/leptoskit/issues/721)

leptoskit builds a screen again whenever a signal it read changes. Two things broke under that: a `RwSignal::new(..)` inside a screen ran again with every rebuild, so a chosen filter or a typed field reset as soon as the screen redrew, and a `use_query` in a screen refetched in a loop, because the rebuild its data caused mounted the query again while the data already counted as stale.

- `leptoskit_native`: `remember(|| initial)` returns the same `RwSignal` on every build of the screen, told apart by its place in the source and by order. `remember_keyed(&key, || initial)` names the value by a key instead, for rows that move. A value is released once a whole build no longer reaches it, such as after navigating to another route. `guest::mount` keeps the values for native apps. In the browser, `web::screen(move || ..)` replaces `move || web::render(..)` and does the same. `ScreenState` is the store underneath, for hosts of their own. Both functions are in `leptoskit::prelude`, and the module is `leptoskit::remember`.
- `leptoskit_query`: a query mounted within two seconds of this client's last fetch of it takes that fetch's data or error instead of fetching again, like SWR's deduping interval. The window covers `RefetchOn::Always` and `retry_on_mount`, and not invalidation, `refetch()`, focus, reconnect, or intervals. Data from `set_query_data`, initial data, or hydration doesn't count as a fetch. Set it with `with_mount_dedupe` on `QueryDefaults`, `QueryOptions`, or `QueryObserverOptions`; `Duration::ZERO` gives TanStack's behaviour. `QueryOptions`, `QueryDefaults`, and `ResolvedQueryOptions` gain a `mount_dedupe` field, so code that builds them with struct literals needs it too.

An app with its own keyed store and a long stale time to work around this can switch to `remember` and drop both.

- **Ding-inspired toast pills.** Status toasts arrive as a 44px shape, spring open into a pill, morph in place when a newer message arrives, and shrink before they leave — the same phase model and spring tunings as [Ding](https://github.com/YusufCeng1z/ding). Hold a `Toaster`, call `success` / `error` / `warning` / `info` / `loading`, rebuild from `toaster.state()`, and advance phases with `poll`. `ToastViewport` draws the stack from shared builders, so every backend that already draws `View`, `Text`, `Button`, and `Vector` can show a toast. _Packages:_ 🟠 _leptoskit_native_, 🟠 _leptoskit_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #651](https://github.com/leptoskit/leptoskit/pull/651)

#### Video players and Video surface

_Packages:_ 🟠 _leptoskit_native_, 🟠 _leptoskit_plugin_video_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #675](https://github.com/leptoskit/leptoskit/pull/675)

`leptoskit_plugin_video` is a new runtime plugin, `video`, for host-owned URL video players. Create or replace a source, play, pause, seek, set muted/loop/rate, snapshot status, and release the player. Show the same player with a new `Video(player_id)` native widget kind that binds a host-owned id the way `Scene3d` binds a scene (requires an accessible label or `aria_hidden`).

iOS and macOS use `AVPlayer` with an `AVPlayerLayer` surface; Android uses Media3 ExoPlayer with a `PlayerView`; the browser uses an HTML `<video>` element. Windows, Linux, and HarmonyOS return `Unavailable`. Tracks, caching, AirPlay, fullscreen controls, and iOS picture-in-picture enter against an attached player remain out of scope for this release.

- **`leptoskit debug` prints a running app's element tree, runtime errors, and render commits from the devtools hub, and `leptoskit debug open` prints the hub address for the inspector.** `leptoskit mcp` adds `runtime_errors`, `performance`, and `read_logs`, so an agent can see the tree, the errors, the logs, and the commits of an app that `leptoskit dev` is running, and can still capture a screenshot where the platform allows it. `leptoskit device install` installs an Android package, an iOS app, or checks a desktop path. `leptoskit device open` launches it. `leptoskit device sanity` does both and then injects tap, long press, pan, and pinch on the targets that can produce them: every gesture on Android and on an iOS simulator or device, tap and long press and pan on macOS, Windows, and Linux. Linux pinches through `--driver`. A mouse session cannot pinch, and the reply says so. _Packages:_ 🟠 _leptoskit_testing_, 🟠 _leptoskit_cli_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #740](https://github.com/leptoskit/leptoskit/pull/740)
- **Multiple frontend applications in one workspace.** `leptoskit new --frontends landing,app,admin` scaffolds a `leptoskit.workspace.toml` with separate apps under `apps/` (landing and admin web-only; `app` keeps native hosts). Workspace applications may declare an optional `kind` (`landing`, `product`, `admin`, `server`) for tooling. Docs cover the multi-frontend layout; shared auth across frontends remains out of scope. _Packages:_ 🟠 _leptoskit_config_, 🟠 _leptoskit_cli_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #701](https://github.com/leptoskit/leptoskit/pull/701)

#### Versioned native capability contracts

_Packages:_ 🟠 _leptoskit_config_, 🟠 _leptoskit_bridge_ffi_, 🟠 _leptoskit_codepush_, 🟠 _leptoskit_cli_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #630](https://github.com/leptoskit/leptoskit/pull/630)

Native builds now resolve a typed host capability descriptor from the destination flavour, the plugin registrations they actually generated, and the non-secret plugin options. `native bundle` writes `capabilities.json` beside the bundle with a stable SHA-256 digest, and the mobile host embeds the same descriptor in `codepush.json`.

`[native]` gains `required-plugins`, `optional-plugins`, `required-capabilities`, and `optional-capabilities`. A required plugin or capability the selected host cannot run fails the build before packaging, regardless of `missing-plugins`; an optional omission warns with the platform and reason. Existing `plugins` and `missing-plugins` keep their behavior when no requirement lists are set, and a capability declaration never grants access.

Signed releases carry versioned required/optional requirements in `runtime.json`. The installed host checks them against its own descriptor before staging or activating an update; unsupported, unknown, or experimental-without-opt-in requirements are refused. Releases built before this format still pass the legacy ABI, shell, and plugin checks and are reported as legacy, never as verified-compatible. Unknown contract versions and conflicting or duplicate entries are rejected.

Guest requirements bind the bridge protocol as well as the rendering ABI. Explicitly optional plugins no longer enter the legacy required-plugin list. Watch preparation checks requirements before writing its stubs and emits a descriptor; standalone bundles place host evidence beside the output directory.

Updates whose older-style plugin grants have no available implementation report `incomplete` while preserving the existing missing-plugin fallback. Explicit required entries still reject unavailable plugins; explicit optional entries may be omitted. A versioned host descriptor must agree with its own grant list before it can check a release.

Generated iOS and Android update controls show whether host requirements were checked, plugin availability remains incomplete, or only legacy checks ran. A missing report from an older runtime is shown as unreported. None of these states claims that the updated app passed device tests.

The descriptor also names `host.timer` where the generated host enables the reserved `leptoskit.timer` plugin, so `required-capabilities = ["host.timer"]` fails before packaging on hosts that run no guest and is checked by code push on older store builds.

#### Describe independently configured applications

_Packages:_ 🟠 _leptoskit_config_, 🟠 _leptoskit_cli_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #629](https://github.com/leptoskit/leptoskit/pull/629)

Add a separate `leptoskit.workspace.toml` manifest and `lk workspace explain` / `check`. Each application references its own configuration. Startup dependencies are validated independently of call bindings, so bidirectional calls do not become readiness cycles. Unknown references, escaping paths, repeated configurations, conflicting managed identities, and duplicate binding variables fail before a build starts.

The versioned report exposes only selected configuration and environment variable names. It explicitly marks artifacts, API compatibility, database ownership, service identity, and deployment as not checked. Workspace inspection runs no plugin programs or build tools. Image creation, binding injection, change impact, and release orchestration remain future consumers of this graph.

#### Android alarms that run app code

_Packages:_ 🟠 _leptoskit_bridge_ffi_, 🟠 _leptoskit_plugin_android_alarm_manager_, 🟠 _leptoskit_cli_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #627](https://github.com/leptoskit/leptoskit/pull/627)

`leptoskit_plugin_android_alarm_manager` ports Flutter's `android_alarm_manager_plus`. Guests schedule one-shot and repeating Android `AlarmManager` alarms by integer ID, with exact, wakeup, allow-while-idle, alarm-clock and reschedule-on-reboot options, check and request the Android 12+ exact-alarm permission (or fall back to inexact timing), cancel and list alarms, and receive fired alarms through a bounded on-device queue that survives process death: `next()` waits for one, `drain()` takes the ones that fired while the app wasn't running. An alarm marked `headless` starts the app's guest without a window when no guest is running, so `handle_launch` can run app code for it within an eight-second budget. iOS, macOS, Windows, Linux, HarmonyOS and the browser return `Unavailable`.

The plugin declares its alarm and boot receivers with the `android-receivers` runtime metadata, which the host-project docs now describe. The Android host gains `NativeHeadlessGuest`, which runs the bundled guest with no window for a receiver or job and discards what it draws.

#### Android Back goes back a route

_Packages:_ 🟠 _leptoskit_bridge_ffi_, 🟠 _leptoskit_wasm_, 🟢 _leptoskit_cli_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #726](https://github.com/leptoskit/leptoskit/pull/726)

The Back gesture and Back button in an Android app now go back one route through the file router, and leave the app only from the first route. Before, Back always left the app: Android 16 delivers Back to an app that targets it only through an `OnBackInvokedCallback`, which the generated activity never registered, and on every Android version the host's question to the router went nowhere.

- `leptoskit_wasm`: `WasmHost::system_back` offers Back to the guest's optional `leptoskit_system_back` export, then flushes and commits what changed. A guest without the export answers `EventOutcome::Unhandled`, and the platform leaves.
- `leptoskit_bridge_ffi`: `lk_guest_system_back` offers Back on the guest thread, and `lk_guest_take_system_back` takes the answers in order. `LeptosKitApplicationView.offerSystemBack` now takes a callback, `(Boolean) -> Unit`, that runs on the main thread with `true` when the router went back.
- `leptoskit_cli`: the generated `MainActivity.kt` registers an `OnBackInvokedCallback` on Android 13 and later and keeps `onBackPressed` for earlier versions; the generated manifest sets `android:enableOnBackInvokedCallback="true"`.

Apps scaffolded before this keep leaving on Back until they copy the new `MainActivity.kt` Back code and the manifest attribute from a new project; [the Android page](https://leptoskit.dev/docs/platforms/android.html#the-back-gesture) lists them.

#### File-route navigators, deep links, and system Back

_Packages:_ 🟢 _leptoskit_bridge_ffi_, 🟢 _leptoskit_cli_, 🟠 _leptoskit_router_, 🟠 _leptoskit_router_codegen_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #679](https://github.com/leptoskit/leptoskit/pull/679)

Layouts can export `navigator()` returning a typed `Stack`, `Tabs`, or `Drawer`. Folder groups like `(tabs)/` stay URL-less; the navigator return value chooses the chrome. Tab and drawer roots are `impl Into<Route>`, not string names. Stacks can present matched routes as a sheet or modal above the previous screen, and optionally offer an edge hit target for Back.

`Navigation::open_url` / `open_url_with` turn incoming URLs into typed routes (custom schemes and `http(s)`). `apply_activations` drains a host deep-link queue into the router. `bind_system_back` and `leptoskit_system_back` let Android system Back pop router history when the host offers it. Generated Android activities call into that path before finishing.

[`examples/file_routes`](https://github.com/leptoskit/leptoskit/tree/main/examples/file_routes) shows Stack + Tabs + Drawer, deep links, and Back. True UIKit interactive-pop and AppKit history swipe still need host presentation; the guide names those gaps.

- **Add `leptoskit_plugin_contacts` for contacts permission, system pick-one, and a bounded query (max 32) on iOS and Android.** Desktop and browser hosts report unavailable. _Packages:_ 🟠 _leptoskit_plugin_contacts_, 🟢 _leptoskit_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #686](https://github.com/leptoskit/leptoskit/pull/686)
- **Home-screen and desktop widgets.** `leptoskit_plugin_home_widgets` publishes bounded text snapshots for iOS and macOS WidgetKit and Android App Widgets, then asks the OS to reload them. Guests call `guest::HomeWidgets` for capability, set/get/clear, reload, and installed queries. Apps still own their WidgetKit extensions and App Widget providers; Apple needs an App Group for a shared suite, and Windows, Linux, and the browser remain unavailable. _Packages:_ 🟠 _leptoskit_plugin_home_widgets_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #579](https://github.com/leptoskit/leptoskit/pull/579)
- **Cellular carrier and radio snapshot.** `leptoskit_plugin_cellular` reads carrier name, MCC/MNC, radio generation, and VoIP allowance when the platform exposes them. Unknown, Unavailable, and Restricted stay explicit. iOS uses CoreTelephony, Android uses permission-light TelephonyManager fields, and desktop plus the browser answer Unavailable. _Packages:_ 🟠 _leptoskit_plugin_cellular_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #674](https://github.com/leptoskit/leptoskit/pull/674)
- **Electron desktop parity, first wave.** Desktop hosts get Electron-shaped lifecycle APIs without embedding Chromium: `power_monitor` for suspend/resume/lock/AC events and Windows idle queries, `deep_link` for inbound URL/file activations and association descriptors, `updater` hooks with pluggable backends (separate from code push), and second-launch argument handoff on `single_instance`. Docs map Electron concepts to leptoskit plugins and name the remaining gaps. _Packages:_ 🟠 _leptoskit_plugin_deep_link_, 🟠 _leptoskit_plugin_power_monitor_, 🟠 _leptoskit_plugin_single_instance_, 🟠 _leptoskit_plugin_updater_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #580](https://github.com/leptoskit/leptoskit/pull/580)

#### Device and package details, as in device_info_plus

_Packages:_ 🟠 _leptoskit_plugin_device_, 🟠 _leptoskit_plugin_application_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #626](https://github.com/leptoskit/leptoskit/pull/626)

`leptoskit_plugin_device` now reports what Flutter's `device_info_plus` does. Every platform adds a model identifier (`iPhone15,2`, `Mac14,2`), kernel release, available memory, data-volume size and free space, and CPU count, and a new `Snapshot::platform` section carries one platform's own details: Android `Build` fields (SDK and preview SDK level, security patch, fingerprint, `Build.ID`, board, ABI lists, sorted system features, low-RAM class), Apple flags (iOS app on Mac or Vision, Mac Catalyst, OS build, CPU frequency, localized model), Windows release registry values, Linux `os-release` fields, and browser client-hint brands. `web::high_entropy_values()` asks the browser for its detailed hints only when you call it.

Persistent identifiers stay out of the snapshot. The new `Device::identifiers()` call returns the user-assigned device name, the iOS vendor identifier and the desktop machine identifier, each `None` where the platform withholds it; on iOS 16 and later the device name needs Apple's user-assigned-device-name entitlement.

`leptoskit_plugin_application` now reports `package_info_plus`'s remaining fields: the Android signing certificate's SHA-256 digest, the installer store (the installing package on Android; App Store, TestFlight, development or simulator on iOS, guessed from the receipt), and install and update times. `executable_times()` gives Rust hosts the executable's file times.

Both snapshots move to wire version 2. Decoders still read version 1 replies from older native hosts, but a guest built before this release cannot read a version 2 reply, so ship guests built against this version with the updated host. `Snapshot` gained public fields, so code that builds one with a struct literal needs `..Default::default()`.

- **Android device-policy plugin.** `leptoskit_plugin_device_policy` exposes an explicit, least-privilege Device Admin / Device Policy Manager surface for Android: status without prompting, user-driven activation, lock, password quality and minimum length, camera disable, confirmed wipe, and admin removal. Non-Android platforms return `Unavailable`. Apps own the manifest receiver and the `device_admin.xml` policy file so they declare only the policies they need. Wipe requires `WipeRequest::factory_reset()`; empty payloads cannot factory-reset a device. Google Play and enterprise-policy constraints still apply and are documented with the API. _Packages:_ 🟠 _leptoskit_plugin_device_policy_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #578](https://github.com/leptoskit/leptoskit/pull/578)
- **`leptoskit dev` serves phone guests on the local network and prints a QR code for them.** The LeptosKit app scans that code, or takes the address, and runs the unsigned debug guest. Only a local `http` or `https` address is accepted. _Packages:_ 🟠 _leptoskit_plugin_dev_client_, 🟠 _leptoskit_cli_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #727](https://github.com/leptoskit/leptoskit/pull/727)

#### Android intents plugin

_Packages:_ 🟠 _leptoskit_plugin_android_intent_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #623](https://github.com/leptoskit/leptoskit/pull/623)

`leptoskit_plugin_android_intent` ports Flutter's `android_intent_plus`: guests describe an intent with `Intent::new().action(…).data(…).extra(…)` and launch it, open it through a titled chooser, send it as a broadcast, start a service, check whether an activity resolves it, read the package, class, and label it resolves to, or launch an `intent:` URI. Extras cover booleans, ints, longs, doubles, strings, nested bundles, byte arrays, integer and string lists, and boolean, int, long, double, and string arrays.

Only Android registers the plugin; everywhere else `guest::AndroidIntent::new` returns `Unavailable`. Requests travel as a bounded little-endian wire that the Kotlin plugin validates again, because guest code can change over the air: URI-permission grant flags and `file:` URIs are refused with `PermissionDenied`, extras can't carry parcelables, and `intent:` URIs can't name a component or selector and only reach browsable activities. A missing activity or service is `Unavailable` and an Android `SecurityException` is `PermissionDenied`. Resolution on Android 11 and later only sees apps the app's own `<queries>` declares.

#### SMS, mail composer, and dismissible web browser plugins

_Packages:_ 🟢 _leptoskit_plugin_linking_, 🟠 _leptoskit_plugin_sms_, 🟠 _leptoskit_plugin_mail_composer_, 🟠 _leptoskit_plugin_web_browser_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #673](https://github.com/leptoskit/leptoskit/pull/673)

Three new Expo-equivalent runtime plugins cover composer and browsing presentation that were still missing:

- **`sms`** queries composer availability and presents the system SMS sheet with optional recipients and body. Completion is the OS callback only, never delivery. Outbound `sms:` opens stay in `linking`.
- **`mail-composer`** queries availability and presents the system mail sheet with to/cc/bcc/subject/body. Attachments are deferred in this MVP. Outbound `mailto:` opens stay in `linking`.
- **`web-browser`** opens HTTP(S) in Safari View Controller / Custom Tabs and answers on dismiss. It is distinct from `web-auth` (no OAuth return address) and from `linking.open` (which does not wait for dismiss).

Each plugin ships guest clients, wire codecs, Apple/Android backends (with unavailable stubs where a platform has no composer or dismissible sheet), docs, and tests. Desktop hosts mark the plugins unavailable via metadata.

#### iOS Live Activities

_Packages:_ 🟠 _leptoskit_plugin_live_activities_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #577](https://github.com/leptoskit/leptoskit/pull/577)

Apps can start, update, and end iOS Live Activities (Lock Screen and Dynamic Island) through the new `leptoskit_plugin_live_activities` crate and the `live-activities` runtime plugin. Guests call `are_enabled`, `start`, `update`, `end`, `wait_push_token`, and `list`. Content state is a bounded string map that a Widget Extension renders via `LeptosKitLiveActivityAttributes`.

Only iOS implements ActivityKit. Android, macOS, the browser, and the unsupported host stub return `Unavailable` after validating payloads. Apps must set `NSSupportsLiveActivities` in Info.plist, ship a Widget Extension, and enable Push Notifications when requesting push tokens. Custom attribute types, push-to-start tokens, and remote payload helpers are still out of scope.

#### Every transport a connection uses

_Packages:_ 🟠 _leptoskit_plugin_network_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #624](https://github.com/leptoskit/leptoskit/pull/624)

A network reading now lists every transport the active connection uses, as Flutter's `connectivity_plus` does, instead of naming only one. `Snapshot::transports` is a `Transports` set of `Transport` values — Wi-Fi, cellular, ethernet, Bluetooth, VPN, satellite, and other — so an Android app on a VPN over Wi-Fi sees both, and an app on satellite data sees satellite alongside cellular. `Snapshot::connection` still names the primary one, and the set always contains it.

Android reports every transport `NetworkCapabilities` lists, including Wi-Fi Aware as Wi-Fi and satellite on Android 15 and later. iOS and macOS report Wi-Fi, cellular, ethernet, and other from `NWPath`, plus satellite when the path is ultra-constrained (iOS and macOS 26 and later); they have no VPN or Bluetooth interface type, so a VPN tunnel shows as other. The browser maps `NetworkInformation.type`, and desktop Rust still infers no transport.

The wire snapshot grows from 32 to 36 bytes, and every side refuses transport bits it does not define. Rebuild the native app together with the guest; a guest built with this release can't read an older shell's snapshots, and the reverse.

#### Wi-Fi name and local addresses

_Packages:_ 🟠 _leptoskit_plugin_network_info_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #624](https://github.com/leptoskit/leptoskit/pull/624)

The new `leptoskit_plugin_network_info` crate, runtime name `network-info`, ports Flutter's `network_info_plus`. `guest::NetworkInfo::snapshot` returns the Wi-Fi network's name (SSID) and BSSID, and its IPv4 address, IPv6 address, subnet mask, broadcast address, and gateway, for apps that set up or find devices on the local network.

Every field is `None` when the platform withholds it; the plugin never passes on stand-ins such as Android's `<unknown ssid>` or `02:00:00:00:00:00`, and never asks for permission. The SSID and BSSID need location permission, which the `location` plugin requests; on iOS they also need the Access Wi-Fi Information entitlement. An SSID is up to 32 bytes that need not be valid text: `Ssid::as_bytes` returns them as reported, and `Ssid::to_str` returns text when they are UTF-8.

iOS reads `NEHotspotNetwork`, macOS CoreWLAN, and Android `WifiManager` with the Wi-Fi network's `LinkProperties`. Desktop Rust reports the addresses and gateway of the interface the default route leaves through, without a name, and the browser returns `Unavailable`. It is a separate plugin from `network` so an app that only checks connectivity never declares Wi-Fi permissions or hands Wi-Fi identifiers to its guest code.

#### App shortcuts, icon badges, and NFC reads

_Packages:_ 🟠 _leptoskit_plugin_nfc_, 🟠 _leptoskit_plugin_app_shortcuts_, 🟠 _leptoskit_plugin_badge_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #581](https://github.com/leptoskit/leptoskit/pull/581)

Three new runtime plugins cover high-value native capabilities that were still missing:

- **`app-shortcuts`** publishes up to eight static launcher shortcuts (iOS quick actions, Android `ShortcutManager`, macOS dock-menu store) and returns the id that launched the app.
- **`badge`** sets, reads, and clears the app icon badge count on Apple and Android, with a Linux Unity/KDE D-Bus client for custom hosts.
- **`nfc`** queries hardware readiness and reads one bounded NDEF text or URI record on iOS and Android.

Each plugin ships guest clients, wire codecs, Apple/Android backends (with stubs where a platform has no hardware), docs, and tests. Windows jump-list publishing, full Siri App Intents, background NFC, and badge Badging API support remain follow-ups.

- **Add `leptoskit_plugin_print` for HTML and PDF URI print dialogs on iOS, Android, and macOS.** Completion means the dialog finished or the platform accepted a job, never that paper left a printer. _Packages:_ 🟠 _leptoskit_plugin_print_, 🟢 _leptoskit_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #683](https://github.com/leptoskit/leptoskit/pull/683)

#### Add the recorder plugin: continuous microphone segments

_Packages:_ 🟠 _leptoskit_plugin_recorder_, 🟢 _leptoskit_plugin_background_, 🟢 _leptoskit_cli_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #737](https://github.com/leptoskit/leptoskit/pull/737)

Add a `leptoskit_plugin_recorder` crate with a typed binary guest client, Swift and Kotlin handlers, and automatic native registration under the runtime name `recorder`. It records the microphone continuously in the background as timed audio segments: AAC-LC, mono, `.m4a`, rotated at each boundary with no gap between segments, for apps that upload and transcribe each segment themselves.

`start` takes a directory inside the app's own data or cache directories and a segment length (default 300 s, clamped 5–3600 s), sample rate (default 16 kHz), AAC bitrate (default 24 kbps) and activity threshold (default −45 dBFS). `next_segment` is a cancellable wait that returns each finished segment with its path, start time, duration, size, peak level and `active_ms` — the milliseconds of 30 ms frames above the threshold — so the app can skip uploading quiet segments without decoding them. The plugin never deletes a delivered segment and never decides whether one is silent; the app owns both choices. `status` reports idle, recording, interrupted (a call or Siri), silenced (Android: another app holds the microphone) or failed with a reason, plus the open segment's start time.

A small append-only journal beside the segments survives a crash: the next `start` (or a `recover` call) delivers leftover finalised files as `ended_by = recovered` segments and deletes the truncated files that cannot be read, reporting each deletion once. iOS records through an `AVAudioEngine` input tap into an AAC `AVAudioFile`, configures `.playAndRecord` with `.mixWithOthers`, `.allowBluetooth` and `.defaultToSpeaker` so the owner's music and calls keep working, closes the segment on an interruption and resumes when the system allows it, and rebuilds the tap on route changes and after a media services reset; recording can only start in the foreground, and the `audio` background mode keeps it running. Android records `AudioRecord` PCM into a `MediaCodec` AAC encoder and `MediaMuxer` on its own thread, reports `silenced` through the audio recording callback, and requires the `background` plugin's microphone-typed foreground service for background recording — which Android 14 cannot start from `BOOT_COMPLETED`, so the owner must open the app once after a reboot. The `background` plugin now exposes the running service's type mask so `start` can fail with a clear error instead of recording into a process the system will kill.

- **Add `leptoskit_plugin_device_motion` for fused attitude, user acceleration, and rotation-rate readings on iOS, Android, and the browser.** Desktop hosts report unavailable. _Packages:_ 🟠 _leptoskit_plugin_device_motion_, 🟢 _leptoskit_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #684](https://github.com/leptoskit/leptoskit/pull/684)
- **Add `leptoskit_plugin_age_range` for declared age-range capability status and threshold request (Declined / Unavailable / Shared bounds).** Android, desktop, and browser return Unavailable in this MVP; live OS prompts remain later scope. _Packages:_ 🟠 _leptoskit_plugin_age_range_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #697](https://github.com/leptoskit/leptoskit/pull/697)
- **Add `leptoskit_plugin_app_integrity` for attestation capability status and bounded challenge→token requests.** Hosts return Unavailable in this MVP until App Attest / Play Integrity wiring lands. _Packages:_ 🟠 _leptoskit_plugin_app_integrity_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #698](https://github.com/leptoskit/leptoskit/pull/698)
- **Add `leptoskit_plugin_asset` for packaged-asset capability status and name→URI resolve.** Hosts return Unavailable in this MVP until packaged-asset ownership wiring lands. _Packages:_ 🟠 _leptoskit_plugin_asset_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #699](https://github.com/leptoskit/leptoskit/pull/699)
- **Add `leptoskit_plugin_diagnostics` for start/end span and event recording.** Hosts return Unavailable in this MVP until a diagnostics backend is chosen. _Packages:_ 🟠 _leptoskit_plugin_diagnostics_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #700](https://github.com/leptoskit/leptoskit/pull/700)
- **Add `leptoskit_plugin_blob` for bounded create/read/release of owned byte blobs.** Hosts return Unavailable in this MVP until a durable resource protocol lands. _Packages:_ 🟠 _leptoskit_plugin_blob_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #700](https://github.com/leptoskit/leptoskit/pull/700)
- **Native capability parity: badges and shortcut hosts.** Browser pages can set and clear app-icon badges through the Badging API (`web::set` / `web::clear`), and `web::get` returns the last count this adapter published. Generated iOS, Android, and macOS hosts now call `noteLaunch` when the OS activates an app shortcut: Android dynamic shortcuts carry a unique intent per id, the macOS host builds the dock menu from `DockShortcutStore`, and iOS notes home-screen quick actions. Docs match the real backends for menus, badges, and shortcuts. _Packages:_ 🟠 _leptoskit_plugin_app_shortcuts_, 🟠 _leptoskit_plugin_badge_, 🟢 _leptoskit_cli_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #628](https://github.com/leptoskit/leptoskit/pull/628)

#### Sound playback

_Packages:_ 🟠 _leptoskit_plugin_audio_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #656](https://github.com/leptoskit/leptoskit/pull/656)

`leptoskit_plugin_audio` is a new runtime plugin, `audio`, for music and sound effects. Load a sound from the app's assets, its data or cache folder, or bytes, and play it as many voices as you need: looping without a gap, fading in, ramping volume, panning, through two low-pass or high-pass filters whose cutoff glides, paused and resumed where it stopped, and stopped with a fade. The master bus has the same volume, filters, and pause, so a soundtrack can be veiled, held while the app is in the background, or silenced at once, and `crossfade` brings one piece in as another falls away. `events()` reports voices that played to their end and audio-session interruptions.

iOS, Android, and macOS share one Rust engine: symphonia decodes Ogg Vorbis, WAV, MP3, FLAC, and AAC; music streams a second ahead instead of sitting in memory decoded; a lock-free mixer runs in the device callback; and cpal plays through Core Audio and AAudio. `configure` picks an ambient or playback session: the `AVAudioSession` category on iOS, and audio focus on Android. Browser apps use `web::Audio` over Web Audio, which starts on the page's first tap or key press. HarmonyOS has no output yet, Android does not report losing audio focus, and native builds don't copy `assets/` into the app yet.

- **Add `leptoskit_plugin_background_task` for named OS background-task register/unregister/status/complete (`BGTaskScheduler` / `WorkManager`), distinct from `leptoskit_plugin_background` AppService keep-alive.** Guaranteed cadence and suspended Wasm execution remain later scope. _Packages:_ 🟠 _leptoskit_plugin_background_task_, 🟢 _leptoskit_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #696](https://github.com/leptoskit/leptoskit/pull/696)
- **Add `leptoskit_plugin_calendar` for calendar permission, listing calendars (max 64), and creating or reading bounded events on iOS and Android.** Desktop and browser hosts report unavailable. _Packages:_ 🟠 _leptoskit_plugin_calendar_, 🟢 _leptoskit_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #687](https://github.com/leptoskit/leptoskit/pull/687)
- **Library image and video picker.** `leptoskit_plugin_image_picker` picks one photo-library image or video with cancellation, an exclusive busy slot, and a bounded temporary URI plus width/height/mime/size metadata. Camera capture stays for a later `camera` plugin. Apple uses PHPicker; Android uses the activity-result broker; the browser returns a `blob:` URI from a trusted file input. _Packages:_ 🟠 _leptoskit_plugin_image_picker_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #674](https://github.com/leptoskit/leptoskit/pull/674)
- **Mobile picture-in-picture and system overlays.** Apps can keep a call or video-like surface visible while another app is open. The new `picture-in-picture` runtime plugin reports capabilities and the active session, enters Android activity picture-in-picture as a cancellable lease, and — after the user grants `SYSTEM_ALERT_WINDOW` in system settings — holds an Android system overlay window. Overlay status and `request_overlay` open the grant screen without inventing a permission. iOS reports whether system PiP is supported but does not yet enter it without an attached video player, and has no draw-over-apps overlay. Desktop and the browser return `Unavailable`. _Packages:_ 🟠 _leptoskit_plugin_picture_in_picture_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #576](https://github.com/leptoskit/leptoskit/pull/576)
- **Builder-friendly native menus.** `leptoskit_plugin_menu` adds chained constructors (`Menu`, `MenuBar`, `Submenu`, `Command`, `Check`, `Separator`) that build the same validated definitions hosts already attach. On macOS, `ApplicationMenu::install` builds and leases the AppKit menu bar in one step; Windows gains the matching `WindowMenu::install` helper. Linux still has no standalone menu bar and exposes `linux::attach_menu_bar` as an explicit `Unavailable` stub; tray / status-item menus keep sharing `DesktopMenu` with `leptoskit_plugin_tray`. Docs cover the macOS path and the Windows/Linux parity gaps. _Packages:_ 🟠 _leptoskit_plugin_menu_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #575](https://github.com/leptoskit/leptoskit/pull/575)
- **Add `leptoskit_plugin_video_thumbnails` to extract one JPEG frame from an app-owned video URI at a time offset (`AVAssetImageGenerator` / `MediaMetadataRetriever`).** Multi-frame extraction remains later scope. _Packages:_ 🟠 _leptoskit_plugin_video_thumbnails_, 🟢 _leptoskit_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #694](https://github.com/leptoskit/leptoskit/pull/694)

#### Share files, subject and title, and where it went

_Packages:_ 🟠 _leptoskit_plugin_sharing_, 🟠 _leptoskit_cli_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #625](https://github.com/leptoskit/leptoskit/pull/625)

`leptoskit_plugin_sharing` now covers Flutter `share_plus`'s features. A share can carry up to 32 files, optionally with text: `File::app("exports/report.pdf")` names a file in your app's files directory (the root the `filesystem` plugin writes), and `File::bytes("chart.png", bytes)` shares data from memory, each with an optional MIME type and display name. `with_subject` sets the email subject, `with_title` the sheet or chooser title, `with_anchor` where the iPad popover or macOS picker points, and `excluding` leaves iOS activities out of the sheet.

`share` now returns a `ShareResult`: `Success` with the destination the user picked (the iOS activity type, the macOS service title, or the Android component), `Dismissed`, or `Unavailable` when the platform cannot tell, as in browsers. A result never confirms delivery. On Android this replaces the old behaviour, where dismissing the chooser and choosing an app looked the same.

Native hosts check that an app file stays inside the files directory, refusing symlinks that lead out of it, and copy shared files into a private cache that the next file share clears. On Android the plugin ships its own `FileProvider` and grants read access only to the chosen app, only for those copies; it adds `androidx.core:core`. Browsers share in-memory files through Web Share Level 2 and return `Unavailable`, without a download fallback, when they cannot. Windows and Linux still have no native share sheet.

Breaking: the wire protocol is versioned and changed. The previous request (a kind and one value) and its empty reply are refused, `share` returns `ShareResult` instead of `()`, `web::share` resolves to a `ShareResult` (a browser cancellation is `Dismissed`, no longer a `Cancelled` error, and an iOS or macOS dismissal is `Dismissed` rather than a `Cancelled` error), and `Content::as_str` and `Content::is_url` are replaced by `shared_text`, `shared_url`, `shared_files`, `subject` and `title`.

The CLI reads a new `android-file-providers` plugin metadata key: it copies the plugin's paths XML into the generated resources and declares an unexported `FileProvider` in the generated Android manifest.

- **Add `leptoskit_plugin_media_library` for bounded photo-library permission, asset listing, and saving an app-owned file into the system library.** Change observation remains later scope. _Packages:_ 🟠 _leptoskit_plugin_media_library_, 🟢 _leptoskit_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #689](https://github.com/leptoskit/leptoskit/pull/689)

#### Preserve null, empty collections and omitted arguments in generated SDK requests.

_Packages:_ 🟠 _leptoskit_rpc_codegen_, 🟢 _leptoskit_rpc_, 🟢 _leptoskit_cli_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #669](https://github.com/leptoskit/leptoskit/pull/669)

Query and form encoders now match the server's serde_qs 1.1 protocol: explicit null is a bare key and an empty collection is `key=`. JSON and live encoders keep explicit null, and required nullable arguments in Go and Rust keep their keys. Python optional arguments use the exported `UNSET` for omission and `None` for explicit null. Generated Rust and CLI packages need `serde_qs` 1.1.3; update an existing package's manifest when regenerating its code.

Clients, OpenAPI, AsyncAPI and compatibility checks now follow each argument's `required` flag rather than inferring it from nullability. Specs from `#[api]` functions mark `Option` arguments optional, because the server reads a missing key as `None`. The spec format is now version 4; older specs still load, and their nullable arguments stay optional. A spec written by this version needs an equally new CLI to generate clients.

In regenerated Dart clients, `Option` arguments become optional `Presence<T?>` parameters instead of required `T?` ones. Omit them, or pass `Presence.present(value)`.

Strict diagnostics report a Go or Rust optional nullable argument only when its schema declares a default, and no longer report a Dart query/form limitation that the encoder now handles. Mandatory SDK acceptance decodes generated HTTP requests with the server codec.

#### Server-rendered queries hydrate without a mismatch

_Packages:_ 🟠 _leptoskit_rpc_, 🟠 _leptoskit_query_, 🟢 _leptoskit_cli_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #756](https://github.com/leptoskit/leptoskit/pull/756) · _Closed issues:_ [#722](https://github.com/leptoskit/leptoskit/issues/722)

A page that rendered a query on the server now hydrates as the same page in the browser. Before, only a query's data was carried, so a query that failed on the server started from "loading" in the browser, leptos reported a hydration mismatch, and the page stopped updating.

- `QueryScope::with_ssr_errors` (and `with_error_codec`) streams a server render's error alongside the data, so the browser shows the same error first. RPC queries and paginated queries opt in, and `FnError` is now `Serialize` and `Deserialize`.
- A query seeded from the server doesn't fetch while the page hydrates. Its mount refetch — stale data in the background, or a retry after an error — starts once hydration has finished, so the first render matches the server's HTML.
- On the server, reading a query that is still loading inside `<Suspense>` holds the boundary until it settles, as a leptos resource does, so the server renders the outcome it streams.
- Streamed payloads escape `<`, `>`, `&`, and U+2028/U+2029, so data and errors can't close the `<script>` they're embedded in.

Apps using `render = "ssr"` should read queries inside `<Suspense>`; outside a boundary the server renders whatever the query holds at that moment.

New apps put their web adapter inside that boundary, including the database and sign-in scaffolds. Existing portable apps should wrap the closure that calls `leptoskit::web::render` in the same way.

Generated browser entrypoints configure the RPC and auth clients inside the mount callback, after leptos has set up its executor and owner, so the live socket starts on the first page load. A query observer reads `enabled` without subscribing while it binds or runs its refetch interval, so an `enabled` closure that reads another query no longer logs a reactive-tracking warning on every page.

#### A fallible wait for hosts without a timer

_Packages:_ 🟠 _leptoskit_query_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #609](https://github.com/leptoskit/leptoskit/pull/609)

`leptoskit_query::try_sleep` reports `SleepError::Unavailable` or `SleepError::Rejected` instead of pretending a wait elapsed. Generated sign-in screens use it for a rate-limit countdown, so on a host with no usable timer the button becomes a manual retry, the server's "try again in …" message stays visible, and the screen never claims time that did not pass. The server remains the authority on when the window opens.

This is the compatibility surface for the native guest timer work. When that timer lands, its host-backed `try_sleep` implementation replaces the fallback here without changing the screen's behavior.

#### Service workers and stronger extension messaging

_Packages:_ 🟠 _leptoskit_extension_, 🟠 _leptoskit_service_worker_, 🟠 _leptoskit_, 🟢 _leptoskit_cli_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #584](https://github.com/leptoskit/leptoskit/pull/584)

Web apps can enable the `service-worker` feature to register a worker script, exchange messages, describe caching strategies, open Cache Storage, and manage Web Push subscriptions. Calls return `Unavailable` outside a secure browser context.

Browser extensions gain long-lived `runtime::connect` ports (with `Background::on_connect`), storage `clear` / `keys` / `get_bytes_in_use`, and permissions `get_all` / `on_added` / `on_removed`. The generated background worker keeps ports alive so the wasm side can post and disconnect by id.

- **New database and sign-in apps start with a real notes screen.** A `leptoskit new --db sqlite`, `--db postgres`, or `--auth` project now opens on a notes screen instead of the counter. It uses the shared typed `Notes` and `AddNote` operations the scaffold already publishes: loading, empty, failed, and loaded states, body validation before the mutation, save feedback, and a Refresh button. The browser follows the live socket; native apps use the HTTP plugin and Refresh because their generated hosts don't grant the WebSocket plugin, and the Linux, Windows, and terminal hosts say plainly that they can't reach the server. With `--auth`, the screen is rebuilt per account and the previous account's cached notes are dropped first, so signing out or signing in as someone else never shows the previous person's list. `--db` now also grants the native HTTP plugin and configures its origin. Projects without a database keep the counter. _Packages:_ 🟠 _leptoskit_cli_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #609](https://github.com/leptoskit/leptoskit/pull/609)
- **`leptoskit dev` rebuilds an edit with a line-table dev profile, compiles the server and the wasm client in parallel when the edit needs both, and copies assets without invoking cargo.** `leptoskit budgets` runs the caps and boot checks and names the build, test, lint, and workspace commands. _Packages:_ 🟠 _leptoskit_cli_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #770](https://github.com/leptoskit/leptoskit/pull/770)
- **New apps use the file router by default.** `leptoskit new` enables the `router` plugin, scaffolds `index.rs` / `_layout.rs` / `+not-found.rs` under the app crate's `src/routes/`, commits a generated `routes.rs`, and depends on `leptoskit_router`. Builder `[lint]` stays the default so the starter `Page` and `Layout` components stay clean under `leptoskit lint --deny-warnings`. _Packages:_ 🟠 _leptoskit_cli_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #702](https://github.com/leptoskit/leptoskit/pull/702)
- **`leptoskit new` opens on a home screen, a kit of the shared controls, and a list of the platforms the app runs on, in a tab bar.** The home screen still counts with **Add one**. _Packages:_ 🟠 _leptoskit_cli_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #727](https://github.com/leptoskit/leptoskit/pull/727)

#### Release independently built workspace applications

_Packages:_ 🟠 _leptoskit_cli_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #629](https://github.com/leptoskit/leptoskit/pull/629)

Add `workspace plan` and `workspace deploy` for a set of prebuilt, independently pinned image descriptors. Plans freeze the workspace, parsed app configurations, artifact pins, readiness order, and credential variable references. Execution stages exact descriptors, invokes the existing managed release checks in each app directory, and retains a durable progress journal when a later app fails.

Add `deploy --reuse-current` to revalidate a current verified release without booting it again. Reuse checks current authorization, descriptor identity, recorded configuration, database history, image identity, running-container membership, effective app environment, and `/readyz`. Older history without configuration evidence requires a new version. Changed credentials fail reuse rather than being silently ignored. Successful release operation state is now written after backup installation and pruning finish.

Workspace retries revalidate every selected app, including previously completed entries; the journal never authorizes reuse. Automatic builds, source-change impact, private networking, caller identity, and cross-application compatibility remain separate work.

### Fixes

- **Bound analytics batch decoding and extreme clocks.** Analytics ingestion now limits the memory used when decoding binary batches, including repeated dictionary references, before allocating strings or collections. Extreme client clock offsets are clamped instead of overflowing timestamp arithmetic. The client core is also included in the regular Wasm lint checks. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #779](https://github.com/leptoskit/leptoskit/pull/779)
- **Server rendering can spawn tasks.** `leptoskit::server::serve_listener_with`, and with it `serve_listener`, `serve_router`, and `serve_router_with`, now makes tokio the global `any_spawner` executor when it starts serving, unless the app already chose one. Before, a server-rendered page that used `leptoskit::query`, a leptos resource, or anything else that spawns a task panicked with "tried to spawn a Future with Executor::spawn() before a global executor was initialized", unless the app knew to call `Executor::init_tokio()` itself. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #720](https://github.com/leptoskit/leptoskit/pull/720)
- **Reject future backup timestamps when reporting health.** Managed backup status no longer reports a verification timestamp in the future as healthy. A successful restore check must have a nonnegative age within the configured freshness window, and any later recorded backup failure still makes the status unhealthy. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #621](https://github.com/leptoskit/leptoskit/pull/621)

#### Install valid scheduled backup units

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #621](https://github.com/leptoskit/leptoskit/pull/621)

Managed backups now generate separate systemd directives for the recovery command, writable state paths, and timer installation. Previously, malformed newline escapes joined these settings to preceding lines and could prevent scheduled backups from running. The host verifies both generated units before initializing the backup repository or enabling the timer.

After upgrading, run `lk infra backups install` for existing managed deployments to replace their generated units.

#### Consolidation preserves kept migrations

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #586](https://github.com/leptoskit/leptoskit/pull/586)

`db migration consolidate --keep` now folds only the schema changes before the kept migrations. Previously, it included their changes in the consolidated migration as well, so applying the resulting history could try to add the same column twice. Keeping every draft leaves history unchanged, and an oversized keep list reports an error instead of panicking.

- **Fix the Apple device-keys signing failure path to bridge a retained Core Foundation error explicitly before encoding its status.** This avoids a Swift compiler diagnostic failure when building native hosts that use device keys. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #807](https://github.com/leptoskit/leptoskit/pull/807)
- **Scripts for your own docs components.** `leptoskit_docs::Site::script` adds a script from the book to every page, loaded with `defer` after the built-in one, for components that need behaviour. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #590](https://github.com/leptoskit/leptoskit/pull/590)

#### Explicit backend selection and readiness probes

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #569](https://github.com/leptoskit/leptoskit/pull/569)

`build.backend` explicitly selects an API backend or a static app. Build, serve and deployment generation honor it; CSR apps no longer need a separate client package to deploy a backend. Without an explicit choice, Cargo binary targets determine backend presence.

`serve.probes` declares the `/healthz` and `/readyz` lifecycle instead of guessing from words in Rust source. New projects enable it. Existing managed apps must implement the readiness routes and set `serve.probes = true` before deploying. The declaration is checked by deployment health probes at runtime; it is not evidence that a server is healthy.

- **Browser extension port events compile for Wasm.** Give JavaScript object property reads a distinct internal name from serialized namespace reads, so extension port message and disconnect listeners compile alongside the developer-tools API. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #609](https://github.com/leptoskit/leptoskit/pull/609)
- **Merge accounts in apps with the identity provider.** An app that links `leptoskit_idp` can now merge two accounts and bring the other account's data along. Before, `idp.grants` declared no `on_account_merge` strategy, so `lk db check` failed and "bring its data" merges were refused. A merge now moves the absorbed account's grants to the account that stays. If both accounts granted the same app, the surviving account keeps its own grant and the absorbed account's grant is removed along with that account. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #719](https://github.com/leptoskit/leptoskit/pull/719)
- **Job workers wake when the change listener starts.** Starting job workers now wakes them once their database change listener is ready. A job enqueued between a worker's first queue scan and listener startup no longer waits for the next polling interval. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #572](https://github.com/leptoskit/leptoskit/pull/572)
- **Workers claim only the jobs they can run.** A server's job workers now take only jobs its own build declares. Before, during a rolling deploy, an old server could pick up a job the new build had just added, fail it with "no job is named …", and spend one of its attempts each time, until it gave up for good. Now the job waits for a server that knows it. A job no running build declares any more stays pending, where `leptoskit jobs list --state pending` shows it and `leptoskit jobs cancel` clears it. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #564](https://github.com/leptoskit/leptoskit/pull/564)

#### Kubernetes manifests that start

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #562](https://github.com/leptoskit/leptoskit/pull/562)

`leptoskit generate deploy --target kubernetes` now writes manifests a cluster accepts and runs:

- **Names:** Deployments, Services, Secrets, claims, and labels use the app's name with `_` as `-`, so an app called `my_app` gets `my-app`, a valid Kubernetes and DNS name. The image keeps its name.
- **The user:** each pod runs as the image's `app` user by number (`runAsUser`, `runAsGroup`, and `fsGroup` 10001). Before, the kubelet refused to start every container, because `runAsNonRoot` can't check a user named only by name. Volumes are now writable to that user on any storage class. The generated `Dockerfile` creates the `app` group with gid 10001 itself, rather than leaving the number to `useradd`.
- **Services without an app database:** a service with `database = true` in an app that has no `[db]` gets its SQLite file on a volume of its own under Compose and Kubernetes, with one replica, as it does when the app is on SQLite. Before, the file went on the read-only root filesystem, and Kubernetes gave the service no `DATABASE_URL`.

The analytics example's server now answers `/healthz` and `/readyz`, which every generated health check calls.

- **The live example on leptoskit.dev answers sooner.** The landing page's live example loaded the whole hosted app's Wasm client, management console included, before its button worked. The console now loads as a lazy chunk of its own, the client builds with `optimize = "smallest"`, and the page starts the client, its glue, and the Rive runtime downloading together instead of one after another. The example's module drops from 370 KiB to 188 KiB gzip-compressed, and on a phone emulated with Lighthouse's slow 4G and 4x CPU slowdown its first press updates the count about half as long after the example scrolls into view. The hosted console loads the same module and then its chunk, and says the app could not load if the chunk never arrives. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #557](https://github.com/leptoskit/leptoskit/pull/557)
- **The observatory and postcard examples open on Linux.** `cargo run -p observatory_example` and `cargo run -p postcard_example` now open a real GTK 4 window on Linux, as they already did with AppKit on macOS. Their `snapshot` command renders the window with GTK's own renderer to a PNG. Elsewhere they still print the screen's accessibility tree instead. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #660](https://github.com/leptoskit/leptoskit/pull/660)
- **Commonplace and Huddle run on Linux.** The Commonplace and Huddle examples now target Linux too. `leptoskit build --platform linux` builds each one's GTK app from its new `linux/` runner. The apps reach their server through the HTTP plugin and keep the session in the desktop's keyring, and Huddle's chat goes live over the WebSocket plugin. Along with file_routes, observatory, postcard, and the gallery, every example with a native app now runs on Linux. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #665](https://github.com/leptoskit/leptoskit/pull/665)
- **Live updates resume when a slow reader catches up.** A live connection retains its unfinished transition and resumes it when transport capacity becomes available, even without another database write. Later changes coalesce into one current snapshot. Large transitions continue from their remaining chunks, and removed or replaced subscriptions do not receive unsent results from their earlier registration. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #611](https://github.com/leptoskit/leptoskit/pull/611)
- **Keep pending live query results bounded.** The live hub keeps one pending batch per connection instead of retaining every database round while a session is busy. Repeated changes retain the latest complete value or error for each subscription, including identity checks that must happen before delivery. Removing or replacing a subscription also removes its queued result. Live queries still deliver current state, not an event for every intermediate write. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #622](https://github.com/leptoskit/leptoskit/pull/622)

#### Bound pending live action and stream replies

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #622](https://github.com/leptoskit/leptoskit/pull/622)

Live actions keep their admission slot until the session handles their reply, so a busy connection cannot accumulate unlimited completed actions. Stream producers wait for bounded output capacity even when a client renews credits early; ordered items and terminal errors are retained. An outgoing stream frame larger than 1 MiB ends that stream with a 413 error.

Stream flushing during reauthentication and server drain also respects the connection's stall timeout.

Existing generated and handwritten stream pumps keep using the synchronous `StreamSink::fail` helper. The session waits for the recorded terminal error before releasing the producer task.

- **Live recovery retains current query errors.** When a live query fails, a full recovery transition now repeats that failure instead of replacing it with an earlier successful value. Reauthentication failures also replace the stored result before any recovery snapshot is produced. A subsequent successful query restores the current value. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #611](https://github.com/leptoskit/leptoskit/pull/611)

#### Isolate reused live stream IDs

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #622](https://github.com/leptoskit/leptoskit/pull/622)

Cancelling or replacing a stream now discards its buffered items and terminal frame, so they cannot reach a later stream with the same client ID. Streams retain their admission slot until their terminal frame is consumed. Handwritten producers that return or panic also send a terminal response; panic details stay on the server.

Consuming a terminal frame also cancels its producer, so a handwritten producer cannot keep running after its stream is retired.

Dropping a session also cancels its owned stream producers, including when its task is cancelled before normal cleanup.

- **Finish split updates before advancing the resume cursor.** A live transition that needs several frames now advances its version only on the final frame. Reconnecting after an earlier frame replays the unfinished changes, and mutations continue waiting until the complete transition arrives. The frame format is unchanged. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #611](https://github.com/leptoskit/leptoskit/pull/611)

#### Deploy an app that lives inside a Cargo workspace

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #601](https://github.com/leptoskit/leptoskit/pull/601)

`leptoskit generate deploy` now finds the server and client packages through Cargo's workspace metadata, so an app folder with only `leptoskit.toml` and member crates works, including members matched by workspace globs. The generated container uses the Cargo workspace as its build context, builds from the app directory, and copies the workspace target binary, the app's `dist/`, and the nearest workspace or app toolchain file. Compose, Fly, Kubernetes, and managed infrastructure use the same context and `Dockerfile` path.

Inside a workspace, the container ignore policy is written as `Dockerfile.dockerignore` beside the app's `Dockerfile`, so Docker applies it to the workspace context instead of ignoring the app-root `.dockerignore`. It excludes nested build caches, databases, environment files, managed infrastructure plans and state, and Kamal secrets, keeps exclusions already in the workspace and app `.dockerignore` files, and never rewrites those files. Standalone apps keep their existing files and commands, custom generated files stay yours, and a path that would escape the workspace is refused before it becomes a build context.

Nested recipes also accept `LEPTOSKIT_CLI_PATH` as an explicit build argument for a CLI crate inside the workspace, and normalize the container target directory independently of host build settings.

- **Publish the complete platform dependency set.** Home widgets and the experimental watch crates now participate in the release manifest. Watch packages release in lockstep with the toolkit, so published feature combinations can resolve their dependencies. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #592](https://github.com/leptoskit/leptoskit/pull/592)

#### Keep platform integrations consistent

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #592](https://github.com/leptoskit/leptoskit/pull/592)

New platform plugins and the service-worker crate now have shared summary sources for their README and Rust documentation. Formatting and generated documentation are synchronized after the plugin integrations.

Android plugin sources follow the repository's Kotlin formatting rules. Experimental Wear OS template classes now match their filenames; they remain host stubs.

- **Idle time on Windows.** The power monitor plugin builds on Windows again. It read the time since the last keyboard or mouse input through the wrong `windows-sys` module, so apps that used it failed to compile for Windows. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #667](https://github.com/leptoskit/leptoskit/pull/667)

#### Keep shared release history readable

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #654](https://github.com/leptoskit/leptoskit/pull/654)

Managed deployment now checks the complete proposed release history against its 4 MiB read limit before booting applications or changing their database. Large verified descriptors and repeated evidence can otherwise make a successful deployment's history unreadable by the next release or recovery command. The final write is checked again; existing history is never silently truncated or discarded.

Pulled and running image references must contain exactly 64 lowercase hexadecimal SHA-256 characters after the configured repository name. A matching repository prefix alone no longer qualifies a malformed reference as a registry digest.

This guard prevents new unreadable history. It does not archive existing history, repair an already oversized file, or provide unlimited release retention.

- **Check the complete published RPC contract.** `rpc check` now detects changes to existing argument types and requiredness, referenced schemas, channel payloads and join keys, live paths, and error encodings. Current exclusions cannot erase operations from the previous published contract. Missing references and ambiguous operation names require review. The shared `ApiSpec::breaking_changes` comparison is available without the code generators feature for artifact tooling; it checks declarations and does not claim runtime or authorization compatibility. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #608](https://github.com/leptoskit/leptoskit/pull/608)
- **Require generated client acceptance before publication.** Every supported SDK now runs its live-server journey on pull requests and before publication. Explicitly selected SDK tests fail when their language toolchain is missing, rather than reporting success without executing. Run the same serial acceptance suite locally with `devenv shell test:sdk`; the development environment includes its toolchains. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #598](https://github.com/leptoskit/leptoskit/pull/598)
- **Resume generated live clients from applied results.** TypeScript, Python, Dart, Go, Rust, and CLI clients keep their applied cursor when a server welcomes them. A reconnect interrupted before its recovered results arrive now resumes from the previous complete transition, and pending mutations cannot use the welcome's server version as evidence that their results are on screen. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #611](https://github.com/leptoskit/leptoskit/pull/611)

#### Entrances let go once they have played

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #573](https://github.com/leptoskit/leptoskit/pull/573)

On the web, a motion that plays from a pose to the element's own style, whether it enters, scrolls into view (`in_view`), or follows a timeline, now lets go of the element once it has played instead of holding its last keyframe. A reveal that clips from an inset therefore no longer keeps clipping what the element paints outside its box, such as a heading's descenders. Keyframes that end on a pose of their own, at 1.0, still hold it.

The leptoskit.dev landing page and documentation get the same care:

- Section titles show their descenders, set display headings at a looser leading from shared tokens, and wrap each line of a title evenly.
- The hero's fan of screens no longer covers the copy on tablets and small laptops.
- The widget map fits a phone with reduced motion.
- The documentation's hero mark lands smoothly, and stays still for people who ask for less motion.
- The documentation's page titles no longer widen the page on a tablet, and its hero title keeps each sentence whole on a phone.
- Small coral text and the "Get started" button meet WCAG AA contrast in light and dark.

- **Bundled SQLite is 3.54.1.** `leptoskit_db`'s `sqlite` feature, `leptoskit_db_schema`, `leptoskit_deploy_console`, and `leptoskit_plugin_sqlite` bundle SQLite through `rusqlite`, which moves from 0.40.2 to 0.41.1 so that `libsqlite3-sys` moves from 0.38.2 to 0.39.1 and the bundled SQLite from 3.53.2 to 3.54.1. Before 3.54.0, SQLite could return uninitialized page-cache memory when a process opened a database alongside a `-wal` file written with a different page size (CVE-2026-119375, RUSTSEC-2026-0459). Nothing in leptoskit's own API changed; `rusqlite` stays an implementation detail. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #725](https://github.com/leptoskit/leptoskit/pull/725)
- **SQLite live queries hear other processes' writes.** On SQLite, a live query now updates within a quarter of a second when another process writes to the same database file, such as a second server process. Before, it heard only its own process's writes, and saw another process's writes only once it wrote something itself. Each database file gets one connection that checks for changes four times a second, and only once something subscribes to changes. Writes from tools that aren't leptoskit apps, such as the `sqlite3` shell, still reach subscribers with the app's next write. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #564](https://github.com/leptoskit/leptoskit/pull/564)
- **Honor the selected application's frontend lint rules, exclusions and Clippy setting when linting.** Lint's partial configuration loader now follows `--app-target` and `app.default_target`, while still ignoring unrelated unfinished settings. Server-only targets keep the root lint configuration. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #808](https://github.com/leptoskit/leptoskit/pull/808)
- **Package reproduced backends and retain image evidence.** Add an opt-in backend-only image packaging command that reproduces independently pinned bundles, copies the verified executable without application compilation, checks extracted image bytes and configuration, and writes an operator-pinned descriptor. Managed releases retain this evidence across rollback and recovery. Website and service-bearing bundles remain unsupported. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #568](https://github.com/leptoskit/leptoskit/pull/568)
- **Verified backends can use infrastructure services.** Verified backend packaging now distinguishes application services from the separate cache and relay containers configured under `infra.services`. Those infrastructure settings no longer incorrectly reject a backend-only image. Additional application executables still require their own evidence and remain unsupported by this packaging profile. The descriptor verifies the backend image; it does not attest the infrastructure containers or their runtime configuration. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #592](https://github.com/leptoskit/leptoskit/pull/592)
- **Bind supported client contracts to verified releases.** `infra build-verified` can compare the reproduced executable's RPC export with independently pinned published contracts. Version 3 descriptors retain the exact specs and recompute the structural comparison when consumed. Version 2 operator trust files require a nonempty set of supported contract pins across deployment, reuse, rollback, and recovery; publishers cannot substitute their own baseline. Legacy evidence remains readable and explicitly unchecked. This checks declared wire shapes, not runtime behavior or authorization compatibility. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #608](https://github.com/leptoskit/leptoskit/pull/608)

#### Bind RPC evidence to the verified backend

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #568](https://github.com/leptoskit/leptoskit/pull/568)

Export RPC specifications from the checked executable under container confinement and hash the exact exported bytes. Record compatibility as not checked; require an explicit operator declaration for backends without RPC. Keep legacy releases from replacing verified history.

Hash executable bytes directly, preserving differences between non-UTF-8 bytes, and normalize Podman image IDs before recording descriptors.

Validate descriptor digests, pinned recipes and platform consistency before release. Reject history that discards verified evidence, and stop applications when automatic recovery fails verification.

- **Web builds ship the app's own Wasm client.** `leptoskit build` bound the first Wasm module cargo reported. When the client crate depended on another crate that also declares a `cdylib`, cargo compiled that dependency to Wasm first, and the build shipped the dependency's module under the app's name: the page loaded without an error, but the app's start function never ran. Web builds and native guest builds now take the module of the client package itself. This is what kept the live example on leptoskit.dev from responding to presses. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #556](https://github.com/leptoskit/leptoskit/pull/556)

#### Smaller server-rendered pages

_Packages:_ 🟢 _leptoskit_testing_library_, 🟢 _leptoskit_native_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #741](https://github.com/leptoskit/leptoskit/pull/741)

Server-rendered and static pages no longer repeat leptoskit's style reset for every differently styled element. Each element's typed styles start from about sixty declarations, such as `display:flex`, `flex-shrink:0`, and zeroed margins and padding, and every distinct look carried its own copy of all of them, about a kilobyte each. `leptoskit::web::render` now writes those defaults once per stylesheet, and each style scope's sheet holds only what differs from them, often a single declaration. Elements look the same: the defaults keep the specificity and order they had, below utility classes. A list of two dozen rows that differ only in width used to carry 27 copies of the reset and now carries one, and its markup is under 9 KB. Style scopes are also 16 hex digits instead of 64, since every element carries its scope.

The root element of each render now carries `data-lk-root`, which keeps one render's defaults from resetting the elements of a render earlier in the page. `leptoskit_testing_library` leaves it out of the markup it prints, as it does `data-lk-style`.

- **Build the public and hosted website clients with a size-optimized Rust standard library during release packaging.** This keeps the initial download within the existing 180 KiB budget while preserving the live example and the console's lazy loading. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #734](https://github.com/leptoskit/leptoskit/pull/734)
- **Transparent scenes show the page behind them in the browser.** A scene with `Backdrop::Transparent` drew over solid black in the browser. wgpu lists only `Opaque` as a WebGPU canvas's alpha mode, though every browser canvas also takes `premultiplied`, so the canvas was set up opaque. Browser canvases now composite premultiplied, as native surfaces already did. A 3D view over a photo, a camera picture, or another 3D view shows them through its empty pixels. WebGL2, where wgpu refuses anything but opaque, is unchanged. _Packages:_ 🟢 _leptoskit_3d_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #715](https://github.com/leptoskit/leptoskit/pull/715)
- **Keep Android builds warning-free: omit the unused stderr fallback on Logcat, simplify pointer decisions, and account for Clippy's Android TLS false positive while retaining const initialization.** _Packages:_ 🟢 _leptoskit_game_, 🟢 _leptoskit_native_, 🟢 _leptoskit_android_, 🟢 _leptoskit_bridge_ffi_, 🟢 _leptoskit_log_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #734](https://github.com/leptoskit/leptoskit/pull/734)
- **Release the game host with the SDK.** The optional `leptoskit_game` crate joins the SDK's coordinated release group, so published apps using the `game` feature can resolve its matching version. _Packages:_ 🟢 _leptoskit_game_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #734](https://github.com/leptoskit/leptoskit/pull/734)

#### Plugin options the app would crash on stop the build instead

_Packages:_ 🟢 _leptoskit_xr_, 🟢 _leptoskit_bridge_ffi_, 🟢 _leptoskit_plugin_http_, 🟢 _leptoskit_plugin_websocket_, 🟢 _leptoskit_plugin_shell_, 🟢 _leptoskit_cli_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #729](https://github.com/leptoskit/leptoskit/pull/729)

An Android app whose `leptoskit.toml` had an empty `[native.plugin-options.websocket]` built, installed, and then stopped at launch with `NativeApplicationInitializationFailure: register plugin: InvalidArgument`. The WebSocket plugin can't start without `origins`, the build only checked that the table was there, and the error didn't say which plugin had failed.

`leptoskit build` now stops with the plugin and the option it needs:

```
plugin `websocket` needs `origins` in [native.plugin-options.websocket]: without it, the app stops at launch when the plugin starts
```

An async plugin lists the options its factory can't start without in its runtime metadata, and an empty string, list, or table counts as left out:

```toml
[package.metadata.leptoskit.runtime]
name = "websocket"
rust-factory = "from_config"
rust-async = true
required-options = ["origins"]
```

`http` and `websocket` require `origins`, `shell` requires `commands`, and `xr` requires nothing. Each crate's tests hold its factory to its list.

When a plugin still fails to start, the error names it: ``register plugin `websocket`: InvalidArgument``. Generated registrants install every plugin through the new `App::register`, and a host directory that can't be used is reported as `app context: …` rather than as a plugin's failure.

The `shell` plugin is now left out of iOS, Android, and HarmonyOS builds, where it can't start processes and its factory always failed, which also stopped the app at launch.

#### Native props read the same whatever serde_json's features

_Packages:_ 🟢 _leptoskit_animation_, 🟢 _leptoskit_native_, 🟢 _leptoskit_wasm_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #730](https://github.com/leptoskit/leptoskit/pull/730)

An Android app that also used Solana's clients drew nothing: its first vector failed in the Wasm host with `invalid type: map, expected f32`. Those clients turn on serde_json's `arbitrary_precision` for the whole build, and under it a buffered number with a fraction is a map. The host read a node's props through `#[serde(flatten)]`, which buffers every value, and so did each animation `Keyframe`.

The host now reads a node with `WireNode::from_json`: the props and the keys beside them in two passes, with nothing buffered, and `Keyframe` lists its look's `opacity` and `transform` itself. Its JSON is unchanged.

The guest and the host now share `leptoskit_native::wire`: one table of kind codes and one `WireNode`. That fixes two more ways the sides disagreed:

- The host's own kind table stopped at `Camera`, so a `Map` or a `LivePhoto` showed as a plain view.
- An image's `source` was taken by the DevTools location, which used the same key, so native images had no source. A `DevTools` guest now sends its location as `source_location`.

`test:wire` runs the guest-to-host tests under `arbitrary_precision` too.

- **Hydrated web screens refresh their stylesheet when classes change.** Switching a theme or opening a panel could introduce new style scopes while leaving the initial CSS in place, so controls lost their layout and colours. The browser regression hydrates server HTML and changes padding in both directions. _Packages:_ 🟢 _leptoskit_native_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #785](https://github.com/leptoskit/leptoskit/pull/785)

#### Regions committed together keep their own widgets

_Packages:_ 🟢 _leptoskit_native_, 🟢 _leptoskit_wasm_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #733](https://github.com/leptoskit/leptoskit/pull/733) · _Related issues:_ [#2](https://github.com/leptoskit/leptoskit/issues/2)

`NativeSurface::try_patch` applies patches in tree order, and it answered in that order too, though its contract (and the wasm host, which pairs each answer with the patch it sent) is the order given. A commit that changed two regions whose wire ids ran against the tree, such as a label above a list that gained a row, paired one region's wire ids with the other's widgets. A debug host stopped with "surface and wire tree diverged"; a release host could send a widget's events to another node. On a Solana Seeker, buying anything in Reapers stopped the app.

`try_patch` now answers in the order given. The wasm host's incremental tests check, after every commit, that each widget pairs with the wire node it was built from.

- **The web launch screen lifts once the app is drawn.** The HTML splash that `[app.launch_screen]` injects into `index.html` used to stay over the app for good: nothing called its `leptoskitDismissSplash()`. The web renderer now fades it out once the app's first render is painted, as iOS and Android end theirs at the first frame. An app that calls `leptoskit::splash::hold()` keeps it until `leptoskit::splash::dismiss()`, which now lifts it on the web too. _Packages:_ 🟢 _leptoskit_native_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #717](https://github.com/leptoskit/leptoskit/pull/717)
- **`leptoskit device open` fails when the app never takes the screen, instead of reporting success while the launcher is still in front.** A tap, long press, or pan past the display fails too: `input` exits 0 for those points and delivers nothing. An Android pinch still waits for its touchscreen to attach, but that wait is no longer part of the gesture clock, so a 300 ms pinch is 300 ms of contact with both pointers. `device list` marks an AVD as running when that emulator is already up. `leptoskit mcp http` answers `application/json`, and the screenshot tool's schema resolves. _Packages:_ 🟢 _leptoskit_testing_, 🟢 _leptoskit_cli_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #751](https://github.com/leptoskit/leptoskit/pull/751)
- **An Android pinch from `leptoskit device` is two pointers again.** Two `input swipe`s each own one pointer, and on API 35 the activity received only one of them while the step still reported success. The pinch now registers one touchscreen with `uinput` and moves both fingers in the same report. _Packages:_ 🟢 _leptoskit_testing_, 🟢 _leptoskit_cli_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #745](https://github.com/leptoskit/leptoskit/pull/745)

#### Boot iOS for sanity checks and use a mouse on macOS

_Packages:_ 🟢 _leptoskit_testing_, 🟢 _leptoskit_cli_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #747](https://github.com/leptoskit/leptoskit/pull/747)

`leptoskit device sanity` on an iOS simulator now boots that simulator before `simctl install`. A shut-down simulator refuses the install. One that is already booted stays up: the boot step treats that state as success. A sanity run that does not install, and a run on a real iPhone, still do not boot.

Pointer actions sent to a macOS session use a mouse. Mac2 does not accept a touch pointer. iOS, Android, Linux, Windows, and the web still send touch.

- **Connected iPhones show up in `device list`.** `leptoskit device list` asked `devicectl` for `--json` and parsed a `devices` array at the top of that output. devicectl does not print JSON that way. It writes a file for `--json-output`, and the phones are under `result.devices`. The id an install, a launch, and an XCUITest session need is the hardware UDID, not the Core Device identifier. A plugged-in iPhone never appeared. Watches and simulators in the same file stay out of the iPhone list. _Packages:_ 🟢 _leptoskit_testing_, 🟢 _leptoskit_cli_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #746](https://github.com/leptoskit/leptoskit/pull/746)

#### Linux pointer gestures from `leptoskit device` no longer pause for about fifteen seconds when the pointer is already on the point.

`xdotool mousemove --sync` waits for a motion event that never arrives in that case, so a tap followed by a long press or a pan at the same point stalled. The move is sent without that wait.

_Packages:_ 🟢 _leptoskit_testing_, 🟢 _leptoskit_cli_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #744](https://github.com/leptoskit/leptoskit/pull/744)

Opening a Linux app also waits until one of its windows is visible, the process exits, or two seconds pass. Gestures used to be injected the moment the process was spawned, which is before the window is mapped, so the tap and the long press landed on the desktop.

Opening an Android app waits until that package is the focused window, then a little longer, or two seconds pass. `am start` was returning while the launcher was still focused, and the focus line itself updates before the activity accepts a tap, so the sanity tap and long press never reached the activity.

`leptoskit device boot` of an AVD that is already running fails immediately and names the emulator serial, instead of starting a second emulator and waiting out the boot deadline. If the emulator process exits before the device is online, the boot fails then, instead of blocking on `adb wait-for-device`.

- **An unknown or custom widget is an empty container on Android, in both the backend and the generated app.** It used to stop the host, which closed the app. GTK already drew one as an empty view. _Packages:_ 🟢 _leptoskit_android_, 🟢 _leptoskit_bridge_ffi_, 🟢 _leptoskit_cli_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #727](https://github.com/leptoskit/leptoskit/pull/727)
- **Harden native-wave plugin correctness.** Live Activities on iOS cancel push-token waits without freezing the per-id busy lock, replace duplicate ActivityKit ids on start, and reject duplicate content keys. Picture-in-picture leases best-effort exit system PiP and observe user exit; `request_overlay` docs match launch-time status. Home widgets stop reporting unrelated pins for unknown kinds, commit preferences before success, and reject dangerous URL schemes. Device-policy and Live Activities stubs validate payloads before Unavailable. Wear/watchOS hosts return errors instead of panicking on viewport kind mismatch. New Android hosts declare `supportsPictureInPicture` on MainActivity. _Packages:_ 🟢 _leptoskit_watchos_, 🟢 _leptoskit_wearos_, 🟢 _leptoskit_plugin_home_widgets_, 🟢 _leptoskit_plugin_device_policy_, 🟢 _leptoskit_plugin_live_activities_, 🟢 _leptoskit_plugin_picture_in_picture_, 🟢 _leptoskit_cli_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #618](https://github.com/leptoskit/leptoskit/pull/618)
- **More ergonomic multi-frontend scaffold.** `--frontends` alone scaffolds landing, app, and admin. Ports are stable per kind. The product app under `apps/app` uses the workspace name as its Cargo package. Frontend names trim whitespace and accept ASCII case; `ApplicationKind` gains `FromStr`, `Display`, and helpers used by the scaffold. Each frontend app uses the same file-router defaults as a normal `leptoskit new`. _Packages:_ 🟢 _leptoskit_config_, 🟢 _leptoskit_cli_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #703](https://github.com/leptoskit/leptoskit/pull/703)

#### Accept managed registries with explicit ports

_Packages:_ 🟢 _leptoskit_config_, 🟢 _leptoskit_cli_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #629](https://github.com/leptoskit/leptoskit/pull/629)

Managed image configuration accepts a registry hostname with a numeric port and an image repository under that registry. Invalid ports, embedded credentials, paths in registry authorities, and tagged image repositories remain rejected.

Verified image packaging accepts `--service <infra.name>` to include Kamal’s required service label in the independently pinned image configuration. Reusing an image checks the label along with the rest of that configuration.

Deployment accepts Docker’s null defaults for absent Podman `Cmd`, `OnBuild`, and `Volumes` inspection fields while still requiring the exact content-addressed image ID. Non-null changes remain rejected.

- **Give the HarmonyOS host's line-height values named, explicit types so generated applications compile with the public API 12 ArkTS compiler.** _Packages:_ 🟢 _leptoskit_platform_harmonyos_, 🟢 _leptoskit_cli_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #734](https://github.com/leptoskit/leptoskit/pull/734)
- **A drawing-scaled vector stroke follows the layout frame, not the view after it has been snapped out to whole pixels.** At 420 dpi a 10dp edge becomes 27px, and the stroke was a little wider than the layout asked for. `parse_and_launch` no longer opens an `intent:` URI that has no scheme. API 35's Settings activity is browsable, so an action-only Settings URI used to start it. The activity it does open still has to declare `CATEGORY_BROWSABLE` on the filter that matched. _Packages:_ 🟢 _leptoskit_bridge_ffi_, 🟢 _leptoskit_plugin_android_intent_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #739](https://github.com/leptoskit/leptoskit/pull/739)
- **Composite Android 3D scenes with the interface.** Android `Scene3d` views draw into transparent `TextureView` surfaces in the normal view hierarchy. Page backgrounds no longer cover the models, and controls remain above the scene and its camera picture. Changing between the camera and a transparent backdrop no longer removes and recreates the view's GPU surface. Rust releases the native window before Android releases its surface texture. _Packages:_ 🟢 _leptoskit_bridge_ffi_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #734](https://github.com/leptoskit/leptoskit/pull/734)

#### Android text wraps to the lines it needs

_Packages:_ 🟢 _leptoskit_bridge_ffi_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #728](https://github.com/leptoskit/leptoskit/pull/728)

Android measured any plain one-paragraph text with `BoringLayout`, which always lays out exactly one line, even when the text was wider than its box. Layout then gave a paragraph one line's height, and the `TextView` drew only its first line: every paragraph of Reapers on a Solana Seeker was clipped. Text now takes `BoringLayout` only when it fits, and wraps through `StaticLayout` otherwise, as `TextView` does. The narrowest box a text can take is its widest word, with one word to a line.

Text views now also break lines with the measurer's strategy (simple, no hyphenation) instead of Android's balanced default, so a view never needs a line more than its frame was measured to.

- **Expose the Apple development-session callback from the shared bridge so the development-client plugin can compile independently of the app host.** _Packages:_ 🟢 _leptoskit_bridge_ffi_, 🟢 _leptoskit_plugin_dev_client_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #734](https://github.com/leptoskit/leptoskit/pull/734)
- **Generated Apple apps follow light and dark.** The generated macOS and iOS apps now tell your screens when the system switches between light and dark, so `dark:` classes update without a resize. A screen that first lays out while the system is dark also starts dark; before, it stayed light until its window changed size, which a menu-bar app or a fixed-size window never does. _Packages:_ 🟢 _leptoskit_bridge_ffi_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #596](https://github.com/leptoskit/leptoskit/pull/596)

#### Apple plugin and JNI builds on current toolchains

_Packages:_ 🟢 _leptoskit_bridge_ffi_, 🟢 _leptoskit_plugin_contacts_, 🟢 _leptoskit_plugin_device_keys_, 🟢 _leptoskit_plugin_live_activities_, 🟢 _leptoskit_plugin_live_photo_, 🟢 _leptoskit_plugin_nfc_, 🟢 _leptoskit_plugin_print_, 🟢 _leptoskit_plugin_age_range_, 🟢 _leptoskit_plugin_app_integrity_, 🟢 _leptoskit_plugin_asset_, 🟢 _leptoskit_plugin_diagnostics_, 🟢 _leptoskit_plugin_blob_, 🟢 _leptoskit_plugin_background_task_, 🟢 _leptoskit_plugin_calendar_, 🟢 _leptoskit_plugin_camera_, 🟢 _leptoskit_plugin_picture_in_picture_, 🟢 _leptoskit_plugin_video_thumbnails_, 🟢 _leptoskit_plugin_sms_, 🟢 _leptoskit_plugin_mail_composer_, 🟢 _leptoskit_plugin_maps_, 🟢 _leptoskit_plugin_media_library_, 🟢 _leptoskit_plugin_web_browser_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #718](https://github.com/leptoskit/leptoskit/pull/718)

The Apple plugin packages now build and test against the current Xcode SDK. Wire readers no longer declare a `bytes` method that shadows their storage, wire-case helpers have distinct names, calendar validation is shared, EventKit events with no identifier fail honestly, camera photo delegates are retained behind a lock, Security keys use the CF bridge, print uses the current AppKit HTML and PDF APIs, and ActivityKit code is iOS-only. Delegate conformances are main-actor isolated, and the tests import internal wire types with `@testable` and supply a real presentation owner.

The JNI smoke test now finds the JDK through `JAVA_HOME`, invokes the macOS SDK compiler through `xcrun`, and stubs the 3D, timer, and DevTools entry points that the application bridge links.

#### Native scenes load models on the first launch

_Packages:_ 🟢 _leptoskit_bridge_ffi_, 🟢 _leptoskit_router_codegen_, 🟢 _leptoskit_cli_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #734](https://github.com/leptoskit/leptoskit/pull/734)

Android, iOS and macOS install the scene asset reader before starting the guest. Models queued before the first native scene view exists now load from the app bundle on a cold launch.

Generated Axum route lists also return their owned paths directly, keeping generated apps free of redundant conversion warnings.

#### Handed-back calls reach the Rust plugin that serves them

_Packages:_ 🟢 _leptoskit_bridge_ffi_, 🟢 _leptoskit_wasm_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #732](https://github.com/leptoskit/leptoskit/pull/732)

An `AppService` such as Solana Kit's wallet service drains the bridge itself and hands back every call it does not own. Those calls skipped the interpreter's timer and went to the platform, which has no timer plugin: on Android every guest `sleep` failed as `Unavailable` in any app with such a service, so nothing that waited on a timer ever ran. A handed-back call now passes the interpreter first, as a queued one always did.

`WasmHost::serve` and `WasmHost::served_plugins` name what the interpreter answers itself, and `App::rust_plugins` lists every plugin answered in Rust: the timer, the frame clock, inline, worker and async plugins. Tests on the guest thread's own step find every registered plugin by id and fail when a call for a Rust plugin reaches the platform's router, so a Rust plugin added later is held to the same rule.

#### A stopped Android app says why, readably

_Packages:_ 🟢 _leptoskit_bridge_ffi_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #731](https://github.com/leptoskit/leptoskit/pull/731)

When an Android app's guest stopped, leptoskit replaced it with a plain `TextView` holding the error. That view took the theme's text color, which in a light system theme is dark, so on an app with a dark background the message was nearly invisible: Reapers looked like a black screen on a Solana Seeker. An app whose plugins failed to start didn't get even that: its activity closed at launch.

Both now show `NativeFailureView`: white text on near-black (17:1), centred and selectable, covering the whole app, the same in either theme. An app that cannot start shows it instead of closing.

- **Android plugins compile against the public SDK again.** Picture-in-picture listens through AndroidX, which is the type the platform stubs actually ship, and still notices a plain activity leaving PiP when it resumes. The camera plugin passes the failure itself to `PluginException`. Device keys read the SPKI bytes from `PublicKey` instead of `ECKey`, which has none. Calendar, contacts, print, SMS, mail, the web browser, and the media library set a string slice's byte order on the `ByteBuffer` `slice()` returns, because Android's `limit` returns `Buffer`. A calendar event reserves room for both text lengths as well as the timestamps, and decoding a contact advances the caller's buffer so the next record in a list can be read. _Packages:_ 🟢 _leptoskit_plugin_contacts_, 🟢 _leptoskit_plugin_device_keys_, 🟢 _leptoskit_plugin_print_, 🟢 _leptoskit_plugin_calendar_, 🟢 _leptoskit_plugin_camera_, 🟢 _leptoskit_plugin_picture_in_picture_, 🟢 _leptoskit_plugin_sms_, 🟢 _leptoskit_plugin_mail_composer_, 🟢 _leptoskit_plugin_media_library_, 🟢 _leptoskit_plugin_web_browser_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #738](https://github.com/leptoskit/leptoskit/pull/738)
- **P3 plugin quality: fix background_task Apple Auto construction and honesty, asset Kotlin NUL parity, age_range docs, and tighter wire tests.** _Packages:_ 🟢 _leptoskit_plugin_age_range_, 🟢 _leptoskit_plugin_app_integrity_, 🟢 _leptoskit_plugin_asset_, 🟢 _leptoskit_plugin_background_task_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #706](https://github.com/leptoskit/leptoskit/pull/706)
- **Blob and diagnostics MVPs now run in process memory / no-op allocators with STATUS probing, instead of Unavailable-only stubs.** _Packages:_ 🟢 _leptoskit_plugin_diagnostics_, 🟢 _leptoskit_plugin_blob_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #705](https://github.com/leptoskit/leptoskit/pull/705)
- **Harden keyboard shortcuts and menu accelerators.** Global shortcuts and menu items now share the same muda `CmdOrCtrl+…` accelerator bounds and conflict rules: chords are capped at 64 printable ASCII bytes, menus reject duplicate accelerators (including equivalent spellings after parse), and dropping a global-shortcut owner retries native unregister so failed RAII cleanup does not leave OS bindings behind. Closing the Apple or Android app-shortcuts plugin clears published launcher entries so they cannot outlive the bridge. _Packages:_ 🟢 _leptoskit_plugin_app_shortcuts_, 🟢 _leptoskit_plugin_menu_, 🟢 _leptoskit_plugin_global_shortcut_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #619](https://github.com/leptoskit/leptoskit/pull/619)
- **Clarify that buffered browser sounds report the decoded AudioContext sample rate, and verify their length and playback controls at both 44.1 kHz and 48 kHz.** _Packages:_ 🟢 _leptoskit_plugin_audio_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #734](https://github.com/leptoskit/leptoskit/pull/734)
- **Android picture-in-picture builds against the public SDK.** Android PiP no longer calls `ActivityInfo.supportsPictureInPicture()`, which is `@hide` and missing from the public SDK stubs (so the generated app failed to compile). Capability checks use the system PiP feature plus an activity context; hosts that omit `android:supportsPictureInPicture` still fail closed when entering PiP. The PiP-mode listener imports `android.app.PictureInPictureModeChangedInfo` instead of the non-existent `android.util` type. _Packages:_ 🟢 _leptoskit_plugin_picture_in_picture_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #710](https://github.com/leptoskit/leptoskit/pull/710)
- **Harden desktop menu bars.** `leptoskit_plugin_menu` now rejects duplicate accelerator chords before native allocation, exposes `DesktopMenu::set_accelerator` and `definition()`, and shares a `root_items_are_submenus` check so Windows `WindowMenu` leases match the macOS / `MenuBar` root-submenu rule. macOS attach refuses off-thread calls; Windows `translate_accelerator` stays safe when muda has no HACCEL. Docs name the remaining generated-host gaps honestly. _Packages:_ 🟢 _leptoskit_plugin_menu_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #617](https://github.com/leptoskit/leptoskit/pull/617)
- **No `service` cfg warning in apps that call a service.** `#[rpc::query(service = "...")]`, and the other attributes that take `service`, no longer make Cargo warn `` unexpected `cfg` condition value: `service` `` in a crate that calls a service without declaring a `service` feature, such as an app. The attributes read whether the crate declares the feature: a service's own crate keeps the body behind it as before, and every other crate gets the typed stub directly, which is what it got before. _Packages:_ 🟢 _leptoskit_rpc_macros_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #752](https://github.com/leptoskit/leptoskit/pull/752)
- **Stop live queries when their screen goes away.** Live query hooks now unsubscribe when their reactive owner is cleaned up or their arguments change. Buffered updates are cancelled too, so a removed screen cannot repopulate the query cache after an account change. _Packages:_ 🟢 _leptoskit_rpc_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #609](https://github.com/leptoskit/leptoskit/pull/609)

#### Retries stop when a host cannot schedule the wait

_Packages:_ 🟢 _leptoskit_rpc_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #609](https://github.com/leptoskit/leptoskit/pull/609)

A keyed HTTP mutation and `call_with_retry` now surface the last network error when the host has no usable timer, instead of waiting forever on a backoff that can never finish. This keeps a native app's Save button from staying on "Saving…" when its host predates the guest timer plugin or rejects the timer.

A live subscription created before the socket opens is sent once: the handshake records which connection delivered it, so the command that woke the connection no longer sends a duplicate `Sub` frame.

- **Keep the file router's SSR module and Axum dependencies on native targets, so enabling all features in browser and Wasm guest builds does not pull in native networking.** _Packages:_ 🟢 _leptoskit_router_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #734](https://github.com/leptoskit/leptoskit/pull/734)
- **SSR apps now register every file route with `leptos_axum`, and the server render reads the request URL instead of always opening on Home.** _Packages:_ 🟢 _leptoskit_router_, 🟢 _leptoskit_cli_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #743](https://github.com/leptoskit/leptoskit/pull/743)
- **Generated routes also compile in private client modules.** The generated `axum_route_list()` helper is optional in client-rendered apps. An unused helper inside a private routes module no longer fails a warnings-denied build. Its documentation names the Axum method without requiring the app to depend directly on `leptos_axum`, so client-only documentation builds work too. SSR hosts can still call the same helper. _Packages:_ 🟢 _leptoskit_router_codegen_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #755](https://github.com/leptoskit/leptoskit/pull/755)
- **Generate SSR paths without redundant conversions.** Generated `axum_route_list` entries now use the `String` that `Pattern::axum_path` returns directly. The redundant conversion made generated SSR routes fail clippy when warnings were denied. _Packages:_ 🟢 _leptoskit_router_codegen_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #752](https://github.com/leptoskit/leptoskit/pull/752)

#### Refuse ambiguous consolidated rollback without panicking

_Packages:_ 🟢 _leptoskit_db_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #621](https://github.com/leptoskit/leptoskit/pull/621)

Rollback now returns a migration-history error when an adopted consolidation leaves more applied history rows than the current registry contains. It stops before reverse SQL or history changes instead of indexing beyond the registry. Ordinary rollback of an intact registry prefix is unchanged.

- **Native sign-out clears the session query when offline.** `sign_out_mutation()` now clears a native app's session query when it forgets its local tokens, including when the server is unreachable or refuses the request. The mutation still reports that error: local sign-out does not claim the server revoked the session. A browser keeps its session query on failure because its cookie remains until the server removes it. _Packages:_ 🟢 _leptoskit_auth_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #754](https://github.com/leptoskit/leptoskit/pull/754)
- **`leptoskit native build` for Android reports the APK size from the app's own `android/` project.** It was still looking under `hosts/android`, so a build that produced an APK said there was nothing to measure. _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #748](https://github.com/leptoskit/leptoskit/pull/748)
- **Keep Android plugins' native libraries on the JNI link line when Cargo prints coloured output, including AAudio for audio plugins.** _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #734](https://github.com/leptoskit/leptoskit/pull/734)

#### Android and Apple hosts ship every view they draw

_Packages:_ 🟢 _leptoskit_cli_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #711](https://github.com/leptoskit/leptoskit/pull/711)

Generated Android projects now compile: the platform inventories include `NativeCamera.kt`, `NativeMap.kt`, `NativeLivePhoto.kt`, and `NativeServiceWakeBroker.kt`, so `NativeWidgetRenderer` and `LeptosKitView` resolve the views they draw and the background wake they install. The macOS and iOS inventories likewise include `NativeCamera.swift`, `NativeMap.swift`, and `NativeLivePhoto.swift`, which their widget renderers already used.

A test now fails when a shipped Kotlin or Swift source refers to a type declared only in a file the platform inventory leaves out, so this class of missing source is caught before Gradle or Xcode sees it.

#### Android hosts link the libraries their plugins need

_Packages:_ 🟢 _leptoskit_cli_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #716](https://github.com/leptoskit/leptoskit/pull/716)

Generated Android and Quest projects now link. The host's `CMakeLists.txt` names the Rust archive and a fixed list of NDK libraries, and a static archive carries none of Cargo's link directives, so a plugin whose Rust code uses a platform library outside that list failed at `ld.lld` with undefined symbols — `AAudio_createStreamBuilder` and its neighbours once the audio plugin was selected.

The Android host build now asks rustc for the libraries the archive needs (`cargo rustc -- --print native-static-libs`) and writes them into the generated link line, in rustc's order and without repeats, so any plugin's libraries (`aaudio`, `OpenSLES`, `mediandk`, `camera2ndk`, `EGL`, `GLESv3`, `vulkan`, …) arrive with the plugin rather than with a list the CLI has to keep in step.

- **Keep the Android API level in native C and C++ builds when a compiler cache wraps the NDK launcher.** _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #734](https://github.com/leptoskit/leptoskit/pull/734)
- **Reading a watched file does not restart the app.** `leptoskit dev` ignores filesystem access events before scheduling a rebuild or marking the configuration, database tables, or Git branch as changed. On Linux, the compiler and CLI reading their inputs could otherwise trigger another build repeatedly, even without an edit. Source edits and file replacements still rebuild the app. _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #774](https://github.com/leptoskit/leptoskit/pull/774)
- **Validate the dev address before watching the workspace.** `leptoskit dev` rejects an invalid web serve address before inspecting Cargo metadata, starting a database, or watching files. Watching still begins before the initial build, so edits made during that build trigger a rebuild. _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #734](https://github.com/leptoskit/leptoskit/pull/734)
- **`lk dev` keeps edits made while it starts.** `leptoskit dev` now starts watching your project before its first build rather than after every platform has started. An edit saved while that first build ran, which takes minutes on a fresh checkout, used to go unseen: the server came up on the code from before the edit and stayed on it until you saved again. The edit now waits in the watcher's queue, and `dev` rebuilds and restarts the server as soon as the platforms are up. The `watching … for changes` line still prints once everything has started. _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #761](https://github.com/leptoskit/leptoskit/pull/761)
- **Image builds retry their tool download.** The Dockerfile that `leptoskit deploy` generates for web apps downloads binaryen's `wasm-opt` from GitHub. It now retries that download up to five times and unpacks it only once it is complete, so a brief GitHub outage no longer fails the image build. _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #668](https://github.com/leptoskit/leptoskit/pull/668)
- **Builder lint keeps the SSR shell; apps pin Rust.** `markup = "builder"` no longer reports ordinary HTML tags in the SSR document `shell` (`<html>`, `<head>`, …)—only leptoskit component trees. The agent skill and linting guide say the same. Deploy generation names `--sdk-path` when a path dependency sits outside the workspace. (New apps already pin `rust-toolchain.toml` from the matching scaffold work.) _Packages:_ 🟢 _leptoskit_cli_, 🟢 _leptoskit_lint_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #650](https://github.com/leptoskit/leptoskit/pull/650)

#### Generated sign-in effects run in native apps

_Packages:_ 🟢 _leptoskit_cli_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #609](https://github.com/leptoskit/leptoskit/pull/609)

`leptoskit new --auth` and `leptoskit generate auth` now use isomorphic effects for the sign-in screen's mount step, the provider-notice step, and the post-sign-in navigation. `Effect::new` is compiled out in a native guest (the generated app does not enable leptos's `effects` feature), so the screen could stay on "Checking whether you're signed in…" forever. The effects still skip server rendering, so the first server HTML matches the browser's first render.

The generated notes journey is now exercised end to end in the real interpreter: the optimized guest loads and refreshes notes over the HTTP plugin, validates the draft, shows pending/saved/failed saves, signs in through the development mailbox, switches accounts without showing the previous account's rows, and rejects late list and save replies after their screen is disposed.

The generated `add_note` operation trims the body before it checks the note's length, so a direct API caller cannot store a note that is only whitespace even though the screen already trims its draft.

The sign-in countdown is tested apart from its executor: a host timer counts the whole wait down, a missing or rejected timer leaves the server's message and a usable manual retry without claiming time passed, a newer refusal or starting over stops an older countdown, and disposing the screen stops it without touching a disposed signal.

`leptoskit generate auth` on an existing database app still adds the screens and asks the app to call `auth::provide()` and put `auth::screens().gate(…)` in front of its own home screen. Rewriting an app's root automatically is not safe, so that manual step stays open; the journey covers the fully scaffolded `--auth` app, which is already wired.

- **`lk lint` passes on a fresh clone.** `leptoskit lint` no longer fails on a fresh clone of an app with native platforms. App-owned runners include a registrant under `.leptoskit/<platform>/`, which Git ignores and only native builds wrote, so clippy stopped on the missing file until you ran a native build. Before clippy starts, `lk lint` now writes missing registrants for the enabled hosts that compile on the current machine and enables the Cargo features their plugins need. Existing registrants stay untouched until the next native build. If a host cannot be prepared, lint reports a warning and clippy names what it cannot compile. _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #753](https://github.com/leptoskit/leptoskit/pull/753)
- **New apps write builder lint to match the starter screens.** `leptoskit new` now sets `[lint] markup = "builder"` with `markup_style` and `component_attribute` denied, so a fresh scaffold's builder home screen is clean under `leptoskit lint --deny-warnings`. The skill, creating guide, linting guide, and small-bundles guide match: omitted `render` still defaults to csr in the schema, but new projects keep the explicit ssr they already write. _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #653](https://github.com/leptoskit/leptoskit/pull/653)
- **Scaffold uses shared Health; skill documents web-only trim.** New apps re-export `Health` from the shared crate in the app library so `leptoskit lint --deny-warnings` no longer fails on an unused `*_shared` dependency before any RPC types are used. The coding-agent skill says there is no frontend-only `new` flag: after scaffolding, set `platforms = ["web"]` and optionally trim unused host workspace members. _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #649](https://github.com/leptoskit/leptoskit/pull/649)
- **New apps pin rustc; skill matches the scaffold.** `leptoskit new` now writes a `rust-toolchain.toml` that pins the SDK channel and the `wasm32-unknown-unknown` target, so `edition = "2024"` projects build even when rustup's default toolchain is older. The coding-agent skill no longer claims the default starter uses regenerating file routes, tells agents to keep the HTML `shell` in leptos `view!` (with a line-local lint allow) when enforcing builders, and shows `#[component]` helpers returning `Element` for `.child(…)` composition. The creating guide notes the SSR default and the toolchain pin. _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #648](https://github.com/leptoskit/leptoskit/pull/648)
- **Web-only lint skips unused hosts; drop an import.** `leptoskit lint`'s clippy pass excludes workspace host crates whose platforms are not listed in `app.platforms`, so setting `platforms = ["web"]` is enough for a green deny-warnings run without GTK or native prepare. `--db` / `--auth` scaffolds no longer put an unused `use leptos::prelude::*` in `App()`. The skill documents the platforms-based clippy skip. _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #657](https://github.com/leptoskit/leptoskit/pull/657)
- **`lk lint` knows the plugin surface tags.** `lk lint` now counts the plugin surfaces `Video`, `Camera`, `Map`, and `LivePhoto` among leptoskit's built-in components, so the markup-style rule checks trees that use them like any other built-in tag. _Packages:_ 🟢 _leptoskit_lint_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #752](https://github.com/leptoskit/leptoskit/pull/752)
- **Over the air updates get their own section.** The home page and README now lead with one component tree and real native widgets. Over-the-air updates have their own section, describing code push for installed native apps and which generated hosts install releases. _Packages:_ 🟢 _leptoskit_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #552](https://github.com/leptoskit/leptoskit/pull/552)

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

Grouped release for `release`.

### Breaking changes

#### leptoskit_testing_library: browser component tests

_Packages:_ 🔴 _leptoskit_testing_library_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_testing_library` provides Testing Library queries and widget-test actions for leptoskit components mounted in a real browser.

It mounts through the web renderer and shares finders with `leptoskit_testing`, so a team can test the browser build with the same vocabulary it uses for native widget tests.

#### Start the first release at 0.2.0

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #486](https://github.com/leptoskit/leptoskit/pull/486)

This is leptoskit's first public release, and every crate in it is `0.2.0`. The workspace, its internal crate dependencies, and the example apps that pin them all start from `0.1.0`, and a breaking (`major`) change before 1.0 raises the minor version, so the release lands on `0.2.0`. Several of the crate names already have a `0.1.0` on crates.io, which never accepts the same version twice, so the first release has to sit above it.

Apps that `leptoskit new` creates pin the exact version of the CLI that made them, `=0.2.0`.

#### leptoskit_yoga: flexbox and CSS grid layout

_Packages:_ 🔴 _leptoskit_yoga_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_yoga` is a from-scratch layout engine that turns one `Style` into flexbox or CSS grid geometry, with no C dependencies and no platform lock-in.

`leptoskit_native` calls it for every commit; backends only apply the frames it returns. It is also usable on its own when a Rust tool needs React Native-style layout or a CSS grid implementation without a browser.

#### leptoskit_use: cross-platform hooks

_Packages:_ 🔴 _leptoskit_use_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_use` provides cross-platform reactive primitives inspired by leptos-use: state, derived values, watchers, timers, storage, and viewport breakpoints without a DOM or executor.

The hooks read leptos signals and accept host services explicitly, so the same code runs in a browser and in a native guest. The `local-storage` feature adds persistent JSON storage in native app directories and the browser's local storage, and the `web` feature adds browser-only hooks such as media queries and event listeners. It is the utility layer beneath app screens, not a UI framework.

#### leptoskit_native: components, Tailwind, and the renderer

_Packages:_ 🔴 _leptoskit_native_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_native` is the native side of the shared tree: builders for every component, from `View`, `Text`, and `Button` to `TextInput`, `PasswordInput`, `Slider`, and `ScrollView`, the Tailwind compiler, the retained element tree, and the diff that turns a commit into a mutation batch.

Every native backend implements `NativeBackend` and consumes the mutations this crate produces; the web build uses its optional `web` feature to render the same builders through leptos. App code normally reaches it through `leptoskit::prelude`, not as a direct dependency, so one screen works on every target.

#### leptoskit_testing: native widget tests

_Packages:_ 🔴 _leptoskit_testing_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_testing` runs Flutter-style widget tests over the production renderer and Yoga layout, so a screen can be tested without a device or simulator.

`WidgetTester` renders a screen, finds elements by text, label, role, or test id with Testing Library-style queries (`get_by_text`, `query_by_label`, and the rest), acts on them, and pumps the renderer a bounded number of times while waiting for a result. The `native-ui` feature goes further and drives the real app through each platform's own automation: XCUITest on iOS, UI Automator on Android, XCTest on macOS, UI Automation on Windows, and AT-SPI on Linux.

#### leptoskit_tui: the terminal backend

_Packages:_ 🔴 _leptoskit_tui_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_tui` draws the same shared element tree in the terminal, for long-lived interactive apps such as editors, dashboards, and chat clients, with keyboard focus, the cursor, mouse input, and scrolling handled by the backend.

Every cell is 8×16 logical pixels, so Yoga layout, Tailwind spacing, and text measurement land on the monospace grid. `leptoskit dev --platform tui` and `leptoskit build --platform tui` run it, and `examples/opencode` is a full coding-agent interface built with it. The terminal's font is the user's, so font sizes and weights become attributes and colors; `Image` and `Vector` draw a placeholder, Rive playback is unsupported, and right-to-left text is not reordered.

#### leptoskit_appkit: the macOS backend

_Packages:_ 🔴 _leptoskit_appkit_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_appkit` is the macOS backend: it applies leptoskit mutation batches to real `NSView`s, from labels and buttons to `NSSecureTextField` password fields, sliders, and scroll views, and measures text with AppKit's own fonts.

A `NativeSurface` built on `AppKitBackend` renders the same tree the web build renders as DOM, and `snapshot_png` draws it offscreen without a window. AppKit is the reference backend that the others follow, and it draws every built-in component. The `macos` feature brings it in, but the macOS app `leptoskit new` generates still draws through a smaller Swift renderer, which covers ten components so far. On other platforms the crate compiles to an empty library.

#### leptoskit_uikit: the iOS backend

_Packages:_ 🔴 _leptoskit_uikit_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_uikit` is the iOS backend: it renders shared leptoskit components as real UIKit views and controls, such as `UISwitch`, `UISlider`, `UIProgressView`, and secure text fields for password inputs, and measures text with Core Text.

It draws every component when you host screens with `UIKitHost` in your own app, which keeps the layout inside a view controller and reports rotation and safe-area insets. The app generated by `leptoskit build` does not use it yet: that app draws ten components through its own Swift renderer. iOS 15 is the minimum, and images load from local files and app bundle resources, not web URLs.

#### leptoskit_android: the Android backend

_Packages:_ 🔴 _leptoskit_android_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_android` is the Android Views backend: it applies the shared mutation stream to real Android views, with Yoga owning exact outer frames and Android owning each control's interior.

Compile `android/WidgetHost.kt` into an Android app and construct `AndroidBackend::new` on the UI thread. Every built-in primitive renders a real view; callbacks propose typed events that the app commits through `NativeSurface`.

#### leptoskit_gtk: the Linux backend

_Packages:_ 🔴 _leptoskit_gtk_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_gtk` renders the shared element tree as real GTK 4 Linux widgets, consuming the same Yoga mutation stream as every other backend, and draws every built-in component. It needs GTK 4.6 or newer.

The Linux app `leptoskit new` generates uses it through the `linux` feature. A host initializes GTK on the application thread, hands a `GtkBackend` to `NativeSurface`, shows a committed root with `open_window`, and runs the loop with `run_application`; GTK objects stay on their creating thread.

#### leptoskit_windows: the Windows backend

_Packages:_ 🔴 _leptoskit_windows_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_windows` renders shared leptoskit elements as Win32 windows and native controls: every element owns a child window, and interactive elements, password fields included, use real Windows controls so keyboard and accessibility behave natively.

`leptoskit build --platform windows` produces an app that draws every component with it and embeds the common-controls manifest for you; a hand-written host must embed the crate's `app.manifest` itself. The backend stays on one UI thread, reuses the shared Yoga layout, and never embeds a browser. `Switch` is drawn as a checkbox, rounded corners have unsmoothed edges, and the generated app can't host plugins yet.

#### leptoskit_aria: accessible primitives

_Packages:_ 🔴 _leptoskit_aria_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_aria` provides headless accessible components for the shared element tree: disclosure, toggle buttons, tabs, labeled fields, and dialogs with roles and state wired for web and native.

It owns behavior, not pixels: app code styles the pieces with Tailwind and gets consistent focus, keyboard, and ARIA semantics on every backend. `leptoskit_form` builds its fields on top of it.

#### leptoskit_analytics: see which routes fail or slow down

_Packages:_ 🔴 _leptoskit_analytics_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #513](https://github.com/leptoskit/leptoskit/pull/513)

`leptoskit_analytics` keeps bounded, in-memory request and browser measurements for a leptoskit app, so you can see which routes fail or slow down without running a separate observability service.

Enable `analytics` alongside `server` on the `leptoskit` dependency. The `server` feature instruments Axum, continues W3C `traceparent` correlation, and serves a protected JSON evidence API at `/_leptoskit/analytics` plus a same-origin browser module at `/_leptoskit/analytics.mjs`. The browser module honors Global Privacy Control and Do Not Track, and collects no user or session ID, cookie, query string, or page content. Each replica reports its own snapshot; connect the deployment service (`lk console serve`) for durable fleet history, or export OTLP/HTTP metrics.

#### leptoskit_deploy_console: self-hosted deployments

_Packages:_ 🔴 _leptoskit_deploy_console_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #513](https://github.com/leptoskit/leptoskit/pull/513)

`leptoskit_deploy_console` is the optional service behind `lk console`: one process that hosts the deployment dashboard, a durable operation queue, a scoped API for agents, and analytics history, backed by SQLite.

Run `lk console serve` on a trusted worker with Docker, OpenTofu, Kamal 2, Git, and SSH, then `lk console connect` from the app's checkout, and `lk infra plan`, `lk infra apply`, and `lk deploy` run through it. It reuses the CLI's OpenTofu and release engine, binds each apply to a reviewed plan, and keeps interrupted operations for explicit reconciliation. The `server` feature enables the service; the dashboard is a leptoskit component tree rendered on the server. It serves one owner's repositories and is not a multi-tenant executor.

#### leptoskit_form: headless form state

_Packages:_ 🔴 _leptoskit_form_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_form` is headless form state in the style of TanStack Form: typed fields, sync validation, touch and submit tracking, and inputs wired through the aria fields.

A form declares `FieldSpec`s once; the same state drives web inputs and native controls, and `leptoskit_aria` supplies the accessible field wiring. Validation and submission logic stay out of the view layer.

#### leptoskit_config: project configuration

_Packages:_ 🔴 _leptoskit_config_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_config` loads and validates `leptoskit.toml`: app identity and platforms, server and build settings, native targets and plugin grants, code push channels, routes, translations, the database, sign-in, infrastructure, browser extensions, store publishing, plugins, formatting, and lint settings.

The CLI, build pipeline, dev server, and native hosts all read the same typed configuration, so they agree on what the app is. Unknown keys fail at load time instead of being ignored. The schema is versioned (version 0, unstable until version 1); `leptoskit migrate` upgrades an older file, and the `schema` feature generates its JSON Schema.

#### leptoskit_plugin: build-time plugins

_Packages:_ 🔴 _leptoskit_plugin_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin` is the build-hook system shared by leptoskit plugins and leptos plugins: generated code, feature flags, environment variables, asset directories, post-build reports, and a plugin's own `plugin:<name>` commands.

Plugins are declared in `leptoskit.toml`. A Rust plugin crate implements `LeptoskitPlugin`; a separate program in any language answers each hook over a versioned JSON protocol; and leptos plugins are forwarded to the app build as cargo features. Runtime native plugins use the bridge instead; this is the build-time half.

#### leptoskit_bridge: the plugin transport

_Packages:_ 🔴 _leptoskit_bridge_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_bridge` is the bounded binary transport between a Wasm guest and the native plugins it is allowed to call.

The host registers each plugin and grants its methods, so an over-the-air bundle cannot add native capabilities. Every call is bounded in size and number, is cancelled when the guest drops it, and fails with error codes the Swift, Kotlin, and C++ hosts share. Most apps never name it directly; each runtime plugin wraps it in a typed `guest` module.

#### leptoskit_bridge_ffi: the native half of the app host

_Packages:_ 🔴 _leptoskit_bridge_ffi_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_bridge_ffi` is the C ABI, plus the Swift, Kotlin, and C++ code, that lets a native app shell host a leptoskit Wasm guest: it runs the guest, forwards its plugin calls, and draws its tree with the platform's own views.

The macOS, iOS, and Android apps `leptoskit new` generates link it, and HarmonyOS links its experimental N-API layer; app authors never add it themselves. Its Swift and Kotlin renderers draw ten components so far, including password fields, and stop on any other with an "unsupported widget" error. The generated Linux and Windows apps use the GTK and Win32 backend crates instead.

#### leptoskit_build: one app, platform modules

_Packages:_ 🔴 _leptoskit_build_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_build` is the build-script helper behind `platform_module!`: it records which platform-specific modules exist and cfg-selects them while one app keeps a single module tree.

Generated apps call it from `build.rs`. It lets a shared module be replaced by a `.macos.rs`, `.ios.rs`, or `.web.rs` file without changing the imports in the rest of the app.

#### leptoskit_plugin_crypto: portable digests and randomness

_Packages:_ 🔴 _leptoskit_plugin_crypto_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_crypto` is the leptoskit plugin for portable SHA-256, SHA-512, secure random bytes, and UUID v4 with a binary native bridge.

Add `leptoskit_plugin_crypto` as a direct dependency and select the `crypto` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic and hashing and entropy run on the host's Rust worker. Guests enable the `guest` feature and call the host through `guest::Crypto`. Hashing works directly on every target; browser apps enable the `web` feature for random bytes and UUIDs from `Crypto.getRandomValues`. There is no pseudo-random fallback, and encryption and key storage are out of scope.

#### leptoskit_plugin_os: host platform identity

_Packages:_ 🔴 _leptoskit_plugin_os_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_os` is the leptoskit plugin for the host operating system and CPU architecture as seen from inside a Wasm guest.

Add `leptoskit_plugin_os` as a direct dependency and select the `os` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. The values come from compile-time constants of the host build, so the guest sees the real platform even though its own target is wasm32.

#### leptoskit_plugin_clipboard: text clipboard access

_Packages:_ 🔴 _leptoskit_plugin_clipboard_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_clipboard` is the leptoskit plugin for system text clipboard access - read, write, and clear - on iOS, Android, macOS, Windows, Linux, and in the browser.

Add `leptoskit_plugin_clipboard` as a direct dependency and select the `clipboard` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Clipboard` rather than resolving the plugin on every call. iOS, Android, and macOS use the bundled Swift and Kotlin handlers, or experimental pure-Rust ones when `native.experimental-rust-plugins = true`; Rust desktop hosts keep a `Clipboard` alive over the system clipboard, and browser apps call the `web` module. Only plain text is supported; browser operations need a secure context and may need a user gesture.

#### leptoskit_plugin_haptics: haptic feedback

_Packages:_ 🔴 _leptoskit_plugin_haptics_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_haptics` is the leptoskit plugin for selection, impact, and notification feedback on native actuators.

Add `leptoskit_plugin_haptics` as a direct dependency and select the `haptics` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Haptics` rather than resolving the plugin on every call. iOS uses UIKit feedback generators, Android uses platform haptic feedback, and macOS approximates the mobile styles with its trackpad performer, through the bundled Swift and Kotlin handlers or, with `native.experimental-rust-plugins = true`, experimental pure-Rust ones. Browsers use the Vibration API where allowed, and Windows and Linux return `Unavailable`. Success means the OS accepted the request, not that the device vibrated.

#### leptoskit_plugin_battery: battery and power state

_Packages:_ 🔴 _leptoskit_plugin_battery_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_battery` is the leptoskit plugin for battery charge, charging state, and system power-saving snapshots with cancellable change observation.

Add `leptoskit_plugin_battery` as a direct dependency and select the `battery` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Battery` rather than resolving the plugin on every call. Unknown readings stay unknown instead of being reported as a full battery, and a simulator is never presented as physical hardware.

#### leptoskit_plugin_device: device information

_Packages:_ 🔴 _leptoskit_plugin_device_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_device` is the leptoskit plugin for device and operating system information without persistent identifiers or user-assigned names.

Add `leptoskit_plugin_device` as a direct dependency and select the `device` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Device` rather than resolving the plugin on every call. Native guests, Rust desktop hosts, and browsers each return a validated binary snapshot with the fields that platform can actually provide.

#### leptoskit_plugin_secure_store: credential-store secrets

_Packages:_ 🔴 _leptoskit_plugin_secure_store_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_secure_store` is the leptoskit plugin for small binary secrets, such as a refresh token, kept in the platform credential store and scoped to one application.

Add `leptoskit_plugin_secure_store` as a direct dependency and select the `secure-store` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::SecureStore` to get, set, and delete secrets of up to 2048 bytes; all of an app's secrets share one encrypted record limited to 2560 bytes and 64 entries, which hosts can lower. iOS and macOS use the Keychain and Android encrypts with an Android Keystore key, while desktop Rust hosts construct `SecureStore` over the Keychain, Windows Credential Manager, or Secret Service. The browser has no secure store, and there is no key listing, biometric prompt, or cloud sync.

#### leptoskit_plugin_linking: external links and settings

_Packages:_ 🔴 _leptoskit_plugin_linking_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_linking` is the leptoskit plugin for validated external HTTP(S), email, phone, and SMS links plus application settings, without executing shell text.

Add `leptoskit_plugin_linking` as a direct dependency and select the `linking` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Linking` rather than resolving the plugin on every call. Incoming deep links need host lifecycle integration and are not handled by this crate.

#### leptoskit_plugin_passkeys: passkeys and Sign in with Apple

_Packages:_ 🔴 _leptoskit_plugin_passkeys_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_passkeys` is the leptoskit plugin for passkeys and Sign in with Apple through the platform's own credential sheet. The sign-in client in `leptoskit_auth` uses it, so apps rarely call it directly.

Add `leptoskit_plugin_passkeys` as a direct dependency and select the `passkeys` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Passkeys` rather than resolving the plugin on every call. Apple platforms use `ASAuthorization` and Android uses Credential Manager, with WebAuthn JSON crossing the bridge unchanged; Sign in with Apple is Apple-only. Windows and Linux report the calls unavailable, and the sign-in client falls back to the system browser there. The relying party's site must list the app in `apple-app-site-association` and `assetlinks.json`, which `leptoskit::server::app_associations` serves from `[native]`.

#### leptoskit_plugin_web_auth: system-browser sign-in

_Packages:_ 🔴 _leptoskit_plugin_web_auth_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_web_auth` is the leptoskit plugin for sign-in sessions in the system browser - ASWebAuthenticationSession on Apple platforms, Auth Tab on Android, and a loopback redirect on desktop.

Add `leptoskit_plugin_web_auth` as a direct dependency and select the `web-auth` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::WebAuth` rather than resolving the plugin on every call. It follows RFC 8252: the provider page never loads in a web view inside the app.

#### leptoskit_plugin_filesystem: application files

_Packages:_ 🔴 _leptoskit_plugin_filesystem_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_filesystem` is the leptoskit plugin for application-private binary files on native and OPFS in the browser, with bounded messages.

Add `leptoskit_plugin_filesystem` as a direct dependency and select the `filesystem` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Filesystem` rather than resolving the plugin on every call. Every native path stays inside the application-owned data directory; guests cannot select an arbitrary filesystem root.

#### leptoskit_plugin_network: network state

_Packages:_ 🔴 _leptoskit_plugin_network_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_network` is the leptoskit plugin for network state and demand-driven change observation.

Add `leptoskit_plugin_network` as a direct dependency and select the `network` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Network` rather than resolving the plugin on every call. Apple uses `NWPathMonitor`, Android uses `ConnectivityManager`, desktop reports interface state, and the browser uses `navigator.onLine` plus `NetworkInformation`; an active connection never claims a server is reachable.

#### leptoskit_plugin_posture: foldable posture

_Packages:_ 🔴 _leptoskit_plugin_posture_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_posture` is the leptoskit plugin for fold state, hinge geometry, and hinge angles, so a layout can put content on either side of a fold.

Add `leptoskit_plugin_posture` as a direct dependency and select the `posture` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::DevicePosture` rather than resolving the plugin on every call, and browser apps use the `web` module. iOS 27.1 uses the hinge interaction API, Android uses Jetpack WindowManager and the hinge angle sensor, HarmonyOS (experimental) uses its display fold status, and Chromium browsers report the fold without angles. macOS, Windows, and Linux report the plugin unavailable.

#### leptoskit_plugin_dialog: native dialogs

_Packages:_ 🔴 _leptoskit_plugin_dialog_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_dialog` is the leptoskit library for asynchronous native file, directory, save, and message dialogs on macOS, Windows, and Linux, plus browser file selection.

Rust desktop hosts and browser apps call it directly: it has no `native.plugins` entry, no guest API, and no automatic registration, because each dialog needs a running event loop and a live parent window. On desktop, `desktop::FileDialog` and `desktop::MessageDialog` borrow that parent window until the dialog closes, and Linux needs an XDG desktop portal and Zenity. In the browser, `web::open_files` uses the File System Access API from a user action on HTTPS, and `web::read_file` reads a selection under an explicit byte limit; browser save, directory, and message dialogs, and mobile dialogs, are not provided.

#### leptoskit_plugin_document_picker: document import

_Packages:_ 🔴 _leptoskit_plugin_document_picker_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_document_picker` is the leptoskit plugin for bounded document import through the iOS document picker, macOS open panel, Android Storage Access Framework, or an HTML file input.

Add `leptoskit_plugin_document_picker` as a direct dependency and select the `document-picker` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and call `pick`, browser apps call `web::pick` from a user action, and Rust desktop hosts on macOS, Windows, and Linux can call `desktop::pick` with a parent window. The user picks one document; the app receives its name, its MIME type when known, and owned bytes up to 4 MiB, and never gets a reusable path into the document provider.

#### leptoskit_plugin_screen_orientation: orientation requests

_Packages:_ 🔴 _leptoskit_plugin_screen_orientation_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_screen_orientation` is the leptoskit plugin for scoped activity, scene, and browser-document orientation requests with explicit restoration.

Add `leptoskit_plugin_screen_orientation` as a direct dependency and select the `screen-orientation` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::ScreenOrientation` rather than resolving the plugin on every call. iOS and Android hold the policy for the app's own scene or activity and restore the previous one when the hold is dropped, browser apps use `web::hold`, and macOS, Windows, and Linux have no native orientation lock. A pending hold is not an acknowledgement that the OS rotated; the snapshot reports the presented interface, not accelerometer orientation.

#### leptoskit_plugin_light_sensor: ambient light

_Packages:_ 🔴 _leptoskit_plugin_light_sensor_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_light_sensor` is the leptoskit plugin for ambient illuminance in lux from Android's light sensor or the browser's AmbientLightSensor.

Add `leptoskit_plugin_light_sensor` as a direct dependency and select the `light_sensor` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::LightSensor` rather than resolving the plugin on every call. iOS, macOS, and desktop have no supported public ambient-light API and return `Unavailable`; display brightness is never substituted for illumination.

#### leptoskit_plugin_pedometer: step counts

_Packages:_ 🔴 _leptoskit_plugin_pedometer_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_pedometer` is the leptoskit plugin for bounded, demand-driven step-counter readings with explicit platform count domains.

Add `leptoskit_plugin_pedometer` as a direct dependency and select the `pedometer` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Pedometer` rather than resolving the plugin on every call. Only iOS (CoreMotion) and Android (its step counter) count steps; the app supplies the motion usage description or activity-recognition permission. iOS reports steps for the requested interval and can read history from the past seven days, while Android reports its cumulative count since boot, so neither is a daily total. macOS, Windows, Linux, and the browser report no sensor and never invent counts.

#### leptoskit_plugin_accelerometer: accelerometer readings

_Packages:_ 🔴 _leptoskit_plugin_accelerometer_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_accelerometer` is the leptoskit plugin for cancellable acceleration readings including gravity, in metres per second squared, along native device axes.

Add `leptoskit_plugin_accelerometer` as a direct dependency and select the `accelerometer` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Accelerometer` rather than resolving the plugin on every call. iOS and Android use real system sensors, desktop has no sensor, and the browser uses Generic Sensor or DeviceMotion.

#### leptoskit_plugin_application: app identity

_Packages:_ 🔴 _leptoskit_plugin_application_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_application` is the leptoskit plugin for installed host application identity: ID, localized display name, release version, and build number.

Add `leptoskit_plugin_application` as a direct dependency and select the `application` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Application` rather than resolving the plugin on every call. Apple reads `Bundle.main`, Android reads the running package record, and a Rust desktop host supplies its own packaging metadata; browsers have no equivalent installed-app identity.

#### leptoskit_plugin_autostart: login startup

_Packages:_ 🔴 _leptoskit_plugin_autostart_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_autostart` is the leptoskit plugin for explicit per-user desktop login registration with a validated identity and command: a LaunchAgent on macOS, an XDG autostart entry on Linux, and the current user's Run key on Windows.

The native host constructs `Autostart` itself with the app's identity, executable, and arguments; it has no `native.plugins` entry and installs nothing by default. The host calls `enable`, `disable`, or `is_registered` directly, or registers it on the bridge so a guest with the `guest` feature can use `guest::AutostartClient` without choosing the command. Constructing it never enables login startup, an existing entry it doesn't own is never overwritten, and iOS, Android, and the browser return `Unavailable`.

#### leptoskit_plugin_barometer: barometric pressure

_Packages:_ 🔴 _leptoskit_plugin_barometer_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_barometer` is the leptoskit plugin for cancellable atmospheric pressure readings in hectopascals.

Add `leptoskit_plugin_barometer` as a direct dependency and select the `barometer` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Barometer` rather than resolving the plugin on every call. iOS and Android use real pressure sensors (iOS needs an `NSMotionUsageDescription`); desktop and the browser report the sensor as unavailable, and no relative altitude estimate is fabricated.

#### leptoskit_plugin_brightness: screen brightness

_Packages:_ 🔴 _leptoskit_plugin_brightness_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_brightness` is the leptoskit plugin for scoped mobile screen brightness overrides with explicit restoration.

Add `leptoskit_plugin_brightness` as a direct dependency and select the `brightness` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Brightness` rather than resolving the plugin on every call. Android changes only the activity window, iOS changes the screen and restores the previous value, and desktop and the browser return `Unavailable`.

#### leptoskit_plugin_gyroscope: gyroscope readings

_Packages:_ 🔴 _leptoskit_plugin_gyroscope_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_gyroscope` is the leptoskit plugin for cancellable angular velocity readings in radians per second around native device axes.

Add `leptoskit_plugin_gyroscope` as a direct dependency and select the `gyroscope` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Gyroscope` rather than resolving the plugin on every call. iOS and Android use real system sensors, desktop has no sensor, and the browser converts DeviceMotion degrees to radians.

#### leptoskit_plugin_http: bounded HTTP

_Packages:_ 🔴 _leptoskit_plugin_http_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_http` is the leptoskit plugin for scoped asynchronous HTTP clients with bounded request and response bodies, a total timeout, and cancellation by dropping the request.

Add `leptoskit_plugin_http` as a direct dependency, select the `http` plugin in `native.plugins`, and list the origins the app may reach under `[native.plugin-options.http]`; there is no wildcard, and the guest can't widen the list. Guests use `default-features = false, features = ["guest"]` and send requests through `guest::HttpClient`. Native requests use reqwest with rustls on the host's async runtime; browser apps keep the default `browser` feature and use fetch with incremental body reading. Redirects are not followed: native callers get the 3xx response, and the browser rejects it.

#### leptoskit_plugin_keep_awake: display wake locks

_Packages:_ 🔴 _leptoskit_plugin_keep_awake_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_keep_awake` is the leptoskit plugin for scoped display wake locks with cancellation-safe ownership.

Add `leptoskit_plugin_keep_awake` as a direct dependency and select the `keep-awake` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::KeepAwake` rather than resolving the plugin on every call. iOS preserves and restores the idle timer, Android uses the activity's keep-screen-on flag, desktop uses OS assertions, and the browser uses the Screen Wake Lock API; none of them promise background execution.

#### leptoskit_plugin_localization: locale preferences

_Packages:_ 🔴 _leptoskit_plugin_localization_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_localization` is the leptoskit plugin for ordered preferred locales, language and region codes, calendar, timezone, first weekday, and clock preference.

Add `leptoskit_plugin_localization` as a direct dependency and select the `localization` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Localization` rather than resolving the plugin on every call. iOS, macOS, and Android read the system settings, desktop Rust hosts register `desktop::Localization`, and the browser calls `web::snapshot()`. Each snapshot reflects current settings and no global locale is mutated; there are no change events yet, so re-query after the app resumes.

#### leptoskit_plugin_magnetometer: magnetometer readings

_Packages:_ 🔴 _leptoskit_plugin_magnetometer_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_magnetometer` is the leptoskit plugin for cancellable magnetic-field readings in microteslas along native device axes.

Add `leptoskit_plugin_magnetometer` as a direct dependency and select the `magnetometer` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Magnetometer` rather than resolving the plugin on every call. This is a magnetic sensor measurement, not a compass heading, and platform calibration differs. iOS and Android use real system sensors (iOS needs `NSMotionUsageDescription`), the browser needs Generic Sensor support, and desktop has no sensor.

#### leptoskit_plugin_permissions: permissions

_Packages:_ 🔴 _leptoskit_plugin_permissions_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_permissions` is the leptoskit plugin for explicit camera and microphone permission queries and requests.

Add `leptoskit_plugin_permissions` as a direct dependency and select the `permissions` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Permissions` rather than resolving the plugin on every call. iOS, macOS, and Android use the system permission dialogs and browser apps call the `web` module; Windows and Linux have no backend. Querying status never shows a prompt, so request only after the user chooses a feature. Other permissions are not covered here: the location and notifications plugins request their own.

#### leptoskit_plugin_single_instance: single-instance locks

_Packages:_ 🔴 _leptoskit_plugin_single_instance_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_single_instance` is the native-host plugin for explicit process-lifetime instance locks.

Native hosts construct it explicitly; it is not a guest plugin and installs nothing by default. Call `InstanceGuard::acquire` before constructing the app and keep the guard alive; macOS, Linux, and Windows release the OS lock on drop or process exit, including a crash. It only provides mutual exclusion: a second launch gets `None` and does not forward its arguments or focus the first instance's window.

#### leptoskit_plugin_menu: native menus

_Packages:_ 🔴 _leptoskit_plugin_menu_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_menu` is the native-host plugin for bounded native menu definitions with explicit desktop ownership: commands, checkboxes, separators, submenus, and accelerators such as `CmdOrCtrl+S`.

Native hosts construct it explicitly on their UI thread; it is not a guest plugin and installs nothing by default. macOS attaches it as the application menu and Windows attaches it to a window the host owns; Linux has no standalone menu bar, so there it only feeds tray menus. Definitions are validated before any native allocation, and iOS, Android, and the browser only validate definitions and show no menu.

#### leptoskit_plugin_tray: tray icons

_Packages:_ 🔴 _leptoskit_plugin_tray_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_tray` is the native-host plugin for explicitly owned native desktop tray icons with bounded events.

Native hosts construct it explicitly; it is not a guest plugin and installs nothing by default. The host calls `TrayHost::install` after its event loop starts and owns removal; Linux uses the GTK-free KSNI backend, and mobile and the browser have no tray.

#### leptoskit_plugin_speech: text-to-speech

_Packages:_ 🔴 _leptoskit_plugin_speech_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_speech` is the leptoskit plugin for bounded, cancellable text-to-speech with one active utterance per plugin.

Add `leptoskit_plugin_speech` as a direct dependency and select the `speech` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Speech` rather than resolving the plugin on every call. iOS and macOS use `AVSpeechSynthesizer`, Android uses `TextToSpeech`, and browser apps use `web::Speech` over `speechSynthesis`; Windows and Linux have no speech backend yet. Overlapping speech returns `Busy`, and pause, resume, and word-boundary events are not implemented.

#### leptoskit_plugin_sqlite: private SQLite

_Packages:_ 🔴 _leptoskit_plugin_sqlite_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_sqlite` is the leptoskit plugin for bounded, application-private SQLite databases with typed bindings and the same bundled engine on every native platform.

Add `leptoskit_plugin_sqlite` as a direct dependency and select the `sqlite` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Sqlite` for parameterized writes, read-only queries, and atomic transaction batches. The database runs in Rust on the host's worker, so iOS, Android, and macOS behave the same; Windows and Linux hosts that register plugins themselves get the same engine, and the browser has no SQLite backend. Guests cannot choose file paths, attach databases, or run pragmas, and each database is capped at 64 MiB by default, with limits on statement, result, and transaction sizes.

#### leptoskit_plugin_store: JSON settings

_Packages:_ 🔴 _leptoskit_plugin_store_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_store` is the leptoskit plugin for bounded JSON application settings with explicit persistence and atomic ordered batches.

Add `leptoskit_plugin_store` as a direct dependency and select the `store` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Store` rather than resolving the plugin on every call. Each application gets one store and guests cannot choose filesystem paths: native apps save to a file in the app data directory and browser apps use `web::Store` over `localStorage`. Nothing saves until `save` or a committed batch, and the store is not encrypted, so keep credentials in the secure store.

#### leptoskit_plugin_image_manipulator: image transforms

_Packages:_ 🔴 _leptoskit_plugin_image_manipulator_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_image_manipulator` is the leptoskit plugin for bounded JPEG and PNG decode, resize, crop, rotate, flip, and encode using one Rust implementation.

Add `leptoskit_plugin_image_manipulator` as a direct dependency and select the `image_manipulator` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests use `default-features = false, features = ["guest"]` and call `guest::ImageManipulator`, which skips the image codecs; native calls run on the Rust worker so a batch takes one bridge round trip. Browser calls use the same engine compiled to Wasm, and large jobs belong in a Web Worker. Images are limited to 4 MiB, 8,192 pixels per side, and 16 operations per request.

#### leptoskit_plugin_screen_capture: capture protection

_Packages:_ 🔴 _leptoskit_plugin_screen_capture_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_screen_capture` is the leptoskit plugin for window-scoped capture prevention and capture observation as separate capabilities.

Add `leptoskit_plugin_screen_capture` as a direct dependency and select the `screen-capture` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::ScreenCapture` rather than resolving the plugin on every call. The plugin never captures pixels or reads screenshots; Android can protect its owned window and, on Android 14 and later, report screenshots; iOS can observe scene capture and app screenshots without prevention; macOS, Windows, Linux, and the browser report both unavailable.

#### leptoskit_plugin_store_review: store review requests

_Packages:_ 🔴 _leptoskit_plugin_store_review_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_store_review` is the leptoskit plugin for owned-window in-app store review requests through StoreKit and Google Play.

Add `leptoskit_plugin_store_review` as a direct dependency and select the `store-review` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::StoreReview` rather than resolving the plugin on every call. Acceptance never proves a dialog appeared or a review was submitted, and the plugin never substitutes another application's scene.

#### leptoskit_plugin_sharing: native sharing

_Packages:_ 🔴 _leptoskit_plugin_sharing_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_sharing` is the leptoskit plugin for bounded text and HTTP(S) URL sharing through the user-selected native share UI.

Add `leptoskit_plugin_sharing` as a direct dependency and select the `sharing` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and call `share`, and browser apps call `web::share` from a user action. iOS, macOS, Android, and browsers with Web Share are supported; Windows and Linux have no native share sheet yet, and files are not shared. The app supplies the content, the user picks the destination, and completion only means the platform finished its API callback.

#### leptoskit_plugin_tracking_transparency: tracking consent

_Packages:_ 🔴 _leptoskit_plugin_tracking_transparency_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_tracking_transparency` is the leptoskit plugin for explicit iOS App Tracking Transparency status and authorization requests.

Add `leptoskit_plugin_tracking_transparency` as a direct dependency and select the `tracking-transparency` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::TrackingTransparency` rather than resolving the plugin on every call. It never reads advertising identifiers or starts tracking; iOS returns the system status, and other platforms return `Unavailable`.

#### leptoskit_plugin_websocket: WebSocket connections

_Packages:_ 🔴 _leptoskit_plugin_websocket_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_websocket` is the leptoskit plugin for scoped WebSocket connections with bounded messages and explicit lifecycle.

Add `leptoskit_plugin_websocket` as a direct dependency and select the `websocket` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Native connections use tokio-tungstenite with rustls and need a Tokio runtime; browser connections own their socket and event callbacks. Every destination origin is explicit.

#### leptoskit_plugin_arguments: launch arguments

_Packages:_ 🔴 _leptoskit_plugin_arguments_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_arguments` is the leptoskit plugin for immutable launch-argument snapshots with typed, non-exiting clap parsing.

Add `leptoskit_plugin_arguments` as a direct dependency and expose the plugin explicitly from the native host; it has no `native.plugins` entry. Guests enable the `guest` feature and reuse `guest::ArgumentsClient` rather than resolving the plugin on every call. The host chooses which arguments to disclose; mobile and browser process capture returns `Unavailable`, while explicit snapshots and parsing still work.

#### leptoskit_plugin_shell: child processes

_Packages:_ 🔴 _leptoskit_plugin_shell_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_shell` is the leptoskit plugin for scoped, bounded, cancellable desktop child processes built from host-defined command capabilities, such as running a bundled tool with `--version`.

Add `leptoskit_plugin_shell` as a direct dependency, select the `shell` plugin in `native.plugins`, and declare each command under `[native.plugin-options.shell.commands]`: an absolute executable and working directory, the allowed argument slots, an explicit environment, output limits, and a deadline. Guests enable the `guest` feature and call `guest::ShellClient::execute` with a command name and arguments; they never send a shell command string. Commands run on macOS, Linux, and Windows, and cancellation, timeouts, and shutdown clean up the child's process group or Windows Job Object. The generated Windows and Linux apps don't register runtime plugins yet, so there a host constructs `Shell` itself; mobile and the browser have no process backend.

#### leptoskit_plugin_logging: structured logging

_Packages:_ 🔴 _leptoskit_plugin_logging_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_logging` is the leptoskit plugin for structured, bounded application logging: records with a severity, a target, a message, and named fields, written as JSON lines to rotated files.

Add `leptoskit_plugin_logging` as a direct dependency and select the `logging` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and write through `guest::Logger`. Native apps write to `<app data>/logs`, keeping four files of up to 1 MiB by default, and the browser logs to the console without persisting anything. There is no global logger, remote backend, or automatic redaction, and guests cannot choose where logs go.

#### leptoskit_plugin_notifications: local notifications

_Packages:_ 🔴 _leptoskit_plugin_notifications_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_notifications` is the leptoskit plugin for immediate local system notifications with explicit permission requests, bounded messages, and cancellation.

Add `leptoskit_plugin_notifications` as a direct dependency and select the `notifications` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Notifications` rather than resolving the plugin on every call. iOS, macOS, and Android post through the system (Android also configures channels), while browser and Linux apps call the `web` and `linux` modules directly; Windows has no backend yet. A successful call means the OS accepted the request, not that a banner appeared, and scheduled notifications, push, and tap routing are not implemented.

#### leptoskit_plugin_local_authentication: biometrics

_Packages:_ 🔴 _leptoskit_plugin_local_authentication_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_local_authentication` is the leptoskit plugin for local device-owner authentication through Apple `LAContext` and Android `BiometricPrompt`.

Add `leptoskit_plugin_local_authentication` as a direct dependency and select the `local-authentication` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::LocalAuthentication` rather than resolving the plugin on every call. This is not account identity or server authorization; the plugin reports explicit terminal outcomes and never invents a success.

#### leptoskit_plugin_location: foreground location

_Packages:_ 🔴 _leptoskit_plugin_location_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_location` is the leptoskit plugin for foreground location permission and cancellable bounded fixes through CoreLocation, Android LocationManager, or browser Geolocation.

Add `leptoskit_plugin_location` as a direct dependency and select the `location` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Location` rather than resolving the plugin on every call. Native reads never prompt; request permission as a separate, intentional action, and the app supplies its usage description and manifest permissions. iOS, macOS, Android, and the browser are supported; Windows and Linux have no location backend yet, and there is no background tracking or geofencing.

#### leptoskit_plugin_global_shortcut: global shortcuts

_Packages:_ 🔴 _leptoskit_plugin_global_shortcut_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_global_shortcut` is the leptoskit plugin for scoped desktop global shortcuts with explicit registration conflicts and bounded events.

The native host creates `GlobalShortcuts` on its event-loop thread after startup; it has no `native.plugins` entry and installs nothing by default. `register` returns a guard that unregisters when dropped, and a host can wrap the manager in `ShortcutPlugin` so a guest with the `guest` feature registers shortcuts and polls events through `guest::GlobalShortcut`. macOS, Windows, and X11 are supported; native Wayland, browsers, and mobile return `Unavailable`, with no XWayland fallback or fake browser shortcut.

#### leptoskit_plugin_window_state: window state

_Packages:_ 🔴 _leptoskit_plugin_window_state_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_window_state` is the native-host plugin for validated window placement and explicit native window restoration.

Native hosts construct it explicitly; it is not a guest plugin and installs nothing by default. The portable state and placement API works everywhere; native capture and restore adapters currently ship for macOS, and the host owns persistence and which window is restored.

#### leptoskit_sensor: the shared sensor protocol

_Packages:_ 🔴 _leptoskit_sensor_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_sensor` is the shared protocol and native lifecycle behind the accelerometer, gyroscope, magnetometer, barometer, light sensor, and pedometer plugins.

Each request takes one fresh reading and then releases the sensor, so repeated calls are not a fixed-rate recording stream; timestamps come from the platform's monotonic clock, not UTC. The sensor plugins share its request and reply layout, cancellation rules, and browser adapters over the Generic Sensor API and device motion events, so an app can switch sensors without relearning the contract. Apps depend on the individual sensor crate; this crate is the common floor.

#### leptoskit_codepush: over-the-air updates

_Packages:_ 🔴 _leptoskit_codepush_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_codepush` ships interface changes to native apps without a store review: signed, content-hashed releases of the Wasm bundle, packed into one file, with binary deltas so a device downloads only the chunks that changed.

`leptoskit codepush` packs, signs, and publishes releases to a static channel layout that any file server, object store, or CDN can host, and `leptoskit_wasm` stages them on the device. A client accepts only releases signed by a trusted key and newer than what it runs, so a compromised registry can neither ship its own code nor roll a device back. A host can also require independent source approvals: rebuilders who built the same files from a public commit and signed to say so.

#### leptoskit_wasm: the on-device Wasm runtime

_Packages:_ 🔴 _leptoskit_wasm_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_wasm` runs the app's interface bundle on the device in `wasmi`, an interpreter that is allowed on iOS where compiling code at runtime is not, and drives a native backend through a versioned ABI.

This is what makes the shipped app a shell: the interface arrives as Wasm, renders through the same renderer, and a code push release can replace it without a store update. Every entry point is fuel-metered and rendering work is capped, so a bad bundle cannot hang the device. The generated iOS, Android, macOS, Windows, Linux, and terminal apps embed it.

#### leptoskit_axum: the server toolkit

_Packages:_ 🔴 _leptoskit_axum_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_axum` is the Axum server toolkit every leptoskit deployment shares: static files with precompressed copies and an `index.html` fallback, `/healthz` and `/readyz` probes with graceful shutdown, code push routes, and the association files that let the native apps use the site's passkeys and sign-in links.

Apps reach it as `leptoskit::server` through the `server` feature, and mount it beside `leptos_axum`; the `analytics` feature adds request measurements. It re-exports `axum`, `tokio`, `tower`, and `tower_http`, so a deployment depends on one crate, and it stays leptos-free so the toolkit composes with any router.

#### leptoskit_rpc_codegen: the RPC specification and clients

_Packages:_ 🔴 _leptoskit_rpc_codegen_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_rpc_codegen` holds the RPC specification behind leptoskit server functions and generates typed clients from it for TypeScript, Python, Go, Rust, Dart, and the command line.

The server writes the specification; the CLI reads it back and generates clients that speak the RPC's own protocol, plus an OpenAPI 3.1 document. The generators sit behind a default feature, so the server binary can depend on the types alone.

#### leptoskit_rpc_macros: RPC attributes

_Packages:_ 🔴 _leptoskit_rpc_macros_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_rpc_macros` provides the attributes behind `leptoskit_rpc`: `#[query]`, `#[mutation]`, and `#[action]` declare operations; `#[stream]`, `#[channel]`, `#[job]`, and `#[cron]` declare live streams, channels, background jobs, and scheduled jobs; and `#[api]` publishes a leptos server function to generated clients.

Use them through `leptoskit_rpc`, which re-exports them. Every target gets the operation's typed argument struct so any client can call it; only a server build keeps the function body and registers its route.

#### leptoskit_rpc: typed server operations

_Packages:_ 🔴 _leptoskit_rpc_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_rpc` is the boundary between an app's shared screens and its server. It adds retries and error classification to leptos server functions, and declares operations (`#[query]`, `#[mutation]`, `#[action]`) that run on the database as the signed-in caller and are called the same way from SSR, the browser, and a native guest through `client::call`.

The `live` feature serves queries over a WebSocket, with a server-sent events fallback, and pushes only what changed when a write commits; screens read them with `use_live`, and join channels with presence through `use_channel`. Live connections are bounded: a per-connection frame budget, a timeout for clients that stop reading, channel rooms tied to the arguments they were authorized for, and capped page sizes. `#[api]` publishes server functions so `leptoskit rpc generate` can write TypeScript, Python, Go, Rust, Dart, and command-line clients plus an OpenAPI document. Pick a database with the `postgres` or `sqlite` feature, and spread live updates across several servers with `relay`.

#### leptoskit_query: async state in the TanStack style

_Packages:_ 🔴 _leptoskit_query_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_query` is TanStack Query for leptos: screens that load data get caching, deduplication, retries, background refetching, infinite queries, and optimistic mutations, all exposed as fine-grained signals so a view re-renders only when the value it reads changes.

Declare a `QueryScope` or `MutationScope` and read it with `use_query` or `use_mutation`. Queries refetch on focus and reconnect, pause offline, stay in sync across browser tabs or native windows through `QueryClient::broadcast`, and can be fetched during server rendering with `with_ssr`. The default `browser` feature wires up the browser's timers and events; native guests enable `guest` instead, and the `db` feature refetches the queries a `leptoskit_db` live change touches. `leptoskit_rpc` builds its client calls on it.

#### leptoskit_virtual: headless virtualization

_Packages:_ 🔴 _leptoskit_virtual_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_virtual` is headless list virtualization in the style of TanStack Virtual: fixed or measured sizes, overscan, aligned scroll targets, and a window over the shared renderer.

It decides which items a viewport intersects so a ten-thousand-row list only builds the visible rows, on web and native alike. Feed a `Virtualizer` the viewport size and scroll offset, render the window with `VirtualList` (vertical or horizontal), and hand back real sizes with `measure` as items are laid out.

#### leptoskit_table: headless tables

_Packages:_ 🔴 _leptoskit_table_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_table` is headless tables in the style of TanStack Table: typed columns, sorting, filtering, pagination, and a renderable grid of ARIA roles.

It computes the model, filtering, then sorting, then paginating, while the screen owns the state: store what `for_header_press`, `for_filter`, and `for_page` propose and rebuild. Render the grid with `DataTable`, or read `rows` and draw your own with leptoskit components; either way it behaves the same on web and native.

#### leptoskit_rive: Rive playback

_Packages:_ 🔴 _leptoskit_rive_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_rive` plays Rive animations that ship inside the app: it checks a `.riv` file before loading it, picks one animation or state machine to run, and sends typed state-machine inputs (booleans, numbers, and triggers) so an animation can react to the interface.

The shared `Rive` component uses it on the web through the `web` feature and on macOS, iOS, Android, Linux, and Windows through the `native` feature; the terminal has no Rive playback. Native playback draws vector scenes only and rejects files that use images, fonts, text, layout, scripts, or audio. It never fetches anything over the network, and it should only load files you trust.

#### leptoskit_rive_runtime: the pinned Rive runtime

_Packages:_ 🔴 _leptoskit_rive_runtime_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_rive_runtime` is the pinned, offline C++ dependency used by `leptoskit_rive` native playback.

Applications use `leptoskit_rive`; this crate exposes no application API. Its separate Cargo build lets changes to the Rust player or image adapter reuse the C++ compilation cache.

#### leptoskit_docs: documentation sites

_Packages:_ 🔴 _leptoskit_docs_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_docs` turns Markdown with leptos components into a static site with navigation, search, highlighting, and checked links.

The leptoskit book is built with it, and apps can use the same generator for their own docs. It is MDX-shaped but Rust-native: components are leptos components, and mistakes fail the build with a location.

#### leptoskit_macros: view!

and #[component]

_Packages:_ 🔴 _leptoskit_macros_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_macros` provides the `view!` markup macro and `#[component]` attribute that expand into the same builders a hand-written tree uses.

Use them through `leptoskit::prelude`. The macro parses rstml markup and emits builder calls, so markup and builder code produce one element tree and neither is a second-class path.

#### leptoskit_router_macros: the Route derive

_Packages:_ 🔴 _leptoskit_router_macros_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_router_macros` provides the `#[derive(Route)]` macro that reads a route struct's fields from the URL by name.

It is an implementation detail of `leptoskit_router`, which re-exports the derive. `Option<T>` may be absent, `Vec<T>` takes every value, and other types are required unless the field is `#[route(default)]`.

#### leptoskit_router: typed file routes

_Packages:_ 🔴 _leptoskit_router_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_router` is the runtime behind leptoskit file routes: each file in `src/routes/` becomes a variant of one generated `Route` enum, so URLs are parsed into typed values and links are written from them, and the two cannot drift apart.

`leptoskit dev`, `leptoskit build`, and `leptoskit routes` generate the enum; the app then holds a `Router` to read the current route, `navigate`, `replace`, go `back`, and build links that render as real `<a href>` links on the web. Each page renders inside the layouts of the folders above it. Natively the history lives in memory; the `web` feature keeps the browser's address bar and back button in step. Routes match from most to least specific, and parameter structs use `#[derive(Route)]`.

#### leptoskit_plugin_router: file routes as a build plugin

_Packages:_ 🔴 _leptoskit_plugin_router_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_router` is the file router as a leptoskit plugin: it generates the typed `Route` module from the files under `src/routes/` before cargo runs, and answers `plugin:router routes`.

Turn it on with a `[[plugins]]` entry named `router` in `leptoskit.toml`; `leptoskit dev`, `build`, and `native` then rewrite `src/routes.rs` only when the routes change, and `leptoskit plugin:router routes --check` fails when the committed module is stale. Apps don't depend on this crate, since the CLI links it; the runtime and the `Route` derive live in `leptoskit_router`.

#### leptoskit_router_codegen: the routes generator

_Packages:_ 🔴 _leptoskit_router_codegen_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_router_codegen` reads a leptoskit `src/routes/` directory and writes its typed `routes.rs`.

The `leptoskit routes` command and the CLI's build pipeline use it; apps never depend on it. It reports every problem with its file and line, and never overwrites a file leptoskit did not write.

#### leptoskit_db_schema: schema diff and migrations

_Packages:_ 🔴 _leptoskit_db_schema_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_db_schema` holds the schema model, snapshot format, diff, and SQL renderers behind leptoskit's generated database migrations.

`leptoskit db` and the `leptoskit_db` runtime use it; apps never depend on it. It is pure - no connection and no files - and renders Postgres and SQLite SQL from the same plan, with each step's risk assessed against existing rows.

#### leptoskit_db_macros: the database macros

_Packages:_ 🔴 _leptoskit_db_macros_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_db_macros` provides the macros behind leptoskit's database layer: `#[db::table]`, `#[db::link]`, `#[db::view]`, `#[db::policy]`, `#[db::mixin]`, `#[db::polymorphic]`, `db::migrations!`, and the `Shape`, `Enum`, and `Scalar` derives.

Apps use them through `leptoskit::db` or `leptoskit_db`; this crate is the macro implementation. Every mistake it can catch is a compile error that says what to write instead.

#### leptoskit_db: tables as Rust structs

_Packages:_ 🔴 _leptoskit_db_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_db` is leptoskit's database layer: tables declared as Rust structs, typed IDs, client-safe public projections that leave out private and secret columns, typed queries and writes, links, access policies, and migrations.

Enable `db` on `leptoskit` for the client side, which has no database code, and `db-postgres` or `db-sqlite` on the server; `db-geo` and `db-vector` add geospatial and vector columns. `leptoskit db` diffs the tables and writes migrations, and `leptoskit db studio` browses the data in development. Auth and IDP build their schema packs on it.

#### leptoskit_auth: sign-in and sessions

_Packages:_ 🔴 _leptoskit_auth_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_auth` is sign-in for leptoskit apps, stored in the app's own database: the `auth` schema pack, revocable sessions, email codes, authenticator apps with recovery codes and step-up, passkeys, third-party providers, opt-in passwords, Solana wallets, account merging, and the signed-in user that access policies read.

Enable `auth` on `leptoskit`, plus `auth-passkeys`, `auth-providers`, `auth-passwords`, `auth-solana`, `auth-smtp`, or `auth-webhook` in the server's crate for the parts that need them. The headless client works on the web and in the generated iOS, Android, and macOS apps, where it uses the passkeys, web-auth, secure-store, and HTTP plugins. `leptoskit_idp` turns the same accounts into an OIDC provider.

#### leptoskit_idp: an OIDC provider for your app

_Packages:_ 🔴 _leptoskit_idp_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_idp` lets other apps sign in with your app: an OAuth 2.1 and OpenID Connect provider built on the app's own database, sessions, and access policies.

It turns the accounts `leptoskit_auth` already manages into an identity provider, without a second user store. Authorization, consent, and token issuance stay in the app.

#### leptoskit_extension: browser extensions

_Packages:_ 🔴 _leptoskit_extension_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_extension` is the runtime and typed WebExtensions bindings behind leptoskit browser extensions for Chromium, Firefox, and Safari.

Apps enable the `extension` feature, name entry points with `entry!`, and build with `--platform extension`. APIs return `Unavailable` instead of throwing, and listeners are removed when their handle drops.

#### leptoskit_devtools_protocol: the DevTools wire format

_Packages:_ 🔴 _leptoskit_devtools_protocol_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_devtools_protocol` holds the versioned serde messages shared by the DevTools agents, hub, interface, and MCP tools.

It has no platform code: the browser, native host, guest, and front end all speak the same protocol. A version mismatch is refused and both versions are named.

#### leptoskit_plugin_devtools: the guest-to-host DevTools bridge

_Packages:_ 🔴 _leptoskit_plugin_devtools_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_devtools` is the bridge plugin that carries a native guest's DevTools data to the host that already reports the tree.

Apps never select it. The shared runtime installs it in `leptoskit dev` builds, and the host merges the guest's route, state, forms, and queries into the session it reports.

#### leptoskit_devtools: the DevTools agents

_Packages:_ 🔴 _leptoskit_devtools_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_devtools` contains the agents that report a running app to the local DevTools hub: the web page, the native host, and the Wasm guest.

Under `leptoskit dev`, the CLI starts the hub and these agents open sessions, mirror trees, and carry commands back. Hooks expose the route, named state, forms, and query caches that only the app knows.

#### leptoskit_l10n: typed translations

_Packages:_ 🔴 _leptoskit_l10n_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_l10n` provides typed translations in the style of Flutter's `gen_l10n`: locales, negotiation, and plural rules behind a generated messages module.

Messages live in one ARB file per locale; `leptoskit l10n` writes the typed module, and app code calls getters instead of parsing ARB at runtime. It works in SSR, the browser, and native guests.

#### leptoskit_l10n_codegen: the translations generator

_Packages:_ 🔴 _leptoskit_l10n_codegen_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_l10n_codegen` reads a leptoskit `locales/` directory of ARB files and writes its typed translations module.

The `leptoskit l10n` command and the CLI's build pipeline use it; apps never depend on it. It checks every translation against the template and reports placeholder, plural, and naming problems with a file and line.

#### leptoskit_cli: the command line

_Packages:_ 🔴 _leptoskit_cli_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_cli` is the `leptoskit` and `lk` command: scaffold a project with `new`, run every platform at once with `dev`, then `build`, `serve`, and `test`. It also writes routes, translations, and database migrations, generates sign-in screens and plugins, ships code push releases, uploads to the app and extension stores with `publish`, drives devices and simulators (also over MCP), and lints and formats the code.

Deployment lives here too: `infra` manages OpenTofu infrastructure, `deploy` releases a commit or rolls back, `console` hosts or connects to the self-hosted deployment service, or pairs a worker with the hosted LeptosKit app, and `verify` reproduces backend builds and checks signed deployment policies. New projects resolve their development secrets through Monosecret, and `leptoskit dev --no-secrets` uses the existing environment. The binary is a thin wrapper over the library, so tooling can drive the same commands directly.

#### leptoskit_fmt: the view!

formatter

_Packages:_ 🔴 _leptoskit_fmt_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_fmt` is the formatter behind `leptoskit format`: rustfmt for Rust and a layout engine for the markup inside `view!`.

rustfmt leaves `view!` markup alone, so this fills the gap without a second Rust formatter: it lays markup out with the project's `rustfmt.toml` width and indentation, copies text exactly, and keeps every comment. The `[format]` table in `leptoskit.toml` sets its style, `--check` fails CI when a file would change, and `--stdin` serves editors.

#### leptoskit_lint: project conventions

_Packages:_ 🔴 _leptoskit_lint_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_lint` contains the rules behind `leptoskit lint`: components carry `#[component]`, and component trees follow the markup style `leptoskit.toml` selects.

rustc and clippy check Rust; these rules check the conventions a leptoskit project relies on, with diagnostics that name the file, line, and fix.

#### leptoskit: the one dependency

_Packages:_ 🔴 _leptoskit_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit` is the crate an app depends on. It re-exports the components, Tailwind styles, layout engine, renderer, and Wasm runtime, with one feature per optional part, so an app names `leptoskit` alone and never keeps the versions of its parts in step by hand.

Pick how the interface runs: `web` with `csr`, `hydrate`, `ssr`, or `islands` renders through leptos, and a backend feature (`macos`, `ios`, `android`, `linux`, `windows`, or `tui`) commits the same tree as real widgets or as terminal cells. `server` adds the Axum toolkit, and `analytics` beside it keeps request and browser measurements. Everything else is opt-in: `rpc`, `query`, `form`, `aria`, `table`, `virtual`, `l10n`, `rive`, `db` with `db-postgres` or `db-sqlite`, `auth` (passwords, passkeys, and providers through their own `auth-*` features), `idp`, `extension`, `codepush`, and `devtools`. `leptoskit::secrets` reads values that Monosecret, the default secret manager, or a trusted host supplies.

### Features

#### Deploy, back up, and roll back on your own servers

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #505](https://github.com/leptoskit/leptoskit/pull/505)

Describe your servers in an `[infra]` table in `leptoskit.toml`, and `lk infra` (also `lk tofu`) and `lk deploy` take the app from there to running Linux hosts. They provision Hetzner or DigitalOcean servers behind a Cloudflare Tunnel, an R2 bucket that serves signed code push updates, and a private R2 or Backblaze bucket for backups, then run the app and Postgres in containers with Kamal:

```sh
lk infra generate
lk infra plan
lk infra apply
lk infra build --version release-001 --push
lk deploy --setup --version release-001
```

- `lk infra apply` applies only the plan you saved and reviewed, and refuses a configuration that changed since. `lk infra offers` lists current prices, and `[infra.budget]` stops a plan whose compute would cost more per month than you allow.
- Several app hosts update one at a time. An optional NATS relay for live connections and a bounded Valkey cache run on the private network; Postgres, NATS, and Valkey stay on one primary host.
- Scheduled backups are encrypted with Restic, restored into a throwaway database to check them, and pruned by your retention settings. `lk infra restore` keeps the previous database beside the restored one.
- Each image carries its migration history, which is checked before the app starts, and each release records its image digest. `lk deploy --rollback <tag>` checks both against the live database. Rolling back a schema change needs reviewed `down.sql` files, stopped writers, and `--rollback-schema --yes`; a failed schema change is never reversed automatically.
- `lk infra ci` writes a GitHub Actions workflow that builds an image for each push to `main`, tagged with its commit, and deploys it.

`lk db migration create` now writes a `down.sql` beside each new `migration.sql`; existing migrations and their checksums are left alone. `lk db rollback` and `Db::rollback` reverse applied migrations newest first, each in its own transaction, and refuse to start when a reverse is missing or still marked `-- leptoskit: irreversible`.

`leptoskit generate deploy` also deploys a client-rendered app whose backend and browser client are separate packages (`build.client_package`). A NATS relay URL that carries a user name and password now passes them to the client instead of to the connection address.

#### Measure your deployed app with the analytics package

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #505](https://github.com/leptoskit/leptoskit/pull/505)

When a deployed route starts failing, you need its error counts, latency, and release in one place. Turn on the `analytics` feature of `leptoskit` on your server and add `[infra.plugins.analytics]` to `leptoskit.toml`:

```toml
[infra.plugins.analytics]
version = 1
environment = "production"
browser_routes = ["/", "/notes/{id}"]
```

The server then measures every HTTP request in bounded per-minute windows inside its own process, with W3C request correlation. `server::Timer` measures a named database, RPC, job, or live operation. An optional browser module records page views, navigation timing, and error counts for the routes you list, without cookies, query strings, or user identifiers, and keeps those reports apart from the server's own measurements. `lk infra analytics` reads one instance's measurements through a key-protected JSON API, and `lk infra plugins` lists what the plugin needs. It adds no containers or cloud resources.

To keep history across instances, connect the app to a deployment service, or set `otlp_endpoint` to export them to an OTLP/HTTP metrics backend.

#### Run your own deployment service

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #505](https://github.com/leptoskit/leptoskit/pull/505)

`lk console serve` runs one service, on a trusted Linux or macOS worker, that holds the deployment dashboard, a queue of deployment jobs, an API for agents, and the analytics history of every instance of your app. Connect an app once, and the ordinary commands run through it, so a deployment keeps going after you close your terminal:

```sh
lk console serve --project /srv/notes --url https://deploy.example.com   # on the worker
lk console connect --url https://deploy.example.com                       # in your checkout
lk infra plan
lk infra apply
lk deploy
```

It reuses the `lk infra` and `lk deploy` engine. Credentials stay on the worker and in a private file outside your repository. `apply` runs only the plan this checkout reviewed, and `lk deploy` builds the current Git commit in a temporary checkout on the worker. An operation interrupted by a restart is marked as such and waits for you to reconcile it, rather than being retried blind. `lk console token` issues project-scoped `read`, `deploy`, and `ingest` keys for agents and apps.

Apps with the analytics plugin get their ingestion key and endpoint automatically. The service keeps up to seven days of measurements per project, alongside process, cgroup, and filesystem samples from each host, and can export metrics to an OTLP/HTTP backend instead. `lk console backup` and `lk console restore` back up and restore the service's own database; a restore revokes the keys it brings back.

#### Manage deployments from a hosted account

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #505](https://github.com/leptoskit/leptoskit/pull/505)

A hosted LeptosKit account keeps deployment requests, application measurements, and OpenTofu state in one place, managed from one app that runs on the web, iOS, and Android. Accounts are created by the service's operator, and subscriptions go through Stripe Checkout and the Stripe customer portal.

Deployments run on a dedicated worker that connects out to the service, so it needs no public address. Pair it with a one-use code from the app, then use the usual commands:

```sh
lk console worker --url https://deploy.example.com --project /srv/notes --data /srv/notes-worker
lk console connect --url https://deploy.example.com
lk deploy
```

Each account's data and keys are kept apart, and no two customers share a worker. With `infra.hosted_state = true`, OpenTofu state is stored encrypted, versioned, and locked by the service. Agent keys can deploy but can't manage billing, pair workers, or apply infrastructure changes. Workers run on your own infrastructure, or the operator can provision a dedicated DigitalOcean installation for you after reviewing its configuration and cost. A worker that stops partway through an operation doesn't repeat it automatically; it waits for the operator to reconcile it.

#### Native packages and a per-platform doctor

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #521](https://github.com/leptoskit/leptoskit/pull/521)

Plugins can now wrap third-party native SDKs the way the platform expects, and apps never run a package manager by hand.

A plugin's Swift Package Manager dependencies were already declared in its `Package.swift`; they now resolve automatically on both iOS and macOS, documented as the way to wrap SDKs like Firebase's Swift package. A library that ships only as a pod is declared in the plugin's runtime metadata instead:

```toml
apple-pods = [
	{ name = "FirebaseCore", version = "~> 12.0" },
]
```

leptoskit generates the Podfile, runs `pod install` before every iOS build (and before an App Store archive), and compiles the plugin's Swift into the app target where the pods are linked. CocoaPods is required only while a selected plugin declares pods — never otherwise — and `leptoskit doctor` reports exactly when it is. Pods are iOS-only; macOS builds with Swift packages.

`leptoskit doctor` now checks each native platform the project targets: Xcode, the iOS simulator runtime, and CocoaPods on iOS; Swift on macOS; Java 17+, the Android SDK, platform 36, an NDK, CMake, and adb on Android; GTK 4 through pkg-config on Linux; plus every Rust target each platform compiles for. Inside a project it reports only the platforms the app ships; `leptoskit doctor --all` checks everything this machine could build. `leptoskit new` now points at it.

Native builds are also quieter where the tools shout and clearer where they were silent: xcodebuild runs with `-quiet` and without index-store chatter, Gradle runs with a plain quiet console, and the Linux, Windows, terminal, and web builds no longer drop the Rust compiler's warnings — and errors — that cargo's JSON message stream carried.

`android-dependencies`, the equivalent Maven mechanism for Android plugins, is now documented alongside.

- **Record and check offline backend bundles.** Add `leptoskit verify record` and `leptoskit verify check` to inventory prepared backend source, dependencies, infrastructure, build declarations, secret names and executable files. Checks require an independently trusted inventory SHA-256 and reject changed, missing or extra entries and symbolic links. These commands check local bundle integrity; they do not attest the build or verify a live backend. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #508](https://github.com/leptoskit/leptoskit/pull/508)

#### Require independent source approvals for code push releases

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #511](https://github.com/leptoskit/leptoskit/pull/511)

Add `[codepush.provenance]` so native apps install a code push release only when enough independent approvers say they rebuilt it from one commit of a public GitHub repository and got byte-identical files. Approvals are a second trust root, separate from the publisher keys: each is signed by an approver over its exact statement, under its own signing context, so a publisher signature can't stand in for one. Devices check approvals before downloading a release and again before every launch, refuse approvers that name different commits, count each approver once, and ignore approvals from any other key, including the publisher's. The bundle shipped in the store build needs none, and without the table nothing changes.

Add `leptoskit codepush approve`, which signs an approval only when a rebuilt directory holds exactly the release's files, `leptoskit codepush attach`, which adds approvals to stored releases without re-signing them, and `leptoskit codepush keygen --approver`. `publish` and `serve` carry each release's approvals at `approvals/<bundle hash>.json`. An approval says who rebuilt identical files from a commit; it is not a security review.

#### Amend drafting between releases

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #530](https://github.com/leptoskit/leptoskit/pull/530)

`[db] draft = "amend"` makes `leptoskit db migration create` rewrite the newest unreleased migration from the snapshot before it instead of stacking a new one, so between releases there is one draft that keeps up with the tables — and the release's `migration consolidate` has one entry to fold or nothing to do. Data migrations (`--empty`) still stack, and a development database that applied the draft's earlier shape is told to run `db reset --yes` to come back in line.

#### Migration consolidation at releases

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #528](https://github.com/leptoskit/leptoskit/pull/528)

`leptoskit db migration consolidate` folds every migration after the latest release marker into one, written from a fresh diff of the release's baseline to the current tables: add-then-drop vanishes, rename chains collapse, and intermediate SQLite table rebuilds never happen. The consolidated migration carries a `replaces.json` naming what it folded, and every database converges on its own — one that already applied the folded migrations records the consolidated one without running it, while a fresh one runs it once; a database that applied only part of the set is refused by name. A migration whose SQL no longer matches what its snapshots generate — a data migration, or one written by an older leptoskit — is refused unless it is named in `--keep`, which leaves it and the migrations after it in place, after the consolidated one.

- **Filters that follow links, totals, and distinct reads.** `Query::aggregate` totals a column over the matching rows — `Sum`, `Avg`, `Min`, or `Max` of a column in one typed statement, with the table's policies applied and `None` when no row matches, as SQL returns. A link column's `has` follows it: `post.author_id.has(|user| user.handle.eq("ada"))` becomes a correlated `EXISTS`, in `filter` and in `!` for its negation, and an optional link that is `NULL` matches nothing. Text columns match regular expressions with `matches_regex` — Postgres's `~`; queries on SQLite refuse it rather than diverge. `distinct()` reads each row once, which matters for views and naturally-keyed tables, and `count()` counts after it. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #533](https://github.com/leptoskit/leptoskit/pull/533)

#### Release markers for database migrations

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #527](https://github.com/leptoskit/leptoskit/pull/527)

`leptoskit db release record <version>` writes a marker into `migrations/registry.txt` that freezes every migration above it as published, pinning the SHA-256 of the schema the release ends at. Editing a frozen migration's snapshot is refused the next time the migrations are read, and a marker with a typo in it never silently unfreezes a release. `leptoskit db status` names the release that froze what, and how many migrations came after it; those are the drafts the next release will consolidate.

- **Wall-clock times, durations, and sequences.** Columns can now hold a `chrono::NaiveDateTime` or `chrono::NaiveTime` — a calendar's date and time or a time of day in no time zone, `timestamp` and `time` on Postgres and ISO 8601 text on SQLite — and a `chrono::Duration`, stored as ISO 8601 duration text (`PT90M`, `-PT36H`) on both databases, so a grace period or a lead time round-trips exactly, sign and fraction included. `#[db(sequence)]` on an `i64` makes the database number the rows in order (`GENERATED BY DEFAULT AS IDENTITY` on Postgres); inserts don't name the column, and SQLite projects are told to key by an `Id` or keep a counter of their own, with a way out in the message. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #535](https://github.com/leptoskit/leptoskit/pull/535)
- **Sign which origins serve a deployment.** Add `leptoskit verify keygen`, `leptoskit verify sign`, and `leptoskit verify deployment`. A project signs a deployment policy listing its origins and the verified releases they may serve, and serves it at `/.well-known/leptoskit/deployment.json`. Anyone can check an origin against the project's published key: a copied website fails because its origin is not listed, a forged policy fails its signature, and `--state` refuses an older or conflicting policy. The check does not show what a server actually runs. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #510](https://github.com/leptoskit/leptoskit/pull/510)
- **Develop and test extensions in Firefox.** Select Firefox with `--browser firefox` when running `leptoskit dev` or `leptoskit test extension`. Development builds use a temporary add-on installed through WebDriver BiDi, reload it after a rebuild, and check page surfaces, background startup, and content script injection. `leptoskit doctor` reports Firefox and geckodriver. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #488](https://github.com/leptoskit/leptoskit/pull/488)

#### Add password fields to the generated sign-in screens

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #494](https://github.com/leptoskit/leptoskit/pull/494)

With `[auth.passwords] enabled = true`, `leptoskit generate auth` now writes password fields into the screens. The sign-in screen offers "Sign in with a password" and resets a forgotten password with an emailed code; the security screen sets, changes, and removes the password, with the server's minimum length beside the field. The fields are `PasswordInput`s marked `current-password` or `new-password`, so password managers fill and save them on the web and in native apps.

Turning passwords on later and running `leptoskit generate auth` again adds the fields to screens you haven't changed, and leaves the ones you have alone unless you pass `--force`. Turning them off never takes fields away.

In app code, `leptoskit::auth::client::sign_in_with_password()` takes an `EmailPassword` (the address and password), alongside `EmailCode` for emailed codes.

#### Package the hosted pilot for a Linux server

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #507](https://github.com/leptoskit/leptoskit/pull/507)

Build the broker, public site, console and documentation as one checksummed Linux release, with a systemd service that keeps persistent data separate from releases. Drain broker requests on termination so service restarts can finish active requests.

Use `[infra.external]` to provision a separately supervised service with the existing OpenTofu integration, including DigitalOcean, Cloudflare routing, the price guard and saved-plan protection, without requiring the managed Postgres/Kamal runtime.

#### Serve the docs book under a strict CSP

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #505](https://github.com/leptoskit/leptoskit/pull/505)

Two `leptoskit_docs::Site` changes let you serve the book from your own site:

- `Site::external_fonts(false)` stops the book loading its fonts from Google Fonts, for a site that supplies local fonts in its own stylesheet or has to work offline.
- The script that applies a reader's saved light or dark theme is now a same-origin file instead of an inline script, so the book works under a content security policy that forbids inline scripts.

The LeptosKit website is built this way: it serves the landing page, a live shared component example, the deployment console at `/console/`, and the whole book at `/docs/`, with local fonts and the Rive runtime packaged alongside. The management app's **Documentation** button opens that book on the web, iOS, and Android.

#### Use Monosecret for development secrets

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #487](https://github.com/leptoskit/leptoskit/pull/487)

New projects declare their database and sign-in secret names in `monosecret.toml`. `leptoskit dev` resolves that manifest through Monosecret before starting, with `--no-secrets` for hosts that supply the environment. Existing projects without a manifest keep their current startup. The secret management guide recommends Monosecret for server and deployment credentials.

`leptoskit::secrets` exposes Monosecret 0.5's small value-access client on native and Wasm targets. Native servers can read named environment values; Wasm hosts provide values explicitly. Default provider and runtime features are disabled, keeping the resolver, network clients, and provider dependencies out of application bundles.

#### Draw password fields in generated apps

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #494](https://github.com/leptoskit/leptoskit/pull/494)

The macOS, iOS, and Android apps `leptoskit new` generates now draw `PasswordInput`, as an `NSSecureTextField`, a secure `UITextField`, and a password `EditText`. Before, a password field stopped those apps with an "unsupported widget" error.

`autocomplete` hints now reach native password managers too. `username`, `email`, `current-password`, and `new-password` set the field's content type on macOS and iOS, its autofill hint on Android, and its input type on HarmonyOS, in the generated apps and in the AppKit, UIKit, and Android backends. `NativeProps::text_content` reads the hint as a `TextContent`. A `PasswordInput` turns off autocorrection, spell checking, and suggestions on iOS and Android, and an Android password field without a hint is offered to autofill as a password.

- **Build and reproduce backends from public commits.** Add `leptoskit verify build`, which builds a backend from a public GitHub commit in a digest-pinned container: the commit's exact files, its locked dependencies, and two compilations with networking disabled that must produce identical executables. It writes the bundle, its inventory, and a SLSA provenance statement. `leptoskit verify reproduce` lets anyone fetch the same commit and dependencies, rebuild, and confirm the published executable byte for byte. Neither command shows what a live server runs. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #509](https://github.com/leptoskit/leptoskit/pull/509)

#### Add generated artwork to the documentation book

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #505](https://github.com/leptoskit/leptoskit/pull/505)

`leptoskit_docs::Site::generated_asset` adds a file your build generates, such as SVG artwork or a stylesheet, to the book under `_leptoskit/generated/`, without writing it into the book's sources. The file is kept through development-server rebuilds, and an invalid file name fails the build before any output changes.

The LeptosKit website and book use it for their hand-drawn look: borders, hatching, and artwork drawn at build time with skribble's rough engine, in a paper-and-ink palette for light and dark modes, with locally served fonts.

#### A GPU canvas for custom drawing

_Packages:_ 🟠 _leptoskit_canvas_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #545](https://github.com/leptoskit/leptoskit/pull/545)

The new `leptoskit_canvas` crate draws the kind of graphics no native widget offers, such as charts, progress rings, and cards with soft shadows and gradients. You describe the drawing as a `Scene`, an ordered list of commands in the style of React Native Skia, and its renderer draws it on the GPU with wgpu (Metal on Apple platforms, Vulkan on Linux and Android, Direct3D 12 on Windows, and WebGPU in the browser).

A scene can fill rectangles with a radius per corner and an optional border, cast CSS-style box shadows, paint with solid colours or linear and radial gradients, fill vector paths with the non-zero or even-odd rule, stroke them with caps and joins, and nest clips, transforms, and layers drawn at an opacity. Edges stay smooth at any scale. A scene is plain data, so a Wasm guest can build one and send it to its host, and the renderer bounds every value before drawing, so a scene from untrusted code cannot make the host allocate without limit.

The crate is standalone in this release: no component uses it yet. `render_to_rgba` draws a scene offscreen and returns its pixels, which suits tests and generated images.

- **Theme tokens in leptoskit.toml and deeper CSS var().** `[theme.colors]` and `[theme.lengths]` in `leptoskit.toml` name design tokens that `Theme::install_maps` loads into the process-wide Theme map. Arbitrary color and length utilities accept `var(--name)`, one nested `var()` fallback, and `/opacity` on color vars. The web adapter prepends `Theme::css()` so the browser sees the same `:root` custom properties native resolves through Theme. Tailwind `@theme` / `tailwind.config` files remain unsupported. _Packages:_ 🟠 _leptoskit_native_, 🟠 _leptoskit_config_, 🟠 _leptoskit_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #538](https://github.com/leptoskit/leptoskit/pull/538)
- **CSS color variables and a Theme map.** Arbitrary color utilities can use `var(--name)` or `var(--name, fallback)`. The web stylesheet keeps the CSS function. Native paint resolves the name through `Theme::set_color` (or the fallback when the theme has no entry). `Theme::css` emits a `:root` sheet of the same names for the browser. Nested `var()`, non-color variables, and opacity modifiers on `var()` stay unsupported. _Packages:_ 🟠 _leptoskit_native_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #536](https://github.com/leptoskit/leptoskit/pull/536)

#### Try native plugins written only in Rust

_Packages:_ 🟠 _leptoskit_config_, 🟠 _leptoskit_bridge_, 🟠 _leptoskit_bridge_ffi_, 🟠 _leptoskit_plugin_clipboard_, 🟠 _leptoskit_plugin_haptics_, 🟠 _leptoskit_cli_

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #506](https://github.com/leptoskit/leptoskit/pull/506)

Set `native.experimental-rust-plugins = true` in `leptoskit.toml` to use the experimental pure-Rust `clipboard` and `haptics` plugins on iOS, macOS, and Android in place of their Swift and Kotlin versions. Without the setting, builds keep the Swift and Kotlin versions.

A plugin of your own can do the same in its `[package.metadata.leptoskit.runtime]` tables. With `rust-context = true`, its factory receives a `leptoskit_bridge::AppContext`, whose `platform()` lends the host's native handles: the `JavaVM` and app `Context` on Android, and the on-screen view controller on iOS and macOS. With `rust-main-thread = true`, its calls run on the platform's UI thread, which UIKit, AppKit, and most Android UI APIs require (`App::install_main` installs such a plugin by hand). Mark a platform's registration `experimental = true` to use it only when the setting is on.

### Fixes

- **Preview the deployment dashboard with example data.** `leptoskit_deploy_console` keeps the dashboard's original design study beside the working deployment service. Its `model`, `store`, and `view` modules and its `preview` example render the dashboard's screens (deployment history, rollback and schema checks, deployment targets) from built-in example data, and export them as an interactive HTML page. The preview needs no server or credentials and never touches your cloud accounts; to deploy, run the service with `lk console serve`. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #505](https://github.com/leptoskit/leptoskit/pull/505)
- **Render Android form controls and dismiss the keyboard.** Generated Android apps now render Switch, Checkbox, Radio, Slider, and progress indicators using native Android widgets. The shared native event bridge accepts checked-state and numeric-value events. Tapping outside an editor clears focus and hides the keyboard in both Android renderers. Generated apps move between inputs with the keyboard Next action and dismiss the keyboard on Done. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #519](https://github.com/leptoskit/leptoskit/pull/519)

#### Render common controls in generated Apple apps

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #520](https://github.com/leptoskit/leptoskit/pull/520)

Generated iOS and macOS apps now render switches, checkboxes, radio options, sliders, progress bars, and activity indicators. Preserve their state through binary transactions, send toggle and value-change proposals to the guest, and expose native selection and value accessibility.

Tapping outside an input dismisses the keyboard while preserving button actions. Return ends editing and submits; guest-controlled focus can move to another field. Scroll views support interactive keyboard dismissal.

Focus requests on iOS no longer reopen the keyboard after dismissal on an unrelated render. macOS controls restore the state accepted by the guest after each proposal.

- **Field keeps its draft, and the gallery fits a phone.** Typing in Field's compose screen no longer clears each keystroke: the screen now owns its draft, keeps it when you leave the editor and come back, and has a Back button. The gallery's browser preview now fits the available width, so its controls stay reachable on phone-sized screens. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #518](https://github.com/leptoskit/leptoskit/pull/518)

#### Harden sign-in, merges, passwords, and the provider

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #498](https://github.com/leptoskit/leptoskit/pull/498)

`leptoskit_auth`:

- Starting a passkey or wallet sign-in no longer cancels everyone else's. These sign-ins share one subject before anyone is known, so a script that kept starting them could break sign-in for every visitor.
- An account that is disabled or locked can no longer be merged into another. Before, merging let a banned account shed its ban, and let someone take guesses at a locked account's second factor.
- Setting or changing a password counts against the account's password limit. A password reset counts against the per-IP limit before it asks the breach service. Argon2 runs at most two hashes per core at once.
- A refresh token traded again within the 30-second grace works only once. If a thief makes that trade, the app's own token ends the session when it is next used, and `refresh_reused` is recorded.

`leptoskit_idp`:

- Scope lists are de-duplicated in linear time; before, a long list cost quadratic time.
- Every authorization parameter is capped at 4 KiB, so pushed requests can't store megabytes.
- A DPoP proof or client assertion with an extreme `iat` or `exp` is refused instead of overflowing.
- A code redeemed by two requests at once ends the tokens the winning request got, as a later replay already did.

#### Harden the CLI against web pages and leaked secrets

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #489](https://github.com/leptoskit/leptoskit/pull/489)

- `leptoskit mcp http` now refuses requests a web page could send: any request with an `Origin` or `Sec-Fetch-Site` header, a `POST` whose content type isn't `application/json`, and a `Host` other than the address it listens on. Before, a site open in your browser could post to it and run the `control` tool. Send `content-type: application/json` with every message. Binding it to a non-loopback `--address` now prints a warning, since it has no authentication.
- The Firefox source archive leaves out symlinks that point outside the project, and `.jks`, `.keystore`, and `.pfx` files, so `leptoskit publish firefox` can't upload a linked credential or an Android keystore.
- A malformed publish credentials file reports the line that is wrong, without quoting it, so the secret on that line isn't printed.
- `leptoskit publish macos` names its `.app`, `.zip`, and `.dmg` from the app's name with path separators replaced, so a name like `AC/DC` stays in the output folder instead of naming a path elsewhere.
- Branch database names no longer panic when a long app name has a multi-byte character where the name is cut.
- The release workflow `leptoskit new` writes now reads the default branch from the event (`$GITHUB_DEFAULT_BRANCH` was never set, so the check failed every tagged release), and checks that manual runs start from a commit on the default branch too.

#### Fix a query deadlock, stale async results, and routing

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #493](https://github.com/leptoskit/leptoskit/pull/493)

Fixes in the client libraries:

- `leptoskit_query`: a watcher that reacts to a query starting to fetch, by cancelling, refetching, or invalidating it, no longer freezes the app. The fetch used to hold a lock while it notified watchers.
- `leptoskit_use`: `use_async_state` keeps the newest result. When two runs overlap and the older one finishes last, its result no longer replaces the newer one.
- `leptoskit_router`: a route under a catch-all, such as `[...path]/edit.rs`, is now reachable. `[...path]/index.rs` used to match `/a/edit` first.
- `leptoskit_l10n`: `negotiate` keeps looking through every following preference in the same language before settling for a language-only match. `[en-GB, en-AU, en-US]` now gets `en-US` from `[en, en-US]` instead of `en`.
- `leptoskit_virtual`: measuring an item at its estimated size no longer recomputes the position of every item after it. Measuring the rows that scroll into view in the middle of a 100,000-item list used to recompute 50,000 positions; now it recomputes none.
- `leptoskit_aria`: `Field::wire` replaces the field's own help and error references instead of adding them again, so wiring an input twice no longer makes a screen reader read its help twice, and an error the field no longer shows is no longer referenced.

- **Refuse older code push releases after losing the newest.** In the generated iOS and Android apps, a device remembers the newest update it has confirmed in a file of its own. Before this change, a damaged update file or a disk error made the device forget it had ever updated, so a stale or hostile `index.json` could install an older release that was still validly signed. The device now refuses anything at or below the newest release it confirmed, until a new store build of the app ships its own bundle. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #500](https://github.com/leptoskit/leptoskit/pull/500)

#### Close an upsert race and keep large writes working

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #501](https://github.com/leptoskit/leptoskit/pull/501)

- An upsert under row policies now locks the row it would overwrite before checking the update policy, and inserts plainly when there is none. Before, on Postgres, an upsert racing another transaction's insert of the same unique value could overwrite that row, `author_id` included, when only the insert policy had been checked.
- A commit of more than 500 rows to a SQLite table with a read policy no longer breaks every live feed that checks that policy. SQLite allows at most 500 terms in one compound `SELECT`.
- Bulk updates, soft-delete cascades, and supertype removals check and change their rows in batches of 500. They no longer fail past the databases' bind limits, or overflow the stack on tables keyed by two columns.
- The studio no longer filters or sorts by `secret` columns, which could read a secret back one character at a time.

#### Move the repository to the `leptoskit` organization

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #522](https://github.com/leptoskit/leptoskit/pull/522)

The project now lives at `leptoskit/leptoskit`. Crate metadata, repository links, badges, the documentation's repository link, and the release workflow's trusted-publishing identity name the new organization.

The public site stays at <https://leptoskit.dev/>, and configuration schema URLs are unchanged. Release downloads and the installed agent skill keep working through GitHub's repository redirects.

- **Allow hosted accounts to subscribe again after cancellation.** Bind replacement Checkout completion to the pending reservation before selecting the new subscription. Delayed events from the old subscription no longer reject the new one or clear its pending checkout, and repeated deliveries remain idempotent. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #507](https://github.com/leptoskit/leptoskit/pull/507)
- **Return from signing out only to the app that asked.** `leptoskit_idp`: `/oauth/end_session` follows `post_logout_redirect_uri` only when the app named by the ID token in `id_token_hint` registered it, or, without an ID token, the app named by `client_id`. In every other case people land on your signed-out page, and `state` is dropped. That covers an unregistered address, a request that names no app, and a `client_id` that names a different app than the ID token. These requests used to get an error page. Now a valid ID token signs the person out in these cases too, where the error used to leave them signed in. A forged ID token is still refused. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #502](https://github.com/leptoskit/leptoskit/pull/502)

#### Commit only what the guest changed

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #543](https://github.com/leptoskit/leptoskit/pull/543)

A native app's commit no longer rebuilds the whole interface. The Wasm host records which nodes the guest changed since its last commit, and the renderer resolves and diffs only those parts, so updating one label in a 1,000-row list takes about 0.08 ms instead of 1.15 ms on an M-series Mac. Trees that use ids, `aria-labelledby` or `aria-describedby`, radio groups, or live regions still commit in full, because those resolve across the whole tree.

`NativeSurface::try_patch` is the new entry point for hosts that track their own changes.

- **Close live connections that exceed their frame budget.** A live WebSocket connection that sends frames faster than the server serves them now receives the rate-limit error and is then closed. Before, it got the error but stayed open. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #487](https://github.com/leptoskit/leptoskit/pull/487)
- **Deliver a live socket's last frames before it closes.** When the server closes a live WebSocket, such as one sending frames faster than it serves them, it now reads what the client is still sending until the client answers the close, for up to five seconds, before dropping the connection. Previously the connection could end in a reset that discarded the server's last frames, so on Windows a client lost the reason it was closed along with the close itself. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #497](https://github.com/leptoskit/leptoskit/pull/497)
- **Report missing desktop pedometer hardware.** The Apple pedometer plugin now answers a valid availability query with `false` on macOS and validates malformed requests. Permission and step-reading calls remain unavailable, and closing the plugin still cancels later calls. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #487](https://github.com/leptoskit/leptoskit/pull/487)

#### Bound SQLite memory and fix secure storage and clipboard

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #495](https://github.com/leptoskit/leptoskit/pull/495)

- The SQLite plugin stops a statement whose working memory grows by more than 64 MiB, and refuses temporary tables, views, indexes and triggers. Before, a `DISTINCT` over generated values, or temporary tables filled one call at a time, could exhaust the app's memory, because neither counted against the page quota.
- `SecureStorage` stores keys it hashes under a `sha256-` prefix. Before, the hex digest of a key such as `auth/token` was itself a readable key, so the two read and wrote the same secret.
- The Android clipboard plugin reads only an item's text. It no longer opens a content URI that another app placed on the clipboard, which read that app's provider on the UI thread with no size limit.

#### Launch the public website

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #523](https://github.com/leptoskit/leptoskit/pull/523)

`leptoskit-site` gains `--public-only`, the packaging mode behind <https://leptoskit.dev>. It produces the same polished landing page with its live Wasm component example, the complete documentation book under `/docs/`, and the configuration schema aliases at their stable `/schema.json` and `/schemas/` paths — as plain static files, with no console entrypoint, account API, or billing backend. The public page renders in a new public mode: navigation and calls to action lead to documentation and source, and the cloud section explains the CLI's deployment engine instead of offering the operator-enrolled pilot. The default packaging keeps building the complete hosted artifact with the console, so the pilot scripts and broker are unchanged; a reused output directory drops the previous console entrypoint when you switch to public-only.

The documentation book and the workspace now publish under the custom domain: the book's default URL and every brand and documentation link move from the old GitHub Pages host to `https://leptoskit.dev` (book pages under `/docs/`, schemas at their existing paths), and the docs workflow builds and deploys the real public-only artifact to GitHub Pages.

The live example's static button waits for the Wasm client to mount before accepting clicks, so the first interaction is not lost during loading.

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Packages:_ 🟢 _leptoskit_testing_library_, 🟢 _leptoskit_yoga_, 🟢 _leptoskit_use_, 🟢 _leptoskit_native_, 🟢 _leptoskit_testing_, 🟢 _leptoskit_tui_, 🟢 _leptoskit_appkit_, 🟢 _leptoskit_uikit_, 🟢 _leptoskit_android_, 🟢 _leptoskit_gtk_, 🟢 _leptoskit_windows_, 🟢 _leptoskit_aria_, 🟢 _leptoskit_analytics_, 🟢 _leptoskit_deploy_console_, 🟢 _leptoskit_form_, 🟢 _leptoskit_config_, 🟢 _leptoskit_plugin_, 🟢 _leptoskit_bridge_, 🟢 _leptoskit_bridge_ffi_, 🟢 _leptoskit_build_, 🟢 _leptoskit_plugin_crypto_, 🟢 _leptoskit_plugin_os_, 🟢 _leptoskit_plugin_clipboard_, 🟢 _leptoskit_plugin_haptics_, 🟢 _leptoskit_plugin_battery_, 🟢 _leptoskit_plugin_device_, 🟢 _leptoskit_plugin_secure_store_, 🟢 _leptoskit_plugin_linking_, 🟢 _leptoskit_plugin_passkeys_, 🟢 _leptoskit_plugin_web_auth_, 🟢 _leptoskit_plugin_filesystem_, 🟢 _leptoskit_plugin_network_, 🟢 _leptoskit_plugin_posture_, 🟢 _leptoskit_plugin_dialog_, 🟢 _leptoskit_plugin_document_picker_, 🟢 _leptoskit_plugin_screen_orientation_, 🟢 _leptoskit_plugin_light_sensor_, 🟢 _leptoskit_plugin_pedometer_, 🟢 _leptoskit_plugin_accelerometer_, 🟢 _leptoskit_plugin_application_, 🟢 _leptoskit_plugin_autostart_, 🟢 _leptoskit_plugin_barometer_, 🟢 _leptoskit_plugin_brightness_, 🟢 _leptoskit_plugin_gyroscope_, 🟢 _leptoskit_plugin_http_, 🟢 _leptoskit_plugin_keep_awake_, 🟢 _leptoskit_plugin_localization_, 🟢 _leptoskit_plugin_magnetometer_, 🟢 _leptoskit_plugin_permissions_, 🟢 _leptoskit_plugin_single_instance_, 🟢 _leptoskit_plugin_menu_, 🟢 _leptoskit_plugin_tray_, 🟢 _leptoskit_plugin_speech_, 🟢 _leptoskit_plugin_sqlite_, 🟢 _leptoskit_plugin_store_, 🟢 _leptoskit_plugin_image_manipulator_, 🟢 _leptoskit_plugin_screen_capture_, 🟢 _leptoskit_plugin_store_review_, 🟢 _leptoskit_plugin_sharing_, 🟢 _leptoskit_plugin_tracking_transparency_, 🟢 _leptoskit_plugin_websocket_, 🟢 _leptoskit_plugin_arguments_, 🟢 _leptoskit_plugin_shell_, 🟢 _leptoskit_plugin_logging_, 🟢 _leptoskit_plugin_notifications_, 🟢 _leptoskit_plugin_local_authentication_, 🟢 _leptoskit_plugin_location_, 🟢 _leptoskit_plugin_global_shortcut_, 🟢 _leptoskit_plugin_window_state_, 🟢 _leptoskit_sensor_, 🟢 _leptoskit_codepush_, 🟢 _leptoskit_wasm_, 🟢 _leptoskit_axum_, 🟢 _leptoskit_rpc_codegen_, 🟢 _leptoskit_rpc_macros_, 🟢 _leptoskit_rpc_, 🟢 _leptoskit_query_, 🟢 _leptoskit_virtual_, 🟢 _leptoskit_table_, 🟢 _leptoskit_rive_, 🟢 _leptoskit_rive_runtime_, 🟢 _leptoskit_docs_, 🟢 _leptoskit_macros_, 🟢 _leptoskit_router_macros_, 🟢 _leptoskit_router_, 🟢 _leptoskit_plugin_router_, 🟢 _leptoskit_router_codegen_, 🟢 _leptoskit_db_schema_, 🟢 _leptoskit_db_macros_, 🟢 _leptoskit_db_, 🟢 _leptoskit_auth_, 🟢 _leptoskit_idp_, 🟢 _leptoskit_extension_, 🟢 _leptoskit_devtools_protocol_, 🟢 _leptoskit_plugin_devtools_, 🟢 _leptoskit_devtools_, 🟢 _leptoskit_l10n_, 🟢 _leptoskit_l10n_codegen_, 🟢 _leptoskit_cli_, 🟢 _leptoskit_fmt_, 🟢 _leptoskit_lint_, 🟢 _leptoskit_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)

#### Fix a rounded-view tap crash; speed up list updates

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #490](https://github.com/leptoskit/leptoskit/pull/490)

Tapping a fully rounded view that clips its content, such as a `rounded-full overflow-hidden` avatar, could crash the app when the view sat at a fractional position. Hit testing on every platform, and the macOS `Pressable`, now handles that corner.

Lists and conditional views update faster in native apps:

- A `For` or `Show` that changes now rebuilds only its own children. It used to copy every node on the screen first, so toggling a `Show` beside 1,000 cards took 3 ms and now takes 4 µs.
- Removing many children at once costs their number rather than its square: clearing 1,000 cards takes about half as long.
- Two changes inside the same fixed-size cell, such as a label whose text and style both change, now lay out only that cell instead of the whole screen.
- On Linux, applying a batch of changes reconfigures the widget tree once per batch instead of once per change, so building a large screen no longer slows down with the square of its size.

#### Harden the Wasm runtime and code push against abuse

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #495](https://github.com/leptoskit/leptoskit/pull/495)

- A guest that imports the wasm-bindgen externref table shim with a reference-typed signature can no longer forge a function reference. Calling through one used to panic the interpreter, which aborted the macOS app.
- Plugin payload copies now cost fuel, so a guest looping over `submit` and `cancel` runs out of fuel instead of copying megabytes per instruction and freezing the app.
- Calls to the console-warn shim are logged at most 16 times per guest.
- Class strings longer than 4 KiB are parsed but no longer cached, so a guest can't pin hundreds of megabytes in the Tailwind parse cache.
- A synchronous Rust plugin that panics on the macOS UI thread now fails that call, and every later call to that plugin, with a native error instead of aborting the app.
- Code push reads `index.json` with a 2 MiB limit instead of 32 MiB, and `CodePushClient` caps rebuilt and downloaded containers at 32 MiB by default instead of 512 MiB. Custom `Transport`s can override the new `fetch_within` to stop reading at that limit.
- A signed release that the installed app can't run is remembered and isn't downloaded again on every check. A new store build forgets the list.
- iOS caches at most 256 colors and 64 fonts, so animating arbitrary colors or text sizes no longer grows memory without limit.

#### Close a channel bypass and bound live connections

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #496](https://github.com/leptoskit/leptoskit/pull/496)

A live connection could join any channel instance by naming its key, even though the server only checked whether the caller could join the instance its arguments described. Members of one room could then see another room's presence and signals and send signals into it, including across servers through the relay. The server now requires the key to be the join arguments it authorized. Two channels whose arguments look alike (`Chat { room }` and `Doc { room }`) no longer share a room.

Live connections are also bounded in more ways:

- 64 channels per connection.
- Join arguments of at most 4 KiB, and presence state of at most 16 KiB.
- At most 16 actions running at once.
- A resume repeats each subscription only once.
- Frames that don't parse count against the frame budget.
- A WebSocket message far over the 1 MiB frame limit closes the connection before it is buffered.
- A client that stops reading is disconnected after 30 seconds (`Live::stall_timeout`), instead of making the server hold everything meant for it.

Reusing a subscription ID for another query no longer returns the earlier query's result when the caller's context runs alone.

`rpc::paginate` reads at most `rpc::MAX_PAGE_ROWS` (1,000) rows per page and `rpc::MAX_PINNED_ROWS` per pinned page, whatever `limit` the client sends.

`leptoskit_axum`'s servers now close a connection that hasn't finished sending its request headers within `HEADER_READ_TIMEOUT` (30 seconds). Without that timeout, a few slow clients could hold every socket the server had.

#### Fix terminal wide text, resizes, and long lists

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #491](https://github.com/leptoskit/leptoskit/pull/491)

Terminal apps draw correctly in three cases they used to get wrong:

- Colored CJK text and emoji no longer overwrite each other. Each wide glyph now lands in the two cells it occupies.
- Resizing the terminal redraws everything on screen. Text that did not move used to stay blank until it changed.
- A scroll view whose content is taller than 32,767 rows, such as a long `VirtualList`, keeps showing its rows once scrolled.

A long scrolled list also renders faster: rows outside the screen are no longer wrapped or drawn. Rendering 10,000 rows into a 40×20 terminal takes 0.1 ms instead of 3 ms.

- **Blink f32 leftover-carry for flexible tracks.** Flexible grid tracks now distribute free space with binary32 flex shares, flooring each share to a 1/64 CSS-pixel layout unit and carrying the truncated fraction into the next track—matching Chrome’s `getBoundingClientRect` for equal and weighted `fr` rows/columns. Non-integral percentages such as `w-1/3` floor into a layout unit; whole-number percentages stay exact. _Packages:_ 🟢 _leptoskit_yoga_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #541](https://github.com/leptoskit/leptoskit/pull/541)
- **Yoga LayoutUnit rounding for Chrome grid parity.** Fractional grid tracks and percentage lengths resolve through Blink-style 1/64 CSS-pixel layout units, so Yoga frames match Chrome `getBoundingClientRect` for the shared Tailwind class trees. The grid parity probe covers nested grids, dense auto-flow, minmax, flex edges, absolute insets, percentages, and overflow. _Packages:_ 🟢 _leptoskit_yoga_, 🟢 _leptoskit_native_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #540](https://github.com/leptoskit/leptoskit/pull/540)
- **Paint italic text and per-edge borders on Linux and Android.** `border-t`, `border-l-2`, and the other edge border utilities now get the same default stroke color as a uniform `border`, and GTK and Android paint each edge instead of leaving those strokes undrawn. The `italic` utility also draws through GTK labels and Android text views, matching the web and the terminal. AppKit, UIKit, and Win32 still leave per-edge borders undrawn and italic upright. _Packages:_ 🟢 _leptoskit_native_, 🟢 _leptoskit_android_, 🟢 _leptoskit_gtk_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #534](https://github.com/leptoskit/leptoskit/pull/534)
- **Use Objective-C message forwarding for UIKit text input traits so debug builds can create text and password fields without crashing.** _Packages:_ 🟢 _leptoskit_uikit_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #516](https://github.com/leptoskit/leptoskit/pull/516)
- **GTK grid parity probe against Yoga frames.** Adds a `grid_parity_gtk` example that mounts the same Tailwind grid/flex fixtures as the web probe on real GTK 4 widgets under Xvfb, asserts allocations match Yoga within half a CSS pixel, and writes PNGs for cross-device layout evidence. _Packages:_ 🟢 _leptoskit_gtk_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #542](https://github.com/leptoskit/leptoskit/pull/542)
- **Reserve private operation logs before launching the deployment command so a repeated operation cannot execute again when its logs already exist.** _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #516](https://github.com/leptoskit/leptoskit/pull/516)
- **Resolve deployment Git operations against the registered repository even when the CLI runs from a Git hook.** _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #516](https://github.com/leptoskit/leptoskit/pull/516)
- **Explain how to connect to the deployment service when its saved credential is missing on Unix.** _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #516](https://github.com/leptoskit/leptoskit/pull/516)
- **Paint italic text and per-edge borders on HarmonyOS.** The ArkUI host now applies `italic` and per-edge `border_widths` (`border-t`, `border-l-2`, …) the same way Android and GTK do, instead of leaving those strokes undrawn and italic upright. Deno render tests lock the Yoga frames from `grid_parity_probe` so absolute ArkUI placement stays aligned with the web HTML probe. _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #539](https://github.com/leptoskit/leptoskit/pull/539)
- **Run recovery SQL without reading psql startup files, so local formatting settings cannot break backup verification.** _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #516](https://github.com/leptoskit/leptoskit/pull/516)
