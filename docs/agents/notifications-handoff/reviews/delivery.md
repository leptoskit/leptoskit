# Review: fanout, the push engine and the transports

Read-only review of `main` at `93e2a2af0` on 18 November 2026. The reviewer built nothing and ran nothing, so every item is a claim: confirm it with a failing test before fixing it. Line numbers are from that commit. [The handoff](../readme.md) says which findings are already being fixed.

I found 13 defects; one is critical, four are high. I read the code only: no build, no test run, no edits. Three points rest on my knowledge of provider behaviour and are marked "plausible". All paths below are under `crates/`.

## Findings, most severe first

### 1. Critical: topic, condition and everyone sends never push to batches written after the first push round has finished

- **Confidence:** confirmed by reading the full path.
- **Where:**
  - `leptoskit_notifications/src/service.rs:1035-1037` calls `enqueue_round`.
  - `enqueue_round` (`leptoskit_notifications/src/service/push.rs:535-551`) uses the fixed job key `push:{send_key}:0`.
  - A job key keeps returning the existing job, done ones included, until the job is pruned (`leptoskit_rpc/src/server/jobs/mod.rs:129-132, 479-486`). The service prunes after 24h (`service.rs:1848`).
  - A round only enqueues a successor if work was waiting when it finished (`service/push.rs:615-646`).
- **Scenario:** an everyone send to 5,000 people, few of whom have devices.
  1. Step 0 writes its deliveries and creates `push:k:0`.
  2. Round 0 sends them, finds nothing waiting, and ends.
  3. Step 3 commits more deliveries. Its `enqueue_round` gets back the finished round 0 and creates nothing.
  4. Those deliveries stay `enqueued` forever, while `send_status` reports `complete`.

  The same happens when a scheduled fanout's `visible_at` arrives while the walk is still going.
- **Smallest fix:** enqueue the round under a per-batch key (for example `push:{key}:batch:{last}`), and give `push_send` `serialize_by = send_key` so rounds for one send never overlap.
- **Test:** a 1,201-subscriber topic send where every subscriber has an endpoint. Run the jobs one at a time so round 0 finishes before step 1. Assert 1,201 deliveries are `accepted`.

### 2. High: a push round has no size bound, and a failed round strands the send's remaining deliveries

- **Confidence:** confirmed.
- **Where:**
  - The round reads every due delivery with `.all()` and works through them in batches of 32 (`service/push.rs:571-649`).
  - `push_send` has a 5-minute timeout and the default 4 retries (`leptoskit_notifications/src/push.rs:801-811`, `leptoskit_rpc/src/jobs.rs:70-79`).
  - On timeout the worker drops the future (`leptoskit_rpc/src/server/jobs/worker.rs:814-825`), which drops the `JoinSet`.
- **Scenario:**
  - A send with tens of thousands of deliveries, or a slow provider (each batch of 32 waits up to the 10–15s HTTP timeout), runs past 5 minutes.
  - Requests in flight are aborted after they may already have reached the provider. They are sent again on retry, so the user gets duplicates.
  - After 5 timed-out runs the job fails. No successor round exists, so everything still queued is never attempted.
  - The same happens if any single delivery returns an error. Examples: a stored `options` value that no longer parses, the `Notification ... .one(tx)` NotFound at `service/push.rs:962-965`, or SQLite busy. That error fails the whole round and aborts its siblings mid-request (`:609-611`).
  - FCM makes it worse: when the token exchange keeps failing, nothing is cached (`fcm.rs:249-271`), so each delivery repeats the slow exchange one after another.
- **Smallest fix:** cap each round at N deliveries or about 4 minutes, then enqueue `round + 1` at once. Record a per-delivery error as that delivery's outcome instead of failing the round with `?`.
- **Test:** a mock boundary that sleeps, with more deliveries than fit in one timeout. Assert every delivery reaches a terminal state, and that no delivery is sent twice except ones whose answer was lost.

### 3. High: every fanout step wakes everyone the send has reached so far, so wake writes grow quadratically

- **Confidence:** confirmed.
- **Where:** each step enqueues `WakeSend { send_key }` (`leptoskit_notifications/src/service.rs:1045-1053`). `wake()` (`service.rs:521-557`) then upserts a wake row for every notification row of the send.
- **Scenario:**
  - A topic send to 100k people runs 200 steps and makes about 10M wake upserts. At 1M people it is about 10^9.
  - Each upsert writes to a live table, which re-runs open subscriptions.
  - These jobs run on the 4 workers that fanout and push share (`leptoskit_notifications/src/main.rs:108`), so those queues starve.
- **Smallest fix:** give the per-step `WakeSend` the batch's person range (after the old cursor, up to `last`) and wake only that range. Keep the `visible` and `expires` wakes covering the whole send.
- **Test:** a 1,201-person topic send. Count wake upserts and assert about one per person per step, not a running total.

### 4. High: the fanout can stall permanently when an older step runs again

- **Confidence:** confirmed by reading. It needs a step to run again after it already enqueued its successor: a worker crash, a lease expiry, or a failed completion write, which the worker ignores (`let _ = record(...)`, `worker.rs:745`).
- **Where:**
  - The successor's key is built from the step number (`service.rs:1063-1072`).
  - The losing step returns `Ok` without enqueuing anything (`service.rs:1027-1031`).
- **Scenario:**
  1. Step N commits, enqueues `fanout:k:N+1`, and its completion write fails.
  2. About 5.5 minutes later step N runs again, alongside the live step K+1. Both read cursor C.
  3. Step N wins: it writes the page and enqueues `fanout:k:N+1`, which already exists, so nothing new is created.
  4. On SQLite, BEGIN IMMEDIATE means step K+1's transaction starts only after step N commits, so it finds every row already present. Its cursor fence fails, it rolls back, and it returns `Ok` with no successor.
  5. The send stays `fanning_out` forever and the rest of the audience is never reached.

  `tests/topics.rs:427-431` sidesteps this key collision on purpose.
- **Smallest fix:** give `fanout_send` `serialize_by = send_key`, and key the successor by the cursor reached (`fanout:{key}:{last}`). In the `advanced == 0` path, if the state is still `fanning_out`, enqueue a successor keyed by the stored cursor.
- **Test:** run two `fanout_step` bodies against the same stored cursor, holding one between its read and its transaction. Assert the send reaches `complete`.

### 5. High: `send_test` holds the database write lock across provider HTTP calls, and `validate_only` and `cancel` hold it for unbounded work

- **Confidence:** confirmed for SQLite.
- **Where:**
  - Mutations run in a serializable transaction (`leptoskit_rpc/src/server/run.rs:266`), which on SQLite is `BEGIN IMMEDIATE` (`leptoskit_db/src/sqlite.rs:141`).
  - `send_test` (`leptoskit_notifications/src/lib.rs:1745-1762`, `service.rs:1751-1785`) makes up to 10 sequential provider calls inside that transaction. Each can take 15s, plus the FCM token exchange or Web Push DNS.
  - `validate_only` walks the whole audience inside the mutation (`lib.rs:1000-1019`, `service.rs:570-625`).
  - `cancel` loads every row of the send and updates them one by one (`lib.rs:1188-1246`).
- **Scenario:**
  - One `send_test` against a person with 10 devices on a degraded provider blocks every write in the service for minutes: sends, `mark_read`, job claims, push outcomes.
  - Job completion writes fail silently, which triggers the replays behind findings 2 and 4.
  - On Postgres it keeps a serializable snapshot open across HTTP.
  - A contended retry pushes again.
- **Smallest fix:**
  - `send_test`: read the endpoints and badge, end the transaction, then call providers, for example from a query or a plain route.
  - `validate_only`: move it to a separate query.
  - `cancel`: use one bulk `UPDATE ... WHERE send_key = ? AND cancelled_at IS NULL` plus an `exists` check.
- **Test:** a hanging boundary during `send_test`. Assert a concurrent `mark_read` and a job completion succeed within a second.

### 6. Medium: an APNs or Web Push outage longer than about 2 minutes loses deliveries for good, and the retry jitter is a constant

- **Confidence:** confirmed.
- **Where:** `retry_delay` and the backoff in `service/push.rs:1068-1093, 1131-1138`; the ×60 multiplier applies only to FCM.
- **Scenario:**
  - A 5xx, timeout or unreachable answer retries after 1, 2, 4, …, 64s. All 8 attempts are spent in about 127s, then the delivery ends `attempts_exhausted`, well inside the 24h horizon.
  - The jitter uses `delivery.id.as_bytes()[0]`, which in a UUID v7 is the top byte of the timestamp. It is `0x01` for every id created between 2004 and 2039, so every delivery gets the same 0–1s jitter. The guide's "stable per-delivery jitter" claim is false.
- **Smallest fix:** use minute-scale backoff for all providers, or spend the budget by time within the horizon. Take the jitter from the random tail of the UUID (`as_bytes()[15]`).
- **Test:** a provider that answers 503 for 10 minutes. Assert the delivery is accepted afterwards, and that two deliveries' retry times differ.

### 7. Medium: Safari Web Push subscriptions are refused at enrollment

- **Confidence:** high, from Apple's documentation (endpoints look like `https://web.push.apple.com/…`, and Apple says to allow `*.push.apple.com`). Not checked against the live docs here.
- **Where:** `leptoskit_notifications/src/service/transport/host.rs:29-33` ships `webpush.apple.com`. The guide repeats it at `docs/src/guide/41-notifications.md:799`.
- **Scenario:** every macOS Safari or iOS home-screen web app enrollment fails with `Invalid`. Edge, which pushes through `*.notify.windows.com`, is refused as well.
- **Smallest fix:** ship `web.push.apple.com`.
- **Test:** enroll a `https://web.push.apple.com/<id>` subscription.

### 8. Medium: FCM pushes with a title plus body over about 1,750 bytes always end `oversized`, though validation allows 2,200 bytes

- **Confidence:** the code path is confirmed. The claim about how FCM counts its limit is plausible, not verified.
- **Where:** `leptoskit_notifications/src/service/transport/fcm.rs:332-357, 536-557, 750-761`.
- **Scenario:**
  - The 4,096-byte check covers the whole HTTP request: the token, the Android config, and a second copy of the title and body in `apns.payload.aps.alert`.
  - Even the minimal envelope comes to about 520 bytes plus twice the title and body. A 2,000-byte body therefore never reaches any FCM device. `tests/push.rs:2764-2781` asserts this.
  - FCM's 4 KB limit applies to the delivered notification and data, not the request, so FCM would very likely accept these.
- **Smallest fix:** drop the duplicate `aps.alert`, since FCM builds it from `notification`, and leave the token and config out of the measurement.
- **Test:** an FCM send with a 2,000-byte body is accepted.

### 9. Medium: silent FCM data messages carrying a badge, sound or Android option show a notification on Android

- **Confidence:** plausible. It depends on FCM treating `android.notification` as a notification payload.
- **Where:** `fcm.rs:663-702` adds `android.notification` (with `notification_count`, `sound`, `channel_id`) even when `is_data()` is true. `check_options` (`leptoskit_notifications/src/options.rs:361-473`) allows those options on data sends.
- **Scenario:** the guide's "silent badge correction" (`kind: data`, `badge: set(2)`) becomes a tray notification on a backgrounded Android app, and `onMessageReceived` is not called. The existing test `the_fcm_data_wire_has_no_notification_block` only uses `priority`.
- **Smallest fix:** never emit `android.notification` for a data message. Carry the badge in `data` instead (for example `lkbadge`).
- **Test:** an FCM data send with a badge and a sound has no `android.notification`.

### 10. Medium: APNs operator misconfiguration invalidates every iOS endpoint, and APNs 403 auth errors end deliveries permanently

- **Confidence:** confirmed.
- **Where:** `leptoskit_notifications/src/service/transport/apns.rs:472-479, 504-509`.
- **Scenario:**
  - A wrong `APNS_TOPIC` returns `DeviceTokenNotForTopic`. A sandbox/production mismatch returns `BadDeviceToken`. Both invalidate each endpoint they hit.
  - Pushes stop until each user relaunches the app and re-enrolls. This contradicts the guide's "only the provider's own unregistered answer invalidates" (`41-notifications.md:761`).
  - A 403 such as `ExpiredProviderToken` (for example from clock skew over 10 minutes) ends each delivery as `provider_rejected`, and the cached token (`apns.rs:147-189`) is never dropped.
- **Smallest fix:** invalidate only on 410 `Unregistered`. Treat 403 as `AuthRejected` and clear the token cache on it.
- **Test:** script these answers and assert endpoints stay live and the token is re-signed.

### 11. Medium: options or specs that pass validation exceed their column bounds, so topic sends get stuck or refused

- **Confidence:** confirmed. `max_len` applies to optional text (`leptoskit_db/src/validate.rs:183-202`).
- **Where:** `notifications.options` is capped at 4,096 chars and `sends.spec` at 8,192 (`leptoskit_notifications/src/lib.rs:211-215, 250-254`).
- **Scenario:**
  - The URL fields alone allow 4×1,024 chars (image, link, web icon, badge icon), so a maximal options value is about 5.7K chars.
  - On a topic, condition or everyone send, `send` answers `fanning_out`. Every fanout step's insert (`service.rs:381-398`) then fails validation until the job dies, leaving the send `fanning_out` with 0 delivered and no error ever shown to the app.
  - A spec over 8,192 chars (4 KB of `data`, escaped, plus a body and options) is refused at `write_fanout_send`, while the same spec sent to recipients succeeds.
- **Smallest fix:** bound the serialized lengths in `check_options` and `check_send_shape`, or widen the columns.
- **Test:** a topic send with large options is refused with 400 at send time.

### 12. Low: idempotency keys of 243–255 bytes pass `check_key` but make the send fail

- **Confidence:** confirmed.
- **Where:**
  - `rpc_jobs.key` is capped at 255 (`leptoskit_rpc/src/server/jobs/mod.rs:131`).
  - The job keys add prefixes: `wake:{key}:visible` (`service.rs:503`), `fanout:{key}:0` (`lib.rs:1082`), `push:{key}:0` (`service/push.rs:445`), and `wake:{key}:fanout:{n}` (`service.rs:1052`).
- **Smallest fix:** hash the send key inside job keys, or cap `MAX_KEY_LEN` at about 200.
- **Test:** a 255-byte key works for both the recipients and topic targets.

### 13. Low: FCM reserved data key `message_type` is not filtered out

- **Confidence:** plausible, based on the FCM v1 rules.
- **Where:** `fcm.rs:591-599`.
- **Scenario:** a send whose `data` contains `message_type` gets a 400 for every FCM delivery, ending `provider_rejected`.
- **Smallest fix:** add `message_type` to the filter.
- **Test:** a send with `data` containing `message_type` keeps that key out of the FCM `data` map.

## The flaky wake test

`wake_jobs_survive_a_restart_and_drain_once_without_spinning` (`tests/live.rs:751-773`) has a race, but it is in the test, not the service.

- The test polls until a `wakes` row exists, then immediately asserts the job is `done`.
- The worker commits the wake row, runs one more page query, and only then records `done` in a separate write (`worker.rs:745`).
- Under load the test sees `running`. The fix is to poll on the job's state instead.

## Areas I examined and found correct

- **Send idempotency:** the stored-send lookup comes before any write. A raced key ends `Contended` and re-runs: SQLite's single writer guarantees that, and Postgres serializable reports a read-then-insert key clash as a serialization failure (40001). The intent hash covers the spec, the target and the recipient set as a sorted set.
- **Fanout cursor and batches:** each page reads one extra row, the cursor is set to the page's last person, the `cursor < last` fence keeps replays moving forward, and the unique `(send_key, person)` key blocks duplicate rows. Counts only move with the advancing commit.
- **Cancel against a running fanout:** the state fence in the step's update holds on SQLite. On Postgres, read committed re-checks the row, and serializable makes the cancel fail and retry.
- **Push outcome fencing:** the endpoint generation is locked, and writes are fenced on state, attempts and generation. Terminal states never regress.
- **Attempt-time checks:** schedule, cancellation, expiry, horizon, attempt count, opt-out and ownership are all checked before sending. Opt-outs bind at send, attempt and inbox; `send_test` ignores them by design.
- **Retry-After:** delta seconds and HTTP dates are both parsed, and a provider's delay is never shortened.
- **APNs headers:** background versus alert push type, priority 5 for background, UUID collapse id (within 64 bytes), expiration as epoch seconds.
- **Web Push:** TTL clamp, Urgency, a 32-character Topic, RFC 8291 encryption matching the RFC's test vector, VAPID audience set to the endpoint's origin.
- **Unread badge:** resolved per recipient from committed state at each attempt. Two concurrent sends can arrive out of order, and reads never push a correction; that is inherent to putting a badge in the payload, not a bug.
- **SQLite vs Postgres:** SQLite timestamps are fixed-width milliseconds, so they order correctly, and UUID cursor and `ORDER BY` order agree.
- **Connections and tokens:** the HTTP client is reused with HTTP/2, redirects off and timeouts set. The APNs token is cached for 50 minutes; the FCM token is cached until 2 minutes before expiry with one exchange in flight at a time.

## What the tests don't exercise that matters most

- A fanout over more than one batch with enrolled endpoints. This would have caught finding 1.
- A step replaying while the live chain runs. The existing replay test deliberately avoids the key collision.
- How many wake writes a large fanout makes.
- Push rounds that exceed the timeout or fail.
- Long mutations running alongside the workers on SQLite.
- Provider outages longer than 2 minutes.
- Real Safari and Edge endpoint URLs.
- FCM data messages carrying a badge or sound.
- The APNs answers `DeviceTokenNotForTopic`, `BadDeviceToken` and 403.
- Maximum-size options and keys sent through a topic target.

## What I could not review

- Live provider rules: FCM's size accounting, whether `android.notification` makes a message display, and Apple's Web Push host. These come from my knowledge, not from fetched docs.
- Postgres behaviour beyond what the code states. I assumed PostgreSQL 9.6 or later.
- The browser receiver JavaScript, the native module, and the live-invalidation internals of `leptoskit_rpc`, which I did not read in depth.
