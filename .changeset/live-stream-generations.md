---
"release": patch
---

# Isolate reused live stream IDs

Cancelling or replacing a stream now discards its buffered items and terminal frame, so they cannot reach a later stream with the same client ID. Streams retain their admission slot until their terminal frame is consumed. Handwritten producers that return or panic also send a terminal response; panic details stay on the server.

Consuming a terminal frame also cancels its producer, so a handwritten producer cannot keep running after its stream is retired.
