---
"leptoskit_cloud": minor
---

# Accounts, tokens and projects for LeptosKit Cloud

`leptoskit_cloud` now declares the control plane's first operations, so a client such as the CLI can call them without building the server: `whoami`, organisations, members and invitations, API tokens, and projects and their environments. Their tables and argument types build everywhere, including wasm; their bodies, sign-in and routes are behind the new `server` feature, with SQLite and Postgres migrations.

The server keeps tenants apart and checks every call against one permission table: an organisation or project you can't see answers "not found" whether it exists or not, an organisation always keeps an owner, and a token never does more than its scopes and its maker's current role allow, or reaches a project it isn't bound to. API tokens look like `lkc_<prefix>_<secret>`, are shown once, stored only as a hash, expire after 90 days unless asked otherwise, and stop the moment they're revoked. Invitations are mailed, single-use, expire in a week, and work only for the address they were sent to. The CLI signs in with the device flow, and every change is written to an audit log that never holds a secret.

A queued deployment can now fail before any work starts, when no builder is available, or a quota, the spend cap or a payment stops it.
