---
"leptoskit_canvist": minor
---

# Draw in the canvist editor, between the paragraphs

Some things are quicker to draw than to describe. With the new `ink` feature an editor holds drawings in the flow of its text, and on the web a mouse, a pen, or a finger draws on them.

```rust
let page = CanvistState::new(Extensions::paragraphs().ink());
page.set_pen(Pen::new().color(0xe1, 0x1d, 0x48).width(12.0));
page.insert_drawing()?;
```

- `Extensions::ink()` adds drawings to what an editor holds, and `CanvistState::insert_drawing()` puts a fresh sheet where the caret is.
- `Pen` is the colour and width of the next stroke, set with `set_pen`. A pen with no colour draws in the colour of the text.
- Pressing on a sheet draws instead of selecting text. Each stroke is one change and one step of undo. A finger that starts on a sheet does not scroll the page or raise the keyboard, and a read-only editor does not draw.
- `CanvistState::undo()` and `redo()` let an app's own buttons step through history, and say whether there was a step to take.
- `CanvistError::Missing` is what an editor made without an extension answers when asked for it.
- Drawings are saved with the text in JSON and Markdown, and a screen reader hears each one as a drawing.

`examples/sketchnote` is a notebook built on it: a page that opens with one sketch already drawn, colours, two nibs, and a count of words, sketches, and strokes.

Only Chrome has been tested, and drawing with a real pen or finger has not been tried on a device.
