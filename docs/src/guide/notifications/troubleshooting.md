# Delivery and troubleshooting

A person says they never got the notification. This page is how you find out why. It explains what the service does with a send's pushes, how to try a message on one real device, how to read what happened to each push, and what every reason code and error means, with what to do about each. Push delivery applies to iOS, macOS, Android, and the web; the inbox row is written whatever happens to the push, on every platform.

## What a send does

When a send writes an inbox row for a person who has enrolled devices, it also queues one **delivery** per device, for each platform whose transport is configured. A background job attempts each delivery when the notification becomes visible, so a scheduled send pushes at its time and not before.

Just before each attempt the service checks again everything that could have changed since the send: whether the send was cancelled, whether the notification expired, whether the person opted out of the category in the meantime, and whether the device is still enrolled. Any of those ends the delivery as `skipped`, with the reason, and nothing is pushed.

Then the provider answers, and one of four things happens:

- **It takes the message.** The delivery is `accepted`.
- **It says try later** (a 429, a 5xx, or a network error). The delivery is `retrying`. The service waits one minute, then two, then four, and so on up to 64 minutes, a little longer each time at random, and longer still when the provider asks for it. It makes at most eight requests, the last about two hours after the first, and then ends the delivery as `rejected` with `attempts_exhausted`.
- **It says the device is gone** (APNs's `410`, FCM's `UNREGISTERED`, a push service's `404` or `410`). The delivery is `rejected` with `unregistered`, and the endpoint is marked invalidated: nothing more is pushed to it, and it stays in the person's device list so the list can explain itself.
- **It refuses for another reason.** The delivery is `rejected`, with `operator_configuration` when the answer means your setup is wrong and `provider_rejected` otherwise. The device stays enrolled.

A delivery also has a deadline: the notification's expiry, the end of the send's [`push_ttl`](shaping-messages.md#replace-an-earlier-push-and-let-a-push-go-stale), or 24 hours after it became visible, whichever is earliest. A retry that would fall after the deadline is not made.

One device failing never undoes anything else. The inbox row was committed with the send, and the person's other devices get their own deliveries.

Delivery is **at least once**. If an answer is lost, the attempt is made again, so a device can receive the same push twice. Every push carries a collapse id (`apns-collapse-id`, the FCM collapse key, the Web Push `Topic`): the notification's id, or the send's `collapse_key` when it set one. A repeat therefore usually replaces the first banner instead of adding a second. A send's `expires_at`, or its `push_ttl` when that ends sooner, is also sent as the provider's time to live, so a provider does not hold a message longer than it is worth delivering.

## Test on a device

Before a send goes to thousands of people, try it on one real device. `send_test` pushes a spec to one person's enrolled devices and answers what each provider said. It writes nothing: no inbox row, no delivery record, no count.

```rust
let outcomes = call_as_app(leptoskit_notifications::SendTest {
    spec: test_spec,
    person: alice_id,
    endpoint: None, // or Some(endpoint_id), to try a single device
})
.await?;

for outcome in &outcomes {
    println!("{:?}: {:?} {:?}", outcome.platform, outcome.state, outcome.reason);
}
```

The spec is a full `SendSpec`, options and silent data messages included, so the device receives exactly what a real send would carry. If the spec has [variants](shaping-messages.md#say-it-in-each-persons-language), the test pushes the one this person would read. It ignores `visible_at` and `local_time` and pushes at once.

Each of the person's live devices, at most ten, is attempted exactly once. There is no queue behind a test, so a state is `accepted`, `rejected`, or `skipped`, never `enqueued` or `retrying`. A failure that a real send would retry answers `rejected` with no reason: call again to retry. There is no idempotency key either, so calling twice pushes twice.

`send_test` is refused with a `400` when no transport is configured at all, and with a `404` when `endpoint` is not one of that person's live devices.

## Read what happened to a send

Two calls answer for a real send, both from your server.

[`send_report`](sending.md#see-what-a-send-came-to) gives the totals: for each platform, how many deliveries are `enqueued`, `retrying`, `accepted`, `rejected`, and `skipped`, and how many ended for each reason. Start there.

`push_deliveries` lists the deliveries one by one, when you need to know what happened for a particular person:

```rust
use leptoskit_notifications::push::PushDeliveries;
use leptoskit_rpc::PageArgs;

let page = call_as_app(PushDeliveries {
    send_key: "invoice-paid-42".into(),
    page: PageArgs { after: None, until: None, limit: 50 },
})
.await?;

for delivery in &page.items {
    println!(
        "{} on {:?}: {:?} {:?} after {} attempts",
        delivery.person, delivery.platform, delivery.state, delivery.reason, delivery.attempts,
    );
}
```

Each item names the notification, the person, the endpoint's opaque id, the platform (`apns`, `fcm`, or `web_push`), the state, the reason when there is one, how many attempts were made, and when. It never carries a device token, a subscription, or anything the provider said in its own words.

Delivery records are kept for [30 days after they finish](privacy-and-retention.md#how-long-things-are-kept) by default, so look while they are there.

### Delivery states

| State      | Meaning                                                                                           |
| ---------- | ------------------------------------------------------------------------------------------------- |
| `enqueued` | waiting for its first attempt, for example because the notification is scheduled                  |
| `retrying` | attempted, failed in a way that may pass, and waiting to try again                                |
| `accepted` | the provider took the message                                                                     |
| `rejected` | the provider refused it for good, or the attempts or the time ran out                             |
| `skipped`  | never sent: the notification or the device was no longer in a state to push to when its turn came |

**`accepted` means the provider took the message. It does not mean a person saw it.** Focus modes, notification permissions, a switched-off phone, and quiet hours all sit between the provider and the person, and the service cannot see past the provider's answer. To learn whether people opened a notification, [record opens](receiving.md#record-the-open).

## Reason codes

Every delivery that ends as `rejected` or `skipped` has one of these reasons. They are the service's own words: a provider's text is never passed on.

### Skipped

| Reason                  | What it means                                                                                                                                                                                                             | What to do                                                                                                                                                       |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `cancelled`             | the send was cancelled before this push went out                                                                                                                                                                          | nothing: this is the cancel working                                                                                                                              |
| `expired`               | the notification's `expires_at` passed before this push went out, or the send's `push_ttl` ran out: the push waited longer than its lifetime, or a `push_ttl` of `0` had its one attempt and the provider did not take it | give the send a later expiry or a longer `push_ttl` if people should still get it                                                                                |
| `opted_out`             | the person opted out of the send's category after the send was made                                                                                                                                                       | nothing: this is the opt-out working                                                                                                                             |
| `endpoint_revoked`      | the device was removed before this push went out: the person signed out or removed it, your server revoked it, the next person on the device took it over, or the person was erased                                       | nothing; the device gets pushes again when its person turns notifications back on                                                                                |
| `endpoint_unavailable`  | the device was already marked gone by a provider's earlier answer, or a browser's push service is no longer one the service posts to                                                                                      | nothing; the app enrolls again when the person next opens it and turns notifications on                                                                          |
| `transport_unavailable` | the device's platform has no transport configured on the service now, though it had one when the device enrolled                                                                                                          | set that platform's variables again and list them under `secrets` ([configure a transport](push-setup.md#configure-a-transport))                                 |
| `subscription_unusable` | a browser's subscription has a key the service cannot encrypt to, so no push can ever reach it                                                                                                                            | have the page unsubscribe and enroll again                                                                                                                       |
| `platform_unsupported`  | the send is a silent data message and the device is a browser                                                                                                                                                             | nothing: data messages are [never pushed to browsers](shaping-messages.md#silent-data-messages). Use a visible send, or let the open page's live inbox update it |

### Rejected

| Reason                   | What it means                                                                                                                                      | What to do                                                                                                                                                                |
| ------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `unregistered`           | the provider says the device or subscription no longer exists: the app was uninstalled, the token expired, or the browser dropped the subscription | nothing; the endpoint is invalidated and no more pushes are sent to it                                                                                                    |
| `operator_configuration` | the provider refused because of your setup, not the device. The device stays enrolled                                                              | see [the checklist below](#when-the-reason-is-operator_configuration)                                                                                                     |
| `provider_rejected`      | the provider refused the message for good for a reason the other codes do not name, such as a request it calls malformed                           | try the same spec with [`preview_push`](shaping-messages.md#preview-a-push) and `send_test`; if a plain send with no options is accepted, an option is the cause          |
| `oversized`              | the message does not fit the provider's size limit even after everything optional was removed                                                      | shorten the title and body; `preview_push` shows the size on each platform                                                                                                |
| `attempts_exhausted`     | eight requests were made and the provider never took the message                                                                                   | a provider outage longer than two hours, or credentials the provider kept refusing (the delivery showed `operator_configuration` while it retried). Check the credentials |
| `horizon_exceeded`       | 24 hours passed since the notification became visible without the provider taking it                                                               | the same: the push was abandoned because it would arrive too late to matter                                                                                               |
| `retry_overflow`         | the provider asked for a retry time the service could not represent                                                                                | should not happen with a real provider; if you see it, report it                                                                                                          |

A delivery that is still `retrying` can also show `operator_configuration`. That means the provider refused the service's own credentials (a signing key APNs does not accept, an FCM service account without permission, a Firebase project it cannot find), or the service could not get an access token at all, and it is trying again in case you fix it. If you do not, the delivery ends as `attempts_exhausted` or `horizon_exceeded`.

### When the reason is `operator_configuration`

This reason means the fix is yours, and it usually affects every device on that platform at once. It shows as a final reason, or on a `retrying` delivery while the service waits. Check, by platform:

**APNs (iOS and macOS)**

- `LEPTOSKIT_NOTIFICATIONS_APNS_ENVIRONMENT` does not match the build. A build run from Xcode registers with `sandbox`. A TestFlight, App Store, or notarized build registers with `production`. A token from one is refused by the other. One service has one environment, so development builds and store builds cannot share a service: run a second service, or test push with TestFlight builds.
- `LEPTOSKIT_NOTIFICATIONS_APNS_TOPIC` is not the app's bundle id.
- The `.p8` key, its key id, or the team id is wrong, or the key was revoked in your Apple developer account.

**FCM (Android)**

- The device's token belongs to another Firebase project than the service account. The app's `google-services.json` and the service's `LEPTOSKIT_NOTIFICATIONS_FCM_SERVICE_ACCOUNT` must come from the same project.
- The service account lacks permission to send, or the Firebase Cloud Messaging API is not enabled for the project.

**Web Push**

- The VAPID key pair changed since the browser subscribed. A subscription works only with the key pair it was made for. Put the old pair back, or have each page unsubscribe and enroll again.
- `LEPTOSKIT_NOTIFICATIONS_WEB_PUSH_VAPID_SUBJECT` is not a `mailto:` or `https:` address the push service accepts.

After fixing the setup, send again with a new idempotency key. Deliveries that already ended are not retried.

## Errors the service answers

A call the service refuses answers an HTTP status and a message. The status tells you the kind of mistake; the message names the field or the limit.

| Status | Kind      | When you see it                                                                                                     |
| ------ | --------- | ------------------------------------------------------------------------------------------------------------------- |
| `400`  | invalid   | the input broke a rule: a field too long, a bad topic name, a condition that does not parse, a limit reached        |
| `403`  | forbidden | the caller may not do this: a person calling a server-only operation, nobody signed in, a topic that is not open    |
| `404`  | not found | the thing named does not exist, or is not the caller's                                                              |
| `409`  | conflict  | the call is valid but clashes with what is stored: a reused key, a send already visible, a draft someone else saved |
| `500`  | server    | something is wrong inside the service; the service's log has the detail                                             |
| `503`  | retry     | the service could not do it just now; the same call can be made again                                               |

In Rust the error is a `leptoskit_rpc::FnError`. A `400` arrives as `FnError::Invalid` with the message, so a form can show it beside the field.

### `403`: the caller may not do this

| Message starts with                          | Cause                                                                                                       | Fix                                                                                                               |
| -------------------------------------------- | ----------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------- |
| `only the app …`                             | a server-only operation was called without the service's token: from a screen, or with `call` on the server | call it from server code with `call_as_app`. [The reference](reference.md#operations) says who may call what      |
| `sign in to read your notifications`         | a person's operation was called with nobody signed in, or the service is not told who is signed in          | sign the person in; check `trust = "first-party"` on the service's table, and `client::configure` in a native app |
| `this topic isn't open to self-subscription` | a person tried to subscribe to a topic that `LEPTOSKIT_NOTIFICATIONS_OPEN_TOPICS` does not match            | open the topic, or subscribe the person from your server with `subscribe_to_topic`                                |
| `this campaign goes to everyone`             | `launch_campaign` on a draft whose target is everyone, without `broadcast: true`                            | pass `broadcast: true` once your server has checked that this person may send to everyone                         |

### `404`: not found

| Message                                                         | Cause                                                                                                                  |
| --------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------- |
| `no send with this idempotency key exists`                      | the key is wrong, or the send's record was deleted by retention                                                        |
| `no notification with this id is in your inbox`                 | the id is wrong, belongs to someone else, or the row was deleted                                                       |
| `no endpoint with this id is yours`                             | the device was already removed, or belongs to someone else                                                             |
| `no live endpoint with this id is this person's`                | `send_test` or `test_campaign` named a device that is not that person's, or that a provider invalidated                |
| `this enrollment is gone: …`                                    | a launch tried to renew an enrollment the server revoked. `reconcile` handles this for you and answers that it is gone |
| `no campaign with this id exists`                               | the id is wrong                                                                                                        |
| `this send was made before the service kept a send's message …` | `duplicate_send` on a send from an older version, which has nothing to copy                                            |

Another person's notification, endpoint, or send is always "not found", never "forbidden": the service does not confirm that something exists to someone who may not see it.

### `409`: conflict

| Message starts with                                                             | Cause and fix                                                                                                           |
| ------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| `this idempotency key was already used for a different send`                    | the same key with a different message, audience, schedule, options, or label. Use a new key for a new send              |
| `this send is already visible, so it can no longer be cancelled`                | `cancel` after the send's time. For a local-time send, after the last person's time                                     |
| `this device belongs to another account`                                        | a browser subscription's address was presented with different keys than the ones stored. The owner must revoke it first |
| `this campaign was saved since revision …`                                      | someone else saved the draft. Read it again and redo the change                                                         |
| `only a draft can be …`                                                         | the campaign is launched, cancelled, or archived. Duplicate it and work on the copy                                     |
| `this campaign is …, and can't be launched`                                     | the same: duplicate it and launch the copy                                                                              |
| `this campaign was launched, and the record of its send has since been deleted` | retention removed the send's record. Duplicate the campaign to send again                                               |
| `this campaign was never launched, so there is no send to cancel`               | archive it instead                                                                                                      |
| `this campaign is archived`                                                     | cancel its send directly, with `cancel` and the campaign's `send_key`                                                   |

### `400`: limits and refusals worth knowing

Most `400`s name a field and its limit and need no table. These are the ones that are not about one field:

| Message starts with                                             | Cause and fix                                                                                                                        |
| --------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------ |
| `push is not configured on this server`                         | a device tried to enroll, or you called `send_test`, and no transport is on. The variables are missing or not listed under `secrets` |
| `push for … is not configured on this server`                   | the same, for one platform: for example an iPhone enrolling while only FCM is configured                                             |
| `a person holds at most 10 live endpoints; revoke one first`    | the person has ten devices enrolled. Let them remove one from their device list                                                      |
| `a person holds at most 500 topics; unsubscribe from one first` | a person's own `subscribe` at the limit                                                                                              |
| `a topic batch names 1 to 1000 people`                          | split the batch                                                                                                                      |
| `an attribute batch names 1 to 1000 people`                     | split the batch                                                                                                                      |
| `an attribute call names at most 100 keys …`                    | split the change into several calls                                                                                                  |
| `there are already 1000 draft campaigns`                        | launch or archive some drafts                                                                                                        |
| `choose who this campaign is for before launching it`           | the draft has no target                                                                                                              |
| `this campaign was scheduled for …, which has passed`           | pick a new time, or clear the schedule to send now                                                                                   |
| `this campaign has recorded 100 tests`                          | duplicate the campaign to go on testing                                                                                              |
| `local_time.default_time_zone: …`                               | the send's default time zone is not an IANA name the service knows                                                                   |
| `… isn't a cursor of this query`                                | `after` or `until` was not a value a previous page answered                                                                          |

A condition that does not parse is also a `400`, and its message gives the byte where parsing stopped.

## When the service will not start

The service checks its configuration when it starts and stops with a message rather than run half configured. `leptoskit dev` prints the message.

| Message                                                                    | Fix                                                                                                             |
| -------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------- |
| `the … push transport is half configured … Missing: …`                     | set every variable of that transport, or none. The message lists the missing ones                               |
| `LEPTOSKIT_NOTIFICATIONS_APNS_ENVIRONMENT is …: use sandbox or production` | the value is not one of the two                                                                                 |
| `LEPTOSKIT_NOTIFICATIONS_OPEN_TOPICS holds …, which isn't a topic pattern` | a pattern uses a character a topic cannot contain                                                               |
| `LEPTOSKIT_NOTIFICATIONS_RETENTION_… isn't a retention setting`            | the variable's name is misspelled: the four are `INBOX_DAYS`, `ENDED_DAYS`, `DELIVERIES_DAYS`, and `SENDS_DAYS` |
| `LEPTOSKIT_NOTIFICATIONS_RETENTION_… is …: use a whole number of days`     | the value is not a number in range, or `off`                                                                    |

A VAPID pair whose halves do not belong together, a service account that is not the JSON Google issues, and an extra push host that is not a plain host name are refused at startup too, each with a message naming the variable.

## Errors in your app

Enrollment and receiving run in your app, and they answer Rust errors rather than statuses.

**`browser::availability()`** answers where a browser stands before you ask for anything:

| Answer            | Meaning                                                   | What to show                                                               |
| ----------------- | --------------------------------------------------------- | -------------------------------------------------------------------------- |
| `Ready`           | push can be offered                                       | the switch                                                                 |
| `InsecureContext` | the page is not served over `https` (or from `localhost`) | nothing: serve the site over `https`                                       |
| `Unsupported`     | the browser has no service worker or no Push API          | on iPhone and iPad, "add this app to your Home Screen"; elsewhere, nothing |
| `Denied`          | the person refused notifications for this site            | how to allow them in the browser's site settings                           |

**`browser::EnrollError`**

| Variant       | Meaning                                                                                                           |
| ------------- | ----------------------------------------------------------------------------------------------------------------- |
| `Unavailable` | push cannot work here; the text is one of the availability reasons above                                          |
| `Denied`      | the person refused the permission prompt                                                                          |
| `NoWorker`    | no service worker is registered: register the receiver script and wait for `ready` first                          |
| `Scope`       | no worker is registered at exactly the scope named in `EnrollOptions::worker_scope`                               |
| `Subscribe`   | the browser refused to subscribe, most often because the call was not made from a click or tap                    |
| `KeyMismatch` | the browser already holds a subscription made for a different VAPID key, such as Firebase's. Unsubscribe it first |
| `Enroll`      | the subscription was made but the service refused it; the inner error is one of the statuses above                |

**`native::EnrollError`** (iOS, macOS, Android)

| Variant            | Meaning                                                                                                                                                                                                                                                             |
| ------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Unavailable`      | this platform has no native enrollment: Windows, Linux, the terminal, or a build that is not a native app's guest                                                                                                                                                   |
| `NoPlugin`         | `notifications` is missing from `[native] plugins`                                                                                                                                                                                                                  |
| `Register`         | asking the system to register failed before it answered                                                                                                                                                                                                             |
| `Failed`           | the system refused. The text is a stable code: `aps-environment` (the entitlement is missing or does not match the profile), `firebase-not-configured` (no `[native.android] push`, or no `google-services.json`), `token-shape`, or `os-refused` for anything else |
| `Unconfirmed`      | the system has not answered yet. The previous enrollment still stands; try again later                                                                                                                                                                              |
| `Enroll`           | the token was obtained but the service refused it; it carries the enrollment to keep, if there is one                                                                                                                                                               |
| `RetireFailed`     | the new enrollment is live, but revoking the old one failed; the next `reconcile` retries it                                                                                                                                                                        |
| `OwnershipUnknown` | the new enrollment is live, but the service could not be asked whose the old one was; the next `reconcile` retries it                                                                                                                                               |

**`ReceiveError`**, from a `Receiver`

| Variant       | Meaning                                                                                                                                           |
| ------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Unavailable` | nothing can be received here: Windows, Linux, the terminal, a browser without service workers, or a native app whose host files predate receiving |
| `NoPlugin`    | `notifications` is missing from `[native] plugins`                                                                                                |
| `Host`        | the host failed to answer; the text says how                                                                                                      |

## A checklist for "the push never arrived"

Work down the list. Each step rules out one layer.

1. **Is the inbox row there?** Open the app as that person. If the item is not in the inbox, the push is not the problem: the person was not in the audience, opted out of the category (`skipped_opted_out` in the send's result), or the send is scheduled or cancelled.
2. **Did the service start with push on?** If the variables are set but not listed under `secrets` in `[services.notifications]`, the service never sees them and push is simply off. `notifications_overview` says, per platform, whether a transport is configured.
3. **Is the device enrolled?** `person_endpoints` lists the person's devices from your server. No device means the app never enrolled: the person did not allow notifications, the enroll call failed, or they signed out.
4. **What did the provider say?** Call `send_test` for that person. `accepted` means the provider has it and the rest is the device: Focus, notification settings, battery saving, or an app that is open (see [what each platform does](receiving.md#what-each-platform-does)). Anything else is a reason code from the tables above.
5. **Was it a data message?** A silent data message shows nothing, is never pushed to a browser, and on iOS is delivered only when the system chooses to wake the app.
6. **Is the build's environment the service's?** On Apple platforms a development build and a production service, or the reverse, give `operator_configuration` on every push.
