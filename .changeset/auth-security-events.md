---
"release": minor
---

# Show people their account's recent activity

`leptoskit::auth` can now show someone what happened to their own account: sign-ins and where they came from, new passkeys, an authenticator turned on or off, merges, apps connected and disconnected, and the rest of the audit trail it already keeps.

- **`GET /auth/events`** and `Auth::security_events` return the signed-in person's `SecurityEvent`s, newest first, 50 to a page (at most 100), with a `next` cursor for older ones. Each has the kind, a detail such as the app's name, the device ("Firefox on macOS"), the IP address, and the time. Because that shows where someone signs in from, it needs a fresh session, as changing a sign-in method does: `step_up_required` or `reauthenticate` otherwise.
- **`security_events()` and `use_security_events()`**, an infinite query with `fetch_next_page`, work the same in the browser, in native apps, and during server rendering.
- **The security page** that `leptoskit generate auth` writes gains "Recent activity", which asks the person to confirm it's them first when it has to.
