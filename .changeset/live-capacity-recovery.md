---
"release": patch
---

# Live updates resume when a slow reader catches up

A live connection retains its unfinished transition and resumes it when transport capacity becomes available, even without another database write. Later changes coalesce into one current snapshot. Large transitions continue from their remaining chunks, and removed or replaced subscriptions do not receive unsent results from their earlier registration.
