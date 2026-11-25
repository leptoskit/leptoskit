# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-25)

### Features

#### Android alarms that run app code

_Owner:_ Ifiok Jr. · _Introduced in:_ [24d6fbc](https://github.com/leptoskit/leptoskit/commit/24d6fbcae5293e9d397cd1d06938ce3ced7f6b54)

`leptoskit_plugin_android_alarm_manager` ports Flutter's `android_alarm_manager_plus`. Guests schedule one-shot and repeating Android `AlarmManager` alarms by integer ID, with exact, wakeup, allow-while-idle, alarm-clock and reschedule-on-reboot options, check and request the Android 12+ exact-alarm permission (or fall back to inexact timing), cancel and list alarms, and receive fired alarms through a bounded on-device queue that survives process death: `next()` waits for one, `drain()` takes the ones that fired while the app wasn't running. An alarm marked `headless` starts the app's guest without a window when no guest is running, so `handle_launch` can run app code for it within an eight-second budget. iOS, macOS, Windows, Linux, HarmonyOS and the browser return `Unavailable`.

The plugin declares its alarm and boot receivers with the `android-receivers` runtime metadata, which the host-project docs now describe. The Android host gains `NativeHeadlessGuest`, which runs the bundled guest with no window for a receiver or job and discards what it draws.

#### Android Back goes back a route

_Owner:_ Ifiok Jr. · _Introduced in:_ [a93214e](https://github.com/leptoskit/leptoskit/commit/a93214edc8e5f6c7999c628285093f15cf188639)

The Back gesture and Back button in an Android app now go back one route through the file router, and leave the app only from the first route. Before, Back always left the app: Android 16 delivers Back to an app that targets it only through an `OnBackInvokedCallback`, which the generated activity never registered, and on every Android version the host's question to the router went nowhere.

- `leptoskit_wasm`: `WasmHost::system_back` offers Back to the guest's optional `leptoskit_system_back` export, then flushes and commits what changed. A guest without the export answers `EventOutcome::Unhandled`, and the platform leaves.
- `leptoskit_bridge_ffi`: `lk_guest_system_back` offers Back on the guest thread, and `lk_guest_take_system_back` takes the answers in order. `LeptosKitApplicationView.offerSystemBack` now takes a callback, `(Boolean) -> Unit`, that runs on the main thread with `true` when the router went back.
- `leptoskit_cli`: the generated `MainActivity.kt` registers an `OnBackInvokedCallback` on Android 13 and later and keeps `onBackPressed` for earlier versions; the generated manifest sets `android:enableOnBackInvokedCallback="true"`.

Apps scaffolded before this keep leaving on Back until they copy the new `MainActivity.kt` Back code and the manifest attribute from a new project; [the Android page](https://leptoskit.dev/docs/platforms/android.html#the-back-gesture) lists them.

#### CanvistEditor: a place for a canvist text editor

_Owner:_ Ifiok Jr. · _Introduced in:_ [5c71502](https://github.com/leptoskit/leptoskit/commit/5c71502b830239c1bcbf9301497142aa2003aa9a)

`CanvistEditor(id)` is a new element, with its own `NativeKind::CanvistEditor`, that marks where a [canvist](https://github.com/canvist/canvist) text editor shows. The app owns the editor through a state handle and gives the element the handle's id, so the document never travels through props. `placeholder`, `read_only`, and an accessible label work as on a `TextArea`, and the element must have a name like any text field.

This change adds only the place. On the web the element renders an empty `<div data-lk-canvist-editor="id">` that the editor's host looks for, and a server-rendered page names the browser's own editor on it when it hydrates; the host is the experimental, unpublished `leptoskit_canvist` crate, and the `leptoskit` component that joins the two comes next. macOS, iOS, Android, Linux, and Windows show an empty view, the terminal a dim block, and the watch surfaces refuse the element. The Swift and Kotlin renderers that generated apps use show an empty view too; the Swift ones used to stop on a kind they did not know.

The wire name is `canvist_editor`, appended last in the wire kind table, so existing codes keep their meaning.

#### Move keyboard focus once with `FocusRequest`

_Owner:_ Ifiok Jr. · _Introduced in:_ [f9752b8](https://github.com/leptoskit/leptoskit/commit/f9752b87d178230e24d89cb8df527e6f7e9518b6)

`FocusRequest::new()` makes a `Copy` handle. Give it to a widget with `focus_on(request)` and call `request()` from any handler: focus moves to that widget once, after the screen next updates, and later updates leave focus where the person has since put it. Use it to land in the code field after a code is sent, to return to the button that opened a menu, or to put a screen reader on an error summary. `focused(true)` is unchanged and still suits a widget that should always hold focus.

It works on every backend: `focus()` on the web (text and containers get `tabindex="-1"` so they can take it), the focus ring in the terminal, `grab_focus` on GTK, first responder on macOS and iOS, `SetFocus` on Windows and `requestFocus` on Android. Where a platform has no keyboard focus for a widget, iOS and Android move the VoiceOver or TalkBack cursor instead, and macOS and Windows leave focus alone. On macOS, Windows, iOS and Android the widget does not hear a `Focus` event for a requested move.

Renderer authors: `NativeProps` gains `focus_request`. `NativeSurface` sends it only with the commit that makes a new request, so a backend focuses `MutationList::focus_target()` after applying a batch and keeps no state.

#### Versioned native capability contracts

_Owner:_ Ifiok Jr. · _Introduced in:_ [442b38b](https://github.com/leptoskit/leptoskit/commit/442b38b81352231ebb9810f646247bd548ed353c)

Native builds now resolve a typed host capability descriptor from the destination flavour, the plugin registrations they actually generated, and the non-secret plugin options. `native bundle` writes `capabilities.json` beside the bundle with a stable SHA-256 digest, and the mobile host embeds the same descriptor in `codepush.json`.

`[native]` gains `required-plugins`, `optional-plugins`, `required-capabilities`, and `optional-capabilities`. A required plugin or capability the selected host cannot run fails the build before packaging, regardless of `missing-plugins`; an optional omission warns with the platform and reason. Existing `plugins` and `missing-plugins` keep their behavior when no requirement lists are set, and a capability declaration never grants access.

Signed releases carry versioned required/optional requirements in `runtime.json`. The installed host checks them against its own descriptor before staging or activating an update; unsupported, unknown, or experimental-without-opt-in requirements are refused. Releases built before this format still pass the legacy ABI, shell, and plugin checks and are reported as legacy, never as verified-compatible. Unknown contract versions and conflicting or duplicate entries are rejected.

Guest requirements bind the bridge protocol as well as the rendering ABI. Explicitly optional plugins no longer enter the legacy required-plugin list. Watch preparation checks requirements before writing its stubs and emits a descriptor; standalone bundles place host evidence beside the output directory.

Updates whose older-style plugin grants have no available implementation report `incomplete` while preserving the existing missing-plugin fallback. Explicit required entries still reject unavailable plugins; explicit optional entries may be omitted. A versioned host descriptor must agree with its own grant list before it can check a release.

Generated iOS and Android update controls show whether host requirements were checked, plugin availability remains incomplete, or only legacy checks ran. A missing report from an older runtime is shown as unreported. None of these states claims that the updated app passed device tests.

The descriptor also names `host.timer` where the generated host enables the reserved `leptoskit.timer` plugin, so `required-capabilities = ["host.timer"]` fails before packaging on hosts that run no guest and is checked by code push on older store builds.

#### Android presents Modal as a dialog and Popover as a popup

_Owner:_ Ifiok Jr. · _Introduced in:_ [f740aa1](https://github.com/leptoskit/leptoskit/commit/f740aa1e5c264305477d17f75c66296993833da2)

On Android a `Modal` is now a dialog in a window of its own: it dims the screen, keeps touches, the keyboard and TalkBack to itself, and is as large as what the modal holds. A `Popover` is a popup window placed beside its anchor where `placement` says. The Back gesture, Escape and a touch outside ask to dismiss. A dialog stays until your `open` signal closes it. The Back gesture closes a popup window before your app hears of it, so a popover your app keeps open is shown again. `initial_focus` moves focus to the named element when the overlay opens. This holds for generated Android apps (the Kotlin renderer) and for Rust hosts on `AndroidBackend`.

What differs from the web: the modal's own background is not drawn, because the dialog dims the screen, and a dialog needs an Activity behind the view the tree is mounted in.

`WidgetHost` gains `overlayShown`, `overlayDialog` and `overlayPopup`. Generated Android projects ship one more Kotlin file, `NativeOverlayView.kt`.

This also restores two things the Android host needed to compile: an import in `WidgetHost.kt` and the `Dismiss` case in the probe.

#### iOS presents Modal as a sheet and Popover as a popover

_Owner:_ Ifiok Jr. · _Introduced in:_ [9055533](https://github.com/leptoskit/leptoskit/commit/905553347de313983dc7fadd12b419d286dabe38)

On iOS a `Modal` is now a form sheet: it dims the screen, keeps touches and VoiceOver to itself, and gives focus back when it closes. A `Popover` is a system popover pointing at its anchor, on an iPhone as well as an iPad. A swipe down on the sheet, a press outside the popover, Escape on a hardware keyboard and the VoiceOver escape gesture all ask to dismiss, and the overlay stays until your `open` signal closes it. `initial_focus` raises the keyboard for a field and moves the VoiceOver cursor for anything else. This holds for generated iOS apps (the Swift renderer) and for Rust hosts on `UIKitBackend`.

What differs from the web: a sheet is as large as what the modal holds on an iPad and as wide as the screen on an iPhone, and the modal's own background is not drawn; and a popover is centred on its anchor, so `placement` picks above or below only.

`UIKitBackend` gains `overlay_shown`, `overlay_controller`, `perform_dismiss` and `present_overlays`. A controller that mounts the tree itself calls `present_overlays` from `viewDidAppear:`; `UIKitHost` does so already. Generated iOS projects ship one more Swift file, `MobileOverlayView.swift`.

#### macOS shows Modal as a sheet and Popover as NSPopover

_Owner:_ Ifiok Jr. · _Introduced in:_ [4cba9d4](https://github.com/leptoskit/leptoskit/commit/4cba9d4cc9bafbfdd824dfca3ce310b3ad3cda0d)

On macOS a `Modal` is now a sheet on the app's window: it dims the window, keeps clicks and the keyboard to itself, and gives focus back when it closes. A `Popover` is an `NSPopover` pointing at its anchor. Escape asks to dismiss either one, and a press outside asks to dismiss a popover. `initial_focus` moves focus into the overlay when it opens. This holds for generated macOS apps (the Swift renderer) and for Rust hosts on `AppKitBackend`.

What differs from the web: a sheet is as large as what the modal holds and the modal's own background is not drawn; a sheet has no outside to press; and a popover is centred on its anchor, so `placement` picks above or below only.

`AppKitBackend` gains `overlay_shown`, `overlay_window` and `perform_dismiss`, and `open_window` shows overlays that were already open. Generated macOS projects ship one more Swift file, `MacOverlayView.swift`.

#### Modal and Popover show content above the screen

_Owner:_ Ifiok Jr. · _Introduced in:_ [08a70cb](https://github.com/leptoskit/leptoskit/commit/08a70cb0dbd8a153b2b8eb285099920fb6d2252c)

`Modal(open)` blocks the screen until it is answered, and `Popover(anchor, open)` hangs from the element with that `id`. Both stay in the tree while closed. `.on_dismiss(...)` hears Escape and a press outside as a request to close, and the overlay closes when the next build says so. `.initial_focus(id)` names what takes focus on opening, and `.placement(Placement::BelowEnd)` picks a popover's side. In `view!` they are `<Modal open=…>` and `<Popover anchor=… open=…>`.

On the web a `Modal` is a `<dialog>` opened with `showModal()` and a `Popover` uses the `popover` attribute, so the browser supplies the top layer, the inert page, the focus trap and focus restore. The terminal draws overlays last, keeps Tab inside an open modal, and gives focus back on close.

macOS, iOS, Android, Linux and Windows draw an overlay as a layer in the right place for now. Nothing asks to dismiss it there, focus is not moved or kept inside, and a parent that clips or scrolls cuts it off.

For backend authors: `NativeKind` gains `Modal` and `Popover`, `NativeEvent` gains `Dismiss`, and `NativeProps` gains `overlay`. A backend that returns `true` from the new `NativeBackend::presents_overlays` is told the real kinds and given open overlays' frames in viewport coordinates. Every other backend is given a `View` with a frame relative to its parent. The guest event ABI gains kind 9 (dismiss), and the bridge event codes gain 10.

- **Add `leptoskit_plugin_live_photo` with create/start/stop/status/release and a `LivePhoto` surface bound by session id (`PHLivePhotoView`).** Android and browser remain Unavailable; editing remains later scope. _Owner:_ Ifiok Jr. · _Introduced in:_ [13da9d6](https://github.com/leptoskit/leptoskit/commit/13da9d61f3252c056816f1da31516fd4e5040f9c)
- **Add `leptoskit_plugin_maps` with create/camera/markers/status/release and a `Map` surface bound by session id (MapKit / Google Maps).** User location and shapes remain later scope. _Owner:_ Ifiok Jr. · _Introduced in:_ [10ddb6f](https://github.com/leptoskit/leptoskit/commit/10ddb6fef86dc107d73054a9e2f2542f6e922da8)

#### TextArea: a field for several lines of text

_Owner:_ Ifiok Jr. · _Introduced in:_ [ce9d470](https://github.com/leptoskit/leptoskit/commit/ce9d470863cd327cdd87a36a12738c0fd356986c)

`TextArea()` is a new component, with its own `NativeKind::TextArea`, for notes, comments and messages. Enter adds a line instead of submitting, long lines wrap, and the field scrolls its own content. It is controlled like `TextInput`: `on_change` receives the proposed text with lines separated by `\n`, and `placeholder`, `read_only`, `max_length`, `capitalize`, `autocorrect` and `autocomplete` apply. It never raises `Submit`.

The web renders a `<textarea>`; macOS an `NSTextView` in an `NSScrollView`; iOS a `UITextView`; Android a multi-line `EditText`; Linux a `gtk::TextView` in a `gtk::ScrolledWindow`; Windows a multi-line `EDIT` control; the terminal a wrapping editor with line-wise arrow keys. The Swift and Kotlin renderers that generated apps use draw it too. GTK and Windows show no placeholder, and HarmonyOS and the watch surfaces do not draw the component.

`aria_multiline(true)` on a `TextInput` or `PasswordInput` now fails validation, because a single-line field that calls itself multi-line misreports itself to assistive technology. Use `TextArea` instead.

- **Typed text-input hints.** `TextInput` now carries the keyboard and text-editing hints that hand-written JavaScript used to provide, so every interface stays in leptoskit's own Wasm tree: `keyboard` (Text, Email, Number, Decimal, Phone, Url, Search), `read_only`, `max_length`, `capitalize` (None, Words, Sentences, Characters), `autocorrect`, and `submit_label` (Done, Go, Next, Search, Send). Each platform maps them to its own controls: the web to `type`, `inputmode`, `readonly`, `maxlength`, `autocapitalize`, `autocorrect`, `spellcheck`, and `enterkeyhint`; iOS to `keyboardType`, `autocapitalizationType`, `autocorrectionType`, `spellCheckingType`, and `returnKeyType`; Android to `inputType`, `imeOptions`, `InputFilter.LengthFilter`, and a cleared key listener for read-only fields; GTK to `set_input_purpose`, `set_editable`, `set_max_length`, and `InputHints`; macOS to `isEditable` and delegate truncation plus the field editor's correction and spell checking; Windows to `EM_SETREADONLY` and `EM_LIMITTEXT`. `read_only` implies `aria-readonly`. The Swift and Kotlin bridge renderers and the terminal backend honour the same props. _Owner:_ Ifiok Jr. · _Introduced in:_ [6d42b06](https://github.com/leptoskit/leptoskit/commit/6d42b068273560b572fa4c7f5f4475b33a03563b)

#### 3D scenes over the live camera on iOS and Android

_Owner:_ Ifiok Jr. · _Introduced in:_ [c215d24](https://github.com/leptoskit/leptoskit/commit/c215d24d84051613fef7bf2171094a72208dabce)

A `Scene3d` whose backdrop is `Backdrop::Camera` now draws over the camera while an `xr` session runs on a phone. The generated iOS and Android apps hand the scene the session's newest frame: the camera picture behind the models, turned upright for the screen's rotation and cropped to fill the view, the session's own viewpoint in place of the scene's camera, and the room's measured depth hiding the parts of the scene behind real things. Start the session with `Features::CAMERA_IMAGE` (and `Features::DEPTH` for occlusion). Without a session the scene draws as before, transparent behind its models.

Android phones without ARCore, such as the Solana Seeker, now show the back camera in `Orientation` sessions too, through Camera2, with the rotation sensor turning the view and the camera's own field of view keeping the scene to the picture's scale. Such a session leaves the picture out, rather than failing, until the user allows the camera, unless it requires `CAMERA_IMAGE`.

On Android a `Scene3d` showing the camera moves below the window's own views, so the interface laid over it stays visible.

For hosts that draw scenes themselves, `leptoskit_3d::host::Hub::set_camera_feed` takes the picture a camera backdrop draws over, `View::set_rotation` the interface's rotation, and the bridge adds `lk_scene3d_view_set_rotation` and `lk_scene3d_view_shows_camera`. `leptoskit_xr`'s new `scene3d` feature makes the plugin's sessions that feed, and `leptoskit build` turns it on, with any other async plugin's `scene3d` feature, when the app shows 3D scenes. `leptoskit_xr::host::install_depth_estimator` (or `XrRuntime::set_depth_estimator`) takes a `DepthEstimator`, which turns small CPU copies of the camera picture (`CameraFrame`, with the camera's `Intrinsics`) into depth for phones that measure none.

#### Immersive sessions from Meta Quest panel apps

_Owner:_ Ifiok Jr. · _Introduced in:_ [e897f31](https://github.com/leptoskit/leptoskit/commit/e897f31c27dbc9cb0c39378f56f705e9ef8e83f8)

A Quest app now draws its 3D scene into the headset. Start an `ImmersiveVr` or `ImmersiveAr` session and choose the scene with the new `Session::present(Some(scene_id))`: each eye sees the scene from the player's eye, over passthrough in `ImmersiveAr`. An app built with `[native.quest] immersive = false`, a 2D panel, opens an immersive activity over the panel for the session and comes back to the panel in Horizon Home when the session ends, while its code keeps running and receiving frames.

`leptoskit build --platform quest` declares that activity in every Quest app, marks a panel app's launcher with Horizon OS's 2D category and targets Horizon OS v69, and turns on `leptoskit_xr`'s new `quest` feature (or any async plugin's `quest` feature). For hosts that draw scenes themselves, `leptoskit_3d::host::Hub::draw_scene` draws a scene on another device, such as a headset's, and `Hub::forget_device` moves scenes off it again.

None of this has run on a headset yet.

### Fixes

- **An unknown or custom widget is an empty container on Android, in both the backend and the generated app.** It used to stop the host, which closed the app. GTK already drew one as an empty view. _Owner:_ Ifiok Jr. · _Introduced in:_ [a7b4d40](https://github.com/leptoskit/leptoskit/commit/a7b4d400154fef9c3dcc13e434766a66240c90b2)
- **A drawing-scaled vector stroke follows the layout frame, not the view after it has been snapped out to whole pixels.** At 420 dpi a 10dp edge becomes 27px, and the stroke was a little wider than the layout asked for. `parse_and_launch` no longer opens an `intent:` URI that has no scheme. API 35's Settings activity is browsable, so an action-only Settings URI used to start it. The activity it does open still has to declare `CATEGORY_BROWSABLE` on the filter that matched. _Owner:_ Ifiok Jr. · _Introduced in:_ [8128505](https://github.com/leptoskit/leptoskit/commit/81285051e628dada5bf9819978e4d608c53b1fb5)
- **Respect fractional Android text width constraints.** Android text measurements now stay within the requested logical width after pixel rounding. This fixes labels reporting a width larger than their layout constraint by part of a physical pixel. _Owner:_ Ifiok Jr. · _Introduced in:_ [d9a66d1](https://github.com/leptoskit/leptoskit/commit/d9a66d1e73141c14be24d3e57bcd4f1cf600bdca)
- **Composite Android 3D scenes with the interface.** Android `Scene3d` views draw into transparent `TextureView` surfaces in the normal view hierarchy. Page backgrounds no longer cover the models, and controls remain above the scene and its camera picture. Changing between the camera and a transparent backdrop no longer removes and recreates the view's GPU surface. Rust releases the native window before Android releases its surface texture. _Owner:_ Ifiok Jr. · _Introduced in:_ [7237130](https://github.com/leptoskit/leptoskit/commit/723713098b125431f6e762cbf4b6f3ebe5506a23)
- **Keep Android builds warning-free: omit the unused stderr fallback on Logcat, simplify pointer decisions, and account for Clippy's Android TLS false positive while retaining const initialization.** _Owner:_ Ifiok Jr. · _Introduced in:_ [7237130](https://github.com/leptoskit/leptoskit/commit/723713098b125431f6e762cbf4b6f3ebe5506a23)

#### Android text wraps to the lines it needs

_Owner:_ Ifiok Jr. · _Introduced in:_ [978e9c5](https://github.com/leptoskit/leptoskit/commit/978e9c59a606318d3ac2e9fe3f203b46cd417d91)

Android measured any plain one-paragraph text with `BoringLayout`, which always lays out exactly one line, even when the text was wider than its box. Layout then gave a paragraph one line's height, and the `TextView` drew only its first line: every paragraph of Reapers on a Solana Seeker was clipped. Text now takes `BoringLayout` only when it fits, and wraps through `StaticLayout` otherwise, as `TextView` does. The narrowest box a text can take is its widest word, with one word to a line.

Text views now also break lines with the measurer's strategy (simple, no hyphenation) instead of Android's balanced default, so a view never needs a line more than its frame was measured to.

- **Expose the Apple development-session callback from the shared bridge so the development-client plugin can compile independently of the app host.** _Owner:_ Ifiok Jr. · _Introduced in:_ [7237130](https://github.com/leptoskit/leptoskit/commit/723713098b125431f6e762cbf4b6f3ebe5506a23)
- **Generated Apple apps follow light and dark.** The generated macOS and iOS apps now tell your screens when the system switches between light and dark, so `dark:` classes update without a resize. A screen that first lays out while the system is dark also starts dark; before, it stayed light until its window changed size, which a menu-bar app or a fixed-size window never does. _Owner:_ Ifiok Jr. · _Introduced in:_ [d16db79](https://github.com/leptoskit/leptoskit/commit/d16db794e0f05db8079965a183badd4ed3ba3644)

#### Apple plugin and JNI builds on current toolchains

_Owner:_ Ifiok Jr. · _Introduced in:_ [fdf5868](https://github.com/leptoskit/leptoskit/commit/fdf586892a1411704c419bfd5069014d4ce5aaa2)

The Apple plugin packages now build and test against the current Xcode SDK. Wire readers no longer declare a `bytes` method that shadows their storage, wire-case helpers have distinct names, calendar validation is shared, EventKit events with no identifier fail honestly, camera photo delegates are retained behind a lock, Security keys use the CF bridge, print uses the current AppKit HTML and PDF APIs, and ActivityKit code is iOS-only. Delegate conformances are main-actor isolated, and the tests import internal wire types with `@testable` and supply a real presentation owner.

The JNI smoke test now finds the JDK through `JAVA_HOME`, invokes the macOS SDK compiler through `xcrun`, and stubs the 3D, timer, and DevTools entry points that the application bridge links.

#### File-route navigators, deep links, and system Back

_Owner:_ Ifiok Jr. · _Introduced in:_ [bbc358b](https://github.com/leptoskit/leptoskit/commit/bbc358babf47a0c482e74527c7011408f620dfe8)

Layouts can export `navigator()` returning a typed `Stack`, `Tabs`, or `Drawer`. Folder groups like `(tabs)/` stay URL-less; the navigator return value chooses the chrome. Tab and drawer roots are `impl Into<Route>`, not string names. Stacks can present matched routes as a sheet or modal above the previous screen, and optionally offer an edge hit target for Back.

`Navigation::open_url` / `open_url_with` turn incoming URLs into typed routes (custom schemes and `http(s)`). `apply_activations` drains a host deep-link queue into the router. `bind_system_back` and `leptoskit_system_back` let Android system Back pop router history when the host offers it. Generated Android activities call into that path before finishing.

[`examples/file_routes`](https://github.com/leptoskit/leptoskit/tree/main/examples/file_routes) shows Stack + Tabs + Drawer, deep links, and Back. True UIKit interactive-pop and AppKit history swipe still need host presentation; the guide names those gaps.

- **Fix wearable wire codecs, watch plugins, and host builds.** Fixes cross-platform wire mismatches in Health (`ABSENT_LENGTH`, `ANDROID_TYPES` / `HARMONY_TYPES` bitmask preserving `HRV_RMSSD`, and exercise `Activity` codes 8..14), HarmonyOS Pedometer (`bounded_reply` 36-byte framing) and Battery charge state mapping, and BLE 16-bit/128-bit UUID normalization. Fixes Apple/Android runtime bugs across `ble`, `watch-sync`, `wearable`, and watchOS `haptics`, adds `Reading::Ecg` for `SensorMask::ECG`, fixes `RotaryState` wrap-around detent math, adds watchOS 10 support to `LeptosKitBridge` and wearable SwiftPM manifests, enables native plugin resolution and unified output paths for `wearos`, `watchos`, and `harmonyos` in `leptoskit_cli`, and adds the wearable crates to CI unit tests. _Owner:_ Ifiok Jr. · _Introduced in:_ [21a5f55](https://github.com/leptoskit/leptoskit/commit/21a5f55fea25e9c1f043da51aae77bcc6296aeb3)
- **Deliver primary-pointer drags, taps and cancellations from Android, iOS and macOS scene views through `Scene::next_pointer`.** Movement uses layout units and a bounded queue coalesces drags independently of model-loading events. _Owner:_ Ifiok Jr. · _Introduced in:_ [7237130](https://github.com/leptoskit/leptoskit/commit/723713098b125431f6e762cbf4b6f3ebe5506a23)

#### Compact layouts, fast Yoga, streaming diff & O(log N) hit

_Owner:_ Ifiok Jr. · _Introduced in:_ [3677b62](https://github.com/leptoskit/leptoskit/commit/3677b6245db20cc2387b3f547855364c319db735)

- Box cold `Style.grid`, `Element.gesture`, and `Element.bindings` fields, pack `Tailwind` into a single `Option<Arc<TailwindInner>>`, gate DevTools-only `#[track_caller]` behind `devtools`, and fuse `Element::validate()` into a single pass.
- Optimize `leptoskit_yoga` with contiguous flex-line ranges, empty/measured leaf fast paths, direct in-flow child iteration, inline `LayoutCache`, and `subtree_pass` dirty pruning.
- Stream `NativeSurface` reconciliation directly without cloning a resolved intermediate tree, prune clean subtrees in `collect_frames`/`place`, binary-search monotonic hit-test siblings in O(log N), switch `Flattener` to a dense zero-clone arena, and encode FFI mutation props by reference without cloning `NativeProps`.

#### Plugin options the app would crash on stop the build instead

_Owner:_ Ifiok Jr. · _Introduced in:_ [da4763e](https://github.com/leptoskit/leptoskit/commit/da4763e983262b2144684984245a2bf27b5a61f5)

An Android app whose `leptoskit.toml` had an empty `[native.plugin-options.websocket]` built, installed, and then stopped at launch with `NativeApplicationInitializationFailure: register plugin: InvalidArgument`. The WebSocket plugin can't start without `origins`, the build only checked that the table was there, and the error didn't say which plugin had failed.

`leptoskit build` now stops with the plugin and the option it needs:

```
plugin `websocket` needs `origins` in [native.plugin-options.websocket]: without it, the app stops at launch when the plugin starts
```

An async plugin lists the options its factory can't start without in its runtime metadata, and an empty string, list, or table counts as left out:

```toml
[package.metadata.leptoskit.runtime]
name = "websocket"
rust-factory = "from_config"
rust-async = true
required-options = ["origins"]
```

`http` and `websocket` require `origins`, `shell` requires `commands`, and `xr` requires nothing. Each crate's tests hold its factory to its list.

When a plugin still fails to start, the error names it: ``register plugin `websocket`: InvalidArgument``. Generated registrants install every plugin through the new `App::register`, and a host directory that can't be used is reported as `app context: …` rather than as a plugin's failure.

The `shell` plugin is now left out of iOS, Android, and HarmonyOS builds, where it can't start processes and its factory always failed, which also stopped the app at launch.

#### Native scenes load models on the first launch

_Owner:_ Ifiok Jr. · _Introduced in:_ [7237130](https://github.com/leptoskit/leptoskit/commit/723713098b125431f6e762cbf4b6f3ebe5506a23)

Android, iOS and macOS install the scene asset reader before starting the guest. Models queued before the first native scene view exists now load from the app bundle on a cold launch.

Generated Axum route lists also return their owned paths directly, keeping generated apps free of redundant conversion warnings.

#### Handed-back calls reach the Rust plugin that serves them

_Owner:_ Ifiok Jr. · _Introduced in:_ [5445246](https://github.com/leptoskit/leptoskit/commit/54452466966f79486f975a84eb139e11a5b0e8ae)

An `AppService` such as Solana Kit's wallet service drains the bridge itself and hands back every call it does not own. Those calls skipped the interpreter's timer and went to the platform, which has no timer plugin: on Android every guest `sleep` failed as `Unavailable` in any app with such a service, so nothing that waited on a timer ever ran. A handed-back call now passes the interpreter first, as a queued one always did.

`WasmHost::serve` and `WasmHost::served_plugins` name what the interpreter answers itself, and `App::rust_plugins` lists every plugin answered in Rust: the timer, the frame clock, inline, worker and async plugins. Tests on the guest thread's own step find every registered plugin by id and fail when a call for a Rust plugin reaches the platform's router, so a Rust plugin added later is held to the same rule.

#### A stopped Android app says why, readably

_Owner:_ Ifiok Jr. · _Introduced in:_ [068178d](https://github.com/leptoskit/leptoskit/commit/068178ddea3abda61c2b0733410dccdd704efa47)

When an Android app's guest stopped, leptoskit replaced it with a plain `TextView` holding the error. That view took the theme's text color, which in a light system theme is dark, so on an app with a dark background the message was nearly invisible: Reapers looked like a black screen on a Solana Seeker. An app whose plugins failed to start didn't get even that: its activity closed at launch.

Both now show `NativeFailureView`: white text on near-black (17:1), centred and selectable, covering the whole app, the same in either theme. An app that cannot start shows it instead of closing.

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

### Breaking changes

#### leptoskit_bridge_ffi: the native half of the app host

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_bridge_ffi` is the C ABI, plus the Swift, Kotlin, and C++ code, that lets a native app shell host a leptoskit Wasm guest: it runs the guest, forwards its plugin calls, and draws its tree with the platform's own views.

The macOS, iOS, and Android apps `leptoskit new` generates link it, and HarmonyOS links its experimental N-API layer; app authors never add it themselves. Its Swift and Kotlin renderers draw ten components so far, including password fields, and stop on any other with an "unsupported widget" error. The generated Linux and Windows apps use the GTK and Win32 backend crates instead.

### Features

#### Try native plugins written only in Rust

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #506](https://github.com/leptoskit/leptoskit/pull/506)

Set `native.experimental-rust-plugins = true` in `leptoskit.toml` to use the experimental pure-Rust `clipboard` and `haptics` plugins on iOS, macOS, and Android in place of their Swift and Kotlin versions. Without the setting, builds keep the Swift and Kotlin versions.

A plugin of your own can do the same in its `[package.metadata.leptoskit.runtime]` tables. With `rust-context = true`, its factory receives a `leptoskit_bridge::AppContext`, whose `platform()` lends the host's native handles: the `JavaVM` and app `Context` on Android, and the on-screen view controller on iOS and macOS. With `rust-main-thread = true`, its calls run on the platform's UI thread, which UIKit, AppKit, and most Android UI APIs require (`App::install_main` installs such a plugin by hand). Mark a platform's registration `experimental = true` to use it only when the setting is on.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
