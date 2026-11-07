# Online schema rollout implementation

Audit A05 remains open. This branch starts the migration protocol required for mixed-version startup; it does not yet admit managed online releases.

## Current implementation

- `verify_expansion` compares schema snapshots using the real diff and renderer. It accepts a bounded structural subset and requires exact SQL bytes. Data-preserving risk classification alone is not compatibility evidence.
- `MigrationWindow` records an existing baseline and ordered expansions. Each participating executable checks its own embedded migrations and schema against a checkpoint. Database history must be the exact set of a registry prefix within the window, including lengths, unique names, and checksums. Unordered database rows are allowed; gaps and duplicates are not.
- `from_json_pinned` compares exact document bytes against an independently supplied digest before decoding. A valid replacement document is not accepted against the original pin. This is integrity, not authorization.
- PostgreSQL takes the existing migration advisory key as a transaction lock, applies all pending expansions and metadata atomically, and limits lock, statement, and total waits. The typed transaction guard queues rollback; the connection ownership guard also closes the connection on cancellation, because dropping a query future alone does not stop a long batch executing on PostgreSQL. Ordinary migration and reverse paths remain strict.
- Inspection found an existing cancellation problem in the ordinary migration lock helper: dropping its future could return a session-locked connection to the pool. The new ownership guard detaches and closes that connection on cancellation or failed unlock. A normal completed unlock permits reuse. The regression test requires an independent connection to acquire the same lock while the original pool remains alive.
- `lk db migration window` reads committed snapshots without building or connecting, validates the window before writing, and persists a new file without replacing an existing output. It does not modify registry, SQL, snapshots, release markers, or checksums.
- `migration_schema!` embeds the registry's latest snapshot as a compiler input. Generated database servers export `MigrationContract` before configuration, authentication, or database startup, then use `migrate_startup`. No window means strict startup. Window configuration requires a path and independent SHA-256 pin; malformed or unsupported configuration fails. Empty, legacy, consolidated, and SQLite checkpoints cannot claim window support.
- Optional verified packaging freezes the window before reproduction, exports the checkpoint from the exact packaged executable in confinement, and compares it with the private reproduced source snapshot. Descriptor format 4 includes exact contract and window bytes and hashes. Existing complete-descriptor approvals and policies cover them; stored evidence rechecks the checkpoint and migration fingerprint. This does not yet deliver the window to managed hosts or authorize mixed-version deployment.

The initial structural subset requires known pack membership and PostgreSQL. It includes plain new tables, nullable columns without defaults or generated expressions, and comments. Arbitrary SQL, extension changes, constraints, indexes, renames, and removals are excluded. This does not prove existing triggers or business logic tolerate a new shape. Applications must deliberately use column-named reads and writes. The database's physical schema is not inferred from history.

## Acceptance status

Implementation and tests are authored; no Cargo compile, test, Clippy, or full gate has run for this branch. Rustfmt parsed the changed source. Mini03's heavy build slot belongs to the native-capability delegate. Do not treat authored tests or documentation as acceptance.

Prepared tests cover exact SQL and snapshot classification; old/new checkpoints; modified, duplicate, missing, and unknown history; document versions and independent pins; output nonreplacement; fresh old-build connections after expansion; mixed old/new column-named writes; rollback of an earlier expansion when a later one fails; development-push refusal; lock deadlines; cancellation and pooled-connection reuse; and explicit SQLite refusal. Additional authored tests cover compiler snapshot inputs, executable exports without startup side effects, paired window configuration, frozen packaging inputs, and substituted checkpoint/history evidence. Generated snapshots and real container export still need verification.

Run with PostgreSQL required once a build slot is available:

```sh
devenv shell -- cargo test -p leptoskit_db_schema --test expansion
devenv shell -- cargo test -p leptoskit_db --lib --features postgres,sqlite migration_window
devenv shell -- cargo test -p leptoskit_db --features postgres,sqlite --test migration_windows
devenv shell -- cargo test -p leptoskit_cli --lib db::window
```

Review CLI help snapshots, feature combinations (`server` without a driver, SQLite, PostgreSQL), ordinary migration regressions, docs, schema policy, Clippy, and the full normal publication hook. Real server processes and managed container rollout still need separate acceptance.

## Required next integration

1. Validate the newly authored executable export, generated startup, and optional packaging integration, including real container export and failures without successful evidence. A legacy binary must never be inferred capable from an image label or a CLI flag.
2. Extend admission beyond the packaged executable's checkpoint: bind old/new source checkpoints, physical migration history, and application compatibility declaration into the complete verified release descriptor, approval statement, and deployment policy. Revalidate all consumers, including reuse and recovery.
3. Distinguish the compiled schema from the expanded physical database in release history. Admission must prove every running or rollback-eligible version is covered before expansion. Restore the old application without reverse SQL only while its window remains active.
4. Provide explicit window retirement and contract-phase rules. After contraction, old builds and rollback targets must be rejected. Backfills, defaults, constraints, live-capture changes, and SQLite need designed rollout strategies rather than a broad “safe” flag.
5. Exercise two real old/new server binaries against real PostgreSQL, including concurrent writes, restart, rollback, lock contention, cancellation, lost responses, source/window substitutions, policy expiry, interrupted deployment, and recovery. Preserve exact source, SQL, image, descriptor, policy, and history evidence.

Do not remove the existing `--maintenance` guard until this integration and acceptance are complete. Runtime attestation, arbitrary application compatibility, and cloud availability are not established by this work.
