# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-13)

### Features

- **Builder-friendly native menus.** `leptoskit_plugin_menu` adds chained constructors (`Menu`, `MenuBar`, `Submenu`, `Command`, `Check`, `Separator`) that build the same validated definitions hosts already attach. On macOS, `ApplicationMenu::install` builds and leases the AppKit menu bar in one step; Windows gains the matching `WindowMenu::install` helper. Linux still has no standalone menu bar and exposes `linux::attach_menu_bar` as an explicit `Unavailable` stub; tray / status-item menus keep sharing `DesktopMenu` with `leptoskit_plugin_tray`. Docs cover the macOS path and the Windows/Linux parity gaps. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #575](https://github.com/leptoskit/leptoskit/pull/575)

### Fixes

- **Harden desktop menu bars.** `leptoskit_plugin_menu` now rejects duplicate accelerator chords before native allocation, exposes `DesktopMenu::set_accelerator` and `definition()`, and shares a `root_items_are_submenus` check so Windows `WindowMenu` leases match the macOS / `MenuBar` root-submenu rule. macOS attach refuses off-thread calls; Windows `translate_accelerator` stays safe when muda has no HACCEL. Docs name the remaining generated-host gaps honestly. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #617](https://github.com/leptoskit/leptoskit/pull/617)
- **Harden keyboard shortcuts and menu accelerators.** Global shortcuts and menu items now share the same muda `CmdOrCtrl+…` accelerator bounds and conflict rules: chords are capped at 64 printable ASCII bytes, menus reject duplicate accelerators (including equivalent spellings after parse), and dropping a global-shortcut owner retries native unregister so failed RAII cleanup does not leave OS bindings behind. Closing the Apple or Android app-shortcuts plugin clears published launcher entries so they cannot outlive the bridge. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #619](https://github.com/leptoskit/leptoskit/pull/619)

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

### Breaking changes

#### leptoskit_plugin_menu: native menus

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_menu` is the native-host plugin for bounded native menu definitions with explicit desktop ownership: commands, checkboxes, separators, submenus, and accelerators such as `CmdOrCtrl+S`.

Native hosts construct it explicitly on their UI thread; it is not a guest plugin and installs nothing by default. macOS attaches it as the application menu and Windows attaches it to a window the host owns; Linux has no standalone menu bar, so there it only feeds tray menus. Definitions are validated before any native allocation, and iOS, Android, and the browser only validate definitions and show no menu.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
