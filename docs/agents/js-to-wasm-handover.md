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

| #  | Change                                                      | Gap      | State       |
| -- | ----------------------------------------------------------- | -------- | ----------- |
| 1  | Text input hints                                            | G9       | #852, open  |
| 2  | `TextArea`                                                  | G12      | #877, open  |
| 3  | Labels that focus their fields                              | G13      | #879, open  |
| 4  | One-shot focus requests                                     | G2       | #887, open  |
| 5  | Arrow-key navigation for tabs, radio groups and menus       | G5       | #892, open  |
| 6  | `Modal` and `Popover` (GTK and Windows still to come)       | G1       | #896, open  |
| 7  | Scroll into view                                            | G4       | not started |
| 8  | A clock hook                                                | G6       | not started |
| 9  | Typed refusals in the rpc client (status, code, retry time) | G8       | not started |
| 10 | Streams without a database                                  | G3       | not started |
| 11 | The ⌘ key on the web                                        | G7a      | not started |
| 12 | New-tab and download links                                  | G14, G15 | not started |

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

### `feat/overlays-android`: stacked on `feat/overlays-uikit`

The third part of G1, for Android: both hosts.

- **`leptoskit_android`** returns `true` from `presents_overlays` and maps event code `12` to `Dismiss`. In `android/WidgetHost.kt` a node with `presented` set never has its view added to its parent's: `InsertChild` skips it and counts only the siblings that are not overlays. `refreshOverlays` runs at the end of each `apply` and from `mount`. A `Modal` is a `ModalDialog`, a `Dialog` whose `cancel()` only emits `Dismiss`, so Back, Escape and a touch outside all ask and nothing closes. Its content view is as large as the union of the modal's children, and the modal's own view is shifted by negative margins so those children land at the dialog's corner. A `Popover` is a focusable `PopupWindow` shown with `showAtLocation` at the frame layout gave it. Its touch interceptor emits `Dismiss` for a touch outside and consumes it.
- **Back on a popup cannot be refused** without subclassing the popup's decor view, so the popup closes, `setOnDismissListener` emits `Dismiss`, and the host shows it again at once if the tree still has it open (`suppress { refreshOverlays() }`). The Kotlin renderer for generated apps cannot do that, because its events reach the app asynchronously: there the popup returns with the next batch.
- **The Kotlin renderer** (`mobile/NativeOverlayView.kt`, in the CLI's `platform_sources::ANDROID`, `Package.swift`'s exclude list and `setup/android_widget_paint`) keeps a `GONE` place holder in the tree and puts children into its `content`, as the Swift renderers do. `WidgetProps` gains `name` (the element's `id`) and `overlay`. Its `DISMISS` is `10`, the bridge's code; the crate's is `12`. The bridge records with `presenting_overlays()` on macOS, iOS and Android.
- **Two fixes this branch needed were pushed to the pull requests that broke the build**, and are cherry-picked here: `import android.text.InputFilter` in `WidgetHost.kt` (#852) and the `Dismiss` arm in `src/probe.rs` (#896). The branches between do not have them until the stack merges in order.
- **Checked here:** `WidgetHost.kt`, `ProbeActivity.kt` and every Kotlin file under `mobile/` and `android/` type-check with `kotlinc` against the API 34 `android.jar` (one error that was already there: `VANILLA_ICE_CREAM` is API 35). `ktlint` is clean. The Rust side is cross-linted for `aarch64-linux-android` with the `probe` feature. **Run here:** nothing: there is no emulator.
- **Tests that CI's Android jobs will be the first to run:** `verifyOverlays` in `android-tests/ProbeActivity.kt` (a real Activity: dialog size and place, popup place, Back and outside touches ask, a removed popover comes down) and `anOverlayKeepsItsPlaceAndWaitsForAWindow` in the bridge's `NativeControlsTest.kt`, which has no window and so checks only the tree.
- **Known limits:** arrow keys in a `Menu` do nothing, because neither host delivers `on_key`. A dialog needs an Activity. A modal's own background is not drawn.

### `feat/public-demo`: no pull request

Twenty-three commits: the public demo service. One process serves one UTC day's generation, resets itself at midnight by re-executing in the same PID, gives each browser its own visitor pass, inbox and real sign-in, hands off to the admin, and lets visitors save private notes that emit one real `note_created` analytics event. [public-demo.md](public-demo.md) on that branch is the operator contract.

- **Verified:** `cargo test -p demo_service` passed 123 tests before the last test-only commit. The last commit stabilises `tests/lifecycle.rs`; that suite then passed 10 consecutive runs of 21 tests.
- **Blocked:** the branch cannot merge as it is. `lint:monostyle` scores the tree at 79.8 for complexity against a floor of 80, and the file it names is `examples/demo_service/assets/demo.js`. Do not patch that file to pass. It is replaced by the conversion.
- **Not verified:** the Linux-only descriptor and lifecycle tests have never run (all work so far was on macOS). Nothing is deployed. systemd behaviour, the edge deployment, the admin hand-over in a real browser across a process restart, and the link from the website are all outstanding.
- **Known gaps:** the admin's Live page sometimes needs a reload before a new event appears (about one run in four). The People page cannot find a demo visitor, because a visitor has no person row.

### `feat/leptoskit-canvist-web`: no pull request, work in progress

One commit marked `wip`. It adds `crates/leptoskit_canvist`, about 2,650 lines: `state.rs`, and under `web/` a measurer, a Canvas2D painter, input, theme and the attach point, plus `tests/native.rs` and `tests/web.rs`. It also touches the workspace `Cargo.toml`, `Cargo.lock`, `deny.toml` (canvist allowed as a git source) and `devenv.nix` (a `test:browser` line), and adds a changeset.

**Treat it as unreviewed and unbuilt.** The agent writing it was stopped mid-task. Nobody has confirmed that it compiles, that its tests pass, or that it follows the brief. `Cargo.lock` was carried across a rebase and may need regenerating. Review it against [The canvist editor](#the-canvist-editor) before building on it, and rewrite what falls short.

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
