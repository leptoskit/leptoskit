---
"release": minor
---

# Generated Linux apps run runtime plugins

The generated Linux app now registers the runtime plugins in `native.plugins` that are written in Rust, as the iOS, Android, and macOS apps already did. Add the plugin crate to `linux/Cargo.toml`, list it in `leptoskit.toml`, and `leptoskit build --platform linux` installs it. Plugins that answer at once run on the GTK thread, blocking ones on worker threads, and async ones such as `http` on a background runtime. Replies reach your screens on the GTK thread without polling. Plugins that keep files use the app's own folders under `~/.local/share` and `~/.cache`, named by `native.package_id`.

Plugins with only a Swift or Kotlin implementation, and app-owned services, are still left out on Linux with a warning, as is every plugin on Windows and in the terminal.

`leptoskit_bridge_ffi::app::PluginHost` is the plugin host that makes this work, for any runner that owns its `WasmHost`. `desktop_context` gives such a runner its app folders. A Linux app created before this release keeps building without plugins; to add them, compare its `linux/src/main.rs` with a new project's.
