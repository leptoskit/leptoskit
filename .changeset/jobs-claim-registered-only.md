---
"release": patch
---

# Job workers leave other builds' jobs alone

Workers now claim only jobs their own build registers with `#[rpc::job]`. Before, a worker took any due job and failed one it didn't know, using up a retry, so during a rolling deploy the old and new builds sharing a database could take and fail each other's jobs. A job no running build registers now stays pending until one does, instead of failing.
