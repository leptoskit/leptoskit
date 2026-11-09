---
"leptoskit_android": patch
"leptoskit_bridge_ffi": patch
"leptoskit_cli": patch
---

An unknown or custom widget is an empty container on Android, in both the backend and the generated app. It used to stop the host, which closed the app. GTK already drew one as an empty view.
