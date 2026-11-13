---
"release": patch
---

# Stop replication capture when returning to triggers

When a migration switches live capture from replication back to triggers, the worker now checks that its publication still exists before reading the slot. It leaves capture mode and removes the unused slot even on an idle database, so that slot stops retaining write-ahead log. This also works on Postgres 18, where an empty decoding read can succeed after the publication was removed.
