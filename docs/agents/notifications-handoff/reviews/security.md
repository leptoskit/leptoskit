# Review: security and privacy

Read-only review of `main` at `93e2a2af0` on 18 November 2026. The reviewer built nothing and ran nothing, so every item is a claim: confirm it with a failing test before fixing it. Line numbers are from that commit. [The handoff](../readme.md) says which findings are already being fixed.

I found no critical or high-severity issues. The authorization model holds: every app-only operation checks the app caller, and every person-side operation takes the person from the forwarded identity. The Web Push host policy and the crypto are also sound. The real defects are in endpoint ownership and in what `forget_person` leaves behind, plus three low-severity resource issues.

As asked, I only read code: no builds, no tests, no edits, no git changes.

## Findings (most severe first)

### 1. Medium: a device enrolled by an account that never revoked keeps getting that account's pushes, and nothing on the server can release it

**Confidence:** confirmed by reading the full path. Whether it happens depends on the app signing someone out without an acknowledged revoke.

**Where:**

- `src/service/push.rs:153-206`: any destination owned by another person is refused with a conflict, even when the provider has invalidated it.
- `src/service/push.rs:328-344`: only the owner can revoke, using their forwarded identity.
- `src/service.rs:1589-1604`: `forget_person` leaves push endpoints in place.
- No app-only operation revokes another person's endpoints.
- The doc at `src/push.rs:690-697` says an invalidated destination "takes it over clean". The code (`service/push.rs:204`) never does.

**Scenario:**

1. A enables notifications on a shared browser or device.
2. A's session ends without a working `disable`: cookie or session expiry, a server-side logout, account deletion, or signing out while offline. The revoke needs A's forwarded identity, which no longer exists.
3. B signs in on the same browser or app install. `reconcile`/`enroll` sends the identical subscription or token and gets "this device belongs to another account".
4. Every later send to A still goes to that device. The OS or the receiver worker shows A's titles and bodies to B.
5. B cannot enroll. Neither B nor the app's server can end this; only A signing in on that device again can.
6. After `forget_person`, the device stays locked to a person who can never sign in again.

**Smallest fix:**

- In `register`, when `existing.person != me` and `existing.credential == canonical` (byte-identical, so possession is proven; for Web Push this includes the keys), revoke the old row, skip its queued deliveries as `EndpointRevoked`, and insert a fresh endpoint for `me`.
- Add an app-only `revoke_person_endpoints(person)` and call it from `forget_person`.
- Fix the doc comment.

### 2. Medium: `forget_person` keeps the person's data, and the client's documented launch-time reconcile undoes it

**Confidence:** confirmed.

**Where:**

- `src/service.rs:1589-1604` deletes only attributes, topic subscriptions and the registry row.
- These person-initiated acts re-register the person:
  - `src/service/push.rs:194` and `:236` (any enrollment, including the idempotent same-token one)
  - `src/lib.rs:1417` and `:1452` (category preferences)
  - `src/service.rs:1334` (self-subscription)
- The clients call enrollment on every launch: `browser.rs:416-434` and `native.rs:317-336`.
- The guide (`docs/src/guide/41-notifications.md:251-257`) offers it for "a deleted account, a withdrawn consent".

**Scenario:**

1. Alice withdraws consent, and the app calls `ForgetPerson(alice)`.
2. On her next launch her client calls `browser::reconcile` or `native::reconcile`, as the guide instructs. That runs `register_push_endpoint` and `register_persons`, so she is back in `audience_members`.
3. The next `SendTarget::Everyone` send writes her an inbox row and pushes to her devices.
4. Even setting a category opt-out re-registers her.

For a deleted account, the service keeps forever:

- the plaintext push credentials
- every inbox row (title, body, data)
- preferences, delivery rows and the wake row

No API lets the app erase any of it.

**Smallest fix:**

- Record a tombstone, either `forgotten_at` on the registry or a small table, that `register_persons` respects for person-initiated acts. Only the app-only attribute and subscription calls, or an explicit app call, clear it.
- Have `forget_person`, or an `erase` flag on it, delete the person's endpoints, skipping their queued deliveries as revoked. It should optionally delete their notifications, preferences and wake row too.

### 3. Low: a person can grow `push_endpoints` without limit by getting fake endpoints invalidated

**Confidence:** the code path is confirmed. That APNs answers `BadDeviceToken` for random tokens is documented provider behaviour; I did not test it.

**Where:**

- `src/push.rs:307-320` accepts any 32–200 hex characters as an APNs token.
- `src/service/push.rs:248-262` counts only live endpoints toward the limit of 10.
- `apns.rs:472-480`: `BadDeviceToken` invalidates the endpoint (`service/push.rs:1011-1028`).
- Invalidated rows are kept until the owner revokes them (`push.rs:33-35`).

**Scenario:**

1. A signed-in user enrolls 10 random hex tokens.
2. Each notification to them (one they can trigger with a second account, for example) makes 10 APNs requests. All 10 come back invalidated, which frees the slots.
3. They enroll 10 more and repeat.

The `push_endpoints` and `push_deliveries` rows grow without bound, and each round spends APNs requests under the operator's key. Web Push subscriptions the user unsubscribes (which produce 410) work the same way.

**Smallest fix:** cap total rows per person at `MAX_ENDPOINTS + MAX_ENDPOINT_HISTORY` at enrollment, deleting the oldest invalidated row when over the cap.

### 4. Low: the `until` page argument bypasses the inbox page limit

**Confidence:** confirmed.

**Where:** `src/lib.rs:1268-1301` clamps `limit` to 100. But `rpc::paginate` (`crates/leptoskit_rpc/src/page.rs:178-193`) ignores `limit` whenever `until` is set and reads up to `MAX_PINNED_ROWS` = 10,000 rows. The same applies to `push_endpoints`, which stays unbounded because of finding 3.

**Scenario:** a person calls `inbox` with `until` set to the smallest UUID. The service reads and serializes up to 10,000 of their own rows, each up to about 6 KB (around 60 MB). As a live query it re-runs on every change. It is their own data only, so the cost is to server resources.

**Smallest fix:** let `paginate` take a pinned-row cap, or refuse `until` pages larger than a few times `MAX_INBOX_PAGE` in these operations.

### 5. Low: the condition parser's recursion is bounded by input length, not by `MAX_CONDITION_DEPTH`

**Confidence:** plausible. The stack overflow depends on frame sizes, which I could not measure without running anything.

**Where:** `src/condition.rs:432-460`. `(` recurses through `expression`, `conjunction` and `unary` without adding to the depth count. `!` recurses first and calls `bounded` only after returning (`:875-884`).

**Scenario:**

- About 1,000 `(` characters fit under the 1 KiB limit. That makes roughly 3,000 nested frames before any check runs.
- That can overflow a 2 MiB tokio worker stack in a debug build, which aborts the whole service rather than panicking.
- A long run of `!` recurses about 1,000 deep before it is refused.
- Only the app's server can submit a condition, so the risk applies when an app passes through staff- or user-written condition text.

**Smallest fix:** keep a nesting counter in `Parser`. Increment it on `(` and `!` and refuse past a small bound before recursing.

## Areas examined and found sound

- **Caller authorization:** every app-only operation compares the caller to `APP_CALLER`. Every person operation takes the person from `ForwardedIdentity`. `ServiceCallers` rejects inner identities that claim the reserved callers. The forwarded-identity check verifies the HMAC in constant time, plus `aud`, `exp` and a maximum lifetime.
- **Operations that take an id:** `mark_read`/`mark_dismissed` use `own_row`, filtered by person. `revoke` is filtered by person and answers the same not-found for "not yours" and "missing". `send_test` filters the endpoint by person.
- **Send keys and idempotency:** only the app can call `send`, `cancel`, `send_status` and `push_deliveries`. RPC receipts are owned per caller (policy and RLS on `owner`), so the `notifications:disable:{id}` keys can't be poisoned across people.
- **Server-side requests (SSRF):** a Web Push endpoint must be https, on port 443, with no userinfo or fragment. The shipped hosts are matched exactly; lookalikes, a trailing dot and userinfo are refused. Extra hosts are DNS-pinned against the full special-purpose address registries, and the policy is re-checked at every attempt. The HTTP client refuses redirects, ignores proxies, sets timeouts and bounds body reads. APNs and FCM URLs are fixed, a device token in a URL path is hex only, the FCM token is JSON-encoded, and `token_uri` is ignored.
- **Secrets:** the `credential` column is `secret` (redacted `Debug`, kept out of the change log). The credential and wire types redact their `Debug` output. Provider bodies are read only to classify the answer, and statuses are sanitized codes. Startup errors name no key material. Job arguments carry only the send key and round.
- **Crypto:**
  - Each message draws a fresh salt and a fresh ephemeral P-256 key from `SystemRandom`.
  - The RFC 8291 HKDF chain matches the Appendix A test vector.
  - There is one record, the record size is 4096, and the body is at most 4096 bytes.
  - VAPID: `aud` is the endpoint's origin, `exp` is 12 hours (under the RFC's 24-hour cap), and `sub` is validated.
  - The APNs ES256 token is cached for 50 minutes (APNs accepts up to 60).
  - The FCM RS256 assertion has a fixed `aud` and scope, and its token is cached until `expires_in` minus 2 minutes, with bounds checked.
  - There is no padding, so the push service can see plaintext length. RFC 8291 allows this.
- **Condition evaluation:** at most 5 topics, 10 predicates, depth 8 and 1 KiB. Conditions that would match everyone are refused, `topic_anchored` is conservative, and each batch loads only the topics and keys the condition names.
- **Open topics:** the glob matcher is correct (leftmost middles, non-overlapping suffix). `*` is not a topic character, and nothing is open by default. `*` spans dots, which the guide documents.
- **Client:** the service worker honors a link only when it resolves to its own origin, so `javascript:` and `data:` fail. The route is baked in as a JSON literal and re-checked at click time. Payload fields are typed and bounded. The Swift and Kotlin hosts never act on `link` or `data`. App `data` can't override envelope keys: it is nested under `lk` for APNs and Web Push and filtered for FCM.
- **Person-side limits:** at most 100 preferences, 500 topics (open topics only for self-subscription), and 10 live endpoints.

## Not reviewed

- Anything that needs execution: real stack depth, live provider answers.
- `leptoskit_db` internals: how `secret` is excluded from the audit log, and the RLS details.
- The live-socket internals beyond the identity lease, and receipt pruning.
- The Postgres migrations, and the `lk` CLI wiring for `trust` and the environment.
- The Swift and Kotlin hosts beyond confirming they don't handle links and don't log tokens.

## Not security, noticed in passing

- **Topic or everyone sends may push only to roughly the first batch (plausible, depends on a race).** `fanout_step` re-enqueues the push job with key `push:{key}:0` (`service/push.rs:535-551`). The job layer returns any existing job with that key, finished ones included, for the 24-hour retention (`leptoskit_rpc/src/server/jobs/mod.rs:479-486`). So if round 0 of an immediate send finishes before later batches commit, those batches' deliveries are never attempted. No test covers push on a topic send.
- **Safari is probably refused.** The shipped host `webpush.apple.com` (`host.rs:32`) looks wrong; Safari endpoints are `web.push.apple.com`. Worth checking against a real Safari subscription.
- **Wrong status label.** A Web Push p256dh value that isn't a point on the curve ends the delivery as `Oversized`, because all four build stages fail and the loop falls through.

PRs: none opened or touched in this review.
