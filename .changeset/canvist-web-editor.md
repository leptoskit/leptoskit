---
"leptoskit_canvist": minor
---

# The canvist text editor on the web

`leptoskit_canvist` hosts the canvist text editor in the browser as pure Rust: `web::attach` builds an opaque canvas and a hidden textarea inside a container, `CanvistState` loads and exports Text, Markdown, and JSON, and typing, Enter and Backspace across paragraphs, undo and redo, pointer selection, plain-text clipboard, IME composition, read-only, placeholder, and theme from computed style all work with dirty-rectangle painting on a `requestAnimationFrame` loop. Canvist's engine crates are pinned to one git revision and its types stay out of the public API, so the crate is unpublished for now; the readme documents the screen-reader limits of a canvas editor.
