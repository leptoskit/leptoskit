# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

### Features

#### Browser Badging API

Browser pages without a native host call `web::set` / `web::clear` over `navigator.setAppBadge` / `clearAppBadge` when the browser exposes them. `web::get` returns the last count this adapter published, because the Badging API has no read-back.

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-25)

### Features

- **Native capability parity: badges and shortcut hosts.** Browser pages can set and clear app-icon badges through the Badging API (`web::set` / `web::clear`), and `web::get` returns the last count this adapter published. Generated iOS, Android, and macOS hosts now call `noteLaunch` when the OS activates an app shortcut: Android dynamic shortcuts carry a unique intent per id, the macOS host builds the dock menu from `DockShortcutStore`, and iOS notes home-screen quick actions. Docs match the real backends for menus, badges, and shortcuts. _Owner:_ Ifiok Jr. · _Introduced in:_ [0c66c58](https://github.com/leptoskit/leptoskit/commit/0c66c58142338c3399198783a442bc777f04476c)

#### App shortcuts, icon badges, and NFC reads

_Owner:_ Ifiok Jr. · _Introduced in:_ [888ff04](https://github.com/leptoskit/leptoskit/commit/888ff04cf9d89bc604a176efa427dd3c3bf4251e)

Three new runtime plugins cover high-value native capabilities that were still missing:

- **`app-shortcuts`** publishes up to eight static launcher shortcuts (iOS quick actions, Android `ShortcutManager`, macOS dock-menu store) and returns the id that launched the app.
- **`badge`** sets, reads, and clears the app icon badge count on Apple and Android, with a Linux Unity/KDE D-Bus client for custom hosts.
- **`nfc`** queries hardware readiness and reads one bounded NDEF text or URI record on iOS and Android.

Each plugin ships guest clients, wire codecs, Apple/Android backends (with stubs where a platform has no hardware), docs, and tests. Windows jump-list publishing, full Siri App Intents, background NFC, and badge Badging API support remain follow-ups.

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0)

### Features

#### leptoskit_plugin_badge: app icon badges

`leptoskit_plugin_badge` sets, reads, and clears app icon badge counts on Apple platforms and Android, with a Linux Unity/KDE D-Bus client for custom hosts. Guests enable the `guest` feature and reuse `guest::Badge`.
