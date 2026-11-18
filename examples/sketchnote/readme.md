# sketchnote

Sketchnote, a notebook page where drawings sit in the flow of the text, and the worked example for the [canvist](https://github.com/canvist/canvist) editor.

Some things are quicker to draw than to describe: the line of a ridge, where a path leaves the trees, the spot a bird was hunting. The page is one editor that holds both. Paragraphs wrap as they do anywhere, and between them sit sketches you draw with a mouse, a pen, or a finger. The notebook opens on a page with one sketch already drawn. **Add a sketch** puts a fresh sheet where the caret is, the round buttons pick a colour, **Fine** and **Marker** pick a nib, and every stroke is one step of **Undo**. The line under the page counts its words, sketches, and strokes after every change.

The app owns the editor through a `CanvistState` handle from `leptoskit_canvist` and shows it with a `CanvistEditor` element. The document never passes through the screen, so nothing re-renders while you write or draw: the buttons call the handle, and the status line hears about changes and counts what the saved page holds.

```bash
leptoskit dev                      # in the browser
cargo test -p sketchnote_example   # the screen, the page it ships, and the buttons
```

The editor draws itself in the browser only for now. On the other platforms the same screen shows its title, pens, and count around an empty view where the page will be.

`fixtures/page.json` is the page the notebook opens on, written the way the editor saves one: the JSON `export(Format::Json)` gives, with one stroke to a line. `web/index.html` has the two lines that load the Wasm module, as in every app `leptoskit new` writes; everything the page does is the Rust in `src/lib.rs`.

`leptoskit_canvist` is not on crates.io yet, so an app outside this repository depends on it by git, with its `web` and `ink` features.
