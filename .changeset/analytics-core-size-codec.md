---
"release": minor
---

# Keep typed analytics capture within the core browser budget

`#[derive(Event)]` and `capture_event` send named properties by default. Enable `leptoskit_analytics`'s `typed` feature to send compact positional values instead, with catalogue negotiation and named retries. Server clients and the service enable this codec automatically. Beacons and saved native queues keep names in either mode.

The browser size check includes a derived event and the capture, identity, group and consent APIs, counts all JavaScript snippets as well as Wasm, and keeps the 12 KiB core limit. `setup/scripts/analytics-size.sh --compact` also measures the optional codec. Browser API calls use small JavaScript helpers to reduce the Wasm and bindings downloaded by apps.
