---
"release": minor
---

# An analytics service that stores events

`leptoskit_analytics` is now a service plugin: add `[services.analytics]` with `crate = "leptoskit_analytics"` to `leptoskit.toml`, and the app gets a process with its own database that records events.

Clients send events to `/_leptoskit/services/analytics/v1/batch` in a compact binary batch format, where the device's details are sent once per batch rather than with every event, or as JSON through the `capture` operation. The service answers once the events are stored. A retried batch is stored once, every refused event is reported with a reason, and a full queue answers `503` with `Retry-After` rather than dropping events.

The clients that send these batches, and the pages that read them, follow in later releases.
