---
"release": minor
---

# Answer analytics batches once they're queued

By default the analytics service answers a batch once its events are stored, so every answer means they're kept. An app that prefers speed can set `LEPTOSKIT_ANALYTICS_ACK=queued` on the service: it answers as soon as the events are queued, and a crash loses at most what was queued. The queue's limits still apply, and a full queue still answers `503`.
