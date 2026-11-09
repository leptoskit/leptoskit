---
"leptoskit_bridge_ffi": patch
"leptoskit_wasm": patch
---

# Handed-back calls reach the Rust plugin that serves them

An `AppService` such as Solana Kit's wallet service drains the bridge itself and hands back every call it does not own. Those calls skipped the interpreter's timer and went to the platform, which has no timer plugin: on Android every guest `sleep` failed as `Unavailable` in any app with such a service, so nothing that waited on a timer ever ran. A handed-back call now passes the interpreter first, as a queued one always did.

`WasmHost::serve` and `WasmHost::served_plugins` name what the interpreter answers itself, and `App::rust_plugins` lists every plugin answered in Rust: the timer, the frame clock, inline, worker and async plugins. Tests on the guest thread's own step find every registered plugin by id and fail when a call for a Rust plugin reaches the platform's router, so a Rust plugin added later is held to the same rule.
