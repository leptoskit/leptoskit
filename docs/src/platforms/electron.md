# Electron concepts in leptoskit

Electron's main-process APIs are a useful checklist of desktop responsibilities. leptoskit does not embed Chromium or Node; it maps those responsibilities onto runtime plugins and host-owned desktop services so the same app tree can still render as native widgets.

Use this page when you know the Electron API and need the leptoskit equivalent. The [plugin support matrix](plugin-inventory.md) says what each crate implements on each platform.

## Map

| Electron concept                                               | leptoskit API                                             | Notes                                                                                                                                                                                                                                                                             |
| -------------------------------------------------------------- | --------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `app` identity / version                                       | `leptoskit_plugin_application`                            | Host packaging metadata, not the guest crate version.                                                                                                                                                                                                                             |
| `process.argv`                                                 | `leptoskit_plugin_arguments`                              | Immutable snapshot; host chooses guest disclosure.                                                                                                                                                                                                                                |
| `requestSingleInstanceLock`                                    | `leptoskit_plugin_single_instance`                        | Lock plus second-launch `notify_running` / `poll_activation`. Focus is still host-owned.                                                                                                                                                                                          |
| `app.setLoginItemSettings`                                     | `leptoskit_plugin_autostart`                              | Host-configured identity and absolute executable.                                                                                                                                                                                                                                 |
| `BrowserWindow` bounds / fullscreen / always-on-top / vibrancy | `leptoskit_plugin_window_state`, `[native.window]`        | `WindowOptions`, `WindowLevel` (`AlwaysOnTop`, `Floating`, `Status`, `ScreenSaver`), `WindowChrome` (`Frameless`, `HiddenTitleBar`), `VibrancyMaterial` (macOS vibrancy, Windows Mica/Acrylic), click-through (`ignore_mouse_events`), workspace pinning, and `WindowController`. |
| Background / menu-bar-only app (`LSUIElement`, close-to-tray)  | `leptoskit_plugin_window_state`, `native.mode`            | `native.mode = "menu-bar"` or `"background"`, `ActivationPolicy` (`Regular`, `Accessory`, `BackgroundOnly`), and `CloseBehavior` (`Quit`, `HideToTray`, `KeepRunningInBackground`, `Minimize`).                                                                                   |
| Dock / taskbar progress & bounce / flash                       | `leptoskit_plugin_window_state`, `leptoskit_plugin_badge` | `TaskbarProgress` (`None`, `Indeterminate`, `Normal`, `Paused`, `Error`), `DockAttention` (`Informational`, `Critical`), and badge counts.                                                                                                                                        |
| `Menu`                                                         | `leptoskit_plugin_menu`                                   | macOS application menus and Windows HWND window menus; Linux tray menus only. Roles and context menus remain.                                                                                                                                                                     |
| `Tray`                                                         | `leptoskit_plugin_tray`                                   | Host-owned icons and menus on macOS/Windows/Linux.                                                                                                                                                                                                                                |
| `globalShortcut`                                               | `leptoskit_plugin_global_shortcut`                        | macOS/Windows/X11; Wayland unavailable.                                                                                                                                                                                                                                           |
| `dialog`                                                       | `leptoskit_plugin_dialog`                                 | Open/save/message dialogs with a parent window.                                                                                                                                                                                                                                   |
| `Notification`                                                 | `leptoskit_plugin_notifications`                          | Immediate local posts; scheduling/actions remain.                                                                                                                                                                                                                                 |
| `clipboard`                                                    | `leptoskit_plugin_clipboard`                              | Text today; images/HTML remain.                                                                                                                                                                                                                                                   |
| `shell.openExternal` / `openPath`                              | `leptoskit_plugin_linking`, `leptoskit_plugin_shell`      | Linking opens validated URLs; shell runs scoped child processes.                                                                                                                                                                                                                  |
| `powerMonitor`                                                 | `leptoskit_plugin_power_monitor`                          | Host-pushed lifecycle events; Windows idle-time queries.                                                                                                                                                                                                                          |
| `powerSaveBlocker`                                             | `leptoskit_plugin_keep_awake`                             | Display-idle inhibition leases.                                                                                                                                                                                                                                                   |
| Custom protocols / deep links / file opens                     | `leptoskit_plugin_deep_link`                              | Inbound queue + association descriptors; packaging still declares schemes/types.                                                                                                                                                                                                  |
| Outbound `https` / `mailto` / `tel`                            | `leptoskit_plugin_linking`                                | Default policy rejects arbitrary custom schemes.                                                                                                                                                                                                                                  |
| `autoUpdater`                                                  | `leptoskit_plugin_updater`                                | Native executable hooks via a host `Backend`. Not Wasm code push.                                                                                                                                                                                                                 |
| Guest UI over-the-air updates                                  | `leptoskit_codepush`                                      | Signed Wasm bundles, separate from native installers.                                                                                                                                                                                                                             |
| `utilityProcess` / `child_process`                             | `leptoskit_plugin_shell`                                  | Bounded, scoped, cancellable children — no Node compatibility.                                                                                                                                                                                                                    |
| `safeStorage`                                                  | `leptoskit_plugin_secure_store`                           | OS credential entries, not a general encrypt facade.                                                                                                                                                                                                                              |
| Desktop capture / screenshots                                  | `leptoskit_plugin_screen_capture`                         | Mobile secure-window / observation today; desktop pixel capture remains.                                                                                                                                                                                                          |

## Always-on-top overlays and background-only desktop apps

Declare desktop window chrome and launch mode in `leptoskit.toml`:

```toml
[native]
mode = "menu-bar" # "window", "menu-bar", or "background"

[native.window]
always_on_top = true
frameless = true
transparent = true
visible_on_all_workspaces = true
close_to_tray = true
```

In a desktop host, configure floating overlays, click-through HUDs, vibrancy/Mica backdrops, close-to-tray behavior, and taskbar progress through `leptoskit_plugin_window_state`:

```rust
use leptoskit_plugin_window_state::ActivationPolicy;
use leptoskit_plugin_window_state::CloseBehavior;
use leptoskit_plugin_window_state::MemoryWindowController;
use leptoskit_plugin_window_state::TaskbarProgress;
use leptoskit_plugin_window_state::VibrancyMaterial;
use leptoskit_plugin_window_state::WindowController;
use leptoskit_plugin_window_state::WindowOptions;

let overlay = WindowOptions::floating_overlay("Quick Notes", 420.0, 260.0)
	.vibrancy(VibrancyMaterial::HudWindow)
	.ignore_mouse_events(false)
	.close_behavior(CloseBehavior::HideToTray);
overlay.validate().unwrap();

let mut controller = MemoryWindowController::new(overlay);
controller
	.set_activation_policy(ActivationPolicy::Accessory)
	.unwrap();
controller
	.set_taskbar_progress(TaskbarProgress::Normal(0.5))
	.unwrap();
assert!(!controller.handle_close_request().unwrap()); // stays alive in tray
```

## First-wave desktop lifecycle

A typical desktop host wires the pieces like this:

1. Acquire `InstanceGuard`. If that returns `None`, call `notify_running` and exit.
2. Construct `PowerMonitor`, `DeepLinks`, menus, tray, and shortcuts on the UI thread after the event loop exists.
3. Push OS open-URL / open-file / suspend / resume events into those queues as the platform delivers them.
4. Optionally register bridge plugins so the Wasm guest can drain events.
5. Supply an `updater::Backend` when the packaged app has a native update feed; use `UnavailableBackend` until then.

```rust,ignore
use leptoskit_plugin_deep_link::DeepLinks;
use leptoskit_plugin_power_monitor::Event;
use leptoskit_plugin_power_monitor::PowerMonitor;
use leptoskit_plugin_single_instance::Activation;
use leptoskit_plugin_single_instance::InstanceGuard;
use leptoskit_plugin_single_instance::notify_running;
use leptoskit_plugin_updater::UnavailableBackend;
use leptoskit_plugin_updater::Updater;

let Some(guard) = InstanceGuard::acquire(&runtime_dir, "dev.example.app")? else {
	notify_running(
		&runtime_dir,
		"dev.example.app",
		&Activation::new(std::env::args_os().next().unwrap(), std::env::args_os().skip(1).collect())?,
	)?;
	return Ok(());
};

let mut power = PowerMonitor::new();
power.push(Event::Resume);

let mut links = DeepLinks::new(["example".into()])?;
links.push_url("example://home")?;

let updater = Updater::new(UnavailableBackend);
let _ = (guard, power, links, updater);
```

## What this wave does not claim

- Writing Info.plist URL types, Windows registry verbs, or `.desktop` MimeType lines for you.
- macOS/Linux idle-time backends, logind suspend observers, or generated host event-loop wiring.
- A shipping update feed, delta encoder, or notarized installer pipeline inside `updater`.
- Desktop screenshot/recording capture, rich clipboard, notification action callbacks, or Wayland global shortcuts.
- Chromium `webContents`, BrowserView, session partitions, or Node integration — those are intentionally out of scope for a native-widget toolkit.

See [Where things stand](../readme.md#where-things-stand) and the matrix rows for each crate for the live gap list.
