# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-25)

### Fixes

- **Give the HarmonyOS host's line-height values named, explicit types so generated applications compile with the public API 12 ArkTS compiler.** _Owner:_ Ifiok Jr. · _Introduced in:_ [7237130](https://github.com/leptoskit/leptoskit/commit/723713098b125431f6e762cbf4b6f3ebe5506a23)
- **HarmonyOS text uses a named line-height value instead of an inline object.** ArkTS treated `{ relative?: number }` as `any` and refused to compile the page. A relative height is still a multiple of the font size, and a fixed height is still in virtual pixels. _Owner:_ Ifiok Jr. · _Introduced in:_ [f5104c6](https://github.com/leptoskit/leptoskit/commit/f5104c6f382ac2e42b512fb160280ee8d375cb9e)
- **Android native hosts link AAudio even when rustc omits it from its reported libraries.** HarmonyOS uses named border and accessibility interfaces so ArkTS can compile the host contract. _Owner:_ Ifiok Jr. · _Introduced in:_ [6919585](https://github.com/leptoskit/leptoskit/commit/691958516d206c0fc9d4f3c879e8fd3b8982d4fa)
