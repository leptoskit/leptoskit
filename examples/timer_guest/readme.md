# Timer guest

A compiled Rust guest that waits on the native runtime's `leptoskit.timer` plugin. It starts a one-second sleep and drops it, then awaits two short sleeps before reporting success. `crates/leptoskit_wasm/tests/timers.rs` loads it in a headless `WasmHost`, routes the sleeps, and checks that they really waited and that the abandoned call never completed.

```sh
devenv shell test:plugins:guest
```
