# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-25)

### Features

#### CanvistEditor: a place for a canvist text editor

_Owner:_ Ifiok Jr. · _Introduced in:_ [5c71502](https://github.com/leptoskit/leptoskit/commit/5c71502b830239c1bcbf9301497142aa2003aa9a)

`CanvistEditor(id)` is a new element, with its own `NativeKind::CanvistEditor`, that marks where a [canvist](https://github.com/canvist/canvist) text editor shows. The app owns the editor through a state handle and gives the element the handle's id, so the document never travels through props. `placeholder`, `read_only`, and an accessible label work as on a `TextArea`, and the element must have a name like any text field.

This change adds only the place. On the web the element renders an empty `<div data-lk-canvist-editor="id">` that the editor's host looks for, and a server-rendered page names the browser's own editor on it when it hydrates; the host is the experimental, unpublished `leptoskit_canvist` crate, and the `leptoskit` component that joins the two comes next. macOS, iOS, Android, Linux, and Windows show an empty view, the terminal a dim block, and the watch surfaces refuse the element. The Swift and Kotlin renderers that generated apps use show an empty view too; the Swift ones used to stop on a kind they did not know.

The wire name is `canvist_editor`, appended last in the wire kind table, so existing codes keep their meaning.

#### Move keyboard focus once with `FocusRequest`

_Owner:_ Ifiok Jr. · _Introduced in:_ [f9752b8](https://github.com/leptoskit/leptoskit/commit/f9752b87d178230e24d89cb8df527e6f7e9518b6)

`FocusRequest::new()` makes a `Copy` handle. Give it to a widget with `focus_on(request)` and call `request()` from any handler: focus moves to that widget once, after the screen next updates, and later updates leave focus where the person has since put it. Use it to land in the code field after a code is sent, to return to the button that opened a menu, or to put a screen reader on an error summary. `focused(true)` is unchanged and still suits a widget that should always hold focus.

It works on every backend: `focus()` on the web (text and containers get `tabindex="-1"` so they can take it), the focus ring in the terminal, `grab_focus` on GTK, first responder on macOS and iOS, `SetFocus` on Windows and `requestFocus` on Android. Where a platform has no keyboard focus for a widget, iOS and Android move the VoiceOver or TalkBack cursor instead, and macOS and Windows leave focus alone. On macOS, Windows, iOS and Android the widget does not hear a `Focus` event for a requested move.

Renderer authors: `NativeProps` gains `focus_request`. `NativeSurface` sends it only with the commit that makes a new request, so a backend focuses `MutationList::focus_target()` after applying a batch and keeps no state.

#### macOS shows Modal as a sheet and Popover as NSPopover

_Owner:_ Ifiok Jr. · _Introduced in:_ [4cba9d4](https://github.com/leptoskit/leptoskit/commit/4cba9d4cc9bafbfdd824dfca3ce310b3ad3cda0d)

On macOS a `Modal` is now a sheet on the app's window: it dims the window, keeps clicks and the keyboard to itself, and gives focus back when it closes. A `Popover` is an `NSPopover` pointing at its anchor. Escape asks to dismiss either one, and a press outside asks to dismiss a popover. `initial_focus` moves focus into the overlay when it opens. This holds for generated macOS apps (the Swift renderer) and for Rust hosts on `AppKitBackend`.

What differs from the web: a sheet is as large as what the modal holds and the modal's own background is not drawn; a sheet has no outside to press; and a popover is centred on its anchor, so `placement` picks above or below only.

`AppKitBackend` gains `overlay_shown`, `overlay_window` and `perform_dismiss`, and `open_window` shows overlays that were already open. Generated macOS projects ship one more Swift file, `MacOverlayView.swift`.

#### Scroll an element into view with `Reveal`

_Owner:_ Ifiok Jr. · _Introduced in:_ [6aef48a](https://github.com/leptoskit/leptoskit/commit/6aef48a580ac38f8dc06b281c49c9036faf04151)

A screen can now bring one element into view without knowing where layout put it. Create a `Reveal`, give it to the element with `reveal_on(&reveal, ScrollAlign::Nearest)`, and call `reveal.request()` from any handler. Each request scrolls once, after the next update, and the person scrolls freely afterwards. `ScrollAlign` is `Start`, `Center`, `End` or `Nearest`, and `Nearest` does nothing when the element is already in view.

On the web this is the browser's `scrollIntoView`, smooth unless the person asked for reduced motion. In native apps the surface works out the offset from layout and scrolls the nearest `ScrollView` around the element. Only that one view scrolls, and an element outside any `ScrollView` is left alone.

The GTK, Windows, AppKit and UIKit backend crates now apply `scroll_offset`, which they ignored before, so `Reveal` and `VirtualList`'s programmatic scrolling work in the generated Windows and Linux apps. GTK, Windows and AppKit jump to the offset; UIKit animates when asked. The terminal and HarmonyOS still don't scroll from code.

`NativeProps` gains a `reveal` field, which the surface answers itself and never sends to a backend.

#### TextArea: a field for several lines of text

_Owner:_ Ifiok Jr. · _Introduced in:_ [ce9d470](https://github.com/leptoskit/leptoskit/commit/ce9d470863cd327cdd87a36a12738c0fd356986c)

`TextArea()` is a new component, with its own `NativeKind::TextArea`, for notes, comments and messages. Enter adds a line instead of submitting, long lines wrap, and the field scrolls its own content. It is controlled like `TextInput`: `on_change` receives the proposed text with lines separated by `\n`, and `placeholder`, `read_only`, `max_length`, `capitalize`, `autocorrect` and `autocomplete` apply. It never raises `Submit`.

The web renders a `<textarea>`; macOS an `NSTextView` in an `NSScrollView`; iOS a `UITextView`; Android a multi-line `EditText`; Linux a `gtk::TextView` in a `gtk::ScrolledWindow`; Windows a multi-line `EDIT` control; the terminal a wrapping editor with line-wise arrow keys. The Swift and Kotlin renderers that generated apps use draw it too. GTK and Windows show no placeholder, and HarmonyOS and the watch surfaces do not draw the component.

`aria_multiline(true)` on a `TextInput` or `PasswordInput` now fails validation, because a single-line field that calls itself multi-line misreports itself to assistive technology. Use `TextArea` instead.

- **Typed text-input hints.** `TextInput` now carries the keyboard and text-editing hints that hand-written JavaScript used to provide, so every interface stays in leptoskit's own Wasm tree: `keyboard` (Text, Email, Number, Decimal, Phone, Url, Search), `read_only`, `max_length`, `capitalize` (None, Words, Sentences, Characters), `autocorrect`, and `submit_label` (Done, Go, Next, Search, Send). Each platform maps them to its own controls: the web to `type`, `inputmode`, `readonly`, `maxlength`, `autocapitalize`, `autocorrect`, `spellcheck`, and `enterkeyhint`; iOS to `keyboardType`, `autocapitalizationType`, `autocorrectionType`, `spellCheckingType`, and `returnKeyType`; Android to `inputType`, `imeOptions`, `InputFilter.LengthFilter`, and a cleared key listener for read-only fields; GTK to `set_input_purpose`, `set_editable`, `set_max_length`, and `InputHints`; macOS to `isEditable` and delegate truncation plus the field editor's correction and spell checking; Windows to `EM_SETREADONLY` and `EM_LIMITTEXT`. `read_only` implies `aria-readonly`. The Swift and Kotlin bridge renderers and the terminal backend honour the same props. _Owner:_ Ifiok Jr. · _Introduced in:_ [6d42b06](https://github.com/leptoskit/leptoskit/commit/6d42b068273560b572fa4c7f5f4475b33a03563b)

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

### Breaking changes

#### leptoskit_appkit: the macOS backend

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_appkit` is the macOS backend: it applies leptoskit mutation batches to real `NSView`s, from labels and buttons to `NSSecureTextField` password fields, sliders, and scroll views, and measures text with AppKit's own fonts.

A `NativeSurface` built on `AppKitBackend` renders the same tree the web build renders as DOM, and `snapshot_png` draws it offscreen without a window. AppKit is the reference backend that the others follow, and it draws every built-in component. The `macos` feature brings it in, but the macOS app `leptoskit new` generates still draws through a smaller Swift renderer, which covers ten components so far. On other platforms the crate compiles to an empty library.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
