# Campaigns

A send from code is finished the moment it is written. A notification a person composes on a screen is not: it is drafted, tried out, corrected by someone else, scheduled, and only then sent. A campaign is the service's record of that work. This page is for you if you are building a compose screen for your own staff. leptoskit's admin does not have that screen yet, so today a campaign is something your own server and your own interface drive through the calls below. Like every send, a launched campaign reaches whatever platforms its audience uses.

A campaign is one send. For a message that goes out again and again on a schedule, use a [recurrence](sending.md#a-send-that-repeats) instead.

## Write a campaign before you send it

`send` is one call from your code: the message and the audience go in and the send is made. A person writing a notification on a screen doesn't work that way. They write half of it and go to lunch. They try it on their own phone. Someone else fixes the wording. They pick a time. Then they press send, sometimes twice. A campaign is where that work lives until it becomes a send:

```rust
use leptoskit_notifications::campaign::{
    CampaignDraft, CreateCampaign, LaunchCampaign, TestCampaign, UpdateCampaign,
};

// Save a draft. Only the name is needed.
let saved = call_as_app(CreateCampaign {
    draft: CampaignDraft { name: "Spring sale".into(), ..CampaignDraft::default() },
    by: admin_name.clone(),
})
.await?;

// Fill it in. The revision says which version this change was made from.
let saved = call_as_app(UpdateCampaign {
    id: saved.id,
    revision: saved.revision,
    draft: CampaignDraft {
        name: "Spring sale".into(),
        spec: SendSpec {
            category: "offers".into(),
            title: "Half price".into(),
            body: "Today only.".into(),
            ..SendSpec::default()
        },
        target: Some(SendTarget::Condition("user.plan == 'pro'".into())),
        note: Some("Check the wording with legal.".into()),
    },
    by: admin_name.clone(),
})
.await?;

// Try it on one person's devices. No inbox row is written.
call_as_app(TestCampaign { id: saved.id, person: my_account, endpoint: None, by: admin_name.clone() })
    .await?;

// Send it.
let launched = call_as_app(LaunchCampaign {
    id: saved.id,
    revision: saved.revision,
    by: admin_name,
    broadcast: false,
})
.await?;
```

Every call takes `by`, the name of the person acting. Your server fills it in from its own session; never take it from a browser. The service stores it and answers it back, and does not check it.

What a campaign does that a bare send doesn't:

- **A draft can be unfinished.** A draft with no title, no audience, or half a condition saves. Only its size is checked. Everything `send` checks is checked when you launch.
- **Two people can't overwrite each other.** Each save names the `revision` it started from. If someone else saved in between, the save is refused with a conflict and the draft is left as it was. Read it again and redo the change.
- **A launch happens once.** Launching the same revision twice makes one send. The second call answers the first send, with `send.duplicate` set to `true`. A double press or a retry after a timeout is safe.
- **A stale schedule is refused.** `send` treats a time that has passed as "now". A draft scheduled for Tuesday morning and launched on Thursday would go out at once, so `launch_campaign` refuses a `visible_at` that has passed. Pick a new time, or clear the schedule to send now.
- **Sending to everyone has to be said out loud.** A draft whose target is `SendTarget::Everyone` is refused unless you pass `broadcast: true`. The service cannot know who your admins are. The flag is your server saying it has checked that this person may send to everyone, and the launch's event records it.
- **There is a record of who did what.** `campaign_events` lists a campaign's events, oldest first: created, tested, launched, cancelled, archived, each with who and when. Saving a draft is not an event; the campaign's own `updated_by` says who saved it last.

The other calls:

| Call                 | What it does                                                                                                                                                                                                |
| -------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `campaign`           | reads one campaign, whole                                                                                                                                                                                   |
| `campaigns`          | lists campaigns newest first, 50 to a page, as short entries; `CampaignFilter` keeps one state                                                                                                              |
| `duplicate_campaign` | saves a copy as a new draft: same message, audience, and note, and **no schedule**                                                                                                                          |
| `duplicate_send`     | saves a new draft from any send that was made, yours or your server's. A recipients send's copy has no audience, because the service does not keep the list of people a send named                          |
| `cancel_campaign`    | cancels a launched campaign's send by [the rules of `cancel`](sending.md#cancel-a-scheduled-send): before it is due it stops, once people have seen it it is refused                                        |
| `archive_campaign`   | puts a campaign away. It can still be read and copied. Archiving cancels nothing: cancel a scheduled campaign first if that is what you mean                                                                |
| `test_campaign`      | pushes the draft to one person's devices now, like [`send_test`](troubleshooting.md#test-on-a-device), ignoring the schedule. It writes no inbox row, and its event says how the push ended, not who got it |

A campaign is in one of four states: `draft`, `launched`, `cancelled`, or `archived`. Whether a launched campaign is still scheduled, sending, or sent is the state of its send: the campaign's `send_key` is the key to pass to [`send_report`](sending.md#see-what-a-send-came-to) or `send_status`. The send is an ordinary send and appears in [`list_sends`](sending.md#browse-the-sends-you-made).

Limits, each refused by name: a name of 1 to 120 bytes, a note of at most 1,000, `by` of 1 to 128, a message and audience no larger than a send's, 1,000 drafts at a time, and 100 recorded tests per campaign.

Campaigns are not deleted by [retention](privacy-and-retention.md#how-long-things-are-kept). A draft is someone's saved work, and the service will not remove it on a timer. If a draft lists recipients, those ids stay in it until you change or archive it, and [`forget_person`](audiences.md#erase-a-person) does not edit drafts: remove a person you have erased from any draft that names them.

Only your app's server can call any of these.
