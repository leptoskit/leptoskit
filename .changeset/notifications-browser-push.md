---
"leptoskit_notifications": minor
---

Browser push enrollment: the client half of Web Push. A new `browser` feature ships the receiver service-worker script (built with your inbox route, served same-origin at the root), a typed availability check, and `enroll`/`disable` flows that ride the person's own RPC identity — no service token, nothing persisted in the worker. Enrollment waits for the active worker, reuses the browser's existing subscription, and re-enrolling the same browser under the same account answers the same endpoint; another live account is refused until the previous one's revoke lands. The receiver shows the service's envelope under a tag built from the notification's id, falls back to one truthful generic banner for missing, oversized, malformed, or mistyped pushes, and routes clicks only to your own inbox route on your own origin. `leptoskit_service_worker` now compiles for `wasm32`, which it never quite did.
