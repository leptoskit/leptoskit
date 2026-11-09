---
"release": minor
---

# Flutter-style splash and icon assets

`[app.icons]` declares launcher icons the way Flutter's icons_launcher does: one default image, per-platform overrides, Android adaptive layers, a notification / status-bar icon, iOS dark and tinted appearances, and web favicon / maskable outputs. Windows gets an `.ico`, Linux gets hicolor PNGs, and free-form `[app.icons.extensions.<name>]` tables keep metadata extensible without opening unknown keys. `app.icon` still works as the simple path.

`[app.launch_screen]` (also `[app.splash]`) grows Flutter native-splash fields: branding, fullscreen, platform enable flags, an Android 12 subsection, web splash HTML/CSS (GIF-friendly), `[app.launch_screen.animated]` for Rive/GIF overlays, and `[app.launch_screen.extensions.<name>]`. `leptoskit::splash::{hold, dismiss}` keeps that overlay until the app is ready.
