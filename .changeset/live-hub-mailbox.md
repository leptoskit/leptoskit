---
"release": patch
---

# Keep pending live query results bounded

The live hub keeps one pending batch per connection instead of retaining every database round while a session is busy. Repeated changes retain the latest complete value or error for each subscription, including identity checks that must happen before delivery. Removing or replacing a subscription also removes its queued result. Live queries still deliver current state, not an event for every intermediate write.
