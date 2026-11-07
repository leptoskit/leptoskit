# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

### Features

#### Generated hosts deliver shortcut launches

Android dynamic shortcuts carry a unique intent per id. Generated iOS, Android, and macOS hosts call `noteLaunch` for quick actions, launcher shortcuts, and dock-menu picks. `DockShortcutStore.plugin` and `AppShortcutsPlugin.shared` expose the registered instance for custom hosts.

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0)

### Features

#### leptoskit_plugin_app_shortcuts: launcher shortcuts

`leptoskit_plugin_app_shortcuts` publishes a bounded static shortcut list to iOS home-screen quick actions, Android `ShortcutManager`, and a macOS dock-menu store. Guests enable the `guest` feature and reuse `guest::AppShortcuts`.
