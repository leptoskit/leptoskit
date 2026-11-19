---
"leptoskit_notifications": minor
---

# Send a notification on a repeating schedule

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
