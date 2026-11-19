---
"leptoskit_admin": minor
"leptoskit_admin_pages": minor
---

# Projects, operations and Workings drawers run in Rust

The Projects page and each operation's page now come from `leptoskit_admin_pages` and hydrate in the browser, with no `admin.mjs`. Their Workings drawers are Rust too: opening one moves focus to its Close button, closing it returns focus, `W` toggles it, Escape closes it, and Copy uses the clipboard plugin. Link to an open drawer with `?workings=<id>`.

A running operation is followed live over the admin's own live connection at `/_leptoskit/admin/live`, which only a signed-in admin can open. The old `/infra/operations/{id}/events` endpoint is gone.
