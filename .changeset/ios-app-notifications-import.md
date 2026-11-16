---
"leptoskit_cli": patch
---

The generated iOS `App.swift` now imports `LeptosKitNotifications` under its `canImport` guard: the app delegate referenced the plugin's broker type whenever the notifications plugin was selected, but no module was imported, so the generated project failed to compile with the plugin present. A macOS test now generates the project both with and without the plugin, builds the plugin's Swift package for the iOS simulator, and typechecks the generated app sources — template, registrant, and the bridge's mobile sources — against the real modules, so the conditional import is proven both ways instead of textually.
