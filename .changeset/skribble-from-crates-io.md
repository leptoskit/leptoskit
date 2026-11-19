---
"leptoskit_skribble": minor
---

# Publish leptoskit_skribble with leptoskit

`leptoskit_skribble` now depends on `skribble_core` and `skribble_rough` 0.1.1 from crates.io instead of a git revision of the skribble repository, so it publishes to crates.io with the rest of leptoskit and apps add it with `cargo add leptoskit_skribble`.

leptoskit's prelude has a `Theme` of its own, so a file that glob-imports both preludes names skribble's with `use leptoskit_skribble::Theme;`.
