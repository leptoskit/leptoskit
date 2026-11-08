---
"release": patch
---

# The observatory and postcard examples open on Linux

`cargo run -p observatory_example` and `cargo run -p postcard_example` now open a real GTK 4 window on Linux, as they already did with AppKit on macOS. Their `snapshot` command renders the window with GTK's own renderer to a PNG. Elsewhere they still print the screen's accessibility tree instead.
