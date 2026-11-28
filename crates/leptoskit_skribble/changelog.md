# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-28)

### Features

#### Publish leptoskit_skribble with leptoskit

_Owner:_ Ifiok Jr. · _Introduced in:_ [0be1926](https://github.com/leptoskit/leptoskit/commit/0be1926c08f92341d557ce03a90226d286178c93)

`leptoskit_skribble` now depends on `skribble_core` and `skribble_rough` 0.1.1 from crates.io instead of a git revision of the skribble repository, so it publishes to crates.io with the rest of leptoskit and apps add it with `cargo add leptoskit_skribble`.

leptoskit's prelude has a `Theme` of its own, so a file that glob-imports both preludes names skribble's with `use leptoskit_skribble::Theme;`.
