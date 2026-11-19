---
"leptoskit_native": minor
"leptoskit_tui": minor
"leptoskit_wasm": minor
"leptoskit_bridge_ffi": minor
"leptoskit_watch": patch
"leptoskit_lint": patch
"leptoskit_docs": patch
"leptoskit": minor
---

# Modal and Popover show content above the screen

`Modal(open)` blocks the screen until it is answered, and `Popover(anchor, open)` hangs from the element with that `id`. Both stay in the tree while closed. `.on_dismiss(...)` hears Escape and a press outside as a request to close, and the overlay closes when the next build says so. `.initial_focus(id)` names what takes focus on opening, and `.placement(Placement::BelowEnd)` picks a popover's side. In `view!` they are `<Modal open=…>` and `<Popover anchor=… open=…>`.

On the web a `Modal` is a `<dialog>` opened with `showModal()` and a `Popover` uses the `popover` attribute, so the browser supplies the top layer, the inert page, the focus trap and focus restore. The terminal draws overlays last, keeps Tab inside an open modal, and gives focus back on close.

macOS, iOS, Android, Linux and Windows draw an overlay as a layer in the right place for now. Nothing asks to dismiss it there, focus is not moved or kept inside, and a parent that clips or scrolls cuts it off.

For backend authors: `NativeKind` gains `Modal` and `Popover`, `NativeEvent` gains `Dismiss`, and `NativeProps` gains `overlay`. A backend that returns `true` from the new `NativeBackend::presents_overlays` is told the real kinds and given open overlays' frames in viewport coordinates. Every other backend is given a `View` with a frame relative to its parent. The guest event ABI gains kind 9 (dismiss), and the bridge event codes gain 10.
