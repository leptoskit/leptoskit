---
"release": minor
---

# Record analytics events from Rust

`leptoskit_analytics::client` records events, people, and groups, the same way on every platform, and depends on nothing, so it adds no JSON, UUID, time, or compression crate to an app. It batches what's recorded, retries with backoff when the service can't be reached, sends smaller batches when one is too large, and drops the oldest events past its limits, reporting how many it dropped.

On the app's server, the `server-client` feature adds `client::server::Analytics`. It reads the service's address and token from what `leptoskit dev` and `leptoskit serve` set, records events without waiting, and sends them in the background:

```rust
let analytics = Analytics::from_env().expect("the app runs the analytics service");
analytics.capture(&user_id, "invoice_paid", props! { "amount_cents" => 1999 });
```

The browser and native transports follow in a later release.
