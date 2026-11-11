---
"leptoskit_bridge_ffi": patch
---

# A stopped Android app says why, readably

When an Android app's guest stopped, leptoskit replaced it with a plain `TextView` holding the error. That view took the theme's text color, which in a light system theme is dark, so on an app with a dark background the message was nearly invisible: Reapers looked like a black screen on a Solana Seeker. An app whose plugins failed to start didn't get even that: its activity closed at launch.

Both now show `NativeFailureView`: white text on near-black (17:1), centred and selectable, covering the whole app, the same in either theme. An app that cannot start shows it instead of closing.
