# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-13)

### Fixes

- **Builder lint keeps the SSR shell; apps pin Rust.** `markup = "builder"` no longer reports ordinary HTML tags in the SSR document `shell` (`<html>`, `<head>`, …)—only leptoskit component trees. The agent skill and linting guide say the same. Deploy generation names `--sdk-path` when a path dependency sits outside the workspace. (New apps already pin `rust-toolchain.toml` from the matching scaffold work.) _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #650](https://github.com/leptoskit/leptoskit/pull/650)
- **`lk lint` knows the plugin surface tags.** `lk lint` now counts the plugin surfaces `Video`, `Camera`, `Map`, and `LivePhoto` among leptoskit's built-in components, so the markup-style rule checks trees that use them like any other built-in tag. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #752](https://github.com/leptoskit/leptoskit/pull/752)

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

### Breaking changes

#### leptoskit_lint: project conventions

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_lint` contains the rules behind `leptoskit lint`: components carry `#[component]`, and component trees follow the markup style `leptoskit.toml` selects.

rustc and clippy check Rust; these rules check the conventions a leptoskit project relies on, with diagnostics that name the file, line, and fix.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
