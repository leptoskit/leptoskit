---
"release": patch
---

# `kube-client` is 4.2.1 and `wasmi` is 2.0.3

`leptoskit_cloud_runtime`'s `kube` feature moves `kube-client` from 4.2.0 to 4.2.1, and takes `kube` and `kube-core` to 4.2.1 with it, because each pins the others to its own version. Before 4.2.1, `kube-client`'s `openssl-tls` connector trusted every root in OpenSSL's default certificate store as well as the cluster's configured CA, so a certificate from any of those roots passed for the cluster's address (RUSTSEC-2026-0600). LeptosKit Cloud's worker already talks to its cluster through the `rustls-tls` connector, which trusts only the configured CA, and never compiled the OpenSSL connector, so this update changes nothing in what the worker accepts. `wasmi` and `wasmi_core` move from 2.0.0 to 2.0.3 across the workspace, because 2.0.0 to 2.0.2 of each were yanked from crates.io, and they take `wasmi_collections` and `wasmi_ir` from 2.0.0 to 2.0.3 with them, because `wasmi` 2.0.3 requires those versions. `wasmi` is the interpreter that runs a native app's Wasm guest: the patch release changes its internals, not the features `leptoskit_wasm` enables. Nothing in leptoskit's own API changed.
