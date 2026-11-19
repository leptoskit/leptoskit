---
"leptoskit_gtk": minor
---

# Linux presents Modal as a window and Popover as a popover

On Linux a `Modal` is now a modal window above your app's window: it has no title bar, is as large as what the modal holds, and keeps presses, keys and the screen reader to itself. A `Popover` is a GTK popover placed beside its anchor where `placement` says. Escape asks either one to dismiss, as do the window manager's close for a modal and a press outside a popover. Neither closes until your `open` signal says so. Focus moves to `initial_focus`, or else to the first control inside, and returns to where it was when a popover closes.

What differs from the web: the modal's own background is not drawn and GTK does not dim the window behind, the window manager chooses where the modal goes, and a press on a popover's anchor does not ask to dismiss, so the control that opened it decides.

`GtkBackend` gains `overlay_shown`, `overlay_window` and `overlay_popover`.
