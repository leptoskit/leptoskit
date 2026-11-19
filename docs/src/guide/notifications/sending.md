# Sending

Your server knows something a person should hear about. This page shows how to send it so that a retry never sends it twice, how to schedule and cancel it, and how to find out what became of it afterwards. Everything here is called from your app's server and works the same whatever platforms your app runs on. A person's device only ever reads.

## Send from your server

Only your app's server sends. Add the crate and call `send` with `call_as_app`, which presents the service's token (that's [`call_as_app`](../32-services.md#calling-as-the-app): it needs the `services` RPC feature on your server, which `leptoskit`'s `server` feature turns on):

```toml
[dependencies]
leptoskit_notifications = "0.3"
```

```rust
use leptoskit_notifications::SendSpec;
use leptoskit_notifications::SendTarget;
use leptoskit_rpc::client::call_as_app;

let sent = call_as_app(leptoskit_notifications::Send {
    idempotency_key: format!("invoice-paid-{invoice_id}"),
    spec: SendSpec {
        category: "billing".into(),
        title: "Invoice paid".into(),
        body: format!("Invoice #{invoice_id} is settled. Thank you!"),
        data: Some(serde_json::json!({ "screen": "invoice", "id": invoice_id }).to_string()),
        ..SendSpec::default()
    },
    target: SendTarget::Recipients(vec![alice_id, bob_id]),
    validate_only: false,
})
.await?;
```

Every named recipient gets one durable row of their own, visible immediately — or at `visible_at`, when you schedule one — until `expires_at`. A recipients send is one transaction, so it lands all at once or not at all, and answers with its final counts in a `SendResult`:

| Field               | What it holds                                                                              |
| ------------------- | ------------------------------------------------------------------------------------------ |
| `delivered`         | how many inbox rows the send wrote                                                         |
| `skipped_opted_out` | how many recipients were skipped because they opted out of the category                    |
| `recipients`        | how many distinct people the call named; 0 for a topic, condition, or everyone send        |
| `targeted`          | how many people the target matched: every named recipient, or the audience found so far    |
| `state`             | `complete` for a recipients send; `fanning_out` for a topic, condition, or everyone send   |
| `duplicate`         | `true` when this key was already used for the same send, and the answer is the first run's |

The `idempotency_key` is how retries stay safe: derive one stable key per logical send (like the invoice id above) and reuse it on retry. The same key with the same intent answers the first run's result and writes nothing — across restarts, and past the send's expiry, because the send's bookkeeping lives in the service's database — and the same key with a different intent is refused with a conflict, so a reused key can't quietly send something else. The key is remembered for as long as the send's record is kept, which is [at least 90 days by default](privacy-and-retention.md#how-long-things-are-kept).

`SendSpec::visible_at` and `expires_at` take RFC 3339 instants, at most a year ahead; a `visible_at` in the past means now, so the notification shows from the moment it is sent and its push has its full delivery window from then. `data` is a JSON object of at most 4 KiB your screens read to route a tap. A send names at most 1,000 recipients, its key is at most 255 bytes, its category at most 64, its title 1 to 200, and its body at most 2,000. The same checks run on both sides: `leptoskit_notifications::check_send` is the function the service itself calls, so your server can reject bad input without a round trip. The whole spec is bounded as the service stores it, too: at most 8 KiB as JSON, with its options at most 4 KiB of that, so a spec that passes the checks is always one the service can keep, whatever its target.

### Cancel a scheduled send

A scheduled send can be cancelled until it becomes visible:

```rust
let cancelled: u32 = call_as_app(leptoskit_notifications::Cancel {
    idempotency_key: "launch-reminder".into(),
})
.await?;
```

After it's due the service refuses with a conflict — what someone has already seen can't be unsent. (A send in [each person's local time](audiences.md#send-at-each-persons-local-time) is due at a different moment for each person, and can be cancelled for the people still waiting.) Cancelling again answers `0`, and so does cancelling a send every recipient opted out of: it delivered nothing, but the service knows the key, and cancelling it is still cancelling it.

A send to a topic, a condition, or everyone can have far more rows than one write should touch. Cancelling one with more than 500 answers how many it stops, stops the send's pushes at once, and cancels the rows themselves in the background, a batch at a time; asked again before they are all done, it answers how many are left.

### A send that repeats

A message that goes out every morning, every Monday, or on the first of each month is a **recurrence**: a schedule the service keeps, which makes one send each time it comes round. You create it once, and there is no scheduler of your own to run.

```rust
use leptoskit_notifications::recurrence::{
    CreateRecurrence, NewRecurrence, RecurrenceRule, Repeat, Weekday,
};

let saved = call_as_app(CreateRecurrence {
    recurrence: NewRecurrence {
        name: "Weekly digest".into(),
        spec: SendSpec {
            category: "digest".into(),
            title: "Your week".into(),
            body: "See what happened this week.".into(),
            ..SendSpec::default()
        },
        target: SendTarget::Topic("digest".into()),
        rule: RecurrenceRule {
            repeat: Repeat::Weekly { every: 1, on: vec![Weekday::Monday] },
            at: "09:00".into(),
            time_zone: "Europe/London".into(),
            starts_on: None,
            ends: None,
            local_time: false,
        },
        note: None,
    },
    by: admin_name.clone(),
    broadcast: false,
})
.await?;

// `saved.next_at` is the instant of the first occurrence.
```

The rule's fields:

| Field        | What it says                                                                                                                                                          |
| ------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `repeat`     | `Daily { every }` for every day or every few days, `Weekly { every, on }` for chosen weekdays, or `Monthly { every, day }` for a day of the month                     |
| `at`         | the time of day as `HH:MM`, read in `time_zone`                                                                                                                       |
| `time_zone`  | an IANA time zone such as `Europe/London`                                                                                                                             |
| `starts_on`  | the first date an occurrence may fall on, as `YYYY-MM-DD`; today when left out. "Every other week" and "every third month" are counted from here                      |
| `ends`       | `RecurrenceEnd::On(date)` for a last date, or `RecurrenceEnd::After(count)` for a number of sends; when left out, it goes on until you end it                         |
| `local_time` | `true` to deliver each occurrence when `at` arrives in [each person's own time zone](audiences.md#send-at-each-persons-local-time), instead of at one instant for all |

A recurrence goes out at most once a day. Four rules decide the edge cases:

- **The time is a wall-clock time.** 09:00 in London is 09:00 in London in winter and in summer. The service follows the time zone's daylight saving changes.
- **A time the clocks skip goes out when the gap ends.** On the night the clocks jump from 02:00 to 03:00, an occurrence at 02:30 goes out at 03:30.
- **A time the clocks repeat goes out the first time only.** On the night the clocks fall back from 02:00 to 01:00, an occurrence at 01:30 goes out once, the first time it is 01:30.
- **A day the month does not have is the month's last day.** Day 31 goes out on 30 April and on 28 or 29 February.

**Each occurrence is an ordinary send.** It passes the same checks as `send`, respects category opt-outs, and reaches the same platforms. Its idempotency key is `recurrence:<id>:<the occurrence's instant in Unix seconds>`, so one occurrence is one send, however many copies of the service find it due at once. Every occurrence's send carries the label `recurrence.<id>`, which is how you find them in [`list_sends`](#browse-the-sends-you-made), and `saved.last_send_key` is the key that [`send_report`](#see-what-a-send-came-to) takes for the latest one. Because the service sets the label and the timing, the message you store has no `label`, `visible_at`, `expires_at`, or `local_time` of its own; a recurrence that carries one is refused.

**The first occurrence is the next one still ahead.** Create a 09:00 recurrence at 10:00 and it first goes out tomorrow.

**After an outage, at most one send is made up.** If the service was down when occurrences were due, it sends only the latest one it missed, and only if that one is less than 24 hours late. Everything else it missed is counted in `skipped` and never sent. Three days of downtime does not become three notifications.

**Pause, resume, and end:**

```rust
use leptoskit_notifications::recurrence::{EndRecurrence, PauseRecurrence, ResumeRecurrence};

// Nothing more goes out until it is resumed.
call_as_app(PauseRecurrence { id: saved.id, by: admin_name.clone() }).await?;

// It picks up at the next occurrence still ahead. What passed while it was
// paused is not sent.
call_as_app(ResumeRecurrence { id: saved.id, by: admin_name.clone() }).await?;

// For good. An ended recurrence cannot be resumed.
call_as_app(EndRecurrence { id: saved.id, by: admin_name }).await?;
```

`recurrence` reads one by id, and `recurrences` lists them a page at a time, optionally in one state. None of these touches a send that was already made: to stop one of those, [cancel it](#cancel-a-scheduled-send) by its key.

A few more things to know:

- **A recurrence cannot be edited.** To change its message, audience, or rule, end it and create a new one.
- **A refused occurrence pauses the recurrence.** If the service refuses an occurrence's send, it sends nothing, pauses the recurrence, and stores the reason in `error`. It does not try again by itself. Resume the recurrence once the cause is gone, or end it and create a corrected one.
- **An audience of everyone must be asked for.** `SendTarget::Everyone` is refused unless `broadcast` is `true`.
- **There can be 100 recurrences that have not ended.** The 101st is refused until you end one. The other limits are in the [reference](reference.md#limits).
- **Only your app's server can call these.**

**When the rule cannot say it.** A rule has no "last weekday of the month", no "twice a day", and no list of holidays to skip. For those, make the sends from your own scheduler:

- Give each occurrence its own idempotency key that names the occurrence, like `daily-digest-2026-11-19`. A job that runs twice for the same day then sends once.
- Give every occurrence the same [`label`](#browse-the-sends-you-made), so you can list them together.
- For "9:00 in each person's own time zone", make each one a [local-time send](audiences.md#send-at-each-persons-local-time).

## Track a large send

A recipients send answers with its final counts because it finished inside the call. A topic, condition, or everyone send answers `fanning_out` and keeps working — so ask it how it's going, and try the whole thing without spending it:

```rust
// How the fanout is going, from your server:
let progress = call_as_app(leptoskit_notifications::SendStatus {
    idempotency_key: "post-1234".into(),
})
.await?;

// What the send would reach right now, writing nothing:
let dry = call_as_app(leptoskit_notifications::Send {
    idempotency_key: "post-1235".into(),
    spec,
    target: SendTarget::Topic("posts.alice".into()),
    validate_only: true,
})
.await?;
```

`send_status` answers the state — `fanning_out`, `complete`, or `cancelled` — and the counts so far: `targeted` (everyone the target matched), `delivered`, and `skipped_opted_out`. Once the state is `complete` the counts are final; `validate_only` answers the same shape for the audience as it stands now, without writing a row, for every target: recipients, topic, condition, and everyone. It checks the send and counts the audience inside the service. Unlike FCM's flag of the same name it never contacts a push provider, so it cannot tell you whether a device token is still good; [`send_test`](troubleshooting.md#test-on-a-device) is the call that asks a provider. A dry run reads the audience a page at a time and holds nothing open while it does, so counting everyone the service knows doesn't hold up its other work. Cancelling works mid-fanout too: the fanout stops before its next batch, the rows written so far are cancelled, and the send answers `cancelled` from then on.

### See what a send came to

After a send has gone out, the question changes from "is it done?" to "did it work?": how many people got it, how many read it, and how many tapped the push. `send_report` answers that for one send, from your server:

```rust
use leptoskit_rpc::client::call_as_app;

let report = call_as_app(leptoskit_notifications::SendReport {
    idempotency_key: "spring-sale-2026".into(),
})
.await?;

println!(
    "{} delivered, {} read, {} opened",
    report.progress.delivered, report.inbox.read, report.inbox.opened,
);
```

The answer has four parts:

- `progress`: exactly what `send_status` answers, so one call is enough for a page that shows both.
- `inbox`: what became of the rows the send wrote. `rows` is how many are stored, `read` how many their recipient has read, `opened` how many tapped the push (see [recording an open](receiving.md#record-the-open)), `dismissed` how many were dismissed, and `cancelled` how many you cancelled before they were due.
- `created_at`: when the send was made.
- `label`: the send's label, when it has one (below).

A few things to know when you read the numbers:

- **`opened` only counts what your app reports.** The service cannot see a tap. It counts an open when your app calls `mark_opened` for it, which is one line in the place you handle taps. An app that doesn't call it has `opened: 0` on every send.
- **Opening counts as reading.** A person who tapped the push has seen the notification, so an open also marks it read. `read` is therefore never smaller than `opened`. Dismissing marks a row read too.
- **Each person counts once.** The counts are of inbox rows, one per person, not of devices or taps. Someone who taps the same notification on their phone and their laptop is one open.
- **The counts are of what is still stored.** The service [deletes old rows](privacy-and-retention.md#how-long-things-are-kept). Once a send's rows have been deleted, `inbox` counts what is left, down to all zeroes, while `progress` still says what the send delivered. Read a send's report while its rows are kept (a year by default) and store the numbers yourself if you need them for longer.
- **A silent data message has no open.** Nobody sees it, so its rows never count as `opened`.
- **Ask when someone looks.** A report reads the send's own rows, so its cost grows with the size of the send and not with the size of your database. That is fine for a page a person opens. Don't poll it every few seconds for every send.

A report also says what the send was and how its pushes went:

- `spec`: the send as you made it, so a screen can show the message or start a new send from it. A send made before the service kept a history may have none.
- `target`: who it went to, as a `TargetSummary`. `kind` is `recipients`, `topic`, `condition`, or `everyone`. `text` is the topic or the condition. For a recipients send there is no text, only `recipients`, the number of people listed: the list itself is not kept.
- `push`: one `PushCounts` per platform the send pushed on, with how many pushes are `enqueued`, `retrying`, `accepted`, `rejected`, and `skipped`, and `reasons`, which says why the rejected and skipped ones ended (`unregistered`, `oversized`, and so on).
- `reads_by_hour`: how many rows were read in each UTC hour, oldest first, for the first 168 hours (a week) in which any was read.

`accepted` means the provider took the push, not that a device showed it. Like the inbox counts, the push counts and the hours are counted over what is still stored.

Only your app's server can read a report. Conversions and funnels — what people did after they opened — are not part of this service.

### Browse the sends you made

A report answers for a send whose key you know. A screen that shows "what have we sent lately?" needs the list. `list_sends` answers it from your server, newest first, a page at a time:

```rust
use leptoskit_notifications::{ListSends, SendFilter, SendState};
use leptoskit_rpc::PageArgs;

let page = call_as_app(ListSends {
    page: PageArgs { after: None, until: None, limit: 20 },
    filter: SendFilter::all().in_state(SendState::Complete),
})
.await?;

for sent in &page.items {
    println!("{} · {} · {} delivered", sent.created_at, sent.title, sent.delivered);
}
```

Each `SendEntry` carries the send's key (pass it to `send_report` for the rest), when it was made, its state, label, category, title, kind, target summary, schedule (`visible_at`, `local_time`, `expires_at`), and the three counts `send_status` answers. Pass the page's `end` as the next call's `after` to read on. A page holds at most 100 sends. `SendFilter::all()` lists everything, and `in_state` keeps one state.

Three things to know:

- **The list starts when you upgrade.** Sends made by an earlier version of the service have a report but are not listed.
- **A send leaves the list when its record is deleted.** That is the [send record's retention](privacy-and-retention.md#how-long-things-are-kept), 90 days by default and longer while any of its rows are stored.
- **A condition is shown as written.** If you put personal values into a condition, anyone who can see this list sees them.

Only your app's server can list sends.

**Name a send with a label.** An idempotency key has to be unique per send, so it makes a poor name for a campaign that goes out in several sends. Give those sends the same `label` instead:

```rust
let spec = SendSpec {
    category: "marketing".into(),
    title: "Spring sale".into(),
    label: Some("spring-sale".into()),
    ..SendSpec::default()
};
```

A label is 1 to 64 ASCII letters, digits, `-`, `_`, or `.`. The service stores it with the send and answers it in the send's report. It is never sent to a device. It is part of what the idempotency key stands for, so repeating a key with a different label is refused like any other changed send.

## When a send is refused

A refused send answers an error with an HTTP status and a message that names what was wrong. The common ones:

| Status | When                                                                                            | What to do                                                                                                  |
| ------ | ----------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| `400`  | a field is empty, too long, or malformed: the message names the field and the limit             | fix the input; nothing was written                                                                          |
| `403`  | the call did not come from your app's server                                                    | call with `call_as_app` from server code, never from a screen                                               |
| `404`  | `send_status`, `send_report`, or `cancel` named a key the service has no record of              | check the key; a record is deleted after its [retention](privacy-and-retention.md#how-long-things-are-kept) |
| `409`  | the key was already used for a different send, or `cancel` named a send that is already visible | use a new key for a new send; a visible send cannot be unsent                                               |

[Delivery and troubleshooting](troubleshooting.md#errors-the-service-answers) lists every error the service answers.
