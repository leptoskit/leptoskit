---
"leptoskit": minor
"leptoskit_native": minor
---

# Add ergonomic platform features and capability catalog

Expose opt-in Cargo features on `leptoskit` (`live-activities`, `home-widgets`, `app-shortcuts`, `android-intent`, `menu`, `window-state`, `updater`, plus the `mobile-surfaces` and `desktop-extras` bundles) that re-export those plugins under `leptoskit::platform::*` and forward `leptoskit/guest`.

Expand `leptoskit_native::target::Capability` with exhaustive iOS and Android OS capabilities (including both wired built-in plugins and transparent entries for OS capabilities not yet wired as native plugins), along with `Capability::ALL`, `as_str()`, `plugin_crate()`, `runtime_plugin_name()`, `cargo_feature()`, `has_builtin_plugin()`, `summary()`, `TargetPlatform::supports()`, `PlatformCapabilities::supported_capabilities()`, and `PlatformCapabilities::available_capabilities()`.
