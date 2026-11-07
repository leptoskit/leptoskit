---
"leptoskit_rpc": patch
---

# Stop live queries when their screen goes away

Live query hooks now unsubscribe when their reactive owner is cleaned up or their arguments change. Buffered updates are cancelled too, so a removed screen cannot repopulate the query cache after an account change.
