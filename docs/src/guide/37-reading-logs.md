# Reading the runtime's log

Your app shows a blank screen on a tester's Android phone, and works everywhere else. DevTools needs `leptoskit dev` running, and the phone isn't on your desk. What you need is the phone's own log: leptoskit writes to it whenever something fails, saying which plugin, call, element, or bytes the failure was about, so a log from the tester is enough to find the cause.

leptoskit sets this up when your app starts. You write no code for it.

## Where the records go

- **Android, Wear OS, and Quest**: logcat. Run `adb logcat -v brief` and look for the `leptoskit/` tags, or filter Android Studio's Logcat on `tag:leptoskit/`.
- **iOS and macOS**: the unified log, under the subsystem `dev.leptoskit`. Run `log stream --level info --predicate 'subsystem == "dev.leptoskit"'`, or filter Console.app on the subsystem. For the iOS Simulator, put `xcrun simctl spawn booted` before `log stream`.
- **Windows and Linux**: the app's standard error.
- **Web**: the browser's console.
- **HarmonyOS** (experimental): standard error, for now; its own log, hilog, is not used yet.

Each record is tagged with the part of leptoskit it comes from. The tag is the logcat tag on Android and the `os_log` category on Apple platforms:

| Tag                       | What it says                                                                                                        |
| ------------------------- | ------------------------------------------------------------------------------------------------------------------- |
| `leptoskit/bridge`        | The app was created or could not be, the interface started or stopped and why, and when the app paused and resumed. |
| `leptoskit/plugin/<name>` | One plugin: that it registered, or why it couldn't, and each call that failed.                                      |
| `leptoskit/render`        | The first screen sent to the native views, and props an element sent that the platform couldn't read.               |
| `leptoskit/wasm`          | Your interface's code stopped: which of its functions was running, and which call into the platform it was making.  |
| `leptoskit/gpu`           | Drawing surfaces for 3D scenes and the GPU behind them: created, resized, lost, and restored.                       |
| `leptoskit/panic`         | A panic on any thread, with where it happened and a backtrace.                                                      |

Libraries in the native runtime that log, such as wgpu, appear under their own crate name.

## What a failure looks like

A plugin that refuses a call names the plugin, the call, the method, and the error:

```text
W leptoskit/plugin/camera: call #12 to `camera` method 3 failed with PermissionDenied: permission denied
```

A Kotlin or Swift plugin that throws is logged with its whole error: each cause and, on Android, each stack.

An element whose props the platform can't read names the element, the prop, and the bytes around the mistake, cut to at most 512 bytes:

```text
E leptoskit/render: node_set_props: node 7 (text) sent props that do not decode at `style.flex_grow`: invalid type: map, expected f32 at line 1 column 37; bytes 0..50 of 50: {"text":"Score","style":{"flex_grow"<<HERE>>:{"value":1}}}
```

When your interface stops, every build logs why at `error`, even when a debug build also shows the message on screen:

```text
E leptoskit/bridge: host call failed: leptoskit_main: wasm `unreachable` instruction executed
```

## How much is logged

A debug build logs `info` and above: failures, and the app's lifecycle, so a failure's record follows the steps that led to it. A release build logs `warn` and above. Set another level for every platform in `leptoskit.toml`:

```toml
[native]
log-level = "debug"
```

| `log-level` | What is logged                                                                                        |
| ----------- | ----------------------------------------------------------------------------------------------------- |
| `off`       | Nothing.                                                                                              |
| `error`     | Failures.                                                                                             |
| `warn`      | Failures, and what leptoskit worked around: a missing image in a release build, a plugin left out.    |
| `info`      | Also the lifecycle: the app created, each plugin registered, the first screen, each pause and resume. |
| `debug`     | Also routine detail, such as surface resizes and cancelled calls.                                     |
| `trace`     | Everything.                                                                                           |

The level is fixed when the app is built, so changing it needs a rebuild.

## Nothing leaves the device

These records are written to the platform's own log and nowhere else. leptoskit sends no telemetry: no crash reports, no analytics, no network request of any kind for logging. A record leaves the device only when someone with access to it, such as a tester running `adb logcat`, copies it out.

Records can include what your app sent to a plugin or an element, in the excerpt of a failed payload. Keep secrets out of element props, as you would out of any log.
