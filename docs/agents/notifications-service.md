# The notifications service

Built 15 November 2026, against `main`. This is the engineering note for the first slice of first-party notification management: the durable store, the send path, and the inbox. The status of each part is in §8; what follows later is in §9.

It follows the [analytics service](analytics-service.md)'s shape: a service crate whose library holds the operations' types (which apps compile) and whose `service` feature holds their bodies, storage, and binary (which only the service compiles). Nothing here invents a new transport, a management dashboard, or a generic delivery framework — those are non-goals (§7).

**The goal.** Every leptoskit app can add one service, `notifications`, and get durable notification management it owns end to end, instead of a third-party management platform: an app server sends to explicit users, each recipient gets exactly one durable inbox row, and every device signed in as that person sees and mutates the same inbox state.

## 1. What is built

- `crates/leptoskit_notifications`: the crate. Without features, it is the operations' types and the shared validation (`check_send` and friends), with no server dependencies (`tests/dependencies.rs` keeps it that way, for the wasm client tree). With `service`, it is the service; `postgres` adds the Postgres dialect.
- Operations, all under `/_leptoskit/services/notifications`:
  - `send` and `cancel` answer only the app (`caller == "app"`, per `ServiceCallers`), as `rpc::client::call_as_app` calls them.
  - `inbox`, `unread_count`, `category_preferences` are live queries for the forwarded person; `mark_read`, `mark_dismissed`, `set_category_preference` are their mutations.
- Tables: `notifications` (one row per recipient; `live`, indexed `(person, id)`), `sends` (one row per idempotency key, with a SHA-256 intent hash and the counts), `category_preferences` (unique `(person, category)`), plus the `rpc` and `live` packs the service runtime needs.
- Migrations: `migrations/` (SQLite) and `migrations-postgres/`, generated the way `leptoskit db migration create` writes them (schema diff from empty), one `init` each.
- `docs/src/guide/41-notifications.md` is the public page; the "Where things stand" section of `docs/src/readme.md` carries the accurate status.

## 2. Trust boundary

The service takes a `ServiceCallers` exactly as the framework prescribes (`routes_with`), and nothing else:

- **The app** presents `LEPTOSKIT_SERVICE_TOKEN` and is the caller `app`. Only it may `send` and `cancel`; both check `cx.identity().caller` and answer `403` otherwise.
- **A person** is a token the app minted for this service (`forward_identity`, HMAC-derived from the same token, `aud`-bound to `notifications`), surfaced as `ForwardedIdentity`. Every inbox operation resolves the owner with `service::me(cx.identity())` from that context — never from an argument, never from a header. Anonymous calls get `403`; a token for another service, an expired one, or a tampered one gets `401`.
- **There is no arbitrary user-id fallback.** No operation accepts a user id on the recipient side; A reading or mutating B's row is a `404`, indistinguishable from the row not existing.
- The usual forwarding-proxy caveat applies: browsers send the app's cookies to public service paths, so the app must keep its session cookies scoped and its edge honest. This is the same exposure every first-party public service has; `trust = "isolated"` exists for services that must not see cookies, and a deployed notifications service should stay first-party.

## 3. Semantics worth knowing when changing this

- **Idempotency is domain-level, not receipt-level.** `send` writes the `sends` row first (its key is the primary key), then fans out, then records the outcome — all in the mutation's one transaction. A retry re-reads the row: same intent (the canonical hash of `SendSpec` plus the deduplicated recipient set) answers the stored counts with `duplicate: true`; a different intent is a `409`. A concurrent duplicate hits the unique key, re-reads, and compares. Receipts (`rpc_receipts`) remain available to callers through the normal RPC layer; the domain row is what survives any client's forgetfulness.
- **Fanout is bounded at `MAX_RECIPIENTS` (1,000) per send** and inserts one row per recipient. Larger audiences are the app's business (loop with distinct keys).
- **Scheduled visibility and expiry are SQL predicates, not a daemon.** `visible` is `visible_at <= now AND cancelled_at IS NULL AND (expires_at IS NULL OR expires_at > now)`, plus the reader's opted-out categories. Nothing wakes to flip a row; nothing deletes expired rows yet (a later prune job can, without changing behavior).
- **Cancel-before-due:** `cancel` refuses once any row of the send is visible and uncancelled; a fully cancelled send answers `0` whenever it's asked again.
- **Preferences bind at both ends:** the send skips opted-out recipients (counted in `skipped_opted_out`), and the inbox hides rows in categories the reader opted out of since, so an opt-out covers history too. At most `MAX_PREFERENCES` (100) rows per person, so the visibility filter stays a bounded `ne` chain.
- **Row ids are UUID v7**, assigned by the service, so `ORDER BY id DESC` is arrival order and a stable pagination cursor; `notifications` is indexed `(person, id)` for exactly that query. Inbox pages clamp to `MAX_INBOX_PAGE` (100) before the framework's own 1,000-row clamp.
- **Live:** `notifications` is a `live` table, so `inbox` and `unread_count` update watching screens on sends and on cross-device reads and dismissals.

## 4. Tests

`tests/service.rs` runs a real server over a migrated SQLite database: authorization (anonymous, wrong token, forged and foreign-service identity tokens, app-only send/cancel), cross-person isolation, cross-device shared state, idempotency across a file-backed database restart, conflicting key reuse, every bound (keys, titles, bodies, data shape and size, recipient count, schedule horizon and ordering), scheduled visibility arriving, cancelling before due and refusing after, expiry leaving every view, preferences at send and at visibility, three-page pagination with cursor order, concurrent duplicate sends fanning out once, and the migrations themselves (tables and packs exist; the `sends` intent row is written). `tests/dependencies.rs` keeps server crates out of the client tree. The Postgres behavior test is `#[ignore]`d behind `LEPTOSKIT_NOTIFICATIONS_TEST_POSTGRES`, as the analytics tests are; the `postgres` feature is compile-checked.

## 5. What an app that uses this must set

- `[services.notifications]` with `crate = "leptoskit_notifications"` (or a workspace `package`), default `forward_identity` on for first-party.
- The app's server serves `rpc::server::Server` with its sign-in `Identify` (the scaffold's shape) so forwarded-identity tokens can be minted.
- Sending needs the service's token on the server side, which `leptoskit dev`/`serve` and deploys provide as `LEPTOSKIT_SERVICE_NOTIFICATIONS_TOKEN`.

## 6. Performance envelope

SQLite in the service's own file handles the target load — badge polls are one indexed count, inbox pages are one indexed range read per page, sends are one transaction with up to 1,000 small inserts. Postgres is the same code with the other dialect. Nothing here is measured against a production-sized corpus yet; before claiming large-fanout numbers, run the load harness the analytics service has.

## 7. Non-goals

- **No OS push transports** (APNs, FCM, Web Push), no device registry, no tokens-at-rest for vendors. The next slices add transports; this crate only stores and serves.
- **No management dashboard**, no admin UI, no generic "transport" trait with speculative backends.
- **No polling daemon** for visibility or expiry; SQL predicates cover both.

## 8. Status

| Part                                   | Status                                                 |
| -------------------------------------- | ------------------------------------------------------ |
| Service crate, storage, ops            | Built and tested on SQLite; Postgres compile-checked   |
| App-only send/cancel                   | Built, behind the `ServiceCallers` trust boundary      |
| Durable per-person inbox               | Built: live queries, bounded pages, cross-device state |
| Idempotency across restart             | Built: file-backed restart test proves it              |
| Categories + opt-outs                  | Built: enforced at send and at visibility              |
| Scheduled visibility + cancel          | Built: SQL visibility, cancel-before-due               |
| Expiry                                 | Built at visibility; no physical prune yet             |
| OS background push (APNs/FCM/Web Push) | Not started; subsequent task                           |
| Native enrollment                      | Not started; subsequent task                           |
| Delivery receipts                      | Not started                                            |
| Physical prune of expired rows         | Not started                                            |

## 9. What follows

The transports land as separate slices on top of this store: device enrollment (a `devices` table keyed by person, with platform credentials at rest), a Web Push sender the service process runs (Web Push needs no vendor contract and serves the browser first), then APNs/FCM senders that read pending, undelivered rows. Composing with the local-notifications plugin is documented for the surfaces where it exists; Windows toasts wait on the plugin's backend.
