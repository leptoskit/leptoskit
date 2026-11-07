---
"release": patch
---

# Bound pending live action and stream replies

Live actions keep their admission slot until the session handles their reply, so a busy connection cannot accumulate unlimited completed actions. Stream producers wait for bounded output capacity even when a client renews credits early; ordered items and terminal errors are retained. An outgoing stream frame larger than 1 MiB ends that stream with a 413 error.

Stream flushing during reauthentication and server drain also respects the connection's stall timeout.

The generated stream implementations now await `StreamSink::fail`. Handwritten implementations using this hidden server helper must await it too.
