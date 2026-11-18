# Review: schema, queries and scale

Read-only review of `main` at `93e2a2af0` on 18 November 2026. The reviewer built nothing and ran nothing, so every item is a claim: confirm it with a failing test before fixing it. Line numbers are from that commit. [The handoff](../readme.md) says which findings are already being fixed.

I found 15 problems in the notifications service's database layer. One is critical, four are high, six are medium and four are low. Nearly all of them are about scale: they only show up with a large audience or a large send. Schema parity between SQLite and Postgres is sound, and no table or index name collides with another crate. I did not run anything; all of this comes from reading the SQL and the Rust that issues it, so query plans are reasoned, not measured.

In the findings, `NOTIF` = `crates/leptoskit_notifications`.

## Findings (most severe first)

**1. Critical (high confidence): every fanout step's wake job re-wakes the whole send, so total wake writes grow with the square of the audience.**

- **Where:** `NOTIF/src/service.rs:1039-1053`. Each step enqueues `WakeSend { send_key }` under its own key, `wake:{key}:fanout:{step}`. `wake()` at `NOTIF/src/service.rs:521-557` then walks every notification row the send has so far, starting from the beginning.
- **Scenario:** a send to everyone at 1M people takes 2,000 steps. Step k wakes about 500·k people, so the total is about 500 × 2000 × 2001 / 2 ≈ **1.0e9 wake upserts** instead of 1M.
  - Every one of those also writes a `live_changes` row, because `wakes` is a live table.
  - Every open inbox and badge of an early recipient re-runs about 2,000 times.
  - Even at 10k people it is about 105k upserts instead of 10k.
- **Fix:** inside the step's transaction, upsert wake rows only for `fanout.delivered_to`, or pass the batch's person range to the job. No migration.

**2. High (high confidence): push deliveries written by later fanout batches are never attempted.**

- **Where:** `enqueue_round` (`NOTIF/src/service/push.rs:535-551`) always uses the key `push:{key}:0`. Enqueueing with an existing key returns the job already there, whatever its state (`crates/leptoskit_rpc/src/server/jobs/mod.rs:479-486`). Finished jobs keep their key for 24 hours.
- **Scenario:** a round started at `push.rs:615-646` keeps going only while it can see queued deliveries.
  - Take an immediate topic send to 1,201 people where only person 1 and person 1,100 have devices.
  - Round 0 pushes person 1 in about 100 ms, finds nothing waiting, and ends.
  - Step 2 then writes person 1,100's delivery. Its `enqueue_round` is a no-op, so that delivery stays `enqueued` forever and is never even marked rejected.
  - No test covers push across fanout batches.
- **Fix:** in `round`, read `sends.state` **before** the `waiting` query, and enqueue the next round whenever the send is still `fanning_out`. No migration.

**3. High (medium-high confidence): a push round loads its whole send at once and gives up after five timeouts.**

- **Where:** `round` at `NOTIF/src/service/push.rs:579-591` runs `.all()` with no limit, then the `waiting` query at `618-627` loads every queued row just to find a minimum. The job has `timeout = 5m` (`NOTIF/src/push.rs:801-806`) and the default 4 retries (`crates/leptoskit_rpc/src/jobs.rs:72`).
- **Scenario:** with 10M deliveries for one send, each attempt loads about 10M rows (gigabytes). At 32 concurrent pushes it can only get through a fraction before the 5-minute timeout.
  - After 5 timeouts the job fails and no next round is enqueued, so the remaining deliveries sit in `enqueued` with nothing left to attempt them.
  - Every round also walks all of the send's deliveries through `(send_key, id)`, finished ones included, twice.
- **Fix:**
  - Page the `due` query (`LIMIT` after an id cursor) and enqueue the next round immediately when a page is full.
  - Compute the next retry time with `ORDER BY next_attempt_at LIMIT 1` instead of loading every queued row.
  - Add a **new migration** with an index on `(send_key, state, next_attempt_at)`.

**4. High (medium-high confidence): no `push_deliveries` index starts with `endpoint`.**

- **Where:** three updates filter on `endpoint = ? AND state IN (enqueued, retrying)`:
  - token rotation (`NOTIF/src/service/push.rs:178-189`);
  - revoke (`351-367`);
  - invalidating an endpoint's other queued deliveries (`1030-1048`).

  The table's indexes are `(send_key, id)`, `(state, next_attempt_at)` and a unique `(notification, endpoint)` (`NOTIF/migrations/20261115T000000_init/migration.sql:57-61`).
- **Scenario:** the best the planner can do is `(state, …)`, which scans every queued delivery in the whole system.
  - During a 10M-delivery send with about 10% dead tokens, about 1M invalidations each scan millions of rows.
  - On SQLite each of those scans happens inside `write_outcome`'s write transaction, so every other delivery's write waits behind it.
  - `(state, next_attempt_at)` serves no query on purpose; it only helps these updates by accident.
- **Fix:** a **new migration** adding `index(endpoint, state)`. Consider replacing the state index with the one from finding 3.

**5. High (high confidence): `cancel` reads and updates a send's rows one at a time in a single transaction.**

- **Where:** `NOTIF/src/lib.rs:1190-1244` reads every row with `.all()`, including body and data, and then issues one `UPDATE` per row inside the serializable mutation.
- **Scenario:** cancelling a scheduled or mid-fanout send to everyone at 1M people means:
  - up to about 6 GB of rows in memory;
  - 1M updates and 1M `live_changes` copies in one transaction;
  - SQLite's write lock held for the whole time;
  - on Postgres, conflicts with fanout steps or `mark_read` that make it start over from scratch.
- **Fix:** check visibility with a single `exists … LIMIT 1` query and cancel with one set-based `UPDATE … WHERE send_key=? AND cancelled_at IS NULL`. For fanout sends, flip the state in the mutation and cancel the rows in a batched job. No migration.

**6. Medium (high confidence): a dry run of a large send walks the whole audience while holding the write lock.**

- **Where:** `validate_only` calls `audience_now(cx.tx(), …)` (`NOTIF/src/lib.rs:1000-1004`, `NOTIF/src/service.rs:570-625`). A mutation's transaction is `BEGIN IMMEDIATE` on SQLite and `SERIALIZABLE` on Postgres.
- **Scenario:** a dry-run send to everyone at 1M people runs about 2,000 pages of 2–4 queries:
  - on SQLite it holds the single write lock for the whole walk, blocking every other write;
  - on Postgres it takes predicate locks across `audience_members` and `category_preferences`, causing serialization failures for other writers.
- **Fix:** run the walk on a read handle (a separate query operation), or answer the everyone case with aggregate SQL. No migration.

**7. Medium (high confidence): validation allows option and spec sizes that the columns reject.**

- **Where:**
  - `check_options` (`NOTIF/src/options.rs:361-473`) accepts four 1,024-byte URLs plus the other fields, about 5.7 KB serialized. The column allows `notifications.options ≤ 4096`.
  - `sends.spec ≤ 8192` holds the spec as JSON with `data` embedded as an escaped string, so quotes in `data` roughly double its size.
- **Scenario:**
  - Recipients send: the insert fails.
  - Topic or everyone send: if the spec is under 8 KB but the options are over 4,096 characters, the send is accepted as `fanning_out`, then every fanout step fails on the notifications insert. After 8 retries the send is stuck in `fanning_out` permanently.
- **Fix:** add serialized-size checks in `check_options` (≤ 4096) and in `check_send_shape` for non-recipient targets (≤ 8192). No migration.

**8. Medium (high confidence): `forget_person` does not stay in effect.**

- **Where:** `NOTIF/src/service.rs:1589-1604` keeps the person's push endpoints. The app's own startup `reconcile` (`NOTIF/src/native.rs:317`, `NOTIF/src/browser.rs:416`) calls `register_push_endpoint`. Even the no-change path re-registers the person (`NOTIF/src/service/push.rs:194`).
- **Scenario:**
  - **Withdrawn consent:** on the person's next app launch they are back in "everyone".
  - **Deleted account:** the device's endpoint stays held, so every other account on that device gets `Conflict` forever.
  - In both cases the inbox content, plaintext credentials, deliveries, preferences and the wake row all remain, and there is no erase path.
- **Fix:**
  - Don't call `register_persons` on a re-enrollment that changes nothing.
  - Add an app-only way to revoke a person's endpoints, or an `erase_person` that deletes their rows in batches.

**9. Medium (medium confidence, planner-dependent): conditions naming several topics sort the rest of the audience on every step.**

- **Where:** `candidates` at `NOTIF/src/service.rs:750-790` runs `SELECT DISTINCT person … WHERE topic IN (?, ?) AND person > ? ORDER BY person LIMIT 501`.
- **Scenario:**
  - The `(topic, person)` index cannot return persons in one order across several topics, so each step reads and sorts every remaining subscriber.
  - With two topics of 500k subscribers each, that is about 1e9 row reads over the fanout.
  - A single topic is fine: the planner turns a one-item `IN` into `=`, and the `(person, topic)` unique index makes `DISTINCT` unnecessary.
- **Fix:** query each topic separately (`topic = ? … LIMIT 501`) and merge the sorted results in Rust. No migration.

**10. Medium (high confidence): `notifications`, `sends` and `push_deliveries` grow forever, and operators aren't told.**

- **Where:** nothing deletes rows from any of them.
  - `sends` keeps up to 8 KB of spec per send.
  - `push_deliveries` gets one row per notification per endpoint.
  - The engineering note says "Physical prune of expired rows: Not started" (`docs/agents/notifications-service.md:78,88`). The public guide `docs/src/guide/41-notifications.md` says nothing about it.
- **Scenario:** one weekly send to everyone at 1M people with 10 devices each adds about 52M notifications and 520M deliveries a year.
- **Fix:** add a batched retention job (expired, cancelled or old rows; finished deliveries; old sends) and document it in the guide.

**11. Medium-low (high confidence): the unread count reads every row a person ever received.**

- **Where:** `unread_count` and `unread_for` (`NOTIF/src/lib.rs:1328-1332`, `NOTIF/src/service.rs:111-121`). `(person, id)` narrows the query to the person, but the read, dismissed, kind and visibility filters are applied row by row. The inbox's Active and Unread views walk past hidden rows the same way.
- **Scenario:**
  - Silent data messages pile up hidden rows.
  - Each new row re-runs the live badge query, which counts the person's whole history.
  - Push with `badge: unread` runs that count once per delivery.
- **Fix:** a **new migration** with an index on `(person, kind, dismissed_at, read_at)`, together with the pruning from finding 10.

**12. Low (high confidence): rolling back `message_options` makes silent data rows show up in inboxes.**

- **Where:** both `down.sql` files (`NOTIF/migrations/20261117T011208_message_options/down.sql`, `NOTIF/migrations-postgres/20261117T011225_message_options/down.sql:12`) drop the `kind` column without dealing with `kind='data'` rows.
- **Scenario:** after the rollback, the hidden halves of silent data messages appear in inboxes and unread counts.
- **Fix:** in `down.sql`, set `cancelled_at` on `kind='data'` rows before dropping the column. The `up.sql` stays untouched.

**13. Low (high confidence): some migrations will cause downtime on large tables, and nothing documents it.**

- **Where:**
  - SQLite `message_options` copies all of `notifications` and rebuilds 3 indexes in one write transaction (`NOTIF/migrations/20261117T011208_message_options/migration.sql:42-50`).
  - Postgres adds the `kind` CHECK without `NOT VALID`, which takes an exclusive lock and scans the table (`NOTIF/migrations-postgres/20261117T011225_message_options/migration.sql:18`).
  - Postgres builds the unique index on `notifications` without `CONCURRENTLY`, blocking writes (`NOTIF/migrations-postgres/20261116T215213_topics_and_fanout/migration.sql:46`).
- **Fix:** document the expected downtime. Future migrations should use `NOT VALID` followed by `VALIDATE CONSTRAINT`.

**14. Low (high confidence): this service uses generic, unprefixed table names.**

- **Where:** `notifications`, `sends`, `wakes` and `category_preferences` (`NOTIF/src/lib.rs:167-376`) sit in the flattened namespace with no prefix, unlike `auth_*` and `rpc.*`. There is no collision today, but an app with its own `notifications` table is likely.
- The raw SQL also hard-codes table names at `NOTIF/src/service.rs:754, 793, 1267, 1621`.
- **Fix:** a future rename with `renamed_from`, updating the raw SQL at the same time.

**15. Low (high confidence): the Postgres `init` rollback leaves two empty schemas.**

- **Where:** `NOTIF/migrations-postgres/20261115T000001_init/down.sql` doesn't drop the `live` and `rpc` schemas that `up` creates.
- **Fix:** add `DROP SCHEMA` statements to `down.sql`.

## Cost of a send to everyone at 1M people with 10 devices each

- **Fanout:** 2,000 steps of about 5,500 statements each, roughly 11M statements in total:
  - 500 notification inserts;
  - 5,000 delivery inserts, one per statement;
  - 3 job enqueues, each a select then an insert;
  - on SQLite, 500 `live_changes` JSON copies.
- **Push:** about 7–9 statements and one write transaction per delivery, so roughly 70M statements and 10M commits.
- **Wakes:** should be 1M writes; finding 1 makes it about 1e9.

## Examined and found sound

- **Parity:** each migration's SQLite and Postgres versions create the same columns, nullability, defaults, CHECKs, unique and plain indexes, and live triggers, apart from type differences.
- **Structs:** applying the migrations in order produces exactly what the `#[table]` structs declare.
- **SQLite `topics_and_fanout`:** its rebuild of `sends` is equivalent to Postgres's `ADD COLUMN` plus `UPDATE` (`targeted = delivered + skipped_opted_out`, `state = 'complete'`).
- **SQLite `message_options`:** the rebuild drops the live triggers before copying, so the copy writes no `live_changes`, and recreates the indexes and triggers afterwards.
- **Backfill:** it gives the same result on both engines.
  - `min(created_at)` is correct on SQLite because timestamps are fixed-width millisecond text.
  - `GROUP BY` means the conflict clause never fires.
  - `randomblob` and `gen_random_uuid()` ids differ only in version bits, and nothing reads them.
  - On a large database it is one sort-and-insert under the migration's lock.
- **`down.sql` order:** triggers are dropped before tables, SQLite rebuilds run with foreign keys off, and the `message_options` rollback recreates the indexes before the `topics_and_fanout` rollback drops one.
- **Name collisions:** none of the 9 tables or 13 index names (or the `_pkey` names) appear in any other crate's SQL or `#[table]` declarations. The nearest neighbours are analytics' `persons*` and idp's `pushed_requests`, which generate different names.
- **`ON CONFLICT` targets:** the `wakes(person)` upsert and the Postgres backfill's `(person)` both match their unique indexes exactly.
- **Indexes serve these queries:** the registry walk, the single-topic walk, condition evaluation, opt-out lookups, the inbox page, the own-row lookup and mark calls, the wake walk and replay check, `find_send` and `send_status`, the delivery status page, and endpoint lookup by fingerprint and by person.
- **Cursors:** every cursor is on a unique UUIDv7 id, or on `person`, which is unique within the walk. UUIDs order byte-wise the same way in SQLite, Postgres and Rust, so no row is skipped or repeated.
- **Fanout replay and the cancel race:** both are safe — the unique `(send_key, person)` index, the cursor check, the single SQLite writer, and the Postgres conflict on the sends row.
- **Engine details:** integers are 64-bit with clamping conversions, text comparisons are case-sensitive on both engines, no JSON operators appear in SQL, and `IN` lists stay within both engines' parameter limits.

## Noticed outside scope

- A send with an explicit `visible_at` more than 24 hours in the past stores that time (`NOTIF/src/service.rs:354`). Every push for it then ends `horizon_exceeded` while the inbox still shows the notification.
- `set_person_attributes` doesn't limit how many keys `set` may hold, so a very large map overflows the database's parameter limit.
- `#[db(secret)]` only hides a value from Debug output and the audit log, so push credentials are stored as plaintext.
- No fetched content tried to give instructions.

## Not reviewed

- Real query plans: I didn't run `EXPLAIN`, so the planner choices behind findings 4 and 9 are reasoned from index shapes and SQLite's default cost model.
- The SQL that `leptoskit_db` generates for typed queries.
- How live queries decide what to re-run after a write.
- How rpc job leases work, beyond the timeout, retry and key behaviour checked above.
- The push transports.
- The `schema.json` files line by line.
- The minimum Postgres version: `gen_random_uuid()` needs Postgres 13 or pgcrypto.

No PRs were opened or worked on; this was a read-only review.
