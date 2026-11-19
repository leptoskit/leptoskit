---
"leptoskit_windows": minor
---

# Windows presents Modal and Popover in popup windows

On Windows a `Modal` is now a popup window owned by your app's window: it is as large as what the modal holds, the window behind takes no input until it closes, and Tab stays inside. A `Popover` is a popup window placed beside its anchor where `placement` says. Escape asks either one to dismiss, as do Alt+F4 for a modal and losing the keyboard for a popover, such as a press elsewhere in your window. Neither closes until your `open` signal says so. Focus moves to `initial_focus`, or else to the first control inside, and returns to where it was on close.

What differs from the web: Windows does not dim the window behind a modal, the popup is a plain rectangle, and a press on a popover's anchor does not ask to dismiss, so the control that opened it decides.

`WindowsBackend` gains `overlay_shown` and `overlay_window`. `WindowsWindow`'s message loops now route Tab and Escape to the popup a message belongs to. An app with its own message loop must do the same with `IsDialogMessageW`.
