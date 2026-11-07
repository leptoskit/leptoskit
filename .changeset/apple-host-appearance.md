---
leptoskit_bridge_ffi: patch
---

# Generated Apple apps follow light and dark

The generated macOS and iOS apps now tell your screens when the system switches between light and dark, so `dark:` classes update without a resize. A screen that first lays out while the system is dark also starts dark; before, it stayed light until its window changed size, which a menu-bar app or a fixed-size window never does.
