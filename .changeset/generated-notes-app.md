---
"leptoskit_cli": minor
---

# New database and sign-in apps start with a real notes screen

A `leptoskit new --db sqlite`, `--db postgres`, or `--auth` project now opens on a notes screen instead of the counter. It uses the shared typed `Notes` and `AddNote` operations the scaffold already publishes: loading, empty, failed, and loaded states, body validation before the mutation, save feedback, and a Refresh button. The browser follows the live socket; native apps use the HTTP plugin and Refresh because their generated hosts don't grant the WebSocket plugin, and the Linux, Windows, and terminal hosts say plainly that they can't reach the server. With `--auth`, the screen is rebuilt per account and the previous account's cached notes are dropped first, so signing out or signing in as someone else never shows the previous person's list. `--db` now also grants the native HTTP plugin and configures its origin. Projects without a database keep the counter.
