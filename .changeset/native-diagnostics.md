---
"release": minor
---

# Failures on a device are written to the platform's log

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
