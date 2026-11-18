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
| 4  | One-shot focus requests                                     | G2       | PR open     |
| 5  | Arrow-key navigation for tabs, radio groups and menus       | G5       | not started |
| 6  | `Modal` and `Popover`                                       | G1       | not started |
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
