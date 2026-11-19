---
"leptoskit_notifications": minor
---

# Count opens, and report what a send came to

A tap on a push reached your app and stopped there: the service never learned of it, the person's other devices still showed the notification as unread, and nothing could say how many people opened a send. Opens are now recorded, and one call reports what became of a send.

- `mark_opened` is a new operation for the signed-in person. It records the first open of one of their notifications and marks it read in the same write. Opening it again changes nothing and succeeds. It is refused for another person's notification exactly as `mark_read` is, and a silent data message has no open.
- `Opened::mark_opened()` in the `receive` module makes that call from the place you handle a tap, on iOS, macOS, Android, and the web. It answers `Ok(false)` without calling anything for a tap that names no notification, such as a local one. On the web the page makes the call; the receiver worker still calls no server.
- `send_report` is a new app-only query. It answers a `SendSummary`: the send's `progress` (what `send_status` answers), its `label`, when it was made, and `inbox`, an `InboxCounts` of `rows`, `read`, `opened`, `dismissed`, and `cancelled`. The counts are one read of the send's own rows through an existing index. They count the rows still stored, so they shrink once retention has deleted a send's rows, while `progress` does not change.
- `SendSpec::label` is an optional name for a send, 1 to 64 ASCII letters, digits, `-`, `_`, or `.`, checked by the new `check_label`. It is stored with the send, answered in its report, never sent to a device, and part of the idempotent intent. A send without one serializes exactly as before.

**What changes for an existing app:**

- The `open_tracking` migration adds the nullable `notifications.opened_at` and `sends.label` on SQLite and Postgres, with no backfill, and replaces the live triggers on `notifications`. Rows written earlier count as unopened.
- `Notification` and `AppSend` each gain a field, which breaks struct literals of them, and `SendSpec` gains `label`, which breaks its struct literals that do not end in `..SendSpec::default()`.
- Nothing is recorded until your app calls `mark_opened`. `send_status` and `SendProgress` are unchanged.
