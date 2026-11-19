# Privacy and data retention

A notifications service holds personal data by its nature: who was told what, on which devices, and whether they read it. You need to be able to say what is stored, for how long, and how one person's data is removed when they ask. This page answers those three questions. Everything here happens in the service and its database, so it is the same whatever platforms your app runs on.

## What the service stores

All of it is in the service's own database, which you host. Nothing is sent to leptoskit or to any third party except the push itself, which goes to Apple, Google, or the browser's push service.

| What                 | Holds                                                                                                                                     | Who can read it                                                                          |
| -------------------- | ----------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------- |
| Inbox rows           | the person's id, the category, title, body, and `data` of each notification, and when it was read, opened, or dismissed                   | the person; your server sees counts, never another person's rows                         |
| Send records         | the idempotency key, the counts, the label, the message, and the topic or condition it went to                                            | your server                                                                              |
| Devices (endpoints)  | the person's id, the platform, the device token or browser subscription, and the language, time zone, and app version the device reported | the person and your server see everything except the credential, which nobody reads back |
| Delivery records     | which notification went to which device, its state, and its reason                                                                        | your server                                                                              |
| Category preferences | the person's id and each category they set                                                                                                | the person; your server sees counts                                                      |
| Topic subscriptions  | the person's id and each topic                                                                                                            | the person; your server sees counts                                                      |
| Attributes           | the person's id and each key and value your server set                                                                                    | your server only                                                                         |
| The person registry  | the id of each person the service knows, and when it first heard of them                                                                  | your server sees counts                                                                  |
| Campaigns            | each draft's message and audience, and the name your server passed as `by` for each event                                                 | your server                                                                              |

The service identifies a person only by the id your app gives it. It holds no name, email address, or session. What a title, a body, `data`, an attribute value, or a condition contains is up to you, so that is where personal data enters: think before putting an email address in a condition or a diagnosis in a title.

A push sent to a provider contains the title, the body, the notification's id, and the send's `data` and options where they fit. Web Push payloads are encrypted for the browser. APNs and FCM payloads are readable by Apple and Google, as with any push.

Device tokens and browser subscription keys are credentials. They are stored in the service's database so that it can push, and no operation answers them. Protect that database and its backups as you would the credentials themselves.

## How long things are kept

A service that never deletes anything gets slower and more expensive every month, and keeps personal data long after anyone needs it. So the service deletes what is over, by itself, once an hour, in small pieces that never hold up a send or an inbox. What counts as over:

| What                                                                                         | Kept for                                                          | Setting                                             | Default  |
| -------------------------------------------------------------------------------------------- | ----------------------------------------------------------------- | --------------------------------------------------- | -------- |
| A notification in someone's inbox, read or not                                               | this long after it became visible                                 | `LEPTOSKIT_NOTIFICATIONS_RETENTION_INBOX_DAYS`      | 365 days |
| A notification nobody can see any more: expired, cancelled, or a silent data message         | this long after it ended                                          | `LEPTOSKIT_NOTIFICATIONS_RETENTION_ENDED_DAYS`      | 30 days  |
| The record of one push to one device, once it finished (accepted, rejected, or skipped)      | this long after it finished                                       | `LEPTOSKIT_NOTIFICATIONS_RETENTION_DELIVERIES_DAYS` | 30 days  |
| The record of a send: its idempotency key, counts, and (for a topic or condition) its target | at least this long after the send, and as long as any row it made | `LEPTOSKIT_NOTIFICATIONS_RETENTION_SENDS_DAYS`      | 90 days  |

Each setting is a whole number of days, or `off` to keep that kind of row forever. A value that is neither refuses to start, named. Like every variable the service reads, a setting only reaches it when your `[services.notifications]` table lists its name under `secrets`:

```toml
[services.notifications]
secrets = [
	"LEPTOSKIT_NOTIFICATIONS_RETENTION_INBOX_DAYS",
	"LEPTOSKIT_NOTIFICATIONS_RETENTION_SENDS_DAYS",
]
```

```bash
LEPTOSKIT_NOTIFICATIONS_RETENTION_INBOX_DAYS=90
LEPTOSKIT_NOTIFICATIONS_RETENTION_SENDS_DAYS=off
```

The service prints what it will keep when it starts, so you can check the settings arrived.

A few rules hold whatever you set:

- **Nothing in use is deleted.** A notification with a push still waiting to go out stays, and so does that push. A send that is still working through its audience keeps every row it has written. A scheduled notification is never too old before its time has come, and one you cancelled ahead of its time is kept until that time is `ENDED_DAYS` in the past.
- **A push record never outlives its notification.** When a notification goes, its push records go with it, whatever their age.
- **A person's inbox simply gets shorter.** Deleted rows leave the inbox and the unread count the same way a dismissed one does. Nothing on an open screen breaks.
- **Campaigns are kept.** A [campaign](campaigns.md#write-a-campaign-before-you-send-it) and its events are not deleted on a timer. Once a launched campaign's send record has been deleted, the campaign can still be read and copied, and launching it again is refused instead of sending twice.
- **The send list is the send records.** A send is in [`list_sends`](sending.md#browse-the-sends-you-made) for as long as its record is kept, and a report's push counts and reads by hour are counted over the rows still stored.
- **A send's report counts what is left.** The read, opened, and dismissed counts in a [send's report](sending.md#see-what-a-send-came-to) are counted over the rows still stored, so they shrink as rows are deleted. What the send delivered is kept on the send's own record and does not change.

**How long a retry is safe.** The idempotency key is remembered for as long as the send's record is kept: at least `SENDS_DAYS` after the send, and for as long as any notification or push record from that send is still stored. With the defaults that is a year for a send that reached an inbox. During that time a repeat of the same call answers the first result and sends nothing. After the record is deleted, the service has forgotten the key, and the same call is a new send. So keep `SENDS_DAYS` longer than the longest time any of your systems might retry a send. It cannot be set below 2 days.

Retention is also the answer for personal data that isn't tied to one person's deletion request. A condition can carry personal values (`user.email == '…'`), and it is stored with its send's record until that record is deleted. To delete one person's data now, use [`forget_person`](audiences.md#erase-a-person).

## Erase one person

Retention removes old data on a timer. A deletion request needs one person's data removed now. [`forget_person`](audiences.md#erase-a-person) does that from your server: it removes the person's inbox, devices and their stored credentials, delivery records, category preferences, topic subscriptions, attributes, and their entry in the registry.

Four things it does not remove, because they are not rows about that person:

- **A send's totals.** "Delivered to 1,204 people" stays 1,204.
- **Campaign drafts.** A draft whose audience is a list of recipients keeps a forgotten person's id until you edit or archive the draft. After erasing someone, remove them from any draft that names them. Campaigns are not removed by retention either.
- **Personal values inside a condition.** A condition such as `user.email == '…'` is stored as text with its send's record, and in a draft that uses it, until that record is deleted. Prefer attributes that do not identify one person.
- **Your own records.** Whatever your app's database holds about the person is yours to delete.

The service keeps no list of people it was told to forget. If your server names the person again in a send, a subscription, an attribute call, or an [import of their device tokens](coming-from-firebase.md#moving-step-by-step), or the person turns notifications on again, the service knows them again from that moment. Erasing someone is therefore also a change to your own code: stop naming them.

If you only need a person's notifications to stop arriving on devices they are signed out of, and not their data removed, use [`revoke_person_endpoints`](push-setup.md#when-the-server-revokes-a-device) instead.
