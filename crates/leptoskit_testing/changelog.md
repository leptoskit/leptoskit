# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-25)

### Features

- **`leptoskit debug` prints a running app's element tree, runtime errors, and render commits from the devtools hub, and `leptoskit debug open` prints the hub address for the inspector.** `leptoskit mcp` adds `runtime_errors`, `performance`, and `read_logs`, so an agent can see the tree, the errors, the logs, and the commits of an app that `leptoskit dev` is running, and can still capture a screenshot where the platform allows it. `leptoskit device install` installs an Android package, an iOS app, or checks a desktop path. `leptoskit device open` launches it. `leptoskit device sanity` does both and then injects tap, long press, pan, and pinch on the targets that can produce them: every gesture on Android and on an iOS simulator or device, tap and long press and pan on macOS, Windows, and Linux. Linux pinches through `--driver`. A mouse session cannot pinch, and the reply says so. _Owner:_ Ifiok Jr. · _Introduced in:_ [d161fdf](https://github.com/leptoskit/leptoskit/commit/d161fdf435c8f5e64cfb09f2ff53feec5619c921)

#### Arrow keys move through tabs and radio cards

_Owner:_ Ifiok Jr. · _Introduced in:_ [bc7c851](https://github.com/leptoskit/leptoskit/commit/bc7c8510366147ca4e256b1a1f3896717f4c50d4)

`leptoskit_aria::Tabs` is now one stop for the Tab key. Only the selected tab is in the Tab order; Left and Right (Up and Down after `.orientation(Orientation::Vertical)`), Home and End move to another tab, select it, and take focus with it.

**Breaking:** the selection callback moved from each tab to the list. Write `tabs.list(move |index| …)` and `tabs.tab(index, label)` where you wrote `tabs.list()` and `tabs.tab(index, label, move |index| …)`. `Tabs` no longer implements `PartialEq`.

`leptoskit_aria::RadioGroup` is new: a choice of one option among several where each option shows any content, such as a card. `RadioGroup::new(id, labels, selected).option(index, content).on_change(…)` becomes an element with `Element::from`. Options carry the `radio` role, their checked state and their position; the group is one Tab stop, and the arrow keys, Home and End move, choose and take focus along.

The arrow keys work on the web and in the terminal. macOS, iOS, Android, Windows and GTK do not deliver key presses to the app yet, so every tab and option stays reachable with Tab there, as before.

`leptoskit_testing::WidgetTester::press_key(finder, key)` presses a key on a widget and bubbles it to the nearest `on_key` handler.

#### TextArea: a field for several lines of text

_Owner:_ Ifiok Jr. · _Introduced in:_ [ce9d470](https://github.com/leptoskit/leptoskit/commit/ce9d470863cd327cdd87a36a12738c0fd356986c)

`TextArea()` is a new component, with its own `NativeKind::TextArea`, for notes, comments and messages. Enter adds a line instead of submitting, long lines wrap, and the field scrolls its own content. It is controlled like `TextInput`: `on_change` receives the proposed text with lines separated by `\n`, and `placeholder`, `read_only`, `max_length`, `capitalize`, `autocorrect` and `autocomplete` apply. It never raises `Submit`.

The web renders a `<textarea>`; macOS an `NSTextView` in an `NSScrollView`; iOS a `UITextView`; Android a multi-line `EditText`; Linux a `gtk::TextView` in a `gtk::ScrolledWindow`; Windows a multi-line `EDIT` control; the terminal a wrapping editor with line-wise arrow keys. The Swift and Kotlin renderers that generated apps use draw it too. GTK and Windows show no placeholder, and HarmonyOS and the watch surfaces do not draw the component.

`aria_multiline(true)` on a `TextInput` or `PasswordInput` now fails validation, because a single-line field that calls itself multi-line misreports itself to assistive technology. Use `TextArea` instead.

### Fixes

- **`leptoskit device open` fails when the app never takes the screen, instead of reporting success while the launcher is still in front.** A tap, long press, or pan past the display fails too: `input` exits 0 for those points and delivers nothing. An Android pinch still waits for its touchscreen to attach, but that wait is no longer part of the gesture clock, so a 300 ms pinch is 300 ms of contact with both pointers. `device list` marks an AVD as running when that emulator is already up. `leptoskit mcp http` answers `application/json`, and the screenshot tool's schema resolves. _Owner:_ Ifiok Jr. · _Introduced in:_ [a9678a5](https://github.com/leptoskit/leptoskit/commit/a9678a57ec571f2318342fd1f04d68c9666b072d)
- **An Android pinch from `leptoskit device` is two pointers again.** Two `input swipe`s each own one pointer, and on API 35 the activity received only one of them while the step still reported success. The pinch now registers one touchscreen with `uinput` and moves both fingers in the same report. _Owner:_ Ifiok Jr. · _Introduced in:_ [b969920](https://github.com/leptoskit/leptoskit/commit/b9699202035dc9c7245236c7d5e8750e816f3e2c)

#### Boot iOS for sanity checks and use a mouse on macOS

_Owner:_ Ifiok Jr. · _Introduced in:_ [c5db144](https://github.com/leptoskit/leptoskit/commit/c5db144b468b11cafed744ffb4ac59356e8488d9) · _Last updated in:_ [b4efffc](https://github.com/leptoskit/leptoskit/commit/b4efffc4eb47c86b523454bfc2508c0fcd97a1e1)

`leptoskit device sanity` on an iOS simulator now boots that simulator before `simctl install`. A shut-down simulator refuses the install. One that is already booted stays up: the boot step treats that state as success. A sanity run that does not install, and a run on a real iPhone, still do not boot.

Pointer actions sent to a macOS session use a mouse. Mac2 does not accept a touch pointer. iOS, Android, Linux, Windows, and the web still send touch.

- **Connected iPhones show up in `device list`.** `leptoskit device list` asked `devicectl` for `--json` and parsed a `devices` array at the top of that output. devicectl does not print JSON that way. It writes a file for `--json-output`, and the phones are under `result.devices`. The id an install, a launch, and an XCUITest session need is the hardware UDID, not the Core Device identifier. A plugged-in iPhone never appeared. Watches and simulators in the same file stay out of the iPhone list. _Owner:_ Ifiok Jr. · _Introduced in:_ [2ce47e0](https://github.com/leptoskit/leptoskit/commit/2ce47e01eedb1af4bc45cb05bca2d01b10c886ad)

#### Linux pointer gestures from `leptoskit device` no longer pause for about fifteen seconds when the pointer is already on the point.

`xdotool mousemove --sync` waits for a motion event that never arrives in that case, so a tap followed by a long press or a pan at the same point stalled. The move is sent without that wait.

_Owner:_ Ifiok Jr. · _Introduced in:_ [86bb058](https://github.com/leptoskit/leptoskit/commit/86bb058f3c5c948f444fe75099ec639b02e7933d)

Opening a Linux app also waits until one of its windows is visible, the process exits, or two seconds pass. Gestures used to be injected the moment the process was spawned, which is before the window is mapped, so the tap and the long press landed on the desktop.

Opening an Android app waits until that package is the focused window, then a little longer, or two seconds pass. `am start` was returning while the launcher was still focused, and the focus line itself updates before the activity accepts a tap, so the sanity tap and long press never reached the activity.

`leptoskit device boot` of an AVD that is already running fails immediately and names the emulator serial, instead of starting a second emulator and waiting out the boot deadline. If the emulator process exits before the device is online, the boot fails then, instead of blocking on `adb wait-for-device`.

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

### Breaking changes

#### leptoskit_testing: native widget tests

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_testing` runs Flutter-style widget tests over the production renderer and Yoga layout, so a screen can be tested without a device or simulator.

`WidgetTester` renders a screen, finds elements by text, label, role, or test id with Testing Library-style queries (`get_by_text`, `query_by_label`, and the rest), acts on them, and pumps the renderer a bounded number of times while waiting for a result. The `native-ui` feature goes further and drives the real app through each platform's own automation: XCUITest on iOS, UI Automator on Android, XCTest on macOS, UI Automation on Windows, and AT-SPI on Linux.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
