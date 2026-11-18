---
"leptoskit_uikit": minor
"leptoskit_bridge_ffi": minor
"leptoskit_cli": patch
---

# iOS presents Modal as a sheet and Popover as a popover

On iOS a `Modal` is now a form sheet: it dims the screen, keeps touches and VoiceOver to itself, and gives focus back when it closes. A `Popover` is a system popover pointing at its anchor, on an iPhone as well as an iPad. A swipe down on the sheet, a press outside the popover, Escape on a hardware keyboard and the VoiceOver escape gesture all ask to dismiss, and the overlay stays until your `open` signal closes it. `initial_focus` raises the keyboard for a field and moves the VoiceOver cursor for anything else. This holds for generated iOS apps (the Swift renderer) and for Rust hosts on `UIKitBackend`.

What differs from the web: a sheet is as large as what the modal holds on an iPad and as wide as the screen on an iPhone, and the modal's own background is not drawn; and a popover is centred on its anchor, so `placement` picks above or below only.

`UIKitBackend` gains `overlay_shown`, `overlay_controller`, `perform_dismiss` and `present_overlays`. A controller that mounts the tree itself calls `present_overlays` from `viewDidAppear:`; `UIKitHost` does so already. Generated iOS projects ship one more Swift file, `MobileOverlayView.swift`.
