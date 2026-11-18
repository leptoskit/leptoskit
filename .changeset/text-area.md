---
"leptoskit_native": minor
"leptoskit_appkit": minor
"leptoskit_uikit": minor
"leptoskit_android": minor
"leptoskit_gtk": minor
"leptoskit_windows": minor
"leptoskit_tui": minor
"leptoskit_bridge_ffi": minor
"leptoskit_testing": minor
"leptoskit_watch": minor
"leptoskit_lint": minor
"leptoskit_docs": minor
"leptoskit": minor
---

# TextArea: a field for several lines of text

`TextArea()` is a new component, with its own `NativeKind::TextArea`, for notes, comments and messages. Enter adds a line instead of submitting, long lines wrap, and the field scrolls its own content. It is controlled like `TextInput`: `on_change` receives the proposed text with lines separated by `\n`, and `placeholder`, `read_only`, `max_length`, `capitalize`, `autocorrect` and `autocomplete` apply. It never raises `Submit`.

The web renders a `<textarea>`; macOS an `NSTextView` in an `NSScrollView`; iOS a `UITextView`; Android a multi-line `EditText`; Linux a `gtk::TextView` in a `gtk::ScrolledWindow`; Windows a multi-line `EDIT` control; the terminal a wrapping editor with line-wise arrow keys. The Swift and Kotlin renderers that generated apps use draw it too. GTK and Windows show no placeholder, and HarmonyOS and the watch surfaces do not draw the component.

`aria_multiline(true)` on a `TextInput` or `PasswordInput` now fails validation, because a single-line field that calls itself multi-line misreports itself to assistive technology. Use `TextArea` instead.
