# Analytics guest

A compiled Rust guest that records events with `leptoskit_analytics`'s native client. Each run starts the client, reports who it records as, records an `opened` event, and flushes. `crates/leptoskit_wasm/tests/analytics_guest.rs` runs it twice in a headless `WasmHost` that plays the HTTP and filesystem plugins. The first run is offline, so the event is saved in the app's files. The second run restores the person and the event, and sends both runs' events in one batch, which the test decodes.

```sh
devenv shell test:plugins:guest
```
