<p align="center">
  <a href="https://leptoskit.dev/">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="docs/src/assets/logo-dark.svg">
      <img src="docs/src/assets/logo-light.svg" alt="leptoskit" width="360">
    </picture>
  </a>
</p>

<p align="center">
  <strong>One Rust codebase. Real native widgets on every screen. Zero-boilerplate cloud to match.</strong>
</p>

<p align="center">
  <a href="https://leptoskit.dev/docs/">Documentation</a> ·
  <a href="https://leptoskit.dev/llms.txt">llms.txt</a> ·
  <a href="https://crates.io/crates/leptoskit">crates.io</a> ·
  <a href="https://docs.rs/leptoskit">docs.rs</a> ·
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml">CI</a>
</p>

---

<!-- {=leptoskitOverview} -->

leptoskit is a full-stack Rust toolkit for taking an app from a single codebase to every screen and to production:

- **One UI, real native widgets:** describe screens once as a tree of components (`View`, `Text`, `Button`, `TextInput`, `ScrollView`, …) styled with built-in Tailwind. On the web it renders to the DOM with SSR, CSR, or islands; on macOS, iOS, Android, Linux, Windows, the terminal, browser extensions, and experimental HarmonyOS it renders native controls with no web view and built-in signed WebAssembly over-the-air updates.
- **Full-stack by default:** typed server functions (`leptoskit_rpc`), file-based routing, cached queries, Rust-struct SQLite or Postgres schemas and migrations (`leptoskit_db`), passkey/OAuth sign-in and OIDC IdP (`leptoskit_auth`, `leptoskit_idp`), background jobs, real-time updates, and native device plugins live beside the UI in one workspace configured by `leptoskit.toml`.
- **Built for AI agents:** compiler-checked builder syntax by default (`[lint] markup = "builder"`), a built-in MCP server (`lk mcp`) for live DevTools inspection and cloud deploys, safe runtime generative UI (`leptoskit_agent_ui`), verifiable releases (`leptoskit verify`), a first-party skill, and `llms.txt` docs.
- **Two clear ways to ship:** deploy straight from `leptoskit.toml` to **LeptosKit Cloud** (`lk cloud launch` / `lk deploy`), or self-host on your own Linux VMs with **OpenTofu + Kamal** (`lk infra`).

<!-- {/leptoskitOverview} -->

## Why leptoskit

Shipping a product across web, phones, desktops, and a backend usually means maintaining four or five separate stacks—React or Next.js for the web, Swift/Kotlin or React Native for mobile, Electron for desktop, a separate auth/database/API layer, and hand-rolled Terraform to deploy it.

leptoskit unifies the entire product stack in one Rust workspace and one `leptoskit.toml`:

- **Real native widgets without WebView or engine bloat:** your UI compiles to WebAssembly, runs inside a lightweight non-JIT interpreter (`wasmi`), computes Flexbox and CSS Grid layout in Rust (`leptoskit_yoga`), and commits mutations straight to real OS views (`NSView`, `UIView`, Android `View`, GTK 4, Win32, ArkUI, terminal cells, and DOM on the web).
- **Small binaries and fast frames:** measured in [`benchmarks.md`](docs/src/benchmarks.md), minimal apps weigh ~5.0 MB on iOS, ~5.75 MB on Android, and ~3.3 MB on macOS (compared to 18–20 MB for React Native and 13.6–36.9 MB for Flutter). On a physical Android device, mounting 1,000 cards takes 108 ms (vs. 253 ms Flutter, 443 ms Lynx, 745 ms React Native) and the 60 Hz ticker wall runs at 5.0 ms/s hitch time (vs. 129.8 ms/s Lynx, 478.0 ms/s React Native, 578.6 ms/s Flutter). On the web, the keyed-table fixture transfers 64.2 KB gzipped with a 5.29 MB heap, and warm rebuilds are 2.3× faster than `cargo-leptos`.
- **Built-in OTA updates and device plugins:** signed, content-hashed WebAssembly bundles with binary deltas and automatic rollback on failed first render (`leptoskit_codepush`), paired with 90+ `leptoskit_plugin_*` crates for camera, audio, video, BLE, NFC, health, location, passkeys, secure store, SQLite, tray/menus, Live Activities, home widgets, and notifications.
- **One type system from Postgres column to iOS button:** declare tables as Rust structs with automatic migrations and `Public*` projection safety (`leptoskit_db`), wire passkeys and 20 OAuth providers (`leptoskit_auth`), publish to app and extension stores (`lk publish`), and ship to LeptosKit Cloud or your own Linux VMs with zero lock-in under the public-domain [Unlicense](license).

## Built for AI agents

leptoskit is designed both for **coding agents building and operating apps** and for **apps that host AI assistants**:

- **One workspace, one compiler, headless verification:** instead of coordinating Swift, Kotlin, TypeScript, SQL, and Terraform across separate repositories, an agent works in one Rust workspace configured by `leptoskit.toml`. `cargo`, `lk lint --deny-warnings`, and `WidgetTester` ([`leptoskit_testing`](crates/leptoskit_testing)) give fast, deterministic feedback across UI, routes, database schema, and backend logic without booting an emulator.
- **Builder syntax by default (`[lint] markup = "builder"`):** `lk new` scaffolds plain Rust builder calls (`View().class("gap-4 p-6").child(Text("Hello"))`) rather than proc-macro markup. Compiler errors point directly at the offending call, `rust-analyzer` completes every method, builds are faster, and standard Rust loops and helpers compose cleanly. (`view!` markup remains supported for humans and is used throughout the examples and docs.)
- **Live inspection and deployment over MCP (`lk mcp`):** coding agents connect to `lk mcp` to inspect running web and native apps (`devtools_sessions`, `inspect_tree`, `inspect_node`, `explain_styles`, `query_cache`, `navigate`, `set_state`, `highlight`, `runtime_errors`, `performance`, `read_logs`, `screenshot`) and to validate or ship to production (`db_check`, `cloud_estimate`, `cloud_deploy`, `cloud_status`, `cloud_logs`, `cloud_secrets_list`) with structured JSON—without shell scraping or exposing secret values.
- **First-party skill and LLM-ready docs:** install the bundled agent skill at [`skills/leptoskit/SKILL.md`](skills/leptoskit/SKILL.md), or load the plain-Markdown documentation index at [https://leptoskit.dev/llms.txt](https://leptoskit.dev/llms.txt) and full book at [https://leptoskit.dev/llms-full.txt](https://leptoskit.dev/llms-full.txt).
- **Safe generative UI inside your app ([`leptoskit_agent_ui`](crates/leptoskit_agent_ui)):** let an in-app assistant answer with streaming, interactive native UI—stats, charts, sliders, tables, and action buttons driven by reactive formulas—written as pure Markdown + component data. It executes no arbitrary code, requires no app-store review to introduce new layouts, exposes `ui.reference()` to teach the model your registered components, and returns structured diagnostics via `ui.check()`. See [interfaces from agents](docs/src/guide/31-agent-interfaces.md).
- **Verifiable releases (`leptoskit verify`):** build reproducible Linux backend binaries with SLSA provenance (`leptoskit verify build` / `reproduce`) so a reviewer—or an auditing agent—can confirm byte-for-byte that the shipped executable matches the public commit. See [verifying a release](docs/src/guide/30-verifying-releases.md).

## Examples

Every screen here is a leptoskit element tree committed as real AppKit views by [`leptoskit_appkit`](crates/leptoskit_appkit), then rendered offscreen at 2x. The phone screens are drawn on macOS at a phone-sized viewport, and `cargo run -p gallery -- snapshot <dir>` regenerates them. Press one to play it: the [examples page](https://leptoskit.dev/docs/examples.html) runs every screen, the dashboard, and the worked examples as their web builds, in your browser.

<p align="center">
  <a href="https://leptoskit.dev/docs/examples.html#lk-play-profile"><img src="https://f002.backblazeb2.com/file/leptoskit/examples/macos/profile.png" alt="A profile screen" width="200"></a>
  <a href="https://leptoskit.dev/docs/examples.html#lk-play-settings"><img src="https://f002.backblazeb2.com/file/leptoskit/examples/macos/settings.png" alt="A settings screen" width="200"></a>
  <a href="https://leptoskit.dev/docs/examples.html#lk-play-chat"><img src="https://f002.backblazeb2.com/file/leptoskit/examples/macos/chat.png" alt="A chat screen" width="200"></a>
</p>

## The platforms

Every platform has a guide in [the documentation](https://leptoskit.dev/docs/platforms/index.html): [web](docs/src/platforms/web.md), [iOS](docs/src/platforms/ios.md), [Android](docs/src/platforms/android.md), [macOS](docs/src/platforms/macos.md), and [Windows/Linux](docs/src/platforms/windows-linux.md), plus [code reuse across platforms](docs/src/platforms/code-reuse.md).

The [`platform_probe`](examples/platform_probe) suite tests the whole stack: yoga layout, renderer diffing, the wasm ABI runtime, and a code push delta update over the real HTTP surface. CI runs it on Linux, macOS, Windows, and the iOS simulator, and cross-compiles it for Android. Locally, run it with `test:all`, `ios_sim_run`, or `android_emulator_run`.

Platform backend crates apply mutation batches to UIKit, AppKit, Android, Win32, and GTK widgets. AppKit is the reference backend. Generated Apple and Android apps use smaller Swift or Kotlin renderers; the backend crates cover more controls. The renderer also has a headless backend for portable tests.

<!-- {=generatedHostLimits} -->

Generated macOS, iOS, and Android apps use the Swift or Kotlin renderers in `leptoskit_bridge_ffi`. They support `View`, `Text`, `Button`, `Pressable`, `TextInput`, `PasswordInput`, `ScrollView`, `Image`, `Vector`, and `Rive`. Other control kinds fail with an unsupported-widget error. Accessible names are forwarded; a labelled `Vector` is an accessible image and `Rive` can be decorative. Most other roles and states are not forwarded. The Rust backend crates support a wider control set. Generated Windows and Linux apps use the full Rust backends, but currently omit runtime plugin registration. Automatic code push update checks currently run only in generated iOS and Android hosts.

<!-- {/generatedHostLimits} -->

## The architecture

| Crate                                                           | What it is                                                                                                                                                                                                     |
| --------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [`leptoskit_yoga`](crates/leptoskit_yoga)                       | A from-scratch layout engine: flexbox **and** CSS grid (`fr`, `minmax`, spans, dense auto-placement), gaps, `aspect_ratio`, RTL, `display: contents`.                                                          |
| [`leptoskit_native`](crates/leptoskit_native)                   | The native renderer, built on the React Native renderer's design: element trees diff into mutation batches that a platform backend applies to real widgets.                                                    |
| [`leptoskit_animation`](crates/leptoskit_animation)             | Animations described once as data and run by each platform's compositor: springs, timing curves, keyframes, transitions, and entering and exiting animations.                                                  |
| [`leptoskit_3d`](crates/leptoskit_3d)                           | GPU 3D scenes: glTF models with skins, morph targets, and animation, owned and drawn by the host with wgpu in a `Scene3d` view, driven by compact ops from the guest.                                          |
| [`leptoskit_xr`](crates/leptoskit_xr)                           | Augmented and virtual reality: ARKit, ARCore, OpenXR (Meta Quest, Android XR), and WebXR sessions, a rotation-sensor fallback, and recorded playback, as a frame stream of poses, planes, anchors, and inputs. |
| [`leptoskit_depth`](crates/leptoskit_depth)                     | Metric depth from one camera picture, on the device: Depth Anything V2 Small run in pure Rust with tract, for AR on phones whose platform measures no depth.                                                   |
| [`leptoskit_canvas`](crates/leptoskit_canvas)                   | A GPU canvas: a display list of rounded rectangles, box shadows, gradients, clips, layers, and filled or stroked vector paths, drawn with wgpu. The game host draws its picture with it.                       |
| [`leptoskit_game`](crates/leptoskit_game)                       | A game host: Rust gameplay on a fixed step, leptoskit menus committed through the native renderer, and a wgpu picture of the world. `leptoskit game` runs it. The sample is `game/lantern`.                    |
| [`leptoskit_appkit`](crates/leptoskit_appkit)                   | The macOS backend: commits mutation batches into real AppKit views, measures text with AppKit's own font metrics, and renders view trees to PNG offscreen.                                                     |
| [`leptoskit_wasm`](crates/leptoskit_wasm)                       | The on-device runtime: runs the app's wasm bundle in `wasmi`, an interpreter that needs no JIT, and drives the renderer through a versioned, fuel-metered ABI.                                                 |
| [`leptoskit_log`](crates/leptoskit_log)                         | The runtime's own log: records and panics go to logcat, `os_log`, standard error, or the browser console, with the plugin, call, node, and payload a failure was about. Nothing leaves the device.             |
| [`leptoskit_codepush`](crates/leptoskit_codepush)               | Over-the-air updates: content-hashed manifests, a single-file bundle container, binary deltas, an update protocol, Axum routes, and a client that verifies every hash.                                         |
| [`leptoskit_axum`](crates/leptoskit_axum)                       | The Axum server toolkit: static files with precompressed gzip, SPA fallback, health checks, and the code push registry. Axum is the one supported server.                                                      |
| [`leptoskit_rpc`](crates/leptoskit_rpc)                         | Retrying queries and mutations over leptos server functions, with errors classified so "offline" and "invalid input" stop looking the same.                                                                    |
| [`leptoskit_db`](crates/leptoskit_db)                           | Rust-struct database schemas for SQLite and Postgres with automatic migrations and `Public*` projection types that keep private columns off the wire.                                                          |
| [`leptoskit_auth`](crates/leptoskit_auth)                       | Full sign-in, MFA, passkeys, and 20 OAuth providers, paired with [`leptoskit_idp`](crates/leptoskit_idp) when your app also acts as an OIDC identity provider.                                                 |
| [`leptoskit_agent_ui`](crates/leptoskit_agent_ui)               | Safe streaming UI written as Markdown + component data by LLMs: reactive variables, formulas, charts, sliders, tables, and actions rendered as native widgets with no code execution.                          |
| [`leptoskit_router`](crates/leptoskit_router)                   | Typed file routes: `src/routes/` becomes one `Route` enum that parses URLs and writes hrefs, generated by `leptoskit routes` with `leptoskit_router_codegen`.                                                  |
| [`leptoskit_query`](crates/leptoskit_query)                     | Async state in the style of TanStack Query: cached, deduplicated, retrying queries, infinite queries, and mutations on fine-grained signals, synced across tabs and windows.                                   |
| [`leptoskit_virtual`](crates/leptoskit_virtual)                 | Headless list virtualization in the style of TanStack Virtual: a windowing core over fixed or measured sizes, overscan, aligned scroll targets, and a scroll container that renders only the visible items.    |
| [`leptoskit_skribble`](crates/leptoskit_skribble)               | Hand-drawn components, charts, and maps from skribble, drawn by `skribble_core` and rendered as vectors on every platform.                                                                                     |
| [`leptoskit_table`](crates/leptoskit_table)                     | Headless tables in the style of TanStack Table: typed columns, sortable and filterable rows, pagination, and shared grid semantics consumed according to each renderer’s accessibility support.                |
| [`leptoskit_use`](crates/leptoskit_use)                         | Cross-platform reactive hooks: state, derived values, watchers, timers, storage, and viewport breakpoints without a DOM or executor.                                                                           |
| [`leptoskit_aria`](crates/leptoskit_aria)                       | Headless accessible components: disclosure, toggle buttons, tabs, labeled fields, and dialogs expressed in shared metadata, with platform-specific accessibility limits.                                       |
| [`leptoskit_form`](crates/leptoskit_form)                       | Headless form state in the style of TanStack Form: typed fields, sync validation, touch and submit tracking, and inputs wired through the aria fields.                                                         |
| [`leptoskit_config`](crates/leptoskit_config)                   | `leptoskit.toml`: app identity, platforms, servers, cloud environments and storage, native targets, code push channels, and plugin declarations.                                                               |
| [`leptoskit_cloud`](crates/leptoskit_cloud)                     | Shared types, source archive, price estimator, and control plane for LeptosKit Cloud (paired with `leptoskit_cloud_runtime`, `leptoskit_cloud_worker`, `leptoskit_cloud_platform`, and `leptoskit_cloud_app`). |
| [`leptoskit_plugin`](crates/leptoskit_plugin)                   | The plugin system: build hooks for leptoskit plugins, leptos plugins forwarded as cargo features, and 90+ `leptoskit_plugin_*` native device capability crates.                                                |
| [`leptoskit_docs`](crates/leptoskit_docs)                       | Markdown with leptos components, in the style of MDX, and the static site generator that builds these docs: navigation, search, highlighting, and checked links.                                               |
| [`leptoskit_cli`](crates/leptoskit_cli)                         | The `leptoskit` (`lk`) binary: `new`, `dev`, `build`, `cloud`, `infra`, `deploy`, `publish`, `verify`, `mcp`, `game`, `serve`, `lint`, `format`, `codepush`, `doctor`, `completions`.                          |
| [`leptoskit_testing`](crates/leptoskit_testing)                 | Native widget tests with semantic finders, interactions, bounded pumping, and diagnostic trees.                                                                                                                |
| [`leptoskit_lint`](crates/leptoskit_lint)                       | The project rules behind `leptoskit lint`: components carry `#[component]`, and trees are written the way `[lint] markup` says.                                                                                |
| [`leptoskit_testing_library`](crates/leptoskit_testing_library) | Testing Library queries and widget-test actions (`tap`, `enter_text`, `toggle`) for leptoskit components in a real browser, sharing `leptoskit_testing`'s finders.                                             |
| [`leptoskit`](crates/leptoskit)                                 | The meta crate that re-exports the toolkit.                                                                                                                                                                    |

## The CLI

```bash
cargo install leptoskit_cli --locked

lk new my_app                            # one app, all six platforms enabled (add --web-only --cloud --auth for web + cloud)
lk dev                                   # run every platform, rebuild on change
lk routes                                # write the typed Route enum from src/routes/
lk build --release --platform web        # cached builds
lk mcp                                   # start the MCP server for coding agents (live DevTools + cloud tools)
lk cloud login && lk cloud launch        # ship straight to LeptosKit Cloud from leptoskit.toml
lk deploy                                # deploy to LeptosKit Cloud (or self-hosted OpenTofu/Kamal)
lk codepush release                      # pack dist/ into a versioned over-the-air release
lk verify build --recipe recipe.json     # build a reproducible, SLSA-provenanced backend release
lk doctor                                # check the toolchain
```

The install includes `lk`, a shorter alias for `leptoskit`. Help and completions answer to whichever name you type.

## Shipping your app

<!-- {=deploymentTracks} -->

| Track                         | Best for                                                                               | Primary commands                                                                     | What it manages                                                                                                                                                       |
| ----------------------------- | -------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **LeptosKit Cloud** (managed) | Shipping fast with zero infrastructure boilerplate, including from AI agents           | `lk cloud login`, `lk cloud launch`, `lk deploy`, `lk cloud status`, `lk cloud logs` | Source archiving, OCI builds, rolling releases, scale-to-zero, managed Postgres, secrets vault, custom domains, and usage metering from `[cloud]` in `leptoskit.toml` |
| **Self-hosted** (`lk infra`)  | Running on your own Hetzner, DigitalOcean, or Linux servers with full provider control | `lk infra generate`, `lk infra plan`, `lk infra apply`, `lk deploy`                  | OpenTofu VMs/networks/Cloudflare tunnels, Kamal app + Postgres containers, and encrypted Restic backups (`infra.mode = "graph"` is experimental)                      |

`lk console` is the legacy operator-enrolled deployment dashboard and agent API; new projects should pick either **LeptosKit Cloud** (`lk cloud`) or **Self-hosted** (`lk infra`).

<!-- {/deploymentTracks} -->

Read the [LeptosKit Cloud guide](docs/src/guide/41-leptoskit-cloud.md) or [Deploying a server](docs/src/guide/24-deploying.md) for step-by-step instructions.

`leptoskit new amazing` creates `amazing_app`, `amazing_backend`, and `amazing_shared` crates, with the SSR server in root `src/main.rs`. The app owns routes, components, and state for every platform. Backend implementation stays server-only, and shared contracts live in the shared crate. App-owned `web/`, `ios/`, `android/`, `macos/`, `windows/`, and `linux/` folders contain platform integration. Generated state stays in ignored `.leptoskit/` directories.

Each build selects one platform feature. Run `leptoskit build --platform web`, `leptoskit build --platform macos`, or `leptoskit native build ios-simulator`. Use `leptoskit::platform_module!` to replace a shared module with a `.<platform>.rs` file while keeping its imports. See [creating a project](docs/src/guide/02-creating.md) for dependencies and overrides.

Speed is a feature. cargo stays in charge of compiling, and leptoskit fingerprints the steps after it, so wasm-bindgen and `wasm-opt` only run again when the compiled wasm actually changed. The fingerprints live in `target/leptoskit/cache/`.

## Native is really native

Native renderers translate the shared element tree into platform widgets. An element can use several native views or layers, and supported component kinds differ by host.

## Over the air updates

An installed native app is a small host that runs your interface as a WebAssembly bundle. Code push ships a new bundle over the air: signed, content-hashed releases, binary deltas, and a client that verifies every byte before it stages the update. A trial release replaces the working one only after it renders, so a release that fails to start is rejected and the app keeps running. The generated iOS and Android apps install these releases; the generated macOS, Windows, and Linux apps don't check for updates yet. Updates must satisfy the installed host's ABI, signing and version policy, and the distribution platform's rules. See [code push](docs/src/guide/05-code-push.md).

## Server functions

[`leptoskit_rpc`](crates/leptoskit_rpc) wraps leptos server functions in retrying, error-classified queries and mutations. The same calls run from the browser, from native shells, and during SSR.

## Contributing

[`devenv`](https://devenv.sh/) provides the reproducible development environment.

```bash
devenv shell install:all   # tools and dependencies
devenv shell test:all      # nextest + doctests
devenv shell lint:all      # clippy -D warnings, leptoskit format, dprint, cargo-deny
devenv shell fix:all       # autofix everything fixable
devenv shell docs:serve    # the documentation site, with live reload
```

Editor settings live in `setup/editors`. Run `setup:vscode` or `setup:helix` to copy them into place. [`CONTRIBUTING.md`](CONTRIBUTING.md) covers coverage, snapshots, the docs site, the configuration schema, and benchmarks; [`AGENTS.md`](AGENTS.md) is the starting point for coding agents.

## Releases

Releases run through [monochange](https://crates.io/crates/monochange) changesets:

1. A feature branch carries a changeset: `monochange run change --package leptoskit --bump patch --reason "..."`.
2. Every pull request runs `release pr (simulated)`, which performs the release planning, lockfile refreshes, and formatting without committing or opening a pull request, so release problems fail on the change that caused them.
3. The `release` workflow keeps one release pull request open that summarises the pending changesets. The monochange GitHub App writes the verified release commit and opens the pull request as the monochange bot, so the release PR runs its own workflows without a personal access token. Merging it cuts the release.
4. The merge pushes the tag, drafts the GitHub release, and dispatches the `publish` workflow, which publishes every crate to crates.io in dependency order. It uses trusted publishing when the OIDC identity verifies, and falls back to `CARGO_REGISTRY_TOKEN` for a crate's first publish, before its trusted publisher exists.

Every crate releases in lockstep as one version group under a single tag. Each keeps its own `changelog.md`, and the root carries the combined summary.

## License

[Unlicense](license)
