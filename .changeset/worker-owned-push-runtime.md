---
"leptoskit_rpc": minor
"leptoskit_notifications": patch
---

Jobs can carry process-local typed runtime state through `Jobs::state` and read it with `JobCx::state`. This state stays separate from the durable job owner's authorization context and is never serialized.

Notifications routes and workers now share an explicit push runtime. Queued push arguments contain only send and round IDs, keeping database URLs and credentials out of job metadata and allowing pending work to recover with a fresh runtime after restart.
