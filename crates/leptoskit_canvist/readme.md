<h1 align="center">leptoskit_canvist</h1>

<p align="center">
  <a href="https://leptoskit.dev/"><img src="https://raw.githubusercontent.com/leptoskit/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

<p align="center">
  <strong>leptoskit_canvist</strong> puts the <a href="https://github.com/canvist/canvist">canvist</a> text editor in a leptoskit app. On the web its whole host is Rust compiled to Wasm.
</p>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#where-things-stand"><strong>Where things stand</strong></a> ·
  <a href="https://github.com/leptoskit/leptoskit"><strong>Repository</strong></a>
</p>

<br>

## Overview

A notes app, a comment box, or a document page needs an editor that behaves the same everywhere and that the app can load, watch, and save. canvist is a text editor that lays out and draws its own text on a canvas instead of handing it to `contenteditable`, so the same engine can run in a browser and, later, behind native views.

This crate hosts it. An app owns an editor through a `CanvistState` handle: it loads a document, hears about changes, and exports the document when it is time to save. On the web, the editor appears in the `CanvistEditor` element the app gives the handle's id to.

This first slice edits plain paragraphs on the web. It is experimental and unpublished.

## How it fits

- **The handle** (`CanvistState`, on every target) is `Copy`, like a signal. It loads and exports plain text, Markdown, and JSON, reports changes through `on_change`, and holds the read-only flag and the placeholder. On a server it converts and reads documents an editor saved.
- **The element** (`CanvistEditor(id)`, in leptoskit itself) only says where the editor shows. The document never travels through its props, because replacing a whole document on every keystroke breaks input methods, the selection, and undo. The element's placeholder, `read_only`, label, `focused`, and classes do reach the editor, and follow it as they change.
- **The web host** (`web`, behind the `web` feature, `wasm32` only) watches the page once an app has asked a handle for its `id()`. When an element that names an editor appears, it builds three things inside it: an opaque canvas that paints the visible part of the document, a transparent text field at the caret that takes the keyboard and input methods, and a hidden copy of the text for screen readers. There is no JavaScript to ship beside the Wasm: the browser APIs are called from Rust through `web-sys`.
- **canvist's types stay inside.** The engine crates are pinned to one git revision, and nothing of theirs appears in this crate's API, so a canvist release cannot break an app that only holds a handle.

The editor looks like the text around it. It reads its font, size, line height, colours, and width from the element it is attached to, and reads them again when the element is resized, when it gains focus, and when the system switches between light and dark.

## Use it

```rust
use leptoskit_canvist::CanvistState;
use leptoskit_canvist::Extensions;
use leptoskit_canvist::Format;

let note = CanvistState::new(Extensions::paragraphs());
note.load(Format::Markdown, "Hello\n\nWorld")?;
note.set_placeholder("Write a note…");
note.on_change(|change| {
	if change.doc_changed {
		// Mark the note as unsaved.
	}
});
```

Show it with leptoskit's `CanvistEditor` element. On the web, with this crate's `web` feature, the editor appears in the element and leaves with it, on a server-rendered page too once it hydrates:

```rust,ignore
CanvistEditor(note.id())
	.accessibility_label("Note")
	.class("min-h-40 w-full rounded-lg border p-3 text-base")
```

Call `note.dispose()` when the screen that made the editor goes away, on the server too: the handle is `Copy`, so nothing else can tell when to free the editor.

A page that is not built with leptoskit attaches the editor to an element itself, and keeps what comes back for as long as the editor is on screen:

```rust,ignore
let attachment = leptoskit_canvist::web::attach(&container, note)?;
```

Until canvist releases and this crate can be published, an app depends on it by git, beside `leptoskit`. The `leptoskit` feature `canvist` comes with that release.

## Where things stand

| What                                                                    | Web        | Everywhere else |
| ----------------------------------------------------------------------- | ---------- | --------------- |
| Load and export plain text, Markdown, and JSON; `on_change`             | Yes        | Yes             |
| Typing, Enter, Shift+Enter, Backspace and Delete across paragraphs      | Yes        | No editor yet   |
| Undo and redo                                                           | Yes        | No editor yet   |
| Arrow keys, Home and End, with Shift to select; select all              | Yes        | No editor yet   |
| Pointer selection: click, drag, Shift+click, double click, triple click | Yes        | No editor yet   |
| Cut, copy, and paste                                                    | Plain text | No editor yet   |
| Input methods (composition through the text field)                      | Yes        | No editor yet   |
| Read-only and a placeholder                                             | Yes        | No editor yet   |
| In a `CanvistEditor` element, with its settings; after hydration        | Yes        | An empty view   |
| Bold, italic, headings, lists, links, ink                               | Not yet    | Not yet         |
| Rich clipboard (HTML), drag and drop                                    | Not yet    | Not yet         |

**Screen readers.** The text field is announced as a multi-line text box with the element's `aria-label`, its read-only state, and its placeholder, and the hidden copy lets a screen reader read the document. A screen reader cannot yet follow the caret through the text or read a selection back the way it does in a `<textarea>`. Every canvas editor has this limit; the browser's `EditContext` API, which this slice does not use yet, narrows it.

**Size.** A page with one editor and nothing else (`examples/editor.rs`) is 586 KB of Wasm, 258 KB gzipped, built with the settings `bundle.optimize = "smallest"` uses and `wasm-opt -Oz`. The glue `wasm-bindgen` generates beside it is 8 KB gzipped. Showing the editor in a `CanvistEditor` element links the watcher that finds the element, which adds 3 KB gzipped; that figure leaves out leptoskit itself.

**Why it is unpublished.** canvist's released version predates the per-extension crates this depends on, so they are pinned to a git revision, and Cargo does not publish a crate with git dependencies. When canvist releases them, the pins become versions and this crate can be published.
