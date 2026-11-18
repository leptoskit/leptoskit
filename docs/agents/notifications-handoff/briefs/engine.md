# Task: fix the fanout and push job engine in the leptoskit notifications service

The brief this work was started from, on a Mac, before it moved to cloud sessions. The findings, the scope and what to build still stand. Where it names a gate, follow [the handoff](../readme.md) instead.

You are fixing defects that a code review found in merged code. Do the work yourself; start no subagents.

## Background

leptoskit is a Rust UI toolkit with first-party backend services. `crates/leptoskit_notifications` is its notification service: a durable per-person inbox plus push delivery over APNs, FCM and Web Push, on SQLite or Postgres. A send to a topic, a condition or everyone fans out through a durable job in batches; push deliveries are worked through by a second job in rounds. The jobs run on leptoskit's own job system in `crates/leptoskit_rpc/src/server/jobs/`.

Read first: `AGENTS.md`, `docs/agents/notifications-service.md`, then `src/lib.rs`, `src/service.rs`, `src/service/push.rs`, `src/push.rs`, and the tests in `tests/topics.rs`, `tests/segments.rs`, `tests/push.rs`, `tests/live.rs`.

Base commit: `93e2a2af050fea905b51e69b98219691552ca047` (`main`).

## How to work through the findings

Each item below is a reviewer's claim, with the place they pointed at. Line numbers are from the base commit. For every item:

1. Read the code and decide whether the claim is right.
2. If it is, write a test that fails on the current code for the stated reason, and show it failing.
3. Fix the cause, and show the test passing.
4. If the claim is wrong, or the suggested fix is not the right one, say so with your reasoning and do what is right instead. The suggested fixes are starting points, not instructions.

Do not weaken or delete an existing test to get a green run. Where an existing test asserts the defective behaviour, change it and say which.

## The findings

### 1. Later fanout batches are never pushed (critical)

`enqueue_round` (`src/service/push.rs:535-551`) enqueues the push job under the fixed key `push:{send_key}:0`. The job layer returns the existing job for a key whatever its state, finished ones included, until it is pruned (`crates/leptoskit_rpc/src/server/jobs/mod.rs:479-486`). A round only enqueues a successor if deliveries were waiting when it finished (`src/service/push.rs:615-646`). So when round 0 of a topic, condition or everyone send finishes before a later fanout step commits its deliveries, that step's `enqueue_round` is a no-op and those deliveries stay `enqueued` forever, while `send_status` reports complete. The same happens when a scheduled fanout's `visible_at` arrives while the walk is still running.

Test to write: a topic send to more than two batches of people where every person has an endpoint, with jobs run one at a time so round 0 finishes before step 1. Every delivery must end `accepted`.

### 2. A push round has no size bound, and one error strands the rest

The round reads every due delivery with `.all()` and works through them 32 at a time (`src/service/push.rs:571-649`). The job has a 5 minute timeout and the default retries (`src/push.rs:801-811`). A large send or a slow provider runs past the timeout; requests in flight are aborted and sent again on retry; after the retries are spent the job fails with no successor, and everything still queued is never attempted. A single delivery returning an error (a stored `options` value that no longer parses, the `NotFound` at `src/service/push.rs:962-965`, a busy database) fails the whole round with `?`.

Wanted: a round does a bounded amount of work (by count and by time, well under the timeout) and then enqueues its successor at once; the queries page instead of loading a whole send; the next retry time comes from an ordered `LIMIT 1` query rather than loading every queued row; and one delivery's error becomes that delivery's recorded outcome instead of failing the round. Rounds for one send should not overlap; check whether the job system offers serialization by key and use it if so.

### 3. Every fanout step wakes everyone the send has reached so far

Each step enqueues `WakeSend { send_key }` (`src/service.rs:1045-1053`), and `wake()` (`src/service.rs:521-557`) upserts a wake row for every notification row of the send. Wake writes therefore grow with the square of the audience: about a billion for a send to a million people, each on a live table.

Wanted: a step wakes only the people in its own batch. The wakes for `visible_at` and expiry must still cover the whole send. Test by counting wake writes for a send across three batches.

### 4. The fanout can stall permanently when an older step runs again

The successor job's key is built from the step number (`src/service.rs:1063-1072`), and a step that loses the cursor race returns `Ok` without enqueuing anything (`src/service.rs:1027-1031`). If step N commits and enqueues N+1 but its completion write is lost, N later runs again beside the live chain; depending on ordering, the live step finds its work done, returns `Ok`, and no successor exists. The send stays `fanning_out` forever. `tests/topics.rs:427-431` avoids this key collision deliberately.

Wanted: a replayed or concurrent step can never end the chain while the send is still fanning out. Test with two step bodies against the same stored cursor.

### 5. Long work inside a write transaction

- `send_test` (`src/lib.rs:1745-1762`, `src/service.rs:1751-1785`) makes up to 10 sequential provider HTTP calls inside the mutation's transaction, which is `BEGIN IMMEDIATE` on SQLite. A slow provider blocks every write in the service, including job completion writes.
- `validate_only` walks the whole audience inside the mutation (`src/lib.rs:1000-1019`, `src/service.rs:570-625`).
- `cancel` loads every row of the send, including bodies, and updates them one by one (`src/lib.rs:1188-1246`).

Wanted: no provider call while a write transaction is open; the dry-run walk does not hold the write lock (keep `validate_only` working for existing callers); `cancel` is set-based, and for a large fanout send does not load or update a million rows in one transaction. Keep the public signatures compatible unless there is no other way; if you must change one, say so clearly.

### 6. Retries are spent in two minutes, and the jitter is a constant

`retry_delay` and the backoff (`src/service/push.rs:1068-1093, 1131-1138`) retry after 1, 2, 4 … 64 seconds for APNs and Web Push, so all 8 attempts are gone in about 127 seconds and the delivery ends `attempts_exhausted`, far inside the 24 hour horizon. The jitter uses the first byte of a UUID v7, which is the top byte of the timestamp and so the same for every delivery.

Wanted: an outage of an hour does not lose deliveries; the schedule stays inside the existing horizon; jitter actually varies per delivery. Update the guide's sentence about retries to match.

### 7. Values that pass validation overflow their columns

`notifications.options` holds at most 4,096 characters and `sends.spec` 8,192 (`src/lib.rs:211-215, 250-254`), but `check_options` (`src/options.rs:361-473`) allows about 5.7 KB of options and the spec can exceed 8,192 once `data` is escaped. A recipients send then fails at insert; a topic send is accepted as `fanning_out` and every fanout step fails until the job dies, leaving the send stuck with no error.

Wanted: the send is refused at send time with a clear message, on every target, also under `validate_only`. Prefer bounding the serialized sizes in validation over a migration. Do not add a migration in this task.

### 8. Idempotency keys of 243 to 255 bytes break the send

`check_key` allows 255 bytes, but job keys add prefixes (`wake:{key}:visible`, `fanout:{key}:0`, `push:{key}:0`, `wake:{key}:fanout:{n}`) and the job key column holds 255 (`crates/leptoskit_rpc/src/server/jobs/mod.rs:131`).

Wanted: a 255 byte key works on every target, without changing which keys existing callers may use.

### 9. A flaky test

`wake_jobs_survive_a_restart_and_drain_once_without_spinning` (`tests/live.rs:751-773`) polls until a wake row exists and then asserts the job is `done`, but the worker records `done` in a separate write after the row commits. Poll on what is asserted.

### 10. A past `visible_at` is stored as given

A send with a `visible_at` more than 24 hours in the past stores that time (`src/service.rs:354`), so every push for it ends `horizon_exceeded` while the inbox shows the notification. The documented rule is that an earlier time means now. Decide where the fix belongs and make the push and the inbox agree.

## Scope

In scope: `crates/leptoskit_notifications` (source and tests), and `crates/leptoskit_rpc` only if a finding cannot be fixed without a change to the job system; if you change it, keep the change minimal and run its tests. Fix the sentences in `docs/src/guide/41-notifications.md` and `docs/agents/notifications-service.md` that describe behaviour you changed, and add a changeset in `.changeset/` (copy an existing one's format).

Out of scope:

- New migrations, new indexes, retention or pruning (a later task).
- Provider transports under `src/service/transport/` (another task is changing them now).
- Endpoint enrollment, the condition language, and the client code in `src/native.rs`, `src/browser.rs` and the plugin crate (another task is changing them now). Keep your diff away from `register`, `reconcile` and `src/condition.rs`, and keep every change as local as you can, because that other work will be rebased onto yours.
- Refactors, renames, dependency bumps and formatting sweeps.
- `monochange check --fix` (it strips comments from `Cargo.toml`).

## Acceptance: run these and return the real output

1. The notifications crate's whole test suite, the way the repository's `test:cargo` script runs it (read `devenv.nix`), including the Postgres tests. Every finding you fixed must be covered on both engines where the behaviour is engine-specific (findings 4 and 5 especially).
2. Each new test run three times in a row.
3. `devenv shell -- cargo test -p leptoskit_admin` and `devenv shell -- cargo test -p leptoskit_cli --lib -- jobs:: auth_command::`. If you changed `leptoskit_rpc`, its tests too.
4. `devenv shell lint:all`
5. Last, on your final tree: `devenv shell lint:push` (clippy, the full test suite, `cargo deny`; it takes a long time). This is the gate for merging, so it must pass on exactly the tree you hand back. If a test outside this task fails, rerun it alone once; report it as unrelated only with evidence.
