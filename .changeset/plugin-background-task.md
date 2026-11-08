---
"leptoskit_plugin_background_task": minor
"leptoskit": patch
---

Add `leptoskit_plugin_background_task` for named OS background-task register/unregister/status/complete (`BGTaskScheduler` / `WorkManager`), distinct from `leptoskit_plugin_background` AppService keep-alive. Guaranteed cadence and suspended Wasm execution remain later scope.
