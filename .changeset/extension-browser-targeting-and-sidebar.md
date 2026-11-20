---
"leptoskit_cli": minor
"leptoskit_config": minor
"leptoskit_extension": minor
---

# Add extension browser detection and full side panel API

- `leptoskit_extension::browser` adds runtime browser detection (`ExtensionBrowser` covering Chrome, Edge, Brave, Opera, Arc, Vivaldi, Firefox, and Safari; `BrowserFamily`; `BrowserInfo`; `BrowserSwitch`; and `with_browser` test scoping).
- `leptoskit_extension::side_panel` adds full Chromium `chrome.sidePanel` and Firefox `browser.sidebarAction` support: `open_target`, `open_for_tab`, `close`, `toggle`, `is_open`, `set_options` / `get_options`, `set_behavior` / `get_behavior`, `get_layout`, `set_title` / `get_title`, and `capabilities`.
- Adds `leptoskit_extension::windows`, `leptoskit_extension::offscreen`, `leptoskit_extension::omnibox`, and `leptoskit_extension::native_messaging`.
- Adds `[extension].optional_permissions` and `[extension.side_panel]` `title` / `open_at_install` options to `leptoskit_config` and generated MV3 manifests.
