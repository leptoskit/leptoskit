---
"leptoskit_notifications": minor
---

Translated variants and delivery in each person's local time. `SendSpec` gains `variants`, a map from a BCP 47 language tag to a `Variant` that replaces the title, the body, the Apple subtitle, and the web action titles for people who read that language, and `local_time`, a `LocalTime { at, default_time_zone }` that makes each person's row visible, and pushed, when a wall-clock time such as `2026-12-01T09:00` arrives in their time zone. Every person still gets exactly one inbox row, and its push carries the same text.

Language and time zone resolve per person: the `locale` or `time_zone` attribute the app set wins, then the person's most recently seen live endpoint, then the send's default (its own text, and `default_time_zone`, which is UTC when left out). A locale matches the variant with its exact tag, then with its last subtag dropped, down to the language; nothing matching reads the send's own text. Time zones are IANA names looked up in bundled time zone data (`jiff`, behind the `service` feature only). A zone the service does not know on a person is skipped; an unknown `default_time_zone` is refused. A local time already past for a person arrives at once, a time a daylight-saving change skips arrives after the gap, and a repeated time arrives the first time.

A send carries at most 32 variants and still fits `MAX_SPEC_LEN`; a variant translates only text the send has, and a data message carries none. `local_time` and `visible_at` together are refused. `expires_at` must be after the local time has reached the last time zone (`at` plus twelve hours, as UTC), and the schedule limit is counted to that moment. Both fields are part of a send's idempotent intent and serialize to nothing when unset, so earlier sends answer their retries unchanged.

`cancel` now stops a local-time send for the people still waiting once others have seen it, and answers `0` when asked again; a send nobody is waiting for is still refused. Pushes for waiting rows are queued for their own instants, and a fanout enqueues one `wake_local` job per batch and distinct instant.

A new `endpoint_seen` migration on both engines adds the nullable `push_endpoints.seen_at`, set when an endpoint enrolls, changes its details, or renews (at most once an hour for an unchanged renewal). It adds an empty column and takes no measurable time.

**Source-breaking for struct literals:** `SendSpec` has two new fields and `PushEndpoint` one. `SendSpec` now implements `Default`, so `..SendSpec::default()` fills in what a send does not set.
