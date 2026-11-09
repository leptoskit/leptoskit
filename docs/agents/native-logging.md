# Native logging

`leptoskit_log` is where the runtime's records go: logcat, `os_log`, stderr, or the browser console. The public page is [Reading the runtime's log](../src/guide/37-reading-logs.md); this note is for changing the runtime without bringing back a silent failure.

## Why `log`, not `tracing`

wgpu and naga already speak `log`, so one logger shows the GPU's own records under the same sinks. The runtime needs records, not spans, and `log` costs next to nothing in a Wasm build. A `tracing` subscriber would have meant a bridge for wgpu and more code in every binary.

## The rules

- **Failures that stop drawing go through `App::fail`.** It stores the message for `lk_app_error` and logs it at `error` once. Never assign `App::error` directly.
- **Plugin call failures are logged in one place:** `Bridge::complete_reply` in `leptoskit_bridge`. Every host, Kotlin, Swift, worker, async, and inline, completes through it, so a new host gets the record without logging it again. `Cancelled` is `debug`; every other error is `warn`.
- **Wasm errors name where they happened.** `in_import` prefixes a host trap with the import; `in_export` prefixes it with the export. A new import or export goes through them.
- **Payload excerpts use `leptoskit_log::Excerpt`**, never a slice of the payload: it is bounded to 512 bytes and cut on character boundaries.
- **Kotlin and Swift log through `NativeLog`**, which calls `lk_log_write`, so host records share the runtime's level and tags. Pass the `Throwable` or `Error`, not its message, so the cause chain is kept.
- **The level is compiled in.** `leptoskit_log::level()` reads `LEPTOSKIT_LOG_LEVEL` with `option_env!`; the CLI sets it from `[native] log-level` on the host, desktop, and web builds (`native::set_log_level`, `build::platform_env`).

## Testing records

`leptoskit_log`'s `testing` feature adds `capture::start()`, which keeps records in memory. A test binary has one logger, shared by tests running at once, so each test finds its own records by a name only it uses: a plugin name, a node id, a payload. `Records::expect` fails with every record kept, which makes a missing record easy to read.

The tests that hold each failure path to its record:

| Path                                    | Test                                                           |
| --------------------------------------- | -------------------------------------------------------------- |
| Registration refused, every `Error`     | `crates/leptoskit_bridge/tests/diagnostics.rs`                 |
| FFI completions, FFI refusals, host log | `crates/leptoskit_bridge_ffi/tests/diagnostics.rs`             |
| App creation, breadcrumbs, guest traps  | the `app` module of the same file (`--features app`)           |
| Props, imports, exports                 | `crates/leptoskit_wasm/tests/diagnostics.rs`                   |
| Excerpt bounds and UTF-8 (property)     | `crates/leptoskit_log/src/excerpt.rs`                          |
| Panic hook chaining                     | `crates/leptoskit_log/tests/panic.rs`                          |
| Swift error chains                      | `crates/leptoskit_bridge_ffi/apple-tests/NativeLogTests.swift` |

## Not covered yet

- A panic or `log` record inside the guest's Wasm goes nowhere on native: the guest has no import to write a record. A `leptoskit.log` import, or a guest-side plugin, would carry them.
- Route changes are not logged on native, for the same reason.
- HarmonyOS writes to stderr; hilog is not wired.
- The Kotlin `NativeLog` has no unit test of its own; the Android build and a run on an emulator cover it.
