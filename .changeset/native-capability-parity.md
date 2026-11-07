---
"leptoskit_plugin_badge": minor
"leptoskit_plugin_app_shortcuts": minor
"leptoskit_cli": patch
---

# Native capability parity: badges and shortcut hosts

Browser pages can set and clear app-icon badges through the Badging API (`web::set` / `web::clear`), and `web::get` returns the last count this adapter published. Generated iOS, Android, and macOS hosts now call `noteLaunch` when the OS activates an app shortcut: Android dynamic shortcuts carry a unique intent per id, the macOS host builds the dock menu from `DockShortcutStore`, and iOS notes home-screen quick actions. Docs match the real backends for menus, badges, and shortcuts.
