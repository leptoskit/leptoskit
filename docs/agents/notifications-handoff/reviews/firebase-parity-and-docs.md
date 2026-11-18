# Review: Firebase parity and the public guide

Read-only review of `main` at `93e2a2af0` on 18 November 2026. The reviewer built nothing and ran nothing, so every item is a claim: confirm it with a failing test before fixing it. Line numbers are from that commit. [The handoff](../readme.md) says which findings are already being fixed.

I've finished the review without editing anything, building anything or changing git state. The main finding: most of the server side is already built, but four kinds of problem matter more than the Firebase gaps.

- **Device side:** apps never receive a push or a data message in their own code, on any platform.
- **Push correctness:** Safari can't enroll for Web Push, and two of the APNs refusals that invalidate endpoints are usually the operator's mistake, not a dead device.
- **Configuration:** the guide never says that the push credentials have to be listed under `secrets`, so following it leaves push silently off.
- **Code examples:** about a dozen guide examples won't compile against the current signatures.

None of the fetched pages tried to instruct me.

**Path shorthand.** `lib`, `opt`, `push`, `cond`, `svc`, `browser`, `native` are the `.rs` files in `crates/leptoskit_notifications/src/`. `svc/push` and `svc/config` are in `src/service/`; `fcm`, `apns`, `wp` and `host` are in `src/service/transport/`. `rx` is `src/browser/receiver.js`. `G:NNN` is a line of `docs/src/guide/41-notifications.md`.

## Problems found while checking "done" claims against the code

These come first because several "done" rows below depend on them.

1. **Silent data messages never reach app code, on any platform.**
   - On iOS, the generated app delegate only forwards the device token (`crates/leptoskit_cli/src/native/templates/App.swift:33-49`). Nothing handles an incoming push.
   - On Android, the generated messaging service only overrides the token-refresh callback (`crates/leptoskit_cli/src/native/project.rs:596-604`).
   - On the web, the receiver returns early for a data message and never passes it to an open page (`rx:304-309`).
   - The guide says the data "reaches your app… use it to sync, prefetch, or correct state" (G:407). Today it reaches nothing. The same gap means pushes that arrive while the app is open are lost.
2. **Safari can't enroll for Web Push.** The allowed hosts include `webpush.apple.com` (`host:29-33`). Safari subscriptions actually live on `web.push.apple.com`, and Apple says to accept any subdomain of `push.apple.com`. Edge's Windows push service (`*.notify.windows.com`, from my own knowledge) is missing too. The guide repeats the wrong host (G:799).
3. **Silent web pushes get Safari subscriptions revoked.** WebKit revokes a subscription after 3 pushes that show nothing, and Firefox spends a quota of 16. The guide only says Chrome "may stop delivering" (G:411).
4. **Two APNs refusals that are usually configuration mistakes invalidate endpoints.** `BadDeviceToken` and `DeviceTokenNotForTopic` mark the endpoint dead (`apns:472-479`). A wrong `APNS_ENVIRONMENT` or `APNS_TOPIC` would therefore invalidate every iOS device on the first send.
   - Each service also has exactly one APNs environment. Development builds and TestFlight/App Store builds can't share a service; Firebase handles this per token.
   - The guide says "only the provider's own 'unregistered' answer invalidates" (G:761). The engineering note says the same (`docs/agents/notifications-service.md:83`).
5. **Condition values can't contain spaces or quotes, but attribute values can.** The condition parser stops a value at whitespace or `'` (`cond:689-692`). `check_attribute_value` allows both (`lib:796-808`). So a stored value like "New York" can never be matched. The guide claims the opposite: "the same rules `set_person_attributes` enforces, so what you can send is what you can ask about" (G:217).
6. **Probable bug: a data send with a badge, sound or Android options may show a notification on Android.** FCM's Android block gets a `notification` section whenever any of those is set, even for a data send (`fcm:663-702`). That probably turns it into a visible notification. The guide promises "no banner on any platform" and calls a silent badge correction "classic" (G:407, G:424). I could only confirm this from FCM semantics, not on a device.
7. **The FCM reserved-key filter misses one key.** It drops `from`, `google.*` and `gcm.*` from your data (`fcm:591-599`). FCM also reserves `message_type`, which would make the whole delivery fail as `provider_rejected`.
8. **`send_test` keeps the mutation's database transaction open across up to 10 sequential provider calls** (`svc:1709-1756`). The push job explicitly avoids doing this.
9. **The rustdoc on `register_push_endpoint` contradicts the code.** It says an invalidated destination "is not held: registering it takes it over clean" (`push:694-696`). The code refuses with a conflict while another account holds it (`svc/push:153-207`). The guide (G:557) matches the code.
10. **Web action buttons don't report which action was pressed.** Every button behaves like a body click (`rx:366-416`).

## Part 1: Feature comparison

### A. FCM server API, message fields, targets, lifecycle

| #  | Firebase feature                                                                                                                                                                                                                                    | Status                                                                                                                                                                      | leptoskit operation or type                                          | Guide section                                                       |
| -- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------- | ------------------------------------------------------------------- |
| 1  | HTTP v1 send                                                                                                                                                                                                                                        | done (sends to people, not tokens)                                                                                                                                          | `send`, `lib:983`                                                    | Send from your server (G:32)                                        |
| 2  | `notification.title` / `body`                                                                                                                                                                                                                       | done                                                                                                                                                                        | `SendSpec`, `lib:395`                                                | G:32 (limits of 200 and 2,000 bytes undocumented)                   |
| 3  | `notification.image`                                                                                                                                                                                                                                | partial: no iOS Notification Service Extension is generated, so images don't show on iOS                                                                                    | `MessageOptions.image`, `opt:297`; `fcm:568`; `apns:371-374`         | Shape a message (G:351)                                             |
| 4  | `data` payload                                                                                                                                                                                                                                      | done (JSON object ≤4 KiB), but its shape differs by platform: flat strings on FCM `fcm:578-627`, nested under `lk` on APNs `apns:359-363` and web                           | `SendSpec.data`, `lib:400`; `check_data`, `lib:850`                  | G:66; key names undocumented                                        |
| 5  | Notification vs data-only messages                                                                                                                                                                                                                  | partial: wire is correct, no app-side receiver (problems 1 and 6)                                                                                                           | `MessageKind::Data`, `opt:73`                                        | Silent data messages (G:398), inaccurate                            |
| 6  | `android.collapse_key` / `apns-collapse-id` / Web Push `Topic`                                                                                                                                                                                      | partial: always the notification id; the app can't choose a key such as "score-update"                                                                                      | `fcm:633`; `apns:223`                                                | G:795                                                               |
| 7  | `android.priority`, `apns-priority`, Web Push `Urgency`                                                                                                                                                                                             | done (high/normal; no web low/very-low)                                                                                                                                     | `SendPriority`, `opt:109`; `fcm:649`; `apns:204-211`                 | G:382 table                                                         |
| 8  | `ttl`, `apns-expiration`, Web Push `TTL`                                                                                                                                                                                                            | partial: tied to `expires_at`, which also hides the inbox row; no separate push TTL; 24-hour retry horizon                                                                  | `fcm:530-534`; `apns:226-231`; `wp:72` (28-day default)              | G:795, G:765                                                        |
| 9  | `android.notification`: `channel_id`, `color`, `icon`, `tag`, `sticky`, `sound`, `notification_count`                                                                                                                                               | done                                                                                                                                                                        | `AndroidOptions`, `opt:219`; `fcm:663-702`                           | G:382-394                                                           |
| 10 | `click_action` / tap target                                                                                                                                                                                                                         | planned (tap routing); `link` is carried                                                                                                                                    | `MessageOptions.link`, `opt:309`                                     | G:389, G:807                                                        |
| 11 | Localization keys (`title_loc_key`, APNs `loc-key`)                                                                                                                                                                                                 | missing (planned translated variants are a different mechanism)                                                                                                             | none                                                                 | G:810 says "no localization"                                        |
| 12 | Other Android fields: `ticker`, `event_time`, `local_only`, `notification_priority`, `visibility`, vibrate/light settings, `default_*`, `proxy`, `direct_boot_ok`, `restricted_package_name`, `bandwidth_constrained_ok`, `restricted_satellite_ok` | missing                                                                                                                                                                     | none                                                                 | undocumented                                                        |
| 13 | `android_v2` (`remote_notification` / `background_sync`)                                                                                                                                                                                            | missing; the transport still sends the older Android config                                                                                                                 | none                                                                 | undocumented                                                        |
| 14 | APNs `aps`: alert, subtitle, badge, sound, `thread-id`, `category`, `interruption-level`, `relevance-score`, `mutable-content`, `content-available`                                                                                                 | done for these keys; missing critical-sound dictionary, `target-content-id`, `filter-criteria`, custom top-level keys                                                       | `AppleOptions`, `opt:239`; `apns:356-424`; `fcm:713-824`             | G:382-394                                                           |
| 15 | `apns-push-type` (`voip`, `liveactivity`, `complication`, …)                                                                                                                                                                                        | partial: alert and background only                                                                                                                                          | `apns:217`                                                           | undocumented                                                        |
| 16 | `apns.live_activity_token` (Live Activity updates over FCM)                                                                                                                                                                                         | missing, although the live-activities plugin already yields push tokens                                                                                                     | none                                                                 | undocumented                                                        |
| 17 | `webpush.notification` (Notification API options)                                                                                                                                                                                                   | partial: icon, badge, up to 2 actions, require-interaction, dir, lang, image, silent; missing renotify, custom tag, timestamp, vibrate, action identity                     | `WebOptions`, `opt:260`; `rx:311-356`                                | G:392, G:755                                                        |
| 18 | `webpush.fcm_options.link`                                                                                                                                                                                                                          | done on the web (same origin only, `rx:250-284`); native routing planned                                                                                                    | `MessageOptions.link`                                                | G:755                                                               |
| 19 | `fcm_options.analytics_label`                                                                                                                                                                                                                       | planned (per-send statistics; the send key could serve as the label)                                                                                                        | none                                                                 | undocumented                                                        |
| 20 | Target: one device (`token`, now `fid`)                                                                                                                                                                                                             | partial: only through `send_test`, which isn't durable                                                                                                                      | `send_test`, `lib:1746`                                              | Test on a device (G:769)                                            |
| 21 | Target: topic                                                                                                                                                                                                                                       | done (per person, not per device)                                                                                                                                           | `SendTarget::Topic`, `lib:439`                                       | Send to a topic (G:81)                                              |
| 22 | Target: condition                                                                                                                                                                                                                                   | done, extended with `user.*` predicates (problem 5)                                                                                                                         | `SendTarget::Condition`, `lib:444`; `cond:176`, `cond:388`           | Target with conditions (G:178)                                      |
| 23 | Server topic add/remove, ≤1,000 per call                                                                                                                                                                                                            | done                                                                                                                                                                        | `subscribe_to_topic` `lib:1502`; `unsubscribe_from_topic` `lib:1529` | G:85-116                                                            |
| 24 | Topic info (a token's topics, topic size)                                                                                                                                                                                                           | partial: only the person can list their own topics (`my_topics`, `lib:1597`); the app server can't read a person's topics or a topic's size                                 | none                                                                 | G:135                                                               |
| 25 | Device groups                                                                                                                                                                                                                                       | not applicable: one person's endpoints already form the group; Firebase is removing device groups on 2027-09-29                                                             | none                                                                 | none                                                                |
| 26 | Multicast (`sendEachForMulticast`, ≤500 tokens)                                                                                                                                                                                                     | done, by person                                                                                                                                                             | `SendTarget::Recipients` ≤1,000, `lib:107`                           | G:32                                                                |
| 27 | `sendEach` (a batch of different messages)                                                                                                                                                                                                          | missing (one RPC per send)                                                                                                                                                  | none                                                                 | none                                                                |
| 28 | `validate_only`                                                                                                                                                                                                                                     | done, but local only: it checks and counts the audience without contacting providers                                                                                        | `lib:988`, `lib:1000-1019`; `svc:570`                                | Track a large send (G:328); G:349 wrongly says "the way FCM's does" |
| 29 | Delivery receipts, FCM Data API, BigQuery export                                                                                                                                                                                                    | partial: per-endpoint provider acceptance only; no device-side delivery data, no export; open tracking planned                                                              | `push_deliveries` `push:769`; `send_status` `lib:1125`               | G:784-793                                                           |
| 30 | Error codes                                                                                                                                                                                                                                         | done as sanitized reason codes                                                                                                                                              | `DeliveryReason`, `push:562`                                         | partial: 5 of 13 codes never named, no table                        |
| 31 | Removing tokens the provider says are gone                                                                                                                                                                                                          | done, with problem 4                                                                                                                                                        | `fcm:831-905`; `apns:472-491`; `wp:618-623`                          | G:761, inaccurate                                                   |
| 32 | Pruning stale tokens (no refresh in N days)                                                                                                                                                                                                         | missing: endpoints have no `refreshed_at`                                                                                                                                   | `PushEndpoint`, `push:429`                                           | none                                                                |
| 33 | FID migration (`message.token` deprecated)                                                                                                                                                                                                          | missing: transport sends `token` (`fcm:537`); Android calls `getToken` reflectively (`NotificationsPlugin.kt:172`) with firebase-messaging 26.0.2 pinned (`project.rs:145`) | none                                                                 | none                                                                |
| 34 | Retry, `Retry-After`, quotas                                                                                                                                                                                                                        | done (8 attempts, 24-hour horizon, jitter)                                                                                                                                  | `svc/push:571-1131`                                                  | G:757-767                                                           |

### B. Client SDKs

| #  | Firebase feature                                                                                | Status                                                                                                                                                     | leptoskit operation or type                                          | Guide section        |
| -- | ----------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------- | -------------------- |
| 35 | Getting a token                                                                                 | done (iOS, macOS, Android, web)                                                                                                                            | `native::enroll` `native:260`; `browser::enroll` `browser:366`       | G:544-755            |
| 36 | Token refresh (`onNewToken`, `onTokenRefresh`)                                                  | partial: reconciled at the next launch only; Android refreshes sit in the in-memory store until then; the web `pushsubscriptionchange` event isn't handled | `native::reconcile` `native:317`; `browser::reconcile` `browser:416` | G:609-622, G:753     |
| 37 | `deleteToken`                                                                                   | partial, by design: `disable` revokes the service enrollment only                                                                                          | `native:527`; `browser:468`                                          | G:626, G:749         |
| 38 | Permission prompt                                                                               | done; asking for iOS provisional permission is missing (it can only be reported)                                                                           | `guest.rs:19`; `Permission::Provisional`, plugin `lib.rs:94`         | G:596-601            |
| 39 | Auto-initialisation                                                                             | not applicable: never registers without an explicit opt-in                                                                                                 | `native:317-336`                                                     | G:609                |
| 40 | Foreground messages (`onMessage`)                                                               | missing: no handler; iOS has no foreground presentation delegate; the live inbox stands in                                                                 | none                                                                 | G:661 (Android only) |
| 41 | Background handler (`onBackgroundMessage`, `onMessageReceived`, `didReceiveRemoteNotification`) | missing (problem 1)                                                                                                                                        | none                                                                 | G:407, inaccurate    |
| 42 | Subscribing to a topic from the client                                                          | done, limited to topics the operator opened                                                                                                                | `subscribe` `lib:1565`; `unsubscribe` `lib:1582`                     | G:118-176            |
| 43 | Notification channels                                                                           | done through the plugin and `channel_id`; FCM's default-channel manifest metadata isn't generated                                                          | `guest.rs:42`; `opt:223`                                             | G:483-526            |
| 44 | Images on iOS                                                                                   | partial: sets `mutable-content` and the image URL, but provides no extension to download it                                                                | `apns:371-374`                                                       | G:386                |
| 45 | Action buttons (iOS categories, Android, web)                                                   | partial: `aps.category` is set but there's no way to register categories; Android has none; web doesn't report which action (problem 10)                   | `opt:248`; `rx:366-416`                                              | G:392                |
| 46 | Tap handling (`getInitialMessage`, `onMessageOpenedApp`)                                        | planned; the web opens the link or the inbox route                                                                                                         | `rx:366`                                                             | G:807                |
| 47 | Direct boot                                                                                     | missing                                                                                                                                                    | none                                                                 | none                 |
| 48 | Client analytics (receive and open events)                                                      | planned (open tracking)                                                                                                                                    | none                                                                 | none                 |

### C. Console composer and campaigns

| #  | Firebase feature                                        | Status                                                                                         | leptoskit operation or type             | Guide section |
| -- | ------------------------------------------------------- | ---------------------------------------------------------------------------------------------- | --------------------------------------- | ------------- |
| 49 | Composing without code                                  | planned (admin compose screen)                                                                 | none                                    | none          |
| 50 | Target by app (platform)                                | planned (device details, in progress)                                                          | none                                    | G:811         |
| 51 | Target by app version                                   | planned (in progress)                                                                          | none                                    | G:811         |
| 52 | Target by language                                      | planned (in progress)                                                                          | none                                    | G:811         |
| 53 | Target by country                                       | partial: only if the app sets `user.country` itself                                            | `set_person_attributes` `lib:1649`      | G:221-301     |
| 54 | Target by Analytics audience                            | partial: the app computes it into attributes or topics                                         | conditions                              | G:259         |
| 55 | Target by user property                                 | done                                                                                           | attributes plus `user.<key>` conditions | G:221-301     |
| 56 | Target by first open                                    | partial: `AudienceMember.first_seen` exists (`lib:323`) but conditions can't use it            | none                                    | undocumented  |
| 57 | Target by last engagement                               | missing: nothing tracks engagement; a numeric attribute would have to be kept fresh by the app | none                                    | none          |
| 58 | Target by topic                                         | done                                                                                           | `SendTarget::Topic`                     | G:81          |
| 59 | Send now / scheduled                                    | done (push goes at `visible_at`)                                                               | `SendSpec.visible_at`, `lib:405`        | G:62-66       |
| 60 | Recurring campaigns                                     | missing                                                                                        | none                                    | none          |
| 61 | Recipient's time zone                                   | planned                                                                                        | none                                    | G:810         |
| 62 | Conversion events                                       | missing (the separate product analytics service could supply them)                             | none                                    | none          |
| 63 | Experiments / A/B                                       | planned                                                                                        | none                                    | none          |
| 64 | Campaign list and history                               | missing: sends can only be looked up by key                                                    | `send_status`                           | none          |
| 65 | Duplicate a campaign, drafts                            | missing (admin planned)                                                                        | none                                    | none          |
| 66 | Reports: sends, opens, conversions                      | partial: sends and provider acceptance; opens planned                                          | `send_status`, `push_deliveries`        | G:328-349     |
| 67 | Test on device                                          | done (by person and endpoint id rather than raw token)                                         | `send_test` `lib:1746`                  | G:769         |
| 68 | Expires, Android channel, custom data, sound, iOS badge | done                                                                                           | `SendSpec`, `MessageOptions`            | G:351-424     |
| 69 | Cancel a scheduled campaign                             | done, and goes further (can cancel mid-fanout)                                                 | `cancel` `lib:1179`                     | G:68, G:349   |

### D. Adjacent products

| Product                       | Status                                                                                                                                   |
| ----------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| In-App Messaging              | Not in this crate. The live inbox, toasts (`33-toasts.md`) and code push cover display, but there is no event-triggered campaign engine. |
| Remote Config personalisation | Not covered. Code push replaces UI; there is no per-user parameter service.                                                              |
| A/B Testing                   | Notification A/B variants are planned. There is no general experiment service.                                                           |

## Ranked gaps for a team moving off Firebase

**Gaps that matter, most important first:**

1. **Receiving pushes in app code** (rows 5, 40, 41, problem 1). Forward `didReceiveRemoteNotification`/`willPresent` (iOS) and `onMessageReceived` (Android) into the plugin's message store, `postMessage` data envelopes to open web pages, and expose one guest stream API. Until then `kind: data` is inert.
2. **Tap routing** (row 46, planned). Without it, every migrated deep link breaks. Document the `lkn`/`lklink`/`lk` envelope now so apps can route by hand meanwhile.
3. **Safari and Edge Web Push hosts** (problems 2 and 3). Match `*.push.apple.com` and `*.notify.windows.com` by suffix, add a real Safari subscription to the tests, and warn about (or refuse) web data messages.
4. **APNs environment per endpoint and safer invalidation** (problem 4). Have the native module report the build's `aps-environment` with the token, store it per endpoint, pick the APNs host per endpoint, and treat `BadDeviceToken`/`DeviceTokenNotForTopic` as `operator_configuration` until they're seen together with a 410.
5. **Bulk token import** (Coming from Firebase). Add an app-only `import_push_endpoints(person, credentials)` so existing Android FCM tokens (and iOS FCM tokens over the FCM transport) work on day one, instead of waiting for every user to update.
6. **Device targeting by platform, version and language** (rows 50-52, in progress). Store it per endpoint and add `device.*` predicates to the condition grammar.
7. **Campaign list, history, drafts and duplicating** (rows 64-65), which the planned compose screen needs. Add an app-only, paginated `list_sends` that returns the stored spec, target, label and counts, plus a draft state on `sends`.
8. **Open tracking, per-send statistics and labels** (rows 19, 29, 48, 66, planned). Add a person-side `mark_opened`, called from tap routing and the web click, and aggregate it per send key.
9. **Local-time delivery and translated variants** (rows 11, 61, planned). Store the person's time zone and locale on the endpoint, add a per-time-zone fanout path, and add `SendSpec` variants keyed by locale.
10. **App-chosen collapse key and separate push TTL** (rows 6, 8). Add `options.collapse_key` and `options.push_ttl`, so short-lived pushes don't also expire the inbox row.
11. **Condition value grammar** (problem 5). Allow escaped quotes and spaces in condition values, or reject such characters in attribute values.
12. **FID migration** (row 33). Add a `PushCredential::FcmInstallation` that the transport sends as `message.fid`, and move Android enrollment off `getToken` before Firebase removes `token`.
13. **Full erasure and retention** (status table: "Physical prune of expired rows: not started"). Add `erase_person` (inbox rows, endpoints, preferences, deliveries) and a pruning job. `forget_person` is advertised for "a deleted account" but keeps all of that.
14. **Recurring sends** (row 60). Add a stored schedule plus a job that re-sends with derived keys, or document that the app's server should do it.
15. **First open and last engagement as targets** (rows 56-57). Expose `first_seen` and a `last_seen` (bumped on inbox reads and enrollment) as built-in predicates.
16. **iOS rich media** (row 44). Generate a Notification Service Extension that downloads `lkimg`.
17. **Live Activity pushes** (row 16). Add the `liveactivity` push type, with the token taken from the live-activities plugin.
18. **Action buttons end to end** (row 45). A plugin API to register iOS categories and Android actions, and report the action id to app code.
19. **The FCM data and badge bug and the reserved key** (problems 6, 7). Skip the Android notification block for data sends, and drop `message_type`.
20. **Smaller items:**
    - remaining per-platform fields (row 12: visibility, `notification_priority`, renotify, timestamp, vibrate, web low/very-low urgency, critical sound);
    - app-side read of a person's topics and topic sizes (row 24);
    - `refreshed_at` with stale-endpoint pruning (row 32);
    - provisional permission requests (row 38);
    - direct boot (row 47);
    - `sendEach`-style batches (row 27);
    - default channel metadata (row 43).

**Deprecated by Firebase, or not a fit for a per-person service:**

- Device groups (deprecated, removal 2027-09-29). The person already is the group.
- The legacy HTTP and XMPP APIs and the `/batch` endpoint (shut down in 2024).
- Instance ID server APIs for topic management (reported as deprecated in 2026). leptoskit owns its topics.
- Predictions targeting (retired) and Dynamic Links as notification links (shut down 2025).
- Per-device topics, and unrestricted topic subscription from the client. leptoskit deliberately uses per-person topics and an operator allowlist.
- Token-addressed sends as the main API. Device targeting belongs in conditions; `send_test` covers single-device checks.
- Android delivery knobs (`restricted_package_name`, `proxy`, `bandwidth_constrained_ok`, `restricted_satellite_ok`): low value.
- Auto-initialisation. It conflicts with the opt-in design.

## Part 2: Documentation check

### Accuracy mismatches

1. **`SendSpec` examples won't compile.** G:48-55, 98-105, 190-197, 266-273 and 310-317 build `SendSpec` without the `options` field. `SendSpec` has no `Default` (`lib:394-416`), so every one fails.
2. **Wrong `call` path.** G:131-132, 139, 157, 162, 464, 474, 549, 562 and 565 use `leptoskit_rpc::call(op)`. That name is the server-side `call(target, op)` (`crates/leptoskit_rpc/src/lib.rs:172-173`). The client call is `leptoskit_rpc::client::call`, which is what `32-services.md:264` uses.
3. **`UnreadCount` written as a value.** G:456 writes `UnreadCount`. The RPC macro generates a struct with braces (`crates/leptoskit_rpc_macros/src/operation.rs:1054`), so it must be `UnreadCount {}`.
4. **Missing `push::` paths.** G:550 `leptoskit_notifications::PushCredential` and G:787 `leptoskit_notifications::PushDeliveries` both live under `push::` (`push:139`, `push:769`). Only the option types are re-exported at the root (`lib:75-85`).
5. **The `sw.js` handler won't compile.** G:714-717 uses `?` in a function returning `Response`.
6. **The local-notification example fails on Apple.** G:503-511 calls `configure_channel(...).await?` unconditionally. Apple doesn't advertise that method (`apple/NotificationsPlugin.swift:178-180`), so it returns `Unavailable` and `?` exits before `show`. The plugin readme has the same contradiction (`readme.md:54` against `readme.md:114-116`).
7. **Push credentials never reach the service as documented** (G:534, G:122-124). The guide says `lk` "passes through" the environment. Services actually start with a cleared environment and only receive variables listed in `secrets` (`crates/leptoskit_cli/src/services/run.rs:212-225, 286-288`). The crate declares none (`Cargo.toml:192-197`). Someone following the guide gets push silently off.
8. **Shortened variable names are wrong** (G:540). `_VAPID_PUBLIC_KEY` and `_VAPID_SUBJECT` read like the APNs abbreviations, which gives `LEPTOSKIT_NOTIFICATIONS_VAPID_*`. The real names are `LEPTOSKIT_NOTIFICATIONS_WEB_PUSH_VAPID_*` (`svc/config:134-145`).
9. **Condition values** (G:217 against `cond:689-692` and `lib:796-808`): problem 5.
10. **Data messages "reach your app"** (G:407): problem 1. The silent badge correction (G:424): problem 6.
11. **Web data-message warning is incomplete** (G:411): Safari revokes after 3 silent pushes and Firefox spends a quota.
12. **Invalidation** (G:761): problem 4.
13. **Apple Web Push host** (G:799): problem 2.
14. **Reserved data keys** (G:795): `message_type` isn't dropped (problem 7).
15. **`validate_only`** (G:349): it does not do what FCM's does, because it never contacts a provider.
16. **Delivery status shown after the test send** (G:784-791). The guide reads `PushDeliveries` right after `SendTest`, but a test writes no delivery rows.
17. **Send counts** (G:62) leave out `targeted`, `state` and `duplicate`.
18. **"FCM's bounds"** (G:206): only the 5-topic limit is FCM's.
19. **Rustdoc contradiction**: problem 9 (`push:694-696` against G:557 and `svc/push:103-105`).
20. **Crate docs:** the "Who may call what" list leaves out `send_test` (`lib:30-36`).
21. **Crate readme:** `readme.md:17` points to "what push costs you", which the guide doesn't cover.
22. **The engineering note is out of date:**
    - §2 says only `send` and `cancel` are app-only (line 24).
    - §7 says there are "no OS push transports, no device registry" (line 64).
    - Line 102 says native and browser enrollment "remain separate work".
    - §8 says invalidation happens on `UNREGISTERED` only (line 83).
    - §1's operation list leaves out the push operations.
23. **Unused constant:** `MAX_WEB_ICON_LEN` (`opt:47`) is checked nowhere; `MAX_OPTION_URL_LEN` is used instead.

### Things the guide never explains

**Operations and types:**

- the `MarkDismissed` call (only described in prose);
- `InboxShow` `Unread`, `All` and `Dismissed`;
- reading preferences (`category_preferences`), which a settings screen needs;
- the fields of `PersonAttributeChanges` and `TopicChanges`;
- 5 `DeliveryReason` codes: `endpoint_unavailable`, `transport_unavailable`, `provider_rejected`, and `cancelled`/`opted_out` as reasons. There's also no table of codes with what to do about each;
- the shared validators: `check_send*`, `check_spec`, `check_topic`, `check_attribute_*`, `check_key`, `check_category`, `check_data`, `options::check_options`, `condition::check_condition`/`Condition::parse`, `push::check_credential`/`check_web_*`;
- browser errors `NoWorker`, `Scope`, `Subscribe`, `Denied`, `Unavailable` and `Enroll`, plus `DisableError`, `Availability::reason` and `get_registration`;
- native errors `NoPlugin`, `Register`, `Failed` and `Unavailable`, plus `disable_key`, `Retirement::Revoked` and `DisableError`;
- the `PushPlatform` wire names.

**Limits:**

- idempotency key 255; category 64; title 200; body 2,000;
- inbox and topic pages 100; schedule horizon 366 days;
- Android icon 512, tag 256, channel 64; subtitle 200; thread id 64; Apple category 64;
- action id 64, title 100; language 35; relevance score 0–1;
- APNs token 32–200 hex characters; FCM token 4,096; web endpoint 2,048;
- endpoint history page.
- Also the practical push data budget: 4 KiB of `data` is always cut back to the id-only envelope.

**Configuration:**

- how to supply the variables (`secrets` and `env` in `[services.notifications]`) and how to put a multi-line `.p8` or a JSON service account into one;
- how to generate VAPID keys (the CLI has no command for it);
- how the client gets the VAPID public key, for example through `bind`;
- `pool`, `port` (3120), `public` and `database` defaults;
- what happens when `aps-environment` doesn't match: endpoints are invalidated.

**Wire format:** the `lkn`, `lk`, `lklink`, `lkimg`, `lkdata` and `lkbadge` keys. Without them, native apps can't use `data` today.

**Platform behaviour:** iOS banners aren't shown while the app is open; Android notifications without a channel id land in the fallback channel.

**Privacy:** what is stored, for how long, and what `forget_person` leaves behind.

### Quality for app authors

- **Engineering detail on the public page.** OAuth exchange budgets and jitter (G:763-765), receipt pruning (G:751), host pinning internals (G:799), the in-memory store's internals (G:587, G:680), and the pointer to repository engineering notes (G:813). All of these belong in `docs/agents/`.
- **Sections that don't start with the problem they solve.** "Read the inbox" (G:426), "Configure a transport", "What a send does", "Web Push hosts", and the iOS enrollment section, which leads with entitlement mechanics.
- **Knowledge the reader hasn't been given:** the `services` RPC feature and `call_as_app`, live hooks, VAPID, `.p8` keys, service accounts, idempotency.
- **Order a newcomer can't follow.** All the audience material (topics, conditions, attributes, segments, everyone) comes before the reader learns to read an inbox (G:426). Local notifications sit between categories and push. Status reading is buried under "Test on a device". The "what it doesn't do" list is at line 805 instead of up front.
- **Per-platform outcomes are scattered.** There's no table of what happens while the app is closed, in the background, open, or tapped, per platform.
- **One example teaches swallowing errors:** `let _ = …` at G:156.

### Proposed structure once the planned work lands

Split the current 817 lines into a `guide/notifications/` chapter:

1. **Overview and quick start**: the model (person, send, inbox row, endpoint, delivery), a platform matrix, and the shortest path from adding the service through one send to reading and marking an inbox. About 150 lines.
2. **Sending**: send, idempotency, schedule and expiry, cancel, status, dry run, errors, limits.
3. **The inbox**: pages, unread badge (with the `badge` plugin), read and dismiss, filters, categories and a preferences screen.
4. **Audiences**: topics (server and self-subscribe, open-topics config), condition grammar reference, attributes, segments, everyone, device targeting, local time, forget and erase.
5. **Shaping messages**: options table per platform, data messages (once handlers exist), badges, images, actions, translated variants, payload budget.
6. **Push setup**: operator configuration with a real `leptoskit.toml` `secrets` block and key generation, then one subsection each for web, iOS, Android and macOS, each ending with a closed/background/open/tap table.
7. **Receiving on the device**: handlers, tap routing, open tracking.
8. **Delivery and troubleshooting**: lifecycle, reason-code table with fixes, test on device, environment mismatch.
9. **Campaigns**: admin compose, scheduling, A/B, statistics.
10. **Privacy and data retention.**
11. **Coming from Firebase.**
12. **Reference**: operations (with who may call each), types, limits, variables, config keys, features.

Engineering material moves to `docs/agents/notifications-service.md`.

## Outline for "Coming from Firebase"

1. **Who it's for and what stays.** You still need a Firebase project and `google-services.json` for Android delivery, because FCM is the transport. You stop using the console, the composer, Analytics-based targeting, and FCM topics.
2. **Concept mapping:**

   | Firebase                 | leptoskit                        |
   | ------------------------ | -------------------------------- |
   | registration token / FID | endpoint (opaque id, per person) |
   | token-addressed send     | person-addressed `Recipients`    |
   | topic (per device)       | topic (per person)               |
   | condition                | condition plus `user.*`          |
   | user property / audience | person attributes                |
   | campaign                 | send key (admin later)           |
   | `validate_only`          | `validate_only` (local)          |
   | test on device           | `send_test`                      |
   | TTL                      | `expires_at`                     |
   | `collapse_key`           | fixed per notification           |
   | `analytics_label`        | send key                         |
   | data message             | `kind: data`                     |
   | `onMessage`              | live inbox and handlers          |
   | device group             | (none needed)                    |
3. **Code before and after** for each common call: Admin SDK `send({token})` and `sendEachForMulticast`; topic send; condition send; `subscribeToTopic(tokens)`; per-platform `android`/`apns`/`webpush` blocks mapped to `MessageOptions`; `getToken`/`onTokenRefresh` mapped to `native::enroll`/`reconcile`; web `getToken({vapidKey})` mapped to `browser::enroll`; `deleteToken` mapped to `disable`; `onMessage`/`onBackgroundMessage` mapped to the new handlers.
4. **What happens to existing tokens:**
   - Android FCM tokens stay valid when you use the same project's service account, but there's no server import yet; they re-enroll on the app update.
   - iOS FCM tokens aren't APNs tokens. The new build enrolls the raw APNs token, and you should check that its `aps-environment` matches the service.
   - Web subscriptions made for Firebase's key fail with `KeyMismatch`. The page has to unsubscribe the old subscription first, unless you own the private key of an imported key pair.
   - FCM topic memberships can't be exported. Re-subscribe people from your own records with batch calls.
   - Expect a coexistence period: send from both systems with a dedupe key, then cut over.
5. **Honest differences:**
   - Per-person model: one account per device until sign-out.
   - One APNs environment per service.
   - No app-side receive handlers or tap routing yet.
   - No campaign UI, recurring sends, local-time delivery, conversions or A/B yet.
   - Delivery status means provider acceptance only.
   - Safari Web Push needs the host fix.
   - What leptoskit adds: a durable inbox, read state shared across devices, category opt-outs, idempotent sends, cancelling mid-fanout, and a per-recipient unread badge.
6. **A migration checklist.**

## What I could not verify

- No builds, tests or devices, by instruction. Compile failures come from reading signatures. Problems 1, 6 and 8 and the iOS badge-in-background behaviour are inferred from code and platform semantics.
- Composer details beyond the targeting criteria and A/B facts on Firebase's A/B Testing page come from memory.
- The Edge push host is from memory.
- The FID client-SDK details (deprecated `getToken` in firebase-messaging 25.1.x, the Instance ID deprecation) come from third-party write-ups. The official REST reference does confirm `token` is deprecated in favour of `fid`.
- Whether `leptoskit_notifications` 0.3 is published.

Sources:

- [FCM HTTP v1 Message reference](https://firebase.google.com/docs/reference/fcm/rest/v1/projects.messages)
- [FCM send with v1 API (device groups deprecation)](https://firebase.google.com/docs/cloud-messaging/send/v1-api)
- [Firebase A/B Testing for Cloud Messaging](https://firebase.google.com/docs/ab-testing/abtest-with-console)
- [Targeting audiences with FCM (Firebase blog)](https://firebase.blog/posts/2022/12/targeting-audiences-with-fcm/)
- [Redesigned Firebase notifications dashboard](https://firebase.blog/posts/2018/11/introducing-redesigned-firebase/)
- [FCM product page](https://firebase.google.com/products/cloud-messaging/)
- [WWDC22 Meet Web Push for Safari](https://developer.apple.com/videos/play/wwdc2022/10098/)
- [WebKit: Meet Declarative Web Push](https://webkit.org/blog/16535/meet-declarative-web-push/)
- [iOS Web Push requirements (progressivewebapps.com)](https://progressivewebapps.com/background/ios-web-push/)
- [Safari subscription removal after 3 silent pushes (issue)](https://github.com/bulwarkmail/webmail/issues/1322)
- [FCM token to FID write-up (Firerun)](https://firerun.io/blog/firebase-fcm-token-fid-deprecation-2026/)
- [Batch: Android push and Firebase Installation ID](https://batch.com/blog/posts/android-push-firebase-installation-id)

PRs: none (read-only review).
