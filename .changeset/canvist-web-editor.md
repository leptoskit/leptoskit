---
"leptoskit_canvist": minor
---

# A canvist text editor on the web, in Rust

`leptoskit_canvist` is a new, experimental crate that puts the [canvist](https://github.com/canvist/canvist) text editor in a web page. Its whole browser host is Rust compiled to Wasm; no JavaScript is written for it.

An app owns an editor through a `CanvistState` handle, which is `Copy` like a signal:

```rust
let note = CanvistState::new(Extensions::paragraphs());
note.load(Format::Markdown, "Hello\n\nWorld")?;
note.on_change(|change| { /* mark the note unsaved */ });
let attachment = leptoskit_canvist::web::attach(&container, note)?;
```

This first slice edits plain paragraphs: typing (with input methods), Enter and Backspace across paragraphs, undo and redo, keyboard and pointer selection, plain-text cut, copy and paste, read-only, and a placeholder. The editor takes its font, colours, and width from the element it is attached to.

It runs on the web only. On other targets the handle still loads and exports plain text, Markdown, and JSON, but nothing draws an editor yet. Formatting, lists, ink, and the `leptoskit` element that mounts the editor for you come in later changes. The crate is unpublished while its canvist dependencies are pinned to a git revision.
