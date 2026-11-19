# Reference

This page lists everything in one place for when you know what you are looking for: every operation and who may call it, every limit, every variable the service reads, the crate's features, and what to plan for when you deploy or upgrade. The other pages of the chapter explain each of these with an example.

## Operations

Every operation is a struct you pass to a call function. The struct's name is the operation's name in upper camel case: `send_status` is `leptoskit_notifications::SendStatus`. Operations in a module are under that module: `push::PushDeliveries`, `campaign::LaunchCampaign`, `recurrence::CreateRecurrence`, `catalogue::Topics`, `preview::PreviewPush`.

There are two kinds of caller, and no operation serves both:

- **Your app's server** calls with `leptoskit_rpc::client::call_as_app`, which signs the call with the service's token. Anyone else gets a `403`.
- **The signed-in person** calls with `leptoskit_rpc::client::call` or a hook such as `use_live`, from a screen. The service learns who they are from your app, never from an argument, so these operations only ever touch the caller's own rows. With nobody signed in they answer `403`.

### Called by your server

| Operation                 | Module       | What it does                                                         | Explained in                                                        |
| ------------------------- | ------------ | -------------------------------------------------------------------- | ------------------------------------------------------------------- |
| `send`                    | root         | makes a send, or with `validate_only` counts its audience            | [Sending](sending.md#send-from-your-server)                         |
| `cancel`                  | root         | cancels a send that is not yet visible                               | [Sending](sending.md#cancel-a-scheduled-send)                       |
| `send_status`             | root         | a send's state and counts                                            | [Sending](sending.md#track-a-large-send)                            |
| `send_report`             | root         | what became of a send: inbox counts, push counts, reads by hour      | [Sending](sending.md#see-what-a-send-came-to)                       |
| `list_sends`              | root         | the sends that were made, newest first                               | [Sending](sending.md#browse-the-sends-you-made)                     |
| `send_test`               | root         | pushes a spec to one person's devices once, writing nothing          | [Troubleshooting](troubleshooting.md#test-on-a-device)              |
| `preview_push`            | `preview`    | the request each provider would be sent, and its size                | [Shaping messages](shaping-messages.md#preview-a-push)              |
| `subscribe_to_topic`      | root         | subscribes up to 1,000 people to a topic                             | [Audiences](audiences.md#send-to-a-topic)                           |
| `unsubscribe_from_topic`  | root         | unsubscribes up to 1,000 people from a topic                         | [Audiences](audiences.md#send-to-a-topic)                           |
| `set_person_attributes`   | root         | sets and removes attributes for up to 1,000 people                   | [Audiences](audiences.md#describe-people-with-attributes)           |
| `person_attributes`       | root         | one person's attributes                                              | [Audiences](audiences.md#describe-people-with-attributes)           |
| `forget_person`           | root         | erases everything stored about one person                            | [Audiences](audiences.md#erase-a-person)                            |
| `notifications_overview`  | `catalogue`  | how many people, devices per platform, and topics the service knows  | [Audiences](audiences.md#know-your-audience-before-you-send)        |
| `topics`                  | `catalogue`  | topics with their subscriber counts                                  | [Audiences](audiences.md#know-your-audience-before-you-send)        |
| `attribute_keys`          | `catalogue`  | attribute keys with how many people hold each                        | [Audiences](audiences.md#know-your-audience-before-you-send)        |
| `attribute_values`        | `catalogue`  | one key's values with how many people hold each                      | [Audiences](audiences.md#know-your-audience-before-you-send)        |
| `categories`              | `catalogue`  | categories with their opt-out counts and last send                   | [Audiences](audiences.md#know-your-audience-before-you-send)        |
| `estimate_audience`       | `catalogue`  | about how many people a target would reach                           | [Audiences](audiences.md#know-your-audience-before-you-send)        |
| `person_endpoints`        | `push`       | one person's devices, without credentials                            | [Audiences](audiences.md#know-your-audience-before-you-send)        |
| `revoke_person_endpoints` | `push`       | removes every device a person enrolled                               | [Push setup](push-setup.md#when-the-server-revokes-a-device)        |
| `import_push_endpoints`   | `push`       | imports up to 500 FCM or APNs tokens your server already holds       | [Coming from Firebase](coming-from-firebase.md#moving-step-by-step) |
| `push_deliveries`         | `push`       | a send's deliveries, one by one, with state and reason               | [Troubleshooting](troubleshooting.md#read-what-happened-to-a-send)  |
| `create_campaign`         | `campaign`   | saves a new draft                                                    | [Campaigns](campaigns.md)                                           |
| `update_campaign`         | `campaign`   | saves a change to a draft, from a named revision                     | [Campaigns](campaigns.md)                                           |
| `campaign`                | `campaign`   | reads one campaign                                                   | [Campaigns](campaigns.md)                                           |
| `campaigns`               | `campaign`   | lists campaigns, newest first                                        | [Campaigns](campaigns.md)                                           |
| `campaign_events`         | `campaign`   | a campaign's history: created, tested, launched, cancelled, archived | [Campaigns](campaigns.md)                                           |
| `test_campaign`           | `campaign`   | pushes a draft to one person's devices                               | [Campaigns](campaigns.md)                                           |
| `launch_campaign`         | `campaign`   | makes the draft's send, once                                         | [Campaigns](campaigns.md)                                           |
| `cancel_campaign`         | `campaign`   | cancels a launched campaign's send                                   | [Campaigns](campaigns.md)                                           |
| `archive_campaign`        | `campaign`   | puts a campaign away                                                 | [Campaigns](campaigns.md)                                           |
| `duplicate_campaign`      | `campaign`   | copies a campaign into a new draft                                   | [Campaigns](campaigns.md)                                           |
| `duplicate_send`          | `campaign`   | makes a new draft from a send                                        | [Campaigns](campaigns.md)                                           |
| `create_recurrence`       | `recurrence` | saves a send that repeats on a schedule                              | [Sending](sending.md#a-send-that-repeats)                           |
| `recurrence`              | `recurrence` | reads one recurrence                                                 | [Sending](sending.md#a-send-that-repeats)                           |
| `recurrences`             | `recurrence` | lists recurrences, a page at a time                                  | [Sending](sending.md#a-send-that-repeats)                           |
| `pause_recurrence`        | `recurrence` | stops a recurrence from sending until it is resumed                  | [Sending](sending.md#a-send-that-repeats)                           |
| `resume_recurrence`       | `recurrence` | starts a paused recurrence again at its next occurrence              | [Sending](sending.md#a-send-that-repeats)                           |
| `end_recurrence`          | `recurrence` | ends a recurrence for good                                           | [Sending](sending.md#a-send-that-repeats)                           |

### Called by the signed-in person

| Operation                 | Module | What it does                                                        | Explained in                                                 |
| ------------------------- | ------ | ------------------------------------------------------------------- | ------------------------------------------------------------ |
| `inbox`                   | root   | a page of their inbox, live                                         | [The inbox](inbox.md#read-the-inbox-on-any-platform)         |
| `unread_count`            | root   | how many visible items they have not read, live                     | [The inbox](inbox.md#read-the-inbox-on-any-platform)         |
| `mark_read`               | root   | marks one item read                                                 | [The inbox](inbox.md#read-the-inbox-on-any-platform)         |
| `mark_dismissed`          | root   | dismisses one item                                                  | [The inbox](inbox.md#read-the-inbox-on-any-platform)         |
| `mark_opened`             | root   | records that they opened one item from its push, and marks it read  | [Receiving](receiving.md#record-the-open)                    |
| `category_preferences`    | root   | the categories they have set                                        | [The inbox](inbox.md#categories-and-opt-outs)                |
| `set_category_preference` | root   | opts them out of, or back into, one category                        | [The inbox](inbox.md#categories-and-opt-outs)                |
| `subscribe`               | root   | subscribes them to an open topic                                    | [Audiences](audiences.md#let-people-subscribe-themselves)    |
| `unsubscribe`             | root   | unsubscribes them from a topic                                      | [Audiences](audiences.md#let-people-subscribe-themselves)    |
| `my_topics`               | root   | the topics they hold, live                                          | [Audiences](audiences.md#let-people-subscribe-themselves)    |
| `register_push_endpoint`  | `push` | enrolls a device; `native::enroll` and `browser::enroll` call it    | [Push setup](push-setup.md#enroll-a-device)                  |
| `push_endpoints`          | `push` | their devices, without credentials                                  | [Push setup](push-setup.md#when-the-server-revokes-a-device) |
| `push_endpoint_standing`  | `push` | whether one of their enrollments still stands; `reconcile` calls it | [Push setup](push-setup.md#when-the-server-revokes-a-device) |
| `revoke_push_endpoint`    | `push` | removes one of their devices                                        | [Push setup](push-setup.md#when-the-server-revokes-a-device) |

### Checking input before you call

The checks the service runs are public functions of the crate, so a form can run them first and show the same message without a round trip. Each answers `Result<(), String>`:

| Function                                                                  | Checks                                          |
| ------------------------------------------------------------------------- | ----------------------------------------------- |
| `check_send`                                                              | a whole send: key, spec, target                 |
| `check_spec`                                                              | a `SendSpec`, its options and variants included |
| `check_target`                                                            | a `SendTarget`, parsing a condition             |
| `check_key`, `check_category`, `check_label`, `check_data`, `check_topic` | one field each                                  |
| `check_attribute_key`, `check_attribute_value`                            | one attribute key or value                      |
| `options::check_options`                                                  | a `MessageOptions`                              |
| `condition::Condition::parse`                                             | a condition, answering the parsed tree          |

## Limits

**A send**

| What                           | Limit                                        |
| ------------------------------ | -------------------------------------------- |
| Idempotency key                | 1 to 255 bytes                               |
| Category                       | 1 to 64 bytes                                |
| Title                          | 1 to 200 bytes                               |
| Body                           | at most 2,000 bytes                          |
| `data`                         | a JSON object of at most 4,096 bytes         |
| Label                          | 1 to 64 ASCII letters, digits, `-`, `_`, `.` |
| Recipients in one send         | 1,000                                        |
| Options, as stored JSON        | 4,096 bytes                                  |
| The whole spec, as stored JSON | 8,192 bytes                                  |
| Variants                       | 32                                           |
| `visible_at` and `expires_at`  | at most 366 days ahead                       |

**Options**

| What                                           | Limit                                                          |
| ---------------------------------------------- | -------------------------------------------------------------- |
| `image`, `link`, `web.icon`                    | 1,024 bytes                                                    |
| `sound` name                                   | 1 to 64 bytes of letters, digits, `-`, `_`, `.`                |
| `collapse_key`                                 | 1 to 32 letters, digits, `-`, `_` (`MAX_COLLAPSE_KEY_LEN`)     |
| `push_ttl`                                     | 0 to 2,419,200 seconds, which is 28 days (`MAX_PUSH_TTL_SECS`) |
| `android.channel_id`, `android.raw_channel_id` | 64 bytes each; one or the other                                |
| `android.icon`                                 | 512 bytes                                                      |
| `android.tag`                                  | 256 bytes                                                      |
| `apple.subtitle`                               | 200 bytes                                                      |
| `apple.thread_id`, `apple.category`            | 64 bytes each                                                  |
| `apple.relevance_score`                        | 0 to 1                                                         |
| `web.actions`                                  | 2 actions; id 64 bytes, title 100 bytes                        |
| `web.lang`                                     | 35 bytes                                                       |

**Audiences**

| What                                           | Limit                                                                          |
| ---------------------------------------------- | ------------------------------------------------------------------------------ |
| Topic name                                     | 1 to 128 bytes of letters, digits, `-`, `.`, `_`, `~`, `%`                     |
| Topics one person holds                        | 500                                                                            |
| People in one topic batch                      | 1,000                                                                          |
| Attribute key                                  | 1 to 64 bytes of `a-z`, `0-9`, `_`                                             |
| Attribute value                                | 256 bytes, no control characters                                               |
| Attributes one person holds                    | 50                                                                             |
| People in one attribute batch                  | 1,000                                                                          |
| Keys in one attribute call, `set` and `remove` | 100                                                                            |
| Condition                                      | 1,024 bytes; 5 topics; 10 attribute and device predicates; 8 levels of nesting |
| Category preferences one person holds          | 100                                                                            |
| Fanout batch                                   | 500 people                                                                     |

**Devices**

| What                                          | Limit                                                         |
| --------------------------------------------- | ------------------------------------------------------------- |
| Live endpoints one person holds               | 10, plus up to 10 invalidated ones kept for their device list |
| Endpoints in one `import_push_endpoints` call | 1 to 500 (`MAX_IMPORT_ENDPOINTS`); FCM and APNs tokens only   |
| APNs device token                             | 32 to 200 lower-case hex characters                           |
| FCM registration token                        | 4,096 bytes                                                   |
| Web Push endpoint address                     | 2,048 bytes                                                   |
| Device locale                                 | 35 bytes                                                      |
| Device time zone, app version                 | 64 bytes each                                                 |
| Push attempts for one delivery                | 8, within 24 hours of the notification becoming visible       |

**Pages**

| List                                                  | Largest page |
| ----------------------------------------------------- | ------------ |
| `inbox`, `my_topics`, `list_sends`, `campaign_events` | 100          |
| `campaigns`, `recurrences`, and the catalogue lists   | 50           |

**Campaigns**

| What                        | Limit          |
| --------------------------- | -------------- |
| Name                        | 1 to 120 bytes |
| Note                        | 1,000 bytes    |
| `by`                        | 1 to 128 bytes |
| Drafts at one time          | 1,000          |
| Recorded tests per campaign | 100            |

**Recurrences**

| What                                                | Limit                  |
| --------------------------------------------------- | ---------------------- |
| Name                                                | 1 to 120 bytes         |
| Note                                                | 1,000 bytes            |
| `by`                                                | 1 to 128 bytes         |
| Recurrences that have not ended (active and paused) | 100                    |
| How often                                           | at most once a day     |
| `Daily { every }`                                   | 1 to 365 days          |
| `Weekly { every }`                                  | 1 to 52 weeks          |
| `Monthly { every }`                                 | 1 to 12 months         |
| `ends: After(count)`                                | 1 to 10,000 sends      |
| `starts_on`                                         | at most 366 days ahead |
| How late a missed occurrence is still sent          | 24 hours               |
| The stored `error`                                  | 256 bytes              |

**Counts in the catalogue** stop at 100,000 for a topic, attribute, or category, at 1,000,000 for the overview's people, and at 1,000 for the overview's topics. `estimate_audience` walks at most 5,000 people.

## Variables the service reads

The service starts with an empty environment. It receives a variable only when the `secrets` list of your `[services.notifications]` table names it. A variable that is set but not listed does not reach the service.

| Variable                                             | For                                                                 | Explained in                                                                    |
| ---------------------------------------------------- | ------------------------------------------------------------------- | ------------------------------------------------------------------------------- |
| `LEPTOSKIT_NOTIFICATIONS_APNS_KEY_P8`                | APNs: the `.p8` key's contents                                      | [Push setup](push-setup.md#configure-a-transport)                               |
| `LEPTOSKIT_NOTIFICATIONS_APNS_KEY_ID`                | APNs: the key's id                                                  | [Push setup](push-setup.md#configure-a-transport)                               |
| `LEPTOSKIT_NOTIFICATIONS_APNS_TEAM_ID`               | APNs: your team id                                                  | [Push setup](push-setup.md#configure-a-transport)                               |
| `LEPTOSKIT_NOTIFICATIONS_APNS_TOPIC`                 | APNs: the app's bundle id                                           | [Push setup](push-setup.md#configure-a-transport)                               |
| `LEPTOSKIT_NOTIFICATIONS_APNS_ENVIRONMENT`           | APNs: `sandbox` or `production`, for a device that reported neither | [Push setup](push-setup.md#configure-a-transport)                               |
| `LEPTOSKIT_NOTIFICATIONS_FCM_SERVICE_ACCOUNT`        | FCM: the service-account JSON                                       | [Push setup](push-setup.md#configure-a-transport)                               |
| `LEPTOSKIT_NOTIFICATIONS_WEB_PUSH_VAPID_PRIVATE_KEY` | Web Push: the private key                                           | [Push setup](push-setup.md#configure-a-transport)                               |
| `LEPTOSKIT_NOTIFICATIONS_WEB_PUSH_VAPID_PUBLIC_KEY`  | Web Push: the public key                                            | [Push setup](push-setup.md#configure-a-transport)                               |
| `LEPTOSKIT_NOTIFICATIONS_WEB_PUSH_VAPID_SUBJECT`     | Web Push: your contact address                                      | [Push setup](push-setup.md#configure-a-transport)                               |
| `LEPTOSKIT_NOTIFICATIONS_WEB_PUSH_EXTRA_HOSTS`       | Web Push: more push services                                        | [Push setup](push-setup.md#which-push-services-a-browser-may-use)               |
| `LEPTOSKIT_NOTIFICATIONS_OPEN_TOPICS`                | topics people may join themselves                                   | [Audiences](audiences.md#let-people-subscribe-themselves)                       |
| `LEPTOSKIT_NOTIFICATIONS_RETENTION_INBOX_DAYS`       | retention, default 365                                              | [Privacy and data retention](privacy-and-retention.md#how-long-things-are-kept) |
| `LEPTOSKIT_NOTIFICATIONS_RETENTION_ENDED_DAYS`       | retention, default 30                                               | [Privacy and data retention](privacy-and-retention.md#how-long-things-are-kept) |
| `LEPTOSKIT_NOTIFICATIONS_RETENTION_DELIVERIES_DAYS`  | retention, default 30                                               | [Privacy and data retention](privacy-and-retention.md#how-long-things-are-kept) |
| `LEPTOSKIT_NOTIFICATIONS_RETENTION_SENDS_DAYS`       | retention, default 90                                               | [Privacy and data retention](privacy-and-retention.md#how-long-things-are-kept) |

## Settings in `leptoskit.toml`

| Setting                                     | What it does                                                                            |
| ------------------------------------------- | --------------------------------------------------------------------------------------- |
| `[services.notifications] crate`, `version` | names the service's crate; the table's name must be `notifications`                     |
| `[services.notifications] trust`            | `"first-party"` so the service is told who is signed in                                 |
| `[services.notifications] forward_identity` | on by default for a first-party service                                                 |
| `[services.notifications] features`         | `["service", "postgres"]` when your app's database is Postgres                          |
| `[services.notifications] secrets`          | the variables above that the service may read                                           |
| `[native] plugins = ["notifications"]`      | adds the notifications plugin to a native app: local banners, enrollment, and receiving |
| `[native.ios] aps-environment`              | turns push on for the iOS build                                                         |
| `[native.android] push`                     | turns push on for the Android build; needs `android/app/google-services.json`           |
| `[native.macos] aps-environment`            | turns push on for the macOS build                                                       |
| `[native.macos] provisioning-profile`       | the profile a push-enabled Mac app embeds; required with `aps-environment`              |
| `[app.icons] notification`                  | the small icon an Android push shows                                                    |

A push-enabled Mac app also needs the `LEPTOSKIT_MACOS_IDENTITY` environment variable for local builds. [Signing a release](../24-signing.md#macos-apps-that-receive-push) explains it.

The service listens on port 3120 by default, has a database of its own, and is reached through your app at `/_leptoskit/services/notifications/`.

## Features of the crate

| Feature    | Who turns it on                   | What it adds                                                                   |
| ---------- | --------------------------------- | ------------------------------------------------------------------------------ |
| none       | your server and shared code       | the operations' types and the checks                                           |
| `browser`  | your web build                    | the `browser` module: the receiver script, enrollment, and `browser::Receiver` |
| `native`   | your iOS, Android, macOS build    | the `native` module: enrollment and `native::Receiver`                         |
| `service`  | `leptoskit`, to build the service | the service itself, on SQLite                                                  |
| `postgres` | you, in the service's table       | the service on Postgres                                                        |

## Deploying and upgrading

The service ships the way [services](../32-services.md#deploying) do: `leptoskit build` builds its binary, and the deploy targets run it beside your app with its own database. Its `healthz` and `readyz` come from the service framework, and its migrations apply when it starts.

### Upgrading a large database

A new version of the service can change its tables, and it makes those changes when it starts, before it serves anything. On a small database that takes no time you would notice. On a large one a few of the changes take as long as copying or indexing the table, and the service is not serving while they run. Plan a quiet moment for these:

| Change                                                            | SQLite                                                                                                                               | Postgres                                                                                                                   |
| ----------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------- |
| `topics_and_fanout` (adds fanout state to send records)           | copies every send record and builds an index over every notification                                                                 | builds a unique index over every notification and rewrites every send record; writes to both tables wait until it finishes |
| `message_options` (adds a notification's kind and options)        | copies every notification and rebuilds its indexes                                                                                   | reads every notification once to check the new column; nothing can read or write notifications until it finishes           |
| `send_history` (the send list and the audience catalogue)         | builds two small indexes over send records and one each over attributes, category preferences, and devices; writes wait, reads go on | the same five builds; writes to those tables wait, reads go on. You can build them ahead with no waiting, as below         |
| `scale_indexes` (indexes for the unread count and the push queue) | builds one index over every notification and two over every push record; writes wait, reads go on                                    | the same three builds; writes to those two tables wait, reads go on. You can build them ahead with no waiting, as below    |

The other changes add empty tables or empty columns and take no measurable time at any size. That includes `campaigns`, which adds two empty tables, `recurring_sends`, which adds one, and `endpoint_seen`, which adds one empty column recording when each device was last heard from.

On Postgres you can take the wait out of `scale_indexes` and `send_history` by building their indexes yourself before you upgrade, while the old version is still running. Build each with `CREATE INDEX CONCURRENTLY`, under exactly the name and columns the migration file uses, and check that each one finished valid. The migration then finds them and only has a moment's work left. Each migration file's own comment has the details.

Rolling a change back is for emergencies, and some rollbacks lose data: the comment at the top of each `down.sql` says what. Rolling `message_options` back cancels every silent data message, because the older version could not tell them from ordinary notifications and would have shown them in people's inboxes.
