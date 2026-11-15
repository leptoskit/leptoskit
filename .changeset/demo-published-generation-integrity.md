---
"leptoskit_deploy_console": minor
---

# `Store::schema_fingerprint()` and generation integrity

`leptoskit_deploy_console::control::Store` now exposes `Store::schema_fingerprint()`: a pure, non-I/O digest of the management migrations this build applies (version and SQL, length-framed), computed from the migration list itself so there is no second list to keep in step. Anyone holding a published state contract can now refuse schema drift _before_ `Store::open` runs — `Store::open` still verifies the journal and auto-applies new migrations, so before this API a caller had no way to detect a new migration ahead of the open that would apply it.

The public demo runtime (an unreleased example) uses it for exactly that: its generation fingerprint now covers the deploy console's migrations alongside the analytics migrations and the linked SQLite schema, a published marker's seed count is treated as a historical receipt rather than a live invariant, corrupt markers fail closed instead of triggering a rebuild, retention keeps recent published generations across formats as rollback state, and publication is preceded by an explicit fsync barrier over the databases, their WALs, and the directory entries up to the volume root.
