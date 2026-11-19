---
"leptoskit_appkit": minor
"leptoskit_bridge_ffi": minor
"leptoskit_cli": patch
---

# macOS shows Modal as a sheet and Popover as NSPopover

On macOS a `Modal` is now a sheet on the app's window: it dims the window, keeps clicks and the keyboard to itself, and gives focus back when it closes. A `Popover` is an `NSPopover` pointing at its anchor. Escape asks to dismiss either one, and a press outside asks to dismiss a popover. `initial_focus` moves focus into the overlay when it opens. This holds for generated macOS apps (the Swift renderer) and for Rust hosts on `AppKitBackend`.

What differs from the web: a sheet is as large as what the modal holds and the modal's own background is not drawn; a sheet has no outside to press; and a popover is centred on its anchor, so `placement` picks above or below only.

`AppKitBackend` gains `overlay_shown`, `overlay_window` and `perform_dismiss`, and `open_window` shows overlays that were already open. Generated macOS projects ship one more Swift file, `MacOverlayView.swift`.
