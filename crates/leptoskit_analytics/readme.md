# leptoskit_analytics

Product analytics for LeptosKit apps.

The binary endpoint accepts at most 4 MiB per request and 16 MiB after gzip expansion. Decoding also has a 32 MiB allocation budget: dictionary references, repeated identities and collection reservations count toward it. A small encoded batch cannot expand an interned name or identity into unbounded allocations. Malformed or over-budget batches answer `400`; no event from that batch is queued. Extreme client clock corrections are clamped to the receive time.

Declare events as Rust types with the `derive` feature and record them with `client::web::capture_event`, `client::native::capture_event`, or the server client's `capture_event`. This checks property names and types at compile time and sends named properties by default.

For smaller batches, also enable `typed` in a browser or native app:

```toml
leptoskit_analytics = { version = "0.3", features = ["web", "derive", "typed"] }
```

The optional codec sends values by position and asks the service's catalogue for their names. An unknown schema is retried with names and the same event ID. Older services receive one named retry of the whole batch; unreadable acknowledgements retain compact batches for retry. Beacons and saved native queues always retain names. Server clients and the service enable `typed` automatically. Leave it off when the smallest initial download matters more than compact batches; typed capture works in both modes.
