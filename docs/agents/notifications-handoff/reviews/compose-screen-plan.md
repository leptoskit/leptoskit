# Plan: a notification composer in the admin

Read-only review of `main` at `93e2a2af0` on 18 November 2026. The reviewer built nothing and ran nothing, so every item is a claim: confirm it with a failing test before fixing it. Line numbers are from that commit. [The handoff](../readme.md) says which findings are already being fixed.

The composer can't be built on today's admin as it stands. The admin is server-rendered HTML driven by 1,149 lines of hand-written JavaScript (`assets/admin.mjs`, loaded from `shell.rs:781-792`). The notifications service has no listing, draft, catalogue or report operations. The groundwork divides cleanly into three parallel tracks: service changes (can start now), framework widgets (can start now), and the admin's move to Wasm (another line of work). The composer comes after all three.

## File index

Paths below use these short names. Line numbers refer to `origin/main` at `93e2a2af0`.

- **Root:** `the repository root`
- **N-lib:** `{root}/crates/leptoskit_notifications/src/lib.rs`
- **N-service:** `{root}/crates/leptoskit_notifications/src/service.rs`
- **N-push:** `{root}/crates/leptoskit_notifications/src/push.rs`
- **N-cond:** `{root}/crates/leptoskit_notifications/src/condition.rs`
- **N-opts:** `{root}/crates/leptoskit_notifications/src/options.rs`
- **N-apns, N-fcm, N-webpush:** `{root}/crates/leptoskit_notifications/src/service/transport/{apns,fcm,webpush}.rs`
- **A-lib, A-server, A-access, A-shell, A-components, A-style, A-workings:** `{root}/crates/leptoskit_admin/src/{lib,server,access,shell,components,style,workings}.rs`
- **A-aclient, A-aroutes, A-alive:** `{root}/crates/leptoskit_admin/src/analytics/{client,routes,live}.rs`
- **A-mjs:** `{root}/crates/leptoskit_admin/assets/admin.mjs`
- **RPC-client:** `{root}/crates/leptoskit_rpc/src/client/mod.rs`
- **RPC-callers:** `{root}/crates/leptoskit_rpc/src/server/services.rs`
- **AUTH-http:** `{root}/crates/leptoskit_auth/src/http.rs`
- **IDP-tokens:** `{root}/crates/leptoskit_idp/src/tokens.rs`
- **CLI-admin:** `{root}/crates/leptoskit_cli/src/admin.rs`
- **NS-doc:** `{root}/docs/agents/notifications-service.md`
- **AD-doc:** `{root}/docs/agents/admin-design.md`

---

## 1. How the admin reaches the service

### How analytics does it today

- **The browser never calls the service.** The admin's own server calls it at a private URL, signed with the service's token (`AnalyticsService`, A-lib:141-193). `Connection::new` builds a `leptoskit_rpc::client::AppCaller` (A-aclient:71-84, RPC-client:538-598). The token stays on the server; the tests check what the service saw (`rpc_authorizations`, `tests/support/mod.rs:701-710`).
- **Access checks.**
  - Pages go through `server::gate`, which checks the person is signed in and holds one of the configured roles (A-server:124-159, role test at :142).
  - The API goes through `api_gate` (A-server:166-204).
  - Roles come from the main site's `roles` claim, stored per subject at sign-in (`AdminGate`, A-access:117-164) and read back by `AdminRoles` (A-access:168-198). A role removed on the main site only takes effect at the next sign-in (A-access:7-8).
  - Unsafe methods (POST and so on) are also refused when they come from another site: the session layer enforces this (AUTH-http:401-452, applied at :726-746).
- **What identity the call carries.** None beyond "app". `ServiceCallers` turns a known token into `Identity::anonymous().with_caller("app")` (RPC-callers:272-273). The service cannot see which admin acted.
- **How actions are recorded.**
  - Infrastructure actions are recorded with the admin's name through `Principal::owner().with_actor(actor)` (A-server:627).
  - Analytics passes the name as an ordinary argument, `created_by` (A-aroutes:462-468).
  - Deleting an analytics person records nobody (A-aroutes:700 sends only `{ id }`). This is an existing gap worth knowing about.

### How notification operations are authorised

- `send`, `send_status`, `cancel` and `send_test` each compare `cx.identity().caller` with `APP_CALLER` (N-lib:990, 1126, 1179, 1752). `push_deliveries` does the same (N-push:774).
- `send_test` writes nothing: no rows and no record (N-lib:1717-1722). It also accepts any `person`. Today, a test push to someone else's phone leaves no trace.

### The mechanism the composer should use

The Wasm composer only ever calls the admin's own server (same origin, session cookie). The admin's server checks the session, the admin role and a capability role, then calls the service as the app through `AppCaller` with `LEPTOSKIT_SERVICE_NOTIFICATIONS_TOKEN`. This is the analytics pattern unchanged.

The browser-to-admin hop should use typed admin operations rather than new JSON handlers. That means `#[rpc::query]`/`#[rpc::mutation]` operations served by `rpc::server::Server`, identified by `Auth`. `Auth` already implements `Identify` behind `leptoskit_auth`'s `rpc` feature (AUTH-http:2227-2262); the admin doesn't enable that feature today. This keeps the Wasm client free of untyped glue, and lets the Workings drawer show real types on both hops.

| What the composer does             | Admin operation (new)                   | Service call                                                                 |
| ---------------------------------- | --------------------------------------- | ---------------------------------------------------------------------------- |
| Live audience count while building | `EstimateAudience`                      | new `estimate_audience` (§2)                                                 |
| Exact count at review              | `CheckAudience`                         | `Send { validate_only: true, … }`                                            |
| Send or schedule                   | `Launch { id, revision, confirmation }` | new `launch_campaign`, which runs `send`'s own logic in the same transaction |
| Test on my device                  | `TestOnDevice { campaign, endpoint }`   | new `test_campaign`, which wraps `send_test` and records it                  |
| Progress                           | `CampaignProgress`                      | `send_status` / new `send_report`                                            |
| Cancel                             | `CancelCampaign`                        | new `cancel_campaign`, which wraps `cancel` and records who                  |

### What is missing for that

1. **Connection settings.**
   - `NotificationsService` plus `AdminBuilder::notifications(…)`, reading `LEPTOSKIT_SERVICE_NOTIFICATIONS_URL` and `_TOKEN`. This mirrors `AnalyticsService` (A-lib:148-193; builder at :312-315; `Inner` at :423-435).
   - `lk admin serve --notifications-url`, mirroring `--analytics-url` (CLI-admin:58-110).
2. **Capability roles.** The admin has a single gate today: any of `roles` gets in (A-server:142, :186). Proposed `AdminBuilder::notification_roles`:
   - **Read:** the existing admin roles.
   - **Compose** (drafts, previews, estimates, test on your own device): the admin roles by default.
   - **Send** (launch, cancel, test on someone else's device): the admin roles by default.
   - **Broadcast** (`Everyone`, or an audience over a configurable threshold, 10,000 by default): `notifications.broadcast`, granted to nobody by default. The confirm sheet says how to grant it, in the same spirit as the sign-in page's "How admins are chosen" note.
   - This default is a decision for the owner.
3. **Admin-side typed operations and the Wasm client foundation.** This depends on the other line of work (see §5).
4. **A record of who acted, on the service side.** Campaign operations take `by: String`, following analytics' `created_by` convention. The admin's server fills it from the session and never from the browser. Do **not** add `by` to `send`. Apps build `Send { … }` as a struct literal (guide `41-notifications.md:46-58`), so a new field breaks every caller.
5. **Mapping the admin to a person for "test on my device".** The admin is a first-party client, so its OIDC `sub` is the main-site account ID (IDP-tokens:1128-1136). The admin's `Identity.subject` for provider `main` can be read the way `display_name` reads the identity (A-access:202-218), parsed as a UUID, and passed as `person`. This only holds when the app uses account IDs as notification persons, which the forwarded-identity design implies (RPC-callers:357-361). Otherwise the screen falls back to picking a person.
6. **An app-only `person_endpoints`.** Today `push_endpoints` answers only for the forwarded person (N-push:728-735), so the admin can't list its own devices to choose one.

### Security boundary

- **The token** never leaves the admin's server.
- **Same-site checks** apply to unsafe methods (the session layer).
- **Roles:**
  - A role check is `inner.auth.roles(account)` against the configured capability roles.
  - Broadcast should also require a fresh sign-in. That needs framework work: leptoskit_auth's OIDC provider sends only `prompt=select_account` (`crates/leptoskit_auth/src/oauth.rs:1404`); there is no `max_age` or `prompt=login`.
- **The service** re-checks everything:
  - `check_send` and the condition parse.
  - A launch key `campaign:{id}:r{revision}`, so a double press is answered as a duplicate.
  - Refusing a campaign whose scheduled time has passed. The service treats a past `visible_at` as now (N-lib:403), so a stale scheduled draft would otherwise go out immediately.
- **Recording:**
  - Campaign columns `created_by`, `updated_by`, `launched_by` and `cancelled_by`.
  - An append-only `campaign_events` table that also records test sends.
  - The results screen shows this trail.
- **Optional later framework step:** `AppCaller::acting_for(label)`, accepted only alongside the app token, so services can audit without an extra argument on every operation. This changes the trust model and needs its own review.

---

## 2. What the service is missing

All new operations are app-only, using the existing check and its "only the app …: call this from its server, with the service's token" wording. Paging uses `rpc::PageArgs`/`rpc::Page`. `rpc::paginate` needs a unique cursor column (`crates/leptoskit_rpc/src/page.rs:128-156`). Grouped lists use raw `Statement` SQL, as `candidates` already does (N-service:743-819).

| Operation                                      | Shape                                                                                                                                                                                                                                                                                                                                                                                         | Migration                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| ---------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `sends` (query)                                | `(page, #[serde(default)] filter: SendFilter { origin: campaign or server, state, campaign })` → `Page<SendSummary { key, created_at, state, category, title, kind, target: TargetSummary { kind, text, recipients }, visible_at, expires_at, targeted, delivered, skipped_opted_out, campaign, created_by }>`                                                                                | **Yes.** `sends` gains a UUID v7 `id` (the cursor), `category`, `title`, `kind`, `target_kind`, `target_text`, `visible_at`, `expires_at`, `campaign` and `created_by`. Recipients sends currently store `spec: None` (N-service:210-230), so `spec` must start being written for them too. Keep `target` NULL for recipients sends: `answer_send` works out a recipients send by `target` being absent (N-service:305-308). Older rows aren't listed. The service is three days old; backfilling through a job is the alternative. |
| `send_report` (query)                          | `(idempotency_key)` → `SendReport { progress, spec: Option<SendSpec>, target, created_at, inbox: { rows, read, dismissed, cancelled }, push: Vec<{ platform, enqueued, retrying, accepted, rejected, skipped, reasons }>, reads_by_hour }`                                                                                                                                                    | No. It uses `unique(send_key, person)` (N-lib:167-172) and `index(send_key, id)` (N-push:468-474). Inbox reads (`read_at`) are a real metric today. Opens come from the future open-tracking work, so fold into that work's per-send statistics if it already defines them.                                                                                                                                                                                                                                                         |
| `notifications_overview` (query)               | → `{ people_known, people_with_push by platform, transports configured, open_topic_patterns, topics }`                                                                                                                                                                                                                                                                                        | No. Transports come from `PushRuntime::configured` (`transport/mod.rs:553`).                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `topics` (query)                               | `(page, prefix)` → `Page<{ topic, subscribers, open }>`                                                                                                                                                                                                                                                                                                                                       | No. Uses `index(topic, person)` (N-lib:270-275).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| `attribute_keys`, `attribute_values` (queries) | `(page)` → `Page<{ key, people }>`; `(key, prefix, limit ≤ 50)` → `{ values: Vec<{ value, people, targetable }>, more }`                                                                                                                                                                                                                                                                      | **Yes**, an index on `person_attributes(key, value)`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| `categories` (query)                           | `(limit)` → `Vec<{ category, opted_out, last_sent }>`                                                                                                                                                                                                                                                                                                                                         | An index on `category_preferences(category)`; also reads the new `sends.category`.                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| `estimate_audience` (query)                    | `(target, category)` → `{ targeted, skipped_opted_out, delivered, exact, reach: Vec<(PushPlatform, u32)> }`. Walks at most a fixed number of candidates, then answers `exact: false`. `validate_only` walks the whole audience (N-service:570-625), which is too costly to run on every keystroke.                                                                                            | No.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| `person_endpoints` (query, in N-push)          | `(person, page)` → `Page<EndpointSummary>`, no credential                                                                                                                                                                                                                                                                                                                                     | No. Uses `index(person)` (N-push:428).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| `preview_push` (query)                         | `(spec)` → `Vec<{ platform, headers, body, bytes, limit, trimmed, fits }>`. Built by the transports' own shaping code, without credentials. Web Push shows the envelope before encryption.                                                                                                                                                                                                    | No. Needs the pure shaping pulled out of `attempt` (APNs trim loop N-apns:262-288, `PAYLOAD_LIMIT` :44; FCM :62 and :341-350; Web Push `envelope` N-webpush:517).                                                                                                                                                                                                                                                                                                                                                                   |
| Campaigns                                      | `create_campaign(draft, by)`, `update_campaign(id, revision, draft, by)`, `campaign(id)`, `campaigns(page, filter)`, `duplicate_campaign(id, by)`, `duplicate_send(key, by)`, `archive_campaign(id, revision, by)`, `launch_campaign(id, revision, by)` → `{ campaign, send: SendResult }`, `cancel_campaign(id, by)`, `test_campaign(id, person, endpoint, by)`, `campaign_events(id, page)` | **Yes**, two tables: `campaigns` and `campaign_events`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |

### More detail on campaigns, drafts and duplicates

- **Where drafts live.** Not in the admin's database: it holds only sessions, roles and preferences, and is documented as safe to throw away (A-access:10-12, :326-330).
- **Shared types** go in a new `src/campaign.rs`, compiled without features so they also build for Wasm.
  - `CampaignDraft { name, spec, target: Option<SendTarget>, schedule: Schedule, note }`.
  - A lenient `check_draft` allows an empty title or no audience while drafting; `check_send` runs at launch.
- **Stored state** is one of draft, launched, cancelled or archived. The status shown (Scheduled, Sending, Sent) is derived from the linked send.
- **`launch_campaign`** needs `send`'s body (N-lib:982-1095) pulled out into `service::send_in` so both can share it.
- **Duplicating a recipients send** copies the content only; the audience is left empty. The send's 1,000 IDs don't fit the 1,200-byte `target` column (N-lib:250).

### Other service gaps the builder will hit

- **Conditions can't be turned back into text.** `Condition` has no `Display` (only `ConditionError` does, N-cond:153). Add a canonical renderer that round-trips through `parse`.
- **Values with spaces can't be targeted.** Quoted values reject whitespace and `'` (N-cond:689-692), but attribute values may contain both (N-lib:796-808). `country = "United Kingdom"` can be set but can never be matched. Extend the value grammar with escapes, or mark such values `targetable: false`.
- **"Everyone except X" isn't expressible.** Conditions that would match someone with no topics and no attributes are refused (N-cond:216-223), and `Everyone` takes no condition. This is a common Firebase segment; the owner needs to decide whether to support it.
- **`sends` is not a `live` table** (N-lib:222), so progress can't push to an open screen even once the admin has live support.
- **Update NS-doc.** §7's "No management dashboard, no admin UI" (NS-doc:65) needs revising.

---

## 3. The screens

The concept is **proofs and a postmark**, keeping to the admin's paper and ink. You write the message on paper (the manuscript), see how each device prints it (proofs), say who it goes to in a sentence (the address), and when (the postmark).

Underneath, the Workings drawer keeps the admin's five-station shape (AD-doc §1, lines 22-32). Its stations are **Write · Address · Check · Fan out · Deliver**:

- **Write:** the `SendSpec` as Rust.
- **Address:** the `SendTarget` and the condition text, plus which index the fanout walks: the topic index or the whole registry, from `topic_anchored` (N-cond:295-297).
- **Check:** `check_send` and the `validate_only` call with its counts.
- **Fan out:** the `sends` row, about ⌈N/500⌉ `fanout_send` batches (`FANOUT_BATCH`, N-lib:149), and the wake jobs.
- **Deliver:** the real APNs, FCM and Web Push payloads, their bytes against 4,096, and the trim order.

**Copy as Rust** produces `call_as_app(leptoskit_notifications::Send { … })`. The generator must write `options` explicitly, because `SendSpec` has no `Default` (N-lib:394). The guide's own snippet leaves `options` out (guide `41-notifications.md:46-55`).

### Information architecture

- A **Messaging** group in the rail between Analytics and Infrastructure (`Section`, A-shell:68-83; rail :431-552), with **Notifications** and **Audiences**. A fourth area in the phone tab bar (A-shell:554-617).
- Routes:
  - `…/notifications?tab=drafts|scheduled|sent|server`
  - `…/notifications/new`, which creates a draft and redirects
  - `…/notifications/campaigns/<id>/edit`
  - `…/notifications/campaigns/<id>` (results)
  - `…/notifications/sends/<key>` (a send made by the app's server)
  - `…/notifications/audiences`

### The send list

- **Header:** "Notifications" and one primary raised button, **Write a notification**.
- **Four stat tiles, all real numbers:**
  - people you can reach (the registry);
  - people with push, by platform;
  - sent this week;
  - inbox read rate over 30 days.
- **Tabs** (`components::tabs`, A-components:561-602): Drafts · Scheduled · Sent · From your server.
- **Rows** (two lines at 400px):
  - a status word with its icon (`status_word`, A-components:50-54);
  - the name, or the idempotency key in mono for server sends;
  - the audience in words ("Topic `news.sports`", "Everyone", "pro plan in GB");
  - when, delivered, read %, and who sent it ("your server" when there is no campaign).
- **Row overflow menu:** Duplicate, Cancel (danger style, only while cancellable), Archive (drafts only).
- **The "From your server" tab** teaches with a margin note: these came from `call_as_app(Send { … })`.
- **Empty state:** the existing teaching pattern (`empty_state`, A-components:606-633), showing the send code beside the button.

### The composer

A document, not a wizard. Two columns at `xl`, stacked on phones. Pinned chrome comes from layout, since `sticky` is rejected (AD-doc:86).

**Left: four numbered cards**, each with a mint tab to its station, and a margin rail echoing the Workings rail.

1. **Message**
   - Internal name; category (combobox) with the opt-out count ("1,204 opted out of `marketing`").
   - Title and body with **byte** counters: the limits are in bytes (N-lib:97-101).
   - Image URL, with the caveat that iOS needs a Notification Service Extension (N-opts:293-296).
   - Link: warn when its host isn't the app's own.
   - Language tabs and A/B variants once those features land.
   - An **Advanced** disclosure (`leptoskit_aria::DisclosureButton`) holding:
     - sound, badge (number or "their unread count") and priority;
     - silent data message, with the browser-penalty warning (guide `41-notifications.md:411`);
     - data JSON;
     - per-platform blocks, e.g. an Android channel must exist (N-opts:220-222).
2. **Audience**
   - Four choices in plain words: Everyone · A topic · People who match… · Specific people (Advanced; up to 1,000 IDs).
   - **"People who match…" is a sentence builder:** _"People subscribed to [news.sports] **and** whose [plan] [is] [pro]"_, plus "unless" clauses and one level of grouping.
   - The value picker shows counts and marks untargetable values.
   - Under the sentence, the condition text in mono on a sunken well, with Copy. Changes are washed mint, and errors underline the byte offset the local `Condition::parse` reports.
   - **Write it yourself** switches to the text with a live parse.
   - **The audience meter:** "About 48,213 people · 46,900 after opt-outs · 41,002 with a device (iOS 12,040 · Android 25,170 · Web 3,792)". Debounced with `use_debounce_fn` (`crates/leptoskit_use/src/time.rs:122`), and marked "at least" when `exact` is false.
   - **Everyone** carries a note on what "everyone" means: people known through an act, not every account (N-lib:306-316).
3. **Delivery**
   - When: Now · At a time (date, time and zone) · In each person's time (future work).
   - Expiry.
   - **Test on a device:** your own devices from `person_endpoints`, then per-endpoint outcomes as status words with a plain explanation of each sanitized reason.
   - When push isn't configured, say so and name the variables (N-service:1703-1707).
4. **Review**
   - A one-sentence manifest: "You're about to notify **48,213 people** on iOS, Android and the web, now. 1,204 opted out of `marketing` and won't get it."
   - A checklist: counted N minutes ago; tested on a device, or not; link host.
   - The typed `Send` call.
   - The button is a verb with an object: **Send to 48,213 people** or **Schedule for Tue 09:00** (voice, AD-doc:274-282).

**Right: proofs.**

- iOS lock screen, Android shade (collapsed and expanded), web/macOS banner, and the **inbox row**. The durable row is what push only knocks for; that framing is leptoskit's own.
- Silent data shows a dashed outline: "Nothing shown — your app's code receives `data`."
- Proofs are ink and paper, never mint: mint means machinery, never data (AD-doc:14-16).
- Each proof's mint tab opens Deliver with the `preview_push` bytes.
- Line clamps are labelled approximate.

### Confirm sheet

Follows the design doc's confirm sheet (AD-doc:226).

- One sentence of what will happen, a five-line Workings summary, and the environment pill.
- For Everyone or a large audience: type the exact number to confirm, plus a fresh sign-in once the framework supports it.
- Busy state "Sending…", then a quiet "✓ Sent to 48,213 people. Fanning out: 12 of 97 batches."

### Results

- **Header:** status word, "Sent Tue 18 Nov 09:00 by ana@example.com"; Duplicate; Cancel only while allowed.
- **A delivery ledger in two honest units.**
  - **People:** matched → inbox row (opted-out shown hatched as a loss) → read (and opened, once tracked). This reuses the analytics funnel's visual language (AD-doc §6).
  - **Devices:** push deliveries by platform, each as status words. Every reason code gets a plain-language line and what to do, with "accepted means the provider took it, not that anyone saw it".
- **Fanout progress meter** (`components::meter`, A-components:262-284). Refresh is manual until the framework has live support.
- **Reads over time**, series 1.
- **A/B variants side by side.** No significance claims unless the A/B work provides them.
- **The proof of what was sent**, and Workings filled with real values.

### Audiences

- Topics (subscribers, open-to-self chip), attributes (people holding each, top values), and the registry explained.
- Any row offers **Write a notification to this**.

### Learn

- A Notifications track whose lessons are checked against real state (e.g. "You sent a test to your own device ✓" from `campaign_events`).
- Glossary entries: topic, condition, registry, fanout, opt-out, provider acceptance, silent message.

### Safeguards

- **Everyone, or an audience over the threshold:**
  - needs the broadcast role, an exact `validate_only` count at review, and the number typed to confirm;
  - at launch the admin's server re-runs `validate_only` and refuses if the audience grew by more than 10% ("Review again").
- **Scheduled time in the past:** blocked by the composer **and** refused by `launch_campaign` when more than a few minutes past.
- **Far future:** a warning past 30 days; the service already refuses past 366 days (`MAX_SCHEDULE_SECS`, N-lib:153). An expiry too close to the scheduled time gets a warning.
- **Test sends:** rate-limited per admin on the admin's server. Fixed to the admin's own person unless they hold Send. Always recorded, since `send_test` records nothing (N-lib:1717-1722).
- **Repeats:** "You sent this to this audience 2 hours ago."
- **Concurrent edits:** two admins editing one draft get a revision conflict that names who changed it, like `RevisionConflict` (A-server:595).
- **Links:** an off-origin link host is shown prominently at review.
- **Cancel:** offered only while the service allows it; afterwards, "Can't unsend: 3,201 people already have it."

---

## 4. What leptoskit itself must gain first

Checked against `leptoskit_native`'s constructors (`components.rs`, `components/controls.rs`, `components/helpers.rs`) and `Element`'s builder methods (`element.rs`).

1. **A multi-line text field.** `TextInput` is "An editable single-line text field" (`crates/leptoskit_native/src/components.rs:318-327`). Needed for the body (2,000 bytes), data JSON, condition text and pasted IDs. All backends, plus the Swift and Kotlin renderers in `leptoskit_bridge_ffi`.
2. **Input attributes.** Only `value`, `placeholder`, `autocomplete`, `focused` and `enabled` exist (`element.rs:1505-1581`). Needed: input kind (URL, number, email, search), `spellcheck`, enter-key hint, and a character `max_length` (byte limits stay app-side counters).
3. **Label-to-field association.** `leptoskit_aria::Field` wires `aria-labelledby` (`crates/leptoskit_aria/src/field.rs:101-109`), but the label is a plain text node (:78-82). Pressing it doesn't focus the field, and switches, radios and checkboxes need labels you can press.
4. **Modal sheet.** `Dialog` isn't modal, and the renderer has no focus containment (`crates/leptoskit_aria/src/dialog.rs:17-20`; `lib.rs:60-63`). Today's confirm sheet is built by JavaScript (A-mjs:136). Needs focus containment, Escape, an inert background, and focus returned on close.
5. **Popover and combobox/listbox.** There is no select, picker or anchored overlay. The analytics menus are absolute positioning plus hand-written CSS toggled by the script (A-shell:708-709; A-mjs:497-505). Needed for categories, topics, attribute keys and values, and operators.
6. **Date-time input with a time zone.** No component exists. Time-zone data in Wasm (size) is a decision to make with it.
7. **Live subscriptions from the app's side.** `AppCaller` only has `call` (RPC-client:572-597). The analytics tail is a hand-rolled frame client (A-alive:1-12). Needed: `AppCaller::subscribe` and a way to relay it to the admin's Wasm client. The service also needs `sends` to be live.
8. **A fresh sign-in through the main site** for dangerous actions: OIDC `max_age`/`prompt=login` and an `auth_time` check in leptoskit_auth and leptoskit_idp.
9. **Optional:**
   - a navigation guard for unsaved changes (nothing in `leptoskit_router`; draft autosave lowers the need);
   - `AppCaller::acting_for`.

These already exist and need no new framework work:

- `leptoskit_aria` Tabs and Disclosure;
- `Status` live regions;
- `Image`;
- `use_debounce_fn` / `use_interval_fn`;
- the clipboard plugin's web backend (`crates/leptoskit_plugin_clipboard/src/lib.rs:591-605`);
- `hover:`, `truncate` and `tabular-nums` (commit `7eea31d05`). AD-doc §4 and §13 (lines 102-104, 403-410) still list these as gaps and are out of date.

---

## 5. Order of work

Each item is one reviewable PR with its own changeset (AGENTS.md:35). Migrations go in both `migrations/` and `migrations-postgres/`, with `registry.txt`, generated by `leptoskit db --service notifications migration create` (NS-doc:17).

### Docs

**D1. Design note**

- **Files:** AD-doc (a new "Notifications" section: IA, screens, stations, safeguards); NS-doc (§7 line 65, §9).
- **Depends on:** nothing.
- **Verify:** docs lint; owner review of the screens.

### Service (can start now)

**S1. Conditions render back to text; quoted values allow spaces and escapes**

- **Files:** N-cond; guide `41-notifications.md` (conditions and segments sections); NS-doc §3.
- **Depends on:** nothing.
- **Verify:**
  - every existing fixture and generated trees satisfy `parse(to_string(t)) == t`;
  - new tests for spaces and escapes;
  - `tests/topics.rs` and `tests/segments.rs` unchanged and passing (the old grammar is a subset of the new).

**S2. Send history: `sends` list and `send_report`**

- **Files:** N-lib, N-service, migrations, `tests/history.rs` plus `[[test]]` in Cargo.toml, guide "Track a large send", NS-doc.
- **Depends on:** nothing.
- **Verify:**
  - three-page ordering;
  - each send kind listed with its content;
  - report counts over read, dismissed and cancelled rows and push deliveries by state;
  - anonymous and person callers refused;
  - the migration applied over existing rows (the segments tests already have this pattern);
  - recipients-send retries still answer correctly.

**S3. Audience catalogue**

- **Operations:** overview, topics, attributes, categories, `estimate_audience`, `person_endpoints`.
- **Files:** N-lib, N-push, N-service, migration (indexes), `tests/catalogue.rs`.
- **Depends on:** S1 (soft: the `targetable` flag).
- **Verify:**
  - exact counts below the cap, `exact: false` above it on a 12k registry;
  - invalidated endpoints excluded from reach;
  - no credential in any answer.

**S4. `preview_push`**

- **Files:** N-apns, N-fcm, N-webpush, N-push, `tests/push.rs`.
- **Depends on:** nothing.
- **Verify:** preview bytes equal the bytes posted through the injected boundary for every options case; trims in the documented order.

**S5. Campaigns**

- **Files:** new `src/campaign.rs`, N-lib, N-service (`send_in` refactor), migration, `tests/campaigns.rs`, guide, NS-doc status table.
- **Depends on:** S2.
- **Verify:**
  - a double launch is a duplicate;
  - revision conflicts are refused;
  - a past schedule is refused;
  - cancel before due works; cancel after visible is refused;
  - a duplicate resets the schedule;
  - a test writes no inbox row but records an event;
  - non-app callers are refused;
  - Postgres races in `tests/postgres.rs`.

### Framework (in parallel)

Each item gets its own PR, with tests, gallery or probe screens and platform-gap docs:

- **F1** multi-line text field;
- **F2** input attributes;
- **F3** labels;
- **F4a** modal sheet;
- **F4b** popover and combobox;
- **F5** date-time input;
- **F6** live from the app's side;
- **F7** fresh sign-in.

### Admin

**A0. The admin's Wasm client foundation** (the other line of work)

- Hydration entry, bundle, menus, drawer and confirm sheet in Wasm, and a client/server split of `leptoskit_admin` so typed operations compile for Wasm.
- A2–A6 depend on it.

**A1. Connect to the service**

- **Files:** A-lib, new `src/notifications/{mod,client}.rs`, A-server (capability helper), A-shell (rail and tab bar), CLI-admin, `examples/admin_demo` (run the service in-process like analytics, seed people, topics and attributes, add a fake push boundary and an endpoint for Ada), `tests/notifications.rs` plus a fixture like `analytics_service` (`tests/support/mod.rs:602-650`), guide `docs/src/guide/32-admin.md`.
- **Depends on:** nothing on the service side.
- **Verify:**
  - snapshots for "not configured" and "unreachable";
  - the token never appears in a page;
  - non-admins refused.

**A2. List, results and audiences (read-only)**

- **Depends on:** A0, A1, S2, S3.
- **Verify:** insta snapshots; demo walk-through.

**A3. Composer: message, proofs, draft autosave**

- **Depends on:** A0, A1, S4, S5, F1–F3.
- **Verify:**
  - browser tests through `leptoskit_testing`'s web control: typing updates the proofs, the byte counter, and the revision-conflict message;
  - 400px layout.

**A4. Audience builder**

- **Depends on:** A3, S1, S3, F4b.
- **Verify:**
  - unit tests from builder tree to text to parse;
  - the refusal shapes with their explanations;
  - the debounced estimate;
  - the Address station's mint wash.

**A5. Delivery, test, review, launch, cancel**

- **Depends on:** A4, F4a, F5 (F7 for broadcast's fresh sign-in).
- **Verify:**
  - capability gating at the admin-operation level;
  - typed confirmation;
  - refusal when the audience grew by more than 10%;
  - past schedule blocked;
  - an idempotent double press;
  - a test on Ada's demo device.

**A6. Notifications Workings and the Rust code generator**

- **Depends on:** A3.
- **Verify:** snapshot the generated Rust, and compare its JSON against the same values built by hand.

**A7. Learn track and glossary**

- **Depends on:** A5.

### What to look at in a running app

`cargo run -p admin_demo -- --seed`, then sign in as Ada. Check:

- list tabs and row menus;
- typing changes the proofs and washes the Workings;
- the condition text updates live;
- the estimate updates;
- a test send shows outcomes;
- Everyone is blocked until the broadcast role and typed confirmation;
- Grace is refused;
- 400px and dark mode;
- a keyboard-only path through the confirm sheet;
- screen-reader names on every field.

---

## Could not determine by reading

- **The admin's Wasm conversion.** I couldn't see it: no local branch or commit held it. So I don't know its client/server split, whether it will host typed operations or JSON endpoints, or how routing and hydration will work.
- **The future features** (device targeting, translated variants, local-time delivery, open tracking, A/B). None are in this checkout, so the plan names only where they plug in.
- **Second factor on admin sessions.** It's unclear whether an admin's OIDC sign-in sets the session's second-factor state that `require_step_up` reads (`crates/leptoskit_auth/src/mfa.rs:163-169`).
- **Mounting `rpc::server::Server` inside the admin.** I didn't verify it can sit under `/_leptoskit/admin/rpc` beside the auth layer on the admin's SQLite database, which needs the RPC pack tables.
- **Lock-screen truncation lengths per platform.** Not in the repository, so proofs must say they're approximate.
- **Aggregate query cost at scale.** NS-doc §6 says nothing has been measured.
- **Whether the guide's `SendSpec` snippet is compiled.** It omits `options` (guide `41-notifications.md:46-55`).
- **Out of scope here, but relevant to web open tracking:** `crates/leptoskit_notifications/src/browser/receiver.js` is 436 lines of hand-written JavaScript and "calls no server".

### Critical Files for Implementation

- crates/leptoskit_notifications/src/lib.rs
- crates/leptoskit_notifications/src/service.rs
- crates/leptoskit_notifications/src/condition.rs
- crates/leptoskit_admin/src/server.rs
- crates/leptoskit_admin/src/lib.rs
