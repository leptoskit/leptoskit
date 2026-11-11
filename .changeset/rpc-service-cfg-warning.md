---
"leptoskit_rpc_macros": patch
---

# No `service` cfg warning in apps that call a service

`#[rpc::query(service = "...")]`, and the other attributes that take `service`, no longer make Cargo warn `` unexpected `cfg` condition value: `service` `` in a crate that calls a service without declaring a `service` feature, such as an app. The attributes read whether the crate declares the feature: a service's own crate keeps the body behind it as before, and every other crate gets the typed stub directly, which is what it got before.
