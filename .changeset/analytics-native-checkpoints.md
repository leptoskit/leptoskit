---
"release": patch
---

Keep native analytics queues intact when a save fails

Native analytics saves new queue files before atomically committing their checkpoint. Failed writes keep the previous queue and retry later, and concurrent flushes serialize their saves. Restoring into a smaller queue keeps each surviving event's context. Starting while opted out commits an empty checkpoint before opt-in can expose old files again.
