# leptoskit_notifications

Durable notifications for LeptosKit apps: a first-party service, the send format its app's server uses, and the inbox its clients read.

An app's server sends to explicit user ids, and the service stores one durable inbox row per recipient, in a database of its own, with idempotency by send key: the same key with the same intent answers the first run's result across restarts — whenever it is asked, expiry included — and the same key with a different intent is a conflict. Recipients read their own inbox from any device their app signs in on — the web, a native app, or the server while it renders — paginated and bounded, with per-person read, dismissed, and cancelled state shared across devices. Categories and opt-out preferences are enforced at send time and at visibility, and changing one reaches screens that are already open. Scheduled sends become visible when their time comes and can be cancelled until then; expired rows leave every view. Open screens follow all of it live: the service runs wake jobs at each send's turning points, so what time changes, the watching screen re-reads.

What this crate deliberately does not do: OS background push. There is no APNs, FCM, or Web Push transport, and no device enrollment. Delivery is authenticated inbox retrieval while the app runs, today. Compose received items with the local-notifications plugin (`leptoskit_plugin_notifications`) where the platform supports it, and see the notifications guide for the platform gaps.

Add the service with `[services.notifications]` in `leptoskit.toml`; see the notifications guide on the site for the whole flow.
