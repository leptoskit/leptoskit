# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

### Features

#### Generated hosts deliver shortcut launches

Android dynamic shortcuts carry a unique intent per id. Generated iOS, Android, and macOS hosts call `noteLaunch` for quick actions, launcher shortcuts, and dock-menu picks. `DockShortcutStore.plugin` and `AppShortcutsPlugin.shared` expose the registered instance for custom hosts.

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-13)

### Features

- **Native capability parity: badges and shortcut hosts.** Browser pages can set and clear app-icon badges through the Badging API (`web::set` / `web::clear`), and `web::get` returns the last count this adapter published. Generated iOS, Android, and macOS hosts now call `noteLaunch` when the OS activates an app shortcut: Android dynamic shortcuts carry a unique intent per id, the macOS host builds the dock menu from `DockShortcutStore`, and iOS notes home-screen quick actions. Docs match the real backends for menus, badges, and shortcuts. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #628](https://github.com/leptoskit/leptoskit/pull/628)

#### App shortcuts, icon badges, and NFC reads

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #581](https://github.com/leptoskit/leptoskit/pull/581)

Three new runtime plugins cover high-value native capabilities that were still missing:

- **`app-shortcuts`** publishes up to eight static launcher shortcuts (iOS quick actions, Android `ShortcutManager`, macOS dock-menu store) and returns the id that launched the app.
- **`badge`** sets, reads, and clears the app icon badge count on Apple and Android, with a Linux Unity/KDE D-Bus client for custom hosts.
- **`nfc`** queries hardware readiness and reads one bounded NDEF text or URI record on iOS and Android.

Each plugin ships guest clients, wire codecs, Apple/Android backends (with stubs where a platform has no hardware), docs, and tests. Windows jump-list publishing, full Siri App Intents, background NFC, and badge Badging API support remain follow-ups.

### Fixes

- **Harden keyboard shortcuts and menu accelerators.** Global shortcuts and menu items now share the same muda `CmdOrCtrl+…` accelerator bounds and conflict rules: chords are capped at 64 printable ASCII bytes, menus reject duplicate accelerators (including equivalent spellings after parse), and dropping a global-shortcut owner retries native unregister so failed RAII cleanup does not leave OS bindings behind. Closing the Apple or Android app-shortcuts plugin clears published launcher entries so they cannot outlive the bridge. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #619](https://github.com/leptoskit/leptoskit/pull/619)

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0)

### Features

#### leptoskit_plugin_app_shortcuts: launcher shortcuts

`leptoskit_plugin_app_shortcuts` publishes a bounded static shortcut list to iOS home-screen quick actions, Android `ShortcutManager`, and a macOS dock-menu store. Guests enable the `guest` feature and reuse `guest::AppShortcuts`.
