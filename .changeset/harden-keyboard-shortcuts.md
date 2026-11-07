---
"leptoskit_plugin_global_shortcut": patch
"leptoskit_plugin_menu": patch
"leptoskit_plugin_app_shortcuts": patch
---

# Harden keyboard shortcuts and menu accelerators

Global shortcuts and menu items now share the same muda `CmdOrCtrl+…` accelerator bounds and conflict rules: chords are capped at 64 printable ASCII bytes, menus reject duplicate accelerators (including equivalent spellings after parse), and dropping a global-shortcut owner retries native unregister so failed RAII cleanup does not leave OS bindings behind. Closing the Apple or Android app-shortcuts plugin clears published launcher entries so they cannot outlive the bridge.
