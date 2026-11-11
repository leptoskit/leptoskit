---
"leptoskit_bridge_ffi": patch
---

# Android text wraps to the lines it needs

Android measured any plain one-paragraph text with `BoringLayout`, which always lays out exactly one line, even when the text was wider than its box. Layout then gave a paragraph one line's height, and the `TextView` drew only its first line: every paragraph of Reapers on a Solana Seeker was clipped. Text now takes `BoringLayout` only when it fits, and wraps through `StaticLayout` otherwise, as `TextView` does. The narrowest box a text can take is its widest word, with one word to a line.

Text views now also break lines with the measurer's strategy (simple, no hyphenation) instead of Android's balanced default, so a view never needs a line more than its frame was measured to.
