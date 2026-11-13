---
leptoskit_windows: patch
---

# Wait for composition before native pixel verification

Native Windows widget fixtures disable transitions on their own window and wait for queued composition before reading screen pixels. Exact color tolerances and transparency assertions remain unchanged; production windows and desktop settings are unaffected.
