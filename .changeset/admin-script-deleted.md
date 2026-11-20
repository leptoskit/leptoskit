---
"leptoskit_admin": minor
---

# The admin ships no hand-written JavaScript

`assets/admin.mjs` is gone, with its `{BASE}/assets/admin.mjs` route, the server-sent events at `{BASE}/api/analytics/live`, and the server-drawn screens it ran on. Every admin page is drawn by `leptoskit_admin_pages` and picked up by its Wasm client, and the analytics stream is the `analytics_live` rpc stream on the admin's live connection.
