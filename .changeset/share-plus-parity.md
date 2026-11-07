---
"leptoskit_plugin_sharing": minor
"leptoskit_cli": minor
---

# Share files, a subject and a title, and learn where the user shared

`leptoskit_plugin_sharing` now covers Flutter `share_plus`'s features. A share can carry up to 32 files, optionally with text: `File::app("exports/report.pdf")` names a file in your app's files directory (the root the `filesystem` plugin writes), and `File::bytes("chart.png", bytes)` shares data from memory, each with an optional MIME type and display name. `with_subject` sets the email subject, `with_title` the sheet or chooser title, `with_anchor` where the iPad popover or macOS picker points, and `excluding` leaves iOS activities out of the sheet.

`share` now returns a `ShareResult`: `Success` with the destination the user picked (the iOS activity type, the macOS service title, or the Android component), `Dismissed`, or `Unavailable` when the platform cannot tell, as in browsers. A result never confirms delivery. On Android this replaces the old behaviour, where dismissing the chooser and choosing an app looked the same.

Native hosts check that an app file stays inside the files directory, refusing symlinks that lead out of it, and copy shared files into a private cache that the next file share clears. On Android the plugin ships its own `FileProvider` and grants read access only to the chosen app, only for those copies; it adds `androidx.core:core`. Browsers share in-memory files through Web Share Level 2 and return `Unavailable`, without a download fallback, when they cannot. Windows and Linux still have no native share sheet.

Breaking: the wire protocol is versioned and changed. The previous request (a kind and one value) and its empty reply are refused, `share` returns `ShareResult` instead of `()`, `web::share` resolves to a `ShareResult` (a browser cancellation is `Dismissed`, no longer a `Cancelled` error, and an iOS or macOS dismissal is `Dismissed` rather than a `Cancelled` error), and `Content::as_str` and `Content::is_url` are replaced by `shared_text`, `shared_url`, `shared_files`, `subject` and `title`.

The CLI reads a new `android-file-providers` plugin metadata key: it copies the plugin's paths XML into the generated resources and declares an unexported `FileProvider` in the generated Android manifest.
