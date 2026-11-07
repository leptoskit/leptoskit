---
"release": minor
---

# Explicit PostgreSQL expansion windows

Prepare a migration compatibility window with `lk db migration window --baseline <migration> --out <file>`. It checks committed schema snapshots and exact generated SQL, writes a new evidence file without overwriting an existing one, and prints its SHA-256. Preparing a window opens no database and changes no migration or checksum.

Applications that explicitly support the protocol can call `Db::migrate_in_window` with their own embedded schema and migrations. Both the old and the new build must match a checkpoint. A restarted old build accepts later history only inside that window. Unknown migrations, modified checksums, missing history, and development pushes still fail. Ordinary `Db::migrate` and reverse migrations keep their strict checks.

PostgreSQL applies all pending expansions and history entries in one transaction, with bounded lock and statement waits and a total attempt deadline. The first structural subset covers plain new tables, nullable columns without defaults, and comments. It does not certify application business logic, positional SQL, physical schema drift, or custom SQL. SQLite online migration windows are not supported.

Cancelling an ordinary PostgreSQL migration also discards its connection instead of returning a session lock or unfinished transaction to the pool. Completed operations reuse the connection only after the migration lock is released.

Generated database servers embed their latest schema checkpoint with `migration_schema!`, export their migration contract before configuration or database startup, and use `Db::migrate_startup`. Without a configured window startup remains strict. A window requires both `LEPTOSKIT_MIGRATION_WINDOW` and its independent `LEPTOSKIT_MIGRATION_WINDOW_SHA256` pin; invalid or incomplete configuration fails instead of falling back.

Verified image packaging can bind an exact window with `--migration-window` and `--migration-window-sha256`. It compares a confined export from the packaged executable against the reproduced source and includes both documents in the complete release descriptor, covered by its existing approvals and policy digest.

These changes do not remove managed deployment's maintenance requirement. Automatic rollout admission, all-running-version coverage, window delivery, and retiring compatibility windows still need the managed release workflow. A document hash is an integrity pin, not release authorization or runtime attestation.
