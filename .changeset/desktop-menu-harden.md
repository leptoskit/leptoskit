---
leptoskit_plugin_menu: patch
---

# Harden desktop menu bars

`leptoskit_plugin_menu` now rejects duplicate accelerator chords before native allocation, exposes `DesktopMenu::set_accelerator` and `definition()`, and shares a `root_items_are_submenus` check so Windows `WindowMenu` leases match the macOS / `MenuBar` root-submenu rule. macOS attach refuses off-thread calls; Windows `translate_accelerator` stays safe when muda has no HACCEL. Docs name the remaining generated-host gaps honestly.
