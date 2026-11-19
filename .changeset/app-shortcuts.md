---
"leptoskit_native": minor
"leptoskit": minor
---

# Keyboard shortcuts for the whole app on the web

`use_shortcut` runs a function when the person presses a key, wherever focus is, even when nothing has focus. It suits ⌘K to search or a single letter that opens a drawer, which `on_key` can't express because it only hears keys pressed inside its element.

```rust
let search = FocusRequest::new();
use_shortcut(Shortcut::primary("k"), move || search.request());
use_shortcut(Shortcut::key("w").unless_typing(), move || toggle_drawer());
```

`Shortcut::primary` holds ⌘ on Apple devices and Ctrl elsewhere. `.shift()` and `.alt()` add modifiers, and `.unless_typing()` leaves the key alone in a text field. A shortcut stops when its component goes, the newest of two matching shortcuts wins, and a key an element's `on_key` handled never reaches one.

Shortcuts work on the web today. Native hosts and the terminal don't deliver keys to them yet, so there a shortcut does nothing for now.
