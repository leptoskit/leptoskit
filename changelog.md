# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-25)

Grouped release for `release`.

### Features

#### Interfaces from agents

_Owner:_ Ifiok Jr. · _Introduced in:_ [a874d28](https://github.com/leptoskit/leptoskit/commit/a874d281f230304cf9736e0d7e4e943101c82e9e)

`leptoskit_agent_ui`, also `leptoskit`'s `agent-ui` feature, renders interfaces an AI agent writes as text while your app runs. The agent answers in Markdown with components in it — `<Stat>`, `<Chart>`, `<Slider>`, `<Button>`, and more — declares variables with `<Var>` and formulas with `<Let>`, and writes `{formulas}` in text that follow the user's input without a new message. `<View>`, `<Text>`, and `<Pressable>` let it build layouts the other components don't cover. `AgentUi::view` follows a streaming answer and shows a component that is still open. Button presses reach `on_action` with the user's state, and the app decides what they do.

Documents style with Tailwind classes and colour roles such as `bg-surface` and `text-muted`, never colours. Each role reads your `[theme.colors]` entry of the same name, and its `-dark` twin in dark appearance, so documents and the built-in components match your app in light and dark without the agent knowing either; `agent_ui::Theme` renames roles and adds your own.

Sliders, switches, checkboxes, radios, and spinners are the platform's own controls, updated in place so a slider keeps its drag. `Widgets::WithoutControls` draws them from buttons for macOS, iOS, and Android shells generated before 0.3, and `Widgets::Basic` also leaves out vector paths for HarmonyOS.

Documents are data, never code: formulas run on a fixed budget, classes that would lay content over the app or scroll are refused, images load only from allowed addresses, and `Limits` bound every document. `AgentUi::reference` writes the language reference for the agent from the components and colour roles registered right now, and `AgentUi::check` lists a document's problems by line and column.

- **Answer analytics batches once they're queued.** By default the analytics service answers a batch once its events are stored, so every answer means they're kept. An app that prefers speed can set `LEPTOSKIT_ANALYTICS_ACK=queued` on the service: it answers as soon as the events are queued, and a crash loses at most what was queued. The queue's limits still apply, and a full queue still answers `503`. _Owner:_ Ifiok Jr. · _Introduced in:_ [5290ee1](https://github.com/leptoskit/leptoskit/commit/5290ee14349dc7cb53bf525baaf54cf434686359)

#### Route metrics are now `leptoskit_apm`

_Owner:_ Ifiok Jr. · _Introduced in:_ [8c6fcd3](https://github.com/leptoskit/leptoskit/commit/8c6fcd3281c78e3b9a9839c3698803275e421676)

The in-process route metrics that `leptoskit_analytics` held (minute aggregates, request correlation, the browser module at `/_leptoskit/analytics.mjs`, and OTLP export) are now the `leptoskit_apm` crate, and `leptoskit`'s `apm` feature. The name `leptoskit_analytics` is moving to the analytics service, a product-analytics service plugin.

Update your `leptoskit` dependency's features from `analytics` to `apm`, and `leptoskit::analytics::…` to `leptoskit::apm::…`. Both old names still work for this release. The endpoints, `[infra.plugins.analytics]`, and `lk infra analytics` are unchanged.

#### Record analytics events from Rust

_Owner:_ Ifiok Jr. · _Introduced in:_ [e30c2ee](https://github.com/leptoskit/leptoskit/commit/e30c2eea0cbe146a830166992bf2d8f0a78fbd97)

`leptoskit_analytics::client` records events, people, and groups, the same way on every platform, and depends on nothing, so it adds no JSON, UUID, time, or compression crate to an app. It batches what's recorded, retries with backoff when the service can't be reached, sends smaller batches when one is too large, and drops the oldest events past its limits, reporting how many it dropped.

On the app's server, the `server-client` feature adds `client::server::Analytics`. It reads the service's address and token from what `leptoskit dev` and `leptoskit serve` set, records events without waiting, and sends them in the background:

```rust
let analytics = Analytics::from_env().expect("the app runs the analytics service");
analytics.capture(&user_id, "invoice_paid", props! { "amount_cents" => 1999 });
```

The browser and native transports follow in a later release.

#### Keep typed analytics capture within the core browser budget

_Owner:_ Ifiok Jr. · _Introduced in:_ [86f0827](https://github.com/leptoskit/leptoskit/commit/86f0827cf2b493a7473b4ba73c0b6a603c2fad8d)

`#[derive(Event)]` and `capture_event` send named properties by default. Enable `leptoskit_analytics`'s `typed` feature to send compact positional values instead, with catalogue negotiation and named retries. Server clients and the service enable this codec automatically. Beacons and saved native queues keep names in either mode.

The browser size check includes a derived event and the capture, identity, group and consent APIs, counts all JavaScript snippets as well as Wasm, and keeps the 12 KiB core limit. `setup/scripts/analytics-size.sh --compact` also measures the optional codec. Browser API calls use small JavaScript helpers to reduce the Wasm and bindings downloaded by apps.

#### Record analytics events in native apps, offline too

_Owner:_ Ifiok Jr. · _Introduced in:_ [deec4c0](https://github.com/leptoskit/leptoskit/commit/deec4c016022e16f3776ea5fc0059d5d84eeabbc)

With the `native` feature, `leptoskit_analytics::client::native` records events from a native app's interface and sends them to the app's server through the HTTP plugin:

```rust
use leptoskit_analytics::client::native;
use leptoskit_analytics::props;

native::init(native::Options::new("https://app.example")).await;
native::capture_screen("Settings", props! {});
native::capture("exported", props! { "format" => "pdf" });
```

It sends every 30 seconds or 20 events. What it hasn't sent is kept in the app's private files, so events recorded offline, or just before the app closed, are sent the next time it runs, recorded at the time they happened and stored once. Call `native::flush().await` when the app goes to the background. The app needs the `http`, `filesystem`, and, for unpredictable ids, `crypto` plugins.

- **Store route timings in the analytics service.** The analytics service accepts application-performance metrics at `/_leptoskit/services/analytics/otlp/v1/metrics`, in the OTLP/HTTP JSON that `leptoskit_apm`'s exporter sends, so how long each route took is kept beside the product analytics. Each minute of each route is stored once, even when the exporter retries. Only the app may send them, signed with the service's token. Metrics the service doesn't store, such as cumulative histograms or gauges, are reported back as rejected, as OTLP expects. _Owner:_ Ifiok Jr. · _Introduced in:_ [67a5675](https://github.com/leptoskit/leptoskit/commit/67a5675f3e84d9ec221f18d7ba56706dc7d679e2)
- **Rate-limit analytics events per sender.** The analytics service limits how fast each `distinct_id` may send events: 100 a second, with bursts of 2,000, far above what a person's device sends. One stuck client or bot can no longer fill the queue for everyone else. Events over the limit are dropped, counted in the answer's `rate_limited`, and recorded as an ingestion warning. Events the app's server sends aren't limited. `IngestConfig::per_distinct_id` changes the limit, and `None` turns it off. _Owner:_ Ifiok Jr. · _Introduced in:_ [353b616](https://github.com/leptoskit/leptoskit/commit/353b6165d9f2c86e9c92284ec5022b32df9f35dd)

#### An analytics service that stores events

_Owner:_ Ifiok Jr. · _Introduced in:_ [1904827](https://github.com/leptoskit/leptoskit/commit/19048272e427597c79b3f9739ccfa9602092ce95)

`leptoskit_analytics` is now a service plugin: add `[services.analytics]` with `crate = "leptoskit_analytics"` to `leptoskit.toml`, and the app gets a process with its own database that records events.

Clients send events to `/_leptoskit/services/analytics/v1/batch` in a compact binary batch format, where the device's details are sent once per batch rather than with every event, or as JSON through the `capture` operation. The service answers once the events are stored. A retried batch is stored once, every refused event is reported with a reason, and a full queue answers `503` with `Retry-After` rather than dropping events.

The clients that send these batches, and the pages that read them, follow in later releases.

#### Typed analytics events are sent by position

_Owner:_ Ifiok Jr. · _Introduced in:_ [807eb2f](https://github.com/leptoskit/leptoskit/commit/807eb2f61bfa8b3bd57a680ebcbf6443c7746ce5) · _Last updated in:_ [86f0827](https://github.com/leptoskit/leptoskit/commit/86f0827cf2b493a7473b4ba73c0b6a603c2fad8d)

With the optional `typed` feature, a typed event (`#[derive(Event)]`) goes to the analytics service as its schema id and values, without property names, which makes it smaller on the wire. The service names the values from its event catalogue. The app's server fills that catalogue when it starts with every typed event it was built with, as the app, so no token reaches a browser or a native app.

The catalogue keeps every version of an event, so events recorded by an older build, even offline, still read correctly. When the service doesn't know an event's schema yet, it answers so and the client sends the event again with its names, so nothing is lost. An event whose values don't match its schema is refused with a reason. Untyped `capture("name", props)` works as before.

Browser, native and server clients handle unknown schemas and retry with names. Beacons and native snapshots retain names so delivery does not depend on a response or the previous app build's schema metadata. Older services that reject positional events receive one retry of the whole batch with names; unreadable acknowledgements keep events queued.

#### Declare analytics events as Rust types

_Owner:_ Ifiok Jr. · _Introduced in:_ [afcee4d](https://github.com/leptoskit/leptoskit/commit/afcee4df16ba6e3db2edbb01133105362663fff6)

With `leptoskit_analytics`'s `derive` feature, an app can declare an event as a struct, so a misspelled or retyped property is a compile error instead of a gap in the data:

```rust
#[derive(leptoskit_analytics::Event)]
#[event(name = "checkout_completed")]
struct CheckoutCompleted {
	plan: String,
	amount_cents: u32,
}

client.capture_event(CheckoutCompleted { plan: "pro".into(), amount_cents: 1999 });
```

Each field becomes a property of the same name. Without `name`, the event is named after the struct. The app's server records them with `Analytics::capture_event`. The derive refuses names the service would refuse (blank, longer than 200 bytes, or with control characters such as newlines) and names starting with `$`, which leptoskit's own events use, as well as tuple structs and enums, saying what to write instead. A field written `r#type` is the property `type`.

After initializing the browser or native client, record the same event with `client::web::capture_event` or `client::native::capture_event`. Both use the shared typed-event implementation.

#### Record analytics events in the browser

_Owner:_ Ifiok Jr. · _Introduced in:_ [d9a66d1](https://github.com/leptoskit/leptoskit/commit/d9a66d1e73141c14be24d3e57bcd4f1cf600bdca) · _Last updated in:_ [86f0827](https://github.com/leptoskit/leptoskit/commit/86f0827cf2b493a7473b4ba73c0b6a603c2fad8d)

With the `web` feature, `leptoskit_analytics::client::web` records events from a web app's pages to the analytics service on the app's own origin:

```rust
use leptoskit_analytics::client::web;
use leptoskit_analytics::props;

web::init(web::Options::default());
web::capture_pageview(props! {});
web::capture("signed_up", props! { "plan" => "pro" });
web::identify(&user_id, props! { "email" => email });
```

It remembers the person and whether they opted out in `localStorage`, sends every few seconds with `fetch`, and hands what's left to `sendBeacon` when the page is hidden or closed, so the last events aren't lost. It adds only `wasm-bindgen`, `js-sys`, and `wasm-bindgen-futures`, which a leptoskit web app already compiles, and one small JavaScript module.

The client's queue now also counts a batch on its way and the batches' shared details against its limits, and drops an event too large to ever send when it's recorded rather than holding it.

- **Route timings go to the analytics service by themselves.** When an app runs the analytics service and has route metrics turned on, its server now sends them to the service, where they're kept beside the product analytics, with no export settings to write. An export endpoint the app sets itself still wins. The service's token signs the export from the app's server and never reaches a browser. _Owner:_ Ifiok Jr. · _Introduced in:_ [8751905](https://github.com/leptoskit/leptoskit/commit/8751905651c4c0bcb2232eff3e6e13b402e73bf5)

#### Let people choose light or dark

_Owner:_ Ifiok Jr. · _Introduced in:_ [0e7d338](https://github.com/leptoskit/leptoskit/commit/0e7d338999ee2fe8e21ff267d506d2123200fd18)

`dark:` utilities followed only the device's appearance, so an app could not offer "Light", "Dark", and "Match device". `leptoskit::appearance::set(Appearance::Light)` (or `Dark`, or `System` to follow the device again) now switches every `dark:` utility at once, without a reload, and `appearance::get()` reads the choice reactively.

On the web, the choice marks `<html>` with `data-lk-appearance` and sets its `color-scheme`; the generated stylesheet applies `dark:` rules under `prefers-color-scheme: dark` unless the root says `light`, and always when it says `dark`. In a native app, the root of the next render carries the choice (`NativeProps::appearance`), and `NativeSurface` resolves `dark:` against it and passes the result to the backend's `set_dark_mode`, so native controls match. Hosts can also call `NativeSurface::set_appearance`, and read `NativeSurface::appearance` and `NativeSurface::is_dark`. Older shells ignore the choice and keep following the device.

Persisting the choice is the app's job: store `Appearance::as_str()` and call `appearance::set(stored.parse()?)` before the first render.

- **Build several frontends, servers and native destinations from one application's root `leptoskit.toml` and Cargo workspace.** Named targets select a frontend and explicit server package, isolate public inputs, sidecars, outputs and build records, and preserve application identity. Use `--app-target NAME` and `leptoskit targets`; destinations not yet implemented can carry an actionable unavailability reason. Native hosts can live under `platforms/`. macOS menu-bar mode keeps a status item alive and opens its native UI only when requested. _Owner:_ Ifiok Jr. · _Introduced in:_ [9262b5d](https://github.com/leptoskit/leptoskit/commit/9262b5d58a759c21e214adc78a4b8b9666f9a2fd)

#### Admin apps can sign in through your site's own sign-in

_Owner:_ Ifiok Jr. · _Introduced in:_ [5046004](https://github.com/leptoskit/leptoskit/commit/5046004737871562f97079eb522c06bd01ce06bc) · _Last updated in:_ [7eea31d](https://github.com/leptoskit/leptoskit/commit/7eea31d056ac25f0d7f68cffe590fd858b2adaa8)

`leptoskit_idp` gains a standard `roles` scope: when your app registers a `Roles` hook, one of your own first-party apps may ask for it, and the person's roles from that hook land in its ID token and `userinfo` as `"roles": ["admin", …]`. Another site's apps are refused the scope at authorize, and an app acting as itself can't have it.

`leptoskit_auth` closes the loop for the app that signs in through yours:

- `Auth::roles(account)` reads the registered `Roles` hook, `auth::require_role("admin")` guards a leptos server function, `auth::WithRole<Admin>` is the axum extractor, and `Auth::require_role(&me, "admin")` the check behind them.
- `Provider::userinfo(true)` (and `[auth.providers.<id>] userinfo = true`) fetches `/userinfo` with the access token after the code exchange, checks its `sub` against the ID token's, and makes the merged claims available to a gate.
- `AuthBuilder::gate(AdminGate)` registers a `SignInGate`, which runs once per sign-in — through a provider, an emailed code, a password, a passkey, or a wallet — before any session or account is made, so a refusal creates nothing and the sign-in page sees the `not_allowed` error code. `SignInAttempt` names the method, the provider's subject, and the provider's claims, with `roles()` reading the `roles` claim.
- `[auth] methods = ["oidc:main"]` (`AuthBuilder::sign_in_methods`) turns every other way to sign in off: its routes answer 404 and the sign-in pages see none of it. The default is unchanged.

See "Use your site's sign-in for an admin app" in the sign-in-with-your-app guide.

- **Require independent backend release approvals.** Managed backend deployments can require independent signatures over the complete image descriptor and a current version-two deployment policy binding its exact digest and image identity. Reuse, rollback and recovery revalidate operator-owned trust and policy replay state without trusting stored verification flags. _Owner:_ Ifiok Jr. · _Introduced in:_ [08131a2](https://github.com/leptoskit/leptoskit/commit/08131a292dcedf866c63822835cc14678a2c4b1e)

#### Background execution for apps

_Owner:_ Ifiok Jr. · _Introduced in:_ [464514c](https://github.com/leptoskit/leptoskit/commit/464514c0620490ab633cd15b0457b858b8234fb5)

`leptoskit_plugin_background` keeps a LeptosKit app working in the background. On Android it runs a foreground `Service` with a persistent, low-importance notification (its own channel) and wakes the app's Rust `AppService` on a configurable interval through the host's existing service-wake path; a WorkManager periodic fallback restarts the service every 15 minutes, and an opt-in `BOOT_COMPLETED` receiver restarts it after a reboot. The service's `foregroundServiceType` is declared in the generated manifest from the plugin's metadata, with the matching `FOREGROUND_SERVICE_*` permissions, and a guest API asks the user to exempt the app from battery optimisation. On iOS the plugin registers `BGTaskScheduler` app-refresh and processing tasks that wake the `AppService`, with `UIBackgroundModes` and `BGTaskSchedulerPermittedIdentifiers` merged into the generated `Info.plist`. macOS, desktop, and the browser report `Unavailable`.

The guest API is `guest::Background::{start, stop, status, request_ignore_battery_optimizations}`, behind the `guest` feature. `status` reports whether background execution is running, the last wake, and the battery-optimisation state. The `AppService` now receives a wake reason through a new `set_wake_reason` hook, so a service can tell a platform background tick from an ordinary app call.

#### Read every sample as markup or builder calls

_Owner:_ Ifiok Jr. · _Introduced in:_ [333565b](https://github.com/leptoskit/leptoskit/commit/333565b060cfaaedd7a58fc1fec1e6abdd7bd71d)

Code samples on leptoskit.dev that build a component tree now have a Markup and Builder switch. Markup shows the tree as `view!` markup, as before; Builder shows the same tree as builder calls, like `View().class("p-4").child(Text("Hello"))`. One switch changes every sample on the page, and the choice is remembered on the landing page and throughout the docs. It applies before the page first draws, so a sample never flickers from one style to the other, and pages read as markup without JavaScript. Copy copies the style that is showing.

`leptoskit_docs` derives the builder form when it builds a book: each Rust sample with leptoskit `view!` markup gets both forms, laid out the way rustfmt lays out code, and leptos's own `view!` is left alone. Markup that has no builder form fails the build at its line and says why; write `markup-only` after the language, as in `` ```rust markup-only ``, to show a sample as written.

#### Embed the deployment service in your own app

_Owner:_ Ifiok Jr. · _Introduced in:_ [572d975](https://github.com/leptoskit/leptoskit/commit/572d975d32a8c424d231906efad19e16e0398822)

An application with its own sign-in can host the deployment service in-process instead of handing people owner keys. `leptoskit_deploy_console`'s `Service` gains a typed facade — `overview`, `enqueue`, `operation`, `fleet`, `issue_token`, `revoke_token` — that the HTTP handlers now call too, so scope, revision and idempotency rules have one implementation. `Principal` is public with `owner()`, `scoped(scope, project)` and `with_actor`, and the recorded actor appears on operations and in history through a paired migration.

`Service::subscribe(operation_id)` streams progress events and the terminal operation, `GET /api/v1/operations/{id}/events` serves the same updates as server-sent events (the dashboard follows the stream and falls back to polling), and `Service::api_router()` nests under another axum app's path prefix without the dashboard page or script. The deployment engine's `CommandRunner` is re-exported from `leptoskit_cli::console`, so an embedded service runs the same worker given the `lk` executable path and data directory.

#### Dart clients keep requiredness and nullability separate

_Owner:_ Ifiok Jr. · _Introduced in:_ [ae58b5e](https://github.com/leptoskit/leptoskit/commit/ae58b5ecb29336fb434fdb29f97cb7b16582c15b)

Generated Dart models and operation arguments no longer treat a nullable type as an optional one. A required nullable field or argument stays required and always writes its key, including an explicit `null`. Optional nullable values use the new three-state `Presence<T?>` wrapper: `const Presence<T?>.absent()` omits the key, `Presence<T?>.present(null)` sends `null`, and `Presence<T?>.present(value)` sends the value. Optional non-nullable values keep their `T?` shape, so `false`, `0`, empty strings, empty lists, and empty maps are still sent instead of dropped.

JSON and live payloads share the same presence-aware argument map, so HTTP calls, watched queries, live mutations, and channel join arguments round-trip missing, `null`, and real values. Query-string and form encodings cannot represent explicit `null`; `--strict` now reports that remaining encoding limitation instead of claiming the nullable field was lossless.

Regenerating changes the Dart signatures of optional nullable fields and arguments from `T?` to `Presence<T?>`, and required nullable constructor arguments become `required`. Update callers to pass `Presence.present(...)` or `Presence.absent()`, and read `.isPresent`/`.value` when consuming an optional nullable field.

The mandatory SDK acceptance gate now analyzes and runs generated Dart presence fixtures alongside every existing SDK journey. Release binary builds explicitly retain locked dependency resolution.

#### Explicit deployment graph ownership

_Owner:_ Ifiok Jr. · _Introduced in:_ [7c788ae](https://github.com/leptoskit/leptoskit/commit/7c788ae0d8522dc8f0576961ef4bcb9121603f70)

Add a shared graph model for independently released frontends and services. It checks route and artifact ownership, exclusive host placement, declared container-runtime requirements, and readiness dependencies. A release selection builds shared artifact inputs once while keeping unrelated release units out of the deployment selection.

Path mounts use segment boundaries, retain missing assets with their owning frontend, and redirect bare mount paths to their trailing-slash form. Frontends can partition one build output into separate release trees without publishing the same files twice.

Introduce explicit `infra.mode = "graph"` declarations with named artifacts, static/SSR frontends, per-service placement, readiness and rootless runtime requirements. Static projects do not inherit the managed preset's Postgres or backup settings. Ports are checked per host; ambiguous output paths, overlapping build outputs and conflicting bindings are rejected.

`lk infra graph --unit frontends.docs` inspects ownership and the exact release selection without building or contacting providers. `lk infra generate` writes per-host Hetzner/DigitalOcean resources and proxied Cloudflare DNS, with distinct HTTPS and Tunnel ingress. HTTPS origin rules use a recorded dual-stack Cloudflare range snapshot; static-only graphs add no database or backup resources. Artifact, provider and runtime evidence remains explicitly unchecked. Graph plan, apply and release remain unavailable until their lifecycle adapters and acceptance land.

Existing TOML retains its managed behavior and serialized shape. Rust callers constructing infrastructure directly now use `ManagedInfraConfig` and convert it into the `InfraConfig` lifecycle enum. Graph persistence, external service nodes, static publication and deployment remain follow-up work.

Single-app dev, serve, native and deployment commands refuse graph roots before selecting default targets or generating app files. Graph builds select qualified units with `lk build --unit frontends.docs`, run shared recipes once, and capture disjoint asset trees and executables with exact manifests. Service builds keep their Cargo features separate. The website packager accepts the invoking CLI and graph profile. These local build receipts do not claim isolated source reproduction or runtime deployment.

#### Hardware-bound signing keys gated by biometrics

_Owner:_ Ifiok Jr. · _Introduced in:_ [61db3d8](https://github.com/leptoskit/leptoskit/commit/61db3d891028868e60369f3bc365da207f6ea021)

`leptoskit_plugin_device_keys` is a new runtime plugin, `device-keys`, for hardware-bound P-256 signing keys that can never be copied off the device. `generate` creates a non-exportable key under an alias with a `Biometry` or `BiometryOrPasscode` policy; `sign` shows the operating system's biometric or passcode prompt and returns an ECDSA P-256 / SHA-256 signature, so a server can treat it as fresh proof that the device owner authenticated on this device just now. `public_key`, `delete`, and `available` round out the API.

Apple generates the key with `SecKeyCreateRandomKey` in the Secure Enclave (`kSecAttrTokenIDSecureEnclave`, `kSecAttrKeyTypeECSECPrimeRandom` 256-bit) with `SecAccessControlCreateWithFlags` using `.privateKeyUsage` plus `.biometryCurrentSet` or `.userPresence`, and signs with `SecKeyCreateSignature(.ecdsaSignatureMessageX962SHA256)` through an `LAContext`. Android generates in `AndroidKeyStore` with `setUserAuthenticationRequired(true)` and `setUserAuthenticationParameters(0, AUTH_BIOMETRIC_STRONG [| AUTH_DEVICE_CREDENTIAL])` for per-use authentication, `setInvalidatedByBiometricEnrollment(true)` for the `Biometry` policy, and `setIsStrongBoxBacked(true)` with a TEE fallback, then signs through a `BiometricPrompt` `CryptoObject`. A platform without the hardware, including the Secure Enclave-less simulator and browsers, reports `Unavailable`; there is no silent software-key fallback.

`sign` returns `KeyError::{NotFound, Cancelled, LockedOut, NotEnrolled, Invalidated, Unavailable, Failed}`, `generate` additionally `AlreadyExists`, and the public key is returned as both an uncompressed SEC1 point (65 bytes) and SPKI DER so a server can verify with the `p256` crate.

- **A book's logo can lead back to its site.** `Site::home` sets where the logo in a book's header leads, for a book that is one part of a larger site, such as `.home("/")` for a book under `/docs/` beside a landing page. Without it the logo still leads to the book's first page. The leptoskit.dev documentation uses it, so its logo now returns to the landing page. _Owner:_ Ifiok Jr. · _Introduced in:_ [30c2ac0](https://github.com/leptoskit/leptoskit/commit/30c2ac07138beed6d403be7dd6cfdcc91c5a581f)

#### Exact's examples on leptoskit

_Owner:_ Ifiok Jr. · _Introduced in:_ [e52be7d](https://github.com/leptoskit/leptoskit/commit/e52be7d083e3d2390f1a07400add44768a22af3d)

The example apps Exact publishes — Caltrain, Shelf, the calendar, the galleries, and the rest of that list, apart from the game engine — now run as leptoskit screens. `cargo run -p exact_examples -- <name>` prints a screen, and `leptoskit dev` in `examples/exact` serves them in the browser.

`cargo run --release -p exact_bench` times the same list workloads Exact publishes: the 10,000-row lists, the coin list, the extra-heavy feed, a Markdown document, and the feed from the Dioxus and web comparisons. The timings are headless build and commit costs on the machine that runs them.

#### Explicit PostgreSQL expansion windows

_Owner:_ Ifiok Jr. · _Introduced in:_ [d930fcd](https://github.com/leptoskit/leptoskit/commit/d930fcd18c1be26693f8e2b75925e13483e16f05)

Prepare a migration compatibility window with `lk db migration window --baseline <migration> --out <file>`. It checks committed schema snapshots and exact generated SQL, writes a new evidence file without overwriting an existing one, and prints its SHA-256. Preparing a window opens no database and changes no migration or checksum.

Applications that explicitly support the protocol can call `Db::migrate_in_window` with their own embedded schema and migrations. Both the old and the new build must match a checkpoint. A restarted old build accepts later history only inside that window. Unknown migrations, modified checksums, missing history, and development pushes still fail. Ordinary `Db::migrate` and reverse migrations keep their strict checks.

PostgreSQL applies all pending expansions and history entries in one transaction, with bounded lock and statement waits and a total attempt deadline. The first structural subset covers plain new tables, nullable columns without defaults, and comments. It does not certify application business logic, positional SQL, physical schema drift, or custom SQL. SQLite online migration windows are not supported.

Cancelling an ordinary PostgreSQL migration also discards its connection instead of returning a session lock or unfinished transaction to the pool. Completed operations reuse the connection only after the migration lock is released.

Generated database servers embed their latest schema checkpoint with `migration_schema!`, export their migration contract before configuration or database startup, and use `Db::migrate_startup`. Without a configured window startup remains strict. A window requires both `LEPTOSKIT_MIGRATION_WINDOW` and its independent `LEPTOSKIT_MIGRATION_WINDOW_SHA256` pin; invalid or incomplete configuration fails instead of falling back.

Verified image packaging can bind an exact window with `--migration-window` and `--migration-window-sha256`. It compares a confined export from the packaged executable against the reproduced source and includes both documents in the complete release descriptor, covered by its existing approvals and policy digest.

Managed releases deliver that approved window as a read-only bind mount with its exact startup pin. Digest-named files are never replaced; a conflicting file stops the operation. Before recording success, deployment checks the running container's environment, mount, and file digest. Automatic recovery restores the previous release's window configuration or removes the candidate's window when the previous release had none. Initial setup requires a release without a window so the baseline can be installed first.

These changes do not remove managed deployment's maintenance requirement. Automatic online admission, all-running-version coverage, and retiring compatibility windows still need the managed release workflow. A document hash is an integrity pin, not release authorization or runtime attestation.

- **Game host.** `leptoskit game` builds and runs a game: gameplay stays Rust (`Game::setup` and `Game::tick` on a fixed step), and menus are ordinary leptoskit components committed through the same native renderer as the other platforms. The world is drawn with wgpu, or on the CPU when the machine has no adapter. The sample is `game/lantern`. Enable it with the `game` feature. The host writes a PNG frame; it does not open a window or package a phone or browser app. _Owner:_ Ifiok Jr. · _Introduced in:_ [886e7d2](https://github.com/leptoskit/leptoskit/commit/886e7d20057f75301ee3b39ebe9992a35ef39e25)

#### Go clients preserve explicit zero values

_Owner:_ Ifiok Jr. · _Introduced in:_ [c023c0c](https://github.com/leptoskit/leptoskit/commit/c023c0c61388cca3291b4ef46460c0cc524461cb)

Generated Go clients use pointers for optional arguments and model fields, including slices and maps. Passing `nil` omits a field; passing a pointer to `false`, `0`, or an empty string includes that value. JSON and live calls also preserve empty collections; form and query encoding cannot express an empty collection. Previously `omitempty` could discard those values and cause the server to apply a default instead.

Regenerating changes Go source signatures: callers must pass addresses for optional values previously passed directly. The HTTP and live payloads share this representation. Optional nullable fields still have the null-versus-absence limitation reported by strict schema diagnostics.

#### Generate an edge configuration for each public graph host

_Owner:_ Ifiok Jr. · _Introduced in:_ [7c788ae](https://github.com/leptoskit/leptoskit/commit/7c788ae0d8522dc8f0576961ef4bcb9121603f70)

`lk infra generate` now writes `edge-<host>.json` beside the graph's provider configuration. Static frontends keep their declared path mounts and asset ownership; SSR frontends and public services use their own loopback ports. Missing nested assets cannot fall through to the landing page or another backend.

The generated Caddy configuration preserves query strings, checks origin TLS host identity, and derives forwarded identity from the configured ingress trust boundary. Graph services strip ambient cookies unless explicitly declared `trust = "first-party"`. SSR frontend sessions remain enabled. HTTPS accepts only the recorded Cloudflare ranges as trusted proxies; Tunnel listens on loopback and trusts the local tunnel connector.

Generation preserves operator edits unless `--force` is supplied. Use `lk infra release` for installation and activation. Certificates and provider deployment remain explicit prerequisites; generating files alone does not establish a running deployment.

#### Check graph artifacts before consuming them

_Owner:_ Ifiok Jr. · _Introduced in:_ [7c788ae](https://github.com/leptoskit/leptoskit/commit/7c788ae0d8522dc8f0576961ef4bcb9121603f70)

`lk verify graph --bundle <receipt> --sha256 <independent-digest>` checks a graph build against its exact manifest bytes and the current project declarations. It verifies file contents, modes, membership, unit placement, recipes, readiness dependencies and declared immutable toolchain pins without executing the supplied programs.

Use repeated `--unit` options with `--out <existing-parent>` to copy only selected units into a new private staging directory. The original manifest remains pinned; inputs are rechecked when consumed, and a failed check leaves no successful staging child. Source reproduction, approval, executable target compatibility and deployed runtime remain explicitly unchecked.

Build capture now uses the same bounded consumer before publishing its receipt. Both capture and staging request owner-only directory permissions at creation. Duplicate JSON keys, unexpected files, conflicting ownership, overstated verification evidence and aggregate metadata/artifact limits are rejected.

#### Add the health plugin: Health Connect and HealthKit reads

_Owner:_ Ifiok Jr. · _Introduced in:_ [ee9473c](https://github.com/leptoskit/leptoskit/commit/ee9473c2325dd58283fb6215310f114cd336bd1f)

Add a `leptoskit_plugin_health` crate with a typed binary guest client, HealthKit and Health Connect handlers, and automatic native registration under the runtime name `health`.

Read heart rate, resting heart rate, heart-rate variability (RMSSD on Android, SDNN on iOS, never converted), steps, sleep with stages, oxygen saturation, respiratory rate and exercise sessions, each with its source app and device. Permission query and request go through the platform's own flows, reads page through a token, and incremental sync stores an opaque change token: Health Connect's `getChanges` tokens, HealthKit anchors serialised inside it. Availability distinguishes a missing Health Connect app from one needing an update; macOS, other desktops and browsers report unsupported.

Plugins can now also declare manifest additions and signing requirements the CLI merges into generated hosts: `android-query-packages` for package visibility, `android-activities` for the activities and aliases a plugin's Kotlin sources need (Health Connect's permission-rationale entry points are the first user), and `apple-entitlements` the generated iOS app is signed with, dropping the entitlements file again when the last plugin needing one is deselected. The app still owns its `NSHealthShareUsageDescription` purpose string and any optional Health Connect background or history permissions it wants to request.

#### Linux apps get sign-in and the system plugins

_Owner:_ Ifiok Jr. · _Introduced in:_ [54e9bfb](https://github.com/leptoskit/leptoskit/commit/54e9bfb2db606ac28be1a3fc38c9bf6d2bc4826d)

The generated Linux app now registers `secure-store`, `web-auth`, `device`, `localization`, `network`, `battery`, and `linking`, besides the plugins written in Rust that it already ran.

- **Sign-in works on Linux.** Providers sign in through the default browser, which comes back to the app on a loopback address. The session is kept in the desktop's Secret Service keyring, such as GNOME Keyring or KWallet. Linux has no platform passkeys, so `passkeys` is left out there without a warning, and passkey calls answer `unsupported`.
- **New projects' Linux hosts get the plugins too.** `leptoskit new --auth`, `--db`, and `leptoskit generate auth` add sign-in's plugins, or the HTTP plugin, to `linux/Cargo.toml`, so a new app's Linux build reaches its server.
- **Snapshot plugins answer.** `device`, `localization`, `network`, and `battery` answer snapshots on Linux. Waiting for a battery or network change is still unavailable there. `linking` opens links in the default handler.
- **No zone is not an error.** `localization` reports no time zone, instead of failing, on a system with none configured, such as a container.

#### Generated Linux apps run runtime plugins

_Owner:_ Ifiok Jr. · _Introduced in:_ [790ca64](https://github.com/leptoskit/leptoskit/commit/790ca64e962a6b63e8336659bb69041f9b18cd6a)

The generated Linux app now registers the runtime plugins in `native.plugins` that are written in Rust, as the iOS, Android, and macOS apps already did. Add the plugin crate to `linux/Cargo.toml`, list it in `leptoskit.toml`, and `leptoskit build --platform linux` installs it. Plugins that answer at once run on the GTK thread, blocking ones on worker threads, and async ones such as `http` on a background runtime. Replies reach your screens on the GTK thread without polling. Plugins that keep files use the app's own folders under `~/.local/share` and `~/.cache`, named by `native.package_id`.

Plugins with only a Swift or Kotlin implementation, and app-owned services, are still left out on Linux with a warning, as is every plugin on Windows and in the terminal.

`leptoskit_bridge_ffi::app::PluginHost` is the plugin host that makes this work, for any runner that owns its `WasmHost`. `desktop_context` gives such a runner its app folders. A Linux app created before this release keeps building without plugins; to add them, compare its `linux/src/main.rs` with a new project's.

#### Motion for the shared component tree

_Owner:_ Ifiok Jr. · _Introduced in:_ [5dcd32a](https://github.com/leptoskit/leptoskit/commit/5dcd32a9c86e43c79db93710460964fce0a9a173)

Elements can move. `motion(Motion)` describes the poses an element moves between, as opacity, offset, scale, rotation, clipping, and blur, and what drives the movement: appearing (`Motion::enter`), scrolling into view (`in_view`), or a named container scrolling past (`timeline` on the container and `follow` on the elements inside it). `stagger` on a container starts its children's motions one after another. Timing comes from a duration and an `Easing`, or from a `Spring`, and a new `key` plays an enter motion again.

The web renders motion as CSS animations and scroll-driven animations, with springs as `linear()` easing, so nothing runs in script. People who ask for less motion, and browsers without scroll timelines, see every element at rest at once. Native platforms show the resting pose too: motion is web-only for now, and native apps never receive it, so adding it changes nothing they build.

The leptoskit.dev landing page is redesigned around it: one component tree becomes each platform's own widgets as you scroll, with every movement written as `Motion` and the hand-written animation script removed.

#### Animations that run on the platform

_Owner:_ Ifiok Jr. · _Introduced in:_ [f198f49](https://github.com/leptoskit/leptoskit/commit/f198f496a1103cd9ecc53aae034ab0c43d3df3de)

Widgets can now move, scale, and rotate, and animate changes without the app computing a single frame. Tailwind's `translate-*`, `scale-*`, `rotate-*`, `transition-*`, `duration-*`, `delay-*`, `ease-*`, and `animate-spin`/`ping`/`pulse`/`bounce` classes work, as does the `starting:` variant for the look a widget enters from. For anything else, elements take `transform`, `transition`, `keyframes`, `entering`, `exiting`, and `layout_transition` values from the new `leptoskit_animation` crate (re-exported as `leptoskit::native::animation`), with springs, timing curves, and presets modelled on React Native Reanimated.

The guest describes an animation once and the platform runs it: Core Animation in the generated iOS and macOS apps, Android's animators in the generated Android app, and CSS in the browser. The web doesn't run `exiting` or `layout_transition` yet, and Windows, Linux, the terminal, and HarmonyOS ignore transforms and animations for now.

#### Bundled images on every platform, and launch screens

_Owner:_ Ifiok Jr. · _Introduced in:_ [835dacf](https://github.com/leptoskit/leptoskit/commit/835dacf4f6e14ced1821384d112b5f8dca5730e0)

`Image("asset://portraits/ada.webp")` shows `assets/portraits/ada.webp` from your project on the web, iOS, Android, macOS, Windows, Linux, and HarmonyOS (experimental). The browser loads it from `/assets/`, where the web build already serves the folder, and `leptoskit build` now copies `assets/` into every native app: the iOS app bundle, the APK's assets, `Contents/Resources` on macOS, beside the Windows and Linux executable, and HarmonyOS's `rawfile`. Only changed files are copied again, deleted ones leave the app, and hidden files stay behind. Before, the native apps bundled the folder only when they showed 3D scenes. PNG, JPEG, and WebP decode natively; Linux needs the system's WebP loader for WebP. Assets ship with the store build, not with code push updates.

A missing asset stops a debug build with an error that names the source and where it looked, and a release build logs it and leaves the image empty. `leptoskit_native::asset` parses sources the same way on every host and refuses paths that leave the folder.

The audio plugin's `Source::Asset` reads the same copy, so sounds in `assets/` now play on native apps without copying them into the host project. On Android it reads `assets/<path>` inside the APK's assets, where the build puts them; a file you placed at the root of `android/app/src/main/assets/` by hand moves into an `assets/` folder there, or into the project's `assets/`.

`[app.launch_screen]` gives iOS and Android a launch screen: a `background` colour, an optional `background_dark`, and an optional square `image` (PNG or SVG, read like `app.icon`) drawn at `image_size` points or dp. iOS gets a colour set and image set in its asset catalog, named by `UILaunchScreen` in the generated `Info.plist`. Android gets a launch theme for the launcher activity, extending the application's own theme, with the splash screen attributes on Android 12 and later and a window background before; once the first screen draws, the generated app puts the theme's own background back.

#### Failures on a device are written to the platform's log

_Owner:_ Ifiok Jr. · _Introduced in:_ [4098fde](https://github.com/leptoskit/leptoskit/commit/4098fdeaad90eebb87c3ee635d5969bcb534c2fa)

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

_Owner:_ Ifiok Jr. · _Introduced in:_ [1bd0618](https://github.com/leptoskit/leptoskit/commit/1bd0618c284e4dff885524d3d6a23db9cfc668f8)

A generated native app's guest now has a real timer. `leptoskit_bridge::guest::sleep`, re-exported as `leptoskit::guest::sleep`, waits on the host instead of resolving at once, and `leptoskit_query`'s `guest` feature uses it for retry backoff, debounce, and cache expiry. Dropping the future cancels the host's timer, so an abandoned retry or debounced update costs nothing until its deadline.

The timer is the reserved `leptoskit.timer` plugin on the ordinary bridge, not a new mandatory Wasm import, so a bundle still loads on an older host. That host leaves the plugin unregistered and reports `Unavailable`; the query cache's `has_timer` reports `false` there rather than pretending a sleep finished. A custom provider installed with `install_sleep` still takes precedence, and browser builds keep using `setTimeout`.

Generated macOS, iOS, Android, Windows, Linux, terminal, and experimental HarmonyOS apps enable the timer. HarmonyOS schedules one ArkTS timeout for the next host deadline and cancels it when the runtime closes. Hosts bound the timer to 256 waiting or undelivered sleeps and reject a single bridge request longer than 24 hours. Query timers split longer waits into bounded requests. The new `try_sleep` operation reports unavailable or rejected timers: request retries surface their original error, intervals stop, and cache cleanup retains entries when a wait cannot finish. The infallible `sleep` never reports a failed wait as elapsed; it stays pending until dropped.

Live RPC connections also wait after a failed handshake. If a host cannot schedule the retry, the connection becomes offline until new work or credentials request another attempt; missing or full timer queues no longer trigger automatic retry loops.

Timer request and cancellation bursts are drained in bounded batches. If work remains after the per-turn budget, the host requests another immediate turn rather than waiting for an unrelated long timer.

- **Platform extension contracts.** Adds `leptoskit_platform`: versioned manifests, capability matrices, and scaffold file lists for niche OS shells. The CLI exposes a live builtin registry through `leptoskit native platform list|capabilities|scaffold`. Concrete in-tree shells land in follow-up crates; out-of-tree sidecars will reuse the same manifest shape later. _Owner:_ Ifiok Jr. · _Introduced in:_ [9713581](https://github.com/leptoskit/leptoskit/commit/9713581b0c4c54697f31a8c351349ff12558de91)
- **HarmonyOS platform extension crate.** Moves HarmonyOS ArkUI host templates and the capability manifest into `leptoskit_platform_harmonyos`. The CLI still runs prepare, NDK cross-compilation, and HAP packaging; `leptoskit native platform` lists the extension. Authors keep using `--platform harmonyos`. _Owner:_ Ifiok Jr. · _Introduced in:_ [132bec4](https://github.com/leptoskit/leptoskit/commit/132bec49ea418e88c90c065ba71ae13bc15770f6)
- **Meta Quest / Oculus platform extension.** Adds experimental `leptoskit_platform_oculus` host stubs and a capability manifest, registered with `leptoskit native platform`. Shipping headset packaging stays on first-party `quest`; this extension does not add an `oculus` `Platform` enum variant. _Owner:_ Ifiok Jr. · _Introduced in:_ [7afd5ed](https://github.com/leptoskit/leptoskit/commit/7afd5ed19d9dc827e679fb66fd3c530b8ccf3905)

#### Play the examples on the docs site

_Owner:_ Ifiok Jr. · _Introduced in:_ [dfd538f](https://github.com/leptoskit/leptoskit/commit/dfd538f7f7e87c872b5262b75f32935624d5d72e)

The screens on leptoskit.dev's examples page and home page were pictures. Each one is now the example's real web build, running in a sandboxed frame on the page: press, type, scroll, and navigate in it. The gallery's seven screens share one module, chosen with `?screen=`, so the browser downloads it once; Observatory and Postcard gained browser clients, and Field, the file routes example, plays too, with a back button that walks its own history. A player loads as it nears the screen, keeps the page from moving while it does, and switches to the screen's AppKit render, so the native views are one press away.

The gallery's dashboard range picker now chooses what the revenue chart covers: seven days, thirty days, or twelve months. The profile's Follow button follows and unfollows, and counts the follower.

- **A real Rust playground.** Add a standalone, container-isolated Rust compile service and a static playground editor with an opaque-origin browser preview. _Owner:_ Ifiok Jr. · _Introduced in:_ [aaa444b](https://github.com/leptoskit/leptoskit/commit/aaa444b0c4f5d264548fc7c771bf194ca7960044)

#### Plugin fixes from a line-by-line audit

_Owner:_ Ifiok Jr. · _Introduced in:_ [ef8521e](https://github.com/leptoskit/leptoskit/commit/ef8521ec8658b160a0a50609b4563d65589eabb5)

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

_Owner:_ Ifiok Jr. · _Introduced in:_ [c3d5204](https://github.com/leptoskit/leptoskit/commit/c3d52049e59ebdd825d7e56b865d0fa29597f785)

`leptoskit build --release --platform android`, `quest`, and `ios` now make what each store takes, signed with material read only from the environment, so a secret manager can hand it to one build: `monosecret run --profile release -- leptoskit build --release --platform android`.

- **Android and Meta Quest** sign the App Bundle and the APK with your upload key, from `LEPTOSKIT_ANDROID_KEYSTORE` (or the keystore itself, base64-encoded, in `LEPTOSKIT_ANDROID_KEYSTORE_BASE64`, which is written to a private temporary file and removed afterwards), `LEPTOSKIT_ANDROID_KEYSTORE_PASSWORD`, `LEPTOSKIT_ANDROID_KEY_ALIAS`, and `LEPTOSKIT_ANDROID_KEY_PASSWORD`. Gradle reads the passwords from its environment, never its command line. The build checks that both are signed and copies them to `.leptoskit/release/<target>/<name>-<version>.{aab,apk}`.
- **iOS** archives the app and exports `.leptoskit/release/ios/<name>-<version>.ipa` for the team in `LEPTOSKIT_IOS_TEAM_ID` (or `native.apple_team_id`). Automatic signing signs in with the App Store Connect key `leptoskit publish` uses, or the account in Xcode; manual signing names `LEPTOSKIT_IOS_CODE_SIGN_IDENTITY` and `LEPTOSKIT_IOS_PROVISIONING_PROFILE`. `[native.ios]` sets `signing-style` and `export-method` (`app-store-connect`, `ad-hoc`, or `development`), and `--export-method` overrides it for one build.
- `--build-number <n>` sets Android's `versionCode` and iOS's `CFBundleVersion`.

A release never falls back to an unsigned build or the debug key. Without its signing material it stops before compiling and names every missing variable; a keystore or key file inside the project's repository, Android's `debug.keystore`, and the `androiddebugkey` alias are refused. `--unsigned` builds an unsigned release on purpose, as earlier versions did, for measuring and testing. Debug builds and simulator builds are unchanged.

- **File routes under a sub-path.** `Router::from_browser` reads the page's `<base href>`. An app served under `/app/` declares `<base href="/app/" />`: its routes match the path after it, and every link and history entry the router writes starts with it. Without a `<base>`, the app is at the root of its site, as before. _Owner:_ Ifiok Jr. · _Introduced in:_ [dfd538f](https://github.com/leptoskit/leptoskit/commit/dfd538f7f7e87c872b5262b75f32935624d5d72e)

#### RPC clients report schema fidelity loss

_Owner:_ Ifiok Jr. · _Introduced in:_ [d3f3240](https://github.com/leptoskit/leptoskit/commit/d3f3240707f1167804bf9806a1cef162727639c8)

`leptoskit rpc generate` now warns, with the schema path and the target's generated fallback, wherever a client cannot carry a schema's shape: an arbitrary union or fixed tuple that collapses to `any`, an unresolved `$ref`, a multi-type or multi-member `allOf` schema, a non-string enum, nullable fields whose explicit null becomes absence, explicitly allowed extra object properties, and wide integers outside TypeScript or Dart runtime limits. Deliberately unconstrained JSON (`true`, `{}`, an untyped map or array) is reported as intentional, not lossy.

The default output is unchanged: every warning still generates the documented fallback. `leptoskit rpc generate --strict` fails before writing any file when any requested target loses structure, so a pipeline that needs exact types can refuse the fallback instead of discovering it in the generated code. `ApiSpec::diagnostics` exposes the same per-target report to callers of `leptoskit_rpc_codegen`. OpenAPI preserves schema nodes and does not run typed-client diagnostics. Strict mode detects known representation losses; it is not a general JSON Schema validator.

#### GPU 3D scenes

_Owner:_ Ifiok Jr. · _Introduced in:_ [66977bb](https://github.com/leptoskit/leptoskit/commit/66977bb3c65a17ef9a24faacf65d369e18dbe124)

The new `leptoskit_3d` crate, also `leptoskit`'s `3d` feature, draws 3D scenes on the GPU in a new `Scene3d` view: glTF models with skins, morph targets, and animation, lit by a directional light and a soft studio environment, each with a soft shadow beneath it. You build a scene with `scene3d::client::Scene` (load a model by path, spawn it, play or crossfade a clip, set morph weights and material overrides, show or hide named parts, attach one model to another's part, place the camera, and raycast), and the host animates it on the display's own clock. Your interface never re-renders to move a model, and a still scene draws nothing.

Scenes draw with WebGPU or WebGL2 in the browser, Metal in the generated iOS and macOS apps, and OpenGL ES in the generated Android apps, where `dev.leptoskit.scene3d.backend` manifest metadata chooses Vulkan instead. `leptoskit build` bundles the app's `assets/` folder into native apps that use 3D. Models may be compressed with meshopt and quantized, as `gltfpack` writes them. Windows and Linux leave an empty space for the view for now, the terminal draws a placeholder, HarmonyOS stops with an error, and the watch surfaces refuse it.

`leptoskit lint` knows `Scene3d` as a built-in component, so its `markup_style` rule holds a scene to the project's `[lint] markup` style like any other view.

The renderer also draws into any wgpu texture you hand it, from one view or several, over a camera image, and hidden behind real-world depth, for hosts that bring their own surfaces, such as augmented and virtual reality.

#### Schema packs declare their membership

_Owner:_ Ifiok Jr. · _Introduced in:_ [e25ffdd](https://github.com/leptoskit/leptoskit/commit/e25ffdd4dffaa4c0d3b982d080b605a13d3fed5f)

`leptoskit::db::packs!` now records the schema packs an app expects, and export records each pack's identity and the tables it owns in `schema.json` (format 2). A pack that is declared but exports no tables is refused before the first migration, and a pack the latest snapshot or a development push recorded can no longer disappear as an ordinary destructive migration: `db migration create`, `db push`, `db migration consolidate`, and the development watch require `--drop-pack <name>` for each pack whose removal is intended. `--allow-destructive` does not cover a pack removal, so an omitted dependency or feature cannot silently drop a library's tables. Changes inside a pack that is still present stay ordinary migrations.

Pack crates declare their identity and tables once with `leptoskit_db::schema_pack!`, which keeps the pack's tables in the export even when the app only names the pack in `packs!`. `leptoskit_auth`, `leptoskit_idp`, and `leptoskit_rpc` already do. Existing format 1 snapshots remain readable; they carry no membership evidence, so a pack that vanished before the snapshot was written cannot be detected from it, and the next migration writes format 2.

#### Stream motion sensors, and read them all from one plugin

_Owner:_ Ifiok Jr. · _Introduced in:_ [e5fa466](https://github.com/leptoskit/leptoskit/commit/e5fa466f19e1197159440023e85c555cd4d438b2)

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

_Owner:_ Ifiok Jr. · _Introduced in:_ [dd75f2a](https://github.com/leptoskit/leptoskit/commit/dd75f2af611c62fb50fc25edae55e590d5ea5e43)

Every `leptoskit db` command takes `--service <name>` to work on a service's own database instead of the app's: `leptoskit db migration create add_referrer --service analytics` reads the tables from the service's crate and writes the migration into its `migrations/` directory, or `migrations-postgres/` when the app is on Postgres, and `migrate`, `status`, `check`, `push`, `rollback`, `reset`, `studio`, and `release record` work the same way. Without `--url`, a command uses the database `leptoskit dev` gives the service, or `LEPTOSKIT_SERVICE_<NAME>_DATABASE_URL`; the app's `DATABASE_URL` never reaches a service command. A service installed from a registry or git ships its migrations, so the commands that write migrations refuse it. Unknown, remote, and database-less services fail with an error naming `[services.<name>]`.

On the development Postgres `leptoskit` runs, a service now logs in as a role of its own, `<app>_service_<name>`, which owns its database and can't connect to any of the app's databases; its password reaches Postgres only as a SCRAM verifier. When the app's `DATABASE_URL` names another server, and always under `leptoskit serve`, `leptoskit` creates and changes nothing there: the service's database comes from `LEPTOSKIT_SERVICE_<NAME>_DATABASE_URL`. On a git branch with its own app database, the service's database starts as a copy of the main branch's, and `leptoskit db branch list` and `prune` handle it with the app's.

`leptoskit infra` releases record each service database's migrations beside the app's. A release that changes a service's tables needs `--maintenance`, a service database that drifted from the history stops the release, and `lk deploy --rollback <tag> --rollback-schema --yes` checks every database first, then stops the services and takes each service's database back to the release's tables with its down migrations. Releases recorded before this hold only the app's tables, so rolling back to one leaves the services' databases alone. Images built by `leptoskit infra build` carry an `io.leptoskit.services.migrations` label when the app has database-backed services.

#### Operations that run in a service

_Owner:_ Ifiok Jr. · _Introduced in:_ [40300da](https://github.com/leptoskit/leptoskit/commit/40300dac1bd3e410610bea158bee23e8c31923d2)

A query, mutation, or action can belong to a service: a separate process the app runs beside its server. Declare it with `#[rpc::query(service = "analytics")]`, in a library both the service and the app depend on, and call it the way you call the app's own operations. From the browser, `rpc::client::call` and `use_live` send it to `/_leptoskit/services/analytics/` on the app's origin. From a native app, it goes to the same path on the configured origin. From the app's server while it renders, it goes over HTTP to `LEPTOSKIT_SERVICE_ANALYTICS_URL` as an anonymous caller, so a page renders only what the browser could fetch itself. `rpc::client::call_as_app`, on the server only, signs a call with `LEPTOSKIT_SERVICE_ANALYTICS_TOKEN` so the service treats it as the app. Live queries the service owns run on the service's own live connection, over its own database. The app's own credentials, such as a native app's access token, never go to a service unless `ClientConfig::service_credentials` names it, and a service's connection doesn't share the app's offline queue.

The body is compiled only where the declaring crate's `service` feature is on, so the app builds without the service's dependencies. `Server::new(db).service("analytics")` serves only that service's operations, and the app's server never serves them, even when a workspace-wide build links their bodies into it. An app operation and a service operation may share a name. `rpc::server::ServiceCallers` tells the app and its services apart by their tokens: in a service the app is the caller `"app"`, and in the app a service is `"service:<name>"`. The app's live connection refuses a service, whether it connects with the service's token or signs in with it later, because a service reads the app's data by calling its operations.

Jobs, cron schedules, streams, and channels can belong to a service too: `#[rpc::job(service = "analytics")]`, `#[rpc::stream(service = "analytics")]`, `#[rpc::channel(..., service = "analytics")]`. Only the service's workers run and schedule its jobs (`Jobs::service`, defaulting to the installed server's service), enqueueing one from another process is an error, and only the service's live socket serves its streams and channels. A service's operation can't take a toolkit `rpc.*` name, and an app operation named `rpc.…` is no longer served by every service as if it were built in.

`SpecOptions::service` (or `LEPTOSKIT_RPC_SPEC_SERVICE`) exports a service's spec, and the app's spec leaves service operations out. The facade's `rpc-services` feature lets a server that doesn't serve operations itself call a service.

#### A runtime for services that run beside the app's server

_Owner:_ Ifiok Jr. · _Introduced in:_ [40300da](https://github.com/leptoskit/leptoskit/commit/40300dac1bd3e410610bea158bee23e8c31923d2)

`leptoskit::service` is what a service, a separate process the app runs beside its server, needs at runtime. `ServiceEnv::from_env()` reads what `leptoskit dev`, `leptoskit serve`, and deploys start it with: its name, the address to bind, the token its trusted callers present, the app's private URL, its own database, and its volumes. `ServiceEnv::serve` serves its router with `/healthz` and `/readyz` and drains on `SIGTERM`, and `RequireToken` keeps a route to callers holding the token.

`leptoskit::service::main!(analytics_service, run)` writes the whole `main` of a service binary. It also links the library that declares the service's operations, without which the binary would answer every call with `404`. On the app's side, `service_url` and `service_token` read where a service is and how to call it.

#### Services beside the app server

_Owner:_ Ifiok Jr. · _Introduced in:_ [40300da](https://github.com/leptoskit/leptoskit/commit/40300dac1bd3e410610bea158bee23e8c31923d2)

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

_Owner:_ Ifiok Jr. · _Introduced in:_ [40300da](https://github.com/leptoskit/leptoskit/commit/40300dac1bd3e410610bea158bee23e8c31923d2)

`leptoskit generate deploy` and `lk infra` now run an app's `[services]` next to its server, from the same image and release.

- **Dockerfile:** the release build copies each service's binary to `/app/services/<name>`, and the health check asks whichever port the container serves.
- **Compose:** each service is its own entry with its token, database, and volumes. A Caddy edge on the app's port sends public service paths to their service, and strips cookies for isolated ones. On Postgres, each service gets a database and role of its own on the first start.
- **Kubernetes:** each service is a Deployment and a Service with a Secret of its own, and an Ingress routes the public paths, next to `--split-roles` too. The Fly and systemd targets refuse services they can't run.
- **`lk infra`:** each service is a Kamal role on the primary host, reached privately through a network alias. Its public paths go through kamal-proxy with the prefix kept. Its secrets reach only it, through Kamal's `NAME:SECRET` aliases, and the app's environment moves to the app's own role. Releases create each service's Postgres database and role, the backups cover every service's database, and restores bring them back with the app's.
- **Connection budget:** `[infra.postgres]` sets `max_connections` and the app's `app_pool`. `lk infra plan` and `lk doctor` fail when the app's and the services' pools, counted through a deploy, don't fit.

#### Services know who is signed in

_Owner:_ Ifiok Jr. · _Introduced in:_ [7e3fec8](https://github.com/leptoskit/leptoskit/commit/7e3fec81534c1bbc73d9a3142ec8f8c2180e7ece)

A service can now tell who is calling, so it can keep per-person data such as "my events" and per-person live queries. The app vouches for the person: its server mints a token that names them (their `Actor`, such as a `leptoskit_auth` account), lasts five minutes, and works only at the one service it's for. The key comes from the token `leptoskit` already gives the app and each service, through HKDF with its own label and the service's name, so nothing new has to be deployed.

- **Configuration.** `forward_identity` in a `[services.<name>]` table turns it on or off. It defaults to `true` for first-party services and `false` for isolated ones, and only `leptoskit.toml` can set it, never a service's manifest. `leptoskit dev`, `leptoskit serve`, Compose, Kubernetes, `leptoskit infra`, and the Docker steps give the app `LEPTOSKIT_SERVICE_<NAME>_IDENTITY=forward` for each such service, and `leptoskit services` lists it.
- **Browsers and native apps.** Before the first call to a service, the client asks the app for that service's token with the built-in action `rpc.service_token`, signed the way the app already is. It sends the token with that service's HTTP calls and on its live connection only, renews it before it runs out or when the service refuses it, and forgets it on `rpc::client::reauthenticate()`, which `leptoskit_auth`'s client calls on every sign-in and sign-out. With no token to give, calls go anonymously. A token can't be revoked once handed out: a service can keep seeing a person for up to about five and a half minutes after they sign out or their session is revoked.
- **Server rendering.** A page rendered for a signed-in person calls the service as that person, so the page holds what their browser would fetch. `call_as_app` is still the only way to call as the app.
- **In the service.** `ServiceCallers::for_service` accepts these tokens and identifies the person as the caller `person:<id>`, with their `Actor` and a `ForwardedIdentity` in the context. A token for another service, a changed one, or an expired one is refused with `401`. A live connection keeps the person until their token runs out and asks the client for a new one a minute before.

`rpc::server::service_identity_token` and `IdentityKey` mint and check tokens by hand. `examples/analytics` gains a live `mine` query of the signed-in person's own events, and records a signed-in person's events under them.

#### Flutter-style splash and icon assets

_Owner:_ Ifiok Jr. · _Introduced in:_ [30619f1](https://github.com/leptoskit/leptoskit/commit/30619f18231cf9c33e27471245800e2a5d67bd8b)

`[app.icons]` declares launcher icons the way Flutter's icons_launcher does: one default image, per-platform overrides, Android adaptive layers, a notification / status-bar icon, iOS dark and tinted appearances, and web favicon / maskable outputs. Windows gets an `.ico`, Linux gets hicolor PNGs, and free-form `[app.icons.extensions.<name>]` tables keep metadata extensible without opening unknown keys. `app.icon` still works as the simple path.

`[app.launch_screen]` (also `[app.splash]`) grows Flutter native-splash fields: branding, fullscreen, platform enable flags, an Android 12 subsection, web splash HTML/CSS (GIF-friendly), `[app.launch_screen.animated]` for Rive/GIF overlays, and `[app.launch_screen.extensions.<name>]`. `leptoskit::splash::{hold, dismiss}` keeps that overlay until the app is ready.

#### Stores for a state struct

_Owner:_ Ifiok Jr. · _Introduced in:_ [f980cf7](https://github.com/leptoskit/leptoskit/commit/f980cf794a20c534a9f6b0be401401640e06c815)

`leptoskit::store` re-exports Leptos's `reactive_stores`: derive `Store` on a state struct and each field becomes its own signal, so reading `store.title()` subscribes a screen to that field alone and writing it never notifies a sibling. The struct stays a plain Rust value, `ArcStore` holds it for as long as a clone lives (like `ArcRwSignal`), and `Store::new` follows the reactive owner instead (like `RwSignal`). Deriving `Patch` as well adds `.patch(next)`, which applies a whole new value — a reducer's output, or fresh data from the server — and notifies only the fields that changed. The derives need `reactive_stores` in the app's `Cargo.toml`; the types come re-exported, at the version leptoskit was built against. Fields are signals like any other, so the same code runs on web, iOS, Android, macOS, Windows, Linux, the terminal, and HarmonyOS (experimental).

`leptoskit::prelude` also gains the `GetUntracked`, `Update`, `With`, and `WithUntracked` signal traits, next to the `Get` and `Set` it already carried, for reading and changing store fields and signals alike.

The `opencode` example keeps its reducer and now applies it with `patch`, and `commonplace` writes one field at a time; both notify per field on every platform.

#### Web text polish and hover in Tailwind classes

_Owner:_ Ifiok Jr. · _Introduced in:_ [7eea31d](https://github.com/leptoskit/leptoskit/commit/7eea31d056ac25f0d7f68cffe590fd858b2adaa8)

Classes now accept `font-sans`, `font-serif`, `font-mono`, and `font-[var(--name)]` (a family from a theme variable), `tabular-nums` and `normal-nums`, `truncate`, `tracking-*`, and `cursor-*`, and a `hover:` variant that changes colors, opacity, or the cursor while a mouse rests over an element.

They are web polish. In the browser they do what Tailwind does, with `hover:` limited to pointers that can hover, so a tap on a phone never leaves an element stuck in its hover look. Native platforms accept the same class strings and ignore them: text keeps the system face, wrapping, and spacing, and nothing hovers. That is why `hover:` refuses layout utilities such as `hover:p-4`: a hover may change how something looks, never where it is.

- **Thermal state for apps.** `leptoskit_plugin_thermal` tells an app how hot the device is, so it can put heavy work off while the system is throttling. `guest::Thermal::snapshot` reads the state on Apple's four levels (nominal, fair, serious, critical, or unknown), and `wait_for_change` waits for the next different state and cancels when dropped. iOS and macOS read `ProcessInfo.thermalState` and its change notification; Android 10 and later read `PowerManager`'s thermal status and listener, folding its seven levels into the four and keeping the original in `Snapshot::android_status`. Older Android reports unknown, and desktop and browser hosts don't register the plugin. Battery charge, charging state and Low Power Mode stay in `leptoskit_plugin_battery`. _Owner:_ Ifiok Jr. · _Introduced in:_ [bcb276a](https://github.com/leptoskit/leptoskit/commit/bcb276a1a2672c779524375a3875fc0b39b9a36e)

#### New apps publish a typed RPC

_Owner:_ Ifiok Jr. · _Introduced in:_ [cb4f002](https://github.com/leptoskit/leptoskit/commit/cb4f002d6d39293ed2b504b5f3ecb5872d89eb8b)

A new app's API is now typed operations that every platform calls by name and every generated client describes. `leptoskit rpc spec` and `leptoskit rpc generate` work on a fresh scaffold: the server writes the published API and exits before it reads its configuration, opens the database, or loads sign-in's keys, so exporting needs no `DATABASE_URL` and no secrets, and creates no database file.

- Every new app publishes its backend's `health` check, still `GET /api/health`, as an `#[api]` server function, and `<name>_shared`'s `Health` derives `JsonSchema`.
- With `--db` or `--auth`, `<name>_shared` declares a `notes` query and an `add_note` mutation. The browser, the native apps, and the server call them by their argument structs, `Notes {}` and `AddNote { body }`; only the shared crate's `server` feature keeps their bodies, so no client links the database driver. The server serves them through `leptoskit::rpc::server::Server` and over the live socket at `/live`, with a `live` readiness check, and on `SIGTERM` it tells live sockets to reconnect before it drains. The `Note` table is `live`, and the db crate adds the `rpc` schema pack, whose receipts make a re-sent mutation run once.
- With `--auth`, operations run as whoever signed in: each note has an `owner_id`, and the table's policy shows and changes only the caller's own. The live socket follows `[auth] origin` once it's set. `/healthz` and `/readyz` sit outside the sign-in layer, `/readyz` waits for the migrations, and the server drains on shutdown like the others.

The scaffold's notes API changed shape, in new apps only: `GET /api/notes` is now `POST /api/notes` with `{}`, and `POST /api/notes` with `{"body": …}` is now `POST /api/add_note`. A failed call answers `Kind|message` text, and an empty note is `400` rather than `422`; `404`, `409`, and `500` keep their meaning, and with sign-in a signed-out call answers `401`. Apps created by an earlier leptoskit keep the handlers they have.

In a native app's guest, which has no timer, the live client no longer connects when it's configured: it waits until there's a call, a subscription, or a channel to serve, or calls in the offline queue. Before, an app that configured the client without going live, as sign-in's `configure` does once the app has operations, retried a socket it couldn't open without pause and never finished starting. `leptoskit_query::has_timer` says whether a host's `sleep` really waits.

#### Every font weight, and line height

_Owner:_ Ifiok Jr. · _Introduced in:_ [0e7d338](https://github.com/leptoskit/leptoskit/commit/0e7d338999ee2fe8e21ff267d506d2123200fd18)

Text takes all nine of Tailwind's weights: `font-thin`, `font-extralight`, `font-light`, `font-normal`, `font-medium`, `font-semibold`, `font-bold`, `font-extrabold`, and `font-black`. Before, anything outside `font-normal` to `font-bold` stopped the app as an unsupported utility. `FontWeight` gains the five new variants and `FontWeight::value`, the CSS number from 100 to 900. A platform whose system font lacks a weight draws the nearest one, as browsers do.

Line height is a text property now. `leading-none`, `leading-tight`, `leading-snug`, `leading-normal`, `leading-relaxed`, and `leading-loose` set it as a multiple of the font size, `leading-6` (any step on the spacing scale) as a fixed height, and `leading-[1.4]`, `leading-[150%]`, or `leading-[20px]` as you write it; `text-sm/6` sets a size and a line height together. Descendants inherit it, and a multiple follows each one's own font size. Every line takes exactly the line height with its glyphs centered, as in CSS, and layout reserves that height, on the web, macOS, iOS, Android, Linux, and HarmonyOS. Windows and the terminal keep the font's own line height, and native text fields keep their single-line height.

`NativeProps::line_height`, `LineHeight`, `Element::line_height`, and `TextStyle::line_height` carry it. A custom `TextMeasurer` whose platform cannot set line height can call `TextStyle::fit_line_height` to reserve what the renderer draws. `TextStyle` gains a field, so a struct literal needs `line_height: None` or `..TextStyle::new(…)`.

- **Reproduce website assets before verified packaging.** Add an explicit versioned website recipe that runs the production web pipeline offline in a digest-pinned builder, compares complete asset trees, and packages the checked backend and website bytes. Website descriptors bind paths, modes, hashes and build inputs to independent approvals and image-bound deployment policies; incomplete website evidence fails closed. _Owner:_ Ifiok Jr. · _Introduced in:_ [914f504](https://github.com/leptoskit/leptoskit/commit/914f504d4a33e2c8b1aa3607fd195b302c01f561)

#### Watch UI API for Wear OS and watchOS

_Owner:_ Ifiok Jr. · _Introduced in:_ [93cdb45](https://github.com/leptoskit/leptoskit/commit/93cdb45948642656163fb5ddd8d6123674289ad3)

Watch apps get their own surface vocabulary — `WatchApp`, `WatchPage`, `WatchTile`, `WatchGlance`, and `WatchComplication` — under `leptoskit::watch`, with hard layout budgets so a phone tree cannot be pasted onto a watch unnoticed. Content still uses leptoskit elements and Tailwind; the containers are watch-sized.

Android Wear OS (`wearos`) and Apple watchOS (`watchos`) are experimental platforms: config and CLI know them, `leptoskit new --platform wearos|watchos` scaffolds host stubs, and `leptoskit native prepare` writes them. Packaging and device install are not implemented yet. Enable with the `watch`, `wearos`, or `watchos` features.

- **Textured models packed by gltfpack.** Scenes read `KHR_texture_transform`, so a textured model compressed by `gltfpack` draws its textures where they belong. gltfpack quantizes texture coordinates and records the scale and offset that undo it as a texture transform; scenes used to ignore it, and every texture was sampled from its corner. The transform is applied to a primitive's texture coordinates as the model loads. A material's textures share one transform, the first found starting from the base color texture, which is what gltfpack writes. _Packages:_ 🟠 _leptoskit_3d_ _Owner:_ Ifiok Jr. · _Introduced in:_ [fa18362](https://github.com/leptoskit/leptoskit/commit/fa18362be9c37b7ccfb201dc4a1d103541c498ab)
- **Deliver primary-pointer drags, taps and cancellations from Android, iOS and macOS scene views through `Scene::next_pointer`.** Movement uses layout units and a bounded queue coalesces drags independently of model-loading events. _Packages:_ 🟠 _leptoskit_3d_, 🟢 _leptoskit_bridge_ffi_ _Owner:_ Ifiok Jr. · _Introduced in:_ [7237130](https://github.com/leptoskit/leptoskit/commit/723713098b125431f6e762cbf4b6f3ebe5506a23)
- **Additive light for glows, flashes, and sparks.** `Scene::set_blend(node, part, Blend::Additive)` (`Op::SetBlend`) makes a part of a model add its colour to what is drawn behind it, scaled by its opacity, instead of covering it. A muzzle flash, a spark, or a soft halo then reads as light over a camera picture, and black adds nothing, so a round glow needs no cut-out. `Blend::Normal` puts it back. Additive parts draw with the transparent parts. The op is new, so batches without it encode exactly as before. _Packages:_ 🟠 _leptoskit_3d_ _Owner:_ Ifiok Jr. · _Introduced in:_ [284fd88](https://github.com/leptoskit/leptoskit/commit/284fd8831fbc6ae5e085c4270152e74863c2965c)

#### Move a model's own parts from your app

_Packages:_ 🟠 _leptoskit_3d_

_Owner:_ Ifiok Jr. · _Introduced in:_ [107f77a](https://github.com/leptoskit/leptoskit/commit/107f77af379ab2edaa493f6f6de56d4180cc84c4)

`Scene::set_part_offset(node, part, offset, seconds)` (`Op::SetPartOffset`) moves a named part of a model instance, a bone or any node, from where its rest pose or the playing clip puts it. The offset turns the part about its own origin and moves it along its own axes, and everything under the part follows, skinned meshes included, so an app can turn a creature's head towards the camera while its walk clip plays, swing a rigid leg, or open a hinged lid by exactly as much as its own state says.

The host moves the part there over `seconds`, in a straight line from where it is now, on its own display clock. An app that works out a pose on each tick of its own clock and sends it with the tick's length is drawn smoothly between ticks. `Transform::IDENTITY` puts the part back. The op is new, so batches without it encode exactly as before.

#### 3D scenes over the live camera on iOS and Android

_Packages:_ 🟠 _leptoskit_3d_, 🟠 _leptoskit_xr_, 🟠 _leptoskit_bridge_ffi_, 🟠 _leptoskit_cli_

_Owner:_ Ifiok Jr. · _Introduced in:_ [c215d24](https://github.com/leptoskit/leptoskit/commit/c215d24d84051613fef7bf2171094a72208dabce)

A `Scene3d` whose backdrop is `Backdrop::Camera` now draws over the camera while an `xr` session runs on a phone. The generated iOS and Android apps hand the scene the session's newest frame: the camera picture behind the models, turned upright for the screen's rotation and cropped to fill the view, the session's own viewpoint in place of the scene's camera, and the room's measured depth hiding the parts of the scene behind real things. Start the session with `Features::CAMERA_IMAGE` (and `Features::DEPTH` for occlusion). Without a session the scene draws as before, transparent behind its models.

Android phones without ARCore, such as the Solana Seeker, now show the back camera in `Orientation` sessions too, through Camera2, with the rotation sensor turning the view and the camera's own field of view keeping the scene to the picture's scale. Such a session leaves the picture out, rather than failing, until the user allows the camera, unless it requires `CAMERA_IMAGE`.

On Android a `Scene3d` showing the camera moves below the window's own views, so the interface laid over it stays visible.

For hosts that draw scenes themselves, `leptoskit_3d::host::Hub::set_camera_feed` takes the picture a camera backdrop draws over, `View::set_rotation` the interface's rotation, and the bridge adds `lk_scene3d_view_set_rotation` and `lk_scene3d_view_shows_camera`. `leptoskit_xr`'s new `scene3d` feature makes the plugin's sessions that feed, and `leptoskit build` turns it on, with any other async plugin's `scene3d` feature, when the app shows 3D scenes. `leptoskit_xr::host::install_depth_estimator` (or `XrRuntime::set_depth_estimator`) takes a `DepthEstimator`, which turns small CPU copies of the camera picture (`CameraFrame`, with the camera's `Intrinsics`) into depth for phones that measure none.

#### Immersive sessions from Meta Quest panel apps

_Packages:_ 🟠 _leptoskit_3d_, 🟠 _leptoskit_xr_, 🟠 _leptoskit_bridge_ffi_, 🟠 _leptoskit_cli_, 🟢 _leptoskit_config_

_Owner:_ Ifiok Jr. · _Introduced in:_ [e897f31](https://github.com/leptoskit/leptoskit/commit/e897f31c27dbc9cb0c39378f56f705e9ef8e83f8)

A Quest app now draws its 3D scene into the headset. Start an `ImmersiveVr` or `ImmersiveAr` session and choose the scene with the new `Session::present(Some(scene_id))`: each eye sees the scene from the player's eye, over passthrough in `ImmersiveAr`. An app built with `[native.quest] immersive = false`, a 2D panel, opens an immersive activity over the panel for the session and comes back to the panel in Horizon Home when the session ends, while its code keeps running and receiving frames.

`leptoskit build --platform quest` declares that activity in every Quest app, marks a panel app's launcher with Horizon OS's 2D category and targets Horizon OS v69, and turns on `leptoskit_xr`'s new `quest` feature (or any async plugin's `quest` feature). For hosts that draw scenes themselves, `leptoskit_3d::host::Hub::draw_scene` draws a scene on another device, such as a headset's, and `Hub::forget_device` moves scenes off it again.

None of this has run on a headset yet.

#### Add Archetype ECS, scheduler, events, and signals

_Packages:_ 🟠 _leptoskit_game_

_Owner:_ Ifiok Jr. · _Introduced in:_ [0102748](https://github.com/leptoskit/leptoskit/commit/0102748f262439d41a5c7012381d41a64fb35566)

- Replace the linear entity list in `leptoskit_game` with a safe-Rust Archetype SoA ECS featuring generational `Entity` handles, cached O(1) archetype transition edges, inline 256-bit component signature masks, column-level change-detection skipping (`Changed<T>`, `Added<T>`, `Mut<T>`), bulk `spawn_batch`, SIMD-friendly chunk iteration, and native multi-core parallel chunk execution via `rayon`.
- Add `GameApp`, `GamePlugin`, `Schedule` with DAG ordering and conflict-free wave planning, fixed-timestep `Time` accumulation with render interpolation `alpha`, finite `State<S>` / `NextState<S>` transitions, deferred `Commands`, double-buffered `Events<E>`, and Godot-style unique names, groups, and synchronous signal observers while keeping full compatibility with `Game`, `Sim`, `World::cube`, and `leptoskit game run`.

#### Estimated depth for phones that measure none

_Packages:_ 🟠 _leptoskit_xr_, 🟠 _leptoskit_cli_

_Owner:_ Ifiok Jr. · _Introduced in:_ [b878753](https://github.com/leptoskit/leptoskit/commit/b878753721d8c81819b8b332cc0a7b9455097c86)

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

_Owner:_ Ifiok Jr. · _Introduced in:_ [7237130](https://github.com/leptoskit/leptoskit/commit/723713098b125431f6e762cbf4b6f3ebe5506a23)

`Frame::depth` now carries `Option<DepthSource>`: `None`, `Measured`, or `Estimated`. The host tags depth from the camera model separately from platform depth, including on phones with AR tracking and planes but no Depth API or LiDAR. Model estimates can have a different scale from the tracked floor: find each picture's own floor before scaling it to the measured floor's height.

Depth hit tests report `HitKind::EstimatedDepth` for model estimates and `HitKind::Depth` for measured depth, using the source of the picture the request actually hit. Recording and playback retain the source; existing version-one recordings still load, with depth treated conservatively as estimated because they did not record its source.

Apps that checked `frame.depth` as a boolean now use `frame.depth.is_some()` or match its source. Rebuild native hosts alongside their guests.

#### Augmented and virtual reality, and Meta Quest apps

_Packages:_ 🟠 _leptoskit_xr_, 🟠 _leptoskit_config_, 🟠 _leptoskit_cli_

_Owner:_ Ifiok Jr. · _Introduced in:_ [996f9b1](https://github.com/leptoskit/leptoskit/commit/996f9b1814996e5f6b3f565803d53266c3359e78)

The new `leptoskit_xr` crate and its `xr` plugin put an app in the real world. Start a session in the best mode the device has — `PhoneAr` through ARKit on iOS and ARCore on Android, `ImmersiveVr` and `ImmersiveAr` on headsets through OpenXR, WebXR in the browser, or `Orientation`, which turns the view with the phone on any device — and read a stream of frames: where the viewer is, each view's projection, the floors, tables, and walls found so far, pinned anchors, the room's light, and controllers and hands. Hit tests find where a tap or a controller's ray meets the real world, against planes and, on phones that measure it, the depth of the room. The camera and depth pictures stay in the native app for the renderer, and the frames your code receives are capped at `[native.plugin-options.xr] max-frame-rate`, always the newest. A recording replays a session exactly, for tests and simulators, with `replay = "…"`.

`leptoskit build --platform quest` builds the Android project as a Meta Quest app, with the manifest Horizon OS expects: OpenXR, head tracking, optional hand tracking and passthrough, the supported headsets, and a full-screen immersive launcher. `[native.quest]` configures it, and `leptoskit dev --platform quest` installs onto a connected headset.

With the `3d` feature, `host::scene3d::draw` draws a `leptoskit_3d` scene into a session: the camera picture behind it, real-world depth hiding what is behind real things, and one view per eye on a headset.

The backends build for their platforms but have not yet run on a device, so treat them as experimental.

#### Depth from the camera picture, for phones that measure none

_Packages:_ 🟠 _leptoskit_depth_

_Owner:_ Ifiok Jr. · _Introduced in:_ [1468b7b](https://github.com/leptoskit/leptoskit/commit/1468b7b0f64d5788e10effae252efd10badb4554)

The new `leptoskit_depth` crate reads metric depth from one camera picture on the device, for AR on phones whose platform has no depth: the Solana Seeker and other phones without ARCore, ARCore phones without the Depth API, and iPhones without LiDAR. It runs Depth Anything V2 Small, the authors' fine-tune for rooms, quantised to 27 MB, in pure Rust through tract: about a quarter of a second a picture on one core of an Apple M4, with no native library to ship.

`DepthModel::from_bytes` loads the model and `estimate` reads a `Picture` (RGB, RGBA, BGRA, or NV12 pixels, any quarter turn from upright, with the lens's intrinsics) into a `DepthMap` of metres along the camera's forward axis, laid over the picture as the camera read it, its scale corrected for the lens. `DepthWorker` runs it on its own thread about once a second for an AR runtime that hands over frames and must never wait. The model file is not in the crate: `MODEL` pins it by SHA-256 and says where to fetch it, `MODEL_LICENSE` is its Apache-2.0 licence, and an app ships both in its assets.

The `xr` plugin does not call it yet, and it has not run on a phone.

- **Dual-pane and tabletop layouts for foldables.** Fold posture now carries each hinge's `separating` flag (Android `FoldingFeature.isSeparating`), so a fully open continuous foldable stays one pane. `Posture::is_dual_pane`, `layout_mode`, and `use_posture` expose dual-pane, tabletop, book, and single layouts. `DevicePosture::watch` remains how you react to fold-driven configuration changes while the Android activity stays alive. The guide and Android platform page show concrete dual-pane and tabletop examples. _Packages:_ 🟠 _leptoskit_use_, 🟠 _leptoskit_plugin_posture_ _Owner:_ Ifiok Jr. · _Introduced in:_ [930b24e](https://github.com/leptoskit/leptoskit/commit/930b24e989f9e15a809201ea1baf21913c8cd596)

#### Add `use_clock`, a clock that advances by itself

_Packages:_ 🟠 _leptoskit_use_, 🟠 _leptoskit_

_Owner:_ Ifiok Jr. · _Introduced in:_ [95d9169](https://github.com/leptoskit/leptoskit/commit/95d9169b4306a60430916d2d39ab8d4f4afe6785)

`leptoskit_use::use_clock(interval)` returns a `Clock` signal holding the time since it was called, updated about once per `interval` on the platform's own timer: `setTimeout` in the browser, the timer plugin in a native app, and tokio or a timer thread elsewhere. Pass it to `use_timeout_fn`, `use_interval_fn`, `signal_debounced` and the other hooks that take a clock, or read it in a `Text` closure for a countdown. Until now a host had to advance a clock signal itself.

The time is measured from the wall clock and never goes back, so a tab that slept catches up in one step. The timer stops when the owner the clock was made under is cleaned up.

It is behind a new `clock` feature, because it is the one hook in `leptoskit_use` that needs a task executor and depends on `leptoskit_query`. In `leptoskit`, the `clock` feature turns on `use` and `query` too.

#### Keyboard shortcuts for the whole app on the web

_Packages:_ 🟠 _leptoskit_native_, 🟠 _leptoskit_

_Owner:_ Ifiok Jr. · _Introduced in:_ [8de3f4c](https://github.com/leptoskit/leptoskit/commit/8de3f4c497639561553ce1d2767baabbe41f2933)

`use_shortcut` runs a function when the person presses a key, wherever focus is, even when nothing has focus. It suits ⌘K to search or a single letter that opens a drawer, which `on_key` can't express because it only hears keys pressed inside its element.

```rust
let search = FocusRequest::new();
use_shortcut(Shortcut::primary("k"), move || search.request());
use_shortcut(Shortcut::key("w").unless_typing(), move || toggle_drawer());
```

`Shortcut::primary` holds ⌘ on Apple devices and Ctrl elsewhere. `.shift()` and `.alt()` add modifiers, and `.unless_typing()` leaves the key alone in a text field. A shortcut stops when its component goes, the newest of two matching shortcuts wins, and a key an element's `on_key` handled never reaches one.

Shortcuts work on the web today. Native hosts and the terminal don't deliver keys to them yet, so there a shortcut does nothing for now.

#### CanvistEditor: a place for a canvist text editor

_Packages:_ 🟠 _leptoskit_native_, 🟠 _leptoskit_tui_, 🟠 _leptoskit_appkit_, 🟠 _leptoskit_uikit_, 🟠 _leptoskit_gtk_, 🟠 _leptoskit_watch_, 🟠 _leptoskit_bridge_ffi_, 🟠 _leptoskit_docs_, 🟠 _leptoskit_lint_, 🟠 _leptoskit_

_Owner:_ Ifiok Jr. · _Introduced in:_ [5c71502](https://github.com/leptoskit/leptoskit/commit/5c71502b830239c1bcbf9301497142aa2003aa9a)

`CanvistEditor(id)` is a new element, with its own `NativeKind::CanvistEditor`, that marks where a [canvist](https://github.com/canvist/canvist) text editor shows. The app owns the editor through a state handle and gives the element the handle's id, so the document never travels through props. `placeholder`, `read_only`, and an accessible label work as on a `TextArea`, and the element must have a name like any text field.

This change adds only the place. On the web the element renders an empty `<div data-lk-canvist-editor="id">` that the editor's host looks for, and a server-rendered page names the browser's own editor on it when it hydrates; the host is the experimental, unpublished `leptoskit_canvist` crate, and the `leptoskit` component that joins the two comes next. macOS, iOS, Android, Linux, and Windows show an empty view, the terminal a dim block, and the watch surfaces refuse the element. The Swift and Kotlin renderers that generated apps use show an empty view too; the Swift ones used to stop on a kind they did not know.

The wire name is `canvist_editor`, appended last in the wire kind table, so existing codes keep their meaning.

#### Add ergonomic platform features and capability catalog

_Packages:_ 🟠 _leptoskit_native_, 🟠 _leptoskit_

_Owner:_ Ifiok Jr. · _Introduced in:_ [65c9b66](https://github.com/leptoskit/leptoskit/commit/65c9b66b6d97b26fece49f1003ceb73cb80ae59a)

Expose opt-in Cargo features on `leptoskit` (`live-activities`, `home-widgets`, `app-shortcuts`, `android-intent`, `menu`, `window-state`, `updater`, plus the `mobile-surfaces` and `desktop-extras` bundles) that re-export those plugins under `leptoskit::platform::*` and forward `leptoskit/guest`.

Expand `leptoskit_native::target::Capability` with exhaustive iOS and Android OS capabilities (including both wired built-in plugins and transparent entries for OS capabilities not yet wired as native plugins), along with `Capability::ALL`, `as_str()`, `plugin_crate()`, `runtime_plugin_name()`, `cargo_feature()`, `has_builtin_plugin()`, `summary()`, `TargetPlatform::supports()`, `PlatformCapabilities::supported_capabilities()`, and `PlatformCapabilities::available_capabilities()`.

#### Move keyboard focus once with `FocusRequest`

_Packages:_ 🟠 _leptoskit_native_, 🟠 _leptoskit_tui_, 🟠 _leptoskit_appkit_, 🟠 _leptoskit_uikit_, 🟠 _leptoskit_android_, 🟠 _leptoskit_gtk_, 🟠 _leptoskit_windows_, 🟠 _leptoskit_bridge_ffi_, 🟠 _leptoskit_

_Owner:_ Ifiok Jr. · _Introduced in:_ [f9752b8](https://github.com/leptoskit/leptoskit/commit/f9752b87d178230e24d89cb8df527e6f7e9518b6)

`FocusRequest::new()` makes a `Copy` handle. Give it to a widget with `focus_on(request)` and call `request()` from any handler: focus moves to that widget once, after the screen next updates, and later updates leave focus where the person has since put it. Use it to land in the code field after a code is sent, to return to the button that opened a menu, or to put a screen reader on an error summary. `focused(true)` is unchanged and still suits a widget that should always hold focus.

It works on every backend: `focus()` on the web (text and containers get `tabindex="-1"` so they can take it), the focus ring in the terminal, `grab_focus` on GTK, first responder on macOS and iOS, `SetFocus` on Windows and `requestFocus` on Android. Where a platform has no keyboard focus for a widget, iOS and Android move the VoiceOver or TalkBack cursor instead, and macOS and Windows leave focus alone. On macOS, Windows, iOS and Android the widget does not hear a `Focus` event for a requested move.

Renderer authors: `NativeProps` gains `focus_request`. `NativeSurface` sends it only with the commit that makes a new request, so a backend focuses `MutationList::focus_target()` after applying a batch and keeps no state.

- **Gestures on a view.** A view can follow a finger. `gesture(Gesture::tap())`, `pan`, `long_press`, `pinch`, `rotation`, and `fling` recognize the pointer in one arena shared by the Android widget backend and Linux. `simultaneous`, `exclusive`, and `race` compose recognizers the way a pinch and a rotation share one gesture, or a pan wins over a tap. A drag that wins claims the pointer so the control underneath does not also click. A tap does not, so a button still presses. iOS, macOS, Windows, the web, the terminal, and the generated Swift and Kotlin renderers still deliver presses and clicks only. _Packages:_ 🟠 _leptoskit_native_, 🟠 _leptoskit_android_, 🟠 _leptoskit_gtk_, 🟠 _leptoskit_ _Owner:_ Ifiok Jr. · _Introduced in:_ [955a1bf](https://github.com/leptoskit/leptoskit/commit/955a1bf7d00666992a8ccd52032131c7ae97cca1)

#### Hear the ⌘ key in key handlers on the web

_Packages:_ 🟠 _leptoskit_native_, 🟢 _leptoskit_tui_, 🟢 _leptoskit_aria_, 🟢 _leptoskit_wasm_

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

#### A field's label focuses it on the web

_Packages:_ 🟠 _leptoskit_native_, 🟠 _leptoskit_android_, 🟠 _leptoskit_

_Owner:_ Ifiok Jr. · _Introduced in:_ [0170294](https://github.com/leptoskit/leptoskit/commit/0170294ffef2da84f21c303fb7859fc8eec160e8)

A `Text` that names a field through `aria_labelled_by` now renders as `<label for="…">` on the web, so a click on the label focuses a `TextInput`, `PasswordInput`, `TextArea` or `Slider` and toggles a `Checkbox`, `Switch` or `Radio`, with no handler and no script. It applies when the field has an `id`, is named by exactly one plain `Text`, and no other field is named after the same text. `leptoskit_aria::Field` and `leptoskit::form` already build this shape. The markup for such a text changes from `<span>` to `<label>`; its styles and its `id` stay the same.

On Android, the label's view gets `labelFor`, so TalkBack knows which field it names. Other platforms are unchanged: clicking a label is not a convention there, and the field already carries the label's text as its name.

#### Open links in a new tab, save them, or open them natively

_Packages:_ 🟠 _leptoskit_native_, 🟠 _leptoskit_

_Owner:_ Ifiok Jr. · _Introduced in:_ [e5e6912](https://github.com/leptoskit/leptoskit/commit/e5e6912ebad6fcc21582314d0aa6355cca54c554)

A `Pressable` link can now say where it opens and whether it is a download.

```rust
Pressable("Open the admin")
	.href("https://admin.example.com/")
	.target(LinkTarget::NewTab)
	.child(Text("Open the admin ↗"));

Pressable("Export").href("/people/7/export").download("person-7.json");
```

On the web, `LinkTarget::NewTab` renders `target="_blank" rel="noopener noreferrer"` and tells screen reader users the link opens in a new tab. `download(name)` renders `<a href download>`, so the browser saves the file under that name. A name that is not one plain file name is ignored.

Native apps used to ignore `href`. With the new `links` feature, pressing a link that has no `on_press` handler opens an absolute HTTP(S), `mailto:`, `tel:` or `sms:` link outside the app, through `leptoskit_plugin_linking`: directly on macOS, Windows and Linux, and through the host's `linking` plugin in a Wasm guest on iOS and Android. A link with a handler, a relative link and a disabled link behave as before.

#### Modal and Popover show content above the screen

_Packages:_ 🟠 _leptoskit_native_, 🟠 _leptoskit_tui_, 🟠 _leptoskit_bridge_ffi_, 🟠 _leptoskit_wasm_, 🟠 _leptoskit_, 🟢 _leptoskit_watch_, 🟢 _leptoskit_docs_, 🟢 _leptoskit_lint_

_Owner:_ Ifiok Jr. · _Introduced in:_ [08a70cb](https://github.com/leptoskit/leptoskit/commit/08a70cb0dbd8a153b2b8eb285099920fb6d2252c)

`Modal(open)` blocks the screen until it is answered, and `Popover(anchor, open)` hangs from the element with that `id`. Both stay in the tree while closed. `.on_dismiss(...)` hears Escape and a press outside as a request to close, and the overlay closes when the next build says so. `.initial_focus(id)` names what takes focus on opening, and `.placement(Placement::BelowEnd)` picks a popover's side. In `view!` they are `<Modal open=…>` and `<Popover anchor=… open=…>`.

On the web a `Modal` is a `<dialog>` opened with `showModal()` and a `Popover` uses the `popover` attribute, so the browser supplies the top layer, the inert page, the focus trap and focus restore. The terminal draws overlays last, keeps Tab inside an open modal, and gives focus back on close.

macOS, iOS, Android, Linux and Windows draw an overlay as a layer in the right place for now. Nothing asks to dismiss it there, focus is not moved or kept inside, and a parent that clips or scrolls cuts it off.

For backend authors: `NativeKind` gains `Modal` and `Popover`, `NativeEvent` gains `Dismiss`, and `NativeProps` gains `overlay`. A backend that returns `true` from the new `NativeBackend::presents_overlays` is told the real kinds and given open overlays' frames in viewport coordinates. Every other backend is given a `View` with a frame relative to its parent. The guest event ABI gains kind 9 (dismiss), and the bridge event codes gain 10.

#### Add platform targeting and capability queries

_Packages:_ 🟠 _leptoskit_native_, 🟠 _leptoskit_build_, 🟠 _leptoskit_

_Owner:_ Ifiok Jr. · _Introduced in:_ [fee46c0](https://github.com/leptoskit/leptoskit/commit/fee46c0e80a311139a43d5b33ea3f559aae84df6)

`leptoskit::target` adds a shared runtime and compile-time platform API (`TargetPlatform`, `PlatformFamily`, `PlatformSwitch`, `PlatformCapabilities`, `Capability`, `CapabilitySupport`, `with_platform`, and `on_ios` / `on_android` / `on_mobile` / `on_macos` / `on_windows` / `on_linux` / `on_desktop` / `on_apple` / `on_web` / `on_extension` / `on_watch`) so universal apps can cleanly opt into platform-specific features and test every platform branch from any host.

`leptoskit_build::platforms()` now also recognizes `harmonyos`, `wearos`, `watchos`, and `quest` alongside the existing platform features.

#### Camera sessions and Camera surface

_Packages:_ 🟠 _leptoskit_native_, 🟠 _leptoskit_plugin_camera_

_Owner:_ Ifiok Jr. · _Introduced in:_ [f58eb08](https://github.com/leptoskit/leptoskit/commit/f58eb08f0bec47c9958a5b0bb2d5a3d9792cbf28)

`leptoskit_plugin_camera` is a new runtime plugin, `camera`, for host-owned camera sessions. Create or release a session for the back or front lens, set torch when the hardware supports it (otherwise `Unavailable`), take a bounded still into app cache (URI + width/height/mime/size matching `image_picker` Asset framing), and snapshot status. Show the same session with a new `Camera(session_id)` native widget kind that binds a host-owned id the way `Scene3d` and `Video` bind theirs (requires an accessible label or `aria_hidden`).

Ask for camera access through the `permissions` plugin; this plugin returns `PermissionDenied` when capture lacks a grant. iOS and macOS use `AVCaptureSession` / `AVCaptureVideoPreviewLayer`; Android uses CameraX / `PreviewView`; the browser uses `getUserMedia` into HTML `<video>`. Windows, Linux, and HarmonyOS return `Unavailable`. Video recording remains out of scope for this release.

- **Add `leptoskit_plugin_live_photo` with create/start/stop/status/release and a `LivePhoto` surface bound by session id (`PHLivePhotoView`).** Android and browser remain Unavailable; editing remains later scope. _Packages:_ 🟠 _leptoskit_native_, 🟠 _leptoskit_bridge_ffi_, 🟠 _leptoskit_plugin_live_photo_, 🟢 _leptoskit_ _Owner:_ Ifiok Jr. · _Introduced in:_ [13da9d6](https://github.com/leptoskit/leptoskit/commit/13da9d61f3252c056816f1da31516fd4e5040f9c)
- **Add `leptoskit_plugin_maps` with create/camera/markers/status/release and a `Map` surface bound by session id (MapKit / Google Maps).** User location and shapes remain later scope. _Packages:_ 🟠 _leptoskit_native_, 🟠 _leptoskit_bridge_ffi_, 🟠 _leptoskit_plugin_maps_, 🟢 _leptoskit_ _Owner:_ Ifiok Jr. · _Introduced in:_ [10ddb6f](https://github.com/leptoskit/leptoskit/commit/10ddb6fef86dc107d73054a9e2f2542f6e922da8)

#### Screen state and queries survive a rebuild

_Packages:_ 🟠 _leptoskit_native_, 🟠 _leptoskit_query_, 🟠 _leptoskit_

_Owner:_ Ifiok Jr. · _Introduced in:_ [c7a85ef](https://github.com/leptoskit/leptoskit/commit/c7a85ef05c4a1a510f6136b444ad4a0c2ef1af5e)

leptoskit builds a screen again whenever a signal it read changes. Two things broke under that: a `RwSignal::new(..)` inside a screen ran again with every rebuild, so a chosen filter or a typed field reset as soon as the screen redrew, and a `use_query` in a screen refetched in a loop, because the rebuild its data caused mounted the query again while the data already counted as stale.

- `leptoskit_native`: `remember(|| initial)` returns the same `RwSignal` on every build of the screen, told apart by its place in the source and by order. `remember_keyed(&key, || initial)` names the value by a key instead, for rows that move. A value is released once a whole build no longer reaches it, such as after navigating to another route. `guest::mount` keeps the values for native apps. In the browser, `web::screen(move || ..)` replaces `move || web::render(..)` and does the same. `ScreenState` is the store underneath, for hosts of their own. Both functions are in `leptoskit::prelude`, and the module is `leptoskit::remember`.
- `leptoskit_query`: a query mounted within two seconds of this client's last fetch of it takes that fetch's data or error instead of fetching again, like SWR's deduping interval. The window covers `RefetchOn::Always` and `retry_on_mount`, and not invalidation, `refetch()`, focus, reconnect, or intervals. Data from `set_query_data`, initial data, or hydration doesn't count as a fetch. Set it with `with_mount_dedupe` on `QueryDefaults`, `QueryOptions`, or `QueryObserverOptions`; `Duration::ZERO` gives TanStack's behaviour. `QueryOptions`, `QueryDefaults`, and `ResolvedQueryOptions` gain a `mount_dedupe` field, so code that builds them with struct literals needs it too.

An app with its own keyed store and a long stale time to work around this can switch to `remember` and drop both.

#### Scroll an element into view with `Reveal`

_Packages:_ 🟠 _leptoskit_native_, 🟠 _leptoskit_appkit_, 🟠 _leptoskit_uikit_, 🟠 _leptoskit_gtk_, 🟠 _leptoskit_windows_, 🟠 _leptoskit_

_Owner:_ Ifiok Jr. · _Introduced in:_ [6aef48a](https://github.com/leptoskit/leptoskit/commit/6aef48a580ac38f8dc06b281c49c9036faf04151)

A screen can now bring one element into view without knowing where layout put it. Create a `Reveal`, give it to the element with `reveal_on(&reveal, ScrollAlign::Nearest)`, and call `reveal.request()` from any handler. Each request scrolls once, after the next update, and the person scrolls freely afterwards. `ScrollAlign` is `Start`, `Center`, `End` or `Nearest`, and `Nearest` does nothing when the element is already in view.

On the web this is the browser's `scrollIntoView`, smooth unless the person asked for reduced motion. In native apps the surface works out the offset from layout and scrolls the nearest `ScrollView` around the element. Only that one view scrolls, and an element outside any `ScrollView` is left alone.

The GTK, Windows, AppKit and UIKit backend crates now apply `scroll_offset`, which they ignored before, so `Reveal` and `VirtualList`'s programmatic scrolling work in the generated Windows and Linux apps. GTK, Windows and AppKit jump to the offset; UIKit animates when asked. The terminal and HarmonyOS still don't scroll from code.

`NativeProps` gains a `reveal` field, which the surface answers itself and never sends to a backend.

#### TextArea: a field for several lines of text

_Packages:_ 🟠 _leptoskit_native_, 🟠 _leptoskit_testing_, 🟠 _leptoskit_tui_, 🟠 _leptoskit_appkit_, 🟠 _leptoskit_uikit_, 🟠 _leptoskit_android_, 🟠 _leptoskit_gtk_, 🟠 _leptoskit_windows_, 🟠 _leptoskit_watch_, 🟠 _leptoskit_bridge_ffi_, 🟠 _leptoskit_docs_, 🟠 _leptoskit_lint_, 🟠 _leptoskit_

_Owner:_ Ifiok Jr. · _Introduced in:_ [ce9d470](https://github.com/leptoskit/leptoskit/commit/ce9d470863cd327cdd87a36a12738c0fd356986c)

`TextArea()` is a new component, with its own `NativeKind::TextArea`, for notes, comments and messages. Enter adds a line instead of submitting, long lines wrap, and the field scrolls its own content. It is controlled like `TextInput`: `on_change` receives the proposed text with lines separated by `\n`, and `placeholder`, `read_only`, `max_length`, `capitalize`, `autocorrect` and `autocomplete` apply. It never raises `Submit`.

The web renders a `<textarea>`; macOS an `NSTextView` in an `NSScrollView`; iOS a `UITextView`; Android a multi-line `EditText`; Linux a `gtk::TextView` in a `gtk::ScrolledWindow`; Windows a multi-line `EDIT` control; the terminal a wrapping editor with line-wise arrow keys. The Swift and Kotlin renderers that generated apps use draw it too. GTK and Windows show no placeholder, and HarmonyOS and the watch surfaces do not draw the component.

`aria_multiline(true)` on a `TextInput` or `PasswordInput` now fails validation, because a single-line field that calls itself multi-line misreports itself to assistive technology. Use `TextArea` instead.

- **Typed text-input hints.** `TextInput` now carries the keyboard and text-editing hints that hand-written JavaScript used to provide, so every interface stays in leptoskit's own Wasm tree: `keyboard` (Text, Email, Number, Decimal, Phone, Url, Search), `read_only`, `max_length`, `capitalize` (None, Words, Sentences, Characters), `autocorrect`, and `submit_label` (Done, Go, Next, Search, Send). Each platform maps them to its own controls: the web to `type`, `inputmode`, `readonly`, `maxlength`, `autocapitalize`, `autocorrect`, `spellcheck`, and `enterkeyhint`; iOS to `keyboardType`, `autocapitalizationType`, `autocorrectionType`, `spellCheckingType`, and `returnKeyType`; Android to `inputType`, `imeOptions`, `InputFilter.LengthFilter`, and a cleared key listener for read-only fields; GTK to `set_input_purpose`, `set_editable`, `set_max_length`, and `InputHints`; macOS to `isEditable` and delegate truncation plus the field editor's correction and spell checking; Windows to `EM_SETREADONLY` and `EM_LIMITTEXT`. `read_only` implies `aria-readonly`. The Swift and Kotlin bridge renderers and the terminal backend honour the same props. _Packages:_ 🟠 _leptoskit_native_, 🟠 _leptoskit_tui_, 🟠 _leptoskit_appkit_, 🟠 _leptoskit_uikit_, 🟠 _leptoskit_android_, 🟠 _leptoskit_gtk_, 🟠 _leptoskit_windows_, 🟠 _leptoskit_bridge_ffi_, 🟠 _leptoskit_ _Owner:_ Ifiok Jr. · _Introduced in:_ [6d42b06](https://github.com/leptoskit/leptoskit/commit/6d42b068273560b572fa4c7f5f4475b33a03563b)
- **Ding-inspired toast pills.** Status toasts arrive as a 44px shape, spring open into a pill, morph in place when a newer message arrives, and shrink before they leave — the same phase model and spring tunings as [Ding](https://github.com/YusufCeng1z/ding). Hold a `Toaster`, call `success` / `error` / `warning` / `info` / `loading`, rebuild from `toaster.state()`, and advance phases with `poll`. `ToastViewport` draws the stack from shared builders, so every backend that already draws `View`, `Text`, `Button`, and `Vector` can show a toast. _Packages:_ 🟠 _leptoskit_native_, 🟠 _leptoskit_ _Owner:_ Ifiok Jr. · _Introduced in:_ [ab77e98](https://github.com/leptoskit/leptoskit/commit/ab77e9807ae2c781c3d7a80ff5e00c270cfac4b6)

#### Video players and Video surface

_Packages:_ 🟠 _leptoskit_native_, 🟠 _leptoskit_plugin_video_

_Owner:_ Ifiok Jr. · _Introduced in:_ [130c61f](https://github.com/leptoskit/leptoskit/commit/130c61f9f2fa514b14122055e7c1021735ce2a3b)

`leptoskit_plugin_video` is a new runtime plugin, `video`, for host-owned URL video players. Create or replace a source, play, pause, seek, set muted/loop/rate, snapshot status, and release the player. Show the same player with a new `Video(player_id)` native widget kind that binds a host-owned id the way `Scene3d` binds a scene (requires an accessible label or `aria_hidden`).

iOS and macOS use `AVPlayer` with an `AVPlayerLayer` surface; Android uses Media3 ExoPlayer with a `PlayerView`; the browser uses an HTML `<video>` element. Windows, Linux, and HarmonyOS return `Unavailable`. Tracks, caching, AirPlay, fullscreen controls, and iOS picture-in-picture enter against an attached player remain out of scope for this release.

- **`leptoskit debug` prints a running app's element tree, runtime errors, and render commits from the devtools hub, and `leptoskit debug open` prints the hub address for the inspector.** `leptoskit mcp` adds `runtime_errors`, `performance`, and `read_logs`, so an agent can see the tree, the errors, the logs, and the commits of an app that `leptoskit dev` is running, and can still capture a screenshot where the platform allows it. `leptoskit device install` installs an Android package, an iOS app, or checks a desktop path. `leptoskit device open` launches it. `leptoskit device sanity` does both and then injects tap, long press, pan, and pinch on the targets that can produce them: every gesture on Android and on an iOS simulator or device, tap and long press and pan on macOS, Windows, and Linux. Linux pinches through `--driver`. A mouse session cannot pinch, and the reply says so. _Packages:_ 🟠 _leptoskit_testing_, 🟠 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [d161fdf](https://github.com/leptoskit/leptoskit/commit/d161fdf435c8f5e64cfb09f2ff53feec5619c921)

#### Arrow keys move through tabs and radio cards

_Packages:_ 🟠 _leptoskit_testing_, 🟠 _leptoskit_aria_, 🟠 _leptoskit_

_Owner:_ Ifiok Jr. · _Introduced in:_ [bc7c851](https://github.com/leptoskit/leptoskit/commit/bc7c8510366147ca4e256b1a1f3896717f4c50d4)

`leptoskit_aria::Tabs` is now one stop for the Tab key. Only the selected tab is in the Tab order; Left and Right (Up and Down after `.orientation(Orientation::Vertical)`), Home and End move to another tab, select it, and take focus with it.

**Breaking:** the selection callback moved from each tab to the list. Write `tabs.list(move |index| …)` and `tabs.tab(index, label)` where you wrote `tabs.list()` and `tabs.tab(index, label, move |index| …)`. `Tabs` no longer implements `PartialEq`.

`leptoskit_aria::RadioGroup` is new: a choice of one option among several where each option shows any content, such as a card. `RadioGroup::new(id, labels, selected).option(index, content).on_change(…)` becomes an element with `Element::from`. Options carry the `radio` role, their checked state and their position; the group is one Tab stop, and the arrow keys, Home and End move, choose and take focus along.

The arrow keys work on the web and in the terminal. macOS, iOS, Android, Windows and GTK do not deliver key presses to the app yet, so every tab and option stays reachable with Tab there, as before.

`leptoskit_testing::WidgetTester::press_key(finder, key)` presses a key on a widget and bubbles it to the nearest `on_key` handler.

#### Menus and modal dialogs in leptoskit_aria

_Packages:_ 🟢 _leptoskit_tui_, 🟠 _leptoskit_aria_

_Owner:_ Ifiok Jr. · _Introduced in:_ [14db9ea](https://github.com/leptoskit/leptoskit/commit/14db9ea30962657d4f38f0d4629fdfa5d6d6df87)

`Menu::new("account", open).button(avatar).item("Sign out", sign_out).on_toggle(set_open)` is a button that opens a list of actions. Opening moves focus to the first action, Up and Down move through them and wrap, Home and End go to the ends, and Enter or Space runs one. Choosing an action, Escape, Tab, or a press outside closes the menu and gives focus back to the button. `.content(...)` gives an action richer content than its label, and `.placement(...)` picks the side it opens on.

`Dialog::modal(open)` is the dialog as a `Modal`: above the screen, named by its title, described by its description, with focus kept inside on the web and in the terminal. `Dialog::container(open)` stays for a dialog laid out in the page.

In the terminal, Tab that would carry focus out of an open popover asks the popover to close, focus requests and `initial_focus` reach controls with a negative `tab_index`, and only the focused `Pressable` is underlined (every one was).

macOS, iOS, Android, Linux and Windows show a menu and run its actions, but the arrow keys, Escape and a press outside do nothing there yet.

#### macOS shows Modal as a sheet and Popover as NSPopover

_Packages:_ 🟠 _leptoskit_appkit_, 🟠 _leptoskit_bridge_ffi_, 🟢 _leptoskit_cli_

_Owner:_ Ifiok Jr. · _Introduced in:_ [4cba9d4](https://github.com/leptoskit/leptoskit/commit/4cba9d4cc9bafbfdd824dfca3ce310b3ad3cda0d)

On macOS a `Modal` is now a sheet on the app's window: it dims the window, keeps clicks and the keyboard to itself, and gives focus back when it closes. A `Popover` is an `NSPopover` pointing at its anchor. Escape asks to dismiss either one, and a press outside asks to dismiss a popover. `initial_focus` moves focus into the overlay when it opens. This holds for generated macOS apps (the Swift renderer) and for Rust hosts on `AppKitBackend`.

What differs from the web: a sheet is as large as what the modal holds and the modal's own background is not drawn; a sheet has no outside to press; and a popover is centred on its anchor, so `placement` picks above or below only.

`AppKitBackend` gains `overlay_shown`, `overlay_window` and `perform_dismiss`, and `open_window` shows overlays that were already open. Generated macOS projects ship one more Swift file, `MacOverlayView.swift`.

#### iOS presents Modal as a sheet and Popover as a popover

_Packages:_ 🟠 _leptoskit_uikit_, 🟠 _leptoskit_bridge_ffi_, 🟢 _leptoskit_cli_

_Owner:_ Ifiok Jr. · _Introduced in:_ [9055533](https://github.com/leptoskit/leptoskit/commit/905553347de313983dc7fadd12b419d286dabe38)

On iOS a `Modal` is now a form sheet: it dims the screen, keeps touches and VoiceOver to itself, and gives focus back when it closes. A `Popover` is a system popover pointing at its anchor, on an iPhone as well as an iPad. A swipe down on the sheet, a press outside the popover, Escape on a hardware keyboard and the VoiceOver escape gesture all ask to dismiss, and the overlay stays until your `open` signal closes it. `initial_focus` raises the keyboard for a field and moves the VoiceOver cursor for anything else. This holds for generated iOS apps (the Swift renderer) and for Rust hosts on `UIKitBackend`.

What differs from the web: a sheet is as large as what the modal holds on an iPad and as wide as the screen on an iPhone, and the modal's own background is not drawn; and a popover is centred on its anchor, so `placement` picks above or below only.

`UIKitBackend` gains `overlay_shown`, `overlay_controller`, `perform_dismiss` and `present_overlays`. A controller that mounts the tree itself calls `present_overlays` from `viewDidAppear:`; `UIKitHost` does so already. Generated iOS projects ship one more Swift file, `MobileOverlayView.swift`.

#### Android presents Modal as a dialog and Popover as a popup

_Packages:_ 🟠 _leptoskit_android_, 🟠 _leptoskit_bridge_ffi_, 🟢 _leptoskit_cli_

_Owner:_ Ifiok Jr. · _Introduced in:_ [f740aa1](https://github.com/leptoskit/leptoskit/commit/f740aa1e5c264305477d17f75c66296993833da2)

On Android a `Modal` is now a dialog in a window of its own: it dims the screen, keeps touches, the keyboard and TalkBack to itself, and is as large as what the modal holds. A `Popover` is a popup window placed beside its anchor where `placement` says. The Back gesture, Escape and a touch outside ask to dismiss. A dialog stays until your `open` signal closes it. The Back gesture closes a popup window before your app hears of it, so a popover your app keeps open is shown again. `initial_focus` moves focus to the named element when the overlay opens. This holds for generated Android apps (the Kotlin renderer) and for Rust hosts on `AndroidBackend`.

What differs from the web: the modal's own background is not drawn, because the dialog dims the screen, and a dialog needs an Activity behind the view the tree is mounted in.

`WidgetHost` gains `overlayShown`, `overlayDialog` and `overlayPopup`. Generated Android projects ship one more Kotlin file, `NativeOverlayView.kt`.

This also restores two things the Android host needed to compile: an import in `WidgetHost.kt` and the `Dismiss` case in the probe.

#### Linux presents Modal as a window and Popover as a popover

_Packages:_ 🟠 _leptoskit_gtk_

_Owner:_ Ifiok Jr. · _Introduced in:_ [313f08c](https://github.com/leptoskit/leptoskit/commit/313f08c84b08a32ffba7de566395f256e1a56622)

On Linux a `Modal` is now a modal window above your app's window: it has no title bar, is as large as what the modal holds, and keeps presses, keys and the screen reader to itself. A `Popover` is a GTK popover placed beside its anchor where `placement` says. Escape asks either one to dismiss, as do the window manager's close for a modal and a press outside a popover. Neither closes until your `open` signal says so. Focus moves to `initial_focus`, or else to the first control inside, and returns to where it was when a popover closes.

What differs from the web: the modal's own background is not drawn and GTK does not dim the window behind, the window manager chooses where the modal goes, and a press on a popover's anchor does not ask to dismiss, so the control that opened it decides.

`GtkBackend` gains `overlay_shown`, `overlay_window` and `overlay_popover`.

#### Windows presents Modal and Popover in popup windows

_Packages:_ 🟠 _leptoskit_windows_

_Owner:_ Ifiok Jr. · _Introduced in:_ [4296978](https://github.com/leptoskit/leptoskit/commit/4296978771add99ec48b27131f629d9ceb7fa214)

On Windows a `Modal` is now a popup window owned by your app's window: it is as large as what the modal holds, the window behind takes no input until it closes, and Tab stays inside. A `Popover` is a popup window placed beside its anchor where `placement` says. Escape asks either one to dismiss, as do Alt+F4 for a modal and losing the keyboard for a popover, such as a press elsewhere in your window. Neither closes until your `open` signal says so. Focus moves to `initial_focus`, or else to the first control inside, and returns to where it was on close.

What differs from the web: Windows does not dim the window behind a modal, the popup is a plain rectangle, and a press on a popover's anchor does not ask to dismiss, so the control that opened it decides.

`WindowsBackend` gains `overlay_shown` and `overlay_window`. `WindowsWindow`'s message loops now route Tab and Escape to the popup a message belongs to. An app with its own message loop must do the same with `IsDialogMessageW`.

#### Add watch hardware, workout, and companion sync APIs

_Packages:_ 🟠 _leptoskit_watch_

_Owner:_ Ifiok Jr. · _Introduced in:_ [d2ebdc4](https://github.com/leptoskit/leptoskit/commit/d2ebdc49c33eacaaba85fd000dc405f8eb65255a)

Adds portable, `#![forbid(unsafe_code)]` watch OS and hardware APIs to `leptoskit_watch`:

- `crown`: Digital Crown / Wear OS rotary bezel `CrownConfig`, `CrownEvent`, `CrownSensitivity`, and `RotaryState` with detent tick counting.
- `aod`: Always-On Display / ambient lifecycle (`AmbientModeState`, `AmbientPolicy`) and Smart Stack / Tile `RelevanceHint`.
- `workout`: `WorkoutSession` state machine (`Prepared -> Running <-> Paused -> Ended`), `WorkoutActivity`, `WorkoutLocation`, and `WorkoutMetrics`.
- `companion`: Phone-to-watch sync (`CompanionState`, `DeliveryMode::{Immediate, ApplicationContext, BackgroundTransfer}`, `CompanionMessage`, `CompanionLink`, and `MemoryCompanionLink`).
- `interaction`: `WatchHaptic`, `WatchGesture` (`DoubleTap`, `WristFlick`, `CrownPress`, `SideButtonPress`, `ActionButtonPress`), and `WatchCapabilities`.

#### Wearable and smartwatch support across watch OSes and BLE

_Packages:_ 🟠 _leptoskit_watch_, 🟠 _leptoskit_watchos_, 🟠 _leptoskit_wearos_, 🟠 _leptoskit_config_, 🟠 _leptoskit_plugin_battery_, 🟠 _leptoskit_plugin_thermal_, 🟠 _leptoskit_plugin_pedometer_, 🟠 _leptoskit_plugin_health_, 🟠 _leptoskit_plugin_wearable_, 🟠 _leptoskit_plugin_watch_sync_, 🟠 _leptoskit_plugin_ble_, 🟠 _leptoskit_cli_

_Owner:_ Ifiok Jr. · _Introduced in:_ [5b1f3ba](https://github.com/leptoskit/leptoskit/commit/5b1f3ba88e456a3bd5b427603bda36022781444d)

Adds sensor-first wearable application APIs, watch host packaging, and wearable biometric/companion/BLE plugins:

- **`leptoskit_watch`**: adds `WatchKind::HarmonyOs`, Samsung Galaxy Watch and Huawei Watch viewport presets, and the sensor-first `WearableApp` API (`WearableStream`, `SamplingPolicy`, `DeliveryMode`, `WearableStatus`, `WearableSurfaceMode::AutoStatus` / `Custom`, and per-OS `required_permissions()`).
- **`leptoskit_plugin_health`**: adds `SkinTemperature`, `ActiveCalories`, `Distance`, and `FloorsClimbed`; enables `HealthKit` on watchOS alongside iOS; adds HarmonyOS `HealthPlugin.ts` (`@kit.HealthServiceKit`).
- **`leptoskit_plugin_wearable`**: new direct on-watch live & passive biometric sensor plugin covering Wear OS Health Services (`PassiveMonitoringClient`, `MeasureClient`, `SensorManager`, plus optional Samsung Health Sensor SDK runtime hook), watchOS (`HKWorkoutSession`, `HKLiveWorkoutBuilder`, `HKObserverQuery`, `CMPedometer`), and HarmonyOS Wearable (`@kit.SensorServiceKit`).
- **`leptoskit_plugin_watch_sync`**: new bidirectional phone ↔ watch companion sync plugin over Apple `WatchConnectivity`, Android/Wear OS Wearable Data & Message Layer, and Huawei Wear Engine.
- **`leptoskit_plugin_ble`**: new BLE GATT central plugin (`CoreBluetooth`, Android/Wear OS `BluetoothGatt`, HarmonyOS `@kit.ConnectivityKit`) with pure-Rust Bluetooth SIG decoders for Heart Rate + RR-intervals/RMSSD, Pulse Oximeter (`SpO2`), Health Thermometer, Battery, Running Speed & Cadence, and Cycling Power.
- **HarmonyOS plugins**: adds ArkTS implementations for `battery`, `thermal`, and `pedometer`.
- **`leptoskit_config` & `leptoskit_cli`**: adds `[native.wearos]`, `[native.watchos]`, and `[native.harmonyos]` configuration tables; upgrades `wearos` to build Android APKs with Wear OS manifest transformation (watch feature, standalone metadata, tiles/complications); upgrades `watchos` to generate watchOS Xcode build files and invoke `xcodebuild`; updates HarmonyOS host generation to emit configured `deviceTypes` (including `"wearable"`) and `requestPermissions`.

- **The insights engine (§10 of the analytics service design), in its first form.** A new `insights` feature holds the query types — trends, funnels, retention, the web overview, the events list, and the people behind any of their points — as plain serde values an app or an admin compiles without the service. With the `service` feature, the `run` operation plans each query into SQL for SQLite or Postgres and answers with the series, steps, grid, or page beside the SQL it ran, its bound parameters, and its timing (the Workings drawer's data). Property names travel as bound parameters and session columns come from an allow-list, so a name carrying SQL reads a property of that name instead of running it; every query resolves persons through `person_overrides`, so a merge applies to past events at once. Saved insights and dashboards are app-only CRUD over new `insights` and `dashboards` tables, with the default "Web overview" available as data. Live refresh: the batch writer bumps one live `ingest_ticks` row at most once per `refresh_seconds` (default 10, configurable), so watched insights re-run as events arrive without the events table being live. A minimal `live_events` stream tails the writer's last 10,000 events, filtered server-side and resumable, each item with the matched rate. The service's `routes` now also returns its live socket, which the binary starts. All query operations answer to the app's caller, like the person operations before them. _Packages:_ 🟠 _leptoskit_analytics_ _Owner:_ Ifiok Jr. · _Introduced in:_ [c7f6ab9](https://github.com/leptoskit/leptoskit/commit/c7f6ab90ecc33b4f84b76cd79b40e8e322cd3071)

#### Persons, groups, and sessions at the analytics service, and views the browser records on its own.

_Packages:_ 🟠 _leptoskit_analytics_

_Owner:_ Ifiok Jr. · _Introduced in:_ [93b1c47](https://github.com/leptoskit/leptoskit/commit/93b1c4715d67ee50c7cf1da9d84baad94f6b2561)

The service now builds a person for every identified id and joins a visitor's made-up id to their real one when the app identifies them, so what someone did before and after signing in is one story. Person properties keep the value of the most recent event per property, whatever order events arrived in — an offline phone's late batch no longer overwrites what you learned later. Merging two already-identified persons is refused and counted, never guessed. Each visit becomes one session row with its entry and exit pages, duration, bounce, and how it arrived: the channel from the referrer and UTM parameters, and the device, browser, and operating system parsed from the user agent at the service, so clients stay small. The browser's client now records a `$pageview` on load and on history navigation and a `$pageleave` when the page goes, with the session's first view carrying its UTM parameters and referrer; `Options::pageviews` turns that off.

Your app's server can read and manage persons — list them, read one with their events and sessions, delete one and everything stored about them, or export one as JSON — and the service's `person_profiles` setting decides who gets a person at all.

The core browser client grows from 11,985 to 13,584 bytes for the automatic views, so its size budget changes from 12 KiB to 14 KiB, decided in the open as the analytics design's size rule requires.

#### Retention cohorts every period; event definitions

_Packages:_ 🟠 _leptoskit_analytics_

_Owner:_ Ifiok Jr. · _Introduced in:_ [60e11ae](https://github.com/leptoskit/leptoskit/commit/60e11ae75fdffff7d6da8bc30d4369aced2e9d41)

- **Retention cohorts.** A retention query now answers one cohort for every period in its range, oldest first, even when nobody started in a period (its size is 0). `ActorPoint::RetentionCell { start_period, .. }` therefore indexes `cohorts` directly. Each cohort is followed for `periods` periods or until the range ends, so later cohorts have shorter `returns`. First-time cohorts look at the start event's whole history, so someone who first did it before the range doesn't count as new inside it.
- **Wire rename.** A retention query's first-time or recurring choice now travels as `retention_kind`. Before, it travelled as `kind`, which collided with `Query`'s own tag: saving a retention insight failed.
- **`event_definitions`.** This app-only operation lists every event name the service has seen, with:
  - its volume
  - when it was first and last seen
  - the fields of its registered Rust type, if it has one
  - the properties its recent events carried

  The admin's builders read it.

#### `Store::schema_fingerprint()` and generation integrity

_Packages:_ 🟠 _leptoskit_deploy_console_

_Owner:_ Ifiok Jr. · _Introduced in:_ [19fc493](https://github.com/leptoskit/leptoskit/commit/19fc493af2a2b28674f7c64e676c96f7080601fe)

`leptoskit_deploy_console::control::Store` now exposes `Store::schema_fingerprint()`: a pure, non-I/O digest of the management migrations this build applies (version and SQL, length-framed), computed from the migration list itself so there is no second list to keep in step. Anyone holding a published state contract can now refuse schema drift _before_ `Store::open` runs — `Store::open` still verifies the journal and auto-applies new migrations, so before this API a caller had no way to detect a new migration ahead of the open that would apply it.

The public demo runtime (an unreleased example) uses it for exactly that: its generation fingerprint now covers the deploy console's migrations alongside the analytics migrations and the linked SQLite schema, a published marker's seed count is treated as a historical receipt rather than a live invariant, corrupt markers fail closed instead of triggering a rebuild, retention keeps recent published generations across formats as rollback state, and publication is preceded by an explicit fsync barrier over the databases, their WALs, and the directory entries up to the volume root.

- **The analytics charts and builders move to the pages crate.** `leptoskit_admin_pages::analytics` now holds the analytics area's drawing: number and date formatting, a query as an address (`params`), the trend, funnel and retention charts, and each builder's controls. The admin's server still runs the queries and draws these pages itself for now; the move lets the next change hydrate them. Nothing on screen changes. _Packages:_ 🟢 _leptoskit_admin_, 🟠 _leptoskit_admin_pages_ _Owner:_ Ifiok Jr. · _Introduced in:_ [5015d6e](https://github.com/leptoskit/leptoskit/commit/5015d6e05a9adb1f67bb78dc7234ed0972b528a2)

#### Analytics in the admin

_Packages:_ 🟠 _leptoskit_admin_

_Owner:_ Ifiok Jr. · _Introduced in:_ [60e11ae](https://github.com/leptoskit/leptoskit/commit/60e11ae75fdffff7d6da8bc30d4369aced2e9d41)

The admin's Analytics area now reads your site's analytics. Connect it with `.analytics(AnalyticsService::new(url, token))` on the builder (or `AnalyticsService::from_env()`, which reads `LEPTOSKIT_SERVICE_ANALYTICS_URL` and `LEPTOSKIT_SERVICE_ANALYTICS_TOKEN`). The admin calls the analytics service as the app, signed with the service's token, which stays on the admin's server. Without a service, every Analytics page shows how to connect one.

The pages:

- **Overview**: visitors, pageviews, sessions, and bounce rate against the previous period, plus top and entry pages, referrers, channels, UTM, devices, browsers, and operating systems.
- **Trends**: a builder over the events your app has sent, with a `struct` badge on typed ones, and controls for math, breakdown, interval, compare, and range.
- **Funnels**: conversion and the median time to convert.
- **Retention**: a heat grid of cohorts.
- **People**: search and a property filter. Each person's page shows their timeline, ids, and sessions, with export and delete behind an in-page confirmation.
- **Live**: an event tail with a filter and pause. The top bar's pulse follows it.
- **Library**: saved insights and dashboards that update as events arrive.

Every point, step, drop-off, and retention cell opens onto the people behind it. Every insight's **Workings** drawer follows its number through five stations:

- **Emit**: the Rust type.
- **Send**: the bytes it sends.
- **Store**: the tables it lands in.
- **Ask**: the query as Rust, SQL, and HTTP, updating as you change the builder.
- **Draw**: the rows the chart was drawn from.

Learn's analytics lessons now tick from your real data.

#### An admin for your site

_Packages:_ 🟠 _leptoskit_admin_

_Owner:_ Ifiok Jr. · _Introduced in:_ [c3f0a63](https://github.com/leptoskit/leptoskit/commit/c3f0a63a4bb79a192f4abb5aabdb45dcfe308222)

`leptoskit_admin` is one web app for looking after a deployed leptoskit site: its infrastructure, its analytics, and short lessons in both. It is its own app with its own address, and people sign in through your site's own sign-in: they press **Continue**, sign in on your site, and come back. Only accounts your site gives the `admin` role (or the roles you pass to `.roles(…)`) get in; anyone else sees who they signed in as, can switch account, and can read how admins are chosen.

Infrastructure shows what's serving, the changes waiting in `leptoskit.toml` with the plan to review, releases, operations, health from your servers' route metrics, backups, and the configuration. Plan, apply, deploy, first release, maintenance, roll back, health checks, and backup verification each ask you to confirm, run on the deployment service's worker under your name, and stream their progress to the page. Analytics teaches until your app sends its first typed event, and Learn's lessons tick themselves when they become true of your projects. Every card has a **Workings** drawer that walks it back from the check, to the command, to the plan, to the generated `main.tf.json`, to the `leptoskit.toml` lines that declare it.

Build one with `Admin::builder(app_name)`, `.main_site(MainSite::new(issuer, client_id, secret))`, `.keys(…)`, `.data_dir(…)`, and `.control(service)`, then serve `admin.router()`. The `admin_demo` example runs a site and its admin side by side.

#### The analytics builders run as the pages' own Rust

_Packages:_ 🟢 _leptoskit_admin_, 🟠 _leptoskit_admin_pages_

_Owner:_ Ifiok Jr. · _Introduced in:_ [72186b6](https://github.com/leptoskit/leptoskit/commit/72186b64efbc38f26112c16423fe5661dd808290)

Trends, Funnels, Retention and a saved insight are now pages the admin's Wasm client picks up. A builder control changes the page in place: the client asks the same address for the page's model (`x-admin-page: json`), keeps focus and the open Workings drawer where they were, marks the code lines the change changed, says which in the drawer, and scrolls to the first; Back and Forward work the same way. The `+ Add series`, `Breakdown` and `Change` menus are `leptoskit::aria::Menu`s. Save and Add to dashboard ask through the confirm sheet, which can now start a field with a value and offer choices (`Ask::named`, `Ask::choices`, and an `Answer` for what was chosen). A builder opens with its Workings showing from `?workings=<kind>`, and the Ask tabs are a `Tabs` set. The confirm sheet now gives focus back to the control that opened it even when that control never had focus, such as after a pointer press in Safari.

The builders no longer re-read themselves as new events arrive; that comes back with Live's event stream.

- **Dashboards and actors run as the pages' own Rust.** A dashboard, the people behind a point of a chart, and the pages an analytics screen shows instead of its data (connect the service, the service didn't answer, waiting for the first event) are now pages the admin's Wasm client picks up (`Page::Dashboard`, `Page::Actors`, `Page::Notice`). A dashboard and a page waiting for its first event read themselves again in place when the analytics stream has newer events, at most every ten seconds; the waiting page turns into its builder once events exist. _Packages:_ 🟢 _leptoskit_admin_, 🟠 _leptoskit_admin_pages_ _Owner:_ Ifiok Jr. · _Introduced in:_ [1b2dfab](https://github.com/leptoskit/leptoskit/commit/1b2dfab691289e6ebcbf3b3c49d24f08449c4380)
- **A project's Health tab runs as the pages' own Rust.** A project's Health tab is now a page the admin's Wasm client picks up (`Body::Health`), like its other tabs: the health check with its Run button, and the fleet's last hour of host, export and latency charts. The pages crate turns on leptoskit's `apm` feature to draw them. _Packages:_ 🟢 _leptoskit_admin_, 🟠 _leptoskit_admin_pages_ _Owner:_ Ifiok Jr. · _Introduced in:_ [2a3d73d](https://github.com/leptoskit/leptoskit/commit/2a3d73dfcf478704fd3576501b7c98a8ca4eb7a5)

#### Projects, operations and Workings drawers run in Rust

_Packages:_ 🟠 _leptoskit_admin_, 🟠 _leptoskit_admin_pages_

_Owner:_ Ifiok Jr. · _Introduced in:_ [2685e5b](https://github.com/leptoskit/leptoskit/commit/2685e5bfb10ed119600dccfd0316bf9efc0612d4)

The Projects page and each operation's page now come from `leptoskit_admin_pages` and hydrate in the browser, with no `admin.mjs`. Their Workings drawers are Rust too: opening one moves focus to its Close button, closing it returns focus, `W` toggles it, Escape closes it, and Copy uses the clipboard plugin. Link to an open drawer with `?workings=<id>`.

A running operation is followed live over the admin's own live connection at `/_leptoskit/admin/live`, which only a signed-in admin can open. The old `/infra/operations/{id}/events` endpoint is gone.

- **The admin's Learn hub and lessons run in Rust.** The Learn hub and every lesson page now come from `leptoskit_admin_pages` and hydrate in the browser, like the Glossary. They load no `admin.mjs`. Each lesson is still checked against your project's real state on the server, and the page shows the result. _Packages:_ 🟠 _leptoskit_admin_, 🟠 _leptoskit_admin_pages_ _Owner:_ Ifiok Jr. · _Introduced in:_ [10b163d](https://github.com/leptoskit/leptoskit/commit/10b163d3e4e540eadb7ba50d865228cf9859c10e)

#### Live, the pulse and the builders follow an rpc stream

_Packages:_ 🟢 _leptoskit_admin_, 🟠 _leptoskit_admin_pages_

_Owner:_ Ifiok Jr. · _Introduced in:_ [d2e310d](https://github.com/leptoskit/leptoskit/commit/d2e310da369a6ffa9109362bd7e89cf1ad9dfca5)

The Live page is now a page the admin's Wasm client picks up. The admin's analytics stream is an rpc stream, `ops::analytics_live`, served on the admin's own live connection beside an operation's progress: it sends the pulse about once a second and, on Live, each event as it arrives. Live adds each event at the top (keeping 200), holds them while paused and shows how many wait on `Resume (n)`, narrows the list by name as you type, and opens a row to its properties. The rate card and the top bar's pulse follow the stream on every page. The builders read themselves again when there are newer events, at most every ten seconds, unless a menu, a row, the confirm sheet or a field is open.

`model::Pulse` gains `per_minute`, `reason` and `latest`; all three default when absent.

#### The admin's pages start moving to Rust in the browser

_Packages:_ 🟠 _leptoskit_admin_, 🟠 _leptoskit_admin_pages_, 🟠 _leptoskit_cli_

_Owner:_ Ifiok Jr. · _Introduced in:_ [7af0a28](https://github.com/leptoskit/leptoskit/commit/7af0a28a094034a9ef5649ac6fee38e6c4eecf93)

The admin's pages now live in a new crate, `leptoskit_admin_pages`, which builds for the server and for Wasm. `leptoskit_admin` renders a page there from a small model, and the browser hydrates the same component tree, so the page's behaviour is Rust instead of the hand-written `admin.mjs`.

The Glossary is the first page to move. Its theme switch, account menu, search, ⌘K (Ctrl+K elsewhere) to find a screen, and Shift+W for Workings mode all run in Wasm. Every other page still uses `admin.mjs` for now.

Build the client with `leptoskit build --release --platform web` in `crates/leptoskit_admin_pages`, and give the admin its output directory with the new `AdminBuilder::client_dir`, or `lk admin serve --client-dir` (also read from `LEPTOSKIT_ADMIN_CLIENT_DIR`). The admin serves it from `/_leptoskit/admin/pkg/`. Without it, moved pages render but don't respond to presses.

`Environment` now comes from `leptoskit_admin_pages` and is re-exported as before. Parsing one fails with `UnknownEnvironment` instead of `AdminError::Config`.

- **Library, People and a person's page run in Rust.** The analytics Library, the People list and each person's page now come from `leptoskit_admin_pages` and hydrate in the browser, with no `admin.mjs`. Search and filter build the list's address in Rust. Export asks first, and its confirm button is a download link, so the browser downloads the server's attachment itself. Delete asks for the person's name and keeps its button disabled until the name matches. _Packages:_ 🟠 _leptoskit_admin_, 🟠 _leptoskit_admin_pages_ _Owner:_ Ifiok Jr. · _Introduced in:_ [70f15bb](https://github.com/leptoskit/leptoskit/commit/70f15bbb7593bc69fa743ec4c330fc7a9faac3b7)
- **A project's tabs and actions run in Rust.** A project's Overview, Changes, Releases, Operations, Backups and Config tabs now come from `leptoskit_admin_pages` and hydrate in the browser, with no `admin.mjs`. Their actions ask through a confirm sheet built on leptoskit's `Modal`, which keeps focus inside it and gives it back to the button that asked. Pressing a plan row opens the plan's Workings with the lines that cause it marked. Health is still drawn by the server for now. _Packages:_ 🟠 _leptoskit_admin_, 🟠 _leptoskit_admin_pages_ _Owner:_ Ifiok Jr. · _Introduced in:_ [535aa35](https://github.com/leptoskit/leptoskit/commit/535aa353b341e4e4479d5c9ce19f2b407dbf7b55)
- **The admin ships no hand-written JavaScript.** `assets/admin.mjs` is gone, with its `{BASE}/assets/admin.mjs` route, the server-sent events at `{BASE}/api/analytics/live`, and the server-drawn screens it ran on. Every admin page is drawn by `leptoskit_admin_pages` and picked up by its Wasm client, and the analytics stream is the `analytics_live` rpc stream on the admin's live connection. _Packages:_ 🟠 _leptoskit_admin_ _Owner:_ Ifiok Jr. · _Introduced in:_ [baadbbc](https://github.com/leptoskit/leptoskit/commit/baadbbc4d0764fc5a509061fc8c241324e91c921)
- **Sign-in and refusal run as the pages' own Rust.** The admin's sign-in page, and the page that turns away someone who isn't an admin, are now `Page::SignIn`, drawn without the shell and picked up by the admin's Wasm client. Continue and Switch account start sign-in with `leptoskit::auth::client::sign_in_with_provider` (Switch account asks with `Prompt::Login`), and a main site that can't start says why on the card. _Packages:_ 🟢 _leptoskit_admin_, 🟠 _leptoskit_admin_pages_ _Owner:_ Ifiok Jr. · _Introduced in:_ [3f87203](https://github.com/leptoskit/leptoskit/commit/3f8720317586bf893da312f223e99b8dfd0cc6df)

#### Honest simulations and Workings you can follow

_Packages:_ 🟠 _leptoskit_admin_

_Owner:_ Ifiok Jr. · _Introduced in:_ [19fc493](https://github.com/leptoskit/leptoskit/commit/19fc493af2a2b28674f7c64e676c96f7080601fe)

`AdminBuilder::simulated_infrastructure(true)` tells the admin that its control service's worker runs against a stand-in cloud. Every screen then shows **Simulated** beside the environment pill, Overview and Changes say that plans and applies contact no provider, the plan and apply confirmations say they act on the stand-in, and Learn says a plan was applied on the simulated cloud. It's off by default, so a real admin keeps its real-infrastructure wording. `admin_demo` turns it on unless it runs with `--executor lk`.

The Workings drawer is easier to follow:

- Changing the builder while a drawer is open scrolls the drawer (and only the drawer) to the first line that changed, marks its station **changed**, and says what changed under the drawer's title. Focus stays where it was, reduced motion jumps instead of scrolling, and a live refresh never moves the drawer.
- An in-place page update keeps the drawer's scroll position, the stations you opened, and your focus.
- **Copy** sits in a row above each code block, labelled with the language, so it never covers code.
- On phones, stations start as their headers and open when pressed.
- The Ask station's Rust, SQL, and HTTP tabs are real tabs: `aria-selected`, one tab stop, and the arrow keys, Home, and End move between them.
- Escape and Close give focus back to the control that opened the drawer, such as a plan row.

On Live, a typed event always shows its `struct` badge, even when it arrives after an untyped one.

Simulation wording also covers deploy, first release, maintenance, rollback, health checks, and backup verification. Confirmations and lesson evidence distinguish sample results from real work, and simulated plans never claim to execute the operator CLI. The browser CI suite covers Workings, live events, chart colours after builder changes, and confirmation dialog focus.

Releases recorded in the same second keep their journal order, so the current release and rollback choices do not depend on random operation IDs.

Changing a chart's breakdown updates its generated styles immediately, keeping the same series colours as a full reload. Confirmation dialogs keep keyboard focus inside, make the background inert, and return focus to their opener when dismissed; Escape no longer also closes Workings behind the dialog.

Live's Send station measures the newest event fetched for the page, including typed events. It no longer says that no event has arrived when the list already contains events.

- **The web overview runs as the pages' own Rust.** The web overview is now a page the admin's Wasm client picks up (`Page::Web`). Its range, compare and filter chips and its table rows change it in place, the way a builder's controls do, and it reads itself again as events arrive. Add to dashboard asks through the confirm sheet. _Packages:_ 🟢 _leptoskit_admin_, 🟠 _leptoskit_admin_pages_ _Owner:_ Ifiok Jr. · _Introduced in:_ [a8220d3](https://github.com/leptoskit/leptoskit/commit/a8220d3b0ace6e1d3f320e5a83112745f408ddee)

#### The public demo's service runtime

_Packages:_ 🟠 _leptoskit_admin_

_Owner:_ Ifiok Jr. · _Introduced in:_ [19fc493](https://github.com/leptoskit/leptoskit/commit/19fc493af2a2b28674f7c64e676c96f7080601fe) · _Last updated in:_ [108798e](https://github.com/leptoskit/leptoskit/commit/108798e4b4e8c0df3bf25c8bac41bc172500e0e6)

`examples/demo_service` is the public demo's process: one image per UTC day, a daily generation published with a durable marker under kernel file locks, and a same-PID re-exec at midnight. It serves the site, the admin, and a private loopback (health checks and the token-authenticated analytics prefix), refuses every other `Host` with 421, and caps the `/demo/generation` event stream at 1,000 connections. Termination is authoritative: the SIGTERM listener installs before serving and stays armed through the drain and the runtime's shutdown, so a signal that races a midnight reset or a bounded retry — in either order — wins, and the image exits 0 instead of re-executing. Successful operation event streams end after 15 seconds so an overlapping candidate reconnects into the durable terminal state. Origins, the bind, and `Host` headers normalize to one authority representation — lowercase hosts with a single DNS trailing dot, canonically bracketed IPv6, explicit or implicit default ports equal — so misconfigured origins that fold into one authority refuse to start, and requests with a missing, duplicated, or malformed `Host` are refused 421. Visitor onboarding, visual design, and provider deployment are later tasks; operator rules live in `docs/agents/public-demo.md`.

On Linux the service refuses to start while a nonstandard file descriptor survives into it, with no exemption by name anymore: the startup guard closes its own `/proc/self/fd` enumeration handle before inspecting, so a spare open of `/proc/self/fd` itself — or a handle to any file or directory named `fd` — is refused like any other inherited descriptor, and an inspection failure other than a genuinely closed descriptor is fatal rather than silently excused. The Linux-only process proofs live in `examples/demo_service/tests/inherited_descriptors.rs`.

The demo's analytics uses a 5 ms collection window for its full-month seed, preserving committed ingestion and the publication barrier while leaving production analytics defaults unchanged. Five alternating fresh-volume release runs on a warmed macOS host measured a 0.49 s median to readiness; Linux deployment readiness still needs target-host verification.

The service is Unix only, deliberately: its runtime modules, dependencies, and integration tests are gated to `cfg(unix)`, so the workspace still checks on Windows while the binary refuses to start there — a nonzero exit naming the Unix `flock`/`exec` requirement and the Linux deployment, before reading configuration or binding. No Windows emulation exists or is planned; the same-PID exec and kernel locks are the contract.

The demo's front door now works: a visitor mints a browser-owned pass (`__Host-demo-pass`/`demo-pass`, expiring at the next 00:00 UTC, only its hash stored) and gets their own Ada or Grace at a synthetic `@demo.invalid` address. Generation format 2 adds the visitors' tables and the admin client's `roles` scope, so sign-in is real end to end: the email code is gated by pass ownership (`403 not_your_identity` before any challenge, mail, or rate limit), delivered into a bounded in-memory demo inbox (2,000 addresses, 3 messages each, a 10-minute lifetime) whose receipts are recorded durably before success and compared on read, so fresh mail is never reported lost while another process's mail is honestly `lost`; Ada reaches the admin's overview through the real OIDC flow while Grace is refused on its pages and APIs. Caps — 10 new passes per IP per hour and 5,000 per day (about 10,000 people) — are counted and inserted in one immediate transaction, so two service processes on one volume cannot race past them, and existing passes keep signing in when the day is full. `DEMO_CLIENT_IP_HEADER` names the one header the edge overwrites with the visitor's IP; unusable values refuse the request rather than pass as an unlimited caller, and without it only the socket peer is trusted. Notes, checkout, analytics emission, the browser overlap proof, and the edge deployment remain later tasks.

Visitor sign-in now refuses a configured but missing proxy-IP header, and a blank header setting refuses startup. Synthetic addresses have a database uniqueness constraint; malformed or duplicate Origin headers cannot fall back to Fetch Metadata. A dropped inbox delivery records its new receipt so an older retained code is reported unavailable rather than current.

The demo's pages are now real. The front door ("Be Ada or Grace for a day"), sign-in with the demo inbox drawn as the page's underside (Send, Mail, Read, Verify, Session), and Notes' welcome are leptoskit component trees written as `view!` markup in the admin's own colours. Sign-in is the real emailed-code flow: the code appears only in the visitor's private inbox, **Fill in code** copies it, and the states for a wrong code, a rate limit, a pass that is gone, and mail this process no longer holds all say what happened without claiming a deploy did it. A restart between sending and reading keeps the lost-mail explanation; failed sign-out leaves its button usable for another attempt. Notes opens the real admin in a new tab from the configured address; Grace follows the real refusal and can switch to Ada through the site. Every page carries the demo band with the UTC-midnight countdown measured from the server's clock, discloses the shared data and the stand-in cloud, and opens a focus-managed dialog when `/demo/generation` reports a new day; a request made while a generation builds gets a styled, script-free 503 page. `leptoskit_admin::theme_css()` now exposes the admin's `--admin-*` colour custom properties, for the light and dark appearances and `data-lk-appearance`, so a page beside the admin can share them.

Notes is now a real private notebook. A signed-in visitor saves up to 20 notes of up to 280 characters each (`GET` and `POST /demo/notes`, behind the same pass-and-session ownership as the pages, same-origin, a 4 KiB JSON body, and stable refusal codes), kept in a new `demo_notes` table whose policy lets an account read only its own rows, and kept across same-day restarts. Each saved note sends one real typed `note_created` event (word count, the persona, and a pseudonymous `v-` actor, never the text, address, or network details) through the analytics service's own ingestion route, and the answer says whether the service confirmed it: an error, timeout, or partial receipt still saves the note and reports it unconfirmed, and nothing retries. The note and its event are separate transactions, which the docs state. Reading notes sends no event. The page shows the count, the saved notes (as text, never markup), and, after a confirmed save, the exact actor and a link to the admin's Live. Generation format 3 adds the table, so a release starts the day's notebooks empty beside the recent format-2 generation. The pretend checkout is not built.

The front door is now hydrated leptoskit. Its component tree lives in a new crate, `demo_pages` (`examples/demo_service/pages`), which the service renders on the server and `leptoskit build --release --platform web` compiles to the Wasm client the browser attaches. Choosing Ada or Grace is a typed mutation (`POST /rpc/mint`) beside the unchanged form post, and the new-day dialog listens to a resumable stream on the site's live connection; both call the same stores and limits as before. The front door loads no hand-written script. The service now needs `LEPTOS_SITE_ROOT`, the directory that build wrote, and refuses to start without the client in it. Sign-in is hydrated too: the choice, the emailed code and the demo inbox are Rust on both sides, the code is sent and checked with leptoskit's Auth client, and the inbox is a typed query (`/rpc/inbox`). Notes is hydrated as well: the notebook is a `TextArea` that reads and saves through two typed operations, `notes` and `save_note`, behind the same ownership gate as `/demo/notes`, and the account menu is `leptoskit::aria::Menu`. `assets/demo.js` is deleted, and no page carries a hand-written script.

- **`[native.android] push` opts the Android app into FCM push enrollment.** It is a boolean, off by default: an app without it generates byte-for-byte as before. With it, Android generation adds the pinned `com.google.firebase:firebase-messaging` dependency, writes `.leptoskit/android/gradle-plugins.txt` with Google's services plugin, generates the messaging service that forwards FCM token refreshes to the notifications plugin, and declares that service in the manifest. Push on also requires the `notifications` plugin in `[native] plugins`, the app author's `google-services.json` at `hosts/android/app/google-services.json` — never generated or committed by leptoskit — and an app Gradle script that reads the generated plugins file (the current `leptoskit new` scaffold does; older apps get the exact lines from the generation error). The configuration schema carries the new key. _Packages:_ 🟠 _leptoskit_config_ _Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)

#### Add cloud launch/deploy, storage config, and MCP tools

_Packages:_ 🟠 _leptoskit_config_, 🟠 _leptoskit_cli_

_Owner:_ Ifiok Jr. · _Introduced in:_ [94859ef](https://github.com/leptoskit/leptoskit/commit/94859efebd25f1bfb3f1c055c3172de4c309cad9)

- Adds `[cloud.storage.buckets.<name>]` configuration (`public`, `max_size_gb`) and updates the published `leptoskit.toml` JSON schemas.
- Adds `lk cloud launch`, `lk cloud deploy`, `lk cloud status`, `lk cloud logs [--follow]`, `lk cloud estimate`, and `lk cloud open`, wires `lk deploy` to dispatch to LeptosKit Cloud whenever `[cloud].project` is set, and adds Cargo-style external subcommand dispatch (`lk cloud <name>` → `leptoskit-cloud-<name>` on `PATH`, enabling `lk cloud platform`, `lk cloud worker`, and `lk cloud control`).
- Adds `--web-only` and `--cloud` flags to `lk new` and `lk generate app`, allows `--frontends` to combine with `--db`, `--auth`, `--web-only`, and `--cloud`, and falls back with a clear warning instead of hard-failing `lk dev` when `monosecret` is not installed.
- Exposes six cloud and database tools on `lk mcp` (`cloud_estimate`, `cloud_deploy`, `cloud_status`, `cloud_logs`, `cloud_secrets_list`, `db_check`) without ever returning secret values.

- **The groundwork for LeptosKit Cloud.** `leptoskit.toml` accepts a `[cloud]` table that says where LeptosKit Cloud should run your app and how big: a project, an optional region, and per-environment size, replicas, sleeping when idle and custom domains, plus the database's size and backups. Config checks it as it loads, with errors that name the table and key, such as a SQLite app asking for more than one replica, or sleeping while it serves live queries. The new `leptoskit_cloud` crate holds the shared types behind it: slugs, sizes, deployment states, the source archive a deploy will upload (deterministic, with secret files refused), and a monthly cost estimate for a `[cloud]` table. The cloud itself doesn't run yet, so nothing deploys there today. _Packages:_ 🟠 _leptoskit_config_, 🟠 _leptoskit_cloud_ _Owner:_ Ifiok Jr. · _Introduced in:_ [403aaf0](https://github.com/leptoskit/leptoskit/commit/403aaf07e6df872150ffa904a48c1a9d55e1b6ed)
- **`[native.ios] aps-environment` names the APNs environment an iOS app opts into: `development` or `production`, exactly Apple's entitlement strings.** Unset by default. _Packages:_ 🟠 _leptoskit_config_ _Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)
- **`[native.macos] aps-environment` names the APNs environment a macOS app opts into: `development` or `production`, exactly Apple's entitlement strings.** Unset by default. _Packages:_ 🟠 _leptoskit_config_ _Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)

#### Add desktop window overlays and background modes

_Packages:_ 🟠 _leptoskit_config_, 🟠 _leptoskit_plugin_window_state_

_Owner:_ Ifiok Jr. · _Introduced in:_ [9714421](https://github.com/leptoskit/leptoskit/commit/9714421722143255a82869b80b11b2b7be40f7ed)

- `leptoskit_plugin_window_state::desktop` adds Electron-class desktop windowing and lifecycle types: `WindowOptions` (with `floating_overlay` preset), `WindowLevel` (`Normal`, `Floating`, `AlwaysOnTop`, `Status`, `PopUpMenu`, `ScreenSaver`), `WindowChrome` (`Standard`, `Frameless`, `HiddenTitleBar`), `VibrancyMaterial` (macOS vibrancy and Windows Mica/Acrylic), `CloseBehavior` (`Quit`, `HideToTray`, `KeepRunningInBackground`, `Minimize`), `ActivationPolicy` (`Regular`, `Accessory`, `BackgroundOnly`), `TaskbarProgress`, `DockAttention`, `WindowController`, and `MemoryWindowController`.
- `leptoskit_config` adds `NativeMode::Background`, allows `menu-bar` and `background` startup modes across desktop platforms (`macos`, `windows`, `linux`), and adds the `[native.window]` configuration table (`DesktopWindowConfig`).

#### Add extension browser detection and full side panel API

_Packages:_ 🟠 _leptoskit_config_, 🟠 _leptoskit_extension_, 🟠 _leptoskit_cli_

_Owner:_ Ifiok Jr. · _Introduced in:_ [c4aa7df](https://github.com/leptoskit/leptoskit/commit/c4aa7df45dd07fbead27b9b05bba1d67ecfacd7f)

- `leptoskit_extension::browser` adds runtime browser detection (`ExtensionBrowser` covering Chrome, Edge, Brave, Opera, Arc, Vivaldi, Firefox, and Safari; `BrowserFamily`; `BrowserInfo`; `BrowserSwitch`; and `with_browser` test scoping).
- `leptoskit_extension::side_panel` adds full Chromium `chrome.sidePanel` and Firefox `browser.sidebarAction` support: `open_target`, `open_for_tab`, `close`, `toggle`, `is_open`, `set_options` / `get_options`, `set_behavior` / `get_behavior`, `get_layout`, `set_title` / `get_title`, and `capabilities`.
- Adds `leptoskit_extension::windows`, `leptoskit_extension::offscreen`, `leptoskit_extension::omnibox`, and `leptoskit_extension::native_messaging`.
- Adds `[extension].optional_permissions` and `[extension.side_panel]` `title` / `open_at_install` options to `leptoskit_config` and generated MV3 manifests.

- **JSON RPC wire and ambient context.** Queries and mutations speak JSON through serde on `POST /rpc/<name>`. `[rpc]` sets `path`, `live_path`, `transports`, and `default_transport`; the only transport is `json`. A query or mutation can omit its context argument and call `rpc::cx()` or `rpc::mutation_cx()`. _Packages:_ 🟠 _leptoskit_config_, 🟠 _leptoskit_rpc_macros_, 🟠 _leptoskit_rpc_ _Owner:_ Ifiok Jr. · _Introduced in:_ [8bd1891](https://github.com/leptoskit/leptoskit/commit/8bd1891b8d987c9abe19128343e7107d706ecd88)

#### Meta Horizon Store publishing for Meta Quest

_Packages:_ 🟠 _leptoskit_config_, 🟠 _leptoskit_cli_

_Owner:_ Ifiok Jr. · _Introduced in:_ [f9e7c49](https://github.com/leptoskit/leptoskit/commit/f9e7c495e9fb4ef73cd8458d03b103a37083ff31)

Add `[publish.quest]` configuration and `leptoskit publish quest` to build a release-signed Meta Quest APK and upload it to Meta Horizon Store release channels (`alpha`, `beta`, `rc`, `store`, or subchannels) through Meta's `ovr-platform-util upload-quest-build`.

- Configures `app_id`, `channel`, `age_group` (`teens_and_adults`, `mixed_ages`, `children`), `draft`, and optional `obb`, `assets_dir`, and `debug_symbols_dir` in `[publish.quest]` with updated JSON schemas.
- Accepts `LEPTOSKIT_PUBLISH_QUEST_APP_SECRET` (preferred) or `LEPTOSKIT_PUBLISH_QUEST_TOKEN`, supports `LEPTOSKIT_OVR_PLATFORM_UTIL` overrides, redacts secrets from tool errors, and rejects x86-only ABIs before compiling.
- Defaults to `--draft` uploads until `--submit` is passed with an `age_group`, prints the remaining Meta Horizon Developer Dashboard steps (listing copy/art, Data Use Checkup, content rating, and VRC/Content review submission), and wires `quest` into the generated GitHub Actions `release.yml` workflow.

- **Multiple frontend applications in one workspace.** `leptoskit new --frontends landing,app,admin` scaffolds a `leptoskit.workspace.toml` with separate apps under `apps/` (landing and admin web-only; `app` keeps native hosts). Workspace applications may declare an optional `kind` (`landing`, `product`, `admin`, `server`) for tooling. Docs cover the multi-frontend layout; shared auth across frontends remains out of scope. _Packages:_ 🟠 _leptoskit_config_, 🟠 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [6fed659](https://github.com/leptoskit/leptoskit/commit/6fed65923cda6adbd7f8ac645ff2555ff34fb819)

#### Versioned native capability contracts

_Packages:_ 🟠 _leptoskit_config_, 🟠 _leptoskit_bridge_ffi_, 🟠 _leptoskit_codepush_, 🟠 _leptoskit_cli_

_Owner:_ Ifiok Jr. · _Introduced in:_ [442b38b](https://github.com/leptoskit/leptoskit/commit/442b38b81352231ebb9810f646247bd548ed353c)

Native builds now resolve a typed host capability descriptor from the destination flavour, the plugin registrations they actually generated, and the non-secret plugin options. `native bundle` writes `capabilities.json` beside the bundle with a stable SHA-256 digest, and the mobile host embeds the same descriptor in `codepush.json`.

`[native]` gains `required-plugins`, `optional-plugins`, `required-capabilities`, and `optional-capabilities`. A required plugin or capability the selected host cannot run fails the build before packaging, regardless of `missing-plugins`; an optional omission warns with the platform and reason. Existing `plugins` and `missing-plugins` keep their behavior when no requirement lists are set, and a capability declaration never grants access.

Signed releases carry versioned required/optional requirements in `runtime.json`. The installed host checks them against its own descriptor before staging or activating an update; unsupported, unknown, or experimental-without-opt-in requirements are refused. Releases built before this format still pass the legacy ABI, shell, and plugin checks and are reported as legacy, never as verified-compatible. Unknown contract versions and conflicting or duplicate entries are rejected.

Guest requirements bind the bridge protocol as well as the rendering ABI. Explicitly optional plugins no longer enter the legacy required-plugin list. Watch preparation checks requirements before writing its stubs and emits a descriptor; standalone bundles place host evidence beside the output directory.

Updates whose older-style plugin grants have no available implementation report `incomplete` while preserving the existing missing-plugin fallback. Explicit required entries still reject unavailable plugins; explicit optional entries may be omitted. A versioned host descriptor must agree with its own grant list before it can check a release.

Generated iOS and Android update controls show whether host requirements were checked, plugin availability remains incomplete, or only legacy checks ran. A missing report from an older runtime is shown as unreported. None of these states claims that the updated app passed device tests.

The descriptor also names `host.timer` where the generated host enables the reserved `leptoskit.timer` plugin, so `required-capabilities = ["host.timer"]` fails before packaging on hosts that run no guest and is checked by code push on older store builds.

#### Push-enabled hosts now work the way the notifications guide describes.

_Packages:_ 🟠 _leptoskit_config_, 🟠 _leptoskit_cli_, 🟢 _leptoskit_plugin_notifications_, 🟢 _leptoskit_notifications_

_Owner:_ Ifiok Jr. · _Introduced in:_ [7f60843](https://github.com/leptoskit/leptoskit/commit/7f60843eba43202f72117226374f71d7b4f6528d)

**Android.** The scaffold's Gradle script applies Google's services plugin by id (`apply(plugin = plugin)`); the earlier `apply(plugin)` asked Gradle for a script file of that name and failed every push-enabled build. An app scaffolded with the earlier line gets a generation error that names the one-word change. The generated messaging service is declared with the `com.google.firebase.MESSAGING_EVENT` intent filter, so Firebase delivers token refreshes to it instead of to its own default service. The manifest names a default notification channel, `leptoskit.notifications.default`, which the notifications plugin creates at startup, and the notification icon when `[app.icons] notification` generated one. Release builds get R8 keep rules for the Firebase names the plugin looks up by reflection, the plugin finds the task's listener methods on the public `Task` type, and Firebase without configuration answers `firebase-not-configured` even when its exception arrives wrapped by the reflective call.

**Android channels.** The FCM transport sends `android.channel_id` under the `leptoskit.notifications.` prefix the plugin creates channels with, so a send that names `inbox` reaches the channel the app configured as `inbox`. An id that already carries the prefix is sent unchanged.

**iOS and macOS.** The permission request asks for alerts, badges, and sounds, so a push's `aps.badge` and `aps.sound` take effect where the person allows them; it asked for alerts only.

**macOS.** A push-enabled Mac app (`[native.macos] aps-environment`) now needs `[native.macos] provisioning-profile`, a new setting naming the profile to embed: Apple's push entitlement is restricted, and macOS refuses to launch an app that claims it without a matching embedded profile. Generation stops without the profile. `leptoskit build --platform macos` embeds it at `Contents/embedded.provisionprofile` and signs with the identity in `LEPTOSKIT_MACOS_IDENTITY`, refusing to ad-hoc sign; `--release --unsigned` builds without the push entitlement. `leptoskit publish macos` embeds the profile before signing with the Developer ID identity. This follows Apple's signing rules and has not been run against a real Apple developer account. Apps without `aps-environment` generate and sign exactly as before.

**Web.** The receiver worker claims the pages already open when it activates, so a click can route the tab a person enrolled from, and a click on an action button adds the button's id to the opened address as `lka`.

The guide's samples no longer stop on iOS and macOS at `configure_channel` or enroll after a refused permission, and it now covers Web Push on iPhone and iPad needing a Home Screen install, combining the receiver with an app's own service worker, which `aps-environment` value an iOS build wants, and the entitlement a time-sensitive push needs.

- **Publish to the Solana dApp Store.** `[publish.solana]` adds Solana Mobile's store for Seeker and Saga phones to `leptoskit publish`: without `--submit` the run builds the release-signed APK with your dApp Store keystore, checks the listing text and images, the signing key (which must differ from Google Play's), the `arm64-v8a` build target, and the publishing wallet's balance (0.016 SOL minimum, through one RPC call that may only warn on an outage), and sends nothing, because Solana Mobile's official `dapp-store` tool has no draft stage; with `--submit` it runs that tool against the Publisher Portal with the APK, the wallet keypair, and "What's New" from `store/en-US/release_notes.md`, the portal API key held only in that one tool's environment. The keystore reuses the release-signing variables under `LEPTOSKIT_SOLANA_*` names, `leptoskit publish credentials` lists the six new credentials, and the generated release workflow sets up Node and decodes the keystore for it. _Packages:_ 🟠 _leptoskit_config_, 🟠 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [78d16fe](https://github.com/leptoskit/leptoskit/commit/78d16feec6021830c6a57befe95ca0eb4b06bae9)

#### `[rpc].path` and `[rpc].live_path` decide where routes serve

_Packages:_ 🟠 _leptoskit_config_, 🟠 _leptoskit_rpc_, 🟢 _leptoskit_cli_

_Owner:_ Ifiok Jr. · _Introduced in:_ [8bd1891](https://github.com/leptoskit/leptoskit/commit/8bd1891b8d987c9abe19128343e7107d706ecd88)

`RpcConfig.path`, `live_path`, and `transports` were accepted and validated but no server read them: every app served `/rpc` and `/live` whatever the config said. Now they decide the mounts, once, at the app's entrypoint.

The generated app reads `[rpc]` from `leptoskit.toml` and applies it: operation routes serve at `path` (with the JSON routes kept on the legacy `/api` prefix while JSON is a transport), and `Server::rpc_config` carries the resolved live mount through `Server::live`. `Live` takes the same config, so a router built by hand mounts the socket at `live_path` — `/live` while the config stays at its defaults — with its server-sent events and frame posts right next to it. `path` without `live_path` moves the socket to `{path}/live`; an explicit `live_path` mounts it anywhere.

`leptoskit rpc spec` exports the configured live mount, so regenerated clients open their socket where the app serves it. Generated HTTP calls still name the compile-time `/rpc/<op>` route: keep the default `path` when generated clients call the app directly, and use a custom one behind a proxy or gateway that remounts the prefix.

#### Describe independently configured applications

_Packages:_ 🟠 _leptoskit_config_, 🟠 _leptoskit_cli_

_Owner:_ Ifiok Jr. · _Introduced in:_ [b47bc2e](https://github.com/leptoskit/leptoskit/commit/b47bc2e41920c89e7382665a7fccab1c9508de9d)

Add a separate `leptoskit.workspace.toml` manifest and `lk workspace explain` / `check`. Each application references its own configuration. Startup dependencies are validated independently of call bindings, so bidirectional calls do not become readiness cycles. Unknown references, escaping paths, repeated configurations, conflicting managed identities, and duplicate binding variables fail before a build starts.

The versioned report exposes only selected configuration and environment variable names. It explicitly marks artifacts, API compatibility, database ownership, service identity, and deployment as not checked. Workspace inspection runs no plugin programs or build tools. Image creation, binding injection, change impact, and release orchestration remain future consumers of this graph.

#### Meta Horizon platform extension and MHCP packager

_Packages:_ 🟠 _leptoskit_platform_horizon_, 🟠 _leptoskit_cli_

_Owner:_ Ifiok Jr. · _Introduced in:_ [79d602f](https://github.com/leptoskit/leptoskit/commit/79d602ffab7e56bd032ca83be7a1fae42d704d98)

Add the experimental `leptoskit_platform_horizon` platform extension (`horizon`), registered under `leptoskit native platform` alongside `harmonyos` and `oculus`.

- **`leptoskit native platform scaffold horizon`** generates `hosts/horizon/` with Meta Horizon Worlds Desktop Editor TypeScript Custom UI host scripts (`scripts/scene.ts`, `scripts/LeptosKitHost.ts`, `scripts/scene_test.ts`) implementing leptoskit `version: 1` render transactions over `horizon/ui`, plus Meta Horizon Creator Program (MHCP) offline web submission templates (`mhcp/index.html`, `mhcp/BUILD_LOG.md`).
- **`leptoskit native platform package horizon`** builds and validates a deterministic offline HTML5/WebGL/Wasm `.zip` archive for MHCP hackathon submissions: rewrites root-relative asset references in `index.html` to `./...`, omits `.br`/`.gz` server sidecars, rejects external CDN/network URLs and missing relative assets, and enforces the configurable `--max-mb` size cap (default 35 MiB).

#### Android alarms that run app code

_Packages:_ 🟠 _leptoskit_bridge_ffi_, 🟠 _leptoskit_plugin_android_alarm_manager_, 🟠 _leptoskit_cli_

_Owner:_ Ifiok Jr. · _Introduced in:_ [24d6fbc](https://github.com/leptoskit/leptoskit/commit/24d6fbcae5293e9d397cd1d06938ce3ced7f6b54)

`leptoskit_plugin_android_alarm_manager` ports Flutter's `android_alarm_manager_plus`. Guests schedule one-shot and repeating Android `AlarmManager` alarms by integer ID, with exact, wakeup, allow-while-idle, alarm-clock and reschedule-on-reboot options, check and request the Android 12+ exact-alarm permission (or fall back to inexact timing), cancel and list alarms, and receive fired alarms through a bounded on-device queue that survives process death: `next()` waits for one, `drain()` takes the ones that fired while the app wasn't running. An alarm marked `headless` starts the app's guest without a window when no guest is running, so `handle_launch` can run app code for it within an eight-second budget. iOS, macOS, Windows, Linux, HarmonyOS and the browser return `Unavailable`.

The plugin declares its alarm and boot receivers with the `android-receivers` runtime metadata, which the host-project docs now describe. The Android host gains `NativeHeadlessGuest`, which runs the bundled guest with no window for a receiver or job and discards what it draws.

#### Android Back goes back a route

_Packages:_ 🟠 _leptoskit_bridge_ffi_, 🟠 _leptoskit_wasm_, 🟢 _leptoskit_cli_

_Owner:_ Ifiok Jr. · _Introduced in:_ [a93214e](https://github.com/leptoskit/leptoskit/commit/a93214edc8e5f6c7999c628285093f15cf188639)

The Back gesture and Back button in an Android app now go back one route through the file router, and leave the app only from the first route. Before, Back always left the app: Android 16 delivers Back to an app that targets it only through an `OnBackInvokedCallback`, which the generated activity never registered, and on every Android version the host's question to the router went nowhere.

- `leptoskit_wasm`: `WasmHost::system_back` offers Back to the guest's optional `leptoskit_system_back` export, then flushes and commits what changed. A guest without the export answers `EventOutcome::Unhandled`, and the platform leaves.
- `leptoskit_bridge_ffi`: `lk_guest_system_back` offers Back on the guest thread, and `lk_guest_take_system_back` takes the answers in order. `LeptosKitApplicationView.offerSystemBack` now takes a callback, `(Boolean) -> Unit`, that runs on the main thread with `true` when the router went back.
- `leptoskit_cli`: the generated `MainActivity.kt` registers an `OnBackInvokedCallback` on Android 13 and later and keeps `onBackPressed` for earlier versions; the generated manifest sets `android:enableOnBackInvokedCallback="true"`.

Apps scaffolded before this keep leaving on Back until they copy the new `MainActivity.kt` Back code and the manifest attribute from a new project; [the Android page](https://leptoskit.dev/docs/platforms/android.html#the-back-gesture) lists them.

#### File-route navigators, deep links, and system Back

_Packages:_ 🟢 _leptoskit_bridge_ffi_, 🟢 _leptoskit_cli_, 🟠 _leptoskit_router_, 🟠 _leptoskit_router_codegen_

_Owner:_ Ifiok Jr. · _Introduced in:_ [bbc358b](https://github.com/leptoskit/leptoskit/commit/bbc358babf47a0c482e74527c7011408f620dfe8)

Layouts can export `navigator()` returning a typed `Stack`, `Tabs`, or `Drawer`. Folder groups like `(tabs)/` stay URL-less; the navigator return value chooses the chrome. Tab and drawer roots are `impl Into<Route>`, not string names. Stacks can present matched routes as a sheet or modal above the previous screen, and optionally offer an edge hit target for Back.

`Navigation::open_url` / `open_url_with` turn incoming URLs into typed routes (custom schemes and `http(s)`). `apply_activations` drains a host deep-link queue into the router. `bind_system_back` and `leptoskit_system_back` let Android system Back pop router history when the host offers it. Generated Android activities call into that path before finishing.

[`examples/file_routes`](https://github.com/leptoskit/leptoskit/tree/main/examples/file_routes) shows Stack + Tabs + Drawer, deep links, and Back. True UIKit interactive-pop and AppKit history swipe still need host presentation; the guide names those gaps.

- **Add `leptoskit_plugin_contacts` for contacts permission, system pick-one, and a bounded query (max 32) on iOS and Android.** Desktop and browser hosts report unavailable. _Packages:_ 🟠 _leptoskit_plugin_contacts_, 🟢 _leptoskit_ _Owner:_ Ifiok Jr. · _Introduced in:_ [3820e86](https://github.com/leptoskit/leptoskit/commit/3820e86dfca1283e2ed1f36e4a34befdfc03a564)
- **Home-screen and desktop widgets.** `leptoskit_plugin_home_widgets` publishes bounded text snapshots for iOS and macOS WidgetKit and Android App Widgets, then asks the OS to reload them. Guests call `guest::HomeWidgets` for capability, set/get/clear, reload, and installed queries. Apps still own their WidgetKit extensions and App Widget providers; Apple needs an App Group for a shared suite, and Windows, Linux, and the browser remain unavailable. _Packages:_ 🟠 _leptoskit_plugin_home_widgets_ _Owner:_ Ifiok Jr. · _Introduced in:_ [bca6fc6](https://github.com/leptoskit/leptoskit/commit/bca6fc665b53b3f5945c25c13d6b029024cf541c)

#### Add mobile widget surfaces and OS integrations

_Packages:_ 🟠 _leptoskit_plugin_home_widgets_, 🟠 _leptoskit_plugin_live_activities_, 🟠 _leptoskit_plugin_app_shortcuts_

_Owner:_ Ifiok Jr. · _Introduced in:_ [c3e229a](https://github.com/leptoskit/leptoskit/commit/c3e229abe4ab8bec104922c3719c46ae3fe76c73)

- `leptoskit_plugin_home_widgets::surfaces` adds `SurfaceKind` (Home Screen, Lock Screen circular/rectangular/inline, Control Center, Android Quick Settings tile, Android Glance), `TimelineSchedule` / `TimelineEntry` / `ReloadPolicy`, `WidgetAction`, `ControlCenterControl`, and `QuickSettingsTile` with binary bridge codecs.
- `leptoskit_plugin_live_activities::island` adds `DynamicIslandLayout`, `ActivitySchedule` (stale date, relevance score, push-to-start), and `AndroidLiveUpdate` / `ProgressSegment` for Android 16+ Live Updates.
- `leptoskit_plugin_app_shortcuts::integrations` adds `AppIntentDefinition`, `SpotlightItem` (`CoreSpotlight` / `AppSearch`), `UserActivity` (Apple Handoff), `IncomingShare`, `StatusBarConfig`, `SheetDetent`, `DynamicColorRole` (Material You), `BubbleConfig` (Android Bubbles), `MultiSceneConfig` / `PencilPreference` (iPadOS), and `MobileIntegrationHost` / `MemoryMobileIntegrationHost`.

#### Wire mobile Live Activities, widgets, Spotlight, and shares

_Packages:_ 🟠 _leptoskit_plugin_home_widgets_, 🟠 _leptoskit_plugin_live_activities_, 🟠 _leptoskit_plugin_app_shortcuts_

_Owner:_ Ifiok Jr. · _Introduced in:_ [7910a8f](https://github.com/leptoskit/leptoskit/commit/7910a8fd84838cf6f59dab3347c4233cefa0575b)

- `leptoskit_plugin_live_activities`: maps `DynamicIslandLayout` slots onto `ContentState` and `LeptosKitLiveActivityAttributes`, adds `wait_push_to_start_token` (iOS 17.2+), `start_scheduled` / `update_scheduled` (`staleDate` + `relevanceScore`), and `post_android_live_update` / `end_android_live_update` (Android 16+ `Notification.ProgressStyle` with ongoing progress notification fallback).
- `leptoskit_plugin_home_widgets`: adds bridge methods for `TimelineSchedule`, iOS 18+ `ControlCenterControl` (`ControlCenter.shared.reloadControls`), and Android `QuickSettingsTile`, reads `LeptosKitAppGroup` automatically from `Info.plist` on Apple, and adds `LeptosKitWidgetStore` / `HomeWidgetsPlugin.readSnapshot` / `readQuickTile` helpers for native widget extensions and tile services.
- `leptoskit_plugin_app_shortcuts`: wires `CoreSpotlight` indexing (`index_spotlight` / `delete_spotlight`), Apple Handoff (`start_user_activity` / `end_user_activity`), and incoming share intake (`consume_incoming_share`) on both Apple and Android hosts.

- **Cellular carrier and radio snapshot.** `leptoskit_plugin_cellular` reads carrier name, MCC/MNC, radio generation, and VoIP allowance when the platform exposes them. Unknown, Unavailable, and Restricted stay explicit. iOS uses CoreTelephony, Android uses permission-light TelephonyManager fields, and desktop plus the browser answer Unavailable. _Packages:_ 🟠 _leptoskit_plugin_cellular_ _Owner:_ Ifiok Jr. · _Introduced in:_ [55eb268](https://github.com/leptoskit/leptoskit/commit/55eb268983de0dbd62740a84d2f0e8202a475bb7)
- **Electron desktop parity, first wave.** Desktop hosts get Electron-shaped lifecycle APIs without embedding Chromium: `power_monitor` for suspend/resume/lock/AC events and Windows idle queries, `deep_link` for inbound URL/file activations and association descriptors, `updater` hooks with pluggable backends (separate from code push), and second-launch argument handoff on `single_instance`. Docs map Electron concepts to leptoskit plugins and name the remaining gaps. _Packages:_ 🟠 _leptoskit_plugin_deep_link_, 🟠 _leptoskit_plugin_power_monitor_, 🟠 _leptoskit_plugin_single_instance_, 🟠 _leptoskit_plugin_updater_ _Owner:_ Ifiok Jr. · _Introduced in:_ [e4009fd](https://github.com/leptoskit/leptoskit/commit/e4009fd59c9cbe0bc1ebb4391484882e74d0374e)

#### Device and package details, as in device_info_plus

_Packages:_ 🟠 _leptoskit_plugin_device_, 🟠 _leptoskit_plugin_application_

_Owner:_ Ifiok Jr. · _Introduced in:_ [dd17416](https://github.com/leptoskit/leptoskit/commit/dd174162dfd43b30f06f7e21c69784c1e1e52394) · _Last updated in:_ [d0a8882](https://github.com/leptoskit/leptoskit/commit/d0a8882eeb43751a00f3e89cc07fb7b4e0b97413)

`leptoskit_plugin_device` now reports what Flutter's `device_info_plus` does. Every platform adds a model identifier (`iPhone15,2`, `Mac14,2`), kernel release, available memory, data-volume size and free space, and CPU count, and a new `Snapshot::platform` section carries one platform's own details: Android `Build` fields (SDK and preview SDK level, security patch, fingerprint, `Build.ID`, board, ABI lists, sorted system features, low-RAM class), Apple flags (iOS app on Mac or Vision, Mac Catalyst, OS build, CPU frequency, localized model), Windows release registry values, Linux `os-release` fields, and browser client-hint brands. `web::high_entropy_values()` asks the browser for its detailed hints only when you call it.

Persistent identifiers stay out of the snapshot. The new `Device::identifiers()` call returns the user-assigned device name, the iOS vendor identifier and the desktop machine identifier, each `None` where the platform withholds it; on iOS 16 and later the device name needs Apple's user-assigned-device-name entitlement.

`leptoskit_plugin_application` now reports `package_info_plus`'s remaining fields: the Android signing certificate's SHA-256 digest, the installer store (the installing package on Android; App Store, TestFlight, development or simulator on iOS, guessed from the receipt), and install and update times. `executable_times()` gives Rust hosts the executable's file times.

Both snapshots move to wire version 2. Decoders still read version 1 replies from older native hosts, but a guest built before this release cannot read a version 2 reply, so ship guests built against this version with the updated host. `Snapshot` gained public fields, so code that builds one with a struct literal needs `..Default::default()`.

- **Android device-policy plugin.** `leptoskit_plugin_device_policy` exposes an explicit, least-privilege Device Admin / Device Policy Manager surface for Android: status without prompting, user-driven activation, lock, password quality and minimum length, camera disable, confirmed wipe, and admin removal. Non-Android platforms return `Unavailable`. Apps own the manifest receiver and the `device_admin.xml` policy file so they declare only the policies they need. Wipe requires `WipeRequest::factory_reset()`; empty payloads cannot factory-reset a device. Google Play and enterprise-policy constraints still apply and are documented with the API. _Packages:_ 🟠 _leptoskit_plugin_device_policy_ _Owner:_ Ifiok Jr. · _Introduced in:_ [23482c7](https://github.com/leptoskit/leptoskit/commit/23482c7a0485da518e8aedd022a54d9c8a49338d)
- **`leptoskit dev` serves phone guests on the local network and prints a QR code for them.** The LeptosKit app scans that code, or takes the address, and runs the unsigned debug guest. Only a local `http` or `https` address is accepted. _Packages:_ 🟠 _leptoskit_plugin_dev_client_, 🟠 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [a7b4d40](https://github.com/leptoskit/leptoskit/commit/a7b4d400154fef9c3dcc13e434766a66240c90b2)

#### Android intents plugin

_Packages:_ 🟠 _leptoskit_plugin_android_intent_

_Owner:_ Ifiok Jr. · _Introduced in:_ [5261e0a](https://github.com/leptoskit/leptoskit/commit/5261e0a1a578c6ba300de59223516633170f944b)

`leptoskit_plugin_android_intent` ports Flutter's `android_intent_plus`: guests describe an intent with `Intent::new().action(…).data(…).extra(…)` and launch it, open it through a titled chooser, send it as a broadcast, start a service, check whether an activity resolves it, read the package, class, and label it resolves to, or launch an `intent:` URI. Extras cover booleans, ints, longs, doubles, strings, nested bundles, byte arrays, integer and string lists, and boolean, int, long, double, and string arrays.

Only Android registers the plugin; everywhere else `guest::AndroidIntent::new` returns `Unavailable`. Requests travel as a bounded little-endian wire that the Kotlin plugin validates again, because guest code can change over the air: URI-permission grant flags and `file:` URIs are refused with `PermissionDenied`, extras can't carry parcelables, and `intent:` URIs can't name a component or selector and only reach browsable activities. A missing activity or service is `Unavailable` and an Android `SecurityException` is `PermissionDenied`. Resolution on Android 11 and later only sees apps the app's own `<queries>` declares.

#### SMS, mail composer, and dismissible web browser plugins

_Packages:_ 🟢 _leptoskit_plugin_linking_, 🟠 _leptoskit_plugin_sms_, 🟠 _leptoskit_plugin_mail_composer_, 🟠 _leptoskit_plugin_web_browser_

_Owner:_ Ifiok Jr. · _Introduced in:_ [dd9a817](https://github.com/leptoskit/leptoskit/commit/dd9a817d2905561f2dfc047b64d32947d6ed9dfa)

Three new Expo-equivalent runtime plugins cover composer and browsing presentation that were still missing:

- **`sms`** queries composer availability and presents the system SMS sheet with optional recipients and body. Completion is the OS callback only, never delivery. Outbound `sms:` opens stay in `linking`.
- **`mail-composer`** queries availability and presents the system mail sheet with to/cc/bcc/subject/body. Attachments are deferred in this MVP. Outbound `mailto:` opens stay in `linking`.
- **`web-browser`** opens HTTP(S) in Safari View Controller / Custom Tabs and answers on dismiss. It is distinct from `web-auth` (no OAuth return address) and from `linking.open` (which does not wait for dismiss).

Each plugin ships guest clients, wire codecs, Apple/Android backends (with unavailable stubs where a platform has no composer or dismissible sheet), docs, and tests. Desktop hosts mark the plugins unavailable via metadata.

#### iOS Live Activities

_Packages:_ 🟠 _leptoskit_plugin_live_activities_

_Owner:_ Ifiok Jr. · _Introduced in:_ [4c0123c](https://github.com/leptoskit/leptoskit/commit/4c0123ca69be52687c94cfb7b3b0204048ca5bb7)

Apps can start, update, and end iOS Live Activities (Lock Screen and Dynamic Island) through the new `leptoskit_plugin_live_activities` crate and the `live-activities` runtime plugin. Guests call `are_enabled`, `start`, `update`, `end`, `wait_push_token`, and `list`. Content state is a bounded string map that a Widget Extension renders via `LeptosKitLiveActivityAttributes`.

Only iOS implements ActivityKit. Android, macOS, the browser, and the unsupported host stub return `Unavailable` after validating payloads. Apps must set `NSSupportsLiveActivities` in Info.plist, ship a Widget Extension, and enable Push Notifications when requesting push tokens. Custom attribute types, push-to-start tokens, and remote payload helpers are still out of scope.

#### Every transport a connection uses

_Packages:_ 🟠 _leptoskit_plugin_network_

_Owner:_ Ifiok Jr. · _Introduced in:_ [321cb35](https://github.com/leptoskit/leptoskit/commit/321cb35d85f533cd2ba488d0f70787fb070bdacf)

A network reading now lists every transport the active connection uses, as Flutter's `connectivity_plus` does, instead of naming only one. `Snapshot::transports` is a `Transports` set of `Transport` values — Wi-Fi, cellular, ethernet, Bluetooth, VPN, satellite, and other — so an Android app on a VPN over Wi-Fi sees both, and an app on satellite data sees satellite alongside cellular. `Snapshot::connection` still names the primary one, and the set always contains it.

Android reports every transport `NetworkCapabilities` lists, including Wi-Fi Aware as Wi-Fi and satellite on Android 15 and later. iOS and macOS report Wi-Fi, cellular, ethernet, and other from `NWPath`, plus satellite when the path is ultra-constrained (iOS and macOS 26 and later); they have no VPN or Bluetooth interface type, so a VPN tunnel shows as other. The browser maps `NetworkInformation.type`, and desktop Rust still infers no transport.

The wire snapshot grows from 32 to 36 bytes, and every side refuses transport bits it does not define. Rebuild the native app together with the guest; a guest built with this release can't read an older shell's snapshots, and the reverse.

#### Wi-Fi name and local addresses

_Packages:_ 🟠 _leptoskit_plugin_network_info_

_Owner:_ Ifiok Jr. · _Introduced in:_ [321cb35](https://github.com/leptoskit/leptoskit/commit/321cb35d85f533cd2ba488d0f70787fb070bdacf)

The new `leptoskit_plugin_network_info` crate, runtime name `network-info`, ports Flutter's `network_info_plus`. `guest::NetworkInfo::snapshot` returns the Wi-Fi network's name (SSID) and BSSID, and its IPv4 address, IPv6 address, subnet mask, broadcast address, and gateway, for apps that set up or find devices on the local network.

Every field is `None` when the platform withholds it; the plugin never passes on stand-ins such as Android's `<unknown ssid>` or `02:00:00:00:00:00`, and never asks for permission. The SSID and BSSID need location permission, which the `location` plugin requests; on iOS they also need the Access Wi-Fi Information entitlement. An SSID is up to 32 bytes that need not be valid text: `Ssid::as_bytes` returns them as reported, and `Ssid::to_str` returns text when they are UTF-8.

iOS reads `NEHotspotNetwork`, macOS CoreWLAN, and Android `WifiManager` with the Wi-Fi network's `LinkProperties`. Desktop Rust reports the addresses and gateway of the interface the default route leaves through, without a name, and the browser returns `Unavailable`. It is a separate plugin from `network` so an app that only checks connectivity never declares Wi-Fi permissions or hands Wi-Fi identifiers to its guest code.

#### App shortcuts, icon badges, and NFC reads

_Packages:_ 🟠 _leptoskit_plugin_nfc_, 🟠 _leptoskit_plugin_app_shortcuts_, 🟠 _leptoskit_plugin_badge_

_Owner:_ Ifiok Jr. · _Introduced in:_ [888ff04](https://github.com/leptoskit/leptoskit/commit/888ff04cf9d89bc604a176efa427dd3c3bf4251e)

Three new runtime plugins cover high-value native capabilities that were still missing:

- **`app-shortcuts`** publishes up to eight static launcher shortcuts (iOS quick actions, Android `ShortcutManager`, macOS dock-menu store) and returns the id that launched the app.
- **`badge`** sets, reads, and clears the app icon badge count on Apple and Android, with a Linux Unity/KDE D-Bus client for custom hosts.
- **`nfc`** queries hardware readiness and reads one bounded NDEF text or URI record on iOS and Android.

Each plugin ships guest clients, wire codecs, Apple/Android backends (with stubs where a platform has no hardware), docs, and tests. Windows jump-list publishing, full Siri App Intents, background NFC, and badge Badging API support remain follow-ups.

- **Add `leptoskit_plugin_print` for HTML and PDF URI print dialogs on iOS, Android, and macOS.** Completion means the dialog finished or the platform accepted a job, never that paper left a printer. _Packages:_ 🟠 _leptoskit_plugin_print_, 🟢 _leptoskit_ _Owner:_ Ifiok Jr. · _Introduced in:_ [47c9196](https://github.com/leptoskit/leptoskit/commit/47c9196669b4cabf82c712a83d94f2adeb8eb3dd)

#### Add the recorder plugin: continuous microphone segments

_Packages:_ 🟠 _leptoskit_plugin_recorder_, 🟢 _leptoskit_plugin_background_, 🟢 _leptoskit_cli_

_Owner:_ Ifiok Jr. · _Introduced in:_ [0036fe4](https://github.com/leptoskit/leptoskit/commit/0036fe4b9d010f4c568d160e88d4550b2aa5e38a)

Add a `leptoskit_plugin_recorder` crate with a typed binary guest client, Swift and Kotlin handlers, and automatic native registration under the runtime name `recorder`. It records the microphone continuously in the background as timed audio segments: AAC-LC, mono, `.m4a`, rotated at each boundary with no gap between segments, for apps that upload and transcribe each segment themselves.

`start` takes a directory inside the app's own data or cache directories and a segment length (default 300 s, clamped 5–3600 s), sample rate (default 16 kHz), AAC bitrate (default 24 kbps) and activity threshold (default −45 dBFS). `next_segment` is a cancellable wait that returns each finished segment with its path, start time, duration, size, peak level and `active_ms` — the milliseconds of 30 ms frames above the threshold — so the app can skip uploading quiet segments without decoding them. The plugin never deletes a delivered segment and never decides whether one is silent; the app owns both choices. `status` reports idle, recording, interrupted (a call or Siri), silenced (Android: another app holds the microphone) or failed with a reason, plus the open segment's start time.

A small append-only journal beside the segments survives a crash: the next `start` (or a `recover` call) delivers leftover finalised files as `ended_by = recovered` segments and deletes the truncated files that cannot be read, reporting each deletion once. iOS records through an `AVAudioEngine` input tap into an AAC `AVAudioFile`, configures `.playAndRecord` with `.mixWithOthers`, `.allowBluetooth` and `.defaultToSpeaker` so the owner's music and calls keep working, closes the segment on an interruption and resumes when the system allows it, and rebuilds the tap on route changes and after a media services reset; recording can only start in the foreground, and the `audio` background mode keeps it running. Android records `AudioRecord` PCM into a `MediaCodec` AAC encoder and `MediaMuxer` on its own thread, reports `silenced` through the audio recording callback, and requires the `background` plugin's microphone-typed foreground service for background recording — which Android 14 cannot start from `BOOT_COMPLETED`, so the owner must open the app once after a reboot. The `background` plugin now exposes the running service's type mask so `start` can fail with a clear error instead of recording into a process the system will kill.

- **Add `leptoskit_plugin_device_motion` for fused attitude, user acceleration, and rotation-rate readings on iOS, Android, and the browser.** Desktop hosts report unavailable. _Packages:_ 🟠 _leptoskit_plugin_device_motion_, 🟢 _leptoskit_ _Owner:_ Ifiok Jr. · _Introduced in:_ [dda0a88](https://github.com/leptoskit/leptoskit/commit/dda0a88a43b312383c51c7b0c1f46cdad7005e05)
- **Add `leptoskit_plugin_age_range` for declared age-range capability status and threshold request (Declined / Unavailable / Shared bounds).** Android, desktop, and browser return Unavailable in this MVP; live OS prompts remain later scope. _Packages:_ 🟠 _leptoskit_plugin_age_range_ _Owner:_ Ifiok Jr. · _Introduced in:_ [eb316ff](https://github.com/leptoskit/leptoskit/commit/eb316ff3f779f43adc443e024dcca7cace150edf)
- **Add `leptoskit_plugin_app_integrity` for attestation capability status and bounded challenge→token requests.** Hosts return Unavailable in this MVP until App Attest / Play Integrity wiring lands. _Packages:_ 🟠 _leptoskit_plugin_app_integrity_ _Owner:_ Ifiok Jr. · _Introduced in:_ [93e9b66](https://github.com/leptoskit/leptoskit/commit/93e9b66a97a7efd60824484e10e40488794b2a0b)
- **Add `leptoskit_plugin_asset` for packaged-asset capability status and name→URI resolve.** Hosts return Unavailable in this MVP until packaged-asset ownership wiring lands. _Packages:_ 🟠 _leptoskit_plugin_asset_ _Owner:_ Ifiok Jr. · _Introduced in:_ [97a7345](https://github.com/leptoskit/leptoskit/commit/97a734597a1813b29cf7a845d8fd4fa138a9b069)
- **Add `leptoskit_plugin_diagnostics` for start/end span and event recording.** Hosts return Unavailable in this MVP until a diagnostics backend is chosen. _Packages:_ 🟠 _leptoskit_plugin_diagnostics_ _Owner:_ Ifiok Jr. · _Introduced in:_ [9e84b3c](https://github.com/leptoskit/leptoskit/commit/9e84b3cd3294a5bfdb0110230b9bd74b269754c2)
- **Add `leptoskit_plugin_blob` for bounded create/read/release of owned byte blobs.** Hosts return Unavailable in this MVP until a durable resource protocol lands. _Packages:_ 🟠 _leptoskit_plugin_blob_ _Owner:_ Ifiok Jr. · _Introduced in:_ [9e84b3c](https://github.com/leptoskit/leptoskit/commit/9e84b3cd3294a5bfdb0110230b9bd74b269754c2)
- **Native capability parity: badges and shortcut hosts.** Browser pages can set and clear app-icon badges through the Badging API (`web::set` / `web::clear`), and `web::get` returns the last count this adapter published. Generated iOS, Android, and macOS hosts now call `noteLaunch` when the OS activates an app shortcut: Android dynamic shortcuts carry a unique intent per id, the macOS host builds the dock menu from `DockShortcutStore`, and iOS notes home-screen quick actions. Docs match the real backends for menus, badges, and shortcuts. _Packages:_ 🟠 _leptoskit_plugin_app_shortcuts_, 🟠 _leptoskit_plugin_badge_, 🟢 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [0c66c58](https://github.com/leptoskit/leptoskit/commit/0c66c58142338c3399198783a442bc777f04476c)

#### App usage plugin

_Packages:_ 🟠 _leptoskit_plugin_app_usage_

_Owner:_ Ifiok Jr. · _Introduced in:_ [b3d35c1](https://github.com/leptoskit/leptoskit/commit/b3d35c13b10151c8231fea893f5d3d6a941ef516)

Add the `app-usage` plugin, which reads how the device has been used. Select `app-usage` in `native.plugins` and call it from a guest through `guest::AppUsage`:

- `usage_access_status()` reports whether the person has granted Usage Access, without prompting.
- `open_usage_access_settings()` opens Android's Usage Access screen so they can grant it.
- `events(range, max)` returns typed events (an app resuming or pausing, the screen turning on or off, the lock screen showing or being dismissed, startup and shutdown), oldest first and capped at `max`.
- `sessions(range)` returns per-app foreground sessions clipped to the range.
- `summary(range)` returns screen-on time, the unlock count, and per-app foreground totals.

Android reads `UsageStatsManager` and declares `PACKAGE_USAGE_STATS`. iOS and macOS give apps no per-app usage, so sessions and summaries return `Unavailable` there, and `events` waits for the lock and unlock transitions seen while the app runs. Browsers, Windows, and Linux return `Unavailable`.

#### Sound playback

_Packages:_ 🟠 _leptoskit_plugin_audio_

_Owner:_ Ifiok Jr. · _Introduced in:_ [67ea0f1](https://github.com/leptoskit/leptoskit/commit/67ea0f111c73a3d36f2472c94976a24cf52b7e02)

`leptoskit_plugin_audio` is a new runtime plugin, `audio`, for music and sound effects. Load a sound from the app's assets, its data or cache folder, or bytes, and play it as many voices as you need: looping without a gap, fading in, ramping volume, panning, through two low-pass or high-pass filters whose cutoff glides, paused and resumed where it stopped, and stopped with a fade. The master bus has the same volume, filters, and pause, so a soundtrack can be veiled, held while the app is in the background, or silenced at once, and `crossfade` brings one piece in as another falls away. `events()` reports voices that played to their end and audio-session interruptions.

iOS, Android, and macOS share one Rust engine: symphonia decodes Ogg Vorbis, WAV, MP3, FLAC, and AAC; music streams a second ahead instead of sitting in memory decoded; a lock-free mixer runs in the device callback; and cpal plays through Core Audio and AAudio. `configure` picks an ambient or playback session: the `AVAudioSession` category on iOS, and audio focus on Android. Browser apps use `web::Audio` over Web Audio, which starts on the page's first tap or key press. HarmonyOS has no output yet, Android does not report losing audio focus, and native builds don't copy `assets/` into the app yet.

- **Add `leptoskit_plugin_background_task` for named OS background-task register/unregister/status/complete (`BGTaskScheduler` / `WorkManager`), distinct from `leptoskit_plugin_background` AppService keep-alive.** Guaranteed cadence and suspended Wasm execution remain later scope. _Packages:_ 🟠 _leptoskit_plugin_background_task_, 🟢 _leptoskit_ _Owner:_ Ifiok Jr. · _Introduced in:_ [d5f4657](https://github.com/leptoskit/leptoskit/commit/d5f4657d76a38bde21d61af21eff755964a6ac13)
- **Add `leptoskit_plugin_calendar` for calendar permission, listing calendars (max 64), and creating or reading bounded events on iOS and Android.** Desktop and browser hosts report unavailable. _Packages:_ 🟠 _leptoskit_plugin_calendar_, 🟢 _leptoskit_ _Owner:_ Ifiok Jr. · _Introduced in:_ [7b07d8f](https://github.com/leptoskit/leptoskit/commit/7b07d8f5999a34ede92c671fbe1c524dd49b7313)
- **Library image and video picker.** `leptoskit_plugin_image_picker` picks one photo-library image or video with cancellation, an exclusive busy slot, and a bounded temporary URI plus width/height/mime/size metadata. Camera capture stays for a later `camera` plugin. Apple uses PHPicker; Android uses the activity-result broker; the browser returns a `blob:` URI from a trusted file input. _Packages:_ 🟠 _leptoskit_plugin_image_picker_ _Owner:_ Ifiok Jr. · _Introduced in:_ [55eb268](https://github.com/leptoskit/leptoskit/commit/55eb268983de0dbd62740a84d2f0e8202a475bb7)
- **Mobile picture-in-picture and system overlays.** Apps can keep a call or video-like surface visible while another app is open. The new `picture-in-picture` runtime plugin reports capabilities and the active session, enters Android activity picture-in-picture as a cancellable lease, and — after the user grants `SYSTEM_ALERT_WINDOW` in system settings — holds an Android system overlay window. Overlay status and `request_overlay` open the grant screen without inventing a permission. iOS reports whether system PiP is supported but does not yet enter it without an attached video player, and has no draw-over-apps overlay. Desktop and the browser return `Unavailable`. _Packages:_ 🟠 _leptoskit_plugin_picture_in_picture_ _Owner:_ Ifiok Jr. · _Introduced in:_ [859c291](https://github.com/leptoskit/leptoskit/commit/859c291037b19bb85ea1a191c385f0250a2734c8)

#### Add menu roles, iOS menu bar, and signed updater feeds

_Packages:_ 🟠 _leptoskit_plugin_menu_, 🟠 _leptoskit_plugin_updater_

_Owner:_ Ifiok Jr. · _Introduced in:_ [b53298d](https://github.com/leptoskit/leptoskit/commit/b53298d6db24da853e8c2a3d53fa2f6df303b489)

- `leptoskit_plugin_menu` adds standard OS `MenuRole` predefined items (`Role(MenuRole::Copy)`, `About`, `Quit`, `Undo`, `Redo`, …), mutually-exclusive `Radio` items, `ContextMenu()` / `ContextMenuHost` / `MemoryContextMenuHost`, and `leptoskit_plugin_menu::ios` for iOS & iPadOS `UIMenuBuilder` menu bars (`IosMenuBar`, `IosMenuSection`, `IosMenuPlacement`) and `UIKeyCommand` hardware shortcuts.
- `leptoskit_plugin_updater::feed` adds signed release-manifest desktop auto-updating (`ReleaseManifest`, `PlatformArtifact`, `TargetPlatformId`, `UpdateChannel`, `UpdatePolicy`, `SemVer`, `IntegrityVerifier` with built-in FIPS 180-4 SHA-256 verification, `FeedTransport`, and `FeedBackend`).

- **Builder-friendly native menus.** `leptoskit_plugin_menu` adds chained constructors (`Menu`, `MenuBar`, `Submenu`, `Command`, `Check`, `Separator`) that build the same validated definitions hosts already attach. On macOS, `ApplicationMenu::install` builds and leases the AppKit menu bar in one step; Windows gains the matching `WindowMenu::install` helper. Linux still has no standalone menu bar and exposes `linux::attach_menu_bar` as an explicit `Unavailable` stub; tray / status-item menus keep sharing `DesktopMenu` with `leptoskit_plugin_tray`. Docs cover the macOS path and the Windows/Linux parity gaps. _Packages:_ 🟠 _leptoskit_plugin_menu_ _Owner:_ Ifiok Jr. · _Introduced in:_ [1dfe6e6](https://github.com/leptoskit/leptoskit/commit/1dfe6e6937701ff1a5c78a52e356ed03b39e6ab8)
- **Add `leptoskit_plugin_video_thumbnails` to extract one JPEG frame from an app-owned video URI at a time offset (`AVAssetImageGenerator` / `MediaMetadataRetriever`).** Multi-frame extraction remains later scope. _Packages:_ 🟠 _leptoskit_plugin_video_thumbnails_, 🟢 _leptoskit_ _Owner:_ Ifiok Jr. · _Introduced in:_ [88c385c](https://github.com/leptoskit/leptoskit/commit/88c385c35553dfab40a5e0270b848bd2805f2367)

#### Share files, subject and title, and where it went

_Packages:_ 🟠 _leptoskit_plugin_sharing_, 🟠 _leptoskit_cli_

_Owner:_ Ifiok Jr. · _Introduced in:_ [832bc2b](https://github.com/leptoskit/leptoskit/commit/832bc2b072feb4541efe34a14c6f53f8a220d6a7) · _Last updated in:_ [d0a8882](https://github.com/leptoskit/leptoskit/commit/d0a8882eeb43751a00f3e89cc07fb7b4e0b97413)

`leptoskit_plugin_sharing` now covers Flutter `share_plus`'s features. A share can carry up to 32 files, optionally with text: `File::app("exports/report.pdf")` names a file in your app's files directory (the root the `filesystem` plugin writes), and `File::bytes("chart.png", bytes)` shares data from memory, each with an optional MIME type and display name. `with_subject` sets the email subject, `with_title` the sheet or chooser title, `with_anchor` where the iPad popover or macOS picker points, and `excluding` leaves iOS activities out of the sheet.

`share` now returns a `ShareResult`: `Success` with the destination the user picked (the iOS activity type, the macOS service title, or the Android component), `Dismissed`, or `Unavailable` when the platform cannot tell, as in browsers. A result never confirms delivery. On Android this replaces the old behaviour, where dismissing the chooser and choosing an app looked the same.

Native hosts check that an app file stays inside the files directory, refusing symlinks that lead out of it, and copy shared files into a private cache that the next file share clears. On Android the plugin ships its own `FileProvider` and grants read access only to the chosen app, only for those copies; it adds `androidx.core:core`. Browsers share in-memory files through Web Share Level 2 and return `Unavailable`, without a download fallback, when they cannot. Windows and Linux still have no native share sheet.

Breaking: the wire protocol is versioned and changed. The previous request (a kind and one value) and its empty reply are refused, `share` returns `ShareResult` instead of `()`, `web::share` resolves to a `ShareResult` (a browser cancellation is `Dismissed`, no longer a `Cancelled` error, and an iOS or macOS dismissal is `Dismissed` rather than a `Cancelled` error), and `Content::as_str` and `Content::is_url` are replaced by `shared_text`, `shared_url`, `shared_files`, `subject` and `title`.

The CLI reads a new `android-file-providers` plugin metadata key: it copies the plugin's paths XML into the generated resources and declares an unexported `FileProvider` in the generated Android manifest.

- **Add `leptoskit_plugin_media_library` for bounded photo-library permission, asset listing, and saving an app-owned file into the system library.** Change observation remains later scope. _Packages:_ 🟠 _leptoskit_plugin_media_library_, 🟢 _leptoskit_ _Owner:_ Ifiok Jr. · _Introduced in:_ [5e5b6cd](https://github.com/leptoskit/leptoskit/commit/5e5b6cd704fae36b0331982beb725e9a61d1a1e5)
- **Android FCM registration on the plugin side.** `REMOTE_TOKEN` and `REMOTE_REGISTER` work on Android hosts, not iOS alone: a process-wide broker — the same shape as the Swift one — keeps the latest answer, fed by the registration task's own reply and by the messaging service's `onNewToken`, and the Android `REMOTE_REGISTER` asks `FirebaseMessaging` for its token reflectively (the generated app carries `firebase-messaging` only when push is opted into), coalesces OS requests at one, answers `Unconfirmed` on silence, never succeeds after close, and keeps the token out of every log and description. The stable failure codes gain `firebase-not-configured` — the Firebase classes are absent, or the app has no `google-services.json` values — beside `os-refused` and `token-shape`; tokens validate as 1–4096 bytes of printable ASCII and the wire gains state 4 for them. The Android instrumentation suite covers the bounded wait, unconfirmed silence, token-shape refusal, close-while-waiting, and the one-ask convergence with an injected registration boundary. _Packages:_ 🟠 _leptoskit_plugin_notifications_ _Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)

#### Development and store builds share one service

_Packages:_ 🟠 _leptoskit_plugin_notifications_, 🟠 _leptoskit_notifications_

_Owner:_ Ifiok Jr. · _Introduced in:_ [8980de9](https://github.com/leptoskit/leptoskit/commit/8980de9af6db3ed62cd57c2e36e8c461d5a0cea5)

Apple runs two push servers, and a device token is known to one of them. Until now the service used one for every device, so a build run from Xcode and a TestFlight build could not enroll with the same service. Now each iOS and macOS device says which server its build is signed for, and the service pushes it through that one.

- The iOS and macOS hosts answer a new plugin method, `APS_ENVIRONMENT` (method 11): the simulator answers development, a build with an embedded provisioning profile answers what the profile's `aps-environment` entitlement says, and an iOS build without a profile, which came through TestFlight or the App Store, answers production. Android does not answer it.
- `native::enroll` and the launch `reconcile` send the answer with the device's other details, as `DeviceDetails::apns_environment`. Your app writes no code for it, and a change reaches the service at the next launch.
- The service stores it per endpoint and picks Apple's server from it. `LEPTOSKIT_NOTIFICATIONS_APNS_ENVIRONMENT` is now the default, for a device that reported nothing: an older app version, a Mac build without a profile, or an imported token.
- `import_push_endpoints` and a direct `register_push_endpoint` take the same field, so imported APNs tokens can name their environment.
- `person_endpoints` and `push_endpoints` show it under `device`. Conditions do not read it, and it is dropped for an FCM or Web Push endpoint.

The `.p8` key, the team id, and the topic are still one of each, so both builds need the same bundle id and a key that may use both servers.

This release has one migration per engine, `endpoint_apns_environment`, which adds a nullable column to `push_endpoints`. Run `leptoskit db --service notifications migrate` before you start the new version.

Breaking for code that builds these by hand: `DeviceDetails` has a new public field, so a struct literal needs `..DeviceDetails::default()`. `ApnsEnvironment` now lives in `push` and is re-exported from `service::config`, where it was. A host that predates method 11 answers "unavailable" for it, and the app enrolls as before.

The Swift code that reads the provisioning profile was not compiled or run on a device for this release.

#### Device details on push endpoints, and targeting by device

_Packages:_ 🟠 _leptoskit_plugin_notifications_, 🟠 _leptoskit_notifications_

_Owner:_ Ifiok Jr. · _Introduced in:_ [d0ea555](https://github.com/leptoskit/leptoskit/commit/d0ea555a8701f661e4bc72a6d697ad6dc59f062a)

Every push enrollment now carries what the device says about itself — its preferred locale, its time zone, and the app's version — and a send's condition can target devices with them.

- **Device details.** `register_push_endpoint` takes an optional `device` (`DeviceDetails { locale, time_zone, app_version }`), defaulting to none, so existing callers keep working. The service stores the locale as a canonical BCP 47 tag (`en_gb` becomes `en-GB`, at most 35 bytes), the time zone as an IANA-shaped name (at most 64 bytes, checked for shape only), and the app version as free text (at most 64 bytes), refusing a bad detail by name. Re-enrolling replaces them; a change to the details alone never rotates the endpoint's credential or disturbs its queued deliveries. Only the endpoint's owner reads them back, as `EndpointSummary::device`. A new `device_details` migration (SQLite and Postgres) adds the three nullable columns.
- **No app code.** The web client sends `navigator.language` and the `Intl` time zone, and the app's version when `EnrollOptions::app_version` names one. The `native` module reads the locale, time zone, and app version through the notifications plugin's new `DEVICE` method on iOS, macOS (Foundation's preferred language, `TimeZone.current`, `CFBundleShortVersionString`), and Android (the default locale and time zone, the package's version name). `native::reconcile` now sends only when the token or the details changed since the enrollment last sent them — `Enrollment` remembers a digest of what it sent in its new `sent` field — so an app update or a language change reaches the service at the next launch and an unchanged launch makes no service call.
- **Device predicates.** Conditions gain `device.platform` (`'apns'`, `'fcm'`, or `'web_push'`), `device.locale`, `device.language` (the locale's first subtag, so `'fr'` matches `fr-CA`), `device.app_version`, and `device.time_zone`, with the operators `user.` predicates have. `device.app_version` compares as a version: everything from the first `-` or `+` is ignored, the rest must be dot-separated numbers compared left to right with missing parts as 0 (`2.10 > 2.9`, `2.1 == 2.1.0`); an ordering is false for a value that isn't a version. Any other key, or a platform that isn't one, is refused at send time and under `validate_only`, naming what there is.
- **Judged per device.** A person is in the audience when the condition holds with no device — where a device predicate answers false, as an absent attribute does — or on at least one of their live endpoints. They get one inbox row, and the push goes only to the endpoints the condition holds for, decided when the fanout writes the batch's deliveries. "Everyone on Android" is `device.platform == 'fcm'`, and it walks the same registry `SendTarget::Everyone` does.

The plugin's `DEVICE` method (9) answers three length-prefixed strings, empty for unknown, as the new `DeviceInfo`; `guest::Notifications::device_info` reads it, and a host that predates it answers `Unavailable`. The Android host's remote-registration snapshot is now encoded little-endian, as the bridge codec reads it.

- **iOS APNs registration, second patch: the answers are now honest about silence and tight about what crosses.** `REMOTE_REGISTER` bounds OS requests at one — later callers join the wait instead of asking again — and answers `Unconfirmed` (state 3) when its wait bound passes with no new answer, so a prior token is never handed back as though it were fresh; the broker keeps the last answer readable through `REMOTE_TOKEN`, and a plugin closed while waiting never answers a token. Registration refusals cross as the contract's stable codes — `os-refused`, `aps-environment` (matched on domain and code), `token-shape` — never the OS's own error text, which can carry device detail and previously clipped to invalid UTF-8. The token is an even count of hex characters (whole bytes, whatever length the OS chose), the revision is a consistent full `u64` with `0` meaning no answer on both constructor and decoder, the Swift broker's counter saturates instead of trapping, and no `Debug` or description of token, state, or snapshot prints its bytes. The browser implementation moved behind a default `browser` feature (as the HTTP plugin's did), so interpreter guests turn it off and their dependency tree stays free of browser imports. _Packages:_ 🟠 _leptoskit_plugin_notifications_ _Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)
- **macOS joins iOS on the remote-registration wire: the Swift plugin's `REMOTE_TOKEN` and `REMOTE_REGISTER` are advertised on macOS hosts too, where registration asks `NSApplication.registerForRemoteNotifications` instead of `UIApplication`'s. Everything else is the same contract the iOS slice shipped — the process-wide broker the app delegate feeds, one OS request at a time with later callers joining the wait, `Unconfirmed` on silence, the stable refusal codes, lower-case hex tokens of no assumed length, and no token in any log, `Debug`, or description.** No new wire states. _Packages:_ 🟠 _leptoskit_plugin_notifications_ _Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)

#### Taps, foreground pushes, and data messages reach app code

_Packages:_ 🟠 _leptoskit_plugin_notifications_, 🟠 _leptoskit_notifications_, 🟠 _leptoskit_cli_

_Owner:_ Ifiok Jr. · _Introduced in:_ [b1d0c8b](https://github.com/leptoskit/leptoskit/commit/b1d0c8b741c4bd284362b53d998bd3c0b7b707cc)

A push used to end at the banner: nothing told the app which notification was tapped, a push that arrived while an iOS or macOS app was open showed nothing, an Android app in the foreground dropped it, and a silent data message ran no app code. One receiver now answers all of it on iOS, macOS, Android, and the web.

`leptoskit_notifications` gains the `receive` module, built with the `native` or `browser` feature. `Received` is `Opened` (a tap: the notification's id, the send's `link`, the pressed action's id, the send's `data`, a local notification's id, and `launched` for the tap that started the app), `Shown` (a notification arrived while the app was open), or `Data` (a silent data message with its `data` and `badge`). `Opened::route(inbox_route)` answers where the tap should go. `native::Receiver` and `browser::Receiver` both have `new`, `next`, and `pending`, and answer the launching tap first. Where nothing can be received they answer `ReceiveError::Unavailable`.

`leptoskit_plugin_notifications` gains method `RECEIVED` (10) on iOS, macOS, and Android, with `ReceivedItem`, `ReceivedKind`, `encode_received_wait`, `decode_received_wait`, and `guest::Notifications::received`. The host keeps the sixteen most recent items and hands each over once. On iOS and macOS a notification-centre delegate shows a notification while the app is open and records taps; on Android the plugin posts a push that Firebase hands to a foreground app, reads a tap from the activity's intent, and a local `show()` now opens the app when tapped.

`leptoskit_cli` generates the host side: new iOS and macOS apps install the delegate before launch finishes and forward `didReceiveRemoteNotification`, an iOS app with `aps-environment` set declares the `remote-notification` background mode, and the generated Android messaging service forwards `onMessageReceived`.

**What changes for an existing app:**

- `hosts/ios/App.swift` and `hosts/macos/App.swift` are written once and are the app's own, so an app generated earlier must add the delegate lines the guide lists. Without them it behaves as before. Android apps need no change.
- A push that arrives while an iOS, macOS, or Android app is open now shows a banner. An app that drew its own on those platforms would show two.
- The web receiver adds the notification's id as `lkn` to the address it opens for a send's own `link`, as it already did for the inbox route.
- The web receiver first offers a click to an open page and waits up to half a second for the page's answer before loading the address. Only a page that creates a `browser::Receiver` answers; other pages see a click arrive up to half a second later than before.
- A web push flagged as a data message, which the service never sends, now shows the generic banner when no page is in view instead of showing nothing.

The Swift and Kotlin code in this change was not compiled where it was written; its tests run in the Apple and Android CI jobs.

#### Location: continuous background updates

_Packages:_ 🟠 _leptoskit_plugin_location_

_Owner:_ Ifiok Jr. · _Introduced in:_ [a51c332](https://github.com/leptoskit/leptoskit/commit/a51c332f201e582ab58bcd8fb966cef5405442b9)

`leptoskit_plugin_location` grows from foreground reads into a continuous background stream, the shape a life-logging app needs for a constant record of the owner's location.

`request_background_permission` upgrades the foreground grant exactly as each platform requires: iOS asks for "Always" on top of "When In Use", and Android 10+ requests `ACCESS_BACKGROUND_LOCATION` as its own separate step after the fine-location grant; earlier Android versions grant background access implicitly and the call only reports it. The reply is the background scope on its own, so a denied background with a granted foreground reads as denied without wiping the foreground's accuracy report.

`start_updates(UpdateOptions)` starts continuous updates: an accuracy request (best, navigation or balanced), a distance filter in metres (0–10,000), an interval hint in milliseconds (1–3,600,000), a background flag, and iOS's automatic-pause and background-indicator settings. `stop_updates` stops every source without discarding buffered fixes. `next_updates` is a cancellable wait that returns one batch: the fixes buffered since the last call, oldest first, with a `dropped` count for the fixes the bounded native buffer (64) discarded when it was full. An empty batch means no source is running and nothing is buffered: the stream has ended.

iOS also gains `start_visits`, which buffers each visit as a fix at its coordinate timestamped at arrival and, once the visit ends, at departure, and `start_significant_changes`, the low-power source that relaunches a killed app. Both are `Unavailable` on Android, macOS and the browser. After a relaunch the app must start its sources again at startup; iOS then delivers the event that woke it to the new listener.

Android background updates require the `background` plugin's location-typed foreground service (bit 3 of its service-type mask); `start_updates(background: true)` fails with a clear error when it is not running. iOS background updates require the "Always" grant and the `location` background mode. The plugin's registry metadata now requires the app's `NSLocationWhenInUseUsageDescription` and `NSLocationAlwaysAndWhenInUseUsageDescription` purpose strings, adds `location` to `UIBackgroundModes`, and declares `ACCESS_BACKGROUND_LOCATION` and `FOREGROUND_SERVICE_LOCATION` in the Android manifest.

- **Configured RPC endpoints now work for clients alongside the server configuration.** The clients previously called the compile-time defaults: the app's own client posted every call to `/rpc/<op>`, the browser's server-sent events fallback opened `/live/events` and posted frames to `/live/frames`, and the exported spec described every operation at `/rpc/<op>` however the app was configured. The spec export now carries both mounts (`rpc_path` and `live_path`, spec version 4), composes every rpc operation's route from the configured prefix — a service's spec under the service's own prefix — and the foreign and generated SDK clients call those routes and open that socket, so `leptoskit rpc generate` output follows the configuration. The app's own client follows it too: `leptoskit dev` and `leptoskit build` compile the browser and native clients with `LEPTOSKIT_RPC_PATH` and `LEPTOSKIT_RPC_LIVE_PATH` from `leptoskit.toml`, and `ClientConfig::rpc_path`/`live_path` name the mounts by hand for builds that don't go through the CLI. A service's operations keep the default mounts under the service's prefix: `[rpc].path` moves the app's own. Defaults are unchanged (`/rpc`, `/live`, legacy `/api`), as are scope lookup, auth headers, service credentials, and service origin isolation. _Packages:_ 🟠 _leptoskit_rpc_codegen_, 🟠 _leptoskit_rpc_, 🟢 _leptoskit_rpc_macros_, 🟢 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [8bd1891](https://github.com/leptoskit/leptoskit/commit/8bd1891b8d987c9abe19128343e7107d706ecd88)

#### Let an operation refuse with a code and a wait

_Packages:_ 🟠 _leptoskit_rpc_codegen_, 🟠 _leptoskit_rpc_

_Owner:_ Ifiok Jr. · _Introduced in:_ [bd68e95](https://github.com/leptoskit/leptoskit/commit/bd68e9525308319c899d7e4125b570cf2b0d905f)

A query, mutation, or stream can now say why it refused, in a form a screen can match on without comparing sentences. `rpc::Error::rate_limited(wait)` answers `429` with a `Retry-After` header, and `rpc::Error::refused("notes_full", "Your notebook is full")` answers `409` with a code the app chose. Build `rpc::Error::Refused { code, message, status }` for another status.

The caller gets the new `FnError::Refused { status, code, message, retry_after }`, with `FnError::code()` and `FnError::retry_after()` as shortcuts. It arrives the same way over a plain call, the live socket, and a stream, on every platform. A refusal is never retried.

The generated Rust, TypeScript, Dart, Go, and Python clients read it too: each gains a `refused` kind with the code and the wait, and each `LiveError` carries them over the live connection.

Things to know when upgrading:

- `leptoskit_rpc::Error` and `FnError` each gained variants, and `leptoskit_rpc::live::WireError` gained the optional fields `code` and `retry_after`. Code that matches on them without a wildcard arm, or builds a `WireError` by hand, needs updating.
- The generated Rust client's `Error` gained a `Refused` variant, and its `LiveError` two fields.
- A client built or generated before this release reads a refusal as an `other` error whose text is the raw `Refused|{…}` body. Release clients before the server starts refusing this way.

#### Preserve null, empty collections and omitted arguments in generated SDK requests.

_Packages:_ 🟠 _leptoskit_rpc_codegen_, 🟢 _leptoskit_rpc_, 🟢 _leptoskit_cli_

_Owner:_ Ifiok Jr. · _Introduced in:_ [51224bf](https://github.com/leptoskit/leptoskit/commit/51224bf974470ffc7ad6154c57e515f4f83556d3)

Query and form encoders now match the server's serde_qs 1.1 protocol: explicit null is a bare key and an empty collection is `key=`. JSON and live encoders keep explicit null, and required nullable arguments in Go and Rust keep their keys. Python optional arguments use the exported `UNSET` for omission and `None` for explicit null. Generated Rust and CLI packages need `serde_qs` 1.1.3; update an existing package's manifest when regenerating its code.

Clients, OpenAPI, AsyncAPI and compatibility checks now follow each argument's `required` flag rather than inferring it from nullability. Specs from `#[api]` functions mark `Option` arguments optional, because the server reads a missing key as `None`. The spec format is now version 4; older specs still load, and their nullable arguments stay optional. A spec written by this version needs an equally new CLI to generate clients.

In regenerated Dart clients, `Option` arguments become optional `Presence<T?>` parameters instead of required `T?` ones. Omit them, or pass `Presence.present(value)`.

Strict diagnostics report a Go or Rust optional nullable argument only when its schema declares a default, and no longer report a Dart query/form limitation that the encoder now handles. Mandatory SDK acceptance decodes generated HTTP requests with the server codec.

#### Keep a crate's operations behind its own server feature

_Packages:_ 🟠 _leptoskit_rpc_macros_

_Owner:_ Ifiok Jr. · _Introduced in:_ [784ba50](https://github.com/leptoskit/leptoskit/commit/784ba50c9e9009f79cbbe2b99e6d59c55c836ec8)

An API crate that other crates link for its client, such as a CLI calling a hosted service, can now keep its operations' bodies and registrations behind one of its own features:

```toml
[package.metadata.leptoskit.rpc]
server-feature = "server"

[features]
server = ["leptoskit_rpc/server"]
```

Built without that feature, the crate's `#[rpc::query]`, `#[rpc::mutation]`, job, and stream operations keep their argument structs and an `Operation` impl that fails with a message naming the feature. Nothing registers them, so a server in a build that links the crate for its client neither compiles their bodies nor serves them, even when `leptoskit_rpc/server` is on for another crate's sake. `#[api]` describes them in the spec only where they are served. A `server-feature` that names a feature the crate doesn't declare is a compile error. Cargo doesn't rebuild a crate when only its metadata changes, so run `cargo clean -p <crate>` after adding or removing the key.

`#[api(scope = …)]` on an operation now reaches its description. Before, operations were registered with no scopes, so `leptoskit_idp` refused every third-party access token on them.

#### Rate limits, timeouts, and guards for public operations

_Packages:_ 🟠 _leptoskit_rpc_macros_, 🟠 _leptoskit_rpc_, 🟠 _leptoskit_cli_

_Owner:_ Ifiok Jr. · _Introduced in:_ [4c41c6e](https://github.com/leptoskit/leptoskit/commit/4c41c6e61ebf8a586cfdd4e1e6183a9beafbffdc)

Every query, mutation, action, and stream a client can call can now be limited before its body runs, over HTTP and over a live connection alike.

```rust
use leptoskit::rpc::server::{Rate, RateLimit, Server};

let rpc = Server::new(db)
	.identify(auth)
	.client_ip_header("fly-client-ip")
	.rate_limit(RateLimit::new().anonymous(Rate::per_minute(60)))
	.guard(SignedInOnly);
```

- `Server::rate_limit(RateLimit)` counts each caller: by address when nobody is signed in (IPv6 by its /64), and by `Identity::caller` when someone is, with `RateLimit::key` to count every session of one person together. The defaults are 120 calls a minute per address and 600 per signed-in caller, refilled evenly (GCRA). Services and the app calling them aren't counted unless `RateLimit::internal` says so. A refused call is `429` with `Retry-After`, which clients already read as `FnError::Refused` with the code `rate_limited`.
- Counts live in a bounded `MemoryStore` by default. Implement `RateStore` to share them between servers, for example over Redis.
- `#[rpc::query(rate_limit = 10 / 1m, timeout = 5s, max_body = 64KiB)]`, and the same on `#[rpc::mutation]` and `#[rpc::action]`, set one operation's limits. An operation's own `rate_limit` holds even when the server sets no `RateLimit`. They are on `Registration::limits` as `OperationLimits`.
- A call past its timeout is answered `504` with the code `timed_out` (`rpc::server::TIMED_OUT`); arguments over the size limit are answered `413` with `too_large` (`rpc::server::TOO_LARGE`) instead of a `400`. `Server::timeout` and `Server::max_body` set the defaults; the body limit is still 2 MiB unless changed.
- `Server::guard(impl Guard)` runs an app's own check before every call. A `Guard` sees a `Call`: the operation's name and kind, the identity, the client's address, and whether it came over HTTP or a live connection. `Live` and `LiveStreams` take the same `guard`, `rate_limit`, and `client_ip_header`, and `Server::live()` passes its server's on.
- The HTTP routes now identify the caller and run the guards before reading the request body, so a refused caller costs no more than their headers. Calls the server makes in process while it renders a page are not checked again.

Rate limiting is opt-in for existing apps. New database and sign-in projects from `leptoskit new` start with `Server::new(db).rate_limit(RateLimit::new())` in the backend's `operations`.

`Registration` has a new `limits` field: code that builds one by hand adds `limits: OperationLimits::NONE`.

#### Serve streams from a server that has no database

_Packages:_ 🟠 _leptoskit_rpc_macros_, 🟠 _leptoskit_rpc_, 🟠 _leptoskit_

_Owner:_ Ifiok Jr. · _Introduced in:_ [f7bf762](https://github.com/leptoskit/leptoskit/commit/f7bf762393787df34f5c07c85f9741e6cf6be5e9)

A `#[rpc::stream]` used to need the full live server, which reads a database's change log, so a service with something to stream and no database could not serve one.

- **`rpc::StreamCx`** is a new context for a stream's body. It holds who is calling (`get`, `require`, `signed_in`, `identity`) and no database. A stream that takes it runs on any live server. A stream that reads the database keeps `rpc::QueryCx`.
- **`rpc::server::LiveStreams`** serves streams alone, on the same routes as `Live` (the WebSocket, its server-sent events, and their frame posts) with the same limits. It takes `identify`, `origin`, `for_service`, `rpc_config`, `stall_timeout`, and `drain`. `Live::streams_only()` returns one too. A query, mutation, action, or channel sent to it is answered with a `501` that says why.
- **`live-streams`** is a new feature of `leptoskit_rpc` (`rpc-live-streams` through `leptoskit`). It needs no database driver and opens no connection. `live` now includes it.

Nothing changes for a client: `use_stream` reads either server.

Things to know when upgrading:

- A stream with no context argument no longer requires a database driver in the build. It needs `live-streams` or a driver, and `rpc::cx()` inside it works only where the server has a database.
- `StreamOperation::pump` and `StreamRegistration::open` now take the hidden `rpc::server::StreamOpen` in place of a `QueryCx`. Code generated by `#[rpc::stream]` follows by itself; a registration built by hand needs the new type.
- `LiveStreams` keeps an identity until its lease expires or the connection ends. It can't watch the rows a lease reads, because it has no change log.

#### Limit request rates and estimate a `[cloud]` table

_Packages:_ 🟠 _leptoskit_rpc_, 🟠 _leptoskit_cloud_

_Owner:_ Ifiok Jr. · _Introduced in:_ [373ad29](https://github.com/leptoskit/leptoskit/commit/373ad295394a26e69e883a6b28983d35b7e3f02d)

The LeptosKit Cloud control plane now counts every operation against who asked: a person, an API token and its organisation's tokens together, a worker, or the address of a request that is nobody's. A caller past their rate is answered `429` (`rate_limited`) until the window ends. `ControlConfig` gains `rate_limits` and `client_ip_header`, so code that builds one by hand must set them; the server reads the header's name from `LEPTOSKIT_CLOUD_CLIENT_IP_HEADER`. An organisation's checkout and portal pages have a rate of their own.

`leptoskit_cloud::billing` adds `estimate`: what a month of a whole `[cloud]` table is expected to cost on a plan, before anything is deployed.

A backup's dump has a size limit, `ShippingSettings::backup_cap`. Workers are told it in `BackupDatabaseInput::max_bytes` and `RestoreDatabaseInput::max_bytes`, and a dump a worker reports as larger is a backup that failed.

`leptoskit_rpc::Error` gains `TooMany`, answered as `429`, for an operation or an `Identify` that refuses a caller who asks too often. `Identify` gains `admit`, asked before each subscription, mutation, action and stream a live connection sends, so a limit on how often a caller asks covers a connection that is identified once. It admits everything unless implemented.

A `[cloud]` table with more than 50 environments, 50 services or 50 hostnames in an environment is now refused as `invalid_config` before it is resolved.

- **`AppCaller`: call a service as the app from another program.** `AppCaller::new(service, url, token)` calls one service's operations as the app, the same way `call_as_app` does, but it uses the address and token you hand it instead of reading them from the environment. It's for a separate program that acts for the app, such as its admin. `.call(op)` refuses an operation that belongs to a different service. Keep it on the server, like the token it holds. _Packages:_ 🟠 _leptoskit_rpc_ _Owner:_ Ifiok Jr. · _Introduced in:_ [60e11ae](https://github.com/leptoskit/leptoskit/commit/60e11ae75fdffff7d6da8bc30d4369aced2e9d41)

#### Server-rendered queries hydrate without a mismatch

_Packages:_ 🟠 _leptoskit_rpc_, 🟠 _leptoskit_query_, 🟢 _leptoskit_cli_

_Owner:_ Ifiok Jr. · _Introduced in:_ [c451911](https://github.com/leptoskit/leptoskit/commit/c451911f920e3f81df7154ede1e9cc0bff52ff10)

A page that rendered a query on the server now hydrates as the same page in the browser. Before, only a query's data was carried, so a query that failed on the server started from "loading" in the browser, leptos reported a hydration mismatch, and the page stopped updating.

- `QueryScope::with_ssr_errors` (and `with_error_codec`) streams a server render's error alongside the data, so the browser shows the same error first. RPC queries and paginated queries opt in, and `FnError` is now `Serialize` and `Deserialize`.
- A query seeded from the server doesn't fetch while the page hydrates. Its mount refetch — stale data in the background, or a retry after an error — starts once hydration has finished, so the first render matches the server's HTML.
- On the server, reading a query that is still loading inside `<Suspense>` holds the boundary until it settles, as a leptos resource does, so the server renders the outcome it streams.
- Streamed payloads escape `<`, `>`, `&`, and U+2028/U+2029, so data and errors can't close the `<script>` they're embedded in.

Apps using `render = "ssr"` should read queries inside `<Suspense>`; outside a boundary the server renders whatever the query holds at that moment.

New apps put their web adapter inside that boundary, including the database and sign-in scaffolds. Existing portable apps should wrap the closure that calls `leptoskit::web::render` in the same way.

Generated browser entrypoints configure the RPC and auth clients inside the mount callback, after leptos has set up its executor and owner, so the live socket starts on the first page load. A query observer reads `enabled` without subscribing while it binds or runs its refetch interval, so an `enabled` closure that reads another query no longer logs a reactive-tracking warning on every page.

- **Jobs can carry process-local typed runtime state through `Jobs::state` and read it with `JobCx::state`.** This state stays separate from the durable job owner's authorization context and is never serialized. Notifications routes and workers now share an explicit push runtime. Queued push arguments contain only send and round IDs, keeping database URLs and credentials out of job metadata and allowing pending work to recover with a fresh runtime after restart. _Packages:_ 🟠 _leptoskit_rpc_, 🟢 _leptoskit_notifications_ _Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)

#### A fallible wait for hosts without a timer

_Packages:_ 🟠 _leptoskit_query_

_Owner:_ Ifiok Jr. · _Introduced in:_ [1bd0618](https://github.com/leptoskit/leptoskit/commit/1bd0618c284e4dff885524d3d6a23db9cfc668f8)

`leptoskit_query::try_sleep` reports `SleepError::Unavailable` or `SleepError::Rejected` instead of pretending a wait elapsed. Generated sign-in screens use it for a rate-limit countdown, so on a host with no usable timer the button becomes a manual retry, the server's "try again in …" message stays visible, and the screen never claims time that did not pass. The server remains the authority on when the window opens.

This is the compatibility surface for the native guest timer work. When that timer lands, its host-backed `try_sleep` implementation replaces the fallback here without changing the screen's behavior.

- **Generate llms.txt, llms-full.txt, and Markdown chapters.** `leptoskit_docs::Site::build` now emits `llms.txt` (structured chapter index), `llms-full.txt` (concatenated single-file reference), and cleaned `.md` chapter copies beside each HTML page. The site packager and Axum router expose `/llms.txt` and `/llms-full.txt` at the domain root as well as under `/docs/`, and `skills/leptoskit` now ships a `references/platforms.md` catalog for coding agents. _Packages:_ 🟠 _leptoskit_docs_ _Owner:_ Ifiok Jr. · _Introduced in:_ [42d6745](https://github.com/leptoskit/leptoskit/commit/42d6745bd392ec5ba225794fc9fbd8f7ca2c434f)
- **Your own tab bar and sheet surface.** `Tabs::bar` draws the tab bar with your own components instead of the built-in row of titles. It receives a `TabBar` with each tab and whether it is selected, and `TabBar::link` wraps whatever you draw in a link that switches tabs the same way the built-in bar does. `Stack::surface` draws the layer a modal or sheet route sits on, so it can use your app's paper and corners instead of plain white. Both keep the built-in test ids. `Tabs`, `Navigator`, and `IntoNavigator` now require `'static` route types, as `Router` already did. _Packages:_ 🟠 _leptoskit_router_ _Owner:_ Ifiok Jr. · _Introduced in:_ [f2e45e3](https://github.com/leptoskit/leptoskit/commit/f2e45e355aedd42747a69086a8925e37380fdb9c)
- **Set the browser tab's title.** `leptoskit_router::set_title("Plans · Admin")` names what the person is looking at. On the web it sets the document's title, so the tab and the history entry follow the screen as the person moves around without a page load. A server-rendered page still writes its first title in its own `<title>`. Native windows keep the title they were opened with for now, and on a phone it does nothing. _Packages:_ 🟠 _leptoskit_router_, 🟠 _leptoskit_ _Owner:_ Ifiok Jr. · _Introduced in:_ [edcb32f](https://github.com/leptoskit/leptoskit/commit/edcb32f5b4424abc33d17a784c8afb896ea06a14)

#### Control-plane scaling, HA sweeper, CA TLS, and billing

_Packages:_ 🟠 _leptoskit_db_, 🟠 _leptoskit_auth_, 🟠 _leptoskit_cloud_

_Owner:_ Ifiok Jr. · _Introduced in:_ [713fddf](https://github.com/leptoskit/leptoskit/commit/713fddf6095efdf17fbbbbd7e6ea87d9c2d586a6) · _Last updated in:_ [a53a454](https://github.com/leptoskit/leptoskit/commit/a53a4542f7ffd915ae725ede071581ebb9891300)

- `leptoskit_db` now supports custom Postgres CA PEM bundles via `sslrootcert=` or `PGSSLROOTCERT`, and adds session-scoped `Db::try_advisory_lock` for HA leader coordination.
- `update_environment` enforces plan scale limits (`max_size`, `max_replicas`, `require_sleep`, also exposed on `PlanLimits` and enforced on `rollback`) and queues an immediate live-scale cluster release when the caller holds `Permission::Deploy` (allowing scale-down even over spend cap, skipping no-op rollouts, honoring cluster facts, and preserving `schema_uncertain`). Forward deploys clear `scale_differs` once applied.
- Added `cluster_environments` worker RPC so restarted cluster workers immediately re-hydrate tracked environments for status, logs, and compute metering.
- `ShippingSettings` gains O(1)-tracked `organization_runtime_log_cap` (via `log_usage.runtime_bytes`), `run_sweeper` (`LEPTOSKIT_CLOUD_SWEEPER`), and Postgres advisory-lock sweeper leader election across HA replicas.
- Hardened `S3Store` (disabled redirects, bounded streaming hash concurrency, `400`/`403`/`501` checksum fallback, content-length check), `Stripe` (sanitized error telemetry and key length check), `subscriptions` (concurrent checkout customer conflict handling and overdue sweep rotation), and `bootstrap_operator` (concurrency-safe, checks disabled accounts, runs once per database and is not repeated after a revoke).
- `leptoskit_cloud_app` adds a `production = ["ssr", "postgres"]` feature flag.

- **Make `Keys::seal` and `Keys::open` public.** Another crate can now seal bytes under a set of rotating keys, bound to the context parts it names, the way sessions and codes already are. `Keys::shares_key_with` says whether two sets hold the same key under any name, so a server can refuse to use one key for two jobs. _Packages:_ 🟠 _leptoskit_auth_ _Owner:_ Ifiok Jr. · _Introduced in:_ [25cddbe](https://github.com/leptoskit/leptoskit/commit/25cddbed387b5b2483808d0171a5b063884ba739)

#### Ask a provider to sign in again

_Packages:_ 🟠 _leptoskit_auth_, 🟠 _leptoskit_

_Owner:_ Ifiok Jr. · _Introduced in:_ [f25ea8c](https://github.com/leptoskit/leptoskit/commit/f25ea8c94f5875355ce081afae5ac92a652cee48)

`StartProvider` has a `prompt` field. `Some(Prompt::Login)` asks the provider to sign the person in again even when someone is already signed in there, which is how a person switches to another account. `Some(Prompt::SelectAccount)` shows the provider's account picker. The value reaches the provider's authorization page as OpenID Connect's `prompt` parameter, and a provider that doesn't know the parameter ignores it.

```rust
sign_in.mutate(StartProvider {
	provider: "main".into(),
	redirect_to: Some("/admin/sign-in".into()),
	prompt: Some(Prompt::Login),
});
```

Code that builds a `StartProvider` needs `prompt: None` to keep today's behaviour.

- **Android FCM device-token enrollment, in the same `native` module as iOS.** The platform is a transport choice and nothing more: a snapshot whose latest answer is an FCM registration token registers as the typed `PushCredential::Fcm` credential, an APNs hex token registers as `Apns`, and the enroll/reconcile/disable workflow around them — gesture enrollment, nonprompting launch reconciliation with rotation and owner-verified retirement, stable disable keys — is one shared implementation, not a fork. FCM tokens validate as 1–4096 bytes of printable ASCII, never hex, and both token kinds keep redacted `Debug` output. The plugin's snapshot type is renamed `RemoteRegistration` (from `ApnsRegistration`) now that it carries either platform's answer, with a new `RemoteState::FcmToken` variant on wire state 4. _Packages:_ 🟠 _leptoskit_notifications_ _Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)
- **Browser push enrollment, corrected to the browser's real lifecycle: the client half of Web Push.** A new `browser` feature ships the receiver service-worker script (built with your inbox route, served same-origin at the root), a typed availability check, and `enroll`, `reconcile`, and `disable` flows that ride the person's own RPC identity over the feature's own same-origin fetch transport — no service token, nothing persisted in the worker. `reconcile` runs on launch and registers the subscription the browser already holds without ever prompting, answering `None` when there is none; `enroll` runs from the gesture that can subscribe. Enrollment waits for the selected worker's own activation (a failed install errors instead of hanging), matches a requested scope exactly rather than letting a broader worker stand in, and reuses an existing subscription only when it was created for your own VAPID key — another sender's is refused, not replaced. The enrollment handle records the subscription and scope it owns, so `disable` revokes first and then unsubscribes exactly that subscription: a stale handle leaves a replaced subscription and another scope's enrollment alone, and a retried disable whose revoke already landed completes. The receiver shows the service's envelope under a tag built from the notification's id, falls back to one truthful generic banner for missing, oversized, malformed, or mistyped pushes, and routes clicks only to your own inbox route on your own origin. _Packages:_ 🟠 _leptoskit_notifications_ _Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)

#### Save a campaign as a draft and launch it once

_Packages:_ 🟠 _leptoskit_notifications_

_Owner:_ Ifiok Jr. · _Introduced in:_ [99ecf87](https://github.com/leptoskit/leptoskit/commit/99ecf87c362ec9f1cd0d12e6591e9cdbfa1ab06f)

A person composing a notification on a screen writes it in steps, tries it on their own phone, lets someone else fix it, and picks a time before sending. `send` had nowhere to keep that work, and nothing stopped a double press from being two sends. The new `campaign` module keeps a draft in the service and turns it into exactly one send. All of it is for your app's server only.

- `create_campaign` saves a `CampaignDraft` (a name, a `SendSpec`, an optional `SendTarget`, and a note). A draft may be unfinished: only its size is checked.
- `update_campaign` saves a change made from a given `revision` and refuses it with a conflict when the campaign was saved in between.
- `launch_campaign` makes the send, with everything `send` checks. Launching the same revision again answers the first send as a duplicate. A draft whose `visible_at` has passed is refused, and a draft for everyone is refused unless `broadcast` is `true`.
- `cancel_campaign` cancels the send by the rules of `cancel`. `archive_campaign` puts a campaign away.
- `duplicate_campaign` and `duplicate_send` save a copy as a new draft, without the original's schedule. A recipients send's copy has no audience.
- `test_campaign` pushes the draft to one person's devices now, writes no inbox row, and records what the push came to.
- `campaign`, `campaigns`, and `campaign_events` read a campaign, list campaigns, and list who did what. Every write takes `by`, the acting person's name as your server knows it.

**What changes for an existing app:**

- The `campaigns` migration adds two new, empty tables, `campaigns` and `campaign_events`, on SQLite and Postgres. Nothing existing is touched.
- Campaigns are not deleted by retention, and `forget_person` does not edit drafts. A draft that lists recipients keeps those ids until you change it.
- `send`, `cancel`, and every existing operation are unchanged.

#### Choose a collapse key and a push lifetime

_Packages:_ 🟠 _leptoskit_notifications_

_Owner:_ Ifiok Jr. · _Introduced in:_ [83a2053](https://github.com/leptoskit/leptoskit/commit/83a205359a092ad96b4ee03e084a49462fb5f848)

A score that changes every few minutes should replace the last push, not add a banner, and a push worth ten minutes should not stay queued at a provider for days. Until now every push collapsed by its notification's id, and the only lifetime was `expires_at`, which also removes the item from the inbox. Three new options fix this, and a send that sets none of them is byte for byte what it was.

- `MessageOptions::collapse_key` is a key of your own, 1 to 32 letters, digits, `-`, and `_` (`MAX_COLLAPSE_KEY_LEN`). It is sent as `apns-collapse-id`, as FCM's `collapse_key` and the Android notification's tag (unless the send sets `android.tag`), and as the Web Push `Topic` header and the banner's tag in the browser.
- `MessageOptions::push_ttl` is how many seconds a provider may hold the push, counted from when the notification becomes visible, at most 2,419,200 (`MAX_PUSH_TTL_SECS`, 28 days). The inbox item is not affected. With `expires_at` too, the push stops at the earlier one. `0` means one attempt that no provider stores and that is never retried.
- `AndroidOptions::raw_channel_id` names an Android channel exactly as the device holds it, for a channel an earlier build of your app created itself, for example under Firebase. `channel_id` still names a channel made with the plugin. A send sets one or the other.

**What changes for an existing app:**

- `AndroidOptions` and `MessageOptions` have new public fields. Code that builds either one by naming every field no longer compiles: end the literal with `..AndroidOptions::default()` or `..MessageOptions::default()`.
- A delivery that waited past the end of its `push_ttl` ends with the reason `expired`, which until now meant only that `expires_at` had passed.
- The receiver service worker reads a new `lktag` key. A browser still running the previous worker ignores it and shows the banner under the notification's id, as before.
- No migration. Both options are stored in the options JSON each notification row already has.

#### Import the device tokens you already hold

_Packages:_ 🟠 _leptoskit_notifications_

_Owner:_ Ifiok Jr. · _Introduced in:_ [9ad0492](https://github.com/leptoskit/leptoskit/commit/9ad0492713adfc894060c022ffe391b57391f392)

An app that moves from Firebase Cloud Messaging has a table of device tokens, and until now none of them could be used: every device had to update and enroll before it could be pushed to. `push::import_push_endpoints` takes those tokens from your app's server, so people are pushed to from the next send on.

- The call takes 1 to 500 entries (`MAX_IMPORT_ENDPOINTS`), each an `ImportEndpoint`: a person, an FCM registration token or an APNs device token, and optional `DeviceDetails`. A Web Push subscription cannot be imported.
- It answers `ImportedEndpoints`: how many were `imported`, how many were `unchanged` because their person already holds the token, and the `skipped` entries by index, each `held_by_another` or `person_full`. No credential is answered.
- One malformed entry refuses the whole call and names its index. Nothing is written.
- An import never moves a token between people, and never revives an endpoint a provider invalidated.
- `EndpointSummary` has a new `imported` field. It is `true` until the device's own app enrolls the same token for the same person. The endpoint then keeps its id and its queued pushes and takes the details the device reports.

Only your app's server may call it.

**What changes for an existing app:**

- The `endpoint_import` migration adds one nullable column, `push_endpoints.imported_at`, on SQLite and Postgres. Existing rows are not changed.
- `EndpointSummary` has a new public field. Code that builds one by naming every field no longer compiles. The field is left out of the JSON when it is `false`, so existing clients read the same answers as before.
- The service keeps no record of a revoke or an erasure. Importing a token again after its person turned notifications off, or after `forget_person`, enrolls it again.

- **Make the fanout and push engine finish what it starts.** Every batch of a topic, condition, or everyone send is pushed: the push rounds keep going while the walk is still adding deliveries, and a round never takes the key of one that already ran. A round does a bounded share — at most a thousand attempts, started within a minute — and hands the rest on at once, one round of a send at a time; one delivery's failure (an unreadable stored options value, a database error, a row gone mid-attempt) is that delivery's outcome, never the round's. A fanout step claims its batch before writing, keys its successor by the cursor it starts from, and a step that loses its batch to a replay keeps the walk going, so a send can no longer stay `fanning_out` forever. Each step wakes only its own batch's people. Retries wait one minute, doubling, with jitter that differs per delivery, on every platform, so an hour's provider outage loses nothing. Values that would overflow what the service stores are refused at send time on every target and in a dry run (`MAX_SPEC_LEN`, `MAX_TARGET_LEN`, `options::MAX_OPTIONS_LEN`), a full 255-byte idempotency key works everywhere, and a past `visible_at` is stored as now, so its push gets its full window. `cancel` is set-based and cancels a large fanout's rows in batches through the new `cancel_rows` job. Breaking: `send` and `send_test` are now `#[rpc::action]`s, so a dry run walks its audience and a test send waits on providers with no transaction open; the RPC layer keeps no request-ID receipt for them, so a retried `send` answers from its idempotency key and a retried `send_test` pushes again. _Packages:_ 🟠 _leptoskit_notifications_ _Owner:_ Ifiok Jr. · _Introduced in:_ [d1a5857](https://github.com/leptoskit/leptoskit/commit/d1a585735fc14179bce372d21586daa64ab84d56)
- **iOS push enrollment, second patch: the `native` module now covers the whole launch lifecycle, not just the gesture.** `reconcile(previous)` is the launch half and runs only for a device that previously opted in — the saved enrollment is the record — registering with APNs again without prompting (Apple expects it each launch) and never silently re-enabling a device that opted out. Its answer is an explicit transition: `Kept` (same token, same endpoint id, one idempotent call), or `Rotated { current, retired }` where the previous handle is verified against the signed-in person's endpoint list and retired under its owner with the stable `disable` key — rotated tokens never leak orphan endpoints, the quota refusal at ten endpoints is answered by retiring the known-stale handle and re-registering once, and a previous account's leftover handle is reported as `Stale` and dropped without ever being operated under the new identity. Errors name their retry state: `Unconfirmed` leaves the previous enrollment untouched, `Enroll` carries the handle to keep (or none once retired), and `RetireFailed`/`OwnershipUnknown` leave the new enrollment live with the old handle named for retry. The plugin contract is now an optional dependency behind `native` (guest side alone), so the shared types and the service build without it and without its Linux backend; silence answers `Unconfirmed` instead of replaying a stale token, and tokens never print through `Debug`. The whole guest control flow — plugin guest transport and RPC client over the HTTP plugin guest included — runs in a compiled wasm guest against played OS and service boundaries (`crates/leptoskit_wasm/tests/notifications_guest.rs`, wired into `test:plugins:guest`). _Packages:_ 🟠 _leptoskit_notifications_ _Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)
- **The `native` module's enrollment covers macOS: a generated macOS app with `[native.macos] aps-environment` set enrolls its APNs device token through the same `enroll`, `reconcile`, and `disable` calls as iOS and Android — the platform stays a transport choice, the token kind the host answers names the provider kind, and nothing forks beyond it.** Docs and error text name all three platforms now. _Packages:_ 🟠 _leptoskit_notifications_ _Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)

#### Message options, silent data messages, and test sends

_Packages:_ 🟠 _leptoskit_notifications_

_Owner:_ Ifiok Jr. · _Introduced in:_ [93e2a2a](https://github.com/leptoskit/leptoskit/commit/93e2a2af050fea905b51e69b98219691552ca047)

A send can now be shaped per platform. `SendSpec` takes an `options` object — every field optional, and a send without one byte-for-byte what it always was:

- **`kind`**: a visible notification (the default) or a **data message** — silent everywhere: APNs `content-available` under `apns-push-type: background` at priority 5, an FCM data-only message, and a Web Push envelope the receiver worker deliberately does not show. Data sends create no inbox row and never count toward a badge, but their delivery, status, idempotency, cancellation, and fanout behave exactly like a visible send's.
- **`priority`**: `apns-priority` 10/5, `android.priority` HIGH/NORMAL, Web Push `Urgency`.
- **`image`**, **`sound`**, **`link`**: carried where each platform reads it (the web receiver opens a click link on its own origin only; Apple platforms need a Notification Service Extension to display an image).
- **`badge`**: an exact number, or `unread` — resolved **per recipient at push time** to their live unread count, so one send badges each person's own number. The web receiver sets it through `navigator.setAppBadge`.
- **`android`**, **`apple`**, **`web`** overrides: channel, color, icon, tag, sticky on Android; subtitle, thread-id, category, interruption-level, relevance-score on Apple; icon, badge shape, up to two actions, require-interaction, direction, and language on the web.

Everything is validated on both sides with errors naming the field, options count in a send's idempotent intent (the same key with different options is a conflict, and pre-option sends keep answering their own retries), and when a payload is over its provider's limit the extras drop in a documented order — per-platform blocks, image, sound, badge, and link, then the send's `data` — before the id-only envelope.

**`send_test`** (app-only) pushes a spec to one person's live endpoints — FCM's "test on device" — once each, answering per-endpoint outcomes and writing nothing: no inbox row, no delivery row, no count anywhere. It is bounded by the person's enrollment (at most ten live endpoints) and can probe a single endpoint by id.

The receiver service worker honors what arrives: a data message shows nothing but still sets the badge it asked for, options shape the banner, and a click opens the send's link only when it resolves to the app's own origin.

#### Count opens, and report what a send came to

_Packages:_ 🟠 _leptoskit_notifications_

_Owner:_ Ifiok Jr. · _Introduced in:_ [34082d8](https://github.com/leptoskit/leptoskit/commit/34082d8fe982f233b46a515e4fa2803d9219db46)

A tap on a push reached your app and stopped there: the service never learned of it, the person's other devices still showed the notification as unread, and nothing could say how many people opened a send. Opens are now recorded, and one call reports what became of a send.

- `mark_opened` is a new operation for the signed-in person. It records the first open of one of their notifications and marks it read in the same write. Opening it again changes nothing and succeeds. It is refused for another person's notification exactly as `mark_read` is, and a silent data message has no open.
- `Opened::mark_opened()` in the `receive` module makes that call from the place you handle a tap, on iOS, macOS, Android, and the web. It answers `Ok(false)` without calling anything for a tap that names no notification, such as a local one. On the web the page makes the call; the receiver worker still calls no server.
- `send_report` is a new app-only query. It answers a `SendSummary`: the send's `progress` (what `send_status` answers), its `label`, when it was made, and `inbox`, an `InboxCounts` of `rows`, `read`, `opened`, `dismissed`, and `cancelled`. The counts are one read of the send's own rows through an existing index. They count the rows still stored, so they shrink once retention has deleted a send's rows, while `progress` does not change.
- `SendSpec::label` is an optional name for a send, 1 to 64 ASCII letters, digits, `-`, `_`, or `.`, checked by the new `check_label`. It is stored with the send, answered in its report, never sent to a device, and part of the idempotent intent. A send without one serializes exactly as before.

**What changes for an existing app:**

- The `open_tracking` migration adds the nullable `notifications.opened_at` and `sends.label` on SQLite and Postgres, with no backfill, and replaces the live triggers on `notifications`. Rows written earlier count as unopened.
- `Notification` and `AppSend` each gain a field, which breaks struct literals of them, and `SendSpec` gains `label`, which breaks its struct literals that do not end in `..SendSpec::default()`.
- Nothing is recorded until your app calls `mark_opened`. `send_status` and `SendProgress` are unchanged.

#### Devices change hands and forgotten people are erased

_Packages:_ 🟠 _leptoskit_notifications_

_Owner:_ Ifiok Jr. · _Introduced in:_ [75e6f21](https://github.com/leptoskit/leptoskit/commit/75e6f21d8c5dff5e4dba79671e5dd8deac3eba25)

Three things the notifications service got wrong on a shared device and for a deleted account, and four limits it did not have.

- **A shared device changes hands.** When someone enrolls a push destination that another person's enrollment still holds, and presents the byte-identical credential (the same APNs or FCM token; for Web Push the same endpoint and both keys), the service revokes the old enrollment and creates a fresh one for the new person. It used to answer `409`, which left the previous person's notifications arriving on a device the new person could not enroll. A different credential for the same destination is still refused.
- **`forget_person` erases the person.** It now deletes their attributes, topic subscriptions, category preferences, push endpoints (queued deliveries end `skipped`/`endpoint_revoked`), inbox rows, and the registry and wake rows. It used to keep the inbox, preferences, and endpoints. A long inbox is deleted in batches by the new `forget_rows` job; a notification sent after the call is kept. It stays app-only and safe to repeat. No migration.
- **A server-side revoke sticks.** `register_push_endpoint` takes an optional `renews` (the endpoint id a client believes it holds). A renewal only succeeds while that endpoint is still the caller's; otherwise it writes nothing and answers not-found with a message starting `push::ENROLLMENT_GONE` (`push::enrollment_gone` recognizes it). Both `reconcile` functions send it, so a device whose enrollment was revoked — from another device's list, by the app's server, or by `forget_person` — is told so and stays off until the person turns notifications on again.
  - **Breaking:** `browser::reconcile` now takes the saved enrollment, `reconcile(&options, previous.as_ref())`. With none it registers nothing; when the server no longer has it, it answers `None`.
  - **Breaking:** `native::Outcome` gains `Gone(Enrollment)`. Drop the saved enrollment when you see it. A launch with nothing changed now makes one small query (`push_endpoint_standing`) instead of no call.
- **New app-only operation `revoke_person_endpoints(person)`** ends every device of one person, for a server-side sign-out. New person-side query `push_endpoint_standing(endpoint)` answers whether the caller still holds an enrollment.
- **Bounds.**
  - A person keeps at most `MAX_ENDPOINT_HISTORY` (10) invalidated endpoints; a new enrollment deletes the oldest beyond that. The list used to grow without limit.
  - A page pinned with `until` reads at most 500 inbox rows (`MAX_PINNED_INBOX_ROWS`), 20 endpoints, or 500 topics, in page order. It used to read up to 10,000 rows whatever `limit` said.
  - `set_person_attributes` refuses a call that names more than `MAX_ATTRIBUTE_CHANGES` (100) keys for one person across `set` and `remove`.
  - A condition nested more than `MAX_CONDITION_NESTING` (32) parentheses and negations deep is refused before the parser descends, instead of recursing as deep as the text is long.
- **Condition values can hold spaces and quotes.** A value is everything between its quotes, and a quote inside one is written twice: `user.name == 'Mary O''Brien'`. `condition::quoted` writes a value from any string. A backslash is an ordinary character and control characters are refused. Topic names are unchanged, so a condition FCM accepts still parses the same way.

#### Push transports follow each provider's documented rules

_Packages:_ 🟠 _leptoskit_notifications_

_Owner:_ Ifiok Jr. · _Introduced in:_ [5b9dbae](https://github.com/leptoskit/leptoskit/commit/5b9dbae40f0f06fecea2c131711aa5241524fa96)

The APNs, FCM, and Web Push transports were checked against Apple's, Google's, and Microsoft's documentation and the Web Push RFCs, and corrected where they disagreed:

- **Safari and Edge can enroll.** Web Push now accepts any subdomain of `push.apple.com` (Safari's `web.push.apple.com`, as Apple asks senders to allow) and of `notify.windows.com` (the Windows Push Notification Services channels Edge on Windows hands out), matched on whole labels. The never-used `webpush.apple.com` is no longer accepted, and lookalikes, other ports, and credentials in the URL are refused as before.
- **FCM measures what FCM counts.** The 4,096-byte limit is checked on the keys and values a device receives, not the whole HTTP request, and the APNs block no longer repeats the title and body unless a subtitle needs them. The longest title and body a send allows now reach an Android device instead of ending `oversized`. The degradation order is the same on all three transports: the per-platform blocks and the image, then sound, badge, and link, then the send's `data`.
- **Silent means silent.** A data message carries no notification block on any provider: no `android.notification` at FCM, which would show it in the tray, and nothing but `content-available` in APNs's `aps`. A data send's badge rides as `lkbadge` data for the app to apply. `image`, `sound`, `link`, and the `android`, `apple`, and `web` blocks are now refused on a data send, naming the field.
- **Data messages are never pushed to a browser.** Browsers require every push to show a notification, so a data send's Web Push deliveries end `skipped` with the new reason `platform_unsupported`, and its native deliveries go as usual.
- **Only a gone device is invalidated.** APNs `410`, FCM `UNREGISTERED`, and a push service's `404`/`410` invalidate an endpoint. APNs `BadDeviceToken` and `DeviceTokenNotForTopic` (a wrong environment or topic), FCM `SENDER_ID_MISMATCH` and its third-party credential errors, and a refused VAPID signature end the delivery as `operator_configuration` and leave the device enrolled. A provider token APNs refuses is dropped from the cache and the push retried under a fresh one.
- **FCM's reserved data keys** now include `message_type` beside `from`, `google.*`, and `gcm.*`; `lkbadge` joins the envelope's own keys. They are dropped from the FCM copy only.
- **A Web Push subscription whose key is no point on the curve** ends `skipped` with the new reason `subscription_unusable` instead of `oversized`.
- **Configuration.** Setting `LEPTOSKIT_NOTIFICATIONS_WEB_PUSH_EXTRA_HOSTS` without a VAPID pair now refuses to start, naming the missing variables. The guide shows the `secrets` list the `[services.notifications]` table needs for the push variables to reach the service, names every variable in full, and shows how to set a `.p8` key and a service-account JSON and how to generate a VAPID pair.

#### Add durable server-side push delivery and authenticated endpoint enrollment to the notifications service.

A signed-in person's app registers the credential its platform issued — a typed APNs device token (topic and sandbox/production server-side), an FCM registration token, or a Web Push subscription — against the forwarded identity, bounded at ten live endpoints, with the credential stored only as secret material and a SHA-256 fingerprint for idempotence, and never in a list, a status row, an error, or a debug string. A credential another live account holds is refused rather than reassigned; only the owner revokes, and revocation deletes the endpoint and skips its queued deliveries as revoked.

_Packages:_ 🟠 _leptoskit_notifications_

_Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)

A send now fans one durable delivery per notification and live endpoint (`push_deliveries`, unique per pair), attempted by a `push_send` job when the notification becomes visible: schedule, cancellation, expiry, the latest opt-out, and endpoint liveness are re-checked at the moment of the attempt, with the attempt as the documented revocation boundary. Transient failures retry with the provider's `Retry-After` honored outright (or the engine's backoff) up to a bounded attempt count; permanent failures end clearly with sanitized reason codes, and an unregistered credential invalidates its endpoint and skips its siblings. Delivery is at-least-once with stable collapse ids — the notification's id is the `apns-collapse-id`, the FCM collapse key, and the Web Push topic — and provider acceptance is never claimed as a user receipt. The app reads per-endpoint status (`push_deliveries`, app-only) as enqueued, retrying, accepted, rejected, or skipped, and a public recipient can send nothing.

Real transports ship behind the service feature: APNs HTTP/2 with ES256 `.p8` provider tokens (cached), FCM HTTP v1 with service-account JWT-to-OAuth exchange and expiring cached tokens, and Web Push with RFC 8292 VAPID and RFC 8291 `aes128gcm` encryption — all on ring's maintained primitives, over an injected HTTP boundary that refuses redirects, ignores proxy variables, verifies TLS, and bounds response reads. Web Push endpoints are held to a fixed shipped-provider host policy (extras opt-in, DNS-validated and pinned), payload limits are checked on the encoded bytes with a minimal id-only envelope fallback, and half-configured providers refuse to start with every missing variable named while unconfigured transports stay explicitly unavailable. New sends already past their expiry are refused. Native token acquisition and generated browser push handlers remain future work.

#### Correct the push transports' provider interoperability and the endpoint/delivery lifecycle, ahead of any release.

_Packages:_ 🟠 _leptoskit_notifications_

_Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)

The FCM service account now must be the JSON Google actually issues — an RSA key — and its OAuth assertion is built and signed with the maintained `jsonwebtoken` crate as RS256, verified against Google's contract rather than the ES256 EC key path that never worked against the real endpoint. The exchange is single-flight with a cached token, a fixed first-party token endpoint (the JSON's `token_uri` is ignored), strict rejection of malformed or nonsensical token answers, a bounded one-refresh retry when the provider answers 401, and a clear split between operator-account trouble — which stalls the delivery as `retrying` without spending its attempts — and a device the provider no longer knows, which alone invalidates an endpoint (a generic 404 from FCM no longer kills a device). Web Push encryption and VAPID are now the maintained `web-push` crate's (Mozilla's `ece` underneath) instead of hand-rolled RFC 8291/8292 code, with the same injected HTTP boundary and the payload-sizing fallback kept, and the crate's output verified in tests by an independent ring-based receiver.

Endpoint identity is now the platform and destination — the token, or a Web Push subscription's endpoint URL — not the whole credential: a browser that rotates its keys keeps its endpoint, its owner, and its queued deliveries (the stored credential rotates in place, and padded or unpadded key encodings enroll as the same subscription), while another account can never take a live destination over by presenting new keys. An invalidated destination can be re-enrolled by its owner (reactivated) or taken over by the device's next account. `push_endpoints` is now paginated and includes invalidated endpoints with their state, and a person's invalidated history is bounded, so devices that churn can't grow the table or hide live endpoints behind the ten-endpoint bound.

Delivery writes are fenced on each row's own state and attempt count, so a revocation, a sibling invalidation, or a replayed job round that lands while an attempt is in flight can no longer resurrect or regress what already committed; the retry bound is eight actual network attempts (skips and operator stalls spend none); and the FCM payload now carries the send's expiry as the platforms' TTL (`android.ttl`, `apns-expiration`), keeps its collapse key through the id-only fallback, and drops the caller's reserved data keys (`lkn`, `from`, `google.*`, `gcm.*`). Web Push extra hosts resolve within a bounded time, DNS failure retries instead of ending the delivery, pinned connections re-resolve per attempt without a rebinding hole (the client cache keys on the exact address set and stays bounded), the private-address check covers the full IANA special-purpose registries, and a malformed extra host refuses to start by name. Secret-bearing types — the credentials, the wire requests and responses — print redacted `Debug`, and the push configuration is carried per service instance (routes and job arguments) instead of one process-wide interchangeable global. The real HTTP boundary is exercised against local servers: redirects surfaced never followed, endless bodies refused at the read bound, pinned connections pinned, and HTTP/2 spoken.

#### Send a notification on a repeating schedule

_Packages:_ 🟠 _leptoskit_notifications_

_Owner:_ Ifiok Jr. · _Introduced in:_ [997222d](https://github.com/leptoskit/leptoskit/commit/997222dc4cc3b5ca07a0f8c3664175355273052b)

A message that goes out every morning or every Monday used to be a send your own server made each time, from its own scheduler. The new `recurrence` module keeps the schedule in the service, which makes one send per occurrence. All of it is for your app's server only.

```rust
use leptoskit_notifications::recurrence::{
    CreateRecurrence, NewRecurrence, RecurrenceEnd, RecurrenceRule, Repeat,
};

let saved = call_as_app(CreateRecurrence {
    recurrence: NewRecurrence {
        name: "Morning reminder".into(),
        spec: SendSpec {
            category: "reminders".into(),
            title: "Time to check in".into(),
            ..SendSpec::default()
        },
        target: SendTarget::Topic("reminders".into()),
        rule: RecurrenceRule {
            repeat: Repeat::Daily { every: 1 },
            at: "08:00".into(),
            time_zone: "America/New_York".into(),
            starts_on: None,
            ends: Some(RecurrenceEnd::After(30)),
            local_time: false,
        },
        note: None,
    },
    by,
    broadcast: false,
})
.await?;
```

- A rule repeats daily, weekly on chosen weekdays, or monthly on a day of the month, at a wall-clock time in a named IANA time zone. It can end on a date or after a number of sends. `local_time: true` delivers each occurrence at that time in each person's own time zone.
- The time follows daylight saving changes. A time the clocks skip goes out when the gap ends, a time the clocks repeat goes out the first time only, and day 31 in a shorter month is the month's last day.
- Each occurrence is an ordinary send: the same checks, the same opt-outs. Its idempotency key is `recurrence:<id>:<the occurrence's instant in Unix seconds>`, so it is made once however many copies of the service find it due. Its label is `recurrence.<id>`, which is how you find a recurrence's sends in `list_sends`.
- After the service was down, it sends at most the latest missed occurrence, and only if that is less than 24 hours late. The rest are counted as `skipped`.
- `pause_recurrence`, `resume_recurrence`, and `end_recurrence` stop, restart, and finish one. `recurrence` reads one and `recurrences` lists them. A recurrence cannot be edited: end it and create another.
- An occurrence whose send is refused pauses the recurrence and stores the reason in `error`.
- `SendTarget::Everyone` is refused unless `broadcast` is `true`. At most 100 recurrences can be active or paused at one time, and a recurrence goes out at most once a day.

**What changes for an existing app:**

- The `recurring_sends` migration adds one new, empty table, `recurrences`, on SQLite and Postgres. Nothing existing is touched.
- The service runs one more job, `recur`, at each occurrence's time and once an hour.
- `forget_person` now also takes the person out of every recurrence that lists them as a recipient, and ends a recurrence left with nobody.
- An ended recurrence is deleted `LEPTOSKIT_NOTIFICATIONS_RETENTION_SENDS_DAYS` after it ended. Active and paused ones are never deleted.
- `send`, `cancel`, and every existing operation are unchanged.

#### Retention, and indexes for the reads that grew with the tables.

The service now deletes what is over, once an hour, in small batches that never hold a long write: inbox rows 365 days after they became visible, expired, cancelled, and silent rows 30 days after they ended, finished push deliveries after 30 days, and a send's record 90 days after the send, once no notification or delivery of it remains. Each is set in days, or `off`, with `LEPTOSKIT_NOTIFICATIONS_RETENTION_INBOX_DAYS`, `_ENDED_DAYS`, `_DELIVERIES_DAYS`, and `_SENDS_DAYS`; a bad value refuses to start. A row with a queued delivery, a send still fanning out, and a scheduled notification are never deleted. An idempotency key is remembered for as long as its send's record is kept — at least `SENDS_DAYS`, never less than 2 — and is a new send after that. `service::workers_with_settings` takes the retention for an embedded service, and `service::retention::prune` runs one sweep directly.

_Packages:_ 🟠 _leptoskit_notifications_

_Owner:_ Ifiok Jr. · _Introduced in:_ [2cedeb1](https://github.com/leptoskit/leptoskit/commit/2cedeb1a738ac9706f97633b51e4c15b171d5fc7)

A new `scale_indexes` migration on both engines adds `push_deliveries (endpoint, state)`, so a token rotation, a revoke, or an invalidation no longer reads every queued delivery; `push_deliveries (send_key, state, id)`, which a push round now reads one state at a time instead of walking the send's finished deliveries; and `notifications (person, kind, dismissed_at, read_at)` for the unread count. It drops `push_deliveries (state, next_attempt_at)`, which no query used. On Postgres the indexes are built under a lock that makes writes to the table wait; build them first with `CREATE INDEX CONCURRENTLY` under the same names to avoid that. A condition naming several topics now reads each topic's subscribers in index order and merges them, instead of sorting the union on every batch.

Two rollbacks are corrected: rolling `message_options` back cancels every silent data message instead of leaving it to appear in an inbox, and rolling the Postgres `init` back drops the `live` and `rpc` schemas when they are left empty.

- **Person attributes, segments, and send to everyone.** The service keeps a registry of the people it knows — registered when the app's server sets their attributes, when they subscribe to a topic, set a category preference, or enroll a push endpoint, and backfilled from those tables by the migration; a person named only in a recipients send is never registered. The app's server describes people with `set_person_attributes` (batch, ≤1,000 people, idempotent; keys `a-z0-9_` ≤64 bytes, values ≤256 bytes, ≤50 attributes per person with cap-skipping counted; `person_attributes` reads back; `forget_person` removes the person from the registry with their attributes and topic subscriptions, keeping their inbox) — app-only by design, so a segment can't be forged from the client side. The condition grammar extends FCM's with `user.<key> == 'v'`, `!=`, `in ['a','b']`, `has(user.<key>)`, and finite-number `<`, `<=`, `>`, `>=` (true only when both sides parse as finite numbers; `NaN`/`inf` literals refused), mixing freely with `'t' in topics` under `&&`/`||`/`!`/parens with byte-offset errors and bounds (≤5 topics, ≤10 attribute predicates, depth ≤8, ≤1 KiB); an absent attribute answers false to every predicate, and the everyone-in-disguise refusal now checks topics _and_ attributes. `SendTarget::Everyone` reaches every registered person, and a condition no longer needs to be anchored on a topic: one that structurally requires a topic still fans out over the topic index, while an attribute-anchored one (and everyone) fans out over the registry — same batched, cursor-advancing, idempotent fanout job, same opt-out/visibility/push/live behaviour, `validate_only` counting every target. SQLite and Postgres migrations add `persons` and `person_attributes` with the registry backfill. _Packages:_ 🟠 _leptoskit_notifications_, 🟠 _leptoskit_ _Owner:_ Ifiok Jr. · _Introduced in:_ [644b0a3](https://github.com/leptoskit/leptoskit/commit/644b0a354e16352b37cfb3c78934d5c4e5b76a6e)

#### List sends, size an audience, and preview a push

_Packages:_ 🟠 _leptoskit_notifications_

_Owner:_ Ifiok Jr. · _Introduced in:_ [d45cced](https://github.com/leptoskit/leptoskit/commit/d45cced1f47bab9b8c7ae5fad9ef89c937875ce0)

A screen for composing notifications had nothing to ask the service: it could not list what was sent, could not show which topics or attributes exist or how many people a target would reach, and could not show a push before it went out. The service now answers all three, to your app's server only.

- `Condition` implements `Display`. `to_string()` writes one canonical spelling of a parsed condition, and parsing that text gives back the same tree. A screen that builds a condition as a tree can hand the service text without writing its own quoting.
- `list_sends` is a new app-only query. It answers the sends made since this version, newest first, in pages of at most 100, each as a `SendEntry` with its key, state, label, category, title, kind, target, schedule and counts. `SendFilter` keeps one state.
- `send_report` answers four more fields on `SendSummary`: `spec` (the send as it was made), `target` (a `TargetSummary`), `push` (deliveries per platform by state, with the reasons the ended ones ended) and `reads_by_hour` (reads per UTC hour, up to 168 hours).
- The new `catalogue` module has seven app-only queries: `notifications_overview`, `topics`, `attribute_keys`, `attribute_values`, `categories`, `estimate_audience` and, in `push`, `person_endpoints`. Lists are pages of at most 50. Every count is a `Count { count, exact }` that stops at a cap. `estimate_audience` walks at most 5,000 people and says whether its answer is exact. A device that was revoked or stopped working is not counted as reach, and no answer carries a credential.
- `preview_push` in the new `preview` module is an app-only query. It answers, for APNs, FCM and Web Push, the headers and the exact body a provider would be sent for a spec, the bytes counted against the provider's limit, and what was trimmed to fit. It is built by the code that sends, reads no credential, and needs no push configuration.

**What changes for an existing app:**

- The `send_history` migration adds the nullable `sends.history_id` and `sends.category` and five indexes, on SQLite and Postgres, with no backfill. Sends made earlier keep their reports and are not listed. On a large Postgres database the indexes can be built ahead with `CREATE INDEX CONCURRENTLY`; the guide's "Upgrading a large database" says how.
- A recipients send now stores its spec with its record, as topic and condition sends already did, until the record's retention deletes it.
- `SendSummary` no longer implements `Eq`, because it now holds a `SendSpec`. `AppSend` gains two fields, which breaks struct literals of it.
- Inside the `service` feature, `transport` requests are now built by a `shape` function per provider. What is sent is unchanged.

- **Add `leptoskit_notifications`, the first-party notifications service: an app's server sends idempotently to explicit user ids, and each recipient gets one durable inbox row in the service's own database — scheduled visibility with cancel-before-due, expiry, category opt-outs enforced at send and at visibility, and per-person read and dismissed state shared across devices, read through bounded, live-paginated queries on every platform the app's screens run.** Time reaches open screens: every send enqueues a wake job when it becomes visible and at its expiry, so a scheduled item appears and an expired one leaves a watching screen without any other write; preference changes update open inboxes the same way. An accepted send answers its stored outcome forever, however the clock moves on, and a known zero-fanout send cancels idempotently. Delivery is authenticated inbox retrieval while the app runs; OS background push (APNs, FCM, Web Push) and device enrollment are not part of this change. The `leptoskit` facade re-exports the shared types behind a `notifications` feature, and the guide is under Services. _Packages:_ 🟠 _leptoskit_notifications_, 🟠 _leptoskit_ _Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)
- **Audience partitioning by topics and conditions, with a durable batched fanout.** A send can now target `SendTarget::Topic` or `SendTarget::Condition` beside today's `SendTarget::Recipients`: the app's server subscribes people to per-person topics in idempotent batches (`subscribe_to_topic`/`unsubscribe_from_topic`, at most 1,000 people per call; a person holds at most 500 topics, and a full person is skipped and counted rather than refusing the batch), and a person may subscribe themselves (`subscribe`, `unsubscribe`, `my_topics`, live and paginated) to the topics the operator opened in `LEPTOSKIT_NOTIFICATIONS_OPEN_TOPICS` — patterns like `news.*`, nothing open by default, and the person always read from the forwarded identity. Conditions use FCM's grammar verbatim (`'a' in topics && ('b' in topics || !('c' in topics))`), validated on both sides with byte-offset errors, FCM's bounds (≤5 topics, depth ≤8, ≤1 KiB), and unanchored conditions — ones true for a person holding no topics — refused, since there is no everyone audience. A topic or condition send answers `fanning_out` immediately and a job walks the audience in 500-person batches — one transaction per batch for the inbox rows, push deliveries, and the stored cursor, unique per send and person so a crashed or replayed batch never duplicates, the audience snapshotted as the fanout reaches each person. `send_status` answers the state and counts; `validate_only` counts the audience without writing; `cancel` stops a fanout mid-walk and cancels the rows written so far. SQLite and Postgres migrations add the `topic_subscriptions` table, the fanout bookkeeping on `sends`, and the per-send-per-person uniqueness on `notifications` (existing rows backfilled). Breaking: `Send` and `check_send` take the target — `recipients: vec![…]` becomes `target: SendTarget::Recipients(vec![…])` — and `SendResult` gains `state` and `targeted`. _Packages:_ 🟠 _leptoskit_notifications_, 🟠 _leptoskit_ _Owner:_ Ifiok Jr. · _Introduced in:_ [ed7b1e7](https://github.com/leptoskit/leptoskit/commit/ed7b1e77bb3ac93482b58819cdad9df80fdaa166)

#### Translated variants and delivery in each person's local time.

`SendSpec` gains `variants`, a map from a BCP 47 language tag to a `Variant` that replaces the title, the body, the Apple subtitle, and the web action titles for people who read that language, and `local_time`, a `LocalTime { at, default_time_zone }` that makes each person's row visible, and pushed, when a wall-clock time such as `2026-12-01T09:00` arrives in their time zone. Every person still gets exactly one inbox row, and its push carries the same text.

_Packages:_ 🟠 _leptoskit_notifications_

_Owner:_ Ifiok Jr. · _Introduced in:_ [8b56972](https://github.com/leptoskit/leptoskit/commit/8b569722a21f1d87a315715b4d129f75b7bf49bd)

Language and time zone resolve per person: the `locale` or `time_zone` attribute the app set wins, then the person's most recently seen live endpoint, then the send's default (its own text, and `default_time_zone`, which is UTC when left out). A locale matches the variant with its exact tag, then with its last subtag dropped, down to the language; nothing matching reads the send's own text. Time zones are IANA names looked up in bundled time zone data (`jiff`, behind the `service` feature only). A zone the service does not know on a person is skipped; an unknown `default_time_zone` is refused. A local time already past for a person arrives at once, a time a daylight-saving change skips arrives after the gap, and a repeated time arrives the first time.

A send carries at most 32 variants and still fits `MAX_SPEC_LEN`; a variant translates only text the send has, and a data message carries none. `local_time` and `visible_at` together are refused. `expires_at` must be after the local time has reached the last time zone (`at` plus twelve hours, as UTC), and the schedule limit is counted to that moment. Both fields are part of a send's idempotent intent and serialize to nothing when unset, so earlier sends answer their retries unchanged.

`cancel` now stops a local-time send for the people still waiting once others have seen it, and answers `0` when asked again; a send nobody is waiting for is still refused. Pushes for waiting rows are queued for their own instants, and a fanout enqueues one `wake_local` job per batch and distinct instant.

A new `endpoint_seen` migration on both engines adds the nullable `push_endpoints.seen_at`, set when an endpoint enrolls, changes its details, or renews (at most once an hour for an unchanged renewal). It adds an empty column and takes no measurable time.

**Source-breaking for struct literals:** `SendSpec` has two new fields and `PushEndpoint` one. `SendSpec` now implements `Default`, so `..SendSpec::default()` fills in what a send does not set.

#### Accounts, tokens and projects for LeptosKit Cloud

_Packages:_ 🟠 _leptoskit_cloud_

_Owner:_ Ifiok Jr. · _Introduced in:_ [9bd21da](https://github.com/leptoskit/leptoskit/commit/9bd21da8b6fcad9589adc4968b71fb92a656c42a)

`leptoskit_cloud` now declares the control plane's first operations, so a client such as the CLI can call them without building the server: `whoami`, organisations, members and invitations, API tokens, and projects and their environments. Their tables and argument types build everywhere, including wasm; their bodies, sign-in and routes are behind the new `server` feature, with SQLite and Postgres migrations.

The server keeps tenants apart and checks every call against one permission table: an organisation or project you can't see answers "not found" whether it exists or not, an organisation always keeps an owner, and a token never does more than its scopes and its maker's current role allow, or reaches a project it isn't bound to. API tokens look like `lkc_<prefix>_<secret>`, are shown once, stored only as a hash, expire after 90 days unless asked otherwise, and stop the moment they're revoked. Invitations are mailed, single-use, expire in a week, and work only for the address they were sent to. The CLI signs in with the device flow, and every change is written to an audit log that never holds a secret.

A queued deployment can now fail before any work starts, when no builder is available, or a quota, the spend cap or a payment stops it.

#### Back up and restore a managed database

_Packages:_ 🟠 _leptoskit_cloud_

_Owner:_ Ifiok Jr. · _Introduced in:_ [dddcd97](https://github.com/leptoskit/leptoskit/commit/dddcd97281bb14d1648cfb3b77517c23e27bca13)

`leptoskit_cloud::databases` adds three operations. `create_backup` asks an environment's cluster for a dump of its Postgres database, kept for 30 days. `database_backups` lists them, with the latest restore. `restore_backup` puts one back over the database it was taken from, for owners and admins who type `<project>/<environment>` to confirm, after the cluster has taken a backup of what it replaces.

The control plane keeps a record of each backup and never its bytes. `JobKind`, `JobInput` and `JobOutcome` gain `BackupDatabase` and `RestoreDatabase`, with the contract a cluster worker must keep on `BackupDatabaseInput` and `RestoreDatabaseInput`. `deploy` and `rollback` answer `database_restoring` while a restore is under way. Backups on a schedule are not available yet.

#### Add environment variables and the secrets vault

_Packages:_ 🟠 _leptoskit_cloud_

_Owner:_ Ifiok Jr. · _Introduced in:_ [25cddbe](https://github.com/leptoskit/leptoskit/commit/25cddbed387b5b2483808d0171a5b063884ba739)

`leptoskit_cloud::config` adds `variables`, `set_variable`, `unset_variable`, `secrets`, `set_secret` and `unset_secret`, behind a new `manage_config` permission for owners, admins and developers. Variables are written into the next deployment's release; secrets are named in it and their values are sealed per organisation with AES-256-GCM-SIV, bound to where they were stored, and leave the control plane only in a cluster worker's claim of that release, with a lease row for each.

A release whose secrets can't be read fails with the new `secret_unavailable` blocker. The control plane needs `LEPTOSKIT_CLOUD_VAULT_KEYS`, and `ControlConfig` gains `vault`. An environment's variables take at most 256 KiB together and its secrets 512 KiB, and `set_secret` takes a `SecretValue`, which is never repeated in an error.

#### Give Postgres apps a managed database

_Packages:_ 🟠 _leptoskit_cloud_

_Owner:_ Ifiok Jr. · _Introduced in:_ [ac56912](https://github.com/leptoskit/leptoskit/commit/ac56912d7456e158da2a78464c5e19a4940b5ec1)

An app whose `[db]` is Postgres can now be deployed to a cluster that has a Postgres server. `deploy` gives its environment a database and a login role of its own, and the app's process starts with `DATABASE_URL` as a secret, handed by value only to the cluster worker that claims the release. `leptoskit_cloud::databases` adds `database`, which answers the database's name, size and whether a release has used it, and never its password.

`ClusterFacts` gains `postgres`, operators get `set_cluster_facts`, `ReleaseInput` and `DestroyEnvironmentInput` gain `database`, and `PlanError::PostgresUnavailable` now means the environment's cluster has no Postgres server. Dedicated instances and backups are not available yet.

#### Uploads, deployments and workers for LeptosKit Cloud

_Packages:_ 🟠 _leptoskit_cloud_

_Owner:_ Ifiok Jr. · _Introduced in:_ [f3c87dd](https://github.com/leptoskit/leptoskit/commit/f3c87dd0aa250554006aecfcdc607cd0d7bc95d1)

The control plane can now ship an app. A client uploads its source archive straight to object storage through a presigned address, which the server checks for size and SHA-256 before anything may deploy it. It then calls `deploy` and watches the deployment go from queued through building, migrating and rolling out to live, as live queries. Each deployment records who signed it off and from where, the configuration it resolved, and its source. Its sheet shows a line per part (server, services, migrations, web assets) with stamps, build steps and fine print. An environment has one deployment in flight at a time. `cancel_deployment`, `rollback` and `preview_release` (with a monthly estimate when the operator configures prices) round it out, along with paged, redacted `build_logs` and `runtime_logs` and `environment_status`.

Build and cluster workers have their own protocol and keys (`lkw_…`), paired once with a code an operator issues. Workers `claim` one job at a time with a long poll, `renew` their lease (which says when a deployment was cancelled), stream logs, and `finish` idempotently. A build whose worker disappears is retried, then fails as `builder_unavailable`. A release whose worker disappears is never replayed: it waits for the worker's saved result or an operator's `reconcile`. Operators register clusters, pair and revoke workers, and list jobs; an environment is placed on a cluster in its region at its first deployment. Deleting an environment that ran queues its teardown.

The `server` feature now needs object storage: an S3-compatible store, or a local directory for development and tests. The tables are compiled only with `server`, so a client build sees just the plain types the operations take and return, whatever database features the rest of its build turns on. A new `deploy` permission, held by owners, admins and developers and granted to tokens by the `deploy` scope, covers uploading, deploying, cancelling and rolling back.

#### Add custom domains to the control plane

_Packages:_ 🟠 _leptoskit_cloud_

_Owner:_ Ifiok Jr. · _Introduced in:_ [bbccf61](https://github.com/leptoskit/leptoskit/commit/bbccf6129b044e6d4e5221b7df8741b3cae96cb2)

`leptoskit_cloud::domains` adds `domains`, `attach_domain`, `verify_domain` and `detach_domain`. A domain is proved with a TXT record read over DNS-over-HTTPS, a verified hostname belongs to one environment in the whole cloud, and the environment's next release routes it with a certificate of its own. A detached domain keeps its hostname until a deployment without it is live, and a rollback never routes a domain the environment no longer holds.

`ControlConfig` gains `resolver`, and the control plane reads `LEPTOSKIT_CLOUD_DNS_RESOLVER`.

#### Count usage, limit plans and cap spending

_Packages:_ 🟠 _leptoskit_cloud_

_Owner:_ Ifiok Jr. · _Introduced in:_ [4a543fe](https://github.com/leptoskit/leptoskit/commit/4a543fedc532f7671c20e435a0ca95181e19447a)

`leptoskit_cloud::billing` adds `plans`, `usage` and `set_spend_cap`. The control plane now counts build minutes and compute seconds per project and calendar month, and `usage` prices a month on the organisation's plan with the operator's catalogue. Once a month's usage reaches the spend cap, `deploy` answers `spend_cap_reached`; nothing running is stopped.

A plan in the price catalogue can carry `limits` (`organizations`, `projects`, `custom_domains`) and a `spend_cap`; `create_organization`, `create_project` and `attach_domain` answer `quota_exceeded` at a limit. A control plane now refuses to start with a catalogue that has no `hobby` plan. `Permission` gains `ViewBilling` and `ManageBilling`, and `pricing::Plan` gains `limits` and `spend_cap`, so code that builds a `Plan` by hand must set them. Nothing charges anyone yet.

#### Only people manage members and tokens in LeptosKit Cloud

_Packages:_ 🟠 _leptoskit_cloud_

_Owner:_ Ifiok Jr. · _Introduced in:_ [9bd21da](https://github.com/leptoskit/leptoskit/commit/9bd21da8b6fcad9589adc4968b71fb92a656c42a)

An API token can no longer invite people, revoke invitations, change a member's role, remove a member, or list, make or revoke tokens, whatever its scopes: those answer `403 person_required` to a token, and need someone signed in, in the browser or through the CLI. The `admin` scope now means deleting projects and environments, and nothing more. A leaked token can't turn itself into a membership or a longer-lived token.

`create_token` is now an action, so a token's value is never stored anywhere, not even as the remembered answer to a request with an `Idempotency-Key`; the header is ignored. Instead a live token's name is unique in its organisation, and asking again for a name a live token has answers `409 token_name_taken`, naming that token so it can be revoked, rather than quietly making a second one.

An invitation is only as good as its sender's authority when it is accepted: if they were demoted or left, it no longer works, and their pending invitations they can no longer give are revoked. Inviting an address again needs the authority to cancel each invitation it replaces, and each one cancelled is written to the audit log.

A personal organisation's name falls back to a random suffix after `ana`, `ana-2` and `ana-3`, so nobody can stop someone's first sign-in by taking every name their address suggests.

#### Sell plans through Stripe

_Packages:_ 🟠 _leptoskit_cloud_

_Owner:_ Ifiok Jr. · _Introduced in:_ [9dcd16a](https://github.com/leptoskit/leptoskit/commit/9dcd16acc5aabae6efd69e03663913cf48aed3d6)

`leptoskit_cloud::billing` adds `subscription`, `checkout` and `portal`. With a Stripe key, a webhook secret and a price ID for each plan that is sold, an organisation's owners and billing members can subscribe to a plan on Stripe Checkout and manage it on Stripe's customer portal. The plan changes only when Stripe says the subscription did: the control plane verifies each event's signature at `/webhooks/stripe`, then asks Stripe what the subscription is now.

`ControlConfig` gains `payments`, so code that builds one by hand must set it (`None` sells nothing). The `leptoskit_cloud_app` server reads `LEPTOSKIT_CLOUD_STRIPE_KEY`, `LEPTOSKIT_CLOUD_STRIPE_WEBHOOK_SECRET`, `LEPTOSKIT_CLOUD_STRIPE_PRICES`, `LEPTOSKIT_CLOUD_STRIPE_AUTOMATIC_TAX` and `LEPTOSKIT_CLOUD_STRIPE_TEST_MODE`; a Stripe test key is refused outside `--dev` without the last. A plan that is sold needs a `monthly` price and a `spend_cap` in the catalogue, and `set_spend_cap` now refuses a cap above the plan's own (`cap_above_plan`). Usage beyond what a plan includes is still not invoiced.

- **Store publishing needs the `publish` token scope.** An API token with only the `deploy` scope can no longer publish to the app and extension stores. Publishing has its own permission now, which owners, admins and developers hold and which only the `publish` scope grants to a token, so that scope now lets a token publish where before it granted nothing but reading. A CI job that runs `lk cloud publish` needs both scopes, because the command uploads the source archive before it queues the publish: `lk cloud token create ci --scope deploy --scope publish`. A token with `deploy` alone is refused by `publish` with `403 insufficient_scope`. Reading a publish, its report and its log is unchanged. _Packages:_ 🟠 _leptoskit_cloud_ _Owner:_ Ifiok Jr. · _Introduced in:_ [b2f38fb](https://github.com/leptoskit/leptoskit/commit/b2f38fb0e3ffb4aad87342a3576b2e5abfc1efd8)

#### Publish to app and extension stores from the cloud

_Packages:_ 🟠 _leptoskit_cloud_, 🟢 _leptoskit_cli_

_Owner:_ Ifiok Jr. · _Introduced in:_ [101b2ee](https://github.com/leptoskit/leptoskit/commit/101b2ee1e7acf5a060554a6a0bd7c9f597ff7903)

`leptoskit_cloud::publishing` adds `publish`, which builds an uploaded archive for its stores and uploads it on one of the organisation's build workers, as `lk publish` does, with the environment's secrets named as `lk publish` reads them. `publication`, `publications` and `publication_log` read a publish back, with its report and its log. A publish is never run twice, since it may have reached a store. `JobKind`, `JobInput` and `JobOutcome` gain `Publish`; store credentials may now be set as secrets, and no release is given them.

`leptoskit publish` takes a keystore's `_BASE64` variable in place of its path for Google Play and the Solana dApp Store, and with `LEPTOSKIT_PUBLISH_CONTENTS_ONLY=1` refuses a credential that names a file.

#### Harden cloud manifests and fix Day-1 platform blockers

_Packages:_ 🟠 _leptoskit_cloud_runtime_

_Owner:_ Ifiok Jr. · _Introduced in:_ [29e916b](https://github.com/leptoskit/leptoskit/commit/29e916be038faad465ec62d3fc0e0dff905cfd56)

`ClusterProfile` gains `Serialize`/`Deserialize`, `edge_terminated_custom_domains` (for Cloudflare for SaaS custom domains without cert-manager annotations), `tenant_node_selector` and `tenant_tolerations` (to isolate tenant pods onto dedicated tenant pools), and `tmp_size_limit_mib` (bounding `/tmp` `emptyDir` and container/namespace `ephemeral-storage`). `worker_admission_policy` emits a `ValidatingAdmissionPolicy` restricting cluster-worker mutations to `lk-*` tenant namespaces.

The platform generator fixes CloudNativePG superuser access, `max_connections` (300), and `pg_hba`; renames the KEDA HTTP `HelmChart` to `keda-add-ons-http`; re-enables `metrics-server` and kubelet metrics scraping for CPU HPAs; adds k3s `secrets-encryption`; fixes Cloudflare Tunnel catch-all routing for custom domains and adds an `apps-http01` HTTP-01 `ClusterIssuer` for `cert-manager` mode; keeps Zot cluster-local by default; hardens `cloudflared` and `platform-ingress` allowlist policies; allows ICMP in `nftables`; supports per-node `isolation_platform` overrides and hot/cold S3 backup splits; and fixes hybrid `growth.toml` disk sizing and `advertise-address`.

- **Apply tenant releases through Kubernetes with bounded rollout waits, observed status, logs, safe pruning and namespace deletion.** Align sleep routing and cluster profiles with the platform add-ons and generate the worker's permissions. _Packages:_ 🟠 _leptoskit_cloud_runtime_ _Owner:_ Ifiok Jr. · _Introduced in:_ [ce06091](https://github.com/leptoskit/leptoskit/commit/ce06091df4bfe19985bb279678f2776aea1744dd)
- **LeptosKit Cloud's runtime crate: a release as data, and the Kubernetes objects that carry it.** `ReleaseSpec` is the whole desired state of one environment — the app and its services with exact image digests, ports, health paths, environment and secret variables, resources, scale, and volumes; the routes; and the policy — with typed validation that refuses a spec no runtime could honor, and a namespace hashed from the organisation, project, and environment ids so no customer-chosen name ever reaches Kubernetes. The `Runtime` trait is the adapter boundary (`apply`, `status`, `logs`, `destroy`), usable as `dyn Runtime`, with a recording fake behind the `testing` feature that validates specs and scripts ready, never-ready, and unavailable rollouts. The `kubernetes` feature builds the release's manifests deterministically: the namespace under the restricted Pod Security profile, quota and limit range, default-deny network policies with the release's doors only, a token-less service account, hardened Deployments, Services, KEDA's HTTP scaled object or a horizontal autoscaler, disruption budgets, and Ingresses with wildcard or cert-manager certificates — refusing, before anything is applied, sleep without the HTTP add-on, cookie stripping a plain Ingress cannot do, a secret with no value, and an issued certificate with no issuer. _Packages:_ 🟠 _leptoskit_cloud_runtime_ _Owner:_ Ifiok Jr. · _Introduced in:_ [a66ec54](https://github.com/leptoskit/leptoskit/commit/a66ec5415deb156d6ac5e38f2c3aa29c4e172003)
- **Reserve one surge pod for every process that rolls, and expose `Quota::for_processes` so control-plane quotas match runtime validation.** Processes with persistent volumes use Recreate and need no surge capacity. _Packages:_ 🟠 _leptoskit_cloud_runtime_ _Owner:_ Ifiok Jr. · _Introduced in:_ [ce06091](https://github.com/leptoskit/leptoskit/commit/ce06091df4bfe19985bb279678f2776aea1744dd)

#### Publish cloud CLI crates and prebuilt release binaries

_Packages:_ 🟠 _leptoskit_cloud_platform_, 🟠 _leptoskit_cloud_worker_, 🟠 _leptoskit_cloud_app_, 🟢 _leptoskit_cli_

_Owner:_ Ifiok Jr. · _Introduced in:_ [e47b3de](https://github.com/leptoskit/leptoskit/commit/e47b3de2192792651b71741c9094b14cf49492be)

- Registers `leptoskit_cloud_platform` (`leptoskit-cloud-platform`), `leptoskit_cloud_worker` (`leptoskit-cloud-worker`), and `leptoskit_cloud_app` (`leptoskit-cloud-control`) in `monochange.toml` and removes `publish = false` so all three crates version and publish with the main release group.
- Adds `[package.metadata.binstall]` to all three crates and extends `.github/workflows/publish.yml` to build, archive, upload, and provenance-attest prebuilt binaries for all release targets alongside `leptoskit` and `lk`.
- Bundles the console's two Latin font subsets inside `crates/leptoskit_cloud_app/assets/fonts/` so `leptoskit_cloud_app` packages cleanly for crates.io.
- Updates `lk cloud <name>` missing-binary hints to recommend `cargo binstall` and `cargo install`.

#### Service workers and stronger extension messaging

_Packages:_ 🟠 _leptoskit_extension_, 🟠 _leptoskit_service_worker_, 🟠 _leptoskit_, 🟢 _leptoskit_cli_

_Owner:_ Ifiok Jr. · _Introduced in:_ [363ebad](https://github.com/leptoskit/leptoskit/commit/363ebadd2effe51d9b954c31ab0b6bcbfdc46ab8)

Web apps can enable the `service-worker` feature to register a worker script, exchange messages, describe caching strategies, open Cache Storage, and manage Web Push subscriptions. Calls return `Unavailable` outside a secure browser context.

Browser extensions gain long-lived `runtime::connect` ports (with `Background::on_connect`), storage `clear` / `keys` / `get_bytes_in_use`, and permissions `get_all` / `on_added` / `on_removed`. The generated background worker keeps ports alive so the wasm side can post and disconnect by id.

- **Opt-in FCM for generated Android apps.** `[native.android] push = true` writes the Firebase messaging dependency into `.leptoskit/android/plugin-dependencies.txt`, writes `.leptoskit/android/gradle-plugins.txt` (the Google services plugin id, applied by the app's Gradle script), generates `LeptosKitMessagingService.kt` — whose `onNewToken` lands FCM registration-token refreshes in the notifications plugin's broker — and declares that service in the manifest. With push off, none of it is written and stale copies are removed, so existing apps generate byte-for-byte as before. Generation with push on stops before any write when the `notifications` plugin is not selected, when the app author's `google-services.json` is missing, or when the app's `build.gradle.kts` doesn't read the generated plugins file, naming the file and the lines to add. New scaffolds apply the generated plugins list and put the Google services plugin on the classpath, and the Android snapshots cover the whole opt-in and opt-out. _Packages:_ 🟠 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)
- **`lk admin serve --analytics-url`.** `lk admin serve` connects the admin's Analytics area to your site's analytics service. It takes the address from `--analytics-url`, or from `LEPTOSKIT_SERVICE_ANALYTICS_URL` when the flag isn't given, and the token from `LEPTOSKIT_SERVICE_ANALYTICS_TOKEN`. If an address is given but the token is missing, it stops and says which variable to set. With neither, the Analytics pages explain how to connect. _Packages:_ 🟠 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [60e11ae](https://github.com/leptoskit/leptoskit/commit/60e11ae75fdffff7d6da8bc30d4369aced2e9d41)
- **`lk admin serve`.** `lk admin serve --project <repo> --url <admin origin> --main-site <site origin> --client-id <id>` runs the admin for the repositories you name, with the deployment service and its worker inside it. It reads the client secret your site issued from `LEPTOSKIT_ADMIN_CLIENT_SECRET`, its session keys from `LEPTOSKIT_ADMIN_KEYS`, and the deployment service's owner credential from `LEPTOSKIT_CONTROL_KEY`. `--role` changes which role gets in, and `--environment` labels a staging admin. _Packages:_ 🟠 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [c3f0a63](https://github.com/leptoskit/leptoskit/commit/c3f0a63a4bb79a192f4abb5aabdb45dcfe308222)
- **Add `lk cloud` account commands: device login, private rotating credentials, organizations and members, projects and environments, scoped API tokens, and project linking.** Every command supports JSON output. Deployment commands keep their existing behavior. _Packages:_ 🟠 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [5e39638](https://github.com/leptoskit/leptoskit/commit/5e396387225c6d0f53e7762277214b6e54998208)
- **`lk cloud` manages an environment's variables, secrets, domains, deployments, rollback and database, publishes to stores from the cloud with `lk cloud publish`, and shows usage.** A CI token from `LEPTOSKIT_CLOUD_TOKEN` now goes only to the hosted cloud or to an origin named by `--url` or `LEPTOSKIT_CLOUD_URL`. _Packages:_ 🟠 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [b89a935](https://github.com/leptoskit/leptoskit/commit/b89a935c46194ab9626490fc8a7e09f11d7a63fc)
- **New database and sign-in apps start with a real notes screen.** A `leptoskit new --db sqlite`, `--db postgres`, or `--auth` project now opens on a notes screen instead of the counter. It uses the shared typed `Notes` and `AddNote` operations the scaffold already publishes: loading, empty, failed, and loaded states, body validation before the mutation, save feedback, and a Refresh button. The browser follows the live socket; native apps use the HTTP plugin and Refresh because their generated hosts don't grant the WebSocket plugin, and the Linux, Windows, and terminal hosts say plainly that they can't reach the server. With `--auth`, the screen is rebuilt per account and the previous account's cached notes are dropped first, so signing out or signing in as someone else never shows the previous person's list. `--db` now also grants the native HTTP plugin and configures its origin. Projects without a database keep the counter. _Packages:_ 🟠 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [1bd0618](https://github.com/leptoskit/leptoskit/commit/1bd0618c284e4dff885524d3d6a23db9cfc668f8)
- **Opt-in APNs for generated iOS apps.** `[native.ios] aps-environment = "development"` (or `"production"`) writes the `aps-environment` entitlement — Apple's exact string, not a boolean — into `.leptoskit/ios/App.entitlements`, and the generated app delegate forwards `didRegisterForRemoteNotificationsWithDeviceToken` and `didFailToRegisterForRemoteNotificationsWithError` to the notifications plugin's broker. It must match the app's provisioning profile and the notifications service's `LEPTOSKIT_NOTIFICATIONS_APNS_ENVIRONMENT`. Apps without the setting generate exactly as before, with no push entitlement, so local-only notifications users stay local-only; no provisioning or certificate access happens in the build. The configuration schema carries the new key. _Packages:_ 🟠 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)
- **Opt-in APNs for generated macOS apps.** `[native.macos] aps-environment = "development"` (or `"production"`) writes the `com.apple.developer.aps-environment` entitlement — Apple's macOS key, its exact string — into `.leptoskit/macos/App.entitlements`; the local build's ad-hoc signature and a `leptoskit publish macos` distribution both sign the app with it, and the generated macOS app delegate forwards `didRegisterForRemoteNotificationsWithDeviceToken` and `didFailToRegisterForRemoteNotificationsWithError` to the notifications plugin's broker. It must match the notifications service's `LEPTOSKIT_NOTIFICATIONS_APNS_ENVIRONMENT`. Apps without the setting generate exactly as before, with no push entitlement, so local-only notifications users stay local-only. _Packages:_ 🟠 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)
- **`leptoskit dev` rebuilds an edit with a line-table dev profile, compiles the server and the wasm client in parallel when the edit needs both, and copies assets without invoking cargo.** `leptoskit budgets` runs the caps and boot checks and names the build, test, lint, and workspace commands. _Packages:_ 🟠 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [1f7f146](https://github.com/leptoskit/leptoskit/commit/1f7f1462a17a9a33eccb933909f6cdae20d03052)
- **New apps use the file router by default.** `leptoskit new` enables the `router` plugin, scaffolds `index.rs` / `_layout.rs` / `+not-found.rs` under the app crate's `src/routes/`, commits a generated `routes.rs`, and depends on `leptoskit_router`. Builder `[lint]` stays the default so the starter `Page` and `Layout` components stay clean under `leptoskit lint --deny-warnings`. _Packages:_ 🟠 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [a8d02ab](https://github.com/leptoskit/leptoskit/commit/a8d02abfb577de8ac09d481ee5844814147fc6aa)
- **`leptoskit new` opens on a home screen, a kit of the shared controls, and a list of the platforms the app runs on, in a tab bar.** The home screen still counts with **Add one**. _Packages:_ 🟠 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [a7b4d40](https://github.com/leptoskit/leptoskit/commit/a7b4d400154fef9c3dcc13e434766a66240c90b2)

#### Release independently built workspace applications

_Packages:_ 🟠 _leptoskit_cli_

_Owner:_ Ifiok Jr. · _Introduced in:_ [b47bc2e](https://github.com/leptoskit/leptoskit/commit/b47bc2e41920c89e7382665a7fccab1c9508de9d)

Add `workspace plan` and `workspace deploy` for a set of prebuilt, independently pinned image descriptors. Plans freeze the workspace, parsed app configurations, artifact pins, readiness order, and credential variable references. Execution stages exact descriptors, invokes the existing managed release checks in each app directory, and retains a durable progress journal when a later app fails.

Add `deploy --reuse-current` to revalidate a current verified release without booting it again. Reuse checks current authorization, descriptor identity, recorded configuration, database history, image identity, running-container membership, effective app environment, and `/readyz`. Older history without configuration evidence requires a new version. Changed credentials fail reuse rather than being silently ignored. Successful release operation state is now written after backup installation and pruning finish.

Workspace retries revalidate every selected app, including previously completed entries; the journal never authorizes reuse. Automatic builds, source-change impact, private networking, caller identity, and cross-application compatibility remain separate work.

### Fixes

- **The analytics example's page responds again.** The analytics example's buttons did nothing, and its counts never updated, because the page stopped while starting in the browser. It now starts the browser's task runner before connecting to the analytics service, so the Buy and Share buttons record events and the summary updates live. Each event's row now shows its new count when it changes, instead of the first count it showed. _Owner:_ Ifiok Jr. · _Introduced in:_ [3b98ee7](https://github.com/leptoskit/leptoskit/commit/3b98ee70aaa253a29593865c3f9e8529c40110c9)
- **Bound analytics batch decoding and extreme clocks.** Analytics ingestion now limits the memory used when decoding binary batches, including repeated dictionary references, before allocating strings or collections. Extreme client clock offsets are clamped instead of overflowing timestamp arithmetic. The client core is also included in the regular Wasm lint checks. _Owner:_ Ifiok Jr. · _Introduced in:_ [3d0bdbd](https://github.com/leptoskit/leptoskit/commit/3d0bdbd19985c60228978dae8a0c7dcd2a6973d8)
- **Keep native analytics queues intact when a save fails.** Native analytics saves new queue files before atomically committing their checkpoint. Failed writes keep the previous queue and retry later, and concurrent flushes serialize their saves. Restoring into a smaller queue keeps each surviving event's context. Starting while opted out commits an empty checkpoint before opt-in can expose old files again. _Owner:_ Ifiok Jr. · _Introduced in:_ [deec4c0](https://github.com/leptoskit/leptoskit/commit/deec4c016022e16f3776ea5fc0059d5d84eeabbc)

#### Clarify verification coverage

_Owner:_ Ifiok Jr. · _Introduced in:_ [761e5c8](https://github.com/leptoskit/leptoskit/commit/761e5c8b7294232d16751af87e704ab2b6c943b1)

The maintainer verification plan now includes the implemented runtime image, website, approval and RPC compatibility paths. A consolidated audit record separates those checks from remaining platform, online migration, provenance and failover work.

#### Check auth module integration before writing files

_Owner:_ Ifiok Jr. · _Introduced in:_ [9586259](https://github.com/leptoskit/leptoskit/commit/95862598fd7df4522441ae8987a94f4a5785941d)

`lk generate auth` parses client and server roots before generation. Existing module declarations with comments, unusual spacing, or visibility no longer receive a duplicate declaration. Inline modules, redirected module paths, duplicate declarations, and a conflicting server module directory fail before screens or configuration are written, even with `--force`.

Setup reminders recognize direct function calls instead of comments, strings, or function references. Providing auth without gating screens keeps the root integration reminder. These reminders do not verify startup behavior, routing, or database authorization; existing apps still need the printed integration steps and runtime checks.

#### Sign-in email picks up six SMTP security fixes

_Owner:_ Ifiok Jr. · _Introduced in:_ [5116559](https://github.com/leptoskit/leptoskit/commit/511655984e342d14b77a57e5322ab9fdbd293ce8)

`leptoskit_auth`'s `smtp` feature sends sign-in email through `lettre`, which published fixes for six advisories (RUSTSEC-2026-0560 to RUSTSEC-2026-0565): a man-in-the-middle forging replies on a `STARTTLS` connection, command injection through a bare line break in a message, a message for an address with a quoted local part going to a different address, and three ways a malicious server or a crafted display name could crash or exhaust the sender. The locked version moves from 0.11.23 to 0.11.24. Nothing changes in how you configure or call it.

If your app has its own lockfile, run `cargo update -p lettre` to take the fixed release.

#### Find auth targets through Cargo

_Owner:_ Ifiok Jr. · _Introduced in:_ [9586259](https://github.com/leptoskit/leptoskit/commit/95862598fd7df4522441ae8987a94f4a5785941d)

`lk generate auth` resolves client and backend packages through Cargo metadata. Nested app configurations, workspace globs, inherited dependencies, and custom library or binary source paths now receive modules beside the selected target. The server embeds the selected app configuration and resolves its paths relative to that app, even when its Cargo package lives elsewhere in the workspace.

Ambiguous binaries, missing dependencies, renamed toolkit imports, and source targets outside the workspace fail before generation writes files. An explicitly disabled backend is left alone. Existing application roots and database policies still need the integration steps printed by the command.

When the client and backend share a package, the command explains how to enable `leptoskit/server` for the backend build without adding server dependencies to browser or guest builds.

- **Server rendering can spawn tasks.** `leptoskit::server::serve_listener_with`, and with it `serve_listener`, `serve_router`, and `serve_router_with`, now makes tokio the global `any_spawner` executor when it starts serving, unless the app already chose one. Before, a server-rendered page that used `leptoskit::query`, a leptos resource, or anything else that spawns a task panicked with "tried to spawn a Future with Executor::spawn() before a global executor was initialized", unless the app knew to call `Executor::init_tokio()` itself. _Owner:_ Ifiok Jr. · _Introduced in:_ [ab557f9](https://github.com/leptoskit/leptoskit/commit/ab557f9fdf7ae39f376f76bf6b00bcce950b89f7)
- **Reject future backup timestamps when reporting health.** Managed backup status no longer reports a verification timestamp in the future as healthy. A successful restore check must have a nonnegative age within the configured freshness window, and any later recorded backup failure still makes the status unhealthy. _Owner:_ Ifiok Jr. · _Introduced in:_ [8051706](https://github.com/leptoskit/leptoskit/commit/8051706d7c9f9c655f5dc639b8acff2da3c2d47c)

#### Install valid scheduled backup units

_Owner:_ Ifiok Jr. · _Introduced in:_ [8051706](https://github.com/leptoskit/leptoskit/commit/8051706d7c9f9c655f5dc639b8acff2da3c2d47c)

Managed backups now generate separate systemd directives for the recovery command, writable state paths, and timer installation. Previously, malformed newline escapes joined these settings to preceding lines and could prevent scheduled backups from running. The host verifies both generated units before initializing the backup repository or enabling the timer.

After upgrading, run `lk infra backups install` for existing managed deployments to replace their generated units.

- **Fix Commonplace's browser extension build.** Commonplace builds its browser extension again, including the clipper and project selection. The extension paths now import the signal trait used to update their store fields. _Owner:_ Ifiok Jr. · _Introduced in:_ [d9a66d1](https://github.com/leptoskit/leptoskit/commit/d9a66d1e73141c14be24d3e57bcd4f1cf600bdca)
- **Local deployment services ignore proxy settings.** `lk console`, `lk deploy`, `lk infra plan`/`apply`, and the hosted worker now connect straight to a deployment service on `localhost`, `127.0.0.1`, or `[::1]` even when `ALL_PROXY`, `HTTPS_PROXY`, or `HTTP_PROXY` is set. Before, every request went to the proxy, which can't reach your own machine, and failed with "deployment service request failed". Remote HTTPS services still use your proxy. _Owner:_ Ifiok Jr. · _Introduced in:_ [1ce9e6e](https://github.com/leptoskit/leptoskit/commit/1ce9e6ec64cead13b6d9e11c117967afbb0e133f)

#### Consolidation preserves kept migrations

_Owner:_ Ifiok Jr. · _Introduced in:_ [e25ffdd](https://github.com/leptoskit/leptoskit/commit/e25ffdd4dffaa4c0d3b982d080b605a13d3fed5f)

`db migration consolidate --keep` now folds only the schema changes before the kept migrations. Previously, it included their changes in the consolidated migration as well, so applying the resulting history could try to add the same column twice. Keeping every draft leaves history unchanged, and an oversized keep list reports an error instead of panicking.

- **Generated deploy images install esbuild.** The Dockerfiles `leptoskit deploy` and `leptoskit infra` generate now install esbuild next to binaryen, pinned to the same release the development environment provides, so a release web client builds inside the image and minifies its JavaScript instead of stopping at `could not run esbuild`. _Owner:_ Ifiok Jr. · _Introduced in:_ [f9efacb](https://github.com/leptoskit/leptoskit/commit/f9efacb92e078db52491e3c56a4fbe817b6cebf8)
- **`leptoskit dev` stops rebuilding on its own build output.** The dev watcher now ignores everything under the project's `target/` directory and resolves the paths it matches against — through symlinks and `..` — to the form filesystem events actually use. Before, the wasm client's own development target directory (`target/leptoskit-dev-wasm/`, which a workspace member's cargo target directory doesn't cover) was watched like source, so every rebuild rewrote `analytics_app.wasm` under it, woke the watcher, and started the next rebuild: `lk dev` on an app with a service plugin rebuilt in a loop with no edits at all. Edits to sources, manifests, and `leptoskit.toml` still rebuild, and apps whose target directory is a symlink (or chosen relative to the workspace) no longer depend on the reported path matching the one events carry. _Owner:_ Ifiok Jr. · _Introduced in:_ [5b17524](https://github.com/leptoskit/leptoskit/commit/5b17524127661050f742b6518055eddf1c83a7ac)
- **Fix the Apple device-keys signing failure path to bridge a retained Core Foundation error explicitly before encoding its status.** This avoids a Swift compiler diagnostic failure when building native hosts that use device keys. _Owner:_ Ifiok Jr. · _Introduced in:_ [67165da](https://github.com/leptoskit/leptoskit/commit/67165da4842b729de1ea43dee531c94d613df217)
- **Scripts for your own docs components.** `leptoskit_docs::Site::script` adds a script from the book to every page, loaded with `defer` after the built-in one, for components that need behaviour. _Owner:_ Ifiok Jr. · _Introduced in:_ [dfd538f](https://github.com/leptoskit/leptoskit/commit/dfd538f7f7e87c872b5262b75f32935624d5d72e)

#### Explicit backend selection and readiness probes

_Owner:_ Ifiok Jr. · _Introduced in:_ [7fa1b20](https://github.com/leptoskit/leptoskit/commit/7fa1b202bb1004e7a49035f21f7f86fb38737efc)

`build.backend` explicitly selects an API backend or a static app. Build, serve and deployment generation honor it; CSR apps no longer need a separate client package to deploy a backend. Without an explicit choice, Cargo binary targets determine backend presence.

`serve.probes` declares the `/healthz` and `/readyz` lifecycle instead of guessing from words in Rust source. New projects enable it. Existing managed apps must implement the readiness routes and set `serve.probes = true` before deploying. The declaration is checked by deployment health probes at runtime; it is not evidence that a server is healthy.

- **Browser extension port events compile for Wasm.** Give JavaScript object property reads a distinct internal name from serialized namespace reads, so extension port message and disconnect listeners compile alongside the developer-tools API. _Owner:_ Ifiok Jr. · _Introduced in:_ [1bd0618](https://github.com/leptoskit/leptoskit/commit/1bd0618c284e4dff885524d3d6a23db9cfc668f8)
- **Faster game frames.** The game host keeps one wgpu device for the whole run, stamps HUD text onto the picture instead of drawing one GPU shape per pixel, and fills CPU rows in spans. A missing adapter is remembered. The run report includes `frame_us`, the mean time of a frame. _Owner:_ Ifiok Jr. · _Introduced in:_ [a69f621](https://github.com/leptoskit/leptoskit/commit/a69f621d207cf33cd0b3791d6ba2627573a43723)

#### Check pack removal in generated apps

_Owner:_ Ifiok Jr. · _Introduced in:_ [02b3c60](https://github.com/leptoskit/leptoskit/commit/02b3c601d86a5d2d63431d6e0d962817aa03f9f0)

The required generated database journey now checks accidental and explicit schema-pack removal against real migration files and persisted notes. Python with SQLite is required rather than silently skipping the data check. Migration consolidation reports a missing pack before generic table drift so the required removal intent is clear.

- **Require generated application journeys before publication.** Publication now requires the database, auth and universal generated-app journeys on the validated release commit, before uploading artifacts or publishing crates. The same `devenv shell test:projects` command runs in PR/main CI and `test:all`. Missing tools, a missing individual test, and a failed journey stop the gate; Node-based client behavior cannot silently skip. _Owner:_ Ifiok Jr. · _Introduced in:_ [02b3c60](https://github.com/leptoskit/leptoskit/commit/02b3c601d86a5d2d63431d6e0d962817aa03f9f0)
- **Keep large graph inventories out of SSH replies.** Graph release preparation, activation and status return compact journal summaries. Complete recovery inventories stay on the host, so publishing another large static tree no longer overflows the bounded transport response and blocks retries. _Owner:_ Ifiok Jr. · _Introduced in:_ [7c788ae](https://github.com/leptoskit/leptoskit/commit/7c788ae0d8522dc8f0576961ef4bcb9121603f70)

#### Connect graph releases to checked executables

_Owner:_ Ifiok Jr. · _Introduced in:_ [7c788ae](https://github.com/leptoskit/leptoskit/commit/7c788ae0d8522dc8f0576961ef4bcb9121603f70)

Prepare graph service and SSR releases from their checked executable and asset inventories. Declare the backend's listen variable with `serve.address-env`; plans bind the exact backend, resolved service settings, Linux format and host account. Packaging reads the private stage without rebuilding. Generated host initialization creates dedicated service accounts.

Dependency observers, mounted executable probes, rootless engines, secret delivery and persistent service data still require further integration and fail explicitly. Graph integrity checks remain separate from source reproduction, release authorization and runtime attestation.

Release help names every artifact, trust and recovery input, and the published configuration schemas include the declared executable listen-address variable.

- **Recover independent application instances.** Connect the graph host installer to immutable executable releases, separate candidate ports, process-bound readiness, selected route promotion, boot enablement, retirement and rollback. Preserve unselected application instances and route bindings during a docs-only release. Record derived process and edge inputs before activation and reject later substitutions. Recovery keeps ownership until old processes and routes are ready, and tolerates an edge unit that was never installed. The CLI consumes checked executable contracts. Process readiness remains a local observation; real-host service isolation and reboot recovery require separate acceptance. _Owner:_ Ifiok Jr. · _Introduced in:_ [7c788ae](https://github.com/leptoskit/leptoskit/commit/7c788ae0d8522dc8f0576961ef4bcb9121603f70)
- **Check executable process readiness.** Add the Linux executable process adapter with explicit toolkit listen environments, immutable executable checks, isolated systemd instances and process-bound local readiness observations. Ship its source inside the pinned SSH worker. The CLI uses this adapter through checked executable contracts, with candidate routing and recovery handled by the graph release state machine. Live service isolation remains unverified. _Owner:_ Ifiok Jr. · _Introduced in:_ [7c788ae](https://github.com/leptoskit/leptoskit/commit/7c788ae0d8522dc8f0576961ef4bcb9121603f70)
- **Recover interrupted graph releases after a CLI upgrade.** Graph release apply, rollback and status accept `--recover-plan-sha256` with the independently retained digest of the original saved plan. Recovery rechecks the original artifacts, configuration, provider bindings and host trust while preserving the operation and host identities expected by the interrupted journal. The current worker restores legacy state before preparing a fresh transaction; changed intent and unrelated operations remain rejected. _Owner:_ Ifiok Jr. · _Introduced in:_ [7c788ae](https://github.com/leptoskit/leptoskit/commit/7c788ae0d8522dc8f0576961ef4bcb9121603f70)
- **Reject inherited routes that graph releases cannot deliver.** Graph releases now reject alongside-app `public` and `routes` settings inherited from a service package's metadata. Use the service's explicit graph domain and mount instead. Deployment no longer silently omits those inherited routing requirements. _Owner:_ Ifiok Jr. · _Introduced in:_ [7c788ae](https://github.com/leptoskit/leptoskit/commit/7c788ae0d8522dc8f0576961ef4bcb9121603f70)
- **Resume prepared rollbacks after upgrading.** Recover graph release journals written before explicit rollback intent and transaction IDs existed. Verify the original request and retained artifact pins before restoring its prior state, then prepare a fresh transaction from current routes. Interrupted recovery retains its newly recorded intent; rejected contexts and substituted files cannot cancel an existing candidate. Direct activation retries reuse the checked installed edge binary and do not require the caller to resend a payload. _Owner:_ Ifiok Jr. · _Introduced in:_ [7c788ae](https://github.com/leptoskit/leptoskit/commit/7c788ae0d8522dc8f0576961ef4bcb9121603f70)
- **Inspect graph executables before Linux deployment.** `lk verify graph --linux-architecture x86_64` (or `aarch64`) inspects selected executable formats without running them. It rejects foreign formats and architecture mismatches and reports ELF interpreters and library dependencies. This does not establish installed libraries, readiness, authorization, or graph deployment. _Owner:_ Ifiok Jr. · _Introduced in:_ [7c788ae](https://github.com/leptoskit/leptoskit/commit/7c788ae0d8522dc8f0576961ef4bcb9121603f70)
- **Recover releases created by the original static worker.** Interrupted static releases created before executable support can now recover after a CLI upgrade. Recovery checks their exact installed assets, Caddy and static configuration against the pinned historical host, without requiring metadata that the original worker never recorded. Executable journals retain their derived-state checks, and changed inputs fail before recovery changes runtime state. _Owner:_ Ifiok Jr. · _Introduced in:_ [7c788ae](https://github.com/leptoskit/leptoskit/commit/7c788ae0d8522dc8f0576961ef4bcb9121603f70)

#### Activate services on hosts without public routes

_Owner:_ Ifiok Jr. · _Introduced in:_ [7c788ae](https://github.com/leptoskit/leptoskit/commit/7c788ae0d8522dc8f0576961ef4bcb9121603f70)

Graph releases can activate executable services on a host with no public routes without supplying Caddy, TLS certificates, a tunnel or an edge account. Public hosts still require independently pinned Caddy. Mixed selections send the edge runtime only to public hosts. Private services retain process readiness, immutable artifact checks, isolated version history and rollback. Changing a host between public and private roles requires a separate migration; existing edge units are never silently disabled.

- **Preserve declared service listen variables.** Allow the graph process adapter to use an explicitly declared listen-address environment variable, including the playground's `PLAY_BIND`. Require its value to match the private loopback address instead of restricting services to toolkit-specific variable names. Executable release plans bind the declared address and account. Real-host service acceptance remains separate from static release acceptance. _Owner:_ Ifiok Jr. · _Introduced in:_ [7c788ae](https://github.com/leptoskit/leptoskit/commit/7c788ae0d8522dc8f0576961ef4bcb9121603f70)

#### Provision declared graph hosts from a saved plan

_Owner:_ Ifiok Jr. · _Introduced in:_ [7c788ae](https://github.com/leptoskit/leptoskit/commit/7c788ae0d8522dc8f0576961ef4bcb9121603f70)

Graph projects now support `lk infra plan`, `apply`, and `output`. A saved plan binds the current configuration, generated provider and edge files, provider lock, backend identity, workspace, OpenTofu version and binary plan. Changed inputs require another plan. Failed replanning invalidates the old receipt, and applying consumes it before infrastructure changes begin.

Host and network replacement is refused. An optional compute budget totals all declared hosts, including dedicated service hosts. Output reads only named public host/domain values and distinguishes provider observations from runtime checks. Graph provisioning still does not install or activate application artifacts, edge runtimes or certificates.

- **Recover rejected static releases.** Fix the static release SSH command and enforce transfer limits while data arrives. Reject incomplete remote activation results, allow a corrected release after a candidate fails without changing live pointers, and verify restored routes before recording recovery. Enable the edge service for reboot and disable it when a failed initial release has no previous deployment. The static release, selected update and requested rollback path has real Linux/SSH acceptance. _Owner:_ Ifiok Jr. · _Introduced in:_ [7c788ae](https://github.com/leptoskit/leptoskit/commit/7c788ae0d8522dc8f0576961ef4bcb9121603f70)
- **Preserve unrelated routes during rollback.** Give every graph release transaction its own edge configuration so rolling back one service preserves other services' current bindings. Keep earlier transaction history and recheck preserved routes before promotion. Reject mismatched worker request contexts before cleanup can cancel an existing prepared release. _Owner:_ Ifiok Jr. · _Introduced in:_ [7c788ae](https://github.com/leptoskit/leptoskit/commit/7c788ae0d8522dc8f0576961ef4bcb9121603f70)
- **Report missing service metadata.** Return a service capture diagnostic when the selected or workspace manifest is absent from resolved Cargo metadata, before attempting to read its package version. This keeps malformed or inconsistent source resolution from panicking during graph builds. _Owner:_ Ifiok Jr. · _Introduced in:_ [7c788ae](https://github.com/leptoskit/leptoskit/commit/7c788ae0d8522dc8f0576961ef4bcb9121603f70)
- **Bind graph service receipts to resolved inputs.** Graph receipts capture resolved service settings and exact supported workspace or path build inputs, recheck them during verification and staging, and reject legacy service receipts or unsupported source evidence. Static receipts remain compatible. _Owner:_ Ifiok Jr. · _Introduced in:_ [7c788ae](https://github.com/leptoskit/leptoskit/commit/7c788ae0d8522dc8f0576961ef4bcb9121603f70)

#### Deliver declared literal settings to graph services

_Owner:_ Ifiok Jr. · _Introduced in:_ [7c788ae](https://github.com/leptoskit/leptoskit/commit/7c788ae0d8522dc8f0576961ef4bcb9121603f70)

Graph service capture and release now carry non-secret `env` settings from package metadata and project overrides into the process contract. Empty values, Unicode, spaces, quotes, backslashes, dollar signs and percent signs are preserved. Changes are bound to the captured configuration, saved release plan and installed process record, including rollback. A service cannot override its declared listen variable through `env`.

These values are public release inputs and appear in receipts and unit files. Secret delivery remains unsupported. Unsupported control characters and oversized environments fail before release writes rather than being silently changed or omitted.

- **Bind service paths to captured manifests.** Check that a path-based service's selected Cargo manifest is the manifest at its declared directory before normalizing portable evidence, and recheck that association when consuming a receipt. Bind the recorded lock to its workspace directory and require its local package/version entry. Replacing the selected manifest with another package's identical bytes no longer establishes the declared source identity. _Owner:_ Ifiok Jr. · _Introduced in:_ [7c788ae](https://github.com/leptoskit/leptoskit/commit/7c788ae0d8522dc8f0576961ef4bcb9121603f70)
- **Install static graph releases.** Add `lk infra release plan/apply/status/rollback` for selected static frontends on trusted Linux hosts. Check pinned artifacts, placement, SSH trust and runtime inputs before installing and probing candidates. Retain releases and journals for recovery and rollback. Service and SSR units use explicit executable contracts; unresolved readiness dependencies are rejected. Graph integrity receipts do not satisfy strict release authorization. The static release, selected update and requested rollback path has real Linux/SSH acceptance. _Owner:_ Ifiok Jr. · _Introduced in:_ [7c788ae](https://github.com/leptoskit/leptoskit/commit/7c788ae0d8522dc8f0576961ef4bcb9121603f70)
- **The hosted app's wasm clients leave out panic source locations and `Debug` text.** Those strings are different in every crate, so brotli barely shrinks them, and they had pushed the public site's initial load over its budget. `remember_keyed` is not used here; it tells call sites apart by those locations. _Owner:_ Ifiok Jr. · _Introduced in:_ [f5104c6](https://github.com/leptoskit/leptoskit/commit/f5104c6f382ac2e42b512fb160280ee8d375cb9e)
- **Merge accounts in apps with the identity provider.** An app that links `leptoskit_idp` can now merge two accounts and bring the other account's data along. Before, `idp.grants` declared no `on_account_merge` strategy, so `lk db check` failed and "bring its data" merges were refused. A merge now moves the absorbed account's grants to the account that stays. If both accounts granted the same app, the surviving account keeps its own grant and the absorbed account's grant is removed along with that account. _Owner:_ Ifiok Jr. · _Introduced in:_ [605721a](https://github.com/leptoskit/leptoskit/commit/605721a51568f5c50f0cc9b78f2f9506457c568d)
- **Job workers wake when the change listener starts.** Starting job workers now wakes them once their database change listener is ready. A job enqueued between a worker's first queue scan and listener startup no longer waits for the next polling interval. _Owner:_ Ifiok Jr. · _Introduced in:_ [c023c0c](https://github.com/leptoskit/leptoskit/commit/c023c0c61388cca3291b4ef46460c0cc524461cb)
- **Workers claim only the jobs they can run.** A server's job workers now take only jobs its own build declares. Before, during a rolling deploy, an old server could pick up a job the new build had just added, fail it with "no job is named …", and spend one of its attempts each time, until it gave up for good. Now the job waits for a server that knows it. A job no running build declares any more stays pending, where `leptoskit jobs list --state pending` shows it and `leptoskit jobs cancel` clears it. _Owner:_ Ifiok Jr. · _Introduced in:_ [b77a6df](https://github.com/leptoskit/leptoskit/commit/b77a6df7ae6819d901ab3ee0abc001d8221581c4)

#### Kubernetes manifests that start

_Owner:_ Ifiok Jr. · _Introduced in:_ [8a09552](https://github.com/leptoskit/leptoskit/commit/8a0955226b2d83b2d4613f876baacd3ff7b4ef0f)

`leptoskit generate deploy --target kubernetes` now writes manifests a cluster accepts and runs:

- **Names:** Deployments, Services, Secrets, claims, and labels use the app's name with `_` as `-`, so an app called `my_app` gets `my-app`, a valid Kubernetes and DNS name. The image keeps its name.
- **The user:** each pod runs as the image's `app` user by number (`runAsUser`, `runAsGroup`, and `fsGroup` 10001). Before, the kubelet refused to start every container, because `runAsNonRoot` can't check a user named only by name. Volumes are now writable to that user on any storage class. The generated `Dockerfile` creates the `app` group with gid 10001 itself, rather than leaving the number to `useradd`.
- **Services without an app database:** a service with `database = true` in an app that has no `[db]` gets its SQLite file on a volume of its own under Compose and Kubernetes, with one replica, as it does when the app is on SQLite. Before, the file went on the read-only root filesystem, and Kubernetes gave the service no `DATABASE_URL`.

The analytics example's server now answers `/healthz` and `/readyz`, which every generated health check calls.

- **The live example on leptoskit.dev answers sooner.** The landing page's live example loaded the whole hosted app's Wasm client, management console included, before its button worked. The console now loads as a lazy chunk of its own, the client builds with `optimize = "smallest"`, and the page starts the client, its glue, and the Rive runtime downloading together instead of one after another. The example's module drops from 370 KiB to 188 KiB gzip-compressed, and on a phone emulated with Lighthouse's slow 4G and 4x CPU slowdown its first press updates the count about half as long after the example scrolls into view. The hosted console loads the same module and then its chunk, and says the app could not load if the chunk never arrives. _Owner:_ Ifiok Jr. · _Introduced in:_ [fd63890](https://github.com/leptoskit/leptoskit/commit/fd638905f5a37ce57d7f0783425b933f61f915aa)
- **The observatory and postcard examples open on Linux.** `cargo run -p observatory_example` and `cargo run -p postcard_example` now open a real GTK 4 window on Linux, as they already did with AppKit on macOS. Their `snapshot` command renders the window with GTK's own renderer to a PNG. Elsewhere they still print the screen's accessibility tree instead. _Owner:_ Ifiok Jr. · _Introduced in:_ [87c52f1](https://github.com/leptoskit/leptoskit/commit/87c52f1484be8e41c3b845046a8f88f6b2c07f0c)
- **Commonplace and Huddle run on Linux.** The Commonplace and Huddle examples now target Linux too. `leptoskit build --platform linux` builds each one's GTK app from its new `linux/` runner. The apps reach their server through the HTTP plugin and keep the session in the desktop's keyring, and Huddle's chat goes live over the WebSocket plugin. Along with file_routes, observatory, postcard, and the gallery, every example with a native app now runs on Linux. _Owner:_ Ifiok Jr. · _Introduced in:_ [794f7ad](https://github.com/leptoskit/leptoskit/commit/794f7adbfbe817884ae1c4770fbd24f62f3da16a)
- **Live updates resume when a slow reader catches up.** A live connection retains its unfinished transition and resumes it when transport capacity becomes available, even without another database write. Later changes coalesce into one current snapshot. Large transitions continue from their remaining chunks, and removed or replaced subscriptions do not receive unsent results from their earlier registration. _Owner:_ Ifiok Jr. · _Introduced in:_ [8cd9cd2](https://github.com/leptoskit/leptoskit/commit/8cd9cd2a78071621ae36334dc8c083a7bb1f8d6d)
- **Keep pending live query results bounded.** The live hub keeps one pending batch per connection instead of retaining every database round while a session is busy. Repeated changes retain the latest complete value or error for each subscription, including identity checks that must happen before delivery. Removing or replacing a subscription also removes its queued result. Live queries still deliver current state, not an event for every intermediate write. _Owner:_ Ifiok Jr. · _Introduced in:_ [560060b](https://github.com/leptoskit/leptoskit/commit/560060b880392234148d900562c69f934e36309b)

#### Bound pending live action and stream replies

_Owner:_ Ifiok Jr. · _Introduced in:_ [560060b](https://github.com/leptoskit/leptoskit/commit/560060b880392234148d900562c69f934e36309b)

Live actions keep their admission slot until the session handles their reply, so a busy connection cannot accumulate unlimited completed actions. Stream producers wait for bounded output capacity even when a client renews credits early; ordered items and terminal errors are retained. An outgoing stream frame larger than 1 MiB ends that stream with a 413 error.

Stream flushing during reauthentication and server drain also respects the connection's stall timeout.

Existing generated and handwritten stream pumps keep using the synchronous `StreamSink::fail` helper. The session waits for the recorded terminal error before releasing the producer task.

- **Live recovery retains current query errors.** When a live query fails, a full recovery transition now repeats that failure instead of replacing it with an earlier successful value. Reauthentication failures also replace the stored result before any recovery snapshot is produced. A subsequent successful query restores the current value. _Owner:_ Ifiok Jr. · _Introduced in:_ [8cd9cd2](https://github.com/leptoskit/leptoskit/commit/8cd9cd2a78071621ae36334dc8c083a7bb1f8d6d)

#### Isolate reused live stream IDs

_Owner:_ Ifiok Jr. · _Introduced in:_ [560060b](https://github.com/leptoskit/leptoskit/commit/560060b880392234148d900562c69f934e36309b)

Cancelling or replacing a stream now discards its buffered items and terminal frame, so they cannot reach a later stream with the same client ID. Streams retain their admission slot until their terminal frame is consumed. Handwritten producers that return or panic also send a terminal response; panic details stay on the server.

Consuming a terminal frame also cancels its producer, so a handwritten producer cannot keep running after its stream is retired.

Dropping a session also cancels its owned stream producers, including when its task is cancelled before normal cleanup.

- **Finish split updates before advancing the resume cursor.** A live transition that needs several frames now advances its version only on the final frame. Reconnecting after an earlier frame replays the unfinished changes, and mutations continue waiting until the complete transition arrives. The frame format is unchanged. _Owner:_ Ifiok Jr. · _Introduced in:_ [8cd9cd2](https://github.com/leptoskit/leptoskit/commit/8cd9cd2a78071621ae36334dc8c083a7bb1f8d6d)
- **Package the logging crate before its registry release.** The logging crate no longer depends on its own unpublished version to enable its test capture sink. Run its capture and panic integration tests with `--features testing`; the workspace's all-feature test gate continues to run both. Production features are unchanged. _Owner:_ Ifiok Jr. · _Introduced in:_ [c5297c2](https://github.com/leptoskit/leptoskit/commit/c5297c213aadecdd0c597cff86e4884d69c5d679)

#### Deploy an app that lives inside a Cargo workspace

_Owner:_ Ifiok Jr. · _Introduced in:_ [7977a93](https://github.com/leptoskit/leptoskit/commit/7977a93f40a60ce0637fad1e9530775b425c7af2)

`leptoskit generate deploy` now finds the server and client packages through Cargo's workspace metadata, so an app folder with only `leptoskit.toml` and member crates works, including members matched by workspace globs. The generated container uses the Cargo workspace as its build context, builds from the app directory, and copies the workspace target binary, the app's `dist/`, and the nearest workspace or app toolchain file. Compose, Fly, Kubernetes, and managed infrastructure use the same context and `Dockerfile` path.

Inside a workspace, the container ignore policy is written as `Dockerfile.dockerignore` beside the app's `Dockerfile`, so Docker applies it to the workspace context instead of ignoring the app-root `.dockerignore`. It excludes nested build caches, databases, environment files, managed infrastructure plans and state, and Kamal secrets, keeps exclusions already in the workspace and app `.dockerignore` files, and never rewrites those files. Standalone apps keep their existing files and commands, custom generated files stay yours, and a path that would escape the workspace is refused before it becomes a build context.

Nested recipes also accept `LEPTOSKIT_CLI_PATH` as an explicit build argument for a CLI crate inside the workspace, and normalize the container target directory independently of host build settings.

- **Refresh opencode's generated routes.** The opencode example now includes the route-list helper emitted by the current router generator, so checking its committed routes no longer blocks browser and terminal builds. _Owner:_ Ifiok Jr. · _Introduced in:_ [d9a66d1](https://github.com/leptoskit/leptoskit/commit/d9a66d1e73141c14be24d3e57bcd4f1cf600bdca)
- **Publish the complete platform dependency set.** Home widgets and the experimental watch crates now participate in the release manifest. Watch packages release in lockstep with the toolkit, so published feature combinations can resolve their dependencies. _Owner:_ Ifiok Jr. · _Introduced in:_ [63c564b](https://github.com/leptoskit/leptoskit/commit/63c564bde19e982a356fdd433907fab851248315)

#### Keep platform integrations consistent

_Owner:_ Ifiok Jr. · _Introduced in:_ [63c564b](https://github.com/leptoskit/leptoskit/commit/63c564bde19e982a356fdd433907fab851248315)

New platform plugins and the service-worker crate now have shared summary sources for their README and Rust documentation. Formatting and generated documentation are synchronized after the plugin integrations.

Android plugin sources follow the repository's Kotlin formatting rules. Experimental Wear OS template classes now match their filenames; they remain host stubs.

- **Keep plugin package readme paths portable.** The web-auth and passkeys crates explicitly name their existing `readme.md` files. Cargo no longer adds an automatically detected `README.md` alongside the lowercase file, so their published archives extract on case-insensitive filesystems. _Owner:_ Ifiok Jr. · _Introduced in:_ [3624af9](https://github.com/leptoskit/leptoskit/commit/3624af92b00b5ffc4d96b92fb1bdd61beaecb724)
- **Idle time on Windows.** The power monitor plugin builds on Windows again. It read the time since the last keyboard or mouse input through the wrong `windows-sys` module, so apps that used it failed to compile for Windows. _Owner:_ Ifiok Jr. · _Introduced in:_ [fe172e5](https://github.com/leptoskit/leptoskit/commit/fe172e5be3ad0008d9d397a987cc5cb52867867a)
- **Prepare dependencies for offline compile checks.** `test:cargo` fetches the locked host dependencies before running tests. Restored build outputs alone do not provide every source archive needed by the offline compile-fail fixtures. A preparation failure now stops the gate before the test suites start. _Owner:_ Ifiok Jr. · _Introduced in:_ [c9ee8d4](https://github.com/leptoskit/leptoskit/commit/c9ee8d44ca774d0612bc6f802c2f9ad3edd17565)

#### Keep shared release history readable

_Owner:_ Ifiok Jr. · _Introduced in:_ [d930fcd](https://github.com/leptoskit/leptoskit/commit/d930fcd18c1be26693f8e2b75925e13483e16f05)

Managed deployment now checks the complete proposed release history against its 4 MiB read limit before booting applications or changing their database. Large verified descriptors and repeated evidence can otherwise make a successful deployment's history unreadable by the next release or recovery command. The final write is checked again; existing history is never silently truncated or discarded.

Pulled and running image references must contain exactly 64 lowercase hexadecimal SHA-256 characters after the configured repository name. A matching repository prefix alone no longer qualifies a malformed reference as a registry digest.

This guard prevents new unreadable history. It does not archive existing history, repair an already oversized file, or provide unlimited release retention.

#### Stop replication capture when returning to triggers

_Owner:_ Ifiok Jr. · _Introduced in:_ [d9a66d1](https://github.com/leptoskit/leptoskit/commit/d9a66d1e73141c14be24d3e57bcd4f1cf600bdca)

When a migration switches live capture from replication back to triggers, the worker now checks that its publication still exists before reading the slot. It leaves capture mode and removes the unused slot even on an idle database, so that slot stops retaining write-ahead log. This also works on Postgres 18, where an empty decoding read can succeed after the publication was removed.

- **Check the complete published RPC contract.** `rpc check` now detects changes to existing argument types and requiredness, referenced schemas, channel payloads and join keys, live paths, and error encodings. Current exclusions cannot erase operations from the previous published contract. Missing references and ambiguous operation names require review. The shared `ApiSpec::breaking_changes` comparison is available without the code generators feature for artifact tooling; it checks declarations and does not claim runtime or authorization compatibility. _Owner:_ Ifiok Jr. · _Introduced in:_ [fee9bbb](https://github.com/leptoskit/leptoskit/commit/fee9bbbba40da89036de6ac1d1d60f004080be59)
- **Validate RPC specifications through every JSON reader.** Deserializing `ApiSpec` with Serde now validates specification versions and migrates older nullable arguments in the same way as `ApiSpec::from_json`. Unsupported versions fail instead of bypassing the compatibility checks. Version 4 keeps required nullable arguments required. _Owner:_ Ifiok Jr. · _Introduced in:_ [0f79eb0](https://github.com/leptoskit/leptoskit/commit/0f79eb0eb9a42755251e52d5e8f18c446e453ed8)
- **Require generated client acceptance before publication.** Every supported SDK now runs its live-server journey on pull requests and before publication. Explicitly selected SDK tests fail when their language toolchain is missing, rather than reporting success without executing. Run the same serial acceptance suite locally with `devenv shell test:sdk`; the development environment includes its toolchains. _Owner:_ Ifiok Jr. · _Introduced in:_ [1f1aee1](https://github.com/leptoskit/leptoskit/commit/1f1aee14f8d30f080e906909133c6819a018f1ed)
- **Resume generated live clients from applied results.** TypeScript, Python, Dart, Go, Rust, and CLI clients keep their applied cursor when a server welcomes them. A reconnect interrupted before its recovered results arrive now resumes from the previous complete transition, and pending mutations cannot use the welcome's server version as evidence that their results are on screen. _Owner:_ Ifiok Jr. · _Introduced in:_ [8cd9cd2](https://github.com/leptoskit/leptoskit/commit/8cd9cd2a78071621ae36334dc8c083a7bb1f8d6d)

#### Entrances let go once they have played

_Owner:_ Ifiok Jr. · _Introduced in:_ [30c2ac0](https://github.com/leptoskit/leptoskit/commit/30c2ac07138beed6d403be7dd6cfdcc91c5a581f)

On the web, a motion that plays from a pose to the element's own style, whether it enters, scrolls into view (`in_view`), or follows a timeline, now lets go of the element once it has played instead of holding its last keyframe. A reveal that clips from an inset therefore no longer keeps clipping what the element paints outside its box, such as a heading's descenders. Keyframes that end on a pose of their own, at 1.0, still hold it.

The leptoskit.dev landing page and documentation get the same care:

- Section titles show their descenders, set display headings at a looser leading from shared tokens, and wrap each line of a title evenly.
- The hero's fan of screens no longer covers the copy on tablets and small laptops.
- The widget map fits a phone with reduced motion.
- The documentation's hero mark lands smoothly, and stays still for people who ask for less motion.
- The documentation's page titles no longer widen the page on a tablet, and its hero title keeps each sentence whole on a phone.
- Small coral text and the "Get started" button meet WCAG AA contrast in light and dark.

- **Bundled SQLite is 3.54.1.** `leptoskit_db`'s `sqlite` feature, `leptoskit_db_schema`, `leptoskit_deploy_console`, and `leptoskit_plugin_sqlite` bundle SQLite through `rusqlite`, which moves from 0.40.2 to 0.41.1 so that `libsqlite3-sys` moves from 0.38.2 to 0.39.1 and the bundled SQLite from 3.53.2 to 3.54.1. Before 3.54.0, SQLite could return uninitialized page-cache memory when a process opened a database alongside a `-wal` file written with a different page size (CVE-2026-119375, RUSTSEC-2026-0459). Nothing in leptoskit's own API changed; `rusqlite` stays an implementation detail. _Owner:_ Ifiok Jr. · _Introduced in:_ [39a130c](https://github.com/leptoskit/leptoskit/commit/39a130c13a7a7e6f51152f85105275f13425ad7f)
- **SQLite live queries hear other processes' writes.** On SQLite, a live query now updates within a quarter of a second when another process writes to the same database file, such as a second server process. Before, it heard only its own process's writes, and saw another process's writes only once it wrote something itself. Each database file gets one connection that checks for changes four times a second, and only once something subscribes to changes. Writes from tools that aren't leptoskit apps, such as the `sqlite3` shell, still reach subscribers with the app's next write. _Owner:_ Ifiok Jr. · _Introduced in:_ [b77a6df](https://github.com/leptoskit/leptoskit/commit/b77a6df7ae6819d901ab3ee0abc001d8221581c4)
- **Honor the selected application's frontend lint rules, exclusions and Clippy setting when linting.** Lint's partial configuration loader now follows `--app-target` and `app.default_target`, while still ignoring unrelated unfinished settings. Server-only targets keep the root lint configuration. _Owner:_ Ifiok Jr. · _Introduced in:_ [92ac609](https://github.com/leptoskit/leptoskit/commit/92ac609ece229043427c7edd7964a7ec7518696e)
- **Retry stalled TypeScript live connections.** Generated TypeScript RPC clients retry a connection if the WebSocket upgrade or the server's welcome does not finish within ten seconds. Subscriptions and pending writes survive the retry, and late callbacks from the abandoned socket cannot replace the new connection. Set `live.connectTimeoutMs` to allow a different connection deadline. _Owner:_ Ifiok Jr. · _Introduced in:_ [d9a66d1](https://github.com/leptoskit/leptoskit/commit/d9a66d1e73141c14be24d3e57bcd4f1cf600bdca)
- **Package reproduced backends and retain image evidence.** Add an opt-in backend-only image packaging command that reproduces independently pinned bundles, copies the verified executable without application compilation, checks extracted image bytes and configuration, and writes an operator-pinned descriptor. Managed releases retain this evidence across rollback and recovery. Website and service-bearing bundles remain unsupported. _Owner:_ Ifiok Jr. · _Introduced in:_ [f741989](https://github.com/leptoskit/leptoskit/commit/f7419898aaa71b3cdde3d6fb233504660e2789ce)
- **Exercise nonempty RPC image contracts.** The dependency-free `verified-backend` acceptance fixture now exports a query and a channel, so a real reproduction exercises a nonempty structural contract instead of an empty one. The acceptance driver derives its pinned baseline from that actual candidate export and rejects self-consistent baselines that remove or change a previously published function or channel. This is packaging and declaration evidence, not endpoint, cloud, provenance, or runtime-attestation proof. _Owner:_ Ifiok Jr. · _Introduced in:_ [e79e920](https://github.com/leptoskit/leptoskit/commit/e79e920916b4938cdb7b9ebf2786275f6e90c957)
- **Verified backends can use infrastructure services.** Verified backend packaging now distinguishes application services from the separate cache and relay containers configured under `infra.services`. Those infrastructure settings no longer incorrectly reject a backend-only image. Additional application executables still require their own evidence and remain unsupported by this packaging profile. The descriptor verifies the backend image; it does not attest the infrastructure containers or their runtime configuration. _Owner:_ Ifiok Jr. · _Introduced in:_ [63c564b](https://github.com/leptoskit/leptoskit/commit/63c564bde19e982a356fdd433907fab851248315)
- **Bind supported client contracts to verified releases.** `infra build-verified` can compare the reproduced executable's RPC export with independently pinned published contracts. Version 3 descriptors retain the exact specs and recompute the structural comparison when consumed. Version 2 operator trust files require a nonempty set of supported contract pins across deployment, reuse, rollback, and recovery; publishers cannot substitute their own baseline. Legacy evidence remains readable and explicitly unchecked. This checks declared wire shapes, not runtime behavior or authorization compatibility. _Owner:_ Ifiok Jr. · _Introduced in:_ [fee9bbb](https://github.com/leptoskit/leptoskit/commit/fee9bbbba40da89036de6ac1d1d60f004080be59)

#### Bind RPC evidence to the verified backend

_Owner:_ Ifiok Jr. · _Introduced in:_ [f741989](https://github.com/leptoskit/leptoskit/commit/f7419898aaa71b3cdde3d6fb233504660e2789ce)

Export RPC specifications from the checked executable under container confinement and hash the exact exported bytes. Record compatibility as not checked; require an explicit operator declaration for backends without RPC. Keep legacy releases from replacing verified history.

Hash executable bytes directly, preserving differences between non-UTF-8 bytes, and normalize Podman image IDs before recording descriptors.

Validate descriptor digests, pinned recipes and platform consistency before release. Reject history that discards verified evidence, and stop applications when automatic recovery fails verification.

- **Web builds ship the app's own Wasm client.** `leptoskit build` bound the first Wasm module cargo reported. When the client crate depended on another crate that also declares a `cdylib`, cargo compiled that dependency to Wasm first, and the build shipped the dependency's module under the app's name: the page loaded without an error, but the app's start function never ran. Web builds and native guest builds now take the module of the client package itself. This is what kept the live example on leptoskit.dev from responding to presses. _Owner:_ Ifiok Jr. · _Introduced in:_ [4f4f9b7](https://github.com/leptoskit/leptoskit/commit/4f4f9b7612429aa34c066847c012940e003ce60d)

#### Smaller server-rendered pages

_Packages:_ 🟢 _leptoskit_testing_library_, 🟢 _leptoskit_native_

_Owner:_ Ifiok Jr. · _Introduced in:_ [f8440e7](https://github.com/leptoskit/leptoskit/commit/f8440e78ff6fad3cf23ac1257bf146d31778eddd)

Server-rendered and static pages no longer repeat leptoskit's style reset for every differently styled element. Each element's typed styles start from about sixty declarations, such as `display:flex`, `flex-shrink:0`, and zeroed margins and padding, and every distinct look carried its own copy of all of them, about a kilobyte each. `leptoskit::web::render` now writes those defaults once per stylesheet, and each style scope's sheet holds only what differs from them, often a single declaration. Elements look the same: the defaults keep the specificity and order they had, below utility classes. A list of two dozen rows that differ only in width used to carry 27 copies of the reset and now carries one, and its markup is under 9 KB. Style scopes are also 16 hex digits instead of 64, since every element carries its scope.

The root element of each render now carries `data-lk-root`, which keeps one render's defaults from resetting the elements of a render earlier in the page. `leptoskit_testing_library` leaves it out of the markup it prints, as it does `data-lk-style`.

- **Build the public and hosted website clients with a size-optimized Rust standard library during release packaging.** This keeps the initial download within the existing 180 KiB budget while preserving the live example and the console's lazy loading. _Owner:_ Ifiok Jr. · _Introduced in:_ [7237130](https://github.com/leptoskit/leptoskit/commit/723713098b125431f6e762cbf4b6f3ebe5506a23)
- **Recognize Windows workspace paths in deployments.** Recognize Cargo workspace and build paths on Windows when generating deployments, including paths written without Rust's canonical Windows prefix. _Owner:_ Ifiok Jr. · _Introduced in:_ [d9a66d1](https://github.com/leptoskit/leptoskit/commit/d9a66d1e73141c14be24d3e57bcd4f1cf600bdca)

#### Compact layouts, fast Yoga, streaming diff & O(log N) hit

_Packages:_ 🟢 _leptoskit_yoga_, 🟢 _leptoskit_native_, 🟢 _leptoskit_bridge_ffi_

_Owner:_ Ifiok Jr. · _Introduced in:_ [3677b62](https://github.com/leptoskit/leptoskit/commit/3677b6245db20cc2387b3f547855364c319db735)

- Box cold `Style.grid`, `Element.gesture`, and `Element.bindings` fields, pack `Tailwind` into a single `Option<Arc<TailwindInner>>`, gate DevTools-only `#[track_caller]` behind `devtools`, and fuse `Element::validate()` into a single pass.
- Optimize `leptoskit_yoga` with contiguous flex-line ranges, empty/measured leaf fast paths, direct in-flow child iteration, inline `LayoutCache`, and `subtree_pass` dirty pruning.
- Stream `NativeSurface` reconciliation directly without cloning a resolved intermediate tree, prune clean subtrees in `collect_frames`/`place`, binary-search monotonic hit-test siblings in O(log N), switch `Flattener` to a dense zero-clone arena, and encode FFI mutation props by reference without cloning `NativeProps`.

#### Faster Wasm host, external measure & in-place guest tree

_Packages:_ 🟢 _leptoskit_yoga_, 🟢 _leptoskit_native_, 🟢 _leptoskit_wasm_

_Owner:_ Ifiok Jr. · _Introduced in:_ [53e3f0e](https://github.com/leptoskit/leptoskit/commit/53e3f0e757532c73ef87282497e75c28e7df0fde)

- Consolidate `leptoskit_wasm`'s `HostState` into a single `FastHash` node map with dense `wire_by_node` lookups, bool-deduped dirty lists, cached `TypedFunc` Wasm exports, and O(1) tail detachment in `HostState::detach`.
- Add `Yoga::set_measured` and `Yoga::calculate_layout*_with_measure` so `NativeSurface` measures text via an inline `Copy` `TextMeasureCache` reading `&shadow.props` directly instead of allocating per-node `Arc<RwLock<_>>` closures and cloning strings.
- Add `NativeBackend::apply_owned` so `HeadlessBackend` moves `MutationList` without cloning every `NativeProps`, share a static default `ApproxTextMeasurer`, and accelerate `LogBudget::admit` with exact fast-path JSON byte accounting.
- Reconcile `guest::tree::Tree::update` in place with a leaf fast path, immediate subtree eviction reusing a single DFS stack, and reverse build-order deletions (leaves and trailing siblings first).

- **Transparent scenes show the page behind them in the browser.** A scene with `Backdrop::Transparent` drew over solid black in the browser. wgpu lists only `Opaque` as a WebGPU canvas's alpha mode, though every browser canvas also takes `premultiplied`, so the canvas was set up opaque. Browser canvases now composite premultiplied, as native surfaces already did. A 3D view over a photo, a camera picture, or another 3D view shows them through its empty pixels. WebGL2, where wgpu refuses anything but opaque, is unchanged. _Packages:_ 🟢 _leptoskit_3d_ _Owner:_ Ifiok Jr. · _Introduced in:_ [33ad5e1](https://github.com/leptoskit/leptoskit/commit/33ad5e1171f97f4086bf6ec9290e75aef956cdc0)
- **Keep Android builds warning-free: omit the unused stderr fallback on Logcat, simplify pointer decisions, and account for Clippy's Android TLS false positive while retaining const initialization.** _Packages:_ 🟢 _leptoskit_game_, 🟢 _leptoskit_native_, 🟢 _leptoskit_android_, 🟢 _leptoskit_bridge_ffi_, 🟢 _leptoskit_log_ _Owner:_ Ifiok Jr. · _Introduced in:_ [7237130](https://github.com/leptoskit/leptoskit/commit/723713098b125431f6e762cbf4b6f3ebe5506a23)
- **Release the game host with the SDK.** The optional `leptoskit_game` crate joins the SDK's coordinated release group, so published apps using the `game` feature can resolve its matching version. _Packages:_ 🟢 _leptoskit_game_ _Owner:_ Ifiok Jr. · _Introduced in:_ [7237130](https://github.com/leptoskit/leptoskit/commit/723713098b125431f6e762cbf4b6f3ebe5506a23)

#### Plugin options the app would crash on stop the build instead

_Packages:_ 🟢 _leptoskit_xr_, 🟢 _leptoskit_bridge_ffi_, 🟢 _leptoskit_plugin_http_, 🟢 _leptoskit_plugin_websocket_, 🟢 _leptoskit_plugin_shell_, 🟢 _leptoskit_cli_

_Owner:_ Ifiok Jr. · _Introduced in:_ [da4763e](https://github.com/leptoskit/leptoskit/commit/da4763e983262b2144684984245a2bf27b5a61f5)

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

_Owner:_ Ifiok Jr. · _Introduced in:_ [736759c](https://github.com/leptoskit/leptoskit/commit/736759cc6082c21aa05025aab6823b785a0c3a21)

An Android app that also used Solana's clients drew nothing: its first vector failed in the Wasm host with `invalid type: map, expected f32`. Those clients turn on serde_json's `arbitrary_precision` for the whole build, and under it a buffered number with a fraction is a map. The host read a node's props through `#[serde(flatten)]`, which buffers every value, and so did each animation `Keyframe`.

The host now reads a node with `WireNode::from_json`: the props and the keys beside them in two passes, with nothing buffered, and `Keyframe` lists its look's `opacity` and `transform` itself. Its JSON is unchanged.

The guest and the host now share `leptoskit_native::wire`: one table of kind codes and one `WireNode`. That fixes two more ways the sides disagreed:

- The host's own kind table stopped at `Camera`, so a `Map` or a `LivePhoto` showed as a plain view.
- An image's `source` was taken by the DevTools location, which used the same key, so native images had no source. A `DevTools` guest now sends its location as `source_location`.

`test:wire` runs the guest-to-host tests under `arbitrary_precision` too.

- **Fix panic when shrinking children during reconcile.** - Slice added children from `common..new_len` instead of `old_len..new_len` in `NativeSurface::update_source_node` so removing children from an existing container no longer panics with an out-of-range slice index. _Packages:_ 🟢 _leptoskit_native_ _Owner:_ Ifiok Jr. · _Introduced in:_ [5d323bf](https://github.com/leptoskit/leptoskit/commit/5d323bfc6b15ae70a243fbe3d0d21bd6836d1765)
- **Hydrated web screens refresh their stylesheet when classes change.** Switching a theme or opening a panel could introduce new style scopes while leaving the initial CSS in place, so controls lost their layout and colours. The browser regression hydrates server HTML and changes padding in both directions. _Packages:_ 🟢 _leptoskit_native_ _Owner:_ Ifiok Jr. · _Introduced in:_ [9189d6a](https://github.com/leptoskit/leptoskit/commit/9189d6a89173a9ff009bf0a7d30b0639fd356fa9)

#### Regions committed together keep their own widgets

_Packages:_ 🟢 _leptoskit_native_, 🟢 _leptoskit_wasm_

_Owner:_ Ifiok Jr. · _Introduced in:_ [7e61e9b](https://github.com/leptoskit/leptoskit/commit/7e61e9bceefea4db7c8e3a70fdd31deb21597898)

`NativeSurface::try_patch` applies patches in tree order, and it answered in that order too, though its contract (and the wasm host, which pairs each answer with the patch it sent) is the order given. A commit that changed two regions whose wire ids ran against the tree, such as a label above a list that gained a row, paired one region's wire ids with the other's widgets. A debug host stopped with "surface and wire tree diverged"; a release host could send a widget's events to another node. On a Solana Seeker, buying anything in Reapers stopped the app.

`try_patch` now answers in the order given. The wasm host's incremental tests check, after every commit, that each widget pairs with the wire node it was built from.

#### Smaller web Wasm, lazy listeners & precompressed serving

_Packages:_ 🟢 _leptoskit_native_, 🟢 _leptoskit_axum_, 🟢 _leptoskit_cli_

_Owner:_ Ifiok Jr. · _Introduced in:_ [4001dbb](https://github.com/leptoskit/leptoskit/commit/4001dbb8ed91ecbdcb3caffdc86690b17607ba9c)

- Remove `sha2` from `leptoskit_native`'s `web` feature in favor of a deterministic 64-bit `ScopeHasher`, emit non-default base CSS style deltas directly with `styles::delta_for`, fuse tree pre-scans into one `scan_tree` pass, attach DOM event listeners lazily via `Either` only when handled, and make `NodePath` zero-sized without `devtools`.
- Inject `<link rel="modulepreload">` and `<link rel="preload" as="fetch" type="application/wasm" crossorigin>` hints into built `index.html` files and enable `--strip-producers --low-memory-unused` in `leptoskit_cli`'s `wasm-opt` pipeline.
- Negotiate precompressed `.br`/`.gz` `index.html` in `leptoskit_axum`'s `SpaFallback` and serve precompressed static site/docs assets with `Vary: accept-encoding` in `leptoskit_app`.

#### Binary LKW1 guest wire, smaller guests, and fast Tailwind

_Packages:_ 🟢 _leptoskit_native_, 🟢 _leptoskit_wasm_, 🟢 _leptoskit_cli_

_Owner:_ Ifiok Jr. · _Introduced in:_ [42c3217](https://github.com/leptoskit/leptoskit/commit/42c32178289561103166d9067be713926265efb2)

- Stream native guest trees to the host with a compact binary wire protocol (`LKW1`) into a reused staging arena instead of per-node JSON serialization, while keeping single-pass JSON decoding for legacy and JS guests.
- Strip dead browser `wasm-bindgen` custom sections and unreachable exports before `wasm-opt -Oz` in `leptoskit_cli`'s guest optimizer, reducing native guest `.wasm` sizes by ~45–52% raw (~29–35% brotli).
- Defer `For`/`Show` region evaluation until expansion (avoiding duplicate subtree construction) and replace heavy sorting networks in `Tailwind::parse` with a compact insertion sort and bitflag fast paths.

- **App shortcuts wait while a modal is open.** On the web, an app shortcut such as Escape or W no longer runs while a `Modal` is open, and it no longer takes the key from the modal. Escape now closes the modal, as it should. Before, an app with its own Escape shortcut kept the key, so the modal stayed open. Shortcuts work again as soon as the modal closes. _Packages:_ 🟢 _leptoskit_native_ _Owner:_ Ifiok Jr. · _Introduced in:_ [535aa35](https://github.com/leptoskit/leptoskit/commit/535aa353b341e4e4479d5c9ce19f2b407dbf7b55)

#### Keep focus across web rebuilds and inside modals

_Packages:_ 🟢 _leptoskit_native_, 🟢 _leptoskit_

_Owner:_ Ifiok Jr. · _Introduced in:_ [a1dd169](https://github.com/leptoskit/leptoskit/commit/a1dd169266d9fc0c2bc17705cdcc0afe846fb584)

On the web, a screen that rebuilt replaced every landmark (`main`, `nav`, `header`, `footer`, `aside`, `article`, `section`, `search`), every heading, list, paragraph and label with a new element, instead of updating the one already there. Anything inside went with it: a text field lost its focus and its selection, and a person typing was dropped back to the top of the page.

Those elements now update in place, as `div` and `span` always did, so focus stays where it was after any rebuild.

A rebuild also no longer takes a node's `id`, `role`, `aria-*` and `tabindex` attributes off and puts them back: it sets the ones whose value changed and removes the ones that are gone. Before, a focused element that is only focusable through `tabindex`, such as a dialog's heading, lost focus to the page on every rebuild.

A `Modal` on the web now wraps Tab: past its last control focus comes back to its first, and Shift+Tab past its first goes to its last, instead of leaving the page for the browser's toolbar.

- **The web launch screen lifts once the app is drawn.** The HTML splash that `[app.launch_screen]` injects into `index.html` used to stay over the app for good: nothing called its `leptoskitDismissSplash()`. The web renderer now fades it out once the app's first render is painted, as iOS and Android end theirs at the first frame. An app that calls `leptoskit::splash::hold()` keeps it until `leptoskit::splash::dismiss()`, which now lifts it on the web too. _Packages:_ 🟢 _leptoskit_native_ _Owner:_ Ifiok Jr. · _Introduced in:_ [44cfd11](https://github.com/leptoskit/leptoskit/commit/44cfd118a6ab5aa6e98eb641048fa307c91e47d2)
- **`leptoskit device open` fails when the app never takes the screen, instead of reporting success while the launcher is still in front.** A tap, long press, or pan past the display fails too: `input` exits 0 for those points and delivers nothing. An Android pinch still waits for its touchscreen to attach, but that wait is no longer part of the gesture clock, so a 300 ms pinch is 300 ms of contact with both pointers. `device list` marks an AVD as running when that emulator is already up. `leptoskit mcp http` answers `application/json`, and the screenshot tool's schema resolves. _Packages:_ 🟢 _leptoskit_testing_, 🟢 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [a9678a5](https://github.com/leptoskit/leptoskit/commit/a9678a57ec571f2318342fd1f04d68c9666b072d)
- **An Android pinch from `leptoskit device` is two pointers again.** Two `input swipe`s each own one pointer, and on API 35 the activity received only one of them while the step still reported success. The pinch now registers one touchscreen with `uinput` and moves both fingers in the same report. _Packages:_ 🟢 _leptoskit_testing_, 🟢 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [b969920](https://github.com/leptoskit/leptoskit/commit/b9699202035dc9c7245236c7d5e8750e816f3e2c)

#### Boot iOS for sanity checks and use a mouse on macOS

_Packages:_ 🟢 _leptoskit_testing_, 🟢 _leptoskit_cli_

_Owner:_ Ifiok Jr. · _Introduced in:_ [c5db144](https://github.com/leptoskit/leptoskit/commit/c5db144b468b11cafed744ffb4ac59356e8488d9) · _Last updated in:_ [b4efffc](https://github.com/leptoskit/leptoskit/commit/b4efffc4eb47c86b523454bfc2508c0fcd97a1e1)

`leptoskit device sanity` on an iOS simulator now boots that simulator before `simctl install`. A shut-down simulator refuses the install. One that is already booted stays up: the boot step treats that state as success. A sanity run that does not install, and a run on a real iPhone, still do not boot.

Pointer actions sent to a macOS session use a mouse. Mac2 does not accept a touch pointer. iOS, Android, Linux, Windows, and the web still send touch.

- **Connected iPhones show up in `device list`.** `leptoskit device list` asked `devicectl` for `--json` and parsed a `devices` array at the top of that output. devicectl does not print JSON that way. It writes a file for `--json-output`, and the phones are under `result.devices`. The id an install, a launch, and an XCUITest session need is the hardware UDID, not the Core Device identifier. A plugged-in iPhone never appeared. Watches and simulators in the same file stay out of the iPhone list. _Packages:_ 🟢 _leptoskit_testing_, 🟢 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [2ce47e0](https://github.com/leptoskit/leptoskit/commit/2ce47e01eedb1af4bc45cb05bca2d01b10c886ad)

#### Linux pointer gestures from `leptoskit device` no longer pause for about fifteen seconds when the pointer is already on the point.

`xdotool mousemove --sync` waits for a motion event that never arrives in that case, so a tap followed by a long press or a pan at the same point stalled. The move is sent without that wait.

_Packages:_ 🟢 _leptoskit_testing_, 🟢 _leptoskit_cli_

_Owner:_ Ifiok Jr. · _Introduced in:_ [86bb058](https://github.com/leptoskit/leptoskit/commit/86bb058f3c5c948f444fe75099ec639b02e7933d)

Opening a Linux app also waits until one of its windows is visible, the process exits, or two seconds pass. Gestures used to be injected the moment the process was spawned, which is before the window is mapped, so the tap and the long press landed on the desktop.

Opening an Android app waits until that package is the focused window, then a little longer, or two seconds pass. `am start` was returning while the launcher was still focused, and the focus line itself updates before the activity accepts a tap, so the sanity tap and long press never reached the activity.

`leptoskit device boot` of an AVD that is already running fails immediately and names the emulator serial, instead of starting a second emulator and waiting out the boot deadline. If the emulator process exits before the device is online, the boot fails then, instead of blocking on `adb wait-for-device`.

- **An unknown or custom widget is an empty container on Android, in both the backend and the generated app.** It used to stop the host, which closed the app. GTK already drew one as an empty view. _Packages:_ 🟢 _leptoskit_android_, 🟢 _leptoskit_bridge_ffi_, 🟢 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [a7b4d40](https://github.com/leptoskit/leptoskit/commit/a7b4d400154fef9c3dcc13e434766a66240c90b2)
- **On GTK 4.22, buttons, radios, switches, checkboxes, and drawings are registered with assistive technologies after the window is on screen.** That release could assign them a bus path before the accessibility connection was up and then never publish the object, so a screen reader saw the frame and the slider but not the named control. Right-to-left text fields on that release also keep logical start on the right. GTK 4.22 no longer mirrors `xalign` itself, so the older compensation was pinning the text to the left. _Packages:_ 🟢 _leptoskit_gtk_ _Owner:_ Ifiok Jr. · _Introduced in:_ [f8aeca8](https://github.com/leptoskit/leptoskit/commit/f8aeca89b200e4e20479d0ec2bfece3dcbc5c6e9)
- **Wait for composition before native pixel verification.** Native Windows widget fixtures disable transitions on their own window and wait for queued composition before reading screen pixels. Exact color tolerances and transparency assertions remain unchanged; production windows and desktop settings are unaffected. _Packages:_ 🟢 _leptoskit_windows_ _Owner:_ Ifiok Jr. · _Introduced in:_ [73b0e5b](https://github.com/leptoskit/leptoskit/commit/73b0e5b607b305f3d1f6e16e4a0f4bd43d7480c9)
- **Fix wearable wire codecs, watch plugins, and host builds.** Fixes cross-platform wire mismatches in Health (`ABSENT_LENGTH`, `ANDROID_TYPES` / `HARMONY_TYPES` bitmask preserving `HRV_RMSSD`, and exercise `Activity` codes 8..14), HarmonyOS Pedometer (`bounded_reply` 36-byte framing) and Battery charge state mapping, and BLE 16-bit/128-bit UUID normalization. Fixes Apple/Android runtime bugs across `ble`, `watch-sync`, `wearable`, and watchOS `haptics`, adds `Reading::Ecg` for `SensorMask::ECG`, fixes `RotaryState` wrap-around detent math, adds watchOS 10 support to `LeptosKitBridge` and wearable SwiftPM manifests, enables native plugin resolution and unified output paths for `wearos`, `watchos`, and `harmonyos` in `leptoskit_cli`, and adds the wearable crates to CI unit tests. _Packages:_ 🟢 _leptoskit_watch_, 🟢 _leptoskit_bridge_ffi_, 🟢 _leptoskit_plugin_haptics_, 🟢 _leptoskit_plugin_battery_, 🟢 _leptoskit_plugin_pedometer_, 🟢 _leptoskit_plugin_health_, 🟢 _leptoskit_plugin_wearable_, 🟢 _leptoskit_plugin_watch_sync_, 🟢 _leptoskit_plugin_ble_, 🟢 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [21a5f55](https://github.com/leptoskit/leptoskit/commit/21a5f55fea25e9c1f043da51aae77bcc6296aeb3)
- **Harden native-wave plugin correctness.** Live Activities on iOS cancel push-token waits without freezing the per-id busy lock, replace duplicate ActivityKit ids on start, and reject duplicate content keys. Picture-in-picture leases best-effort exit system PiP and observe user exit; `request_overlay` docs match launch-time status. Home widgets stop reporting unrelated pins for unknown kinds, commit preferences before success, and reject dangerous URL schemes. Device-policy and Live Activities stubs validate payloads before Unavailable. Wear/watchOS hosts return errors instead of panicking on viewport kind mismatch. New Android hosts declare `supportsPictureInPicture` on MainActivity. _Packages:_ 🟢 _leptoskit_watchos_, 🟢 _leptoskit_wearos_, 🟢 _leptoskit_plugin_home_widgets_, 🟢 _leptoskit_plugin_device_policy_, 🟢 _leptoskit_plugin_live_activities_, 🟢 _leptoskit_plugin_picture_in_picture_, 🟢 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [879db6b](https://github.com/leptoskit/leptoskit/commit/879db6b2334d0c5bd8519f9a26f7a3eb92fca7f0)
- **Compare the fleet's measurements.** `Window`, `HostSample` and `ExportHealth` derive `PartialEq`, so a page model that carries them can be compared. _Packages:_ 🟢 _leptoskit_apm_ _Owner:_ Ifiok Jr. · _Introduced in:_ [2a3d73d](https://github.com/leptoskit/leptoskit/commit/2a3d73dfcf478704fd3576501b7c98a8ca4eb7a5)
- **Preserve typed fallback context and queue bounds.** Keyed retries of unknown typed schemas retain their original context, UUID and field precedence. Entries expanded past a batch or queue byte limit are dropped and counted, and total queue accounting includes retained contexts. Catalogue validation also checks known vector element types recursively. _Packages:_ 🟢 _leptoskit_analytics_ _Owner:_ Ifiok Jr. · _Introduced in:_ [807eb2f](https://github.com/leptoskit/leptoskit/commit/807eb2f61bfa8b3bd57a680ebcbf6443c7746ce5)
- **Keep graph placement separate from application frontends.** Graph artifact placement uses `[infra.frontends.<name>]`. Application build frontends keep their existing top-level `[frontends.<name>]` contract. This avoids colliding configuration types when named application targets and deployment graphs are used in the same toolkit. Graph unit identifiers such as `frontends.docs` and all ownership, route and recovery checks are unchanged. _Packages:_ 🟢 _leptoskit_config_, 🟢 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [7c788ae](https://github.com/leptoskit/leptoskit/commit/7c788ae0d8522dc8f0576961ef4bcb9121603f70)
- **More ergonomic multi-frontend scaffold.** `--frontends` alone scaffolds landing, app, and admin. Ports are stable per kind. The product app under `apps/app` uses the workspace name as its Cargo package. Frontend names trim whitespace and accept ASCII case; `ApplicationKind` gains `FromStr`, `Display`, and helpers used by the scaffold. Each frontend app uses the same file-router defaults as a normal `leptoskit new`. _Packages:_ 🟢 _leptoskit_config_, 🟢 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [fa8c81a](https://github.com/leptoskit/leptoskit/commit/fa8c81afb142de65ec3b1f0908c787ac164cb306)

#### Accept managed registries with explicit ports

_Packages:_ 🟢 _leptoskit_config_, 🟢 _leptoskit_cli_

_Owner:_ Ifiok Jr. · _Introduced in:_ [b47bc2e](https://github.com/leptoskit/leptoskit/commit/b47bc2e41920c89e7382665a7fccab1c9508de9d)

Managed image configuration accepts a registry hostname with a numeric port and an image repository under that registry. Invalid ports, embedded credentials, paths in registry authorities, and tagged image repositories remain rejected.

Verified image packaging accepts `--service <infra.name>` to include Kamal’s required service label in the independently pinned image configuration. Reusing an image checks the label along with the rest of that configuration.

Deployment accepts Docker’s null defaults for absent Podman `Cmd`, `OnBuild`, and `Volumes` inspection fields while still requiring the exact content-addressed image ID. Non-null changes remain rejected.

- **Give the HarmonyOS host's line-height values named, explicit types so generated applications compile with the public API 12 ArkTS compiler.** _Packages:_ 🟢 _leptoskit_platform_harmonyos_, 🟢 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [7237130](https://github.com/leptoskit/leptoskit/commit/723713098b125431f6e762cbf4b6f3ebe5506a23)
- **HarmonyOS text uses a named line-height value instead of an inline object.** ArkTS treated `{ relative?: number }` as `any` and refused to compile the page. A relative height is still a multiple of the font size, and a fixed height is still in virtual pixels. _Packages:_ 🟢 _leptoskit_platform_harmonyos_ _Owner:_ Ifiok Jr. · _Introduced in:_ [f5104c6](https://github.com/leptoskit/leptoskit/commit/f5104c6f382ac2e42b512fb160280ee8d375cb9e)
- **Android native hosts link AAudio even when rustc omits it from its reported libraries.** HarmonyOS uses named border and accessibility interfaces so ArkTS can compile the host contract. _Packages:_ 🟢 _leptoskit_platform_harmonyos_, 🟢 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [6919585](https://github.com/leptoskit/leptoskit/commit/691958516d206c0fc9d4f3c879e8fd3b8982d4fa)
- **A drawing-scaled vector stroke follows the layout frame, not the view after it has been snapped out to whole pixels.** At 420 dpi a 10dp edge becomes 27px, and the stroke was a little wider than the layout asked for. `parse_and_launch` no longer opens an `intent:` URI that has no scheme. API 35's Settings activity is browsable, so an action-only Settings URI used to start it. The activity it does open still has to declare `CATEGORY_BROWSABLE` on the filter that matched. _Packages:_ 🟢 _leptoskit_bridge_ffi_, 🟢 _leptoskit_plugin_android_intent_ _Owner:_ Ifiok Jr. · _Introduced in:_ [8128505](https://github.com/leptoskit/leptoskit/commit/81285051e628dada5bf9819978e4d608c53b1fb5)
- **Respect fractional Android text width constraints.** Android text measurements now stay within the requested logical width after pixel rounding. This fixes labels reporting a width larger than their layout constraint by part of a physical pixel. _Packages:_ 🟢 _leptoskit_bridge_ffi_ _Owner:_ Ifiok Jr. · _Introduced in:_ [d9a66d1](https://github.com/leptoskit/leptoskit/commit/d9a66d1e73141c14be24d3e57bcd4f1cf600bdca)
- **Composite Android 3D scenes with the interface.** Android `Scene3d` views draw into transparent `TextureView` surfaces in the normal view hierarchy. Page backgrounds no longer cover the models, and controls remain above the scene and its camera picture. Changing between the camera and a transparent backdrop no longer removes and recreates the view's GPU surface. Rust releases the native window before Android releases its surface texture. _Packages:_ 🟢 _leptoskit_bridge_ffi_ _Owner:_ Ifiok Jr. · _Introduced in:_ [7237130](https://github.com/leptoskit/leptoskit/commit/723713098b125431f6e762cbf4b6f3ebe5506a23)

#### Android text wraps to the lines it needs

_Packages:_ 🟢 _leptoskit_bridge_ffi_

_Owner:_ Ifiok Jr. · _Introduced in:_ [978e9c5](https://github.com/leptoskit/leptoskit/commit/978e9c59a606318d3ac2e9fe3f203b46cd417d91)

Android measured any plain one-paragraph text with `BoringLayout`, which always lays out exactly one line, even when the text was wider than its box. Layout then gave a paragraph one line's height, and the `TextView` drew only its first line: every paragraph of Reapers on a Solana Seeker was clipped. Text now takes `BoringLayout` only when it fits, and wraps through `StaticLayout` otherwise, as `TextView` does. The narrowest box a text can take is its widest word, with one word to a line.

Text views now also break lines with the measurer's strategy (simple, no hyphenation) instead of Android's balanced default, so a view never needs a line more than its frame was measured to.

- **Expose the Apple development-session callback from the shared bridge so the development-client plugin can compile independently of the app host.** _Packages:_ 🟢 _leptoskit_bridge_ffi_, 🟢 _leptoskit_plugin_dev_client_ _Owner:_ Ifiok Jr. · _Introduced in:_ [7237130](https://github.com/leptoskit/leptoskit/commit/723713098b125431f6e762cbf4b6f3ebe5506a23)
- **Generated Apple apps follow light and dark.** The generated macOS and iOS apps now tell your screens when the system switches between light and dark, so `dark:` classes update without a resize. A screen that first lays out while the system is dark also starts dark; before, it stayed light until its window changed size, which a menu-bar app or a fixed-size window never does. _Packages:_ 🟢 _leptoskit_bridge_ffi_ _Owner:_ Ifiok Jr. · _Introduced in:_ [d16db79](https://github.com/leptoskit/leptoskit/commit/d16db794e0f05db8079965a183badd4ed3ba3644)

#### Apple plugin and JNI builds on current toolchains

_Packages:_ 🟢 _leptoskit_bridge_ffi_, 🟢 _leptoskit_plugin_contacts_, 🟢 _leptoskit_plugin_device_keys_, 🟢 _leptoskit_plugin_live_activities_, 🟢 _leptoskit_plugin_live_photo_, 🟢 _leptoskit_plugin_nfc_, 🟢 _leptoskit_plugin_print_, 🟢 _leptoskit_plugin_age_range_, 🟢 _leptoskit_plugin_app_integrity_, 🟢 _leptoskit_plugin_asset_, 🟢 _leptoskit_plugin_diagnostics_, 🟢 _leptoskit_plugin_blob_, 🟢 _leptoskit_plugin_background_task_, 🟢 _leptoskit_plugin_calendar_, 🟢 _leptoskit_plugin_camera_, 🟢 _leptoskit_plugin_picture_in_picture_, 🟢 _leptoskit_plugin_video_thumbnails_, 🟢 _leptoskit_plugin_sms_, 🟢 _leptoskit_plugin_mail_composer_, 🟢 _leptoskit_plugin_maps_, 🟢 _leptoskit_plugin_media_library_, 🟢 _leptoskit_plugin_web_browser_

_Owner:_ Ifiok Jr. · _Introduced in:_ [fdf5868](https://github.com/leptoskit/leptoskit/commit/fdf586892a1411704c419bfd5069014d4ce5aaa2)

The Apple plugin packages now build and test against the current Xcode SDK. Wire readers no longer declare a `bytes` method that shadows their storage, wire-case helpers have distinct names, calendar validation is shared, EventKit events with no identifier fail honestly, camera photo delegates are retained behind a lock, Security keys use the CF bridge, print uses the current AppKit HTML and PDF APIs, and ActivityKit code is iOS-only. Delegate conformances are main-actor isolated, and the tests import internal wire types with `@testable` and supply a real presentation owner.

The JNI smoke test now finds the JDK through `JAVA_HOME`, invokes the macOS SDK compiler through `xcrun`, and stubs the 3D, timer, and DevTools entry points that the application bridge links.

#### Native scenes load models on the first launch

_Packages:_ 🟢 _leptoskit_bridge_ffi_, 🟢 _leptoskit_router_codegen_, 🟢 _leptoskit_cli_

_Owner:_ Ifiok Jr. · _Introduced in:_ [7237130](https://github.com/leptoskit/leptoskit/commit/723713098b125431f6e762cbf4b6f3ebe5506a23)

Android, iOS and macOS install the scene asset reader before starting the guest. Models queued before the first native scene view exists now load from the app bundle on a cold launch.

Generated Axum route lists also return their owned paths directly, keeping generated apps free of redundant conversion warnings.

#### Handed-back calls reach the Rust plugin that serves them

_Packages:_ 🟢 _leptoskit_bridge_ffi_, 🟢 _leptoskit_wasm_

_Owner:_ Ifiok Jr. · _Introduced in:_ [5445246](https://github.com/leptoskit/leptoskit/commit/54452466966f79486f975a84eb139e11a5b0e8ae)

An `AppService` such as Solana Kit's wallet service drains the bridge itself and hands back every call it does not own. Those calls skipped the interpreter's timer and went to the platform, which has no timer plugin: on Android every guest `sleep` failed as `Unavailable` in any app with such a service, so nothing that waited on a timer ever ran. A handed-back call now passes the interpreter first, as a queued one always did.

`WasmHost::serve` and `WasmHost::served_plugins` name what the interpreter answers itself, and `App::rust_plugins` lists every plugin answered in Rust: the timer, the frame clock, inline, worker and async plugins. Tests on the guest thread's own step find every registered plugin by id and fail when a call for a Rust plugin reaches the platform's router, so a Rust plugin added later is held to the same rule.

#### A stopped Android app says why, readably

_Packages:_ 🟢 _leptoskit_bridge_ffi_

_Owner:_ Ifiok Jr. · _Introduced in:_ [068178d](https://github.com/leptoskit/leptoskit/commit/068178ddea3abda61c2b0733410dccdd704efa47)

When an Android app's guest stopped, leptoskit replaced it with a plain `TextView` holding the error. That view took the theme's text color, which in a light system theme is dark, so on an app with a dark background the message was nearly invisible: Reapers looked like a black screen on a Solana Seeker. An app whose plugins failed to start didn't get even that: its activity closed at launch.

Both now show `NativeFailureView`: white text on near-black (17:1), centred and selectable, covering the whole app, the same in either theme. An app that cannot start shows it instead of closing.

- **Android plugins compile against the public SDK again.** Picture-in-picture listens through AndroidX, which is the type the platform stubs actually ship, and still notices a plain activity leaving PiP when it resumes. The camera plugin passes the failure itself to `PluginException`. Device keys read the SPKI bytes from `PublicKey` instead of `ECKey`, which has none. Calendar, contacts, print, SMS, mail, the web browser, and the media library set a string slice's byte order on the `ByteBuffer` `slice()` returns, because Android's `limit` returns `Buffer`. A calendar event reserves room for both text lengths as well as the timestamps, and decoding a contact advances the caller's buffer so the next record in a list can be read. _Packages:_ 🟢 _leptoskit_plugin_contacts_, 🟢 _leptoskit_plugin_device_keys_, 🟢 _leptoskit_plugin_print_, 🟢 _leptoskit_plugin_calendar_, 🟢 _leptoskit_plugin_camera_, 🟢 _leptoskit_plugin_picture_in_picture_, 🟢 _leptoskit_plugin_sms_, 🟢 _leptoskit_plugin_mail_composer_, 🟢 _leptoskit_plugin_media_library_, 🟢 _leptoskit_plugin_web_browser_ _Owner:_ Ifiok Jr. · _Introduced in:_ [f33c376](https://github.com/leptoskit/leptoskit/commit/f33c3764e2a3836560d9e22f54b6be6595d8ba65)

#### Fix wearable plugin SwiftPM manifests and Android await

_Packages:_ 🟢 _leptoskit_plugin_health_, 🟢 _leptoskit_plugin_wearable_, 🟢 _leptoskit_plugin_watch_sync_, 🟢 _leptoskit_plugin_ble_

_Owner:_ Ifiok Jr. · _Introduced in:_ [69d0cb2](https://github.com/leptoskit/leptoskit/commit/69d0cb211dfc8d7ae4ee2be28cc8f476d491555b)

- Added missing `Package.swift` manifests to `leptoskit_plugin_ble`, `leptoskit_plugin_watch_sync`, and `leptoskit_plugin_wearable` so iOS, macOS, and watchOS hosts resolve their Swift packages.
- Fixed Swift 6 strict concurrency (`MainActor.assumeIsolated`), exclusivity in `WireReader`, and HealthKit `appleSleepingWristTemperature` switch expressions.
- Replaced external `kotlinx-coroutines-play-services` and `kotlinx-coroutines-guava` `.await()` calls in `WatchSyncPlugin.kt` and `WearablePlugin.kt` with self-contained `suspendCancellableCoroutine` helpers and moved `deferred.await()` outside `synchronized`.

- **Report HealthKit read permissions as unknown rather than deriving an exact read mask from write authorization.** An empty inexact mask must not prevent an app from reading records the owner has authorized. _Packages:_ 🟢 _leptoskit_plugin_health_ _Owner:_ Ifiok Jr. · _Introduced in:_ [ee9473c](https://github.com/leptoskit/leptoskit/commit/ee9473c2325dd58283fb6215310f114cd336bd1f)
- **Fix WireReader collision in WatchSyncPlugin.swift.** Renamed `WireReader.bytes` storage property to `buffer` to avoid a Swift redeclaration error with `mutating func bytes()`. _Packages:_ 🟢 _leptoskit_plugin_watch_sync_ _Owner:_ Ifiok Jr. · _Introduced in:_ [8646768](https://github.com/leptoskit/leptoskit/commit/8646768a82ccb661a821aa3d32a2973a77a30508)
- **Fix Swift 6 delegate isolation in BlePlugin.swift.** Adopted `@preconcurrency CBCentralManagerDelegate` and `@preconcurrency CBPeripheralDelegate` on `@MainActor BlePlugin` (whose central manager runs on `.main`) to resolve Swift 6 `sending` data-race errors on `CBPeripheral`. _Packages:_ 🟢 _leptoskit_plugin_ble_ _Owner:_ Ifiok Jr. · _Introduced in:_ [73b6c7e](https://github.com/leptoskit/leptoskit/commit/73b6c7ec3c8dd3f823034658eb34f4342a2dd580)
- **P3 plugin quality: fix background_task Apple Auto construction and honesty, asset Kotlin NUL parity, age_range docs, and tighter wire tests.** _Packages:_ 🟢 _leptoskit_plugin_age_range_, 🟢 _leptoskit_plugin_app_integrity_, 🟢 _leptoskit_plugin_asset_, 🟢 _leptoskit_plugin_background_task_ _Owner:_ Ifiok Jr. · _Introduced in:_ [e4c23c8](https://github.com/leptoskit/leptoskit/commit/e4c23c81dc4c8ad7032b6b7c1e257bbbeef6da83)
- **Blob and diagnostics MVPs now run in process memory / no-op allocators with STATUS probing, instead of Unavailable-only stubs.** _Packages:_ 🟢 _leptoskit_plugin_diagnostics_, 🟢 _leptoskit_plugin_blob_ _Owner:_ Ifiok Jr. · _Introduced in:_ [8108def](https://github.com/leptoskit/leptoskit/commit/8108def0b96f9241954831697a3b1267808784c7)
- **Harden keyboard shortcuts and menu accelerators.** Global shortcuts and menu items now share the same muda `CmdOrCtrl+…` accelerator bounds and conflict rules: chords are capped at 64 printable ASCII bytes, menus reject duplicate accelerators (including equivalent spellings after parse), and dropping a global-shortcut owner retries native unregister so failed RAII cleanup does not leave OS bindings behind. Closing the Apple or Android app-shortcuts plugin clears published launcher entries so they cannot outlive the bridge. _Packages:_ 🟢 _leptoskit_plugin_app_shortcuts_, 🟢 _leptoskit_plugin_menu_, 🟢 _leptoskit_plugin_global_shortcut_ _Owner:_ Ifiok Jr. · _Introduced in:_ [89b4fea](https://github.com/leptoskit/leptoskit/commit/89b4feab1dcdc4767bd4589186f22b7c4be384a7)
- **Clarify that buffered browser sounds report the decoded AudioContext sample rate, and verify their length and playback controls at both 44.1 kHz and 48 kHz.** _Packages:_ 🟢 _leptoskit_plugin_audio_ _Owner:_ Ifiok Jr. · _Introduced in:_ [7237130](https://github.com/leptoskit/leptoskit/commit/723713098b125431f6e762cbf4b6f3ebe5506a23)
- **The background plugin no longer crashes iOS apps.** iOS ends an app that registers a `BGTaskScheduler` handler after launch finishes, or registers the same one twice, and the `background` plugin registered its handlers in `start`, which the guest calls once the app is already running. The handlers are now registered by `BackgroundPlugin.registerLaunchHandlers()`, which the generated iOS app delegate calls from `application(_:willFinishLaunchingWithOptions:)`; repeated calls are ignored, and a task whose identifier or background mode is missing from the app's `Info.plist` is skipped instead of registered. `start` now only schedules the next app refresh. An app whose `hosts/ios/App.swift` was generated before this keeps launching, and its `start` answers `Unavailable` on iOS until it adds the call. _Packages:_ 🟢 _leptoskit_plugin_background_, 🟢 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [1c9b39d](https://github.com/leptoskit/leptoskit/commit/1c9b39d6edb307df0591ef530ac490b4576942e9)
- **Background-service checks see the running service.** On Android, the location and recorder plugins check that the `background` plugin's foreground service runs with the right type before they start in the background. They read that state by name, and it was never visible to them, so the check always failed and background location updates and background recording were refused even with the service running. The state is now a public field that release builds keep through R8. _Packages:_ 🟢 _leptoskit_plugin_background_ _Owner:_ Ifiok Jr. · _Introduced in:_ [a51c332](https://github.com/leptoskit/leptoskit/commit/a51c332f201e582ab58bcd8fb966cef5405442b9) · _Last updated in:_ [c4c8cef](https://github.com/leptoskit/leptoskit/commit/c4c8cef6212c4a7b4855dfff669ad58162f3db8e)
- **Android picture-in-picture builds against the public SDK.** Android PiP no longer calls `ActivityInfo.supportsPictureInPicture()`, which is `@hide` and missing from the public SDK stubs (so the generated app failed to compile). Capability checks use the system PiP feature plus an activity context; hosts that omit `android:supportsPictureInPicture` still fail closed when entering PiP. The PiP-mode listener imports `android.app.PictureInPictureModeChangedInfo` instead of the non-existent `android.util` type. _Packages:_ 🟢 _leptoskit_plugin_picture_in_picture_ _Owner:_ Ifiok Jr. · _Introduced in:_ [d25d5a1](https://github.com/leptoskit/leptoskit/commit/d25d5a1dff535c16cc8865c90f4169bc0f759805)
- **Harden desktop menu bars.** `leptoskit_plugin_menu` now rejects duplicate accelerator chords before native allocation, exposes `DesktopMenu::set_accelerator` and `definition()`, and shares a `root_items_are_submenus` check so Windows `WindowMenu` leases match the macOS / `MenuBar` root-submenu rule. macOS attach refuses off-thread calls; Windows `translate_accelerator` stays safe when muda has no HACCEL. Docs name the remaining generated-host gaps honestly. _Packages:_ 🟢 _leptoskit_plugin_menu_ _Owner:_ Ifiok Jr. · _Introduced in:_ [32fc47d](https://github.com/leptoskit/leptoskit/commit/32fc47dfe02bf57a215c1079952d182c3c6cb4f2)
- **Android push registration replies in the right byte order.** On Android the plugin wrote a remote registration's revision, state and string length big-endian, while every other reply and the Rust side that reads it are little-endian. An app asking for its push token read a state of `50331648` instead of `3`, or a length of hundreds of megabytes, and failed to decode the reply. The snapshot is now written little-endian like the rest. _Packages:_ 🟢 _leptoskit_plugin_notifications_ _Owner:_ Ifiok Jr. · _Introduced in:_ [1cf8d42](https://github.com/leptoskit/leptoskit/commit/1cf8d4299a5e4daa2822eaa90ba30b2d327b0e78)
- **Record server argument defaults in exported schemas without executing their default functions.** Strict Go, Rust and CLI generation now reports nullable arguments whose omission invokes a server default but whose explicit null cannot be represented by the generated client. Ordinary optional arguments remain accepted. Strict generation also rejects query/form parameters containing free-form JSON, whose empty arrays and objects cannot round-trip through that codec. Use JSON input encoding for those endpoints. Typed lists and maps remain supported. _Packages:_ 🟢 _leptoskit_rpc_codegen_, 🟢 _leptoskit_rpc_ _Owner:_ Ifiok Jr. · _Introduced in:_ [0857012](https://github.com/leptoskit/leptoskit/commit/0857012d89f7a81925de440a09af034c95a2a4c1)
- **A stream body that omits its context and reads it with `rpc::cx()` now sees the query context the server passed in, while it is constructed and while its items are polled.** Streams ran outside the query task, so `rpc::cx()` answered "outside a query" and the stream failed; the whole body now runs inside the context's task — constructing the stream and driving its items — the same as queries and mutations, so a lazily-polled stream keeps the context too. _Packages:_ 🟢 _leptoskit_rpc_macros_, 🟢 _leptoskit_rpc_ _Owner:_ Ifiok Jr. · _Introduced in:_ [8bd1891](https://github.com/leptoskit/leptoskit/commit/8bd1891b8d987c9abe19128343e7107d706ecd88)
- **No `service` cfg warning in apps that call a service.** `#[rpc::query(service = "...")]`, and the other attributes that take `service`, no longer make Cargo warn `` unexpected `cfg` condition value: `service` `` in a crate that calls a service without declaring a `service` feature, such as an app. The attributes read whether the crate declares the feature: a service's own crate keeps the body behind it as before, and every other crate gets the typed stub directly, which is what it got before. _Packages:_ 🟢 _leptoskit_rpc_macros_ _Owner:_ Ifiok Jr. · _Introduced in:_ [b4efffc](https://github.com/leptoskit/leptoskit/commit/b4efffc4eb47c86b523454bfc2508c0fcd97a1e1)
- **Stop live queries when their screen goes away.** Live query hooks now unsubscribe when their reactive owner is cleaned up or their arguments change. Buffered updates are cancelled too, so a removed screen cannot repopulate the query cache after an account change. _Packages:_ 🟢 _leptoskit_rpc_ _Owner:_ Ifiok Jr. · _Introduced in:_ [1bd0618](https://github.com/leptoskit/leptoskit/commit/1bd0618c284e4dff885524d3d6a23db9cfc668f8)

#### Retries stop when a host cannot schedule the wait

_Packages:_ 🟢 _leptoskit_rpc_

_Owner:_ Ifiok Jr. · _Introduced in:_ [1bd0618](https://github.com/leptoskit/leptoskit/commit/1bd0618c284e4dff885524d3d6a23db9cfc668f8)

A keyed HTTP mutation and `call_with_retry` now surface the last network error when the host has no usable timer, instead of waiting forever on a backoff that can never finish. This keeps a native app's Save button from staying on "Saving…" when its host predates the guest timer plugin or rejects the timer.

A live subscription created before the socket opens is sent once: the handshake records which connection delivered it, so the command that woke the connection no longer sends a duplicate `Sub` frame.

- **No dead-code warnings for a client-only wasm build.** A crate that turns on only `leptoskit_rpc`'s `client` feature and builds for `wasm32-unknown-unknown`, such as a shared types crate a dashboard compiles, no longer fails `clippy -D warnings` with `service_origin` and `at_service` "never used". Those helpers are now compiled only where something reaches a service: the browser (`web`), a native app (`guest`), and a server (`services`). Nothing changes for builds that use them. _Packages:_ 🟢 _leptoskit_rpc_ _Owner:_ Ifiok Jr. · _Introduced in:_ [784ba50](https://github.com/leptoskit/leptoskit/commit/784ba50c9e9009f79cbbe2b99e6d59c55c836ec8)
- **Social images accept absolute URLs.** `Site::social_image` uses an absolute URL as it is instead of joining it onto the site's address, so a docs site can serve its social card from object storage. A relative path still joins the site URL as before. The leptoskit docs host their social card on Backblaze, which keeps the image out of the repository. _Packages:_ 🟢 _leptoskit_docs_ _Owner:_ Ifiok Jr. · _Introduced in:_ [8f27e1f](https://github.com/leptoskit/leptoskit/commit/8f27e1f4cb00fc54c8b749d537b3bf8a31bd535d)
- **Keep the file router's SSR module and Axum dependencies on native targets, so enabling all features in browser and Wasm guest builds does not pull in native networking.** _Packages:_ 🟢 _leptoskit_router_ _Owner:_ Ifiok Jr. · _Introduced in:_ [7237130](https://github.com/leptoskit/leptoskit/commit/723713098b125431f6e762cbf4b6f3ebe5506a23)
- **SSR apps now register every file route with `leptos_axum`, and the server render reads the request URL instead of always opening on Home.** _Packages:_ 🟢 _leptoskit_router_, 🟢 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [363c36e](https://github.com/leptoskit/leptoskit/commit/363c36ed87c461f093d2a4c53a090ef8606f9228)
- **Generated routes also compile in private client modules.** The generated `axum_route_list()` helper is optional in client-rendered apps. An unused helper inside a private routes module no longer fails a warnings-denied build. Its documentation names the Axum method without requiring the app to depend directly on `leptos_axum`, so client-only documentation builds work too. SSR hosts can still call the same helper. _Packages:_ 🟢 _leptoskit_router_codegen_ _Owner:_ Ifiok Jr. · _Introduced in:_ [011854e](https://github.com/leptoskit/leptoskit/commit/011854e681643ab8ea28bac74329ef95c0ba5429)
- **Generate SSR paths without redundant conversions.** Generated `axum_route_list` entries now use the `String` that `Pattern::axum_path` returns directly. The redundant conversion made generated SSR routes fail clippy when warnings were denied. _Packages:_ 🟢 _leptoskit_router_codegen_ _Owner:_ Ifiok Jr. · _Introduced in:_ [b4efffc](https://github.com/leptoskit/leptoskit/commit/b4efffc4eb47c86b523454bfc2508c0fcd97a1e1)

#### Refuse ambiguous consolidated rollback without panicking

_Packages:_ 🟢 _leptoskit_db_

_Owner:_ Ifiok Jr. · _Introduced in:_ [8051706](https://github.com/leptoskit/leptoskit/commit/8051706d7c9f9c655f5dc639b8acff2da3c2d47c)

Rollback now returns a migration-history error when an adopted consolidation leaves more applied history rows than the current registry contains. It stops before reverse SQL or history changes instead of indexing beyond the registry. Ordinary rollback of an intact registry prefix is unchanged.

- **Count a request by its connection without the header.** With `client_ip_header` set, a request the header was missing from, or whose header wasn't an address, had no address at all, and the limits kept per address didn't count it. Such a request is now taken to come from its connection's address. _Packages:_ 🟢 _leptoskit_auth_ _Owner:_ Ifiok Jr. · _Introduced in:_ [373ad29](https://github.com/leptoskit/leptoskit/commit/373ad295394a26e69e883a6b28983d35b7e3f02d)
- **Native sign-out clears the session query when offline.** `sign_out_mutation()` now clears a native app's session query when it forgets its local tokens, including when the server is unreachable or refuses the request. The mutation still reports that error: local sign-out does not claim the server revoked the session. A browser keeps its session query on failure because its cookie remains until the server removes it. _Packages:_ 🟢 _leptoskit_auth_ _Owner:_ Ifiok Jr. · _Introduced in:_ [10a1d5d](https://github.com/leptoskit/leptoskit/commit/10a1d5dbcfd61ecb6eaefa14e0a55670a5680dcd)
- **Bound push retries by eight delivery requests and the earlier of expiry or 24 hours after visibility.** Honor seconds and HTTP-date Retry-After without shortening provider delays; report sanitized configuration, horizon, and overflow reasons. Count authentication refusals, bound OAuth recovery, and keep pending work moving after credential rotation. _Packages:_ 🟢 _leptoskit_notifications_ _Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)
- **Disable only the authenticated person's service enrollment, preserving the browser subscription and permission for reuse.** Require a successful revoke acknowledgement with a stable retry key; never infer success from an endpoint list. Remove the unused endpoint-matching unsubscribe helper. _Packages:_ 🟢 _leptoskit_notifications_, 🟢 _leptoskit_service_worker_ _Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)
- **Fence push endpoint ownership and credential rotation.** Keep endpoint ownership until explicit revocation, enforce the live endpoint cap on reactivation, and fence delivery outcomes by credential generation so late provider answers cannot invalidate rotated credentials or change their queue. Recognize FCM unregistration only from a typed FCM error detail. _Packages:_ 🟢 _leptoskit_notifications_ _Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)

#### Split the notifications guide and add Firebase migration

_Packages:_ 🟢 _leptoskit_notifications_

_Owner:_ Ifiok Jr. · _Introduced in:_ [ab02d3c](https://github.com/leptoskit/leptoskit/commit/ab02d3c1cf15353734b5a1fe34115384ab0d4455)

The notifications guide is now a chapter of twelve pages instead of one long page: an overview with a quick start, sending, the inbox, audiences, shaping messages, push setup, receiving on the device, delivery and troubleshooting, campaigns, privacy and data retention, coming from Firebase, and a reference.

- **Troubleshooting** lists every delivery reason code, every error the service answers with its HTTP status, the messages the service stops with at startup, and the enrollment and receiving errors in the app, each with what to do.
- **Coming from Firebase** maps Firebase Cloud Messaging's concepts and calls to this service, gives a migration in steps, and lists what is not there yet.
- **Reference** lists every operation with who may call it, every limit, every variable, and the settings in `leptoskit.toml`.
- The examples that no longer compiled are fixed: sends are built with `..SendSpec::default()`, a person's calls use `leptoskit_rpc::client::call`, and `RegisterPushEndpoint`, `PushDeliveries`, `Channel::new`, the service-worker route, and the receiver component match the current API.
- The signing and publishing pages now explain `[native.macos] provisioning-profile` and `LEPTOSKIT_MACOS_IDENTITY`, which a Mac app that receives push needs.

The crate's readme no longer says that nothing reports a tap to the service, and points at the chapter.

- **Push signing no longer pulls in the `rsa` crate.** The FCM service account's RS256 OAuth assertion is built and signed with ring's constant-time RSA directly — the same primitives APNs's ES256 provider token already used — so `jsonwebtoken` and its RustCrypto stack leave the dependency tree. Startup refusals, claims, caching, and the wire contract are unchanged. _Packages:_ 🟢 _leptoskit_notifications_ _Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)
- **Web Push now encrypts (RFC 8291 `aes128gcm`) and signs VAPID (RFC 8292 ES256) directly on ring instead of through the `web-push` crate, whose `jwt-simple`/`ece` dependency chain pulled the `rsa` and `crypto-bigint` advisories.** Each message draws a fresh ephemeral P-256 key and salt, the RFC 8291 Appendix A vector is asserted byte for byte in the tests, and the wire is unchanged: same headers, same envelope, same VAPID header shape. _Packages:_ 🟢 _leptoskit_notifications_ _Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)
- **Build logs are charged what they keep, and bounded in chunks.** A build's log is charged to the build's cap (`ShippingSettings::build_log_cap`) and to its organisation's (`organization_log_cap`) as its chunks are stored, not as the worker sent their lines. A chunk of blank lines used to be kept and charged nothing, so neither cap ever engaged; it now counts like any other. A job's log also has at most `MAX_BUILD_LOG_CHUNKS` (4,096) chunks over all its attempts, whatever their bytes, and a chunk past that is refused with `409 log_full`. _Packages:_ 🟢 _leptoskit_cloud_ _Owner:_ Ifiok Jr. · _Introduced in:_ [0696997](https://github.com/leptoskit/leptoskit/commit/0696997b3d6d4e549e19a8e2b21713da4dbde9f6)
- **Hold an organisation's own spend cap to its plan's cap.** An organisation's own spend cap could only be set up to the cap of the plan it was on at the time. When the organisation later moved to a plan with a lower cap, its own cap stayed as it was, so usage was allowed past the new plan's cap. Now, when a plan change moves an organisation to a plan whose cap is lower than its own cap, the own cap comes down to that plan's cap. An own cap that already fits is left as it is, whichever way the organisation moves. _Packages:_ 🟢 _leptoskit_cloud_ _Owner:_ Ifiok Jr. · _Introduced in:_ [ee00cca](https://github.com/leptoskit/leptoskit/commit/ee00cca1030469e08f62ffb4449d4ad503c052c8)

#### Close the control plane's second security review

_Packages:_ 🟢 _leptoskit_cloud_

_Owner:_ Ifiok Jr. · _Introduced in:_ [dedf032](https://github.com/leptoskit/leptoskit/commit/dedf032eb758ec35adc6689619328df598e6c427)

An upload keeps the size it was begun with, and each archive has its own object key, so no ticket uploads more than the storage cap counted and a swept archive never takes a later upload with it. `claim` filters by role, organisation and cluster and pages through the queue, so one cluster's or organisation's backlog no longer starves the rest.

A build's log, an organisation's build logs, and the time one attempt at a build may run are all capped (`ShippingSettings::build_log_cap`, `organization_log_cap`, `build_deadline`). A build's lease past its end no longer renews, logs, or finishes. A build can only name images in the repository its job was given. `rollback` is refused while the database's migrations are uncertain. A removal that failed stays in front of operators, with its reason in `JobSummary::failure`, until it is reconciled or its worker reports it done. A status report's lines are stored in chunks small enough to read back.

Log lines hide a secret split over a chunk's lines, and write control characters out instead of passing them to a reader's terminal.

- **Promote the bootstrap operator once per database.** With `LEPTOSKIT_CLOUD_BOOTSTRAP_OPERATOR_EMAIL` set, `leptoskit-cloud-control` makes that address an operator while the database has never had one. Before, it did so again at every start or `migrate` that found no operator, so revoking that address was undone by the next start. The promotion now stops once the audit log holds an `operator.bootstrap` or `operator.grant` entry, so `operator revoke` stays final. `operator revoke` says so when it removes the configured address and no operator remains. _Packages:_ 🟢 _leptoskit_cloud_, 🟢 _leptoskit_cloud_platform_, 🟢 _leptoskit_cloud_app_ _Owner:_ Ifiok Jr. · _Introduced in:_ [a53a454](https://github.com/leptoskit/leptoskit/commit/a53a4542f7ffd915ae725ede071581ebb9891300)
- **Refuse `rollback` at the spend cap, as `deploy` is.** Once an organisation's usage for the month reaches its spend cap, `rollback` answers `spend_cap_reached` and makes no deployment, as `deploy` does. Until now a rollback could still put an earlier release live past the cap, and bring back a larger scale than the one running. What already runs keeps running: nothing is slept, stopped or deleted. Under the cap, rolling back works as before. _Packages:_ 🟢 _leptoskit_cloud_ _Owner:_ Ifiok Jr. · _Introduced in:_ [ef1837a](https://github.com/leptoskit/leptoskit/commit/ef1837a3eb2b7bd8d5672b5839912d68dd810837)
- **Keep control-plane tables and authentication dependencies behind the server feature so client types compile independently of database backend features.** Check isolated clients with no backend, SQLite, Postgres, both backends, and WebAssembly. _Packages:_ 🟢 _leptoskit_cloud_ _Owner:_ Ifiok Jr. · _Introduced in:_ [5e39638](https://github.com/leptoskit/leptoskit/commit/5e396387225c6d0f53e7762277214b6e54998208)

#### The control plane's sweeper runs with several replicas

_Packages:_ 🟢 _leptoskit_cloud_, 🟢 _leptoskit_cloud_platform_, 🟢 _leptoskit_cloud_app_

_Owner:_ Ifiok Jr. · _Introduced in:_ [1dc5cd4](https://github.com/leptoskit/leptoskit/commit/1dc5cd4261a10990a6b1a49726698cca5f276ed6)

A control plane generated with two or more `[control_plane] replicas` was deployed with `LEPTOSKIT_CLOUD_SWEEPER=false`. A Deployment gives that value to every replica, so none of them ran the background sweeper: a worker's expired lease was not taken back, and expired archives and logs were not deleted. The generated manifest now sets the variable to `true` whatever the replica count. The sweeper already lets one replica sweep at a time with a Postgres advisory lock, and `LEPTOSKIT_CLOUD_SWEEPER=false` still switches it off.

The documentation of `LEPTOSKIT_CLOUD_SWEEPER` in `leptoskit_cloud_app` no longer says to set it to `false` on secondary replicas. The `run_sweeper` setting's documentation in `leptoskit_cloud` no longer tells operators to set it to `false` on secondary replicas.

- **Runtime logs hide the environment's own secrets.** A cluster's worker reports an environment's runtime logs to the control plane, which kept each line as the worker sent it. A process that printed one of the environment's secret values left it readable in the logs by anyone who could view the project. The control plane now opens the environment's current secrets as a report arrives and hides them from its lines and from a failed process's reason, from eight bytes up. A value shorter than that, or one the environment replaced or removed after the process started, is still kept as printed. A report is refused while a secret can't be opened, rather than kept as printed. _Packages:_ 🟢 _leptoskit_cloud_ _Owner:_ Ifiok Jr. · _Introduced in:_ [c3c3a55](https://github.com/leptoskit/leptoskit/commit/c3c3a553b9bc0fb69126dcdb22c1356cb2e13a9e)
- **The crate compiles for `wasm32` again: the crate root's `Listener` re-export was illegal on that target, where the item comes from the browser module rather than the native fallback, and an extension-style listener helper nothing called is gone.** `Registration::wait_until_active` waits for a registration's own worker to activate — unlike `ready`, which waits for the page's — and errors when the install fails or never settles instead of hanging. `PushSubscription` reports the application server key its subscription was created for, and `PushManager::unsubscribe_endpoint` unsubscribes a registration's current subscription only when it is one named endpoint, so whatever replaced it is never destroyed. _Packages:_ 🟢 _leptoskit_service_worker_ _Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)
- **`leptoskit native build` for Android reports the APK size from the app's own `android/` project.** It was still looking under `hosts/android`, so a build that produced an APK said there was nothing to measure. _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [a8fec99](https://github.com/leptoskit/leptoskit/commit/a8fec995896be1fea532ee589dfb8869c28f1df7)
- **Keep Android plugins' native libraries on the JNI link line when Cargo prints coloured output, including AAudio for audio plugins.** _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [7237130](https://github.com/leptoskit/leptoskit/commit/723713098b125431f6e762cbf4b6f3ebe5506a23)

#### Android and Apple hosts ship every view they draw

_Packages:_ 🟢 _leptoskit_cli_

_Owner:_ Ifiok Jr. · _Introduced in:_ [6f7114d](https://github.com/leptoskit/leptoskit/commit/6f7114d8355acef2a1e98cba223a58c5a7a15f5b)

Generated Android projects now compile: the platform inventories include `NativeCamera.kt`, `NativeMap.kt`, `NativeLivePhoto.kt`, and `NativeServiceWakeBroker.kt`, so `NativeWidgetRenderer` and `LeptosKitView` resolve the views they draw and the background wake they install. The macOS and iOS inventories likewise include `NativeCamera.swift`, `NativeMap.swift`, and `NativeLivePhoto.swift`, which their widget renderers already used.

A test now fails when a shipped Kotlin or Swift source refers to a type declared only in a file the platform inventory leaves out, so this class of missing source is caught before Gradle or Xcode sees it.

- **Android plugin builds read rustc's `native-static-libs` note even when Cargo colors it.** A colored `note:` label was skipped, so the generated CMake link line omitted libraries rustc named, including `aaudio` and `OpenSLES`, and the plugin failed to link. _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [f5104c6](https://github.com/leptoskit/leptoskit/commit/f5104c6f382ac2e42b512fb160280ee8d375cb9e)

#### Android hosts link the libraries their plugins need

_Packages:_ 🟢 _leptoskit_cli_

_Owner:_ Ifiok Jr. · _Introduced in:_ [c0c5013](https://github.com/leptoskit/leptoskit/commit/c0c5013b708e17723cb48062036b40f22f694a57)

Generated Android and Quest projects now link. The host's `CMakeLists.txt` names the Rust archive and a fixed list of NDK libraries, and a static archive carries none of Cargo's link directives, so a plugin whose Rust code uses a platform library outside that list failed at `ld.lld` with undefined symbols — `AAudio_createStreamBuilder` and its neighbours once the audio plugin was selected.

The Android host build now asks rustc for the libraries the archive needs (`cargo rustc -- --print native-static-libs`) and writes them into the generated link line, in rustc's order and without repeats, so any plugin's libraries (`aaudio`, `OpenSLES`, `mediandk`, `camera2ndk`, `EGL`, `GLESv3`, `vulkan`, …) arrive with the plugin rather than with a list the CLI has to keep in step.

- **An Android build points cc-rs at the NDK's `llvm-ranlib`, as it already did for `llvm-ar`.** A dependency that builds a vendored OpenSSL through `make` no longer fails with `aarch64-linux-android-ranlib: not found` on machines without `llvm-ranlib` on `PATH`. _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [d163d4c](https://github.com/leptoskit/leptoskit/commit/d163d4ce01a3de4209f807b028ade3ed6cf7df1c)
- **Keep the Android API level in native C and C++ builds when a compiler cache wraps the NDK launcher.** _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [7237130](https://github.com/leptoskit/leptoskit/commit/723713098b125431f6e762cbf4b6f3ebe5506a23)
- **Refuse graph-root auth generation before writes.** Auth generation checks that it is running in an application configuration before writing client screens or dependency features. Deployment graph roots now fail before any generation writes, including with `--force`, instead of rejecting the application-only auth settings after partially changing a workspace client. _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [ac480c5](https://github.com/leptoskit/leptoskit/commit/ac480c5935b3132c20821e2b5143b1e3e5c884fe)
- **Generate auth in the selected backend package.** Auth generation follows `build.server_package`, including a named target's resolved backend, while keeping client selection independent. It defaults to `app.name` when no server package is configured. The generator no longer writes server modules or dependency features into an unselected package that happens to share the application's name. _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [3bfdd7f](https://github.com/leptoskit/leptoskit/commit/3bfdd7fec9c797b7262acefce6264a7124887ef8)
- **Reading a watched file does not restart the app.** `leptoskit dev` ignores filesystem access events before scheduling a rebuild or marking the configuration, database tables, or Git branch as changed. On Linux, the compiler and CLI reading their inputs could otherwise trigger another build repeatedly, even without an edit. Source edits and file replacements still rebuild the app. _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [60ff4c1](https://github.com/leptoskit/leptoskit/commit/60ff4c128909e3837a936d04e92b4037c58cb14e)
- **Validate the dev address before watching the workspace.** `leptoskit dev` rejects an invalid web serve address before inspecting Cargo metadata, starting a database, or watching files. Watching still begins before the initial build, so edits made during that build trigger a rebuild. _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [7237130](https://github.com/leptoskit/leptoskit/commit/723713098b125431f6e762cbf4b6f3ebe5506a23)
- **Flush the development watcher announcement so processes reading piped logs receive it before the initial build finishes.** _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [94cd633](https://github.com/leptoskit/leptoskit/commit/94cd633355dffb56cc7249e234ea5fc994c9ba20)
- **`lk dev` keeps edits made while it starts.** `leptoskit dev` now starts watching your project before its first build rather than after every platform has started. An edit saved while that first build ran, which takes minutes on a fresh checkout, used to go unseen: the server came up on the code from before the edit and stayed on it until you saved again. The edit now waits in the watcher's queue, and `dev` rebuilds and restarts the server as soon as the platforms are up. The `watching … for changes` line still prints once everything has started. _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [fa3d29f](https://github.com/leptoskit/leptoskit/commit/fa3d29f8e8c837b19f92f1e9a95f699803543767)
- **`leptoskit dev` says it is watching as soon as the watcher is running, before the first build.** That build can take minutes, and the message used to wait until the platforms had started, so an edit during it looked ignored. _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [f5104c6](https://github.com/leptoskit/leptoskit/commit/f5104c6f382ac2e42b512fb160280ee8d375cb9e)
- **Image builds retry their tool download.** The Dockerfile that `leptoskit deploy` generates for web apps downloads binaryen's `wasm-opt` from GitHub. It now retries that download up to five times and unpacks it only once it is complete, so a brief GitHub outage no longer fails the image build. _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [3cd7307](https://github.com/leptoskit/leptoskit/commit/3cd7307fbdba2e00927bf057a258097652aefca7)
- **Builder lint keeps the SSR shell; apps pin Rust.** `markup = "builder"` no longer reports ordinary HTML tags in the SSR document `shell` (`<html>`, `<head>`, …)—only leptoskit component trees. The agent skill and linting guide say the same. Deploy generation names `--sdk-path` when a path dependency sits outside the workspace. (New apps already pin `rust-toolchain.toml` from the matching scaffold work.) _Packages:_ 🟢 _leptoskit_cli_, 🟢 _leptoskit_lint_ _Owner:_ Ifiok Jr. · _Introduced in:_ [d6f5489](https://github.com/leptoskit/leptoskit/commit/d6f5489be53cd5b202bfd0f8476839a9ce508d41) · _Last updated in:_ [442b38b](https://github.com/leptoskit/leptoskit/commit/442b38b81352231ebb9810f646247bd548ed353c)

#### Generated sign-in effects run in native apps

_Packages:_ 🟢 _leptoskit_cli_

_Owner:_ Ifiok Jr. · _Introduced in:_ [1bd0618](https://github.com/leptoskit/leptoskit/commit/1bd0618c284e4dff885524d3d6a23db9cfc668f8)

`leptoskit new --auth` and `leptoskit generate auth` now use isomorphic effects for the sign-in screen's mount step, the provider-notice step, and the post-sign-in navigation. `Effect::new` is compiled out in a native guest (the generated app does not enable leptos's `effects` feature), so the screen could stay on "Checking whether you're signed in…" forever. The effects still skip server rendering, so the first server HTML matches the browser's first render.

The generated notes journey is now exercised end to end in the real interpreter: the optimized guest loads and refreshes notes over the HTTP plugin, validates the draft, shows pending/saved/failed saves, signs in through the development mailbox, switches accounts without showing the previous account's rows, and rejects late list and save replies after their screen is disposed.

The generated `add_note` operation trims the body before it checks the note's length, so a direct API caller cannot store a note that is only whitespace even though the screen already trims its draft.

The sign-in countdown is tested apart from its executor: a host timer counts the whole wait down, a missing or rejected timer leaves the server's message and a usable manual retry without claiming time passed, a newer refusal or starting over stops an older countdown, and disposing the screen stops it without touching a disposed signal.

`leptoskit generate auth` on an existing database app still adds the screens and asks the app to call `auth::provide()` and put `auth::screens().gate(…)` in front of its own home screen. Rewriting an app's root automatically is not safe, so that manual step stays open; the journey covers the fully scaffolded `--auth` app, which is already wired.

- **Wait for Caddy to finish executing under its dedicated account before checking graph release readiness.** Recheck process identity and socket ownership after probing so systemd startup and process replacement cannot produce premature restoration or a false ready result. _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [7c788ae](https://github.com/leptoskit/leptoskit/commit/7c788ae0d8522dc8f0576961ef4bcb9121603f70)
- **Allow protected systemd drop-in masks that point exactly to /dev/null during graph release and recovery.** Active overrides and masks with unsafe ownership or parents remain rejected. _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [7c788ae](https://github.com/leptoskit/leptoskit/commit/7c788ae0d8522dc8f0576961ef4bcb9121603f70)
- **Keep graph verification checks portable.** Graph directory creation only mutates its builder when applying Unix permissions. On other hosts, descriptor capability methods retain their instance API and continue refusing operations that require Unix metadata. Windows Clippy no longer rejects these unsupported-platform branches. _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [7c788ae](https://github.com/leptoskit/leptoskit/commit/7c788ae0d8522dc8f0576961ef4bcb9121603f70)
- **The generated iOS `App.swift` now imports `LeptosKitNotifications` under its `canImport` guard: the app delegate referenced the plugin's broker type whenever the notifications plugin was selected, but no module was imported, so the generated project failed to compile with the plugin present.** A macOS test now generates the project both with and without the plugin, builds the plugin's Swift package for the iOS simulator, and typechecks the generated app sources — template, registrant, and the bridge's mobile sources — against the real modules, so the conditional import is proven both ways instead of textually. _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)
- **`lk lint` passes on a fresh clone.** `leptoskit lint` no longer fails on a fresh clone of an app with native platforms. App-owned runners include a registrant under `.leptoskit/<platform>/`, which Git ignores and only native builds wrote, so clippy stopped on the missing file until you ran a native build. Before clippy starts, `lk lint` now writes missing registrants for the enabled hosts that compile on the current machine and enables the Cargo features their plugins need. Existing registrants stay untouched until the next native build. If a host cannot be prepared, lint reports a warning and clippy names what it cannot compile. _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [a7d277c](https://github.com/leptoskit/leptoskit/commit/a7d277c8d7e0b13b2ad4eb084b8aac83b312577c)
- **Cloud tools on `lk mcp` take no `url`.** The `lk mcp` cloud tools (`cloud_estimate`, `cloud_deploy`, `cloud_status`, `cloud_logs`, `cloud_secrets_list`) take no `url` argument. It came from the model or the client, and with `LEPTOSKIT_CLOUD_TOKEN` set one tool call could send that token to any origin the argument named. The tools now choose the cloud as `lk cloud` does without `--url`: `LEPTOSKIT_CLOUD_URL`, then `[cloud].url` in `leptoskit.toml`, then the hosted cloud. The token still goes only to the hosted cloud or to the origin `LEPTOSKIT_CLOUD_URL` names; when `leptoskit.toml` alone points somewhere else, the tool refuses and says to set `LEPTOSKIT_CLOUD_URL`. _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [637cdc4](https://github.com/leptoskit/leptoskit/commit/637cdc48582dd612b257fb3728c590854bc56a86)
- **Always run the repository's full pre-push gate, including pushes whose commits change no files.** The gate no longer depends on the hook runner selecting at least one changed file. _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [2c909d1](https://github.com/leptoskit/leptoskit/commit/2c909d10b5be31f521b4ea361a62a4f685538894)
- **Release web builds minify the JavaScript glue, its snippets, and the chunk loader with esbuild, next to `wasm-opt`.** Export names and `import.meta` stay, so the page's imports keep working. `esbuild` comes with the development environment; a machine without it can set `bundle.wasm_opt = false` to skip both. _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [f5104c6](https://github.com/leptoskit/leptoskit/commit/f5104c6f382ac2e42b512fb160280ee8d375cb9e)
- **New apps write builder lint to match the starter screens.** `leptoskit new` now sets `[lint] markup = "builder"` with `markup_style` and `component_attribute` denied, so a fresh scaffold's builder home screen is clean under `leptoskit lint --deny-warnings`. The skill, creating guide, linting guide, and small-bundles guide match: omitted `render` still defaults to csr in the schema, but new projects keep the explicit ssr they already write. _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [5d86470](https://github.com/leptoskit/leptoskit/commit/5d86470b55f2b640a68a13c6097503dc23cfea84)
- **Generated servers find `LeptoskitConfig` again.** The database and auth server templates read `[rpc]` through `leptoskit::LeptoskitConfig`, which `leptoskit` exports only from `leptoskit::config` and its prelude, so a newly generated project didn't compile. They now name `leptoskit::config::LeptoskitConfig`. _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [ef5e7c5](https://github.com/leptoskit/leptoskit/commit/ef5e7c58952db616d3de126653148ae14add82a0)
- **Scaffold uses shared Health; skill documents web-only trim.** New apps re-export `Health` from the shared crate in the app library so `leptoskit lint --deny-warnings` no longer fails on an unused `*_shared` dependency before any RPC types are used. The coding-agent skill says there is no frontend-only `new` flag: after scaffolding, set `platforms = ["web"]` and optionally trim unused host workspace members. _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [fb5e435](https://github.com/leptoskit/leptoskit/commit/fb5e4353ff93ff3cfed5eab0a9d01997609c2bad)
- **New apps pin rustc; skill matches the scaffold.** `leptoskit new` now writes a `rust-toolchain.toml` that pins the SDK channel and the `wasm32-unknown-unknown` target, so `edition = "2024"` projects build even when rustup's default toolchain is older. The coding-agent skill no longer claims the default starter uses regenerating file routes, tells agents to keep the HTML `shell` in leptos `view!` (with a line-local lint allow) when enforcing builders, and shows `#[component]` helpers returning `Element` for `.child(…)` composition. The creating guide notes the SSR default and the toolchain pin. _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [53ca740](https://github.com/leptoskit/leptoskit/commit/53ca7405fac9fb680cd7468429cf09d9b004e76e) · _Last updated in:_ [442b38b](https://github.com/leptoskit/leptoskit/commit/442b38b81352231ebb9810f646247bd548ed353c)
- **Web-only lint skips unused hosts; drop an import.** `leptoskit lint`'s clippy pass excludes workspace host crates whose platforms are not listed in `app.platforms`, so setting `platforms = ["web"]` is enough for a green deny-warnings run without GTK or native prepare. `--db` / `--auth` scaffolds no longer put an unused `use leptos::prelude::*` in `App()`. The skill documents the platforms-based clippy skip. _Packages:_ 🟢 _leptoskit_cli_ _Owner:_ Ifiok Jr. · _Introduced in:_ [44be954](https://github.com/leptoskit/leptoskit/commit/44be9542fedde87deb6c823b8c7f3f5ad44cc49d) · _Last updated in:_ [442b38b](https://github.com/leptoskit/leptoskit/commit/442b38b81352231ebb9810f646247bd548ed353c)
- **`lk lint` knows the plugin surface tags.** `lk lint` now counts the plugin surfaces `Video`, `Camera`, `Map`, and `LivePhoto` among leptoskit's built-in components, so the markup-style rule checks trees that use them like any other built-in tag. _Packages:_ 🟢 _leptoskit_lint_ _Owner:_ Ifiok Jr. · _Introduced in:_ [b4efffc](https://github.com/leptoskit/leptoskit/commit/b4efffc4eb47c86b523454bfc2508c0fcd97a1e1)
- **Over the air updates get their own section.** The home page and README now lead with one component tree and real native widgets. Over-the-air updates have their own section, describing code push for installed native apps and which generated hosts install releases. _Packages:_ 🟢 _leptoskit_ _Owner:_ Ifiok Jr. · _Introduced in:_ [8a98548](https://github.com/leptoskit/leptoskit/commit/8a98548690f1654a63c9b6adffe8b603ba3bbfb5)

#### Check independent consumers before crate publication

_Packages:_ 🟢 _leptoskit_

_Owner:_ Ifiok Jr. · _Introduced in:_ [b6a1990](https://github.com/leptoskit/leptoskit/commit/b6a1990a574a7c077e4f31d408550ae64045a016)

Compile eight separate browser, guest, server and auth feature combinations from the actual Cargo package payloads before publication. Require complete package resolution and reject server features in client builds. Retain per-consumer build logs and exact archive and lockfile hashes; failed or incomplete runs produce no successful acceptance report.

Reject archive paths and directory names that alias by case on every host, so Linux validation also catches archives that cannot extract on a case-insensitive filesystem.

Run validation during release dry runs without publisher credentials. Before upload, require freshly prepared archives to match the hashes from the successful consumer run on the same revision.

- **Fix the platform module guide's path from the source root.** The creating-an-app guide now uses `components/heading` for a module in `src/components/heading.rs`, matching `platform_module!`'s source-root-relative paths. The generated application's end-to-end test uses the same corrected declaration and still verifies adding and removing a platform override. _Packages:_ 🟢 _leptoskit_ _Owner:_ Ifiok Jr. · _Introduced in:_ [d9a66d1](https://github.com/leptoskit/leptoskit/commit/d9a66d1e73141c14be24d3e57bcd4f1cf600bdca)

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
