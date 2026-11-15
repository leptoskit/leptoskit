---
"leptoskit_notifications": minor
"leptoskit": minor
---

Add `leptoskit_notifications`, the first-party notifications service: an app's server sends idempotently to explicit user ids, and each recipient gets one durable inbox row in the service's own database — scheduled visibility with cancel-before-due, expiry, category opt-outs enforced at send and at visibility, and per-person read and dismissed state shared across devices, read through bounded, live-paginated queries on every platform the app's screens run. Time reaches open screens: every send enqueues a wake job when it becomes visible and at its expiry, so a scheduled item appears and an expired one leaves a watching screen without any other write; preference changes update open inboxes the same way. An accepted send answers its stored outcome forever, however the clock moves on, and a known zero-fanout send cancels idempotently. Delivery is authenticated inbox retrieval while the app runs; OS background push (APNs, FCM, Web Push) and device enrollment are not part of this change. The `leptoskit` facade re-exports the shared types behind a `notifications` feature, and the guide is under Services.
