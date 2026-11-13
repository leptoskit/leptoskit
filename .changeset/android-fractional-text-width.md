---
"leptoskit_bridge_ffi": patch
---

Respect fractional Android text width constraints

Android text measurements now stay within the requested logical width after pixel rounding. This fixes labels reporting a width larger than their layout constraint by part of a physical pixel.
