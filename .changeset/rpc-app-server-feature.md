---
"leptoskit_rpc_macros": minor
---

# Keep a crate's operations behind its own server feature

An API crate that other crates link for its client, such as a CLI calling a hosted service, can now keep its operations' bodies and registrations behind one of its own features:

```toml
[package.metadata.leptoskit.rpc]
server-feature = "server"

[features]
server = ["leptoskit_rpc/server"]
```

Built without that feature, the crate's `#[rpc::query]`, `#[rpc::mutation]`, job, and stream operations keep their argument structs and an `Operation` impl that fails with a message naming the feature. Nothing registers them, so a server in a build that links the crate for its client neither compiles their bodies nor serves them, even when `leptoskit_rpc/server` is on for another crate's sake. `#[api]` describes them in the spec only where they are served. A `server-feature` that names a feature the crate doesn't declare is a compile error. Cargo doesn't rebuild a crate when only its metadata changes, so run `cargo clean -p <crate>` after adding or removing the key.

`#[api(scope = …)]` on an operation now reaches its description. Before, operations were registered with no scopes, so `leptoskit_idp` refused every third-party access token on them.
