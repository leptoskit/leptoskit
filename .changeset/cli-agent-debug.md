---
"leptoskit_cli": minor
"leptoskit_testing": minor
---

`leptoskit debug` prints a running app's element tree, runtime errors, and render commits from the devtools hub, and `leptoskit debug open` prints the hub address for the inspector. `leptoskit mcp` adds `runtime_errors`, `performance`, and `read_logs`, so an agent can see the tree, the errors, the logs, and the commits of an app that `leptoskit dev` is running, and can still capture a screenshot where the platform allows it.

`leptoskit device install` installs an Android package, an iOS app, or checks a desktop path. `leptoskit device open` launches it. `leptoskit device sanity` does both and then injects tap, long press, pan, and pinch on the targets that can produce them: every gesture on Android and on an iOS simulator or device, tap and long press and pan on macOS, Windows, and Linux. Linux pinches through `--driver`. A mouse session cannot pinch, and the reply says so.
