---
"release": minor
---

# Rate-limit analytics events per sender

The analytics service limits how fast each `distinct_id` may send events: 100 a second, with bursts of 2,000, far above what a person's device sends. One stuck client or bot can no longer fill the queue for everyone else. Events over the limit are dropped, counted in the answer's `rate_limited`, and recorded as an ingestion warning. Events the app's server sends aren't limited. `IngestConfig::per_distinct_id` changes the limit, and `None` turns it off.
