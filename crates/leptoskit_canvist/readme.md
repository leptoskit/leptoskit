<h1 align="center">leptoskit_canvist</h1>

<p align="center">
  <strong>leptoskit_canvist</strong> hosts the canvist text editor in the browser: a canvas-based paragraph editor written entirely in Rust, with no hand-written JavaScript.
</p>

## Overview

`leptoskit_canvist` attaches a canvist editor to a container element with `web::attach`. It builds an opaque `<canvas>` for the document and a hidden `<textarea>` for input, and paints only what changed each frame. Text is measured with the browser's own `measureText`, through a Rust `CanvasMeasurer`, so wrapping matches what the browser would lay out.

Apps hold a `CanvistState` — a `Copy` handle to an editor — and load, export, and listen for changes through it. Canvist's own types stay out of the public API: documents cross as `Format::Text`, `Format::Markdown`, `Format::Json`, or `Format::Html`.

```rust
use leptoskit_canvist::{CanvistState, Extensions, Format};

let note = CanvistState::new(Extensions::paragraphs());
note.load(Format::Markdown, "# Hello");
```

`web::attach` and the rest of the host live behind the `web` feature, which is only available for the `wasm32` target.

## What it covers

- Typing, Enter splitting a paragraph, Backspace and Delete joining them;
- undo and redo with the platform's shortcuts;
- arrow keys, Home and End, with Shift for selection;
- click, drag, double-click word, and triple-click paragraph selection;
- plain-text copy, cut, and paste;
- IME composition through the textarea's composition events;
- read-only mode and a placeholder;
- a theme read from the container's computed style, repainted when the colour scheme changes;
- `role="textbox"`, `aria-multiline`, the container's label, and `aria-placeholder` on the textarea.

## Screen-reader limits

A canvas editor cannot give a screen reader the per-line, per-word structure that a `contenteditable` field can: the visible text is pixels, not DOM. The textarea carries the field's role, label, and placeholder, and screen readers announce it, but navigating the document's content by line or word, or reading back a selection, is weaker than in a DOM-based editor. A semantic mirror of the document is planned work, not part of this crate yet.

## Why this crate cannot be published

The canvist engine crates (`canvist_headless`, `canvist_layout`, and the per-extension crates) are pinned to one git revision of canvist's `main`, because canvist's published `0.2.1` predates the per-extension split its `main` has. Cargo refuses to publish a crate with git dependencies, so this crate is `publish = false`. Once canvist releases the extension crates, the pins can move to crates.io versions and the crate can be published.
