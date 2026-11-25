# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-25)

### Features

- **Android FCM device-token enrollment, in the same `native` module as iOS.** The platform is a transport choice and nothing more: a snapshot whose latest answer is an FCM registration token registers as the typed `PushCredential::Fcm` credential, an APNs hex token registers as `Apns`, and the enroll/reconcile/disable workflow around them — gesture enrollment, nonprompting launch reconciliation with rotation and owner-verified retirement, stable disable keys — is one shared implementation, not a fork. FCM tokens validate as 1–4096 bytes of printable ASCII, never hex, and both token kinds keep redacted `Debug` output. The plugin's snapshot type is renamed `RemoteRegistration` (from `ApnsRegistration`) now that it carries either platform's answer, with a new `RemoteState::FcmToken` variant on wire state 4. _Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)

#### Development and store builds share one service

_Owner:_ Ifiok Jr. · _Introduced in:_ [8980de9](https://github.com/leptoskit/leptoskit/commit/8980de9af6db3ed62cd57c2e36e8c461d5a0cea5)

Apple runs two push servers, and a device token is known to one of them. Until now the service used one for every device, so a build run from Xcode and a TestFlight build could not enroll with the same service. Now each iOS and macOS device says which server its build is signed for, and the service pushes it through that one.

- The iOS and macOS hosts answer a new plugin method, `APS_ENVIRONMENT` (method 11): the simulator answers development, a build with an embedded provisioning profile answers what the profile's `aps-environment` entitlement says, and an iOS build without a profile, which came through TestFlight or the App Store, answers production. Android does not answer it.
- `native::enroll` and the launch `reconcile` send the answer with the device's other details, as `DeviceDetails::apns_environment`. Your app writes no code for it, and a change reaches the service at the next launch.
- The service stores it per endpoint and picks Apple's server from it. `LEPTOSKIT_NOTIFICATIONS_APNS_ENVIRONMENT` is now the default, for a device that reported nothing: an older app version, a Mac build without a profile, or an imported token.
- `import_push_endpoints` and a direct `register_push_endpoint` take the same field, so imported APNs tokens can name their environment.
- `person_endpoints` and `push_endpoints` show it under `device`. Conditions do not read it, and it is dropped for an FCM or Web Push endpoint.

The `.p8` key, the team id, and the topic are still one of each, so both builds need the same bundle id and a key that may use both servers.

This release has one migration per engine, `endpoint_apns_environment`, which adds a nullable column to `push_endpoints`. Run `leptoskit db --service notifications migrate` before you start the new version.

Breaking for code that builds these by hand: `DeviceDetails` has a new public field, so a struct literal needs `..DeviceDetails::default()`. `ApnsEnvironment` now lives in `push` and is re-exported from `service::config`, where it was. A host that predates method 11 answers "unavailable" for it, and the app enrolls as before.

The Swift code that reads the provisioning profile was not compiled or run on a device for this release.

- **Browser push enrollment, corrected to the browser's real lifecycle: the client half of Web Push.** A new `browser` feature ships the receiver service-worker script (built with your inbox route, served same-origin at the root), a typed availability check, and `enroll`, `reconcile`, and `disable` flows that ride the person's own RPC identity over the feature's own same-origin fetch transport — no service token, nothing persisted in the worker. `reconcile` runs on launch and registers the subscription the browser already holds without ever prompting, answering `None` when there is none; `enroll` runs from the gesture that can subscribe. Enrollment waits for the selected worker's own activation (a failed install errors instead of hanging), matches a requested scope exactly rather than letting a broader worker stand in, and reuses an existing subscription only when it was created for your own VAPID key — another sender's is refused, not replaced. The enrollment handle records the subscription and scope it owns, so `disable` revokes first and then unsubscribes exactly that subscription: a stale handle leaves a replaced subscription and another scope's enrollment alone, and a retried disable whose revoke already landed completes. The receiver shows the service's envelope under a tag built from the notification's id, falls back to one truthful generic banner for missing, oversized, malformed, or mistyped pushes, and routes clicks only to your own inbox route on your own origin. _Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)

#### Save a campaign as a draft and launch it once

_Owner:_ Ifiok Jr. · _Introduced in:_ [99ecf87](https://github.com/leptoskit/leptoskit/commit/99ecf87c362ec9f1cd0d12e6591e9cdbfa1ab06f)

A person composing a notification on a screen writes it in steps, tries it on their own phone, lets someone else fix it, and picks a time before sending. `send` had nowhere to keep that work, and nothing stopped a double press from being two sends. The new `campaign` module keeps a draft in the service and turns it into exactly one send. All of it is for your app's server only.

- `create_campaign` saves a `CampaignDraft` (a name, a `SendSpec`, an optional `SendTarget`, and a note). A draft may be unfinished: only its size is checked.
- `update_campaign` saves a change made from a given `revision` and refuses it with a conflict when the campaign was saved in between.
- `launch_campaign` makes the send, with everything `send` checks. Launching the same revision again answers the first send as a duplicate. A draft whose `visible_at` has passed is refused, and a draft for everyone is refused unless `broadcast` is `true`.
- `cancel_campaign` cancels the send by the rules of `cancel`. `archive_campaign` puts a campaign away.
- `duplicate_campaign` and `duplicate_send` save a copy as a new draft, without the original's schedule. A recipients send's copy has no audience.
- `test_campaign` pushes the draft to one person's devices now, writes no inbox row, and records what the push came to.
- `campaign`, `campaigns`, and `campaign_events` read a campaign, list campaigns, and list who did what. Every write takes `by`, the acting person's name as your server knows it.

**What changes for an existing app:**

- The `campaigns` migration adds two new, empty tables, `campaigns` and `campaign_events`, on SQLite and Postgres. Nothing existing is touched.
- Campaigns are not deleted by retention, and `forget_person` does not edit drafts. A draft that lists recipients keeps those ids until you change it.
- `send`, `cancel`, and every existing operation are unchanged.

#### Choose a collapse key and a push lifetime

_Owner:_ Ifiok Jr. · _Introduced in:_ [83a2053](https://github.com/leptoskit/leptoskit/commit/83a205359a092ad96b4ee03e084a49462fb5f848)

A score that changes every few minutes should replace the last push, not add a banner, and a push worth ten minutes should not stay queued at a provider for days. Until now every push collapsed by its notification's id, and the only lifetime was `expires_at`, which also removes the item from the inbox. Three new options fix this, and a send that sets none of them is byte for byte what it was.

- `MessageOptions::collapse_key` is a key of your own, 1 to 32 letters, digits, `-`, and `_` (`MAX_COLLAPSE_KEY_LEN`). It is sent as `apns-collapse-id`, as FCM's `collapse_key` and the Android notification's tag (unless the send sets `android.tag`), and as the Web Push `Topic` header and the banner's tag in the browser.
- `MessageOptions::push_ttl` is how many seconds a provider may hold the push, counted from when the notification becomes visible, at most 2,419,200 (`MAX_PUSH_TTL_SECS`, 28 days). The inbox item is not affected. With `expires_at` too, the push stops at the earlier one. `0` means one attempt that no provider stores and that is never retried.
- `AndroidOptions::raw_channel_id` names an Android channel exactly as the device holds it, for a channel an earlier build of your app created itself, for example under Firebase. `channel_id` still names a channel made with the plugin. A send sets one or the other.

**What changes for an existing app:**

- `AndroidOptions` and `MessageOptions` have new public fields. Code that builds either one by naming every field no longer compiles: end the literal with `..AndroidOptions::default()` or `..MessageOptions::default()`.
- A delivery that waited past the end of its `push_ttl` ends with the reason `expired`, which until now meant only that `expires_at` had passed.
- The receiver service worker reads a new `lktag` key. A browser still running the previous worker ignores it and shows the banner under the notification's id, as before.
- No migration. Both options are stored in the options JSON each notification row already has.

#### Device details on push endpoints, and targeting by device

_Owner:_ Ifiok Jr. · _Introduced in:_ [d0ea555](https://github.com/leptoskit/leptoskit/commit/d0ea555a8701f661e4bc72a6d697ad6dc59f062a)

Every push enrollment now carries what the device says about itself — its preferred locale, its time zone, and the app's version — and a send's condition can target devices with them.

- **Device details.** `register_push_endpoint` takes an optional `device` (`DeviceDetails { locale, time_zone, app_version }`), defaulting to none, so existing callers keep working. The service stores the locale as a canonical BCP 47 tag (`en_gb` becomes `en-GB`, at most 35 bytes), the time zone as an IANA-shaped name (at most 64 bytes, checked for shape only), and the app version as free text (at most 64 bytes), refusing a bad detail by name. Re-enrolling replaces them; a change to the details alone never rotates the endpoint's credential or disturbs its queued deliveries. Only the endpoint's owner reads them back, as `EndpointSummary::device`. A new `device_details` migration (SQLite and Postgres) adds the three nullable columns.
- **No app code.** The web client sends `navigator.language` and the `Intl` time zone, and the app's version when `EnrollOptions::app_version` names one. The `native` module reads the locale, time zone, and app version through the notifications plugin's new `DEVICE` method on iOS, macOS (Foundation's preferred language, `TimeZone.current`, `CFBundleShortVersionString`), and Android (the default locale and time zone, the package's version name). `native::reconcile` now sends only when the token or the details changed since the enrollment last sent them — `Enrollment` remembers a digest of what it sent in its new `sent` field — so an app update or a language change reaches the service at the next launch and an unchanged launch makes no service call.
- **Device predicates.** Conditions gain `device.platform` (`'apns'`, `'fcm'`, or `'web_push'`), `device.locale`, `device.language` (the locale's first subtag, so `'fr'` matches `fr-CA`), `device.app_version`, and `device.time_zone`, with the operators `user.` predicates have. `device.app_version` compares as a version: everything from the first `-` or `+` is ignored, the rest must be dot-separated numbers compared left to right with missing parts as 0 (`2.10 > 2.9`, `2.1 == 2.1.0`); an ordering is false for a value that isn't a version. Any other key, or a platform that isn't one, is refused at send time and under `validate_only`, naming what there is.
- **Judged per device.** A person is in the audience when the condition holds with no device — where a device predicate answers false, as an absent attribute does — or on at least one of their live endpoints. They get one inbox row, and the push goes only to the endpoints the condition holds for, decided when the fanout writes the batch's deliveries. "Everyone on Android" is `device.platform == 'fcm'`, and it walks the same registry `SendTarget::Everyone` does.

The plugin's `DEVICE` method (9) answers three length-prefixed strings, empty for unknown, as the new `DeviceInfo`; `guest::Notifications::device_info` reads it, and a host that predates it answers `Unavailable`. The Android host's remote-registration snapshot is now encoded little-endian, as the bridge codec reads it.

#### Import the device tokens you already hold

_Owner:_ Ifiok Jr. · _Introduced in:_ [9ad0492](https://github.com/leptoskit/leptoskit/commit/9ad0492713adfc894060c022ffe391b57391f392)

An app that moves from Firebase Cloud Messaging has a table of device tokens, and until now none of them could be used: every device had to update and enroll before it could be pushed to. `push::import_push_endpoints` takes those tokens from your app's server, so people are pushed to from the next send on.

- The call takes 1 to 500 entries (`MAX_IMPORT_ENDPOINTS`), each an `ImportEndpoint`: a person, an FCM registration token or an APNs device token, and optional `DeviceDetails`. A Web Push subscription cannot be imported.
- It answers `ImportedEndpoints`: how many were `imported`, how many were `unchanged` because their person already holds the token, and the `skipped` entries by index, each `held_by_another` or `person_full`. No credential is answered.
- One malformed entry refuses the whole call and names its index. Nothing is written.
- An import never moves a token between people, and never revives an endpoint a provider invalidated.
- `EndpointSummary` has a new `imported` field. It is `true` until the device's own app enrolls the same token for the same person. The endpoint then keeps its id and its queued pushes and takes the details the device reports.

Only your app's server may call it.

**What changes for an existing app:**

- The `endpoint_import` migration adds one nullable column, `push_endpoints.imported_at`, on SQLite and Postgres. Existing rows are not changed.
- `EndpointSummary` has a new public field. Code that builds one by naming every field no longer compiles. The field is left out of the JSON when it is `false`, so existing clients read the same answers as before.
- The service keeps no record of a revoke or an erasure. Importing a token again after its person turned notifications off, or after `forget_person`, enrolls it again.

- **Make the fanout and push engine finish what it starts.** Every batch of a topic, condition, or everyone send is pushed: the push rounds keep going while the walk is still adding deliveries, and a round never takes the key of one that already ran. A round does a bounded share — at most a thousand attempts, started within a minute — and hands the rest on at once, one round of a send at a time; one delivery's failure (an unreadable stored options value, a database error, a row gone mid-attempt) is that delivery's outcome, never the round's. A fanout step claims its batch before writing, keys its successor by the cursor it starts from, and a step that loses its batch to a replay keeps the walk going, so a send can no longer stay `fanning_out` forever. Each step wakes only its own batch's people. Retries wait one minute, doubling, with jitter that differs per delivery, on every platform, so an hour's provider outage loses nothing. Values that would overflow what the service stores are refused at send time on every target and in a dry run (`MAX_SPEC_LEN`, `MAX_TARGET_LEN`, `options::MAX_OPTIONS_LEN`), a full 255-byte idempotency key works everywhere, and a past `visible_at` is stored as now, so its push gets its full window. `cancel` is set-based and cancels a large fanout's rows in batches through the new `cancel_rows` job. Breaking: `send` and `send_test` are now `#[rpc::action]`s, so a dry run walks its audience and a test send waits on providers with no transaction open; the RPC layer keeps no request-ID receipt for them, so a retried `send` answers from its idempotency key and a retried `send_test` pushes again. _Owner:_ Ifiok Jr. · _Introduced in:_ [d1a5857](https://github.com/leptoskit/leptoskit/commit/d1a585735fc14179bce372d21586daa64ab84d56)
- **iOS push enrollment, second patch: the `native` module now covers the whole launch lifecycle, not just the gesture.** `reconcile(previous)` is the launch half and runs only for a device that previously opted in — the saved enrollment is the record — registering with APNs again without prompting (Apple expects it each launch) and never silently re-enabling a device that opted out. Its answer is an explicit transition: `Kept` (same token, same endpoint id, one idempotent call), or `Rotated { current, retired }` where the previous handle is verified against the signed-in person's endpoint list and retired under its owner with the stable `disable` key — rotated tokens never leak orphan endpoints, the quota refusal at ten endpoints is answered by retiring the known-stale handle and re-registering once, and a previous account's leftover handle is reported as `Stale` and dropped without ever being operated under the new identity. Errors name their retry state: `Unconfirmed` leaves the previous enrollment untouched, `Enroll` carries the handle to keep (or none once retired), and `RetireFailed`/`OwnershipUnknown` leave the new enrollment live with the old handle named for retry. The plugin contract is now an optional dependency behind `native` (guest side alone), so the shared types and the service build without it and without its Linux backend; silence answers `Unconfirmed` instead of replaying a stale token, and tokens never print through `Debug`. The whole guest control flow — plugin guest transport and RPC client over the HTTP plugin guest included — runs in a compiled wasm guest against played OS and service boundaries (`crates/leptoskit_wasm/tests/notifications_guest.rs`, wired into `test:plugins:guest`). _Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)
- **The `native` module's enrollment covers macOS: a generated macOS app with `[native.macos] aps-environment` set enrolls its APNs device token through the same `enroll`, `reconcile`, and `disable` calls as iOS and Android — the platform stays a transport choice, the token kind the host answers names the provider kind, and nothing forks beyond it.** Docs and error text name all three platforms now. _Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)

#### Message options, silent data messages, and test sends

_Owner:_ Ifiok Jr. · _Introduced in:_ [93e2a2a](https://github.com/leptoskit/leptoskit/commit/93e2a2af050fea905b51e69b98219691552ca047)

A send can now be shaped per platform. `SendSpec` takes an `options` object — every field optional, and a send without one byte-for-byte what it always was:

- **`kind`**: a visible notification (the default) or a **data message** — silent everywhere: APNs `content-available` under `apns-push-type: background` at priority 5, an FCM data-only message, and a Web Push envelope the receiver worker deliberately does not show. Data sends create no inbox row and never count toward a badge, but their delivery, status, idempotency, cancellation, and fanout behave exactly like a visible send's.
- **`priority`**: `apns-priority` 10/5, `android.priority` HIGH/NORMAL, Web Push `Urgency`.
- **`image`**, **`sound`**, **`link`**: carried where each platform reads it (the web receiver opens a click link on its own origin only; Apple platforms need a Notification Service Extension to display an image).
- **`badge`**: an exact number, or `unread` — resolved **per recipient at push time** to their live unread count, so one send badges each person's own number. The web receiver sets it through `navigator.setAppBadge`.
- **`android`**, **`apple`**, **`web`** overrides: channel, color, icon, tag, sticky on Android; subtitle, thread-id, category, interruption-level, relevance-score on Apple; icon, badge shape, up to two actions, require-interaction, direction, and language on the web.

Everything is validated on both sides with errors naming the field, options count in a send's idempotent intent (the same key with different options is a conflict, and pre-option sends keep answering their own retries), and when a payload is over its provider's limit the extras drop in a documented order — per-platform blocks, image, sound, badge, and link, then the send's `data` — before the id-only envelope.

**`send_test`** (app-only) pushes a spec to one person's live endpoints — FCM's "test on device" — once each, answering per-endpoint outcomes and writing nothing: no inbox row, no delivery row, no count anywhere. It is bounded by the person's enrollment (at most ten live endpoints) and can probe a single endpoint by id.

The receiver service worker honors what arrives: a data message shows nothing but still sets the badge it asked for, options shape the banner, and a click opens the send's link only when it resolves to the app's own origin.

#### Count opens, and report what a send came to

_Owner:_ Ifiok Jr. · _Introduced in:_ [34082d8](https://github.com/leptoskit/leptoskit/commit/34082d8fe982f233b46a515e4fa2803d9219db46)

A tap on a push reached your app and stopped there: the service never learned of it, the person's other devices still showed the notification as unread, and nothing could say how many people opened a send. Opens are now recorded, and one call reports what became of a send.

- `mark_opened` is a new operation for the signed-in person. It records the first open of one of their notifications and marks it read in the same write. Opening it again changes nothing and succeeds. It is refused for another person's notification exactly as `mark_read` is, and a silent data message has no open.
- `Opened::mark_opened()` in the `receive` module makes that call from the place you handle a tap, on iOS, macOS, Android, and the web. It answers `Ok(false)` without calling anything for a tap that names no notification, such as a local one. On the web the page makes the call; the receiver worker still calls no server.
- `send_report` is a new app-only query. It answers a `SendSummary`: the send's `progress` (what `send_status` answers), its `label`, when it was made, and `inbox`, an `InboxCounts` of `rows`, `read`, `opened`, `dismissed`, and `cancelled`. The counts are one read of the send's own rows through an existing index. They count the rows still stored, so they shrink once retention has deleted a send's rows, while `progress` does not change.
- `SendSpec::label` is an optional name for a send, 1 to 64 ASCII letters, digits, `-`, `_`, or `.`, checked by the new `check_label`. It is stored with the send, answered in its report, never sent to a device, and part of the idempotent intent. A send without one serializes exactly as before.

**What changes for an existing app:**

- The `open_tracking` migration adds the nullable `notifications.opened_at` and `sends.label` on SQLite and Postgres, with no backfill, and replaces the live triggers on `notifications`. Rows written earlier count as unopened.
- `Notification` and `AppSend` each gain a field, which breaks struct literals of them, and `SendSpec` gains `label`, which breaks its struct literals that do not end in `..SendSpec::default()`.
- Nothing is recorded until your app calls `mark_opened`. `send_status` and `SendProgress` are unchanged.

#### Devices change hands and forgotten people are erased

_Owner:_ Ifiok Jr. · _Introduced in:_ [75e6f21](https://github.com/leptoskit/leptoskit/commit/75e6f21d8c5dff5e4dba79671e5dd8deac3eba25)

Three things the notifications service got wrong on a shared device and for a deleted account, and four limits it did not have.

- **A shared device changes hands.** When someone enrolls a push destination that another person's enrollment still holds, and presents the byte-identical credential (the same APNs or FCM token; for Web Push the same endpoint and both keys), the service revokes the old enrollment and creates a fresh one for the new person. It used to answer `409`, which left the previous person's notifications arriving on a device the new person could not enroll. A different credential for the same destination is still refused.
- **`forget_person` erases the person.** It now deletes their attributes, topic subscriptions, category preferences, push endpoints (queued deliveries end `skipped`/`endpoint_revoked`), inbox rows, and the registry and wake rows. It used to keep the inbox, preferences, and endpoints. A long inbox is deleted in batches by the new `forget_rows` job; a notification sent after the call is kept. It stays app-only and safe to repeat. No migration.
- **A server-side revoke sticks.** `register_push_endpoint` takes an optional `renews` (the endpoint id a client believes it holds). A renewal only succeeds while that endpoint is still the caller's; otherwise it writes nothing and answers not-found with a message starting `push::ENROLLMENT_GONE` (`push::enrollment_gone` recognizes it). Both `reconcile` functions send it, so a device whose enrollment was revoked — from another device's list, by the app's server, or by `forget_person` — is told so and stays off until the person turns notifications on again.
  - **Breaking:** `browser::reconcile` now takes the saved enrollment, `reconcile(&options, previous.as_ref())`. With none it registers nothing; when the server no longer has it, it answers `None`.
  - **Breaking:** `native::Outcome` gains `Gone(Enrollment)`. Drop the saved enrollment when you see it. A launch with nothing changed now makes one small query (`push_endpoint_standing`) instead of no call.
- **New app-only operation `revoke_person_endpoints(person)`** ends every device of one person, for a server-side sign-out. New person-side query `push_endpoint_standing(endpoint)` answers whether the caller still holds an enrollment.
- **Bounds.**
  - A person keeps at most `MAX_ENDPOINT_HISTORY` (10) invalidated endpoints; a new enrollment deletes the oldest beyond that. The list used to grow without limit.
  - A page pinned with `until` reads at most 500 inbox rows (`MAX_PINNED_INBOX_ROWS`), 20 endpoints, or 500 topics, in page order. It used to read up to 10,000 rows whatever `limit` said.
  - `set_person_attributes` refuses a call that names more than `MAX_ATTRIBUTE_CHANGES` (100) keys for one person across `set` and `remove`.
  - A condition nested more than `MAX_CONDITION_NESTING` (32) parentheses and negations deep is refused before the parser descends, instead of recursing as deep as the text is long.
- **Condition values can hold spaces and quotes.** A value is everything between its quotes, and a quote inside one is written twice: `user.name == 'Mary O''Brien'`. `condition::quoted` writes a value from any string. A backslash is an ordinary character and control characters are refused. Topic names are unchanged, so a condition FCM accepts still parses the same way.

#### Push transports follow each provider's documented rules

_Owner:_ Ifiok Jr. · _Introduced in:_ [5b9dbae](https://github.com/leptoskit/leptoskit/commit/5b9dbae40f0f06fecea2c131711aa5241524fa96)

The APNs, FCM, and Web Push transports were checked against Apple's, Google's, and Microsoft's documentation and the Web Push RFCs, and corrected where they disagreed:

- **Safari and Edge can enroll.** Web Push now accepts any subdomain of `push.apple.com` (Safari's `web.push.apple.com`, as Apple asks senders to allow) and of `notify.windows.com` (the Windows Push Notification Services channels Edge on Windows hands out), matched on whole labels. The never-used `webpush.apple.com` is no longer accepted, and lookalikes, other ports, and credentials in the URL are refused as before.
- **FCM measures what FCM counts.** The 4,096-byte limit is checked on the keys and values a device receives, not the whole HTTP request, and the APNs block no longer repeats the title and body unless a subtitle needs them. The longest title and body a send allows now reach an Android device instead of ending `oversized`. The degradation order is the same on all three transports: the per-platform blocks and the image, then sound, badge, and link, then the send's `data`.
- **Silent means silent.** A data message carries no notification block on any provider: no `android.notification` at FCM, which would show it in the tray, and nothing but `content-available` in APNs's `aps`. A data send's badge rides as `lkbadge` data for the app to apply. `image`, `sound`, `link`, and the `android`, `apple`, and `web` blocks are now refused on a data send, naming the field.
- **Data messages are never pushed to a browser.** Browsers require every push to show a notification, so a data send's Web Push deliveries end `skipped` with the new reason `platform_unsupported`, and its native deliveries go as usual.
- **Only a gone device is invalidated.** APNs `410`, FCM `UNREGISTERED`, and a push service's `404`/`410` invalidate an endpoint. APNs `BadDeviceToken` and `DeviceTokenNotForTopic` (a wrong environment or topic), FCM `SENDER_ID_MISMATCH` and its third-party credential errors, and a refused VAPID signature end the delivery as `operator_configuration` and leave the device enrolled. A provider token APNs refuses is dropped from the cache and the push retried under a fresh one.
- **FCM's reserved data keys** now include `message_type` beside `from`, `google.*`, and `gcm.*`; `lkbadge` joins the envelope's own keys. They are dropped from the FCM copy only.
- **A Web Push subscription whose key is no point on the curve** ends `skipped` with the new reason `subscription_unusable` instead of `oversized`.
- **Configuration.** Setting `LEPTOSKIT_NOTIFICATIONS_WEB_PUSH_EXTRA_HOSTS` without a VAPID pair now refuses to start, naming the missing variables. The guide shows the `secrets` list the `[services.notifications]` table needs for the push variables to reach the service, names every variable in full, and shows how to set a `.p8` key and a service-account JSON and how to generate a VAPID pair.

#### Add durable server-side push delivery and authenticated endpoint enrollment to the notifications service.

A signed-in person's app registers the credential its platform issued — a typed APNs device token (topic and sandbox/production server-side), an FCM registration token, or a Web Push subscription — against the forwarded identity, bounded at ten live endpoints, with the credential stored only as secret material and a SHA-256 fingerprint for idempotence, and never in a list, a status row, an error, or a debug string. A credential another live account holds is refused rather than reassigned; only the owner revokes, and revocation deletes the endpoint and skips its queued deliveries as revoked.

_Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)

A send now fans one durable delivery per notification and live endpoint (`push_deliveries`, unique per pair), attempted by a `push_send` job when the notification becomes visible: schedule, cancellation, expiry, the latest opt-out, and endpoint liveness are re-checked at the moment of the attempt, with the attempt as the documented revocation boundary. Transient failures retry with the provider's `Retry-After` honored outright (or the engine's backoff) up to a bounded attempt count; permanent failures end clearly with sanitized reason codes, and an unregistered credential invalidates its endpoint and skips its siblings. Delivery is at-least-once with stable collapse ids — the notification's id is the `apns-collapse-id`, the FCM collapse key, and the Web Push topic — and provider acceptance is never claimed as a user receipt. The app reads per-endpoint status (`push_deliveries`, app-only) as enqueued, retrying, accepted, rejected, or skipped, and a public recipient can send nothing.

Real transports ship behind the service feature: APNs HTTP/2 with ES256 `.p8` provider tokens (cached), FCM HTTP v1 with service-account JWT-to-OAuth exchange and expiring cached tokens, and Web Push with RFC 8292 VAPID and RFC 8291 `aes128gcm` encryption — all on ring's maintained primitives, over an injected HTTP boundary that refuses redirects, ignores proxy variables, verifies TLS, and bounds response reads. Web Push endpoints are held to a fixed shipped-provider host policy (extras opt-in, DNS-validated and pinned), payload limits are checked on the encoded bytes with a minimal id-only envelope fallback, and half-configured providers refuse to start with every missing variable named while unconfigured transports stay explicitly unavailable. New sends already past their expiry are refused. Native token acquisition and generated browser push handlers remain future work.

#### Correct the push transports' provider interoperability and the endpoint/delivery lifecycle, ahead of any release.

_Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)

The FCM service account now must be the JSON Google actually issues — an RSA key — and its OAuth assertion is built and signed with the maintained `jsonwebtoken` crate as RS256, verified against Google's contract rather than the ES256 EC key path that never worked against the real endpoint. The exchange is single-flight with a cached token, a fixed first-party token endpoint (the JSON's `token_uri` is ignored), strict rejection of malformed or nonsensical token answers, a bounded one-refresh retry when the provider answers 401, and a clear split between operator-account trouble — which stalls the delivery as `retrying` without spending its attempts — and a device the provider no longer knows, which alone invalidates an endpoint (a generic 404 from FCM no longer kills a device). Web Push encryption and VAPID are now the maintained `web-push` crate's (Mozilla's `ece` underneath) instead of hand-rolled RFC 8291/8292 code, with the same injected HTTP boundary and the payload-sizing fallback kept, and the crate's output verified in tests by an independent ring-based receiver.

Endpoint identity is now the platform and destination — the token, or a Web Push subscription's endpoint URL — not the whole credential: a browser that rotates its keys keeps its endpoint, its owner, and its queued deliveries (the stored credential rotates in place, and padded or unpadded key encodings enroll as the same subscription), while another account can never take a live destination over by presenting new keys. An invalidated destination can be re-enrolled by its owner (reactivated) or taken over by the device's next account. `push_endpoints` is now paginated and includes invalidated endpoints with their state, and a person's invalidated history is bounded, so devices that churn can't grow the table or hide live endpoints behind the ten-endpoint bound.

Delivery writes are fenced on each row's own state and attempt count, so a revocation, a sibling invalidation, or a replayed job round that lands while an attempt is in flight can no longer resurrect or regress what already committed; the retry bound is eight actual network attempts (skips and operator stalls spend none); and the FCM payload now carries the send's expiry as the platforms' TTL (`android.ttl`, `apns-expiration`), keeps its collapse key through the id-only fallback, and drops the caller's reserved data keys (`lkn`, `from`, `google.*`, `gcm.*`). Web Push extra hosts resolve within a bounded time, DNS failure retries instead of ending the delivery, pinned connections re-resolve per attempt without a rebinding hole (the client cache keys on the exact address set and stays bounded), the private-address check covers the full IANA special-purpose registries, and a malformed extra host refuses to start by name. Secret-bearing types — the credentials, the wire requests and responses — print redacted `Debug`, and the push configuration is carried per service instance (routes and job arguments) instead of one process-wide interchangeable global. The real HTTP boundary is exercised against local servers: redirects surfaced never followed, endless bodies refused at the read bound, pinned connections pinned, and HTTP/2 spoken.

#### Taps, foreground pushes, and data messages reach app code

_Owner:_ Ifiok Jr. · _Introduced in:_ [b1d0c8b](https://github.com/leptoskit/leptoskit/commit/b1d0c8b741c4bd284362b53d998bd3c0b7b707cc)

A push used to end at the banner: nothing told the app which notification was tapped, a push that arrived while an iOS or macOS app was open showed nothing, an Android app in the foreground dropped it, and a silent data message ran no app code. One receiver now answers all of it on iOS, macOS, Android, and the web.

`leptoskit_notifications` gains the `receive` module, built with the `native` or `browser` feature. `Received` is `Opened` (a tap: the notification's id, the send's `link`, the pressed action's id, the send's `data`, a local notification's id, and `launched` for the tap that started the app), `Shown` (a notification arrived while the app was open), or `Data` (a silent data message with its `data` and `badge`). `Opened::route(inbox_route)` answers where the tap should go. `native::Receiver` and `browser::Receiver` both have `new`, `next`, and `pending`, and answer the launching tap first. Where nothing can be received they answer `ReceiveError::Unavailable`.

`leptoskit_plugin_notifications` gains method `RECEIVED` (10) on iOS, macOS, and Android, with `ReceivedItem`, `ReceivedKind`, `encode_received_wait`, `decode_received_wait`, and `guest::Notifications::received`. The host keeps the sixteen most recent items and hands each over once. On iOS and macOS a notification-centre delegate shows a notification while the app is open and records taps; on Android the plugin posts a push that Firebase hands to a foreground app, reads a tap from the activity's intent, and a local `show()` now opens the app when tapped.

`leptoskit_cli` generates the host side: new iOS and macOS apps install the delegate before launch finishes and forward `didReceiveRemoteNotification`, an iOS app with `aps-environment` set declares the `remote-notification` background mode, and the generated Android messaging service forwards `onMessageReceived`.

**What changes for an existing app:**

- `hosts/ios/App.swift` and `hosts/macos/App.swift` are written once and are the app's own, so an app generated earlier must add the delegate lines the guide lists. Without them it behaves as before. Android apps need no change.
- A push that arrives while an iOS, macOS, or Android app is open now shows a banner. An app that drew its own on those platforms would show two.
- The web receiver adds the notification's id as `lkn` to the address it opens for a send's own `link`, as it already did for the inbox route.
- The web receiver first offers a click to an open page and waits up to half a second for the page's answer before loading the address. Only a page that creates a `browser::Receiver` answers; other pages see a click arrive up to half a second later than before.
- A web push flagged as a data message, which the service never sends, now shows the generic banner when no page is in view instead of showing nothing.

The Swift and Kotlin code in this change was not compiled where it was written; its tests run in the Apple and Android CI jobs.

#### Send a notification on a repeating schedule

_Owner:_ Ifiok Jr. · _Introduced in:_ [997222d](https://github.com/leptoskit/leptoskit/commit/997222dc4cc3b5ca07a0f8c3664175355273052b)

A message that goes out every morning or every Monday used to be a send your own server made each time, from its own scheduler. The new `recurrence` module keeps the schedule in the service, which makes one send per occurrence. All of it is for your app's server only.

```rust
use leptoskit_notifications::recurrence::{
    CreateRecurrence, NewRecurrence, RecurrenceEnd, RecurrenceRule, Repeat,
};

let saved = call_as_app(CreateRecurrence {
    recurrence: NewRecurrence {
        name: "Morning reminder".into(),
        spec: SendSpec {
            category: "reminders".into(),
            title: "Time to check in".into(),
            ..SendSpec::default()
        },
        target: SendTarget::Topic("reminders".into()),
        rule: RecurrenceRule {
            repeat: Repeat::Daily { every: 1 },
            at: "08:00".into(),
            time_zone: "America/New_York".into(),
            starts_on: None,
            ends: Some(RecurrenceEnd::After(30)),
            local_time: false,
        },
        note: None,
    },
    by,
    broadcast: false,
})
.await?;
```

- A rule repeats daily, weekly on chosen weekdays, or monthly on a day of the month, at a wall-clock time in a named IANA time zone. It can end on a date or after a number of sends. `local_time: true` delivers each occurrence at that time in each person's own time zone.
- The time follows daylight saving changes. A time the clocks skip goes out when the gap ends, a time the clocks repeat goes out the first time only, and day 31 in a shorter month is the month's last day.
- Each occurrence is an ordinary send: the same checks, the same opt-outs. Its idempotency key is `recurrence:<id>:<the occurrence's instant in Unix seconds>`, so it is made once however many copies of the service find it due. Its label is `recurrence.<id>`, which is how you find a recurrence's sends in `list_sends`.
- After the service was down, it sends at most the latest missed occurrence, and only if that is less than 24 hours late. The rest are counted as `skipped`.
- `pause_recurrence`, `resume_recurrence`, and `end_recurrence` stop, restart, and finish one. `recurrence` reads one and `recurrences` lists them. A recurrence cannot be edited: end it and create another.
- An occurrence whose send is refused pauses the recurrence and stores the reason in `error`.
- `SendTarget::Everyone` is refused unless `broadcast` is `true`. At most 100 recurrences can be active or paused at one time, and a recurrence goes out at most once a day.

**What changes for an existing app:**

- The `recurring_sends` migration adds one new, empty table, `recurrences`, on SQLite and Postgres. Nothing existing is touched.
- The service runs one more job, `recur`, at each occurrence's time and once an hour.
- `forget_person` now also takes the person out of every recurrence that lists them as a recipient, and ends a recurrence left with nobody.
- An ended recurrence is deleted `LEPTOSKIT_NOTIFICATIONS_RETENTION_SENDS_DAYS` after it ended. Active and paused ones are never deleted.
- `send`, `cancel`, and every existing operation are unchanged.

#### Retention, and indexes for the reads that grew with the tables.

The service now deletes what is over, once an hour, in small batches that never hold a long write: inbox rows 365 days after they became visible, expired, cancelled, and silent rows 30 days after they ended, finished push deliveries after 30 days, and a send's record 90 days after the send, once no notification or delivery of it remains. Each is set in days, or `off`, with `LEPTOSKIT_NOTIFICATIONS_RETENTION_INBOX_DAYS`, `_ENDED_DAYS`, `_DELIVERIES_DAYS`, and `_SENDS_DAYS`; a bad value refuses to start. A row with a queued delivery, a send still fanning out, and a scheduled notification are never deleted. An idempotency key is remembered for as long as its send's record is kept — at least `SENDS_DAYS`, never less than 2 — and is a new send after that. `service::workers_with_settings` takes the retention for an embedded service, and `service::retention::prune` runs one sweep directly.

_Owner:_ Ifiok Jr. · _Introduced in:_ [2cedeb1](https://github.com/leptoskit/leptoskit/commit/2cedeb1a738ac9706f97633b51e4c15b171d5fc7)

A new `scale_indexes` migration on both engines adds `push_deliveries (endpoint, state)`, so a token rotation, a revoke, or an invalidation no longer reads every queued delivery; `push_deliveries (send_key, state, id)`, which a push round now reads one state at a time instead of walking the send's finished deliveries; and `notifications (person, kind, dismissed_at, read_at)` for the unread count. It drops `push_deliveries (state, next_attempt_at)`, which no query used. On Postgres the indexes are built under a lock that makes writes to the table wait; build them first with `CREATE INDEX CONCURRENTLY` under the same names to avoid that. A condition naming several topics now reads each topic's subscribers in index order and merges them, instead of sorting the union on every batch.

Two rollbacks are corrected: rolling `message_options` back cancels every silent data message instead of leaving it to appear in an inbox, and rolling the Postgres `init` back drops the `live` and `rpc` schemas when they are left empty.

- **Person attributes, segments, and send to everyone.** The service keeps a registry of the people it knows — registered when the app's server sets their attributes, when they subscribe to a topic, set a category preference, or enroll a push endpoint, and backfilled from those tables by the migration; a person named only in a recipients send is never registered. The app's server describes people with `set_person_attributes` (batch, ≤1,000 people, idempotent; keys `a-z0-9_` ≤64 bytes, values ≤256 bytes, ≤50 attributes per person with cap-skipping counted; `person_attributes` reads back; `forget_person` removes the person from the registry with their attributes and topic subscriptions, keeping their inbox) — app-only by design, so a segment can't be forged from the client side. The condition grammar extends FCM's with `user.<key> == 'v'`, `!=`, `in ['a','b']`, `has(user.<key>)`, and finite-number `<`, `<=`, `>`, `>=` (true only when both sides parse as finite numbers; `NaN`/`inf` literals refused), mixing freely with `'t' in topics` under `&&`/`||`/`!`/parens with byte-offset errors and bounds (≤5 topics, ≤10 attribute predicates, depth ≤8, ≤1 KiB); an absent attribute answers false to every predicate, and the everyone-in-disguise refusal now checks topics _and_ attributes. `SendTarget::Everyone` reaches every registered person, and a condition no longer needs to be anchored on a topic: one that structurally requires a topic still fans out over the topic index, while an attribute-anchored one (and everyone) fans out over the registry — same batched, cursor-advancing, idempotent fanout job, same opt-out/visibility/push/live behaviour, `validate_only` counting every target. SQLite and Postgres migrations add `persons` and `person_attributes` with the registry backfill. _Owner:_ Ifiok Jr. · _Introduced in:_ [644b0a3](https://github.com/leptoskit/leptoskit/commit/644b0a354e16352b37cfb3c78934d5c4e5b76a6e)

#### List sends, size an audience, and preview a push

_Owner:_ Ifiok Jr. · _Introduced in:_ [d45cced](https://github.com/leptoskit/leptoskit/commit/d45cced1f47bab9b8c7ae5fad9ef89c937875ce0)

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

- **Add `leptoskit_notifications`, the first-party notifications service: an app's server sends idempotently to explicit user ids, and each recipient gets one durable inbox row in the service's own database — scheduled visibility with cancel-before-due, expiry, category opt-outs enforced at send and at visibility, and per-person read and dismissed state shared across devices, read through bounded, live-paginated queries on every platform the app's screens run.** Time reaches open screens: every send enqueues a wake job when it becomes visible and at its expiry, so a scheduled item appears and an expired one leaves a watching screen without any other write; preference changes update open inboxes the same way. An accepted send answers its stored outcome forever, however the clock moves on, and a known zero-fanout send cancels idempotently. Delivery is authenticated inbox retrieval while the app runs; OS background push (APNs, FCM, Web Push) and device enrollment are not part of this change. The `leptoskit` facade re-exports the shared types behind a `notifications` feature, and the guide is under Services. _Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)
- **Audience partitioning by topics and conditions, with a durable batched fanout.** A send can now target `SendTarget::Topic` or `SendTarget::Condition` beside today's `SendTarget::Recipients`: the app's server subscribes people to per-person topics in idempotent batches (`subscribe_to_topic`/`unsubscribe_from_topic`, at most 1,000 people per call; a person holds at most 500 topics, and a full person is skipped and counted rather than refusing the batch), and a person may subscribe themselves (`subscribe`, `unsubscribe`, `my_topics`, live and paginated) to the topics the operator opened in `LEPTOSKIT_NOTIFICATIONS_OPEN_TOPICS` — patterns like `news.*`, nothing open by default, and the person always read from the forwarded identity. Conditions use FCM's grammar verbatim (`'a' in topics && ('b' in topics || !('c' in topics))`), validated on both sides with byte-offset errors, FCM's bounds (≤5 topics, depth ≤8, ≤1 KiB), and unanchored conditions — ones true for a person holding no topics — refused, since there is no everyone audience. A topic or condition send answers `fanning_out` immediately and a job walks the audience in 500-person batches — one transaction per batch for the inbox rows, push deliveries, and the stored cursor, unique per send and person so a crashed or replayed batch never duplicates, the audience snapshotted as the fanout reaches each person. `send_status` answers the state and counts; `validate_only` counts the audience without writing; `cancel` stops a fanout mid-walk and cancels the rows written so far. SQLite and Postgres migrations add the `topic_subscriptions` table, the fanout bookkeeping on `sends`, and the per-send-per-person uniqueness on `notifications` (existing rows backfilled). Breaking: `Send` and `check_send` take the target — `recipients: vec![…]` becomes `target: SendTarget::Recipients(vec![…])` — and `SendResult` gains `state` and `targeted`. _Owner:_ Ifiok Jr. · _Introduced in:_ [ed7b1e7](https://github.com/leptoskit/leptoskit/commit/ed7b1e77bb3ac93482b58819cdad9df80fdaa166)

#### Translated variants and delivery in each person's local time.

`SendSpec` gains `variants`, a map from a BCP 47 language tag to a `Variant` that replaces the title, the body, the Apple subtitle, and the web action titles for people who read that language, and `local_time`, a `LocalTime { at, default_time_zone }` that makes each person's row visible, and pushed, when a wall-clock time such as `2026-12-01T09:00` arrives in their time zone. Every person still gets exactly one inbox row, and its push carries the same text.

_Owner:_ Ifiok Jr. · _Introduced in:_ [8b56972](https://github.com/leptoskit/leptoskit/commit/8b569722a21f1d87a315715b4d129f75b7bf49bd)

Language and time zone resolve per person: the `locale` or `time_zone` attribute the app set wins, then the person's most recently seen live endpoint, then the send's default (its own text, and `default_time_zone`, which is UTC when left out). A locale matches the variant with its exact tag, then with its last subtag dropped, down to the language; nothing matching reads the send's own text. Time zones are IANA names looked up in bundled time zone data (`jiff`, behind the `service` feature only). A zone the service does not know on a person is skipped; an unknown `default_time_zone` is refused. A local time already past for a person arrives at once, a time a daylight-saving change skips arrives after the gap, and a repeated time arrives the first time.

A send carries at most 32 variants and still fits `MAX_SPEC_LEN`; a variant translates only text the send has, and a data message carries none. `local_time` and `visible_at` together are refused. `expires_at` must be after the local time has reached the last time zone (`at` plus twelve hours, as UTC), and the schedule limit is counted to that moment. Both fields are part of a send's idempotent intent and serialize to nothing when unset, so earlier sends answer their retries unchanged.

`cancel` now stops a local-time send for the people still waiting once others have seen it, and answers `0` when asked again; a send nobody is waiting for is still refused. Pushes for waiting rows are queued for their own instants, and a fanout enqueues one `wake_local` job per batch and distinct instant.

A new `endpoint_seen` migration on both engines adds the nullable `push_endpoints.seen_at`, set when an endpoint enrolls, changes its details, or renews (at most once an hour for an unchanged renewal). It adds an empty column and takes no measurable time.

**Source-breaking for struct literals:** `SendSpec` has two new fields and `PushEndpoint` one. `SendSpec` now implements `Default`, so `..SendSpec::default()` fills in what a send does not set.

### Fixes

- **Bound push retries by eight delivery requests and the earlier of expiry or 24 hours after visibility.** Honor seconds and HTTP-date Retry-After without shortening provider delays; report sanitized configuration, horizon, and overflow reasons. Count authentication refusals, bound OAuth recovery, and keep pending work moving after credential rotation. _Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)

#### Push-enabled hosts now work the way the notifications guide describes.

_Owner:_ Ifiok Jr. · _Introduced in:_ [7f60843](https://github.com/leptoskit/leptoskit/commit/7f60843eba43202f72117226374f71d7b4f6528d)

**Android.** The scaffold's Gradle script applies Google's services plugin by id (`apply(plugin = plugin)`); the earlier `apply(plugin)` asked Gradle for a script file of that name and failed every push-enabled build. An app scaffolded with the earlier line gets a generation error that names the one-word change. The generated messaging service is declared with the `com.google.firebase.MESSAGING_EVENT` intent filter, so Firebase delivers token refreshes to it instead of to its own default service. The manifest names a default notification channel, `leptoskit.notifications.default`, which the notifications plugin creates at startup, and the notification icon when `[app.icons] notification` generated one. Release builds get R8 keep rules for the Firebase names the plugin looks up by reflection, the plugin finds the task's listener methods on the public `Task` type, and Firebase without configuration answers `firebase-not-configured` even when its exception arrives wrapped by the reflective call.

**Android channels.** The FCM transport sends `android.channel_id` under the `leptoskit.notifications.` prefix the plugin creates channels with, so a send that names `inbox` reaches the channel the app configured as `inbox`. An id that already carries the prefix is sent unchanged.

**iOS and macOS.** The permission request asks for alerts, badges, and sounds, so a push's `aps.badge` and `aps.sound` take effect where the person allows them; it asked for alerts only.

**macOS.** A push-enabled Mac app (`[native.macos] aps-environment`) now needs `[native.macos] provisioning-profile`, a new setting naming the profile to embed: Apple's push entitlement is restricted, and macOS refuses to launch an app that claims it without a matching embedded profile. Generation stops without the profile. `leptoskit build --platform macos` embeds it at `Contents/embedded.provisionprofile` and signs with the identity in `LEPTOSKIT_MACOS_IDENTITY`, refusing to ad-hoc sign; `--release --unsigned` builds without the push entitlement. `leptoskit publish macos` embeds the profile before signing with the Developer ID identity. This follows Apple's signing rules and has not been run against a real Apple developer account. Apps without `aps-environment` generate and sign exactly as before.

**Web.** The receiver worker claims the pages already open when it activates, so a click can route the tab a person enrolled from, and a click on an action button adds the button's id to the opened address as `lka`.

The guide's samples no longer stop on iOS and macOS at `configure_channel` or enroll after a refused permission, and it now covers Web Push on iPhone and iPad needing a Home Screen install, combining the receiver with an app's own service worker, which `aps-environment` value an iOS build wants, and the entitlement a time-sensitive push needs.

- **Disable only the authenticated person's service enrollment, preserving the browser subscription and permission for reuse.** Require a successful revoke acknowledgement with a stable retry key; never infer success from an endpoint list. Remove the unused endpoint-matching unsubscribe helper. _Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)
- **Fence push endpoint ownership and credential rotation.** Keep endpoint ownership until explicit revocation, enforce the live endpoint cap on reactivation, and fence delivery outcomes by credential generation so late provider answers cannot invalidate rotated credentials or change their queue. Recognize FCM unregistration only from a typed FCM error detail. _Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)

#### Split the notifications guide and add Firebase migration

_Owner:_ Ifiok Jr. · _Introduced in:_ [ab02d3c](https://github.com/leptoskit/leptoskit/commit/ab02d3c1cf15353734b5a1fe34115384ab0d4455)

The notifications guide is now a chapter of twelve pages instead of one long page: an overview with a quick start, sending, the inbox, audiences, shaping messages, push setup, receiving on the device, delivery and troubleshooting, campaigns, privacy and data retention, coming from Firebase, and a reference.

- **Troubleshooting** lists every delivery reason code, every error the service answers with its HTTP status, the messages the service stops with at startup, and the enrollment and receiving errors in the app, each with what to do.
- **Coming from Firebase** maps Firebase Cloud Messaging's concepts and calls to this service, gives a migration in steps, and lists what is not there yet.
- **Reference** lists every operation with who may call it, every limit, every variable, and the settings in `leptoskit.toml`.
- The examples that no longer compiled are fixed: sends are built with `..SendSpec::default()`, a person's calls use `leptoskit_rpc::client::call`, and `RegisterPushEndpoint`, `PushDeliveries`, `Channel::new`, the service-worker route, and the receiver component match the current API.
- The signing and publishing pages now explain `[native.macos] provisioning-profile` and `LEPTOSKIT_MACOS_IDENTITY`, which a Mac app that receives push needs.

The crate's readme no longer says that nothing reports a tap to the service, and points at the chapter.

- **Push signing no longer pulls in the `rsa` crate.** The FCM service account's RS256 OAuth assertion is built and signed with ring's constant-time RSA directly — the same primitives APNs's ES256 provider token already used — so `jsonwebtoken` and its RustCrypto stack leave the dependency tree. Startup refusals, claims, caching, and the wire contract are unchanged. _Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)
- **Web Push now encrypts (RFC 8291 `aes128gcm`) and signs VAPID (RFC 8292 ES256) directly on ring instead of through the `web-push` crate, whose `jwt-simple`/`ece` dependency chain pulled the `rsa` and `crypto-bigint` advisories.** Each message draws a fresh ephemeral P-256 key and salt, the RFC 8291 Appendix A vector is asserted byte for byte in the tests, and the wire is unchanged: same headers, same envelope, same VAPID header shape. _Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)
- **Jobs can carry process-local typed runtime state through `Jobs::state` and read it with `JobCx::state`.** This state stays separate from the durable job owner's authorization context and is never serialized. Notifications routes and workers now share an explicit push runtime. Queued push arguments contain only send and round IDs, keeping database URLs and credentials out of job metadata and allowing pending work to recover with a fresh runtime after restart. _Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)
