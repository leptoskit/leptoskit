---
"leptoskit_plugin_menu": minor
"leptoskit_plugin_updater": minor
---

# Add menu roles, iOS menu bar, and signed updater feeds

- `leptoskit_plugin_menu` adds standard OS `MenuRole` predefined items (`Role(MenuRole::Copy)`, `About`, `Quit`, `Undo`, `Redo`, …), mutually-exclusive `Radio` items, `ContextMenu()` / `ContextMenuHost` / `MemoryContextMenuHost`, and `leptoskit_plugin_menu::ios` for iOS & iPadOS `UIMenuBuilder` menu bars (`IosMenuBar`, `IosMenuSection`, `IosMenuPlacement`) and `UIKeyCommand` hardware shortcuts.
- `leptoskit_plugin_updater::feed` adds signed release-manifest desktop auto-updating (`ReleaseManifest`, `PlatformArtifact`, `TargetPlatformId`, `UpdateChannel`, `UpdatePolicy`, `SemVer`, `IntegrityVerifier` with built-in FIPS 180-4 SHA-256 verification, `FeedTransport`, and `FeedBackend`).
