# A canvist rich-text editor in leptoskit: design investigation

A read-only investigation into shipping a rich-text editor component in leptoskit, powered by canvist's Rust crates. It also covers how the editor relates to the plain `TextArea` that is being added separately (G12 in `js-to-wasm-audit.md:428-459`).

- **leptoskit:** branch `feat/public-demo` at `5c4b52dda`.
- **canvist:** `main` at `ee11cba6` (2026-11-10), cloned to a local clone of canvist.
- **Date:** 2026-11-18. Nothing was edited, built, or published.

Citations of the form `crates/...:N` are paths in the leptoskit repository. Citations of the form `canvist:...:N` are paths in the canvist clone.

---

## Key findings

1. **canvist's Rust crates already contain the whole engine, but every _host_ is in another language.** `canvist_web` is a Rust `WebEditor` that expects JavaScript to own "sizing, scrolling, the frame loop, input, and fonts" (`canvist:crates/canvist_web/src/lib.rs:6-11`). It measures text through a JavaScript `HostMeasurer` interface (`canvist:crates/canvist_web/src/measure.rs:19-72`). The DOM host is about 3,900 lines of TypeScript in `@canvist/web` (`canvist:packages/canvist-web/src/editor.ts`, `input.ts`, `measure.ts`, `fonts.ts`, `mirror.ts`, `placeholder.ts`, `edit-context.ts`). The iOS and Android hosts are Swift and Kotlin views over UniFFI (`canvist:crates/canvist_ffi/src/lib.rs:6-26`). Under the owner's no-hand-written-JS rule, the browser host has to be rewritten in Rust. That rewrite is the single biggest piece of web work.
2. **leptoskit already has the right pattern for this.** It is the host-owned surface: `Scene3d`, `Video`, `Camera`, `Map`, and `LivePhoto` are `NativeKind`s whose element carries only an id (`crates/leptoskit_native/src/props.rs:57-71,321-341`). An optional crate then draws into it, and a Wasm guest drives it through a plugin (`docs/agents/3d.md:9,22`; `crates/leptoskit_3d/src/client.rs:1-6`). A `RichText` kind built the same way keeps canvist out of core entirely.
3. **canvist cannot usefully run inside the native Wasm guest.** On native platforms the editor should live in the host process, as 3D scenes do. The guest runs in an interpreter on its own thread (`docs/agents/guest-thread.md:1-24`), with fuel limits (`crates/leptoskit_wasm/src/lib.rs:224,231`) and asynchronous plugin calls only. `UITextInput`, `NSTextInputClient`, and Android's `InputConnection` need synchronous answers on the UI thread about text and geometry, and canvist's measurer is synchronous (`canvist:crates/canvist_layout/src/measure.rs:86-92`). The guest holds a handle and receives coalesced change events.
4. **leptoskit's own drawing surfaces cannot draw the editor.** `Vector` and `leptoskit_canvas` have no text primitive (`crates/leptoskit_canvas/Cargo.toml:13`). `leptoskit_game` draws HUD text with a 5×7 bitmap font (`crates/leptoskit_game/src/font.rs:1`) and opens no window (`docs/src/readme.md:112`). leptoskit's `TextMeasurer` returns a size for a whole string (`crates/leptoskit_native/src/props.rs:1192-1202`), but canvist needs an advance for every character, measured in batches. The pixels have to come from canvist's display list, painted by Canvas2D, CoreGraphics, Android `Canvas`, or `vello_cpu`.
5. **The canvist APIs leptoskit needs are mostly in crates that aren't published.** `canvist_web`, `canvist_ffi`, `canvist_canvas_wasm`, and `canvist_headless_wasm` are `publish = false` (`canvist:crates/canvist_web/Cargo.toml:10`, `canvist_ffi/Cargo.toml:12`), and canvist intends that (`canvist:docs/internal/first-release.md:46`). crates.io has `0.2.1` of 12 engine crates (`cargo search canvist`, 2026-11-18). But `main` has since split every extension into its own crate (`canvist:.changeset/extension-crates.md`), and those extension crates exist on crates.io only as `0.0.0` placeholders. A published leptoskit crate can depend on none of this until canvist releases again.
6. **Native input is the platform view's job, so leptoskit's missing native `on_key` does not block typing.** The native backends deliver no `on_key` and ignore `focused(true)` (`docs/src/readme.md:137`). They do emit `Focus` and `Blur` for text fields (`crates/leptoskit_appkit/src/views.rs:464-469`, `crates/leptoskit_uikit/src/events.rs:274-281`, `crates/leptoskit_android/src/backend.rs:468-469`, `crates/leptoskit_gtk/src/backend.rs:1386-1389`). A host-owned editor view takes keys, IME, selection, and the clipboard straight from the platform, as canvist's `CanvistView` does. What is missing is focus requests, plus an editor view on each platform.
7. **Native support is uneven, and canvist covers only part of it.** canvist has working views for iOS (UIKit, iOS 17, `canvist:Package.swift:17`) and Android (minSdk 24). It has none for AppKit: its desktop editor is a winit window painted with vello (`canvist:crates/canvist_desktop/Cargo.toml:23`), although its core runs without winit (`canvist:crates/canvist_desktop/src/lib.rs:23`). Windows has two leptoskit-side blockers: the generated Windows runner registers no runtime plugins (`docs/src/readme.md:118`), and GTK 4.6 has no way to expose custom-drawn text to screen readers.
8. **Size: about 350 KB gzipped on the web, which lazy loading keeps off pages without an editor.** canvist's StarterKit editor ships 348 KB of gzipped wasm (`canvist:docs/src/guides/performance.md:62,68`). The model alone is 38 KB, and layout with both renderers is 133 KB (`canvist:docs/internal/architecture.md:116-121`). Collaboration adds 140 KB, 190 KB, or 250 KB gzipped, depending on which canvist doc you read (`canvist:docs/src/guides/collaboration.md:31,49`; `canvist:crates/canvist_web/Cargo.toml:33`). leptoskit already splits `#[lazy]` code and lazy routes into chunks by default (`docs/src/guide/03-configuration.md:115`). On native, the guest bundle doesn't grow, so code push payloads stay the same.
9. **The dependency direction is leptoskit → canvist, with no cycle.** The integration needs a core kind, renderer hosts in `leptoskit_bridge_ffi`, and CLI feature detection (`crates/leptoskit_cli/src/native/registry.rs:329-334,548-553,600-614`). Only leptoskit can own those. canvist should not depend on leptoskit. Both use Rust 1.95, edition 2024, wasm-bindgen 0.2.129, web-sys 0.3.106, the Unlicense, and wgpu 30.

## Recommendations in brief

- **Name the pieces:** the component `RichTextEditor`, the state handle `RichTextState`, the optional crate `leptoskit_rich_text`, the `leptoskit` feature `rich-text`, and the module `leptoskit::rich_text`. Core gets `NativeKind::RichText` (wire name `rich_text`), the plugin is `leptoskit.rich_text`, and the bridge feature is `rich_text`.
- **Core gets only an id-carrying kind,** exactly like `Scene3d`. Everything that touches canvist lives in `leptoskit_rich_text`.
- **The editor's state belongs to a handle, not to props.** The app owns a `RichTextState`, whose commands and change events go to the host without re-rendering the element. A fully controlled `value` round-trip per keystroke would break IME, selection, and undo. The document travels as ProseMirror/TipTap JSON, Markdown, HTML, or plain text. canvist's types stay out of leptoskit's public API.
- **Ask canvist for three things:**
  1. A publishable `canvist_web` with a Rust-facing measurer, plus a Rust DOM host behind a feature.
  2. A host-neutral `canvist_session` crate extracted from `canvist_ffi`.
  3. A Cargo feature in place of `--cfg canvist_omit_docs`.
- **Start with the smallest slice:** web only, plain paragraphs, the textarea input path, and Text/Markdown/JSON export. The plan is in section 6. Ship `TextArea` (G12) first. It unblocks the demo, and the editor reuses its `read_only` and `placeholder` plumbing.
- **Reconsider whether this is generic.** A rich-text editor is table stakes. canvist's distinctive features are where the twist is: ink, kinetic text, and dictation that fades from provisional into ink (`canvist:crates/canvist_layout/Cargo.toml` features; `display/mod.rs` `settling`). They are a better reason to adopt it than "bold and headings everywhere". Pick one of them for the first showcase rather than building collaboration up front.

---

## 1. Integration seam

### 1.1 What canvist provides

| Crate                                                    | What it is                                                                                                                                                                                                                                      | On crates.io                                                                                           | Use in leptoskit                                                                                                                                    |
| -------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------- |
| `canvist_model`                                          | Block document model, steps, transactions                                                                                                                                                                                                       | 0.2.1                                                                                                  | Indirectly                                                                                                                                          |
| `canvist_core`                                           | Editor state, commands, keymaps, input rules, history                                                                                                                                                                                           | 0.2.1                                                                                                  | Indirectly. `Command` is an opaque closure (`canvist:crates/canvist_core/src/context.rs:31`), so it cannot cross the guest boundary.                |
| `canvist_extensions` + `canvist_extension_*`             | The StarterKit and extras, one Cargo feature each                                                                                                                                                                                               | 0.2.1 as one crate. `main` splits it, and the split crates are only `0.0.0` placeholders on crates.io. | Yes. leptoskit features select the extension set.                                                                                                   |
| `canvist_headless`                                       | `Engine` and `Doc`: load and export JSON, HTML, Markdown, and text (`canvist:crates/canvist_headless/src/format.rs:19-29`), named commands, and `Doc::edit` for hosts (`doc.rs:387-401`)                                                        | 0.2.1                                                                                                  | **Yes, on every platform and on the server**                                                                                                        |
| `canvist_layout`                                         | Incremental layout, hit testing, `DisplayList` (`canvist:crates/canvist_layout/src/display/mod.rs:111-171`), `to_html` for SSR (`render/html.rs:99`), the `TextMeasurer` trait (`measure.rs:86-92`)                                             | 0.2.1                                                                                                  | **Yes.** On the web use `default-features = false` with `editor` and `json`, as `canvist_web` does (`canvist:crates/canvist_web/Cargo.toml:17-20`). |
| `canvist_paint`                                          | The `Painter` trait (`canvist:crates/canvist_paint/src/painter.rs:91-112`), vello (GPU), `vello_cpu`, SVG                                                                                                                                       | 0.2.1                                                                                                  | GTK and Windows, CPU raster via the `cpu` feature                                                                                                   |
| `canvist_web`                                            | Rust `WebEditor`: Canvas2D painter, dirty rectangles, input entry points (`canvist:crates/canvist_web/src/input.rs:65-302`). Measures through a JS `HostMeasurer`.                                                                              | **No** (`publish = false`)                                                                             | Yes, once it is publishable and has a Rust measurer. Otherwise leptoskit has to re-implement it.                                                    |
| `canvist_desktop`                                        | `DesktopEditor` with `InputEvent` (`canvist:crates/canvist_desktop/src/input.rs:16-93`), parley and fontique system fonts, `render_rgba8` (`paint.rs:69`), and an AccessKit tree (`accessibility.rs:153`). winit is optional (`Cargo.toml:23`). | 0.2.1                                                                                                  | GTK and Windows hosts, with `default-features = false`                                                                                              |
| `canvist_ffi`                                            | UniFFI editor for iOS and Android: UTF-16 text index (`canvist:crates/canvist_ffi/src/text.rs`), platform measurer, display frames, `Command { name, args }` (`types.rs:193-198`)                                                               | **No**                                                                                                 | Its host-neutral half, if it is extracted into a published crate                                                                                    |
| `canvist_collab`                                         | yrs binding, y-sync, awareness, shared undo                                                                                                                                                                                                     | 0.2.1                                                                                                  | The opt-in `collab` feature                                                                                                                         |
| `canvist_canvas`, `canvist_scene`, `canvist_canvas_wasm` | The design canvas (vello, wgpu, parley, 2.0 MB gzipped in the browser: `canvist:docs/internal/canvas.md:237`)                                                                                                                                   | Partly                                                                                                 | **No.** This is not the text editor.                                                                                                                |

The JS packages (`canvist-react`, `canvist-vue`, `canvist-react-native`, `canvist-lynx`, `canvist-ssr`, `canvist-web`) are all off-limits. The Swift and Kotlin views in `canvist:platforms/apple` and `canvist:platforms/android` are fair game, because leptoskit already ships Swift and Kotlin renderers (`crates/leptoskit_bridge_ffi/readme.md`, "Overview").

### 1.2 How leptoskit hosts custom-drawn content today

| Mechanism                                                                       | How it works                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      | Fit for a rich-text editor                                                                                                                            |
| ------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Host-owned surface kinds** (`Scene3d`, `Video`, `Camera`, `Map`, `LivePhoto`) | The element carries an id (`crates/leptoskit_native/src/element.rs:640-649`). The web renderer emits a tagged element (`crates/leptoskit_native/src/web/mod.rs:769-814`). An optional crate attaches to it, as `leptoskit_3d` does through a `MutationObserver` (`crates/leptoskit_3d/src/web.rs:1-8,150-235`). Generated apps draw in Swift and Kotlin (`crates/leptoskit_bridge_ffi/mobile/NativeScene3d.swift:18-24`, `src/scene3d.rs:1-12`). The backend crates hold an empty view (`crates/leptoskit_appkit/src/backend.rs:325-336`). The CLI turns on the bridge feature when the app depends on the crate (`crates/leptoskit_cli/src/native/registry.rs:329-334,600-614`). | **This is the model to follow.**                                                                                                                      |
| `Rive`                                                                          | A data prop in core (`crates/leptoskit_native/src/props.rs:320`). CPU frames are presented through `NativeBackend::present_rive` (`crates/leptoskit_native/src/backend.rs:136-147`) into an `NSImage` (`crates/leptoskit_appkit/src/rive.rs`).                                                                                                                                                                                                                                                                                                                                                                                                                                    | The bitmap presentation path can be reused for GTK and Windows. But it special-cases Rive in the backend trait, so generalise it rather than copy it. |
| `NativeKind::Custom(name)`                                                      | A plain view on native backends. The web renderer panics on it (`crates/leptoskit_native/src/web/mod.rs:965-969`). It has no guest ABI code (`crates/leptoskit_native/src/wire.rs:44-49`, `guest/tree.rs:662-665`). The docs say a custom widget "is an empty container until a host registers one" (`docs/src/readme.md:101`), but no registry exists.                                                                                                                                                                                                                                                                                                                           | Not usable today. Making it real is the long-term generalisation (section 3.6).                                                                       |
| `Vector`, `leptoskit_canvas`, `leptoskit_game`                                  | Paths and shapes with no text                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     | Ruled out (finding 4)                                                                                                                                 |

### 1.3 Recommended shape

```
          app code (web page, or Wasm guest in the interpreter)
               │  RichTextState: load / run / export / on_change
               ▼
┌───────────── leptoskit_rich_text (optional crate) ─────────────┐
│ web: in-page host        guest: client over leptoskit.rich_text │
└───────┬──────────────────────────────┬─────────────────────────┘
        │ attaches to                   │ plugin calls (async, coalesced events)
        ▼                               ▼
<div data-lk-rich-text=id>        native host (bridge_ffi `rich_text`):
emitted by core's web renderer    canvist engine per editor id, behind a Hub
                                  + a platform editor view per RichText node
```

- **Core (`leptoskit_native`):**
  - `NativeKind::RichText`, appended to the wire table after `TextArea`. Codes are append-only (`crates/leptoskit_native/src/wire.rs:13-17`).
  - `NativeProps::rich_text: Option<u32>`, and `Element::rich_text(id)`.
  - A web container element.
  - An empty view in every backend and renderer that has no host yet.

  Nothing in core depends on canvist. `leptoskit_native` already depends on `leptoskit_3d` only for its id and protocol types, with default features off (`crates/leptoskit_native/Cargo.toml:19`, root `Cargo.toml:231`). The rich-text kind doesn't even need that.
- **`leptoskit_rich_text`:**
  - `web`: the in-page host.
  - `guest`: the plugin client.
  - `host`: the engine and the `leptoskit.rich_text` plugin, which `leptoskit_bridge_ffi` and the GTK and Windows runners link.
  - `ssr` / `server`: headless conversion and the `to_html` first paint.
  - `collab`.
  - Extension features mirroring `canvist_extensions`.
- **`leptoskit_bridge_ffi`:** a `rich_text` feature, like `scene3d` (`crates/leptoskit_bridge_ffi/Cargo.toml:44-48`). It adds the `lk_rich_text_*` C ABI and the Swift and Kotlin views.
- **CLI:** a `uses_rich_text` check beside `uses_scene3d` (`crates/leptoskit_cli/src/native/registry.rs:600-614`). It turns on the bridge feature and forwards the app's extension features to the host build.

### 1.4 Getting pixels on screen, per platform

| Platform                                                                                                   | Engine                                                       | Measure                                                                                                                                 | Paint                                                                                                  | Input                                                                                                  | Works today                                                                              | Needs                                                                                                                    |
| ---------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| **Web**                                                                                                    | `canvist_headless` + `canvist_layout`, in the app's own wasm | The browser, through `measureText` on a canvas configured like the painting canvas (`canvist:packages/canvist-web/src/measure.ts:1-11`) | Canvas2D, with an opaque context and dirty rectangles (`canvist:crates/canvist_web/src/paint.rs:1-18`) | `EditContext` on the canvas, or a hidden textarea (`canvist:packages/canvist-web/src/input.ts:1-13`)   | canvist works only with the TS host                                                      | A Rust `CanvasMeasurer`, a Rust DOM host, and the core container                                                         |
| **iOS** (generated app, Swift renderer)                                                                    | Rust in the app's staticlib                                  | CoreText, called from Rust through the C ABI (canvist's `CoreTextMeasurer`)                                                             | CoreGraphics `DisplayPainter` (`canvist:platforms/apple/Sources/Canvist/DisplayPainter.swift`)         | `UITextInput` + `UITextInteraction` (`canvist:platforms/apple/Sources/Canvist/CanvistView.swift:1-27`) | canvist's view works standalone                                                          | A `NativeRichTextView` adapted from `CanvistView` and bound to `lk_rich_text_*` rather than UniFFI, plus the plugin      |
| **Android** (generated app, Kotlin renderer)                                                               | Same                                                         | `Paint` (`PaintMeasurer.kt`)                                                                                                            | `Canvas` (`DisplayPainter.kt`)                                                                         | `InputConnection` (`CanvistInputConnection.kt`)                                                        | Same                                                                                     | Same, in Kotlin. JNI replaces JNA and UniFFI.                                                                            |
| **macOS** (generated app, Swift renderer)                                                                  | Same                                                         | CoreText (shared with iOS)                                                                                                              | CoreGraphics (shared with iOS)                                                                         | `NSTextInputClient`                                                                                    | **Nothing in canvist.** canvist_desktop is winit-based, and `CanvistView` is UIKit only. | A new AppKit view. Its measurer and painter can be shared with iOS.                                                      |
| **Linux** (`WasmHost<GtkBackend>`, `crates/leptoskit_cli/src/native/templates/LinuxMain.rs:26-42`)         | `canvist_desktop` core, without winit                        | parley + fontique (fontconfig, loaded at run time)                                                                                      | `vello_cpu` to RGBA, then `gdk::MemoryTexture` in a custom widget                                      | `EventControllerKey` + `IMMulticontext` + gestures                                                     | canvist_desktop runs only in a winit window                                              | A GTK host in `leptoskit_gtk`, or in `leptoskit_rich_text::host::gtk`                                                    |
| **Windows** (`WasmHost<WindowsBackend>`, `crates/leptoskit_cli/src/native/templates/WindowsMain.rs:23-47`) | `canvist_desktop` core                                       | parley + fontique (DirectWrite)                                                                                                         | `vello_cpu`, then `StretchDIBits`                                                                      | `WM_KEYDOWN`/`WM_CHAR`, IMM32 composition                                                              | Same                                                                                     | A custom child window class, plus **runtime plugin registration in the Windows runner** (`docs/src/readme.md:118`)       |
| Terminal, HarmonyOS, watch                                                                                 | n/a                                                          | n/a                                                                                                                                     | n/a                                                                                                    | n/a                                                                                                    | n/a                                                                                      | A placeholder, as `Scene3d` has (`docs/src/readme.md:113`). The terminal could fall back to a Markdown `TextArea` later. |

Two native paint strategies were considered:

- **Platform text rendering** on iOS, Android, and macOS. canvist chose it for mobile and measured a p95 keystroke-to-paint of 1.2 ms on iOS and 3.7 ms on Android (`canvist:docs/src/guides/performance.md`). It matches the rest of a leptoskit app's native text: system fallback fonts, emoji, and antialiasing.
- **vello on a wgpu layer,** as `Scene3d` draws into a `CAMetalLayer`. It would give one code path, but it adds vello, wgpu, and parley to every shell, and its text looks different from the native labels around it.

Use platform rendering on Apple and Android. On GTK and Windows, CPU-raster `canvist_desktop`, because it already handles measuring and painting consistently.

### 1.5 The native Wasm guest

The guest never runs canvist. It holds a `RichTextState` whose `guest` transport sends `Create { id, extensions, theme }`, `Load`, `Run { name, args }`, `Query`, `Export`, `SetEditable`, and `Close` through `leptoskit.rich_text`. It receives events through a long-poll `NEXT_EVENTS` call, which is the `Scene3d` pattern (`docs/agents/3d.md:22`).

Events are coalesced, so a burst of typing wakes the guest once with the latest revision. Keystroke latency stays entirely in the host. The guest's `leptoskit_event` rebuild never runs per character.

Consequences:

- The engine version and extension set are compiled into the shell. A code-pushed guest that asks for an extension the shell lacks gets `Unavailable`. The plugin handshake should report the engine version and the registered extension names.
- Commands cross by name with JSON arguments. canvist's typed handles produce closures, which cannot be serialised. canvist_ffi's `Command { name, args }` is the right wire shape.
- A guest that creates `RichText` on a shell that predates the kind sends an unknown kind code. The host must render an empty view rather than fail, and the client must report `Unavailable`.

---

## 2. Text input

### 2.1 Per platform

| Capability          | Web (canvist's host, to be ported to Rust)                                                                                                                                                                                                                                                                                            | iOS                                                                                                                                                                  | Android                                                                                 | macOS                                                     | GTK                                                                                                                                                                                         | Windows                                                                                                               |
| ------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------- | --------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------- |
| **Keys**            | `keydown`, mapped to `WebEditor::key_down(key, modifiers)` with Shift, Ctrl, Alt, and Meta bits (`canvist:crates/canvist_web/src/input.rs:31-82`). `Mod` is ⌘ on Apple platforms.                                                                                                                                                     | `UIKeyCommand` and `pressesBegan`, through `UITextInput`                                                                                                             | `onKeyDown` and `InputConnection.sendKeyEvent`                                          | `keyDown:` and `interpretKeyEvents:`                      | `EventControllerKey` keyval mapped to DOM names. canvist_desktop's `KeyInput` (`canvist:crates/canvist_desktop/src/input.rs:49-79`)                                                         | `WM_KEYDOWN`/`WM_CHAR`, mapped to `KeyInput`                                                                          |
| **IME composition** | `EditContext` `textupdate`/`characterboundsupdate` (Chromium 121+), or textarea composition events, mapped to `set_composition`/`commit_composition`/`replace_text`/`character_bounds` (`input.rs:85-190`)                                                                                                                            | Marked text in `UITextInput`                                                                                                                                         | `setComposingText`                                                                      | `setMarkedText:` with a UTF-16 index                      | `IMMulticontext` `preedit-changed`/`commit`, mapped to `Ime::Preedit`/`Commit` (`canvist:crates/canvist_desktop/src/input.rs:83-93`). `set_cursor_location` takes `ime_area` (`ime.rs:42`). | IMM32 `WM_IME_COMPOSITION`, with `ImmSetCandidateWindow` placed at `ime_area`                                         |
| **Selection**       | Pointer events mapped to `pointer_down/move/up`, with double and triple click (`input.rs:195-253`)                                                                                                                                                                                                                                    | `UITextInteraction` draws handles, the loupe, and the menu                                                                                                           | `SelectionController.kt`                                                                | Mouse, with `NSTextInputClient` ranges                    | `GestureClick`/`GestureDrag`, mapped to `PointerDown`/`Move`/`Up`                                                                                                                           | Mouse messages, mapped the same way                                                                                   |
| **Clipboard**       | `copy`/`cut`/`paste` events, with HTML and text (`input.rs:256-300`)                                                                                                                                                                                                                                                                  | `UIPasteboard` through the edit menu                                                                                                                                 | `ClipboardManager`                                                                      | `NSPasteboard`                                            | canvist's `Clipboard` trait is synchronous. `gdk::Clipboard` is asynchronous, so it needs a cached read or `arboard`.                                                                       | `arboard` (already 3.7.0 in leptoskit's lockfile)                                                                     |
| **Accessibility**   | The focus target gets `role=textbox`, `aria-multiline`, `aria-label`, `aria-readonly`, and `aria-placeholder` (`canvist:packages/canvist-web/src/input.ts:128-183`). A visually hidden semantic HTML mirror (`mirror.ts:1-10`) carries the content. Screen-reader caret tracking inside a canvas is weaker than in `contenteditable`. | `UITextInput` makes VoiceOver editing work (`CanvistView+Accessibility.swift`)                                                                                       | TalkBack edits the text, and each block is a child node (`CanvistAccessibility.kt:1-7`) | Must be written: the `NSAccessibility` text-area protocol | **Gap.** GTK 4.6 has no custom accessible text (`GtkAccessibleText` arrived in 4.14). An AccessKit AT-SPI adapter would register a second tree beside GTK's.                                | AccessKit `SubclassingAdapter` on the editor's window, built from canvist_desktop's tree (`accessibility.rs:153-209`) |
| **Dictation**       | OS dictation arrives as IME text through `EditContext` or the textarea                                                                                                                                                                                                                                                                | System dictation through `UITextInput`. canvist also has on-device and relay dictation with provisional text (`OnDeviceDictationSource.swift`, `canvist_dictation`). | Voice typing through composing text                                                     | `NSTextInputClient` marked and inserted text              | Through the IBus input method, if present                                                                                                                                                   | Windows voice typing uses TSF. Whether it reaches an IMM32-only window needs checking on a device.                    |

### 2.2 What leptoskit provides and what is missing

**Provided today:**

- Web: `on_key` (`crates/leptoskit_native/src/web/mod.rs:1044-1106`), `focusin`/`focusout` to `Focus`/`Blur` (`web/mod.rs:589-647`), and the level-triggered `focused(bool)` (`element.rs:1530-1539`).
- Native: `Focus`/`Blur` on text fields (finding 6).

**Missing:**

- **Native:** focus requests (G2 in the audit), and `Focus`/`Blur` from the new editor views. Native `on_key` is not needed to type in the editor. It is needed only for app shortcuts around it (G7b).
- **The key ordering contract on the web.** leptoskit promises that "key handlers hear every key inside them, typing included, before the focused control acts on it" (`web/mod.rs:1056-1059`). Its listeners run in the bubble phase on each node (`web/mod.rs:621-630`). A canvas editor that acts in its own target-phase listener would act _before_ those ancestors. The Rust host should handle `keydown` from a window-level bubble listener, filtered to its own input target, and skip events whose default was prevented. With `EditContext`, text arrives in `textupdate` after the default action, so an ancestor that calls `preventDefault` stops the typing too.
- **⌘ shortcuts on the web.** leptoskit's `on_key` drops every Meta key (`web/mod.rs:1060`), and `KeyPress` has no `meta` (`props.rs:1115-1124`). This doesn't affect canvist's own bindings. It does mean an app cannot put a ⌘-shortcut on an ancestor of the editor until G7a lands.
- **`EditContext` bindings.** web-sys 0.3.107 has none (checked in the local registry). They need a hand-written `#[wasm_bindgen] extern "C"` block. That is Rust, not hand-written JS.

---

## 3. Public API

### 3.1 Shape

Builder form follows the repository convention. Docs would show the `view!` equivalent.

```rust
use leptoskit::rich_text::Extensions;
use leptoskit::rich_text::Format;
use leptoskit::rich_text::RichTextEditor;
use leptoskit::rich_text::RichTextState;
use leptoskit::rich_text::commands;

// The app owns the editor's state, as it owns a TableState. The handle is
// a Copy id, so handlers can capture it.
let note = RichTextState::new(Extensions::starter_kit());
note.load(Format::Markdown, &saved_markdown);

// Events come from the state, not the element (the Scene3d model).
note.on_change(move |change| {
	if change.doc_changed {
		dirty.set(true);
	}
});

View()
	.class("flex-col gap-2")
	.child(
		View().class("flex-row gap-1").child(
			Button("Bold")
				.aria_pressed(note.selection().is_active("bold"))
				.on_press(move || {
					note.run(commands::toggle_bold());
				}),
		),
	)
	.child(
		RichTextEditor(note)
			.placeholder("Write a note…")
			.read_only(saving.get())
			.accessibility_label("Note")
			.class(
				"min-h-40 w-full rounded-lg border border-slate-200 bg-white p-3 \
				 text-base leading-6 text-slate-900 dark:bg-slate-900 dark:text-slate-100",
			),
	)

// Saving is asynchronous on every platform: immediate on the web, a plugin
// round trip in a native guest.
let json = note.export(Format::Json).await?;
```

### 3.2 Document value and change events

- **The source of truth is canvist's document**, inside the page on the web and inside the host on native. leptoskit doesn't mirror it in props. A controlled `value` that replaces the whole document on each keystroke would reset composition, selection, and undo grouping. `TextArea` keeps the controlled-string model for plain text.
- **Formats:**
  - `Format::Json`: ProseMirror/TipTap JSON (`canvist:crates/canvist_headless/src/format.rs:19-29`), which is the interchange format. It loads and exports byte-for-byte the same in Rust, TypeScript, and Dart (`canvist:readme.md` "Platforms").
  - `Format::Markdown`, `Format::Html`, and `Format::Text`.
  - `export_json()` returns a `serde_json::Value`.
- **Change events:**
  - `Change { revision, doc_changed, selection_changed }` is cheap, synchronous, and coalesced on native.
  - `note.selection()` is a reactive snapshot of the active marks and blocks, `can_undo`, and `can_redo`. The host computes it once per change, so a toolbar never makes a round trip.
  - `note.snapshot(Format, debounce)` returns a `Signal<Option<String>>` for autosave.
- **Keep canvist's types out of the signatures.** Define leptoskit's own `Format`, `Command` (`{ name: &'static str, args: serde_json::Value }`, like `canvist:crates/canvist_ffi/src/types.rs:193-198`), and `RichTextError`. An `engine` feature can expose `with_doc(|doc: &canvist_headless::Doc| …)` on the web and the host as an escape hatch. canvist is 0.x and moving, and this keeps its releases from becoming breaking changes in leptoskit.

### 3.3 Extensions and schema

- `Extensions::paragraphs()` registers the document, paragraph, history, and hard break. Use it for the first slice.
- `Extensions::starter_kit()` gives TipTap's StarterKit.
- Builder methods (`.bold().heading()`) exist only for Cargo features the app enabled, so using an unregistered extension is a compile error. This keeps canvist's typed-handle guarantee (`canvist:readme.md` "Why?").
- `commands::*` and `queries::*` are generated from `Engine::metadata` into name-plus-arguments constructors. canvist already generates typed handles for Swift, Kotlin, and TypeScript from the same metadata (`canvist:platforms/apple/Sources/Canvist/Extensions.generated.swift`), so a Rust generator is a small addition, upstream or in leptoskit.
- The schema is whatever the extensions register. Loading JSON with unknown nodes fails with an error rather than dropping content (`Engine::from_json`).
- The extension set must match between the guest and the host build. The CLI forwards `leptoskit_rich_text`'s extension features to the host, the way it forwards `scene3d` (`crates/leptoskit_cli/src/native/registry.rs:548-553`).

### 3.4 Read-only, placeholder, and focus

These reuse `Element` props that `TextInput` and `TextArea` already use, or that G9 adds:

- `read_only(bool)` maps to canvist's `setEditable`. Navigation and selection keys still work (`canvist:packages/canvist-web/src/input.ts:37-49`).
- `placeholder(..)` is drawn in the body font, and is announced through `aria-placeholder` on the web (`canvist:packages/canvist-web/src/placeholder.ts`).
- `accessibility_label`, `focused(true)`, and `Focus`/`Blur` handlers work as on any field.

`enabled(false)` blocks focus. `read_only` doesn't.

### 3.5 Styling with leptoskit's Tailwind and theme

- **Box utilities** (size, padding, border, radius, background) style the container, like any element.
- **Typography comes from the container's resolved text style:**
  - `text-*` sets `StarterKitOptions.size`.
  - `leading-*` sets `line_height`.
  - The text colour sets the base colour.
  - `font-sans`/`font-serif`/`font-mono` set `body_family`. This works on the web only, because native text keeps the system face (`crates/leptoskit_native/src/tailwind/declaration.rs:152-155`).
  - The background sets `Chrome.background`.
  - `dark:` sets `ColorScheme::Dark`.

  These map onto `canvist:crates/canvist_layout/src/theme/starter_kit.rs:120-134` and `theme.rs:644-655`. On the web the host reads `getComputedStyle` of the container, so `var(--…)` tokens resolve for free. On native the host reads them from `NativeProps` (font size, line height, colour, background) and the `Theme` maps. It re-themes and repaints when `leptoskit::appearance` changes (`crates/leptoskit_native/src/appearance.rs`).
- **Caret and selection colours** come from a theme token at first. `caret-*` and `selection:bg-*` utilities would be a small, separate class-compiler PR.
- **Heading, code, and quote styles** use canvist's StarterKit theme scaled from the base size in v1. A `RichTextTheme` with per-block overrides, mapped onto canvist's `ThemeBuilder`, can come later. Don't expose `ThemeBuilder` itself.

### 3.6 Collaboration

Collaboration is opt-in through the `collab` feature, which links `canvist_collab`/yrs. It is never in the default build.

- `note.share(client_id)` returns a `Collab` with `updates()`, `apply(&[u8])`, and later awareness, for remote carets.
- Transport is the app's choice. It can use leptoskit's websocket plugin, `#[rpc::stream]` (which needs the audit's G3, streams without a database), y-websocket/Hocuspocus, or canvist Cloud.
- The server side can be a `canvist_headless` y-sync peer behind the `server` feature.

### 3.7 Where it lives

A new optional crate, `crates/leptoskit_rich_text`, behind `leptoskit`'s `rich-text` feature. It is re-exported as `leptoskit::rich_text` and forwarded through `web`, `guest`, and `server`, the way `3d` is (`crates/leptoskit/Cargo.toml:149,262-273`; `src/lib.rs:459`). This matches the owner's direction that heavy capabilities are separately installable modules. Core carries only the id kind.

**Long term, generalise host-owned surfaces.** A real `Custom` registry would need four things: a guest wire code that names the widget, a web renderer hook, a native host registry, and a backend presentation hook that replaces the Rive-specific `present_rive`. With those, `RichText`, `Scene3d`, `Map`, and the rest could all live outside core. Not for the first slice: adding one kind follows five precedents and keeps the ABI append-only.

---

## 4. Dependency direction and versioning

- **leptoskit depends on canvist.** `leptoskit_rich_text` depends on:
  - `canvist_headless` (`default-features = false`, plus the chosen extensions)
  - `canvist_layout` (`default-features = false`; `editor` and `json` on the web, plus `parley` on GTK and Windows through `canvist_desktop`)
  - `canvist_extensions` (features)
  - optionally `canvist_collab` and `canvist_desktop` (`default-features = false`) and `canvist_paint` (`cpu`)

  Depend on the individual crates rather than the `canvist` facade. Its `layout` feature turns on ink, kinetic text, tables, and theme extensions (`canvist:crates/canvist/Cargo.toml`, `layout`), which the web build doesn't want.
- **canvist should not depend on leptoskit.** The integration needs a core kind, bridge renderers, and CLI detection, and only leptoskit can own those. canvist depending on leptoskit would also tie canvist to leptoskit's nightly toolchain and 0.x API. There is no cycle as long as canvist never imports leptoskit. If canvist wants a leptoskit example, it should live in leptoskit's `examples/`.
- **Unpublished crates block publishing.** `canvist_web`, `canvist_ffi`, `canvist_canvas_wasm`, and `canvist_headless_wasm` are deliberately unpublished. Cargo won't publish a crate with git dependencies, even optional ones. So `leptoskit_rich_text` (and any `leptoskit_bridge_ffi` feature that reaches canvist_ffi) must be either:
  - `publish = false` and git-pinned to a canvist commit, as an experimental crate, or
  - built only on published canvist crates.
- **Version skew.** crates.io `0.2.1` (2026-11-07, `canvist:changelog.md:3`) predates `main`'s per-extension crates (`canvist:.changeset/extension-crates.md`). Its `canvist_extensions` has the old feature layout, and the new `canvist_extension_*` crates exist only as `0.0.0`. Pin a git revision until canvist's next release (probably 0.3.0), then move to crates.io. canvist's readme still says it "hasn't had its first release yet" (`canvist:readme.md:80`), which is stale.
- **Toolchains match.** canvist uses Rust 1.96.0 (`canvist:rust-toolchain.toml`) with MSRV 1.95.0. leptoskit uses nightly-2026-10-21 with MSRV 1.95.0. Both are edition 2024, and both lockfiles have wasm-bindgen 0.2.129 and web-sys 0.3.106.
- **`--cfg canvist_omit_docs` is a problem for consumers** (`canvist:crates/canvist_core/src/meta.rs:203-211`). Only global `RUSTFLAGS` can set a cfg, so leptoskit's build couldn't turn it on for canvist alone without changing every crate's cache key. Ask canvist for an `omit-docs` Cargo feature.

**What canvist would need to publish or expose:**

1. **`canvist_web` as a Rust library.** A constructor that takes a Rust measurer, plus a Rust `CanvasMeasurer` ported from `measure.ts`, with its WebKit font-resolution and ligature probes. The wasm-bindgen exports would move behind a default `bindings` feature, and `publish = true`. Then add a `dom` feature with the Rust DOM host: the textarea and `EditContext` input, pointer, clipboard, frame loop, resize and scroll, font invalidation, mirror, and placeholder. In canvist it gets the Playwright parity suites. Dioxus and Yew users would benefit too, and `@canvist/web` could adopt it later.
2. **`canvist_session`,** extracted from `canvist_ffi`. It would hold the host-neutral session: the UTF-16 text index (`canvist:crates/canvist_ffi/src/text.rs`), the measure request and response records (`measure.rs`), display frames and damage, and the accessibility lines, with no UniFFI. `canvist_ffi` would wrap it, and leptoskit would bind it through `lk_rich_text_*`. `NSTextInputClient` uses the same UTF-16 model, so macOS reuses it too.
3. **The per-extension crates and the above, released** at one version.
4. **Optionally,** an AppKit `CanvistNSView` in canvist's Swift package. That would put the macOS view upstream rather than in leptoskit.

---

## 5. Size and performance

**Web (measured by canvist, not by this investigation):**

| What                                             | Gzipped                                                | Source                                                                                       |
| ------------------------------------------------ | ------------------------------------------------------ | -------------------------------------------------------------------------------------------- |
| StarterKit editor as an app ships it             | 362.7 KB total: 348 KB wasm + 14 KB JS                 | `canvist:docs/src/guides/performance.md:62,68`                                               |
| Model only                                       | 38 KB                                                  | `canvist:docs/internal/architecture.md:116-121`                                              |
| + layout and both renderers                      | 133 KB                                                 | same                                                                                         |
| + parley measurer (why the web uses the browser) | 416 KB                                                 | same; `ssr-css-contract.md:105`                                                              |
| `full` (every extension)                         | about 470 KB                                           | `canvist:packages/canvist-web/changelog.md:84`                                               |
| Collaboration                                    | +140 KB, +190 KB, or +250 KB (canvist's docs disagree) | `canvist:docs/src/guides/collaboration.md:31,49`; `canvist:crates/canvist_web/Cargo.toml:33` |

**Estimate for leptoskit, to be confirmed by the first PR's size report.** Inside a leptoskit app, wasm-bindgen, web-sys, serde_json, regex-lite, and unicode-segmentation are shared (both lockfiles have them). `icu_segmenter` (about 14–23 KB) and the canvist code are new. Rough marginal costs:

- about 150–200 KB gzipped for paragraphs only;
- about 280–340 KB for the StarterKit;
- 140–250 KB more for collaboration.

The Rust DOM host's web-sys glue adds a little over the TS host's 14 KB.

**Startup.**

- Put `RichTextEditor` behind a lazy route, `#[lazy]`, or `#[island(lazy)]`. leptoskit splits these with wasm-split by default (`docs/src/guide/03-configuration.md:115`, `docs/src/guide/09-small-bundles.md`), so pages without an editor pay nothing.
- Pre-render the document on the server with `canvist_layout::to_html`, which needs no measurer (`canvist:crates/canvist_layout/src/render/html.rs:99`). The box then shows content while the chunk loads, and the canvas replaces it in place, as `@canvist/web` does (`canvist:packages/canvist-web/src/editor.ts:441-443,762-775`).
- canvist's budgets, from `canvist:docs/src/guides/performance.md`:
  - a keystroke costs about 0.3 µs in the model and 2.5 µs to lay out;
  - opening 10,000 paragraphs takes 26 ms to the first frame on desktop;
  - keystroke-to-paint p95 is 1.2 ms on iOS and 3.7 ms on Android.
- Text is measured by the browser, so wrapping is wrong until fonts load. The host must invalidate on `document.fonts` (`WebEditor::fonts_loaded`, `canvist:crates/canvist_web/src/lib.rs:319`).

**Native.**

- **The guest bundle grows only by the thin client,** so code push payloads are unchanged.
- **The shell grows by the engine.** iOS, Android, and macOS get `canvist_headless` + `canvist_layout` with no parley. GTK and Windows also get parley, fontique, and `vello_cpu`. parley alone is 685 KB raw in wasm (`canvist:docs/internal/ssr-css-contract.md:105`), so expect a native size of the same order. Measure it per target.
- **Typing never enters the interpreter.** Change events are coalesced, and an export costs fuel only in proportion to its bytes (`crates/leptoskit_wasm/src/plugins.rs:31-58`).

---

## 6. Plan

### 6.1 PR sequence

**Prerequisites, already planned elsewhere:** text input hints (`read_only`, G9) and `TextArea` (G12) from the audit (`js-to-wasm-audit.md:537-538`). `RichText` takes the next wire code after `TextArea`.

**canvist repository:**

| #  | PR                                                                                                                                                                                                                                               | Size | Blocks                           |
| -- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ---- | -------------------------------- |
| C1 | `canvist_web`: a Rust measurer trait and Rust `CanvasMeasurer`; wasm-bindgen exports behind `bindings`; publishable                                                                                                                              | M    | L2                               |
| C2 | `canvist_core`: an `omit-docs` Cargo feature (keep the cfg as an alias)                                                                                                                                                                          | S    | L2's size budget                 |
| C3 | `canvist_web` `dom` feature: the Rust DOM host. Textarea input, pointer, plain-text clipboard, frame loop, and resize first; `EditContext`, HTML clipboard, drag and drop, mirror, and fonts next. Covered by canvist's Playwright parity tests. | L    | L2, L4                           |
| C4 | Extract and publish `canvist_session`; `canvist_ffi` wraps it                                                                                                                                                                                    | M–L  | L5–L7                            |
| C5 | Release: the per-extension crates and C1–C4                                                                                                                                                                                                      | S    | publishing `leptoskit_rich_text` |
| C6 | Optional: an AppKit `CanvistNSView` in canvist's Swift package                                                                                                                                                                                   | L    | L7 (alternative)                 |

**leptoskit repository:**

| #      | PR                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  | Size   | Depends on              |
| ------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------ | ----------------------- |
| L1     | `feat(native): a RichText host surface`. Adds `NativeKind::RichText`, its wire code, the `rich_text` prop, `Element::rich_text`, and the web container `<div data-lk-rich-text>`. Every backend and renderer gets an empty view, the terminal a placeholder, and the watch surfaces refuse it. Plus validation, a changeset, and the "Where things stand" list.                                                                                                                     | S      | TextArea                |
| **L2** | **`feat(rich-text): a web editor for paragraphs`.** Adds `leptoskit_rich_text` (`publish = false`, canvist git-pinned) with the `web` feature: `RichTextState`, `RichTextEditor`, `Extensions::paragraphs()`, load and export in Text, Markdown, and JSON, `on_change`, `read_only`, `placeholder`, theme from computed style, textarea input, pointer selection, plain-text clipboard, and `Focus`/`Blur`. Also the `leptoskit` feature `rich-text` and a web-only gallery screen. | M      | L1, C1, C3 (minimal)    |
| L3     | `feat(rich-text): StarterKit, commands, and toolbar state`. Generated `commands` and `queries`, the reactive `selection()` snapshot, and the HTML clipboard.                                                                                                                                                                                                                                                                                                                        | M      | L2                      |
| L4     | `feat(rich-text): EditContext input and server first paint`. `EditContext` where available; the `to_html` SSR fallback that the canvas replaces in place; lazy-chunk guidance in the docs.                                                                                                                                                                                                                                                                                          | M      | L2, C3                  |
| L5     | `feat(rich-text): native host and guest client`. The `host` and `guest` features, the `leptoskit.rich_text` plugin (create, load, run, query, export, `NEXT_EVENTS`, close, plus a version and extension handshake), a Hub with a mutex per editor, the bridge's `rich_text` feature with `lk_rich_text_*`, and CLI detection.                                                                                                                                                      | L      | L3, C4                  |
| L6     | iOS and Android editor views, adapted from canvist's `CanvistView` (Unlicense), bound through the C ABI                                                                                                                                                                                                                                                                                                                                                                             | L      | L5                      |
| L7     | macOS editor view (`NSTextInputClient`, `NSAccessibility`), sharing the CoreText measurer and painter with iOS                                                                                                                                                                                                                                                                                                                                                                      | L      | L6 (or C6)              |
| L8     | GTK and Windows hosts: `canvist_desktop` core, `vello_cpu` frames, `IMMulticontext` and IMM32, AccessKit on Windows. Generalise `present_rive` into a host bitmap hook. **Windows runner plugin registration** is its own earlier PR.                                                                                                                                                                                                                                               | L each | L5                      |
| L9     | `feat(rich-text): collaboration` (`collab` feature)                                                                                                                                                                                                                                                                                                                                                                                                                                 | M      | L3, G3 for an rpc relay |

### 6.2 The smallest first slice: L1 + L2

L1 and L2 give web only, plain paragraphs, and the textarea input path. In practice that is:

- a `<div data-lk-rich-text>` from core, with an opaque canvas and a hidden textarea inside, attached by `leptoskit_rich_text::web`;
- typing, Enter to split a paragraph, Backspace to join two, undo and redo, mouse selection, and plain-text copy and paste;
- `export(Format::Text | Markdown | Json)` and `on_change`;
- `read_only` and `placeholder`;
- body size, colour, and dark mode taken from the container's classes.

Leave these out of the slice: marks, headings, `EditContext`, SSR first paint, native, and collaboration.

If C3 is slow to land, the textarea host (about 500–800 lines of Rust) can start in `leptoskit_rich_text::web` and move upstream afterwards. Building it in canvist is better, because canvist's line-wrapping parity tests then cover it. Do not construct a JS `HostMeasurer` from Rust closures as a shortcut. It works, but it keeps the JS seam the rule is meant to remove.

### 6.3 Risks

| Risk                                                                                                                                                                       | Mitigation                                                                                                                                        |
| -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| Porting canvist's TS host loses hard-won fixes: WebKit system-font resolution, ligature probing, pixel-grid snapping (`canvist:packages/canvist-web/src/measure.ts:15-40`) | Do the port in canvist (C1, C3) so its parity tests run against it                                                                                |
| Canvas editors are weaker for screen readers than `contenteditable`. GTK 4.6 can't expose custom text.                                                                     | The web mirror and the textbox role; AccessKit on Windows; document the GTK gap in "Where things stand" until leptoskit can require GTK 4.14      |
| Native scope is very large: five hosts, no macOS view upstream, no Windows plugins                                                                                         | Ship web, then iOS and Android, which reuse canvist's views; then macOS; then GTK and Windows. Each platform says plainly what's missing.         |
| Old shells and code push: an unknown kind code, or a missing extension                                                                                                     | An empty view plus `Unavailable`; a version and extension handshake in `Create`                                                                   |
| canvist is 0.x and moving fast; its published 0.2.1 predates `main`                                                                                                        | A git pin while experimental; canvist types kept out of the public API; publish only after C5                                                     |
| leptos hydration with DOM children leptos didn't render, inside a leptos-managed element                                                                                   | An L2 test under `hydrate`. If it fails, let core render the canvas, and have the host only attach.                                               |
| The key-ordering contract (section 2.2)                                                                                                                                    | A window-level bubble listener that honours `defaultPrevented`, with a test                                                                       |
| Auto-growing height natively needs content-height feedback from the host to layout                                                                                         | The web grows with CSS. Native starts at a fixed or `min-h` size with internal scroll; add a content-size event later.                            |
| Size creep (collaboration, `full`)                                                                                                                                         | A size budget in L2's PR; lazy chunks; `collab` and `full` never on by default                                                                    |
| A generic feature                                                                                                                                                          | Lead the showcase with one canvist differentiator (ink, kinetic text, or dictation that settles into ink). Keep the toolbar and extras out of v1. |

### 6.4 What to verify

**Web (L2, L4):**

- Browser tests run in leptoskit's harness or a Playwright suite: typing, Enter and Backspace across paragraphs, undo and redo, double-click word and triple-click block selection, Shift-click extension, and copy and paste of text (and HTML in L3).
- `read_only` blocks typing but still allows selection and copy.
- The placeholder shows and hides, and `aria-placeholder` is set.
- The focus target has `role=textbox`, `aria-multiline`, and the label.
- `Focus` and `Blur` reach leptoskit handlers.
- An ancestor `on_key` that returns `Handled` stops the editor from acting.
- Dark mode and resizing repaint and re-wrap. A late font load invalidates the measurements.
- IME composition: through CDP `Input.imeSetComposition` on the textarea path, then `EditContext` in L4.
- SSR HTML and the canvas wrap identically (canvist's `geometry()` compared with DOM rectangles).
- The bundle-size delta is reported by `leptoskit build`, with an explicit budget.
- Keystroke-to-paint latency is measured the way `editor.inputLatency()` measures it.

**Native (L5–L8):**

- The plugin protocol is tested in `async-host-tests`: ordering, coalesced events, `Unavailable` for unknown extensions, and old-shell fallback.
- Per platform:
  - the hardware keyboard, and the software keyboard on mobile;
  - a Japanese IME composition;
  - VoiceOver and TalkBack read and move by word;
  - the clipboard works both ways;
  - the guest's `export` equals the host's document;
  - a `focused(true)` request moves focus, once G2 lands.

**Across platforms:** the same JSON loaded on the web and on iOS exports identical JSON. canvist's conformance fixtures can serve as the corpus (`canvist:conformance/`).

### 6.5 Not verified in this investigation

- No code was built and no sizes were measured. The leptoskit size figures in section 5 are estimates.
- Whether leptos tolerates DOM children it didn't render under hydration.
- Whether Windows voice typing reaches an IMM32-only window.
- The exact native binary growth.
- Whether canvist's Swift and Kotlin views can switch from UniFFI to a C ABI as cleanly as their structure suggests. They use UniFFI records throughout.
