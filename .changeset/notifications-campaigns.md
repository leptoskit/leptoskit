---
"leptoskit_notifications": minor
---

# Save a campaign as a draft and launch it once

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
