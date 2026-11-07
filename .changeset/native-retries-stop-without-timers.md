---
"leptoskit_rpc": patch
---

# Retries stop when a host cannot schedule the wait

A keyed HTTP mutation and `call_with_retry` now surface the last network error when the host has no usable timer, instead of waiting forever on a backoff that can never finish. This keeps a native app's Save button from staying on "Saving…" when its host predates the guest timer plugin or rejects the timer.

A live subscription created before the socket opens is sent once: the handshake records which connection delivered it, so the command that woke the connection no longer sends a duplicate `Sub` frame.
