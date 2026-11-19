---
"leptoskit_notifications": minor
---

# Devices change hands and forgotten people are erased

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
