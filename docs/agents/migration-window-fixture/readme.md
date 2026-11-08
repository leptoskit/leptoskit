# Migration window process fixture

This fixture builds two real executables from the database crate. The default build embeds the baseline; the `expanded` build embeds the nullable-column expansion. `generate` writes exact SQL, schema snapshots, and a pinned window using the real schema renderer. Generated, checksummed bytes live under `fixtures/` so repository formatters do not change their integrity pins.

Run it through the repository development shell with PostgreSQL required. Build `generate` first, run it, then build `backend` once without features and once with `expanded`. Preserve the resulting executables as `old-backend` and `new-backend` in an evidence directory. Run `python3 docs/agents/migration-window-fixture/acceptance.py EVIDENCE_DIRECTORY` with `LEPTOSKIT_POSTGRES_BIN` pointing at the PostgreSQL binaries.

The runner owns a local temporary database cluster and stops it on completion. It checks contract export before invalid startup, concurrent old/new writes, fresh old-process startup after expansion, application rollback without reverse SQL, and window substitution refusal. It retains executable hashes, exported contracts, database SQL, and process logs. This does not establish managed online admission, physical-schema attestation, or retirement coverage.
