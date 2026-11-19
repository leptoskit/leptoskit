---
"leptoskit_notifications": minor
---

# List sends, size an audience, and preview a push

A screen for composing notifications had nothing to ask the service: it could not list what was sent, could not show which topics or attributes exist or how many people a target would reach, and could not show a push before it went out. The service now answers all three, to your app's server only.

- `Condition` implements `Display`. `to_string()` writes one canonical spelling of a parsed condition, and parsing that text gives back the same tree. A screen that builds a condition as a tree can hand the service text without writing its own quoting.
- `list_sends` is a new app-only query. It answers the sends made since this version, newest first, in pages of at most 100, each as a `SendEntry` with its key, state, label, category, title, kind, target, schedule and counts. `SendFilter` keeps one state.
- `send_report` answers four more fields on `SendSummary`: `spec` (the send as it was made), `target` (a `TargetSummary`), `push` (deliveries per platform by state, with the reasons the ended ones ended) and `reads_by_hour` (reads per UTC hour, up to 168 hours).
- The new `catalogue` module has seven app-only queries: `notifications_overview`, `topics`, `attribute_keys`, `attribute_values`, `categories`, `estimate_audience` and, in `push`, `person_endpoints`. Lists are pages of at most 50. Every count is a `Count { count, exact }` that stops at a cap. `estimate_audience` walks at most 5,000 people and says whether its answer is exact. A device that was revoked or stopped working is not counted as reach, and no answer carries a credential.
- `preview_push` in the new `preview` module is an app-only query. It answers, for APNs, FCM and Web Push, the headers and the exact body a provider would be sent for a spec, the bytes counted against the provider's limit, and what was trimmed to fit. It is built by the code that sends, reads no credential, and needs no push configuration.

**What changes for an existing app:**

- The `send_history` migration adds the nullable `sends.history_id` and `sends.category` and five indexes, on SQLite and Postgres, with no backfill. Sends made earlier keep their reports and are not listed. On a large Postgres database the indexes can be built ahead with `CREATE INDEX CONCURRENTLY`; the guide's "Upgrading a large database" says how.
- A recipients send now stores its spec with its record, as topic and condition sends already did, until the record's retention deletes it.
- `SendSummary` no longer implements `Eq`, because it now holds a `SendSpec`. `AppSend` gains two fields, which breaks struct literals of it.
- Inside the `service` feature, `transport` requests are now built by a `shape` function per provider. What is sent is unchanged.
