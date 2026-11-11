---
leptoskit_android: patch
leptoskit_log: patch
leptoskit_native: patch
leptoskit_game: patch
leptoskit_bridge_ffi: patch
---

Keep Android builds warning-free: omit the unused stderr fallback on Logcat, simplify pointer decisions, and account for Clippy's Android TLS false positive while retaining const initialization.
