---
"leptoskit_config": minor
"leptoskit_plugin_window_state": minor
---

# Add desktop window overlays and background modes

- `leptoskit_plugin_window_state::desktop` adds Electron-class desktop windowing and lifecycle types: `WindowOptions` (with `floating_overlay` preset), `WindowLevel` (`Normal`, `Floating`, `AlwaysOnTop`, `Status`, `PopUpMenu`, `ScreenSaver`), `WindowChrome` (`Standard`, `Frameless`, `HiddenTitleBar`), `VibrancyMaterial` (macOS vibrancy and Windows Mica/Acrylic), `CloseBehavior` (`Quit`, `HideToTray`, `KeepRunningInBackground`, `Minimize`), `ActivationPolicy` (`Regular`, `Accessory`, `BackgroundOnly`), `TaskbarProgress`, `DockAttention`, `WindowController`, and `MemoryWindowController`.
- `leptoskit_config` adds `NativeMode::Background`, allows `menu-bar` and `background` startup modes across desktop platforms (`macos`, `windows`, `linux`), and adds the `[native.window]` configuration table (`DesktopWindowConfig`).
