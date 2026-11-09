---
"release": patch
---

# Recover releases created by the original static worker

Interrupted static releases created before executable support can now recover after a CLI upgrade. Recovery checks their exact installed assets, Caddy and static configuration against the pinned historical host, without requiring metadata that the original worker never recorded. Executable journals retain their derived-state checks, and changed inputs fail before recovery changes runtime state.
