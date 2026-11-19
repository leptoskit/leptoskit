# Handover: every interface in leptoskit, compiled to Wasm

Written 18 November 2026, against `main` at `511655984`. This is the working state of one stream of work, for whoever picks it up next. It records the owner's decisions, what is on which branch, what has and has not been verified, and the order to continue in.

Two research reports sit beside this file and are the detailed reference:

- [js-to-wasm-audit.md](js-to-wasm-audit.md): every behaviour in the two hand-written scripts, sorted into "possible today", "small gap", and "needs a leptoskit feature", with a proposed API and per-backend work for each of 17 gaps (G1–G17).
- [canvist-editor-design.md](canvist-editor-design.md): how a canvist-powered editor fits into leptoskit. Its naming and its "upstream to canvist" plan were overridden by the owner; see [The canvist editor](#the-canvist-editor).

## The goal

The owner asked for a full example of leptoskit running locally with analytics, a dashboard, and admin services for infrastructure as code. That grew into `leptoskit_admin` (merged in #835–#845) and a public demo service (`examples/demo_service`, unmerged).

Both the admin and the demo render leptoskit markup on the server only and put all their interactivity in hand-written scripts:

- `crates/leptoskit_admin/assets/admin.mjs`, about 1,400 lines;
- `examples/demo_service/assets/demo.js`, about 1,500 lines.

The owner rejected that. The work now is to close the gaps in leptoskit that those scripts paper over, then rebuild both interfaces as hydrated leptoskit and delete the scripts.

## Decisions the owner has made

Do not reopen these. If one turns out to be unworkable, stop and ask.

1. **No hand-written JavaScript.** Every interface is leptoskit components compiled to Wasm. That covers the examples, the public demo, `leptoskit_admin`, and every service UI. No `.js`, `.mjs` or `.ts` behaviour files and no `<script>` glue. When leptoskit cannot express something, that is a leptoskit feature to design, and the API is discussed with the owner first.
2. **Analytics is a separate, optional module.** Any leptoskit app can install it; it is never bundled into core or turned on by default. The same goes for the admin. Today the `leptoskit` crate's `analytics` feature is only a deprecated alias for `apm`, and the analytics service runs only inside the examples. Packaging it as an installable module needs a design brief the owner approves before any code.
3. **Multi-line text is a new `TextArea()` component** with its own `NativeKind`. It is not `TextInput().multiline(true)`.
4. **Overlays are `Modal` and `Popover`.** On the web they are built on `<dialog>.showModal()` and the `popover` attribute.
5. **The demo is converted first,** then the admin, page by page.
6. **The admin is a separate deployable app** that signs admins in through the main site's own sign-in (`leptoskit_idp`), and it teaches while it is used. See [admin-design.md](admin-design.md).

## The order of framework changes

One pull request per item. Every backend is kept in step, with AppKit as the reference. The gap numbers refer to [js-to-wasm-audit.md](js-to-wasm-audit.md), which has the proposed API and the per-backend table for each.

| #  | Change                                                      | Gap      | State      |
| -- | ----------------------------------------------------------- | -------- | ---------- |
| 1  | Text input hints                                            | G9       | #852, open |
| 2  | `TextArea`                                                  | G12      | #877, open |
| 3  | Labels that focus their fields                              | G13      | #879, open |
| 4  | One-shot focus requests                                     | G2       | #887, open |
| 5  | Arrow-key navigation for tabs, radio groups and menus       | G5       | #892, open |
| 6  | `Modal` and `Popover`                                       | G1       | #896, open |
| 7  | Scroll into view                                            | G4       | #913, open |
| 8  | A clock hook                                                | G6       | #914, open |
| 9  | Typed refusals in the rpc client (status, code, retry time) | G8       | #918, open |
| 10 | Streams without a database                                  | G3       | #923, open |
| 11 | The ⌘ key on the web                                        | G7a      | #924, open |
| 12 | New-tab and download links                                  | G14, G15 | #925, open |

After those: convert the demo (3–4 pull requests), then app shortcuts with native key delivery (G7b), `prompt=login` on provider sign-in (G16) and document titles (G10), then convert the admin page by page (5–7 pull requests).

The existing browser test suites, `examples/demo_service/tests/visitor_pages.mjs` and the admin's `script.mjs`, stay as the acceptance tests for the conversions, so keep the test ids.

## What is on each branch

### `feat/text-input-typed-hints`: pull request #852

Adds `.keyboard()`, `.read_only()`, `.max_length()`, `.capitalize()`, `.autocorrect()` and `.submit_label()` to `TextInput` on the web, AppKit, UIKit, Android, GTK, Windows, the Swift and Kotlin renderers, and the terminal.

- **Verified:** the `leptoskit_native` tests with `web,serde`; AppKit's 14 control tests; the terminal tests; the Swift typecheck tests for the generated iOS and macOS apps; the gallery tree snapshots (only hashes changed). Every lint step of the pre-push gate passed on a second machine on the commit before the last rebase.
- **Not verified:** no complete pre-push gate ran on the final commit. It was pushed with the hook skipped, on the owner's instruction, and **CI on the pull request is the gate.** Merge it with a squash only when CI is green. Nothing has run on an iOS, Android, Linux or Windows device.

### `feat/text-area`: pull request #877, stacked on #852

Adds `TextArea()` and `NativeKind::TextArea` (wire name `text_area`, appended last in the wire kind table) on the web, AppKit, UIKit, Android, GTK, Windows, the terminal, and the Swift and Kotlin renderers. Enter adds a line and never raises `Submit`. `aria_multiline(true)` on a single-line field now fails validation.

- **Run on Linux:** the `leptoskit_native` tests (web markup, validation, wire names, surface events), the terminal's interaction tests, the GTK widget tests under a display, the gallery snapshots (unchanged), host and Wasm clippy, and ktlint.
- **Compiled and linted only, by cross-checking from Linux:** `leptoskit_appkit`, `leptoskit_uikit` and `leptoskit_windows`. Their new tests have not run.
- **Not compiled anywhere before CI:** the Swift renderers (`MacTextArea`, `MobileTextArea`) and the Kotlin ones (`NativeTextArea`, and `text_area` in `WidgetHost.kt`). Swift was parsed with `swift-format` and Kotlin linted with ktlint, nothing more.
- **Known gaps:** GTK and Windows show no placeholder. HarmonyOS and the watch surfaces do not draw the component. The Android probe screen has no `TextArea` yet.

### `feat/label-focus`: stacked on `feat/text-area`

On the web, a plain `Text` that is the one `aria_labelled_by` target of a field renders as `<label for>` (`crates/leptoskit_native/src/web/labels.rs`). Android's `WidgetHost.kt` sets `labelFor`. No new API.

- **Run on Linux:** the `leptoskit_native` tests, including the new markup test and a wire test for the keys Android reads; host and Wasm clippy; ktlint.
- **Not run:** the Android host change, and no browser test clicks a label yet. The demo's `visitor_pages.mjs` covers it once the demo is converted (DM-19).
- **Found on the way, not fixed:** `WidgetHost.kt`'s `applyAccessibility` reads `aria_hidden`, `role`, `aria_live` and the rest from the top level of the props, but the wire nests them in an `aria` object, so the Rust Android backend appears to drop every ARIA property except the name. The Kotlin renderer for generated apps reads the nested object. Needs an Android device to confirm.

### `feat/focus-requests`: stacked on `feat/label-focus`

`FocusRequest::new()`, `Element::focus_on(request)` and `request()` (`crates/leptoskit_native/src/focus.rs`), as the audit's G2 proposes. The handle is a remembered signal holding a serial; `request()` takes the next serial from one process-wide counter. The widget carries the serial in `NativeProps::focus_request`.

- **One place decides "once".** `NativeSurface` remembers the newest serial it has sent and strips `focus_request` from every `Create` and `UpdateProps` that does not carry a newer one (`props_to_send` in `surface/reconcile.rs`). Backends hold no state: after a batch they focus `MutationList::focus_target()`. The web keeps the newest serial it answered in a thread-local and reads `data-lk-focus-request`.
- **Run on Linux:** native, web markup, terminal and GTK tests; a Chrome test for the web; host and Wasm clippy; AppKit, UIKit and Windows cross-linted (compiled, never run); ktlint.
- **Not run:** AppKit, UIKit, Windows, `WidgetHost.kt`, and the Swift and Kotlin renderers. The Swift and Kotlin tests added here first run in CI.
- **Left for later:** on AppKit, UIKit, Windows and Android the move happens while events are held back, so the widget hears no `Focus` event for it. `focused(true)` still does nothing on AppKit, Windows, GTK and Android; the audit hoped this change would fix that, and it does not.

### `feat/roving-focus`: stacked on `feat/focus-requests`

G5 for tabs and radio groups, in `leptoskit_aria` only. No backend changed.

- `Tabs::list` now takes `on_select` and `Tabs::tab` no longer does, as the audit proposes; `.orientation()` picks the arrow axis. `RadioGroup::new(id, labels, selected).option(i, content).on_change(f)` becomes an element with `Element::from`. Both keep one Tab stop (`tab_index` 0 on the current item, -1 on the rest), take the arrow keys, Home and End in an `on_key` on the group, and move focus with a `FocusRequest` held by the current item. The shared key logic is `crates/leptoskit_aria/src/roving.rs`.
- `WidgetTester::press_key` is new in `leptoskit_testing`.
- **Run on Linux:** the `leptoskit_aria` tests, including `tests/terminal.rs`, which drives both components through the terminal backend's real key routing and focus ring; a Chrome test of the same pattern in `leptoskit_native`'s `web_browser.rs`.
- **Native:** AppKit, UIKit, Android, GTK and Windows deliver no key presses yet (G7b) and ignore `tab_index`, so the arrows do nothing there and every item stays tabbable.
- **Not done here: `Menu`.** The audit builds it on `Popover` (G1), so it goes in with change 6.
- **A name to watch:** `leptoskit::aria::RadioGroup` and the prelude's `RadioGroup` (the native radio group name) collide under two glob imports. The docs say to import the aria one by name.

### `feat/overlays`: stacked on `feat/roving-focus`

The first part of G1: the API, the shared surface, the web and the terminal. The native presentations and `leptoskit_aria`'s `Dialog::modal()` and `Menu` follow as their own pull requests on top.

- `Modal(open)` and `Popover(anchor, open)` are `NativeKind::Modal` and `NativeKind::Popover`, with `NativeProps::overlay` (`Overlay { open, anchor, placement, initial_focus }`) and `NativeEvent::Dismiss`. `Element::on_dismiss`, `initial_focus`, `placement`, and `open` and `anchor` for the `view!` tags. Validation refuses an anchor or `initial_focus` id that is not in the tree. A dismissal does not bubble.
- **The surface** takes an overlay out of its parent's layout in `Element::resolved` (closed: `display: none`; open: absolute, a modal sized to the viewport), places it in `surface/layout.rs` (a modal at the viewport origin, a popover beside its anchor with `Placement::position`, after everything else has a position), and hit-tests open overlays first.
- **`NativeBackend::presents_overlays`** decides what a backend is told. `true` (the terminal, and `HeadlessBackend::new().presenting_overlays()`): the real kind, and frames in viewport coordinates. `false` (every other backend, and the recording backend the Swift and Kotlin hosts read): a `View` with a parent-relative frame that lands in the same place. So a native backend needs no code to show an overlay, and each one switches to `true` in the pull request that gives it a real sheet, popover or window.
- **Web:** `web/overlay.rs`. `<dialog data-lk-modal>` and `<div popover="auto" data-lk-popover>`; a microtask after each render calls `showModal()`, `close()`, `showPopover()` and `hidePopover()` to match, places popovers, and moves focus. `cancel`, a click on the dialog itself, and a `toggle` the app did not ask for become `Dismiss`. A popover's anchor button is made its `popoverTargetElement` with the `hide` action, so a press on it is not a light dismiss followed by the app reopening it.
- **Terminal:** overlays paint last, the focus ring is the top modal's content, Escape and outside clicks dispatch `Dismiss`, and `TuiBackend::follow_overlays` moves focus in on open and back on close.
- **Run on Linux:** `leptoskit_native`'s `tests/overlay.rs` and the markup test in `tests/web.rs`; two Chrome tests in `tests/web_browser.rs`; `leptoskit_tui`'s `tests/overlays.rs` through the real host loop.
- **Native:** nothing has run. AppKit, UIKit, GTK, Windows, Android and the Swift and Kotlin renderers only receive a `View`, so the change to them is the two new match arms. Nothing there asks to dismiss an overlay, moves focus into it, or lifts it out of a clipping parent.
- **Not tested:** hydrating a server-rendered `<dialog open>` into a modal. The code path is the same `sync` the Chrome tests run, but no test starts from server markup.

### `feat/aria-menu` (#901): stacked on `feat/overlays`

The second part of G1: the two `leptoskit_aria` components the audit names.

- **`Menu`** (`crates/leptoskit_aria/src/menu.rs`): `Menu::new(id, open).button(el).item(label, on_select)`, plus `.content(el)` for the last item, `.placement(..)` and `.on_toggle(Fn(bool))`. It becomes a `View` holding the button (`{id}-button`, `aria-haspopup="menu"`, `aria-expanded`, `aria-controls`) and a `Popover` with the `menu` role (`{id}-menu`) whose items are `Pressable`s with the `menuitem` role, `tab_index(-1)` and one `FocusRequest` each. Arrow keys go through `roving::target`; Tab closes. The audit sketched `new`, `button` and `item` only: `on_toggle` is how the app's open state hears the button, a choice and a dismissal, and `content` and `placement` pass through to what is underneath.
- **`Dialog::modal(open)`** returns `Element::modal(open)` with the dialog's id and its title and description references. `container(open)` is unchanged.
- **Terminal:** `input::tab` dismisses the top popover when Tab would move focus out of it, `TuiBackend::can_take_focus` lets focus requests and `initial_focus` reach `tab_index(-1)` controls, and `paint` underlines only the focused `Pressable` (it underlined all of them, which hid which menu item had focus).
- **Run on Linux:** `leptoskit_aria`'s `tests/components.rs` (headless), `tests/terminal.rs` (the real terminal host loop: open, arrows, choose, Escape, Tab, focus return) and the new `tests/web.rs` in Chrome (added to `test:browser`); `leptoskit_tui`'s `tests/overlays.rs` and `tests/render.rs`.
- **Native:** nothing has run and nothing native changed. A menu there is a popover drawn in place whose items press; keys and dismissal wait for the native presentations.
- **Not tested:** real Escape and a real pointer press outside a menu in Chrome. The test hides the popover the way the browser does and checks that the menu hears it.

### `feat/overlays-appkit` (#904): stacked on `feat/aria-menu`

The third part of G1, for macOS: both renderers.

- **`leptoskit_appkit`** returns `true` from `presents_overlays`. `overlay.rs` holds `Sheet` (an `NSPanel` subclass begun as a sheet on the window of the modal's parent; `cancelOperation:` becomes `Dismiss`) and `Hanging` (an `NSPopover`, transient, whose delegate answers `popoverShouldClose:` with `Dismiss` and `false`). An overlay's view is never a subview of its parent, so `insert_child` skips overlay siblings when it picks the view to go below. `refresh_overlays` runs after each batch and from `open_window`: an overlay needs a window, and waits for one. The sheet is sized to the union of the modal's children, and the modal's background is cleared.
- **The Swift macOS renderer** does the same with `MacOverlayView` (`desktop/MacOverlayView.swift`, added to `Package.swift` and the CLI's `platform_sources::MACOS`): a hidden view that keeps the overlay's place in the tree, so sibling indexes stay as sent, with the content shown in a sheet or popover. `WidgetProps` gains `id` and `overlay`; both arrive in the JSON form (and in the binary form's extras, which iOS reads). The bridge records with `presenting_overlays()` on macOS only.
- **Compiled here:** the Rust side, cross-linted for `aarch64-apple-darwin` with clippy. **Not compiled here:** any Swift. **Run here:** nothing, on either renderer.
- **Tests that CI's macOS runners will be the first to run:** `crates/leptoskit_appkit/tests/overlays.rs` (real windows) and `controller-tests/MacOverlayTests.swift`. Expect to fix things there. The assertions most likely to need adjusting are the ones about AppKit's own behaviour: `attachedSheet` right after `beginSheet`, a popover showing from a window that was only ordered front, and which side a popover lands on.
- **Known limits:** a popover is centred on its anchor (no start or end alignment). A sheet has no outside press. Arrow keys in a `Menu` still do nothing on macOS, because neither renderer delivers `on_key`. `AppKitBackend::snapshot_png` renders without a window, so an open overlay is not in a snapshot.

### `feat/overlays-uikit` (#906): stacked on `feat/overlays-appkit`

The third part of G1, for iOS: both renderers.

- **`leptoskit_uikit`** returns `true` from `presents_overlays`. `overlay.rs` holds `Presented`: an `OverlayController` (a `UIViewController` subclass) presented `.formSheet` for a `Modal` and `.popover` for a `Popover`, with a `PresentationDelegate`. The presenter is the view controller found along the responder chain from the modal's parent, or the popover's anchor, then whatever that one has already presented. A sheet's swipe reports `Dismiss` from `presentationControllerDidAttemptToDismiss:`; a press outside a popover reports it from `presentationControllerShouldDismiss:`; both answer `false`. Escape (a `UIKeyCommand`) and `accessibilityPerformEscape` report it too. `adaptivePresentationStyle…` answers `.none`, so a popover stays a popover on an iPhone. **This departs from the audit**, which says a popover "becomes a sheet on compact width": a menu as a full sheet covers the button it belongs to.
- **`UIKitBackend::present_overlays`** is public because an overlay that is open before the view joins a window has nothing to present over. `SurfaceController` calls it from `viewDidAppear:`, and `mount` calls it too.
- **The Swift iOS renderer** does the same with `MobileOverlayView` (`mobile/MobileOverlayView.swift`, in `Package.swift`'s exclude list and the CLI's `platform_sources::IOS`): a hidden place holder in the tree whose `content` is presented by a `MobileOverlayController`. Children are counted against the place holder, through `MobileOverlayContent.owner`. The place holder is hidden, so its dismiss event goes straight to `onEvent`: `event(_:_:)` drops what a hidden widget reports. The bridge records with `presenting_overlays()` on macOS and iOS.
- **Animation:** presentations animate unless `UIView.areAnimationsEnabled` is off (or the Rust host has paused animations). Nothing waits for a transition to end, so opening and closing an overlay inside one transition is not handled: UIKit may refuse the second call, and the next transaction puts it right.
- **Compiled here:** the Rust side, cross-linted for `aarch64-apple-ios` with clippy, the probe included. **Not compiled here:** any Swift. **Run here:** nothing.
- **Tests that CI's iOS job will be the first to run:** `overlays` in `crates/leptoskit_uikit/examples/native_probe.rs` and `ios-controls/OverlayControls.swift`. Both switch animations off and expect a presentation, and a dismissal, to be in place when the call returns. If UIKit defers either even without animation, those assertions need `until`.
- **Known limits:** a popover is centred on its anchor. On an iPhone the sheet is the full "large" height however little the modal holds (a custom detent needs iOS 16; the crate supports 15). Arrow keys in a `Menu` do nothing, because neither renderer delivers `on_key`.

### `feat/overlays-android` (#907): stacked on `feat/overlays-uikit`

The third part of G1, for Android: both hosts.

- **`leptoskit_android`** returns `true` from `presents_overlays` and maps event code `12` to `Dismiss`. In `android/WidgetHost.kt` a node with `presented` set never has its view added to its parent's: `InsertChild` skips it and counts only the siblings that are not overlays. `refreshOverlays` runs at the end of each `apply` and from `mount`. A `Modal` is a `ModalDialog`, a `Dialog` whose `cancel()` only emits `Dismiss`, so Back, Escape and a touch outside all ask and nothing closes. Its content view is as large as the union of the modal's children, and the modal's own view is shifted by negative margins so those children land at the dialog's corner. A `Popover` is a focusable `PopupWindow` shown with `showAtLocation` at the frame layout gave it. Its touch interceptor emits `Dismiss` for a touch outside and consumes it.
- **Back on a popup cannot be refused** without subclassing the popup's decor view, so the popup closes, `setOnDismissListener` emits `Dismiss`, and the host shows it again at once if the tree still has it open (`suppress { refreshOverlays() }`). The Kotlin renderer for generated apps cannot do that, because its events reach the app asynchronously: there the popup returns with the next batch.
- **The Kotlin renderer** (`mobile/NativeOverlayView.kt`, in the CLI's `platform_sources::ANDROID`, `Package.swift`'s exclude list and `setup/android_widget_paint`) keeps a `GONE` place holder in the tree and puts children into its `content`, as the Swift renderers do. `WidgetProps` gains `name` (the element's `id`) and `overlay`. Its `DISMISS` is `10`, the bridge's code; the crate's is `12`. The bridge records with `presenting_overlays()` on macOS, iOS and Android.
- **Two fixes this branch needed were pushed to the pull requests that broke the build**, and are cherry-picked here: `import android.text.InputFilter` in `WidgetHost.kt` (#852) and the `Dismiss` arm in `src/probe.rs` (#896). The branches between do not have them until the stack merges in order.
- **Checked here:** `WidgetHost.kt`, `ProbeActivity.kt` and every Kotlin file under `mobile/` and `android/` type-check with `kotlinc` against the API 34 `android.jar` (one error that was already there: `VANILLA_ICE_CREAM` is API 35). `ktlint` is clean. The Rust side is cross-linted for `aarch64-linux-android` with the `probe` feature. **Run here:** nothing: there is no emulator.
- **Tests that CI's Android jobs will be the first to run:** `verifyOverlays` in `android-tests/ProbeActivity.kt` (a real Activity: dialog size and place, popup place, Back and outside touches ask, a removed popover comes down) and `anOverlayKeepsItsPlaceAndWaitsForAWindow` in the bridge's `NativeControlsTest.kt`, which has no window and so checks only the tree.
- **Known limits:** arrow keys in a `Menu` do nothing, because neither host delivers `on_key`. A dialog needs an Activity. A modal's own background is not drawn.

### `feat/overlays-gtk` (#909): stacked on `feat/overlays-android`

The third part of G1, for Linux. GTK has no Swift or Kotlin renderer: generated Linux apps use this backend.

- **`leptoskit_gtk`** returns `true` from `presents_overlays`. `overlay.rs` holds `Overlay`: a host `Frame` in either a modal, undecorated, non-resizable `gtk::Window` (transient for the window of the modal's parent) or a `gtk::Popover` with `autohide` off and no arrow. A display-wide style under the class `leptoskit-overlay` strips the theme's panel, shadow and padding. The node's own frame is a child of that host and never of its parent's content, so `insert_child` counts only the siblings that are not overlays.
- **Placement** uses the frame layout already worked out, not GTK's own. The popover is hung (`Frame::hang`, which also presents it from `size_allocate`, as GTK asks of a popover's parent) on the root node's frame, or on the frame of the overlay it is inside, and points at a one-pixel line along its own top edge.
- **Dismissal:** `autohide` is off so that nothing closes by itself. `watch` puts two capture-phase controllers on each window an overlay shows in: a key controller that turns Escape into `Dismiss` for the top overlay, and a click gesture that turns a press outside the top popover into `Dismiss` without claiming the press. A press on the popover's anchor asks for nothing, as in the terminal. A dialog's `close-request` reports `Dismiss` and is refused.
- **Focus** moves on an idle, to `initial_focus` or the popover's first control (a dialog's window focuses its own first control). A popover remembers what had focus and gives it back when it closes with focus inside.
- **Run here:** `tests/overlays.rs`, four tests on real windows under Xvfb with `xdotool` presses and keys, and all of `tests/widgets.rs`. `test:widgets:gtk` now runs both files. Xvfb has no window manager, so nothing here shows where a window manager puts the dialog.
- **Known limits:** GTK does not dim the window behind a modal. Arrow keys in a `Menu` do nothing, because the backend delivers `on_key` only for radios and sliders. A host that never calls `open_window` or commits after its window maps does not show an overlay that was already open.

### `feat/overlays-windows` (#910): stacked on `feat/overlays-gtk`

The last part of G1. Every backend now returns `true` from `presents_overlays`, except the recording backend the bridge uses for hosts with no overlay support of their own.

- **`leptoskit_windows`**: `overlay.rs` holds `Shell`, one `WS_POPUP` window of class `Leptoskit.NativeOverlay` per `Modal` or `Popover`, owned by the top-level window the overlay's parent is in (the host, or the shell of an overlay it is inside). `Widget::place` asks `Shell::arrange` where the widget's composition window goes: into the shell while the overlay is open, into the parking window while it is not. `place_tree` calls `present_overlay` after the children are placed, which shows or hides the shell and moves focus. A parent's composition bounds skip overlay children.
- **Placement** is the frame layout already worked out, turned into screen pixels from the corner of the host's client area. A modal's shell covers the union of its visible children.
- **A modal** disables its owner (`EnableWindow`) while shown and enables it again before hiding, so activation returns to it. **A popover** leaves the owner alone and reports `Dismiss` from `WM_ACTIVATE` with `WA_INACTIVE`, unless activation went to a popup it owns or the primary button is down over its anchor.
- **Escape and Tab:** `WindowsWindow::run` and `pump_messages` now pass each message to `IsDialogMessageW` for the shell it belongs to (`overlay::dialog_for`, by class atom) and for the host otherwise. The dialog manager turns Escape into `WM_COMMAND` with `IDCANCEL`, which the shell reports as `Dismiss`. `WM_CLOSE` does the same.
- **An overlay open before the host is visible** shows from `WindowsWindow::show`, which places the tree again (`Widget::show_overlays`).
- **Compiled here:** cross-linted for `x86_64-pc-windows-msvc` with clippy, tests included. **Run here:** nothing.
- **Tests that CI's Windows runner will be the first to run:** `crates/leptoskit_windows/tests/overlays.rs`, three tests on real windows. The assertions most likely to need adjusting are `GetFocus()` after a shell is shown, and the exact pixel equality of the shell's and the panel's corners at a fractional DPI scale.
- **Known limits:** Windows does not dim the owner. The shell is a plain rectangle, so a panel with round corners shows the modal's background in the corners. A shell does not follow its owner if the owner moves. Arrow keys in a `Menu` do nothing, because the backend delivers `on_key` only for radios.

### `feat/scroll-into-view` (#913): stacked on `feat/overlays-windows`

G4. `Reveal`, `ScrollAlign` and `Element::reveal_on` in `leptoskit_native` (`reveal.rs`), exported from the `leptoskit` prelude. In `view!` the attribute is `reveal_on=(handle, ScrollAlign::Center)`, because a tuple spreads into two arguments.

- **Web:** `reveal_after_render` in `web/mod.rs` works as the focus request does: the element carries `data-lk-reveal="<serial> <align>"`, a microtask finds the newest serial above the one last answered, and calls `scrollIntoView` with `block` set to the alignment and `inline: nearest`. It is smooth unless `prefers-reduced-motion` matches.
- **Native:** `NativeProps.reveal` never reaches a backend. `props_to_send` strips it and queues each serial above `reveal_answered`. After layout, `scroll_to_reveals` (`surface/layout.rs`) finds the nearest `ScrollView` ancestor, works out the offset from the hit areas, and sends that view two `UpdateProps`: one with `scroll_offset` cleared, then one with the animated offset. The first exists because renderers scroll only when the offset differs from the last one they were sent, and a second reveal of the same place after the person scrolled away must still scroll.
- **Where the view is now:** the surface keeps the last `Scroll` event per view in `scrolled`, set in `dispatch_event`. A host that does not forward scroll events leaves it at the last offset the app or a reveal set, so `Nearest` can misjudge there.
- **Backends that now apply `scroll_offset`:** GTK (`scroll_to_offsets`, sets the adjustments after the batch, raising `upper` to the new extent first), Windows (`Widget::update_props` sets the scroll cell and the batch-end `refresh_scroll` clamps it), AppKit (`scrollToPoint` and `reflectScrolledClipView` after the documents are sized) and UIKit (`setContentOffset:animated:`). Each scrolls when the offset changes and not while it holds the same one. GTK, Windows and AppKit do not animate.
- **Not done:** the terminal and HarmonyOS apply neither. Only the nearest scroll view moves, where the web scrolls every ancestor.

### `feat/clock-hook` (#914): stacked on `feat/scroll-into-view`

G6. `leptoskit_use::use_clock(interval)` returns a `Clock` that advances by itself. It is a loop over `leptoskit_query::try_sleep`, wrapped in `futures::future::abortable` and aborted from `on_cleanup`. The value is `now_ms()` minus the start, kept from going backwards, so it is measured and not counted.

- **Feature:** `clock` on `leptoskit_use` (it brings in `leptoskit_query` and `any_spawner`, which the crate otherwise avoids), and `clock` on `leptoskit`, which turns on `use` and `query`. `leptoskit_use/web` passes `browser` to `leptoskit_query` when both are on.
- **Run here:** `tests/clock.rs` on tokio, and `tests/web_clock.rs` in headless Chrome. `test:browser` in `devenv.nix` now runs the browser one.
- **Not run:** a native guest. There the loop uses the timer plugin through `leptoskit_query`'s `guest` feature, which the query cache already relies on.
- **Limit to know for the conversions:** the timer stops on owner cleanup. A leptos component on the web has an owner that is cleaned when it leaves. A native guest keeps its owner for as long as it runs, so a clock made in a screen there keeps ticking after the screen leaves; make one for the app. A view function that rebuilds starts a new clock on each run, and `remember(|| use_clock(…))` does not fix that on the web, because the closure runs under the owner of the first build, which is cleaned on the next. A clock that a rebuilt screen can keep would be a new API, and has not been asked for.

### `feat/rpc-typed-refusals` (#918): stacked on `feat/clock-hook`

G8. `rpc::Error::rate_limited(wait)` (429) and `rpc::Error::refused(code, message)` (409, or any status through the variant) reach the caller as `FnError::Refused { status, code, message, retry_after }`.

- **The wire:** a failure with a code is `Refused|` followed by a JSON object (`status`, `code`, `message`, `retry_after` in whole seconds, rounded up). It is in the body, so it needs no CORS header to be read, and the in-process path uses the same text. HTTP also sends `Retry-After`. Live and stream frames carry `code` and `retry_after` as optional fields of `WireError`, left out when empty, so older frames still parse.
- **Generated clients:** Rust, TypeScript, Dart, Go, and Python each read the `Refused` kind and the two live fields. The snapshots in `crates/leptoskit_rpc_codegen/tests/snapshots` were reviewed line by line.
- **Run here:** `leptoskit_rpc` and `leptoskit_rpc_codegen` tests. **Not run:** the generated TypeScript, Dart, Go, and Python were not compiled or executed against a server here, only snapshotted.
- **Limit to know for the demo:** only a rate limit carries a wait. The demo's "full, opens in N seconds" (503 with a countdown, DM-14) can't put its countdown in `Error::Refused`, which has no `retry_after` in the audit's API. Use `rate_limited` for it, return the countdown as data, or ask the owner for a wait on `Refused`.
- **Old clients:** one built before this reads a refusal as `Other` with the raw body.

### `feat/streams-without-db` (#923): stacked on `feat/rpc-typed-refusals`

G3. A `#[rpc::stream]` body can take `rpc::StreamCx` (the caller, no database), and `rpc::server::LiveStreams` serves streams on a server that has none. Feature `live-streams` on `leptoskit_rpc`, `rpc-live-streams` on `leptoskit`; `live` includes it.

- **How it is built:** the socket and server-sent events adapters moved from `server/live/` to `server/transport/` and are generic over a small `Host` trait, which `Live` and `LiveStreams` both implement. The per-connection stream bookkeeping moved out of the live session into `StreamSet` in `server/stream.rs`, and both sessions use it. A session hands a stream a hidden `StreamOpen` (the caller, and the database when there is one); the code `#[rpc::stream]` generates turns it into `StreamCx` or `QueryCx`.
- **Where it departs from the audit's wording:** the audit says `live-streams` "needs no `db`". It needs no driver and no connection, but it does turn on the crate's `db` feature, because `Identify` returns `leptoskit_db::Context`. And `Live::streams_only()` exists only where the build has `live`, since `Live` itself needs a driver; `LiveStreams::new()` is the name that always works.
- **Run here:** `tests/live_streams.rs` with only `live-streams` on (no driver), over the WebSocket and over server-sent events, and again with every feature on, where it also opens both kinds of stream on the full `Live`. The existing SQLite live suites still pass after the refactor.
- **Not run:** the Postgres suites (`initdb` won't run as root here), a browser or native client against `LiveStreams`, and `#[rpc::stream(resumable)]` on `LiveStreams` (the same `StreamSet` path as `Live`, which the SQLite suites cover).
- **For the demo:** its three feeds become `#[rpc::stream]` bodies taking `StreamCx`, served by `LiveStreams` beside the demo's own routes. The bounded-window reconnect maps onto `resumable`.

### `feat/key-meta` (#924): stacked on `feat/streams-without-db`

G7a. `KeyPress` has a `meta` field and `KeyPress::meta()`, and `KeyPress::is_primary()` is ⌘ on Apple and Ctrl elsewhere. The web no longer drops ⌘ keys before `on_key`; a key no handler takes is not prevented, so the browser's own ⌘ shortcuts still work.

- **How `is_primary` decides:** in a browser build (`wasm32` with the `web` feature) it reads `navigator.platform` once and looks for `Mac`, `iPhone`, `iPad` or `iPod`. Everywhere else it is `cfg!(target_vendor = "apple")`.
- **Changed on purpose:** `KeyPress::is` is false while ⌘ is held, and the roving groups and `Menu` ignore ⌘ with an arrow or Tab, so no existing handler starts taking the browser's ⌘ shortcuts now that they arrive. `meta` is `#[serde(default)]`, so a guest and host built on either side of this change still read each other's key presses.
- **Run here:** the `leptoskit_native` unit, contract and Chrome tests (a ⌘ key is heard and prevented when handled, left alone when not, and the primary modifier follows the device), `leptoskit_aria`, `leptoskit_tui` and `leptoskit_wasm` tests.
- **Not run:** Safari or any browser on a Mac, so ⌘ as the primary modifier is only covered by the unit test of the platform names.
- **For G7b:** the native backends still deliver no `on_key`. When they do, each must fill `meta` (⌘ on AppKit and UIKit, the Windows key on Win32, Super on GTK, Meta on Android). A Wasm guest in a native host is built for `wasm32` without `web`, so `is_primary` there asks for Ctrl even on a Mac or an iPhone; G7b needs to settle that, probably by having the host say which modifier is primary. The terminal can never see ⌘, so `is_primary` is never true in a terminal on a Mac.

### `feat/link-targets` (#925): stacked on `feat/key-meta`

G14 and G15. `Pressable().href(url).target(LinkTarget::NewTab)` and `.download("name.json")`. The web renders `target="_blank" rel="noopener noreferrer"` with an "Opens in a new tab" `aria-description`, and `<a href download>`. Both are stored in `NativeProps::extra` beside `href`, so no element hash changes for trees that don't use them.

- **Native link opening:** the new `links` feature on `leptoskit_native` (and `leptoskit`) adds an optional dependency on `leptoskit_plugin_linking`. A `Press` that no handler took, on or inside a `Pressable` with an absolute `href`, is opened: `NativeSurface::dispatch_event` calls the plugin's `desktop::open` on its own thread (macOS, Windows, Linux, and so the terminal), and the guest runtime's `dispatch` asks the host's `linking` plugin. No backend or renderer changed, because every one of them already raises `Press` on a pressable. The code is `crates/leptoskit_native/src/links.rs`.
- **Where it departs from the audit:** the audit doesn't say how native opening is switched on; a Cargo feature was the smallest way that keeps `url` and `open` out of apps that don't want them. And there is no framework-level native download: a download link opens like any other link and the browser saves the file. The audit's own recipe (HTTP plugin, then `save_file` or the share sheet) is documented for files behind a sign-in, as app code.
- **Run here:** `leptoskit_native` tests with `web,serde,guest,links`, the Chrome suite (attributes, and a handled press on a new-tab link is prevented), and by hand the ignored test `a_press_on_an_outside_link_opens_it`, which returns `Handled` on Linux; no browser exists here, so nothing visibly opened.
- **Not run:** a real link opening on macOS, Windows, a Linux desktop, iOS or Android. An app built in-process for iOS or Android (not a guest) has no Rust implementation in the linking plugin, so a press there still does nothing.
- **For the conversions:** the demo's "Open the admin" becomes a `NewTab` link, and the admin's export confirmation becomes a `download` link, with no script.

### `fix/web-semantic-rebuild`: stacked on `feat/link-targets`

Found while converting sign-in: on the web every screen rebuild replaced each landmark, heading, list, paragraph and label with a new element, so a focused field inside `<main>` lost focus on any rebuild. The renderer built those with tachys's `custom(name)`, and tachys rebuilds a custom element in place only when its name equals the DOM's `tagName`, which is upper case in an HTML document. The renderer now uses tachys's typed element for each tag (`semantic_element` in `crates/leptoskit_native/src/web/mod.rs`).

- **Run here:** `web_browser` in headless Chrome (38 tests, three new. One: a rebuild keeps `main`, an `h2`, an `article` and the focused field inside them), clippy for `leptoskit_native` on Wasm, and the demo's browser suite on top.
- **Not run:** Firefox (CI runs this suite with geckodriver). No native backend is affected: they do not use this renderer.
- **Upstream:** the case-sensitive comparison is in tachys 0.3.0-rc1 (`HtmlElement::rebuild`, from leptos issue 4412). Worth reporting; the typed elements are right either way.
- **Attributes too.** Found while converting Notes: a node's `id`, `role`, `aria-*` and `tabindex` were a `Vec<AnyAttribute>`, and tachys rebuilds that list by removing every old attribute and building the new ones. A focused heading with `tabindex="-1"` (the new-day dialog's title) lost `tabindex` for a moment on each rebuild, and Chrome moved focus to `<body>` without a blur event. `semantics::Semantics` now diffs by name. Browser test: a rebuild that drops `aria-describedby` touches only that attribute, and the heading keeps focus.
- **Tab in a modal.** Chrome lets Tab leave a modal `<dialog>` for its own toolbar after the last control (and in headless Chrome focus then stays on `<body>`). `Modal` now wraps Tab and Shift+Tab itself (`overlay::browser::keep_tab_inside`); a focused element that is not a stop, such as a dialog's heading, counts by its place. Browser test `tab_wraps_inside_an_open_modal`.
- **Follow-up, not fixed:** on the web, an `Element::text_from` binding is read with tracking (`reactive.rs::read_binding`), so a ticking countdown rebuilds the whole screen once a second, not just its text. The audit's note that only the text updates is wrong. Rebuilds are now cheap and keep focus, but a binding that updates its own text node would be cheaper still.

### `feat/auth-provider-prompt`: G16

`StartProvider` has `prompt: Option<Prompt>` (`Login`, `SelectAccount`), sent as OpenID Connect's `prompt` on the provider's authorization page. The app's prompt replaces the account picker that connecting an account asks for on providers known to accept it (`oauth::prompt_for`). Every `StartProvider { … }` in the repository gained `prompt: None`, including the generated auth pages' templates.

- **Run here:** `leptoskit_auth` unit tests for the parameter, a router test that `POST /auth/oauth/start` with `"prompt": "login"` puts `prompt=login` on the URL, clippy for `leptoskit_auth` with every feature, and the CLI's auth page template tests. The Postgres tests can't run here (the container is root).
- **For the admin:** "Switch account" is `sign_in_with_provider()` with `prompt: Some(Prompt::Login)`, replacing `admin.mjs`'s edit of the returned URL.

### `feat/document-title`: G10

`leptoskit_router::set_title(&str)` sets `document.title` in a browser (the router's `web` feature on Wasm) and does nothing elsewhere. Fragments are left out, as the audit recommends: the admin moves `#workings=` to a query parameter.

- **Run here:** `tests/web_title.rs` in headless Chrome (new, added to `test:browser`), and clippy for `leptoskit_router` on the host and on Wasm with `web`.
- **Not done:** native window titles. AppKit, GTK and Windows set a title only when the window opens, and nothing carries a later one from the app to the host. That needs a backend hook and a bridge message for guests. Nothing in the admin needs it.
- **For the admin:** AD-24's title on a builder swap is `set_title` in the effect that follows the route; the server's shell writes the first `<title>`.

### `feat/app-shortcuts`: G7b, part 1 (the web)

`use_shortcut(Shortcut, run)` in `crates/leptoskit_native/src/shortcut.rs`, re-exported as `leptoskit::shortcut` and in the prelude. `Shortcut::key("w")`, `Shortcut::primary("k")` (⌘ on Apple, Ctrl elsewhere, through `KeyPress::is_primary`), `.shift()`, `.alt()` and `.unless_typing()`. Registrations live in a thread-local list; each call site holds one through `remember`, so a rebuild replaces its `run`, and the entry leaves when the screen or owner goes. The newest match wins. `shortcut::dispatch(&KeyPress, typing)` is what a host calls for a key no element handled.

- **Web:** one `window` `keydown` listener (`web/shortcuts.rs`), installed by the first `use_shortcut`. An element whose `on_key` handled the key stopped it and prevented its default, so it never arrives; a shortcut that runs prevents the default. "Typing" is a text `<input>`, a `<textarea>`, a `<select>` or editable content.
- **Run here:** two unit tests (matching, and the newest wins and leaves with its owner), the module's doc test, the new browser test `app_shortcuts_work_wherever_focus_is_and_yield_to_handled_keys` in headless Chrome, and clippy for `leptoskit_native` on the host and on Wasm.
- **Not run:** Firefox (CI's geckodriver job runs `web_browser`).
- **Delegation caveat:** leptos's `delegation` feature (off in this workspace) puts element handlers on a `window` listener too. If an app turned it on and called `use_shortcut` before any element registered a `keydown` handler, the shortcut listener would run first. Nothing here turns it on.
- **Part 2, not done: native key delivery.** No native host delivers keys to `on_key` or to shortcuts yet, so `use_shortcut` does nothing there. The audit's G7 lists the hooks per platform (AppKit `keyDown:` and a local monitor, UIKit `pressesBegan` and `keyCommands`, Android `dispatchKeyEvent`, GTK `EventControllerKey`, Win32 `WM_KEYDOWN`). Each must fill `KeyPress::meta`, call the element's handlers first, then `shortcut::dispatch`. A Wasm guest in a native host needs the host to say which modifier is primary (see `feat/key-meta`). The admin conversion needs only the web, so this waits.

### `feat/admin-pages-crate`: admin conversion, part 1 (merged, #948)

The admin's pages move to `crates/leptoskit_admin_pages`, one crate built twice like `demo_pages`: `leptoskit_admin` links it with `ssr`, and `leptoskit build` compiles it to Wasm with `hydrate`. `style`, `components`, `workings` and `charts` moved there unchanged, with `leptoskit_admin` importing them, so the unconverted pages render exactly as before. The rail, the tab bar, the brand, the project switcher and the live pulse are shared too.

- **How a page moves:** add a `model::Page` variant with the page's data (serde), build its content in `pages/`, and route it through `server::hydrated_screen`, which renders `leptoskit_admin_pages::document` through `leptos_axum` with the `Model` as context. A moved page loads no `admin.mjs`, and its stylesheet (`shell::stylesheet`) has none of `shell_css`'s attribute state rules: the shell's signals do that work. Old pages keep the old shell (`leptoskit_admin/src/shell.rs`) and the script until they move. Never mix the two on one page.
- **First page: Glossary.** The hydrated `shell::Shell` holds the theme (`leptoskit::appearance::set` plus the `admin_theme` cookie), Workings mode (POST `/api/preferences`), the account menu (`aria::Menu`, test ids `avatar`, `avatar-menu`, `sign-out`; signs out through `leptoskit::auth`'s `sign_out_mutation`), search (results rendered from `shell::search_targets`, Enter opens the first), and the shortcuts ⌘K/Ctrl+K and Shift+W through `use_shortcut`.
- **Serving:** `AdminBuilder::client_dir`, `lk admin serve --client-dir` or `LEPTOSKIT_ADMIN_CLIENT_DIR`, and `admin_demo` reads the same variable. The client is served from `{BASE}/pkg/`. Without one, moved pages render but don't respond. How the client ships with an installed `lk` is ifi's decision (asked in the thread); until then it is built from a checkout.
- **Tests:** `script.mjs` builds the client with `leptoskit build --platform web` unless `LEPTOSKIT_ADMIN_CLIENT` names one, serves `glossary.html` from `pages_for_the_script`, and checks the theme, Shift+W, Ctrl+K and search, and signing out.
- **Next:** see part 2.

### `feat/admin-learn-pages`: admin conversion, part 2

Learn's hub and lessons moved. The lessons and their checks stay in `leptoskit_admin/src/learn.rs`, because the checks read server-side state (projects, operations, analytics facts, preferences); it now builds `model::Learn` and `model::Lesson` values (rows with their evidence, the lesson up next, each lesson's target). `leptoskit_admin_pages/src/pages/learn.rs` draws them, with the same test ids. `server::hydrated_screen` takes a closure over the `Proof` so a page can be built from it, and answers 404 when it returns `None` (an unknown lesson).

- **Tests:** the `learn` and `analytics_learn` snapshots didn't change, so the server's markup is the same as before. `script.mjs` serves `learn.html` and checks that it hydrates without `admin.mjs`, that the theme flips, and that a lesson row opens its lesson.
- **Next:** infrastructure with its drawers and actions, the analytics builder, Live, People and the dialogs, and last delete `admin.mjs` and the attribute rules in `shell_css`.

### `feat/public-demo`: pull request #856 (merged)

Twenty-three commits: the public demo service. One process serves one UTC day's generation, resets itself at midnight by re-executing in the same PID, gives each browser its own visitor pass, inbox and real sign-in, hands off to the admin, and lets visitors save private notes that emit one real `note_created` analytics event. [public-demo.md](public-demo.md) on that branch is the operator contract.

- **Verified:** `cargo test -p demo_service` passed 123 tests before the last test-only commit. The last commit stabilises `tests/lifecycle.rs`; that suite then passed 10 consecutive runs of 21 tests.
- **Blocked:** the branch cannot merge as it is. `lint:monostyle` scores the tree at 79.8 for complexity against a floor of 80, and the file it names is `examples/demo_service/assets/demo.js`. Do not patch that file to pass. It is replaced by the conversion.
- **Linux:** the descriptor and lifecycle tests now run on Linux (see `feat/demo-front-door`).
- **Not verified:** nothing is deployed. systemd behaviour, the edge deployment, the admin hand-over in a real browser across a process restart, and the link from the website are all outstanding.
- **Known gaps:** the admin's Live page sometimes needs a reload before a new event appears (about one run in four). The People page cannot find a demo visitor, because a visitor has no person row.

### `feat/demo-front-door`: stacked on `feat/public-demo`

The first of the demo's conversions: the front door, the band and the new-day dialog are hydrated leptoskit, and that page loads no `demo.js`.

- **Shape:** the pages live in `examples/demo_service/pages` (crate `demo_pages`, `cdylib` and `rlib`). The service renders them with `leptos_axum` and serves the client `leptoskit build --release --platform web` made, from `LEPTOS_SITE_ROOT/pkg`, read into memory at start. `public-demo.md` has the details under "Hydrated pages".
- **Choices made here, for the owner to confirm or change:**
  - Operations are declared in `demo_pages::ops` and call a port the service implements per request. The service keeps its routes, stores and transactions.
  - The pass cookie is set by a demo-only layer around the mutation. No leptoskit API was added for response headers.
  - The rpc server's receipts sit in a private in-memory SQLite database, so the generation format did not change.
  - Only allow-listed operations are served (`SERVED`).
  - The service refuses to start without the built client. That is a new step for whoever deploys it.
- **Verified on Linux (x86-64, cloud container):** `cargo test -p demo_service`, and `tests/visitor_pages.mjs` in headless Chrome against the debug service with the release client. The Linux-only descriptor tests ran for the first time and pass. Two tests fail only for the container: one needs IPv6 loopback, one needs to not be root.
- **Client size:** 352 KiB brotli for the first load (345 KiB Wasm, 7 KiB loader), with `optimize = "smallest"`.
- **Suite changes:** the harness waits for `data-hydrated` on the root element before it acts on a page that brings the client. Assertions that read `#demo-state` on the front door now read the markup.
- **Next:** sign-in, on `feat/demo-sign-in`.

### `feat/demo-sign-in`: stacked on `feat/demo-front-door`

Sign-in is hydrated leptoskit (`pages/src/sign_in.rs`), and loads no `demo.js`. The old server-drawn page (`src/pages/sign_in.rs`) is gone.

- **Shape:** the choice is `aria::RadioGroup`, the stations are `aria::Tabs`, focus moves with `FocusRequest` and the inbox scrolls into view with `Reveal`. Sending and checking the code use `leptoskit::auth::client` through `use_mutation`, so `App` provides a `QueryClient`. The inbox is a new query, `inbox`, on the same port as `mint`.
- **Service change:** the pass gate's `403 not_your_identity` now carries a `message`, because Auth's client reads a body without one as `server`.
- **Classes the compiler refuses**, moved to `assets/demo.css` by test id: `ring-*`, `border-dashed`, `underline` and `decoration-*`, `line-through`, `whitespace-pre-wrap`. They are candidates for the class compiler.
- **Client size:** 506 KiB brotli for the first load, up from 352: Auth's client and the extra widgets. The budget in `pages/leptoskit.toml` is now 520 and says why. This is double the other examples' budget and worth a look before release.
- **Needs `fix/web-semantic-rebuild`:** without it every rebuild drops focus inside `<main>`, and the keyboard steps fail.
- **Suite changes:** sign-in assertions read the markup, the address, and the `<noscript>` form instead of `#demo-state`; the radio options are `#choose-option-0` and `-1`; the inbox request is `/rpc/inbox`; the stations step waits for each focus move, because leptoskit moves focus on the next frame where `demo.js` moved it at once. `tests/signin.rs` checks elements with a `shows` helper, because the stylesheet names the same test ids.
- **Still there:** `GET /demo/inbox`, used by the Rust proofs and by nothing in a page.
- **Next:** Notes, on `feat/demo-notes`.

### `feat/demo-notes`: stacked on `feat/demo-sign-in`

Notes is hydrated leptoskit (`pages/src/notes.rs`), and `assets/demo.js` is deleted. The old server-drawn pages (`src/pages/notes.rs`, `src/pages/chrome.rs`) are gone; `src/pages` keeps only the script-free "opening today's demo" page.

- **Shape:** the notebook is a `TextArea` with the count, a field error tied by `aria-describedby`, and Ctrl or ⌘ + Enter through `on:key` with `KeyPress::meta`. The account menu is `aria::Menu`; the page sets test ids on the menu's parts after building it, and **Open the admin** in the menu opens a new tab with `window.open`, because a menu item that is a link has its click handled. The admin links are `Pressable` with `LinkTarget::NewTab`.
- **Two new operations** on the same port: `notes` (a query) and `save_note` (a mutation). They share the ownership gate with `/demo/notes`: `site::resolve_owner` was factored out of the JSON routes' `owner`. Refusals are `Refused` with status `401` (`no_pass`, `new_day`, `signed_out`), `422` (`empty_note`, `note_too_long`) and `409` (`notes_full`); a cross-origin save is `403`. `RequestBackend` reads the session from `CurrentSession` in the request's extensions.
- **Still there:** `GET` and `POST /demo/notes`, which the Rust proofs drive and no page calls, beside `/demo/inbox` and `/demo/generation`. Removing them means moving `tests/notes.rs` onto the operations.
- **Client size:** 524 KiB brotli; the budget is now 540.
- **CSS:** every rule for the script's classes (`.msg`, `.mail`, `.note`, `.toast`, `data-js`, `data-reset`, `data-shown`) is gone. What is left names elements by test id.
- **Suite changes:** the notebook's recorder watches `/rpc/notes` and `/rpc/save_note` (a save answers `200`, not `201`, and a dropped attempt is recorded with status `0`); results are present or absent rather than `data-shown`; a note's text is `[data-testid="note-body"]`; the new note is `#note-new`; announcements are `[data-testid="announcer"]`; the toast is `[data-testid="toast"]`; a failed sign-out closes the menu, so the retry opens it again; the new-day dialog is a modal `<dialog>`, and the stream counter counts WebSockets too.

### The canvist editor stack: #857, #881, #884, and `feat/canvist-ink`

Four layers, in the order [The canvist editor](#the-canvist-editor) sets. Merge them in this order; each needs the ones before it.

- **#857, `feat/leptoskit-canvist-web`, on `main`.** `crates/leptoskit_canvist`: the `CanvistState` handle on every target, and the web host for plain paragraphs behind the `web` feature, all Rust through `web-sys`. The unbuilt `wip` commit this section used to warn about was reviewed against the brief and almost all rewritten. 19 native tests and 24 browser tests pass in headless Chrome. The example page is 259 KB gzipped with the `smallest` settings.
- **#881, `feat/canvist-element-kind`, on `feat/text-area`.** `NativeKind::CanvistEditor` and `CanvistEditor(id)` in the core, following the host-owned surface pattern of `Scene3d`. The web renders an empty `<div data-lk-canvist-editor="id">`; every native backend holds an empty view, the terminal draws a dim block, and the watch refuses it. Nothing in core depends on canvist.
- **#884, `feat/canvist-editor-component`, on #881's branch with #857's two commits.** Joins them: asking a `CanvistState` for its `id()` starts a watcher in the page that puts the editor in the `CanvistEditor` element that names it, follows the element's placeholder, `read_only`, label, `focused` and classes, and takes the editor out when the element leaves. A server-rendered page gets its editor when it hydrates. Browser tests cover each of those.
- **`feat/canvist-ink`, on #884's branch.** The `ink` feature: `Extensions::ink()`, `Pen`, `insert_drawing()`, `undo()` and `redo()` on the handle, and pointer drawing in the web host (`web/editor/ink.rs` holds the stroke being drawn, `web/input/ink.rs` the pointer and touch handling). `examples/sketchnote` is the first public example: a notebook page that opens with a sketch already drawn. 15 browser tests draw with synthetic mouse, pen and touch events and read the canvas back, and the example has its own browser test. The feature adds 11 KB gzipped to the editor page; the example's whole web build is 501 KB gzipped.

- **Run on Linux:** the tests and clippy each pull request lists, for the host and for `wasm32-unknown-unknown`, and the browser tests in headless Chrome.
- **Not run anywhere:** Safari and Firefox, a real input method, a screen reader, and every native platform beyond compiling.
- **Known gaps:** paragraphs and drawings only, plain-text clipboard, no `EditContext`, no server-rendered first paint, and no native editor. The native hosts come next.
- **Ink has not met a real device.** Touch and pen behaviour (no scroll while drawing, `touch-action: none` under a hovering pen, no on-screen keyboard) is tested with synthetic events in Chrome only. Try a phone, an iPad with a Pencil, and Safari before calling it done. There is no eraser, no pressure, and no way to resize a sheet.
- **A canvist bug the host works around.** At the pinned revision, `DocumentLayout::apply` does not read a drawing's strokes again when a commit changed only the block's attributes, which is all that adding or undoing a stroke changes, so the new stroke never reaches the display list. `Editor::reread_drawings` calls `layout.resync` after any commit that touched an ink block. Report it upstream, and drop the workaround when a newer pin fixes it; `each_stroke_is_one_step_of_undo` in `tests/ink.rs` fails without one or the other.
- **The example has no player on the docs site.** `docs/src/examples.md` lists it with the commands to run it. Adding it to the players there is its own change.
- **The example's `web/index.html` holds the two-line inline module loader** that `leptoskit new` writes for every app (`import init from "./pkg/…js"; await init();`). It is not behaviour, and it goes when the scaffold's does.

### `docs/js-to-wasm-handover`

This file and the two reports.

### Leave alone

- #555 is the monochange release pull request. This work does not touch it.
- Other agents push to this repository constantly. `main` moves often; rebase onto it and never merge it in.

## The canvist editor

The owner wants leptoskit to ship a built-in editor component powered by [canvist](https://github.com/canvist/canvist), a canvas-based text editor written in Rust. It is an optional module, like analytics.

- **leptoskit owns the host code.** It is built in this repository. Nothing is upstreamed to canvist, and canvist is never edited. This overrides the design report's plan to do the browser host as pull requests on canvist.
- **Names follow canvist:** crate `leptoskit_canvist`, state handle `CanvistState`, feature `canvist`. This overrides the report's `leptoskit_rich_text` and `RichTextEditor`. canvist has no leptoskit crate of its own as of this date; if one appears, match its name.
- **Depend on canvist's engine crates only,** each with `default-features = false`: `canvist_headless`, `canvist_layout` (features `editor` and `json`), and the extension crates. Pin them to one git revision. Do not depend on `canvist_web`, `canvist_ffi`, `canvist_canvas*`, the `canvist` facade, or any canvist JavaScript package. Because of the git dependencies the crate is `publish = false` until canvist releases again.
- **The browser host is Rust.** Port the measurer, the painter and the input handling from canvist's `crates/canvist_web` and `packages/canvist-web/src/*.ts` into `leptoskit_canvist`, keeping its WebKit font-resolution, ligature-probe and pixel-snapping fixes. Do not build canvist's JS `HostMeasurer` out of Rust closures.
- **The app owns the editor through a handle,** not a value prop, because replacing the whole document on every keystroke breaks IME, selection and undo. canvist's own types stay out of the public API.
- **On native, the editor lives in the host app,** not the Wasm guest. Platform text input needs synchronous answers on the UI thread.
- **The first public example leads with ink drawing mixed into text,** not a toolbar of bold and italic.
- **The `leptoskit` feature `canvist` waits for canvist to release.** Cargo refuses to publish a crate that depends on an unpublished one, even optionally, and `leptoskit_canvist` is `publish = false` while canvist is git-pinned. Until then an app depends on `leptoskit_canvist` itself, by git, and writes `CanvistEditor(note.id())` with the element `leptoskit` already has. When canvist is on crates.io: move the pins to versions, drop `publish = false`, add `canvist = ["dep:leptoskit_canvist"]` to `leptoskit` with a `leptoskit::canvist` re-export forwarded through `web`, and add the crate to the release group in `monochange.toml`. Decided 2026-11-18.

Order: finish the web editor for plain paragraphs in `leptoskit_canvist`; add the core element kind on every platform after `TextArea` lands, since both change the same core files; then ink; then iOS and Android, macOS, GTK and Windows.

## What remains for a release

**The public demo going live:**

1. Close framework gaps 1–12 above.
2. Rebuild the demo's pages as hydrated leptoskit and delete `demo.js`.
3. Open the demo's pull request and get CI green.
4. Run the Linux-only tests and check systemd behaviour: `Restart=on-failure`, SIGTERM, the midnight re-exec.
5. Deploy behind the edge: separate hostnames for Notes and the admin, a check that the edge overwrites `X-Forwarded-For`, TLS, and a `0700` state volume.
6. Prove the admin hand-over in a real browser across a process restart.
7. Link the demo from the website.

**PostHog-level analytics** is tracked in [analytics-service.md](analytics-service.md) §21. Phase 1 is merged. Feature flags are next, then cohorts and the rest of insights, session replay, experiments, surveys, error tracking, and a PostHog adapter. On the infrastructure side, plan, apply, deploy and rollback have never run against a real cloud provider, and the admin's tests have not run on Postgres.

## Lessons from this stretch of work

- **A child's "lint passed" is not evidence.** One implementation agent reported `lint:all` as passing when clippy, ktlint and swiftlint all failed, and it had only linted Swift that did not compile. Run the gate yourself, or read its real output.
- **Compile the Swift and Kotlin renderers.** The tests `generated_app_swift_typechecks_with_and_without_the_notifications_module` in `leptoskit_cli` do it for Swift.
- **Adding a field to `NativeProps` changes every element hash,** so the gallery tree snapshots change. Regenerate them and check that only the hashes differ.
- **Never kill processes by name.** An agent that ran `pkill -9 -f cargo` on a shared machine killed every other session's builds for an hour.
