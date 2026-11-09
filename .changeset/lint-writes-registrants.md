---
"leptoskit_cli": patch
---

# `lk lint` passes on a fresh clone

`leptoskit lint` no longer fails on a fresh clone of an app with native platforms. The app-owned runners in `macos/`, `ios/`, `android/`, `windows/`, and `linux/` include a registrant under `.leptoskit/<platform>/`, which Git ignores and only native builds wrote, so clippy stopped on the missing file until you ran a native build. `lk lint` now writes any missing registrant for the platforms in `app.platforms` before clippy starts. Registrants that exist are left for the next native build to refresh, and a registrant that can't be written is a warning, with clippy naming the missing file as before.
