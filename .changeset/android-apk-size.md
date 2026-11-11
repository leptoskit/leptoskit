---
"leptoskit_cli": patch
---

`leptoskit native build` for Android reports the APK size from the app's own `android/` project. It was still looking under `hosts/android`, so a build that produced an APK said there was nothing to measure.
