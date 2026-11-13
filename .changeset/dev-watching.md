---
"leptoskit_cli": patch
---

`leptoskit dev` says it is watching as soon as the watcher is running, before the first build. That build can take minutes, and the message used to wait until the platforms had started, so an edit during it looked ignored.
