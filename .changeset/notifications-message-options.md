---
"leptoskit_notifications": minor
---

# Message options, silent data messages, and test sends

A send can now be shaped per platform. `SendSpec` takes an `options` object — every field optional, and a send without one byte-for-byte what it always was:

- **`kind`**: a visible notification (the default) or a **data message** — silent everywhere: APNs `content-available` under `apns-push-type: background` at priority 5, an FCM data-only message, and a Web Push envelope the receiver worker deliberately does not show. Data sends create no inbox row and never count toward a badge, but their delivery, status, idempotency, cancellation, and fanout behave exactly like a visible send's.
- **`priority`**: `apns-priority` 10/5, `android.priority` HIGH/NORMAL, Web Push `Urgency`.
- **`image`**, **`sound`**, **`link`**: carried where each platform reads it (the web receiver opens a click link on its own origin only; Apple platforms need a Notification Service Extension to display an image).
- **`badge`**: an exact number, or `unread` — resolved **per recipient at push time** to their live unread count, so one send badges each person's own number. The web receiver sets it through `navigator.setAppBadge`.
- **`android`**, **`apple`**, **`web`** overrides: channel, color, icon, tag, sticky on Android; subtitle, thread-id, category, interruption-level, relevance-score on Apple; icon, badge shape, up to two actions, require-interaction, direction, and language on the web.

Everything is validated on both sides with errors naming the field, options count in a send's idempotent intent (the same key with different options is a conflict, and pre-option sends keep answering their own retries), and when a payload is over its provider's limit the extras drop in a documented order — per-platform blocks, image, sound, badge, and link, then the send's `data` — before the id-only envelope.

**`send_test`** (app-only) pushes a spec to one person's live endpoints — FCM's "test on device" — once each, answering per-endpoint outcomes and writing nothing: no inbox row, no delivery row, no count anywhere. It is bounded by the person's enrollment (at most ten live endpoints) and can probe a single endpoint by id.

The receiver service worker honors what arrives: a data message shows nothing but still sets the badge it asked for, options shape the banner, and a click opens the send's link only when it resolves to the app's own origin.
