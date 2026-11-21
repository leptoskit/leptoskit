# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-21)

### Features

#### Keyboard shortcuts for the whole app on the web

_Owner:_ Ifiok Jr. · _Introduced in:_ [8de3f4c](https://github.com/leptoskit/leptoskit/commit/8de3f4c497639561553ce1d2767baabbe41f2933)

`use_shortcut` runs a function when the person presses a key, wherever focus is, even when nothing has focus. It suits ⌘K to search or a single letter that opens a drawer, which `on_key` can't express because it only hears keys pressed inside its element.

```rust
let search = FocusRequest::new();
use_shortcut(Shortcut::primary("k"), move || search.request());
use_shortcut(Shortcut::key("w").unless_typing(), move || toggle_drawer());
```

`Shortcut::primary` holds ⌘ on Apple devices and Ctrl elsewhere. `.shift()` and `.alt()` add modifiers, and `.unless_typing()` leaves the key alone in a text field. A shortcut stops when its component goes, the newest of two matching shortcuts wins, and a key an element's `on_key` handled never reaches one.

Shortcuts work on the web today. Native hosts and the terminal don't deliver keys to them yet, so there a shortcut does nothing for now.

#### Ask a provider to sign in again

_Owner:_ Ifiok Jr. · _Introduced in:_ [f25ea8c](https://github.com/leptoskit/leptoskit/commit/f25ea8c94f5875355ce081afae5ac92a652cee48)

`StartProvider` has a `prompt` field. `Some(Prompt::Login)` asks the provider to sign the person in again even when someone is already signed in there, which is how a person switches to another account. `Some(Prompt::SelectAccount)` shows the provider's account picker. The value reaches the provider's authorization page as OpenID Connect's `prompt` parameter, and a provider that doesn't know the parameter ignores it.

```rust
sign_in.mutate(StartProvider {
	provider: "main".into(),
	redirect_to: Some("/admin/sign-in".into()),
	prompt: Some(Prompt::Login),
});
```

Code that builds a `StartProvider` needs `prompt: None` to keep today's behaviour.

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

- **Gestures on a view.** A view can follow a finger. `gesture(Gesture::tap())`, `pan`, `long_press`, `pinch`, `rotation`, and `fling` recognize the pointer in one arena shared by the Android widget backend and Linux. `simultaneous`, `exclusive`, and `race` compose recognizers the way a pinch and a rotation share one gesture, or a pan wins over a tap. A drag that wins claims the pointer so the control underneath does not also click. A tap does not, so a button still presses. iOS, macOS, Windows, the web, the terminal, and the generated Swift and Kotlin renderers still deliver presses and clicks only. _Owner:_ Ifiok Jr. · _Introduced in:_ [955a1bf](https://github.com/leptoskit/leptoskit/commit/955a1bf7d00666992a8ccd52032131c7ae97cca1)

#### A field's label focuses it on the web

_Owner:_ Ifiok Jr. · _Introduced in:_ [0170294](https://github.com/leptoskit/leptoskit/commit/0170294ffef2da84f21c303fb7859fc8eec160e8)

A `Text` that names a field through `aria_labelled_by` now renders as `<label for="…">` on the web, so a click on the label focuses a `TextInput`, `PasswordInput`, `TextArea` or `Slider` and toggles a `Checkbox`, `Switch` or `Radio`, with no handler and no script. It applies when the field has an `id`, is named by exactly one plain `Text`, and no other field is named after the same text. `leptoskit_aria::Field` and `leptoskit::form` already build this shape. The markup for such a text changes from `<span>` to `<label>`; its styles and its `id` stay the same.

On Android, the label's view gets `labelFor`, so TalkBack knows which field it names. Other platforms are unchanged: clicking a label is not a convention there, and the field already carries the label's text as its name.

#### Open links in a new tab, save them, or open them natively

_Owner:_ Ifiok Jr. · _Introduced in:_ [e5e6912](https://github.com/leptoskit/leptoskit/commit/e5e6912ebad6fcc21582314d0aa6355cca54c554)

A `Pressable` link can now say where it opens and whether it is a download.

```rust
Pressable("Open the admin")
	.href("https://admin.example.com/")
	.target(LinkTarget::NewTab)
	.child(Text("Open the admin ↗"));

Pressable("Export").href("/people/7/export").download("person-7.json");
```

On the web, `LinkTarget::NewTab` renders `target="_blank" rel="noopener noreferrer"` and tells screen reader users the link opens in a new tab. `download(name)` renders `<a href download>`, so the browser saves the file under that name. A name that is not one plain file name is ignored.

Native apps used to ignore `href`. With the new `links` feature, pressing a link that has no `on_press` handler opens an absolute HTTP(S), `mailto:`, `tel:` or `sms:` link outside the app, through `leptoskit_plugin_linking`: directly on macOS, Windows and Linux, and through the host's `linking` plugin in a Wasm guest on iOS and Android. A link with a handler, a relative link and a disabled link behave as before.

- **Person attributes, segments, and send to everyone.** The service keeps a registry of the people it knows — registered when the app's server sets their attributes, when they subscribe to a topic, set a category preference, or enroll a push endpoint, and backfilled from those tables by the migration; a person named only in a recipients send is never registered. The app's server describes people with `set_person_attributes` (batch, ≤1,000 people, idempotent; keys `a-z0-9_` ≤64 bytes, values ≤256 bytes, ≤50 attributes per person with cap-skipping counted; `person_attributes` reads back; `forget_person` removes the person from the registry with their attributes and topic subscriptions, keeping their inbox) — app-only by design, so a segment can't be forged from the client side. The condition grammar extends FCM's with `user.<key> == 'v'`, `!=`, `in ['a','b']`, `has(user.<key>)`, and finite-number `<`, `<=`, `>`, `>=` (true only when both sides parse as finite numbers; `NaN`/`inf` literals refused), mixing freely with `'t' in topics` under `&&`/`||`/`!`/parens with byte-offset errors and bounds (≤5 topics, ≤10 attribute predicates, depth ≤8, ≤1 KiB); an absent attribute answers false to every predicate, and the everyone-in-disguise refusal now checks topics _and_ attributes. `SendTarget::Everyone` reaches every registered person, and a condition no longer needs to be anchored on a topic: one that structurally requires a topic still fans out over the topic index, while an attribute-anchored one (and everyone) fans out over the registry — same batched, cursor-advancing, idempotent fanout job, same opt-out/visibility/push/live behaviour, `validate_only` counting every target. SQLite and Postgres migrations add `persons` and `person_attributes` with the registry backfill. _Owner:_ Ifiok Jr. · _Introduced in:_ [644b0a3](https://github.com/leptoskit/leptoskit/commit/644b0a354e16352b37cfb3c78934d5c4e5b76a6e)
- **Add `leptoskit_notifications`, the first-party notifications service: an app's server sends idempotently to explicit user ids, and each recipient gets one durable inbox row in the service's own database — scheduled visibility with cancel-before-due, expiry, category opt-outs enforced at send and at visibility, and per-person read and dismissed state shared across devices, read through bounded, live-paginated queries on every platform the app's screens run.** Time reaches open screens: every send enqueues a wake job when it becomes visible and at its expiry, so a scheduled item appears and an expired one leaves a watching screen without any other write; preference changes update open inboxes the same way. An accepted send answers its stored outcome forever, however the clock moves on, and a known zero-fanout send cancels idempotently. Delivery is authenticated inbox retrieval while the app runs; OS background push (APNs, FCM, Web Push) and device enrollment are not part of this change. The `leptoskit` facade re-exports the shared types behind a `notifications` feature, and the guide is under Services. _Owner:_ Ifiok Jr. · _Introduced in:_ [b782f13](https://github.com/leptoskit/leptoskit/commit/b782f1372a8b11ae1338a003c937282609188021)
- **Audience partitioning by topics and conditions, with a durable batched fanout.** A send can now target `SendTarget::Topic` or `SendTarget::Condition` beside today's `SendTarget::Recipients`: the app's server subscribes people to per-person topics in idempotent batches (`subscribe_to_topic`/`unsubscribe_from_topic`, at most 1,000 people per call; a person holds at most 500 topics, and a full person is skipped and counted rather than refusing the batch), and a person may subscribe themselves (`subscribe`, `unsubscribe`, `my_topics`, live and paginated) to the topics the operator opened in `LEPTOSKIT_NOTIFICATIONS_OPEN_TOPICS` — patterns like `news.*`, nothing open by default, and the person always read from the forwarded identity. Conditions use FCM's grammar verbatim (`'a' in topics && ('b' in topics || !('c' in topics))`), validated on both sides with byte-offset errors, FCM's bounds (≤5 topics, depth ≤8, ≤1 KiB), and unanchored conditions — ones true for a person holding no topics — refused, since there is no everyone audience. A topic or condition send answers `fanning_out` immediately and a job walks the audience in 500-person batches — one transaction per batch for the inbox rows, push deliveries, and the stored cursor, unique per send and person so a crashed or replayed batch never duplicates, the audience snapshotted as the fanout reaches each person. `send_status` answers the state and counts; `validate_only` counts the audience without writing; `cancel` stops a fanout mid-walk and cancels the rows written so far. SQLite and Postgres migrations add the `topic_subscriptions` table, the fanout bookkeeping on `sends`, and the per-send-per-person uniqueness on `notifications` (existing rows backfilled). Breaking: `Send` and `check_send` take the target — `recipients: vec![…]` becomes `target: SendTarget::Recipients(vec![…])` — and `SendResult` gains `state` and `targeted`. _Owner:_ Ifiok Jr. · _Introduced in:_ [ed7b1e7](https://github.com/leptoskit/leptoskit/commit/ed7b1e77bb3ac93482b58819cdad9df80fdaa166)

#### Modal and Popover show content above the screen

_Owner:_ Ifiok Jr. · _Introduced in:_ [08a70cb](https://github.com/leptoskit/leptoskit/commit/08a70cb0dbd8a153b2b8eb285099920fb6d2252c)

`Modal(open)` blocks the screen until it is answered, and `Popover(anchor, open)` hangs from the element with that `id`. Both stay in the tree while closed. `.on_dismiss(...)` hears Escape and a press outside as a request to close, and the overlay closes when the next build says so. `.initial_focus(id)` names what takes focus on opening, and `.placement(Placement::BelowEnd)` picks a popover's side. In `view!` they are `<Modal open=…>` and `<Popover anchor=… open=…>`.

On the web a `Modal` is a `<dialog>` opened with `showModal()` and a `Popover` uses the `popover` attribute, so the browser supplies the top layer, the inert page, the focus trap and focus restore. The terminal draws overlays last, keeps Tab inside an open modal, and gives focus back on close.

macOS, iOS, Android, Linux and Windows draw an overlay as a layer in the right place for now. Nothing asks to dismiss it there, focus is not moved or kept inside, and a parent that clips or scrolls cuts it off.

For backend authors: `NativeKind` gains `Modal` and `Popover`, `NativeEvent` gains `Dismiss`, and `NativeProps` gains `overlay`. A backend that returns `true` from the new `NativeBackend::presents_overlays` is told the real kinds and given open overlays' frames in viewport coordinates. Every other backend is given a `View` with a frame relative to its parent. The guest event ABI gains kind 9 (dismiss), and the bridge event codes gain 10.

#### Add platform targeting and capability queries

_Owner:_ Ifiok Jr. · _Introduced in:_ [fee46c0](https://github.com/leptoskit/leptoskit/commit/fee46c0e80a311139a43d5b33ea3f559aae84df6)

`leptoskit::target` adds a shared runtime and compile-time platform API (`TargetPlatform`, `PlatformFamily`, `PlatformSwitch`, `PlatformCapabilities`, `Capability`, `CapabilitySupport`, `with_platform`, and `on_ios` / `on_android` / `on_mobile` / `on_macos` / `on_windows` / `on_linux` / `on_desktop` / `on_apple` / `on_web` / `on_extension` / `on_watch`) so universal apps can cleanly opt into platform-specific features and test every platform branch from any host.

`leptoskit_build::platforms()` now also recognizes `harmonyos`, `wearos`, `watchos`, and `quest` alongside the existing platform features.

#### Screen state and queries survive a rebuild

_Owner:_ Ifiok Jr. · _Introduced in:_ [c7a85ef](https://github.com/leptoskit/leptoskit/commit/c7a85ef05c4a1a510f6136b444ad4a0c2ef1af5e)

leptoskit builds a screen again whenever a signal it read changes. Two things broke under that: a `RwSignal::new(..)` inside a screen ran again with every rebuild, so a chosen filter or a typed field reset as soon as the screen redrew, and a `use_query` in a screen refetched in a loop, because the rebuild its data caused mounted the query again while the data already counted as stale.

- `leptoskit_native`: `remember(|| initial)` returns the same `RwSignal` on every build of the screen, told apart by its place in the source and by order. `remember_keyed(&key, || initial)` names the value by a key instead, for rows that move. A value is released once a whole build no longer reaches it, such as after navigating to another route. `guest::mount` keeps the values for native apps. In the browser, `web::screen(move || ..)` replaces `move || web::render(..)` and does the same. `ScreenState` is the store underneath, for hosts of their own. Both functions are in `leptoskit::prelude`, and the module is `leptoskit::remember`.
- `leptoskit_query`: a query mounted within two seconds of this client's last fetch of it takes that fetch's data or error instead of fetching again, like SWR's deduping interval. The window covers `RefetchOn::Always` and `retry_on_mount`, and not invalidation, `refetch()`, focus, reconnect, or intervals. Data from `set_query_data`, initial data, or hydration doesn't count as a fetch. Set it with `with_mount_dedupe` on `QueryDefaults`, `QueryOptions`, or `QueryObserverOptions`; `Duration::ZERO` gives TanStack's behaviour. `QueryOptions`, `QueryDefaults`, and `ResolvedQueryOptions` gain a `mount_dedupe` field, so code that builds them with struct literals needs it too.

An app with its own keyed store and a long stale time to work around this can switch to `remember` and drop both.

- **Set the browser tab's title.** `leptoskit_router::set_title("Plans · Admin")` names what the person is looking at. On the web it sets the document's title, so the tab and the history entry follow the screen as the person moves around without a page load. A server-rendered page still writes its first title in its own `<title>`. Native windows keep the title they were opened with for now, and on a phone it does nothing. _Owner:_ Ifiok Jr. · _Introduced in:_ [edcb32f](https://github.com/leptoskit/leptoskit/commit/edcb32f5b4424abc33d17a784c8afb896ea06a14)

#### Arrow keys move through tabs and radio cards

_Owner:_ Ifiok Jr. · _Introduced in:_ [bc7c851](https://github.com/leptoskit/leptoskit/commit/bc7c8510366147ca4e256b1a1f3896717f4c50d4)

`leptoskit_aria::Tabs` is now one stop for the Tab key. Only the selected tab is in the Tab order; Left and Right (Up and Down after `.orientation(Orientation::Vertical)`), Home and End move to another tab, select it, and take focus with it.

**Breaking:** the selection callback moved from each tab to the list. Write `tabs.list(move |index| …)` and `tabs.tab(index, label)` where you wrote `tabs.list()` and `tabs.tab(index, label, move |index| …)`. `Tabs` no longer implements `PartialEq`.

`leptoskit_aria::RadioGroup` is new: a choice of one option among several where each option shows any content, such as a card. `RadioGroup::new(id, labels, selected).option(index, content).on_change(…)` becomes an element with `Element::from`. Options carry the `radio` role, their checked state and their position; the group is one Tab stop, and the arrow keys, Home and End move, choose and take focus along.

The arrow keys work on the web and in the terminal. macOS, iOS, Android, Windows and GTK do not deliver key presses to the app yet, so every tab and option stays reachable with Tab there, as before.

`leptoskit_testing::WidgetTester::press_key(finder, key)` presses a key on a widget and bubbles it to the nearest `on_key` handler.

#### Scroll an element into view with `Reveal`

_Owner:_ Ifiok Jr. · _Introduced in:_ [6aef48a](https://github.com/leptoskit/leptoskit/commit/6aef48a580ac38f8dc06b281c49c9036faf04151)

A screen can now bring one element into view without knowing where layout put it. Create a `Reveal`, give it to the element with `reveal_on(&reveal, ScrollAlign::Nearest)`, and call `reveal.request()` from any handler. Each request scrolls once, after the next update, and the person scrolls freely afterwards. `ScrollAlign` is `Start`, `Center`, `End` or `Nearest`, and `Nearest` does nothing when the element is already in view.

On the web this is the browser's `scrollIntoView`, smooth unless the person asked for reduced motion. In native apps the surface works out the offset from layout and scrolls the nearest `ScrollView` around the element. Only that one view scrolls, and an element outside any `ScrollView` is left alone.

The GTK, Windows, AppKit and UIKit backend crates now apply `scroll_offset`, which they ignored before, so `Reveal` and `VirtualList`'s programmatic scrolling work in the generated Windows and Linux apps. GTK, Windows and AppKit jump to the offset; UIKit animates when asked. The terminal and HarmonyOS still don't scroll from code.

`NativeProps` gains a `reveal` field, which the surface answers itself and never sends to a backend.

#### Service workers and stronger extension messaging

_Owner:_ Ifiok Jr. · _Introduced in:_ [363ebad](https://github.com/leptoskit/leptoskit/commit/363ebadd2effe51d9b954c31ab0b6bcbfdc46ab8)

Web apps can enable the `service-worker` feature to register a worker script, exchange messages, describe caching strategies, open Cache Storage, and manage Web Push subscriptions. Calls return `Unavailable` outside a secure browser context.

Browser extensions gain long-lived `runtime::connect` ports (with `Background::on_connect`), storage `clear` / `keys` / `get_bytes_in_use`, and permissions `get_all` / `on_added` / `on_removed`. The generated background worker keeps ports alive so the wasm side can post and disconnect by id.

#### Serve streams from a server that has no database

_Owner:_ Ifiok Jr. · _Introduced in:_ [f7bf762](https://github.com/leptoskit/leptoskit/commit/f7bf762393787df34f5c07c85f9741e6cf6be5e9)

A `#[rpc::stream]` used to need the full live server, which reads a database's change log, so a service with something to stream and no database could not serve one.

- **`rpc::StreamCx`** is a new context for a stream's body. It holds who is calling (`get`, `require`, `signed_in`, `identity`) and no database. A stream that takes it runs on any live server. A stream that reads the database keeps `rpc::QueryCx`.
- **`rpc::server::LiveStreams`** serves streams alone, on the same routes as `Live` (the WebSocket, its server-sent events, and their frame posts) with the same limits. It takes `identify`, `origin`, `for_service`, `rpc_config`, `stall_timeout`, and `drain`. `Live::streams_only()` returns one too. A query, mutation, action, or channel sent to it is answered with a `501` that says why.
- **`live-streams`** is a new feature of `leptoskit_rpc` (`rpc-live-streams` through `leptoskit`). It needs no database driver and opens no connection. `live` now includes it.

Nothing changes for a client: `use_stream` reads either server.

Things to know when upgrading:

- A stream with no context argument no longer requires a database driver in the build. It needs `live-streams` or a driver, and `rpc::cx()` inside it works only where the server has a database.
- `StreamOperation::pump` and `StreamRegistration::open` now take the hidden `rpc::server::StreamOpen` in place of a `QueryCx`. Code generated by `#[rpc::stream]` follows by itself; a registration built by hand needs the new type.
- `LiveStreams` keeps an identity until its lease expires or the connection ends. It can't watch the rows a lease reads, because it has no change log.

#### TextArea: a field for several lines of text

_Owner:_ Ifiok Jr. · _Introduced in:_ [ce9d470](https://github.com/leptoskit/leptoskit/commit/ce9d470863cd327cdd87a36a12738c0fd356986c)

`TextArea()` is a new component, with its own `NativeKind::TextArea`, for notes, comments and messages. Enter adds a line instead of submitting, long lines wrap, and the field scrolls its own content. It is controlled like `TextInput`: `on_change` receives the proposed text with lines separated by `\n`, and `placeholder`, `read_only`, `max_length`, `capitalize`, `autocorrect` and `autocomplete` apply. It never raises `Submit`.

The web renders a `<textarea>`; macOS an `NSTextView` in an `NSScrollView`; iOS a `UITextView`; Android a multi-line `EditText`; Linux a `gtk::TextView` in a `gtk::ScrolledWindow`; Windows a multi-line `EDIT` control; the terminal a wrapping editor with line-wise arrow keys. The Swift and Kotlin renderers that generated apps use draw it too. GTK and Windows show no placeholder, and HarmonyOS and the watch surfaces do not draw the component.

`aria_multiline(true)` on a `TextInput` or `PasswordInput` now fails validation, because a single-line field that calls itself multi-line misreports itself to assistive technology. Use `TextArea` instead.

- **Typed text-input hints.** `TextInput` now carries the keyboard and text-editing hints that hand-written JavaScript used to provide, so every interface stays in leptoskit's own Wasm tree: `keyboard` (Text, Email, Number, Decimal, Phone, Url, Search), `read_only`, `max_length`, `capitalize` (None, Words, Sentences, Characters), `autocorrect`, and `submit_label` (Done, Go, Next, Search, Send). Each platform maps them to its own controls: the web to `type`, `inputmode`, `readonly`, `maxlength`, `autocapitalize`, `autocorrect`, `spellcheck`, and `enterkeyhint`; iOS to `keyboardType`, `autocapitalizationType`, `autocorrectionType`, `spellCheckingType`, and `returnKeyType`; Android to `inputType`, `imeOptions`, `InputFilter.LengthFilter`, and a cleared key listener for read-only fields; GTK to `set_input_purpose`, `set_editable`, `set_max_length`, and `InputHints`; macOS to `isEditable` and delegate truncation plus the field editor's correction and spell checking; Windows to `EM_SETREADONLY` and `EM_LIMITTEXT`. `read_only` implies `aria-readonly`. The Swift and Kotlin bridge renderers and the terminal backend honour the same props. _Owner:_ Ifiok Jr. · _Introduced in:_ [6d42b06](https://github.com/leptoskit/leptoskit/commit/6d42b068273560b572fa4c7f5f4475b33a03563b)
- **Ding-inspired toast pills.** Status toasts arrive as a 44px shape, spring open into a pill, morph in place when a newer message arrives, and shrink before they leave — the same phase model and spring tunings as [Ding](https://github.com/YusufCeng1z/ding). Hold a `Toaster`, call `success` / `error` / `warning` / `info` / `loading`, rebuild from `toaster.state()`, and advance phases with `poll`. `ToastViewport` draws the stack from shared builders, so every backend that already draws `View`, `Text`, `Button`, and `Vector` can show a toast. _Owner:_ Ifiok Jr. · _Introduced in:_ [ab77e98](https://github.com/leptoskit/leptoskit/commit/ab77e9807ae2c781c3d7a80ff5e00c270cfac4b6)

#### Add `use_clock`, a clock that advances by itself

_Owner:_ Ifiok Jr. · _Introduced in:_ [95d9169](https://github.com/leptoskit/leptoskit/commit/95d9169b4306a60430916d2d39ab8d4f4afe6785)

`leptoskit_use::use_clock(interval)` returns a `Clock` signal holding the time since it was called, updated about once per `interval` on the platform's own timer: `setTimeout` in the browser, the timer plugin in a native app, and tokio or a timer thread elsewhere. Pass it to `use_timeout_fn`, `use_interval_fn`, `signal_debounced` and the other hooks that take a clock, or read it in a `Text` closure for a countdown. Until now a host had to advance a clock signal itself.

The time is measured from the wall clock and never goes back, so a tab that slept catches up in one step. The timer stops when the owner the clock was made under is cleaned up.

It is behind a new `clock` feature, because it is the one hook in `leptoskit_use` that needs a task executor and depends on `leptoskit_query`. In `leptoskit`, the `clock` feature turns on `use` and `query` too.

### Fixes

- **Over the air updates get their own section.** The home page and README now lead with one component tree and real native widgets. Over-the-air updates have their own section, describing code push for installed native apps and which generated hosts install releases. _Owner:_ Ifiok Jr. · _Introduced in:_ [8a98548](https://github.com/leptoskit/leptoskit/commit/8a98548690f1654a63c9b6adffe8b603ba3bbfb5)

#### Check independent consumers before crate publication

_Owner:_ Ifiok Jr. · _Introduced in:_ [b6a1990](https://github.com/leptoskit/leptoskit/commit/b6a1990a574a7c077e4f31d408550ae64045a016)

Compile eight separate browser, guest, server and auth feature combinations from the actual Cargo package payloads before publication. Require complete package resolution and reject server features in client builds. Retain per-consumer build logs and exact archive and lockfile hashes; failed or incomplete runs produce no successful acceptance report.

Reject archive paths and directory names that alias by case on every host, so Linux validation also catches archives that cannot extract on a case-insensitive filesystem.

Run validation during release dry runs without publisher credentials. Before upload, require freshly prepared archives to match the hashes from the successful consumer run on the same revision.

- **Fix the platform module guide's path from the source root.** The creating-an-app guide now uses `components/heading` for a module in `src/components/heading.rs`, matching `platform_module!`'s source-root-relative paths. The generated application's end-to-end test uses the same corrected declaration and still verifies adding and removing a platform override. _Owner:_ Ifiok Jr. · _Introduced in:_ [d9a66d1](https://github.com/leptoskit/leptoskit/commit/d9a66d1e73141c14be24d3e57bcd4f1cf600bdca)
- **Add `leptoskit_plugin_background_task` for named OS background-task register/unregister/status/complete (`BGTaskScheduler` / `WorkManager`), distinct from `leptoskit_plugin_background` AppService keep-alive.** Guaranteed cadence and suspended Wasm execution remain later scope. _Owner:_ Ifiok Jr. · _Introduced in:_ [d5f4657](https://github.com/leptoskit/leptoskit/commit/d5f4657d76a38bde21d61af21eff755964a6ac13)
- **Add `leptoskit_plugin_calendar` for calendar permission, listing calendars (max 64), and creating or reading bounded events on iOS and Android.** Desktop and browser hosts report unavailable. _Owner:_ Ifiok Jr. · _Introduced in:_ [7b07d8f](https://github.com/leptoskit/leptoskit/commit/7b07d8f5999a34ede92c671fbe1c524dd49b7313)
- **Add `leptoskit_plugin_contacts` for contacts permission, system pick-one, and a bounded query (max 32) on iOS and Android.** Desktop and browser hosts report unavailable. _Owner:_ Ifiok Jr. · _Introduced in:_ [3820e86](https://github.com/leptoskit/leptoskit/commit/3820e86dfca1283e2ed1f36e4a34befdfc03a564)
- **Add `leptoskit_plugin_device_motion` for fused attitude, user acceleration, and rotation-rate readings on iOS, Android, and the browser.** Desktop hosts report unavailable. _Owner:_ Ifiok Jr. · _Introduced in:_ [dda0a88](https://github.com/leptoskit/leptoskit/commit/dda0a88a43b312383c51c7b0c1f46cdad7005e05)
- **Add `leptoskit_plugin_live_photo` with create/start/stop/status/release and a `LivePhoto` surface bound by session id (`PHLivePhotoView`).** Android and browser remain Unavailable; editing remains later scope. _Owner:_ Ifiok Jr. · _Introduced in:_ [13da9d6](https://github.com/leptoskit/leptoskit/commit/13da9d61f3252c056816f1da31516fd4e5040f9c)
- **Add `leptoskit_plugin_maps` with create/camera/markers/status/release and a `Map` surface bound by session id (MapKit / Google Maps).** User location and shapes remain later scope. _Owner:_ Ifiok Jr. · _Introduced in:_ [10ddb6f](https://github.com/leptoskit/leptoskit/commit/10ddb6fef86dc107d73054a9e2f2542f6e922da8)
- **Add `leptoskit_plugin_media_library` for bounded photo-library permission, asset listing, and saving an app-owned file into the system library.** Change observation remains later scope. _Owner:_ Ifiok Jr. · _Introduced in:_ [5e5b6cd](https://github.com/leptoskit/leptoskit/commit/5e5b6cd704fae36b0331982beb725e9a61d1a1e5)
- **Add `leptoskit_plugin_print` for HTML and PDF URI print dialogs on iOS, Android, and macOS.** Completion means the dialog finished or the platform accepted a job, never that paper left a printer. _Owner:_ Ifiok Jr. · _Introduced in:_ [47c9196](https://github.com/leptoskit/leptoskit/commit/47c9196669b4cabf82c712a83d94f2adeb8eb3dd)
- **Add `leptoskit_plugin_video_thumbnails` to extract one JPEG frame from an app-owned video URI at a time offset (`AVAssetImageGenerator` / `MediaMetadataRetriever`).** Multi-frame extraction remains later scope. _Owner:_ Ifiok Jr. · _Introduced in:_ [88c385c](https://github.com/leptoskit/leptoskit/commit/88c385c35553dfab40a5e0270b848bd2805f2367)

#### Keep focus across web rebuilds and inside modals

_Owner:_ Ifiok Jr. · _Introduced in:_ [a1dd169](https://github.com/leptoskit/leptoskit/commit/a1dd169266d9fc0c2bc17705cdcc0afe846fb584)

On the web, a screen that rebuilt replaced every landmark (`main`, `nav`, `header`, `footer`, `aside`, `article`, `section`, `search`), every heading, list, paragraph and label with a new element, instead of updating the one already there. Anything inside went with it: a text field lost its focus and its selection, and a person typing was dropped back to the top of the page.

Those elements now update in place, as `div` and `span` always did, so focus stays where it was after any rebuild.

A rebuild also no longer takes a node's `id`, `role`, `aria-*` and `tabindex` attributes off and puts them back: it sets the ones whose value changed and removes the ones that are gone. Before, a focused element that is only focusable through `tabindex`, such as a dialog's heading, lost focus to the page on every rebuild.

A `Modal` on the web now wraps Tab: past its last control focus comes back to its first, and Shift+Tab past its first goes to its last, instead of leaving the page for the browser's toolbar.

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

### Breaking changes

#### leptoskit: the one dependency

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit` is the crate an app depends on. It re-exports the components, Tailwind styles, layout engine, renderer, and Wasm runtime, with one feature per optional part, so an app names `leptoskit` alone and never keeps the versions of its parts in step by hand.

Pick how the interface runs: `web` with `csr`, `hydrate`, `ssr`, or `islands` renders through leptos, and a backend feature (`macos`, `ios`, `android`, `linux`, `windows`, or `tui`) commits the same tree as real widgets or as terminal cells. `server` adds the Axum toolkit, and `analytics` beside it keeps request and browser measurements. Everything else is opt-in: `rpc`, `query`, `form`, `aria`, `table`, `virtual`, `l10n`, `rive`, `db` with `db-postgres` or `db-sqlite`, `auth` (passwords, passkeys, and providers through their own `auth-*` features), `idp`, `extension`, `codepush`, and `devtools`. `leptoskit::secrets` reads values that Monosecret, the default secret manager, or a trusted host supplies.

### Features

- **Theme tokens in leptoskit.toml and deeper CSS var().** `[theme.colors]` and `[theme.lengths]` in `leptoskit.toml` name design tokens that `Theme::install_maps` loads into the process-wide Theme map. Arbitrary color and length utilities accept `var(--name)`, one nested `var()` fallback, and `/opacity` on color vars. The web adapter prepends `Theme::css()` so the browser sees the same `:root` custom properties native resolves through Theme. Tailwind `@theme` / `tailwind.config` files remain unsupported. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #538](https://github.com/leptoskit/leptoskit/pull/538)

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
