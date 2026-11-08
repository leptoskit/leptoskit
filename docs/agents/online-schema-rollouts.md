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
- Optional verified packaging freezes the window before reproduction, exports the checkpoint from the exact packaged executable in confinement, and compares it with the private reproduced source snapshot. Descriptor format 4 includes exact contract and window bytes and hashes. Existing complete-descriptor approvals and policies cover them; stored evidence rechecks the checkpoint and migration fingerprint.
- Managed release preparation installs the approved window under a digest-named host path using a nonreplacing hard link, verifies bytes and mode, and binds it read-only with the exact startup pin. Before recording success, the CLI checks the running container's pin, mount, and host file digest. These are operator-host observations, not runtime attestation. Recovery restores the previous release's window configuration; it cannot accidentally inherit the failed candidate's window. Initial setup refuses window-bearing releases because an existing baseline is required. Online admission remains disabled.

The initial structural subset requires known pack membership and PostgreSQL. It includes plain new tables, nullable columns without defaults or generated expressions, and comments. Arbitrary SQL, extension changes, constraints, indexes, renames, and removals are excluded. This does not prove existing triggers or business logic tolerate a new shape. Applications must deliberately use column-named reads and writes. The database's physical schema is not inferred from history.

## Acceptance status

The implementation compiled and passed scoped acceptance on Mini02 using nightly 2026-10-21, with PostgreSQL and Restic required. Evidence is retained outside the checkout in `/Users/mini02/Developer/migration-window-acceptance-20261107`, with a compact backup in `.git/a05-evidence` and a second machine copy. Normal `lint:push` passed: 5,705 workspace tests, 16 file-route tests, and 43 devtools tests, plus the lint, security, release, schema, and dependency checks. The workspace run skipped 58 opt-in tests; the required generated-app and real-engine journeys were executed separately. Publication-hook logs are retained with the gate evidence.

Executed coverage includes schema classification, macro inputs, bounded window decoding and history checks, driver feature combinations, ordinary migrations, CLI commands, approvals, release recovery, and workspace release regressions. Both real generated database and authentication journeys passed, including contract export before invalid startup configuration and the existing native headless coverage. Five generated snapshots were individually reviewed.

Real PostgreSQL tests proved atomic rollback on a later expansion failure, timeout and cancellation, independent advisory-lock acquisition while the original pool remains alive, and fresh old-build connections after expansion. The first cancellation run exposed that merely dropping the pooled client did not promptly interrupt a blocked backend. The ownership guard now detaches the connection and sends a bounded cancellation request before closing it; the failing case and a partial-expansion cancellation regression then passed.

Two separately compiled old/new executable checkpoints ran concurrently against an owned PostgreSQL instance, used column-named writes, restarted the old executable after expansion, and restored the old process without reverse SQL. Exact fixture sources, SQL, window, contracts, executable hashes, and the database dump were retained. The reproducible fixture is in `migration-window-fixture/` beside this document.

A disposable Linux VM with two CPUs and 3 GiB memory ran the literal production installer template. All 11 checks passed, including concurrent installation, quoted paths, conflicting content/mode, and directory/symlink refusal. The old `ln` negative control reproduced the forbidden link inside an existing directory. The earlier macOS GNU-coreutils evidence in `migration-window-installer-acceptance.json` remains historical evidence, distinct from the later Linux run.

Actual pinned Podman reproduction built the fixture twice and independently reproduced it for verified packaging. Confined export from the packaged executable matched the private source contract and approved window. This used an explicitly recorded local Git transport mapping for the synthetic signed fixture commit; it was not a fetch of a published upstream commit. Real confined-export controls also refused missing output, a failing executable, symlink output, oversized output, and valid schema/history substitutions that differed from the source checkpoint. The exact image archive, source bundle, descriptor, approval, policy, and hashes are retained. OpenSSL independently verified the approval signature, and the CLI validated the signed deployment policy.

Real SSH host checks used Podman behind the fixture host's Docker command. Managed installation, container startup, read-only binding, independent host-file verification, and recovery to both a previous window and the original configuration without a window passed. Changed host bytes, missing startup fields, a writable mount, and a lost reservation were refused. Fedora labeling was disabled only for this owned fixture container. This proves the production delivery and observation primitives, not an end-to-end Kamal/cloud deployment or runtime attestation. CLI command fixtures separately cover automatic recovery orchestration, setup refusal, reused-window substitutions, and unchanged history on refusal; their SSH adapter is explicitly mocked.

R25 command tests passed for one oversized descriptor, repeated individually admissible descriptors that exceed the complete 4 MiB history budget, and malformed registry digests. The candidate includes PR629's configuration hash before preflight; completion stays after backup and pruning. The guard prevents newly unreadable history without truncation. Indexed retention and repair of already oversized history remain open.

To repeat focused coverage:

```sh
devenv shell -- cargo test -p leptoskit_db_schema --test expansion
devenv shell -- cargo test -p leptoskit_db --lib --features postgres,sqlite migration_window
devenv shell -- cargo test -p leptoskit_db --features postgres,sqlite --test migration_windows
devenv shell -- cargo test -p leptoskit_cli --lib db::window
devenv shell -- cargo test -p leptoskit_cli --lib infra::schema_window
devenv shell -- cargo test -p leptoskit_cli --lib infra::approvals
devenv shell -- cargo test -p leptoskit_cli --test infra window
```

The ignored real-engine acceptance tests require the explicitly owned fixture host and packaged evidence; normal test runs do not silently claim those checks.

## Required next integration

1. Extend the accepted delivery primitives to a complete managed online deployment journey. A legacy binary must never be inferred capable from an image label or a CLI flag.
2. Extend admission beyond the packaged executable's checkpoint: bind old/new source checkpoints, physical migration history, and application compatibility declaration into the complete verified release descriptor, approval statement, and deployment policy. Revalidate all consumers, including reuse and recovery.
3. Distinguish the compiled schema from the expanded physical database in release history. Admission must prove every running or rollback-eligible version is covered before expansion. Restore the old application without reverse SQL only while its window remains active.
4. Provide explicit window retirement and contract-phase rules. After contraction, old builds and rollback targets must be rejected. Backfills, defaults, constraints, live-capture changes, and SQLite need designed rollout strategies rather than a broad “safe” flag.
5. Extend the accepted old/new process and delivery checks to managed online deployments with lost responses, policy expiry and interrupted deployment. Preserve exact source, SQL, image, descriptor, policy, and history evidence.

Do not remove the existing `--maintenance` guard until this integration and acceptance are complete. Runtime attestation, arbitrary application compatibility, and cloud availability are not established by this work.
