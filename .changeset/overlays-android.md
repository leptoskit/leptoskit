---
"leptoskit_android": minor
"leptoskit_bridge_ffi": minor
"leptoskit_cli": patch
---

# Android presents Modal as a dialog and Popover as a popup

On Android a `Modal` is now a dialog in a window of its own: it dims the screen, keeps touches, the keyboard and TalkBack to itself, and is as large as what the modal holds. A `Popover` is a popup window placed beside its anchor where `placement` says. The Back gesture, Escape and a touch outside ask to dismiss. A dialog stays until your `open` signal closes it. The Back gesture closes a popup window before your app hears of it, so a popover your app keeps open is shown again. `initial_focus` moves focus to the named element when the overlay opens. This holds for generated Android apps (the Kotlin renderer) and for Rust hosts on `AndroidBackend`.

What differs from the web: the modal's own background is not drawn, because the dialog dims the screen, and a dialog needs an Activity behind the view the tree is mounted in.

`WidgetHost` gains `overlayShown`, `overlayDialog` and `overlayPopup`. Generated Android projects ship one more Kotlin file, `NativeOverlayView.kt`.

This also restores two things the Android host needed to compile: an import in `WidgetHost.kt` and the `Dismiss` case in the probe.
