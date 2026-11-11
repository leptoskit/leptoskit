# Platform validation

How the repository proves that each platform works: the runtime probe, the widget verifiers, the native fixtures, and what CI runs. None of this is needed to build an app with leptoskit, so it lives here rather than on the public site.

## What CI runs per platform

| Platform        | CI job                                                                | What it exercises                                                                                                                                                                                                             |
| --------------- | --------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| macOS (AppKit)  | `test (macos-latest)` and `applications (macos-latest)` in `ci.yml`   | Workspace tests on a real macOS runner, the `leptoskit_appkit` suite with offscreen PNG snapshots, and a generated macOS app build. Not part of the coverage gate.                                                            |
| iOS (UIKit)     | `ios simulator e2e`, `ios applications e2e`, and `ios code push e2e`  | Boots a simulator and runs the instrumented `native_probe` through `verify_simulator`, with coverage, plus the generated app's UI test and the code push journey.                                                             |
| Android         | `android build and widget tests`                                      | An API 35 x86_64 emulator running `verify_device`, the widget suite, `verify_mobile` gallery screenshots, Gradle instrumented tests, and a launch of the R8-minified release build of the file-routes example, with coverage. |
| Linux (GTK 4)   | `coverage (ubuntu-latest)` and `application coverage (ubuntu-latest)` | The real GTK widget suite under Xvfb with the cairo renderer and AT-SPI.                                                                                                                                                      |
| Windows (Win32) | `test (windows-latest)` and `applications (windows-latest)`           | `leptoskit_windows --test widgets` against real Win32 windows, the workspace tests, a generated-app build, and the widget journey. Not part of the coverage gate.                                                             |
| HarmonyOS       | `harmonyos.yml`, separate from `ci.yml`                               | Deno render-model tests, host contracts, and the Wasm counter, then compiles the app and its instrumentation HAPs for both ABIs. Nothing runs on an emulator or device. See [HarmonyOS](harmonyos.md).                        |
| Terminal        | `coverage (ubuntu-latest)` and `application coverage (ubuntu-latest)` | The `leptoskit_tui` suite headlessly (render snapshots, input routing, escape streaming), the gallery terminal conformance snapshots, and a generated terminal app build.                                                     |
| System plugins  | `native-ui.yml`                                                       | Dialog and permission plugins on all five desktop and mobile platforms. It does not test the widget backends.                                                                                                                 |

The workspace tests and generated application builds run in separate jobs. Each generated app retains its own build target. The two Linux jobs export separate instrumented reports, and the coverage gate requires both.

The local pre-push gate includes the same grouped Wasm Clippy checks as CI, covering browser and guest feature combinations. Optional server dependencies, such as the file router's SSR integration, stay behind native target guards even when a browser build enables every feature.

Linux jobs use Nix's C/C++ compilers and Rust host linker with the Nix libraries. `setup:ci` exports those compiler paths and the Wasm browser capabilities file to later steps, which run outside the devenv shell. Keeping only its `bin` directory on `PATH` loses these settings: a system linker can pair the runner's glibc with a newer Nix `libm`, and browser tests can fall back to the runner's Chrome instead of the pinned Chromium.

Desktop workspace jobs have a 150-minute budget for cold builds and the separate dependency graphs used by compile-error fixtures. Linux workspace coverage keeps its 90-minute budget; application coverage has 120 minutes to finish cold generated-app builds, upload coverage, and save its build cache.

On macOS, `lint:all` and the pre-push gate run `lint:swift`, using the same strict configuration as the Apple bridge job.

The generated web dev journey gives the app server and final watcher announcement one shared 15-minute startup budget: phone guest bundles can still be compiling after the web server starts. Reload checks retain their own build deadlines and match the server rebuild's current status line, while SIGTERM shutdown must finish within 15 seconds. An abandoned test session stops its isolated process group so a failed startup leaves no compiler or app server behind.

Android hosts carry rustc's `native-static-libs` into the generated JNI link line. Parser regressions cover plain and coloured compiler notes, including the styled prefix and final library name; captured stderr retains the terminal's colours.

Field's standalone workspace lints its web feature and all test targets in the Wasm gate. Headless and interpreter fixtures, together with their development dependencies, are native-only.

Browser audio tests exercise the default context and explicit 44.1 kHz and 48 kHz contexts. Decoded metadata must match the context rate, and the half-second fixture must retain its length within one output frame after resampling. Chrome test profiles allow autoplay so the muted audio and speech fixtures reach their playback and cancellation checks. The audio engine has its own Wasm Clippy group with its browser feature enabled and guest mode off.

Linux coverage builds use `line-tables-only` debug information for the dev and test profiles, with incremental compilation disabled. This keeps source locations for diagnostics and LLVM coverage while reducing the disk footprint of instrumented binaries and nested fixture builds. Their cache keys include the debug profile so they cannot restore the previous full-debug artifacts.

The Linux workspace coverage job installs NATS at the version provided by devenv and sets `LEPTOSKIT_REQUIRE_NATS=1`. Relay and session-revocation tests must run against a real server; a missing binary fails the job instead of silently skipping them.

The iOS matrix separates widget verification, generated application journeys, and code push. Each job boots its own simulator, and the coverage gate waits for all three jobs to succeed.

The coverage gate merges only the Linux, browser, iOS, and Android reports; see [CONTRIBUTING.md](../../CONTRIBUTING.md#coverage).

Android Clippy checks use the NDK compiler through `setup/scripts/clippy.sh aarch64-linux-android`. The script keeps API 24 explicit in C/C++ and bindgen flags even when a compiler cache hides the NDK compiler's name. The pinned Clippy reports `missing_const_for_thread_local` on some initializers that already use `const`, through Android's fallback TLS expansion ([rust-clippy#13422](https://github.com/rust-lang/rust-clippy/issues/13422)). Each affected static has an Android-only `expect` for that lint, so a fixed compiler reports the expectation as unfulfilled and requires removing it. Keep the const initializer and all other warnings denied.

## Deployments with services

`services-deploy.yml` runs what `leptoskit generate deploy` and `lk infra` write for an app with [services](services.md), for real. It runs on pull requests and pushes to `main` that touch the generators, the service runtime, `examples/analytics`, or `Cargo.lock`, and weekly for the images and tools it pulls.

The project is a copy of this checkout with `examples/analytics` at its root and a `leptoskit.toml` of its own (`setup/scripts/services_deploy_fixture.sh`). It adds the echo service from `setup/services_echo`, which reports the `Cookie` it receives, sets a cookie of its own, writes to a volume, calls the app's `/readyz`, and answers any other path with a marker. The image is built with the generated `Dockerfile`, changed in one line: crates.io has no CLI with this checkout's changes, so the build stage copies in the CLI built from it.

| Job                                    | What it runs and checks                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| -------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `image and Compose (SQLite, Postgres)` | Builds the image, checks its `app` user is uid and gid 10001, and runs the generated Compose project with `docker compose up --wait`, once with the service on SQLite and once on Postgres. `services_deploy_compose.sh` and `services_deploy_checks.py` check, through the Caddy edge, everything listed below this table, and that only the edge publishes a port, a direct call inside the network sees the cookies the edge strips, and every process exits 0 on `SIGTERM`. On Postgres, they check the init script's role and database. |
| `Kubernetes (kind, ingress-nginx)`     | Validates `kubernetes.yaml` with kubeconform against the cluster's version, applies it to a kind cluster behind ingress-nginx with the Secrets the next steps describe, and waits for every rollout. `services_deploy_kubernetes.sh` checks the probes, the Secrets reaching each process, that every Service is `ClusterIP`, and the same end-to-end checks through the Ingress, with the echo service `first-party`, since generation refuses an isolated public service there.                                                            |
| `Kamal configuration`                  | Loads the `managed_kamal` and `managed_services` snapshots, the files `lk infra` releases with, into Kamal 2.10.1 with `kamal config`. Then it asks Kamal's own code for each role's `docker run` and kamal-proxy options (`services_deploy_kamal.sh`): network aliases, commands, volumes, the `NAME:SECRET` aliases resolved through `.kamal/secrets`, and path-prefix routing.                                                                                                                                                            |

The end-to-end checks, on both Compose and Kubernetes:

- the app's `/healthz` and `/readyz`;
- a server-rendered page whose count comes from the service, and a capture through the service's public prefix that the next render counts;
- the app-only operation refusing callers without the token;
- the service's live socket upgrading (`101`) for the public origin and refusing another (`403`);
- paths outside a public prefix, and near misses such as `/_leptoskit/services/echox/…`, reaching the app rather than the service;
- cookies stripped for an isolated service on Compose and passed to a first-party one on Kubernetes;
- the service's volume, its call to the app at `LEPTOSKIT_APP_URL`, and its data surviving a new container or pod.

Kamal itself never deploys here. `lk infra release` drives Kamal against hosts OpenTofu made: it connects to them as root over SSH, waits for cloud-init, boots a Cloudflare tunnel with a real token, and pulls from an authenticated registry. A local stand-in for all of that would test the stand-in more than the configuration, so the job checks the configuration with Kamal's own code instead.

## The platform probe

The [`platform_probe`](../../examples/platform_probe) binary exercises layout, renderer diffing, the wasm ABI runtime, and a code push delta update over HTTP. Its suite runs on the host, the iOS simulator, and an Android emulator:

```bash
devenv shell test:all                      # includes the desktop probe
ios_sim_run                                # the iOS simulator
android_emulator_run                       # an Android device or emulator
```

The probes verify these runtime contracts. The widget backends have verifiers of their own, which commit real widgets on each platform. Neither shows complete CSS support.

Expected output, on a simulator or emulator:

```
PASS yoga:flexbox
PASS yoga:grid
PASS native:diff-and-events
PASS wasm:abi-runtime
PASS codepush:delta-update
platform probe: all probes passed on ios
```

The `codepush:delta-update` probe ingests two releases into a real store, serves them through the real axum router, and updates a client over the wire with a binary delta that hash-verifies before staging, all on the simulator or emulator OS.

On macOS the desktop run is part of `devenv shell test:all`: the same five probes execute as a native arm64 process, so every layer below the widgets runs on the desktop OS, and the widget layer is the AppKit backend above it.

### iOS simulator by hand

```bash
rustup target add aarch64-apple-ios-sim
cargo build --release --package platform_probe --target aarch64-apple-ios-sim
xcrun simctl boot "iPhone 17 Pro"   # or any available device
xcrun simctl install booted target/platform_probe.app
xcrun simctl launch --console-pty booted dev.leptoskit.platform-probe
```

Link with Xcode's clang (the `xcrun --find clang` toolchain), not a nix-wrapped compiler. [`setup/scripts/ios_probe_run.sh`](../../setup/scripts/ios_probe_run.sh) and the CI workflow have the exact environment.

### Android emulator by hand

Cross-compile with an NDK toolchain clang as the linker:

```bash
rustup target add aarch64-linux-android
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=\
"$HOME/Library/Android/sdk/ndk/<version>/toolchains/llvm/prebuilt/<host>/bin/aarch64-linux-android24-clang"
cargo build --release --package platform_probe --target aarch64-linux-android
emulator -avd Pixel_7_Pro_API_36 &          # or boot from your IDE
adb push target/aarch64-linux-android/release/platform_probe /data/local/tmp/
adb shell chmod +x /data/local/tmp/platform_probe
adb shell /data/local/tmp/platform_probe
```

The probe binary needs no JNI, Gradle, or AndroidX: it is pure Rust linking against bionic. Real apps render through the Kotlin `WidgetHost` and build with Gradle and the NDK.

## Tailwind / Yoga grid parity

The shared layout fixtures live in `crates/leptoskit_native/examples/grid_parity_probe.rs`. Yoga frames are compared to Chrome `getBoundingClientRect` (parent-relative). On Linux the same trees mount on real GTK 4 widgets:

```sh
cargo run -p leptoskit_native --example grid_parity_probe --features web -- /tmp/grid-parity
dbus-run-session -- xvfb-run -a env GDK_BACKEND=x11 GSK_RENDERER=cairo \
  cargo run -p leptoskit_gtk --example grid_parity_gtk -- /tmp/grid-parity-gtk
```

`grid_parity_gtk` fails unless every widget allocation matches Yoga (exact rounded size; origin within 0.5 CSS px). Gallery screenshots on GTK (`gallery snapshot`) and Android (`gallery_verify` / `verify_mobile` on an emulator, including a Pixel Fold AVD) are the paint-side cross-check; Android `WidgetHost` already asserts Yoga owns outer control allocation in `verify_device`.

## The portable widget journey

The SDK's `widget_suite` example is the journey CI runs on every OS through `leptoskit test run`:

```sh
cargo build -p leptoskit_testing --example widget_suite
leptoskit test run --binary target/debug/examples/widget_suite
cargo build -p leptoskit_testing --example widget_suite --target aarch64-apple-ios-sim
cargo build -p leptoskit_testing --example widget_suite --target aarch64-linux-android
```

The AppKit suite (`crates/leptoskit_appkit/tests/appkit.rs`) must run on the main thread. It checks the actual `NSTextField` content after an interaction and renders a PNG; set `LEPTOSKIT_TEST_ARTIFACTS` to keep it.

## Native fixtures

The SDK's `native_plugin_suite` example prepares an extended Android fixture with selected optional dependencies. It includes synthetic activity-result broker tests, orientation cancellation/close restoration and bounded light-sensor acquisition. Pedometer registration compiles without requesting real activity permission or history. Document import bounds and cancellation are tested with synthetic streams/files; these fixtures do not validate interactive native picker UI or access user documents.

The repository also includes a native async fixture. Start its loopback server in one terminal, then generate and build its app in another:

```sh
cargo run -p leptoskit_cli --example async_plugin_server
cargo build -p leptoskit_cli --bin leptoskit
cargo run -p leptoskit_cli --example async_plugin_suite -- \
  --cli target/debug/leptoskit --destination target/async-app --target macos
cd target/async-app
leptoskit native build macos
```

Its **Check files** action also checks HTTP binary data, denied origins, redirects, response limits, cancellation, and WebSocket text/binary messages and close. For an arm64 Android emulator, generate with `--target android --host 10.0.2.2` (use `android-x86_64` for x86-64 emulators) so its explicit host scopes point to the development machine. The fixture server binds only loopback; it is test tooling, not an app service.

## Generated notes journey

`crates/leptoskit_cli/tests/db.rs` builds the real `leptoskit new --db sqlite` and `--auth` apps, including the optimized `wasm32-unknown-unknown` guest with the `native` feature, and runs them through `leptoskit_wasm::WasmHost::headless`. The host registers the generated guest's `http` plugin, sends its binary requests through `leptoskit_plugin_http` to the real scaffolded server, and answers the secure-store protocol in memory, so no device, Keychain, or cloud service is touched. The test edits the scaffold's `SERVER` and `[native.plugin-options.http] origins` to one loopback fixture before compiling, so a guest that reached anywhere else would fail.

It drives the generated screens with surface presses and change events, not copied screen logic:

- the database app's loading, empty, failed, and loaded lists; draft validation; a held save that shows "Saving…" and then "Saved."; a failed save; the draft clearing; and Refresh picking up another client's note;
- the auth app's session resolution, emailed code read from `/_leptoskit/mail`, notes over the HTTP plugin, sign-out, a second account with no previous rows, and held list/save replies delivered after their reactive owner was disposed;
- a rate-limited address on the timer-less headless guest: the sixth code request keeps the server's message and a usable manual retry instead of a countdown that can never finish.

The generated app's own `cargo test` also runs the sign-in countdown's state tests: a host timer counts the whole wait down, a missing or rejected timer keeps the server's message without claiming time passed, a newer refusal or starting over stops an older countdown, and a disposed screen ignores both an in-flight take and a late refusal.

The tests are ignored by default because they compile nested apps:

```sh
cargo nextest run -p leptoskit_cli --test db --run-ignored ignored-only \
  -E 'test(the_database_scaffold_migrates_and_serves) | test(the_auth_scaffold_signs_people_in)'
```

They are interpreter and HTTP integration tests. They do not prove the AppKit, UIKit, Android, GTK, or Win32 widget layers, and they do not use a simulator, emulator, or device.

## Code push journey

`cargo run -p leptoskit_cli --example mobile_codepush -- --platform ios|android --device <id> --sdk-path . --cli target/debug/leptoskit` runs the full journey against a booted simulator or emulator, and CI runs it on both platforms. It scaffolds an app, builds four signed guests from one source, and serves them from a local registry. Then it taps a counter that steps by one, downloads and activates a replacement that steps by ten, and relaunches offline. Then it activates a guest that traps on startup and checks that the working guest is still interactive, both straight away and after a relaunch. Finally it installs a second good update after that rollback, and checks that it survives an offline relaunch.

## `leptoskit dev` reload

`crates/leptoskit_cli/tests/dev_reload.rs` checks that `leptoskit dev` reloads a real app: it scaffolds a project against this checkout, runs `leptoskit dev --platform <platform>`, edits the home screen's heading, and waits until every app `dev` launched logs `leptoskit dev: reloaded`. The shells log that line only once the new guest's native views have rendered. On Android the test also reads each device's view hierarchy with `uiautomator dump` and requires the edited text on screen. The tests need the platform toolchain and a device, so they are ignored by default and CI does not run them yet:

```sh
cargo nextest run -p leptoskit_cli --test dev_reload --run-ignored ignored-only
cargo nextest run -p leptoskit_cli --test dev_reload --run-ignored ignored-only -E 'test(android)'
```

iOS uses every booted iPhone simulator, and boots the newest one when none is booted. Android uses every device `adb devices` lists, phones and emulators alike, and boots the first emulator when none is attached. Each run keeps its project and the full `dev` log under `target/dev-reload/<platform>-<time>/`.
