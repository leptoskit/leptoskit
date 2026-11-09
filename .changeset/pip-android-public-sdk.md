---
"leptoskit_plugin_picture_in_picture": patch
---

# Android picture-in-picture builds against the public SDK

Android PiP no longer calls `ActivityInfo.supportsPictureInPicture()`, which is `@hide` and missing from the public SDK stubs (so the generated app failed to compile). Capability checks use the system PiP feature plus an activity context; hosts that omit `android:supportsPictureInPicture` still fail closed when entering PiP. The PiP-mode listener imports `android.app.PictureInPictureModeChangedInfo` instead of the non-existent `android.util` type.
