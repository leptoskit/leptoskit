# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## leptoskit_canvist [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/leptoskit_canvist/v0.3.1) (2026-11-28)

### Features

#### The canvist editor mounts itself in its element

_Owner:_ Ifiok Jr. · _Introduced in:_ [234a7fe](https://github.com/leptoskit/leptoskit/commit/234a7fe9a5347e0b7e8afcbb7d16a67264e3c67a)

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

#### Draw in the canvist editor, between the paragraphs

_Owner:_ Ifiok Jr. · _Introduced in:_ [14fefe3](https://github.com/leptoskit/leptoskit/commit/14fefe30c73152aecbc8aecd3c878bb279a7e342)

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

#### A canvist text editor on the web, in Rust

_Owner:_ Ifiok Jr. · _Introduced in:_ [34f74e6](https://github.com/leptoskit/leptoskit/commit/34f74e6f0278a261b5ede82e1a94ed370c6f5a2a)

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
