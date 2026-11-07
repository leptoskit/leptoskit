---
"release": patch
---

# Bound pending live action and stream replies

Live actions keep their admission slot until the session handles their reply, so a busy connection cannot accumulate unlimited completed actions. Stream producers wait for bounded output capacity even when a client renews credits early; ordered items and terminal errors are retained. An outgoing stream frame larger than 1 MiB ends that stream with a 413 error.

Stream flushing during reauthentication and server drain also respects the connection's stall timeout.

Existing generated and handwritten stream pumps keep using the synchronous `StreamSink::fail` helper. The session waits for the recorded terminal error before releasing the producer task.
