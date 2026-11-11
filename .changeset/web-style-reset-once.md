---
"leptoskit_native": patch
"leptoskit_testing_library": patch
---

# Smaller server-rendered pages

Server-rendered and static pages no longer repeat leptoskit's style reset for every differently styled element. Each element's typed styles start from about sixty declarations, such as `display:flex`, `flex-shrink:0`, and zeroed margins and padding, and every distinct look carried its own copy of all of them, about a kilobyte each. `leptoskit::web::render` now writes those defaults once per stylesheet, and each style scope's sheet holds only what differs from them, often a single declaration. Elements look the same: the defaults keep the specificity and order they had, below utility classes. A list of two dozen rows that differ only in width used to carry 27 copies of the reset and now carries one, and its markup is under 9 KB. Style scopes are also 16 hex digits instead of 64, since every element carries its scope.

The root element of each render now carries `data-lk-root`, which keeps one render's defaults from resetting the elements of a render earlier in the page. `leptoskit_testing_library` leaves it out of the markup it prints, as it does `data-lk-style`.
