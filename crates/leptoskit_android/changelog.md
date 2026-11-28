# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-28)

### Features

#### Move keyboard focus once with `FocusRequest`

_Owner:_ Ifiok Jr. · _Introduced in:_ [f9752b8](https://github.com/leptoskit/leptoskit/commit/f9752b87d178230e24d89cb8df527e6f7e9518b6)

`FocusRequest::new()` makes a `Copy` handle. Give it to a widget with `focus_on(request)` and call `request()` from any handler: focus moves to that widget once, after the screen next updates, and later updates leave focus where the person has since put it. Use it to land in the code field after a code is sent, to return to the button that opened a menu, or to put a screen reader on an error summary. `focused(true)` is unchanged and still suits a widget that should always hold focus.

It works on every backend: `focus()` on the web (text and containers get `tabindex="-1"` so they can take it), the focus ring in the terminal, `grab_focus` on GTK, first responder on macOS and iOS, `SetFocus` on Windows and `requestFocus` on Android. Where a platform has no keyboard focus for a widget, iOS and Android move the VoiceOver or TalkBack cursor instead, and macOS and Windows leave focus alone. On macOS, Windows, iOS and Android the widget does not hear a `Focus` event for a requested move.

Renderer authors: `NativeProps` gains `focus_request`. `NativeSurface` sends it only with the commit that makes a new request, so a backend focuses `MutationList::focus_target()` after applying a batch and keeps no state.

- **Gestures on a view.** A view can follow a finger. `gesture(Gesture::tap())`, `pan`, `long_press`, `pinch`, `rotation`, and `fling` recognize the pointer in one arena shared by the Android widget backend and Linux. `simultaneous`, `exclusive`, and `race` compose recognizers the way a pinch and a rotation share one gesture, or a pan wins over a tap. A drag that wins claims the pointer so the control underneath does not also click. A tap does not, so a button still presses. iOS, macOS, Windows, the web, the terminal, and the generated Swift and Kotlin renderers still deliver presses and clicks only. _Owner:_ Ifiok Jr. · _Introduced in:_ [955a1bf](https://github.com/leptoskit/leptoskit/commit/955a1bf7d00666992a8ccd52032131c7ae97cca1)

#### A field's label focuses it on the web

_Owner:_ Ifiok Jr. · _Introduced in:_ [0170294](https://github.com/leptoskit/leptoskit/commit/0170294ffef2da84f21c303fb7859fc8eec160e8)

A `Text` that names a field through `aria_labelled_by` now renders as `<label for="…">` on the web, so a click on the label focuses a `TextInput`, `PasswordInput`, `TextArea` or `Slider` and toggles a `Checkbox`, `Switch` or `Radio`, with no handler and no script. It applies when the field has an `id`, is named by exactly one plain `Text`, and no other field is named after the same text. `leptoskit_aria::Field` and `leptoskit::form` already build this shape. The markup for such a text changes from `<span>` to `<label>`; its styles and its `id` stay the same.

On Android, the label's view gets `labelFor`, so TalkBack knows which field it names. Other platforms are unchanged: clicking a label is not a convention there, and the field already carries the label's text as its name.

#### Android presents Modal as a dialog and Popover as a popup

_Owner:_ Ifiok Jr. · _Introduced in:_ [f740aa1](https://github.com/leptoskit/leptoskit/commit/f740aa1e5c264305477d17f75c66296993833da2)

On Android a `Modal` is now a dialog in a window of its own: it dims the screen, keeps touches, the keyboard and TalkBack to itself, and is as large as what the modal holds. A `Popover` is a popup window placed beside its anchor where `placement` says. The Back gesture, Escape and a touch outside ask to dismiss. A dialog stays until your `open` signal closes it. The Back gesture closes a popup window before your app hears of it, so a popover your app keeps open is shown again. `initial_focus` moves focus to the named element when the overlay opens. This holds for generated Android apps (the Kotlin renderer) and for Rust hosts on `AndroidBackend`.

What differs from the web: the modal's own background is not drawn, because the dialog dims the screen, and a dialog needs an Activity behind the view the tree is mounted in.

`WidgetHost` gains `overlayShown`, `overlayDialog` and `overlayPopup`. Generated Android projects ship one more Kotlin file, `NativeOverlayView.kt`.

This also restores two things the Android host needed to compile: an import in `WidgetHost.kt` and the `Dismiss` case in the probe.

#### TextArea: a field for several lines of text

_Owner:_ Ifiok Jr. · _Introduced in:_ [ce9d470](https://github.com/leptoskit/leptoskit/commit/ce9d470863cd327cdd87a36a12738c0fd356986c)

`TextArea()` is a new component, with its own `NativeKind::TextArea`, for notes, comments and messages. Enter adds a line instead of submitting, long lines wrap, and the field scrolls its own content. It is controlled like `TextInput`: `on_change` receives the proposed text with lines separated by `\n`, and `placeholder`, `read_only`, `max_length`, `capitalize`, `autocorrect` and `autocomplete` apply. It never raises `Submit`.

The web renders a `<textarea>`; macOS an `NSTextView` in an `NSScrollView`; iOS a `UITextView`; Android a multi-line `EditText`; Linux a `gtk::TextView` in a `gtk::ScrolledWindow`; Windows a multi-line `EDIT` control; the terminal a wrapping editor with line-wise arrow keys. The Swift and Kotlin renderers that generated apps use draw it too. GTK and Windows show no placeholder, and HarmonyOS and the watch surfaces do not draw the component.

`aria_multiline(true)` on a `TextInput` or `PasswordInput` now fails validation, because a single-line field that calls itself multi-line misreports itself to assistive technology. Use `TextArea` instead.

- **Typed text-input hints.** `TextInput` now carries the keyboard and text-editing hints that hand-written JavaScript used to provide, so every interface stays in leptoskit's own Wasm tree: `keyboard` (Text, Email, Number, Decimal, Phone, Url, Search), `read_only`, `max_length`, `capitalize` (None, Words, Sentences, Characters), `autocorrect`, and `submit_label` (Done, Go, Next, Search, Send). Each platform maps them to its own controls: the web to `type`, `inputmode`, `readonly`, `maxlength`, `autocapitalize`, `autocorrect`, `spellcheck`, and `enterkeyhint`; iOS to `keyboardType`, `autocapitalizationType`, `autocorrectionType`, `spellCheckingType`, and `returnKeyType`; Android to `inputType`, `imeOptions`, `InputFilter.LengthFilter`, and a cleared key listener for read-only fields; GTK to `set_input_purpose`, `set_editable`, `set_max_length`, and `InputHints`; macOS to `isEditable` and delegate truncation plus the field editor's correction and spell checking; Windows to `EM_SETREADONLY` and `EM_LIMITTEXT`. `read_only` implies `aria-readonly`. The Swift and Kotlin bridge renderers and the terminal backend honour the same props. _Owner:_ Ifiok Jr. · _Introduced in:_ [6d42b06](https://github.com/leptoskit/leptoskit/commit/6d42b068273560b572fa4c7f5f4475b33a03563b)

### Fixes

- **An unknown or custom widget is an empty container on Android, in both the backend and the generated app.** It used to stop the host, which closed the app. GTK already drew one as an empty view. _Owner:_ Ifiok Jr. · _Introduced in:_ [a7b4d40](https://github.com/leptoskit/leptoskit/commit/a7b4d400154fef9c3dcc13e434766a66240c90b2)
- **Keep Android builds warning-free: omit the unused stderr fallback on Logcat, simplify pointer decisions, and account for Clippy's Android TLS false positive while retaining const initialization.** _Owner:_ Ifiok Jr. · _Introduced in:_ [7237130](https://github.com/leptoskit/leptoskit/commit/723713098b125431f6e762cbf4b6f3ebe5506a23)

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

### Breaking changes

#### leptoskit_android: the Android backend

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_android` is the Android Views backend: it applies the shared mutation stream to real Android views, with Yoga owning exact outer frames and Android owning each control's interior.

Compile `android/WidgetHost.kt` into an Android app and construct `AndroidBackend::new` on the UI thread. Every built-in primitive renders a real view; callbacks propose typed events that the app commits through `NativeSurface`.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
- **Paint italic text and per-edge borders on Linux and Android.** `border-t`, `border-l-2`, and the other edge border utilities now get the same default stroke color as a uniform `border`, and GTK and Android paint each edge instead of leaving those strokes undrawn. The `italic` utility also draws through GTK labels and Android text views, matching the web and the terminal. AppKit, UIKit, and Win32 still leave per-edge borders undrawn and italic upright. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #534](https://github.com/leptoskit/leptoskit/pull/534)
