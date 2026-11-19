---
"leptoskit_native": minor
"leptoskit_android": minor
"leptoskit": minor
---

# A field's label focuses it on the web

A `Text` that names a field through `aria_labelled_by` now renders as `<label for="…">` on the web, so a click on the label focuses a `TextInput`, `PasswordInput`, `TextArea` or `Slider` and toggles a `Checkbox`, `Switch` or `Radio`, with no handler and no script. It applies when the field has an `id`, is named by exactly one plain `Text`, and no other field is named after the same text. `leptoskit_aria::Field` and `leptoskit::form` already build this shape. The markup for such a text changes from `<span>` to `<label>`; its styles and its `id` stay the same.

On Android, the label's view gets `labelFor`, so TalkBack knows which field it names. Other platforms are unchanged: clicking a label is not a convention there, and the field already carries the label's text as its name.
