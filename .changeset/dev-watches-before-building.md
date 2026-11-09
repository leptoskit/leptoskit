---
"leptoskit_cli": patch
---

# `lk dev` keeps edits made while it starts

`leptoskit dev` now starts watching your project before its first build rather than after every platform has started. An edit saved while that first build ran, which takes minutes on a fresh checkout, used to go unseen: the server came up on the code from before the edit and stayed on it until you saved again. The edit now waits in the watcher's queue, and `dev` rebuilds and restarts the server as soon as the platforms are up. The `watching … for changes` line still prints once everything has started.
