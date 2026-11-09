# Windows and Linux

On Windows, your screens are drawn with Win32 common controls: `BUTTON`, `EDIT`, `STATIC`, trackbar, and progress bar windows. On Linux, they are drawn with GTK 4 widgets. Both use leptoskit's layout engine and the platform's own font measurement, and neither embeds a browser.

The [shell plugin](plugin-inventory.md) cleans up command descendants before another invocation can take their capacity slot. Windows uses a Job Object attached before execution starts and retains capacity if any member's termination cannot be confirmed. Linux uses a process group; commands that deliberately leave it need additional containment from the host, such as a cgroup. Hosts that register plugins themselves can also use `power_monitor`, `deep_link`, `updater`, and single-instance handoff; see [Electron concepts](electron.md).

[3D scenes](../guide/33-3d-scenes.md) are not drawn on Windows and Linux yet: a `Scene3d` leaves an empty space the size of the view.

[Rive vector animations](../guide/20-rive.md) use a reusable Win32 bitmap or a GTK memory texture. Generated hosts stop frame work when idle or inactive and follow Windows client-area animation settings or GTK's animation preference. Native playback rejects text, images, layout, scripts, audio, view models, and drawing features it cannot reproduce. Native builds require a C++17 compiler in addition to the platform tools.

## Build and run

Each desktop app builds on its own operating system.

<Tabs>
<Tab label="Windows">

You need the MSVC Rust toolchain and the Windows SDK.

```sh
rustup target add wasm32-unknown-unknown
leptoskit build --platform windows
```

The build prints the path of the executable. The app embeds the common-controls manifest, so controls get the current Windows look, and it lays the screen out again when the window is resized or moved to a display with a different DPI.

</Tab>
<Tab label="Linux">

You need GTK 4.6 or newer, its development files, and `pkg-config`. On Debian and Ubuntu:

```sh
sudo apt install libgtk-4-dev pkg-config
rustup target add wasm32-unknown-unknown
leptoskit build --platform linux
```

The build prints the path of the executable. The app runs in the GTK event loop and passes control events to your screens. The GTK backend also feeds pointer motion into the shared [gesture arena](../guide/38-gestures.md), the same recognizers the Android widget backend uses. The generated Linux app's guest does not receive that stream yet, so a gesture in a `leptoskit new` Linux app does not run until the host forwards samples.

</Tab>
</Tabs>

While you work, run `leptoskit dev` on that machine: it builds and opens the app, and replaces its interface in the same window each time you save. See [building and serving](../guide/04-building.md#leptoskit-dev). The reload lives in the app's own `src/main.rs`, so a project created before `dev` ran desktop apps still opens under `dev` but needs a restart to show a change. To add it, compare with a new project: copy the reload from its `src/main.rs`, and add `codepush-http` to the `leptoskit` features in `Cargo.toml`.

The `windows/` and `linux/` folders are yours: each holds a small Rust program that opens the window and runs your app, plus its `Cargo.toml` for native dependencies. `leptoskit native prepare windows` or `leptoskit native prepare linux` regenerates the build inputs without producing the executable. See [host projects](host-projects.md) for what else lives where.

## How they differ from macOS

- **Windows:** `Switch` is drawn as a checkbox. Rounded corners are cut with a window region, so their edges aren't smoothed, and borders stay rectangular inside them. `ActivityIndicator` is drawn by leptoskit rather than a system control. Accessibility, through UI Automation, has no expanded or collapsed state. A `Vector` with no background takes clicks only where it has ink; clicks on its empty parts reach whatever is underneath.
- **Linux:** live regions update their text but aren't announced, because announcements need GTK 4.14. Italic text and per-edge borders (`border-t`, `border-l-2`, …) paint through GTK the same way they do on the web.
- **Sign-in:** the generated Linux app [signs in](../guide/23-sign-in.md#native-apps) with providers in the default browser, which comes back to the app on a loopback address, and keeps the session in the desktop's Secret Service keyring, such as GNOME Keyring or KWallet. The generated Windows runner can't host plugins yet, so sign-in there answers `config`; a host that registers plugins itself can register `leptoskit_plugin_web_auth::desktop::WebAuth` on a worker thread. There are no native passkeys on Windows or Linux, so passkey calls answer `unsupported` there; people sign in with an emailed code or a provider instead.
- On both, transforms and [animations](../guide/32-animations.md) are ignored: values jump to their new state, and a removed widget disappears at once.
- On both, `Image` loads [`asset://` pictures](../guide/36-assets.md) from the `assets` folder beside the executable and local files, not URLs (Linux needs the system's WebP loader for WebP), and the generated apps don't check for [code push](../guide/05-code-push.md) updates; ship a new build to update them.
- **Menus:** the same `leptoskit_plugin_menu` builders (`MenuBar`, `Submenu`, `Command`, …) compile on every desktop target. On Windows, a host that owns an HWND can attach with `windows::WindowMenu::install` when every root entry is a submenu (the same shape as macOS / `MenuBar`), and must forward accelerators through `translate_accelerator` in its message loop. On Linux, standalone application or window menu bars are unavailable (`supports_menu_bar()` is false); menu trees still feed a KSNI tray through `leptoskit_plugin_tray`. Standard roles, radio groups, icons, and context-menu presentation are not implemented on any desktop target yet. The generated Windows and Linux runners don't host these APIs yet.

## Plugins

The generated Linux app runs [runtime plugins](plugins.md) written in Rust. Add the plugin crate to `linux/Cargo.toml` and list it in `native.plugins`, as on the other platforms:

```toml
[native]
plugins = ["os", "crypto", "store", "http"]

[native.plugin-options.http]
origins = ["https://api.example.com"]
```

The build registers each plugin it can run. Plugins that answer at once run on the GTK thread, blocking ones on a small pool of worker threads, and async ones, such as `http`, on a background runtime. Your screens get their replies on the GTK thread, without the window polling while it waits. Plugins that only have a Swift or Kotlin implementation, and app-owned services, are left out with a warning; calls to them fail with `Unavailable`. The [plugin support matrix](plugin-inventory.md) lists which plugins run on Linux.

Plugins that keep files, such as `store` and `filesystem`, keep them in the app's own folders, named by `native.package_id`: `~/.local/share/<package id>` for data and `~/.cache/<package id>` for caches, or under `$XDG_DATA_HOME` and `$XDG_CACHE_HOME` when those are set.

A Linux app created before plugins ran there keeps building, without its plugins. To add them, compare `linux/src/main.rs` with a new project's: it creates a `PluginHost`, calls the generated `plugins` function before it starts the app and again after a reload, and pumps the host whenever the app wakes.

The generated Windows and terminal apps can't host plugins yet; a build for them leaves every plugin out with a warning.

Build and test on the operating system you ship to. A build that compiles on another OS says nothing about how the widgets render or respond.
