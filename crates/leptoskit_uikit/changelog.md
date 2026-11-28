# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-28)

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

#### An app's own fonts on every platform

_Owner:_ Ifiok Jr. · _Introduced in:_ [b4744bd](https://github.com/leptoskit/leptoskit/commit/b4744bd432011c953a8a1a932f3306b270eed0ef)

Declare the typefaces an app bundles once, in `leptoskit.toml`, and choose one with the class the web already uses:

```toml
[theme.fonts.hand]
family = "Skribble Noto Sketch"
faces = [
	{ file = "fonts/NotoSketch-Regular.ttf" },
	{ file = "fonts/NotoSketch-Bold.ttf", weight = 700 },
	{ file = "fonts/NotoSketch-Italic.ttf", style = "italic" },
]

[theme.fonts.display]
faces = [{ file = "fonts/Recursive.ttf", weight = [300, 1000] }]
```

`font-[var(--hand)]` now draws the family on the web, iOS, Android, and macOS, and every text inside the element inherits it. Faces are matched by weight and style as `@font-face` rules are, a variable face draws the weight asked for on its `wght` axis, and layout measures each label in the face that draws it.

- The build checks every declared file and stops with an error naming the `[theme.fonts]` entry and the path it looked at when a file is missing or isn't a TrueType or OpenType font.
- Native builds write `fonts.json` beside the app bundle; the generated iOS, Android, and macOS hosts register the files before their first layout. The web build writes and links `leptoskit-fonts.css`.
- `leptoskit_native::fonts` holds the manifest, the face selection, and `install` for hosts written in Rust; `NativeProps::font_family` and `TextStyle::{italic, face}` carry the choice to measurement and drawing. `font-sans`, `font-serif`, and `font-mono` now return to the system font on native platforms.
- The AppKit and UIKit backends draw and measure declared faces once installed. The host text measurer's C ABI gains the face to measure in, and `lk_fonts_install` and `lk_fonts_select` let Swift and Kotlin hosts install the manifest and choose faces exactly as layout does.
- Windows, Linux, HarmonyOS, watchOS, the terminal, and the Rust `leptoskit_android` backend still draw the system font.

#### iOS presents Modal as a sheet and Popover as a popover

_Owner:_ Ifiok Jr. · _Introduced in:_ [9055533](https://github.com/leptoskit/leptoskit/commit/905553347de313983dc7fadd12b419d286dabe38)

On iOS a `Modal` is now a form sheet: it dims the screen, keeps touches and VoiceOver to itself, and gives focus back when it closes. A `Popover` is a system popover pointing at its anchor, on an iPhone as well as an iPad. A swipe down on the sheet, a press outside the popover, Escape on a hardware keyboard and the VoiceOver escape gesture all ask to dismiss, and the overlay stays until your `open` signal closes it. `initial_focus` raises the keyboard for a field and moves the VoiceOver cursor for anything else. This holds for generated iOS apps (the Swift renderer) and for Rust hosts on `UIKitBackend`.

What differs from the web: a sheet is as large as what the modal holds on an iPad and as wide as the screen on an iPhone, and the modal's own background is not drawn; and a popover is centred on its anchor, so `placement` picks above or below only.

`UIKitBackend` gains `overlay_shown`, `overlay_controller`, `perform_dismiss` and `present_overlays`. A controller that mounts the tree itself calls `present_overlays` from `viewDidAppear:`; `UIKitHost` does so already. Generated iOS projects ship one more Swift file, `MobileOverlayView.swift`.

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

#### leptoskit_uikit: the iOS backend

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_uikit` is the iOS backend: it renders shared leptoskit components as real UIKit views and controls, such as `UISwitch`, `UISlider`, `UIProgressView`, and secure text fields for password inputs, and measures text with Core Text.

It draws every component when you host screens with `UIKitHost` in your own app, which keeps the layout inside a view controller and reports rotation and safe-area insets. The app generated by `leptoskit build` does not use it yet: that app draws ten components through its own Swift renderer. iOS 15 is the minimum, and images load from local files and app bundle resources, not web URLs.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
- **Use Objective-C message forwarding for UIKit text input traits so debug builds can create text and password fields without crashing.** _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #516](https://github.com/leptoskit/leptoskit/pull/516)
