---
"leptoskit_lint": patch
---

# `lk lint` knows the plugin surface tags

`lk lint` now counts the plugin surfaces `Video`, `Camera`, `Map`, and `LivePhoto` among leptoskit's built-in components, so the markup-style rule checks trees that use them like any other built-in tag.
