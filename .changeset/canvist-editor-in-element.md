---
"leptoskit_canvist": minor
---

# The canvist editor mounts itself in its element

An app no longer attaches the editor by hand. `CanvistState::id()` is the id a `CanvistEditor` element takes, and asking for it on the web starts a watcher in the page: the editor appears in the element that names it when the element does, and leaves with it.

```rust
let note = CanvistState::new(Extensions::paragraphs());

CanvistEditor(note.id())
	.accessibility_label("Note")
	.placeholder("Write a note…")
	.class("min-h-40 w-full rounded-lg border p-3 text-base")
```

The element's settings reach the editor and follow it as they change: the placeholder, `read_only`, the accessible label (`aria-labelledby` too), `focused`, and the classes the editor reads its text style from, along with the app's choice of light or dark. An element that sets no placeholder and is not read-only leaves alone what the app set on the handle. `Focus` and `Blur` handlers on the element hear the editor gain and lose the keyboard. A server-rendered page gets its editor when it hydrates.

`web::attach` stays for pages that are not built with leptoskit, and a page that never asks for an id never starts the watcher.

Editor ids are `u32` now, which is what the element carries.
