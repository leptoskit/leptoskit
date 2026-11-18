---
"leptoskit_rpc": patch
---

# No dead-code warnings for a client-only wasm build

A crate that turns on only `leptoskit_rpc`'s `client` feature and builds for `wasm32-unknown-unknown`, such as a shared types crate a dashboard compiles, no longer fails `clippy -D warnings` with `service_origin` and `at_service` "never used". Those helpers are now compiled only where something reaches a service: the browser (`web`), a native app (`guest`), and a server (`services`). Nothing changes for builds that use them.
