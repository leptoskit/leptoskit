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

#### Hear the ⌘ key in key handlers on the web

_Owner:_ Ifiok Jr. · _Introduced in:_ [25a25a8](https://github.com/leptoskit/leptoskit/commit/25a25a8f96aa4774c9f855eff3423c0061308c6f)

`on_key` on the web now hears keys pressed with ⌘ held. Until now it skipped every one of them, so a web app could not save on ⌘+Enter or open search on ⌘+K.

`KeyPress` gains a `meta` field for the command key, with `KeyPress::meta()` to build one in a test. `KeyPress::is_primary()` is true when the platform's shortcut modifier is held: ⌘ on a Mac, an iPhone or an iPad, and Ctrl everywhere else. In a browser it follows the device the browser runs on.

```rust
View().on_key(move |key| {
	if key.is_primary() && key.key == "Enter" {
		save();
		EventOutcome::Handled
	} else {
		EventOutcome::Unhandled
	}
})
```

A ⌘ key that no handler takes is still the browser's, so ⌘+R reloads and ⌘+L goes to the address bar as before.

Two things to check in existing code. `KeyPress` has a new public field, so a `KeyPress { .. }` literal needs `meta`. And `KeyPress::is` no longer matches while ⌘ is held, which only matters on the web, where those keys never arrived before. `Tabs`, `RadioGroup` and `Menu` leave ⌘ with an arrow key alone, since that is back and forward in a browser on a Mac.

A terminal is never told about ⌘, so `meta` is always false there. The native widget backends do not deliver key presses to `on_key` yet.

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

#### Camera sessions and Camera surface

_Owner:_ Ifiok Jr. · _Introduced in:_ [f58eb08](https://github.com/leptoskit/leptoskit/commit/f58eb08f0bec47c9958a5b0bb2d5a3d9792cbf28)

`leptoskit_plugin_camera` is a new runtime plugin, `camera`, for host-owned camera sessions. Create or release a session for the back or front lens, set torch when the hardware supports it (otherwise `Unavailable`), take a bounded still into app cache (URI + width/height/mime/size matching `image_picker` Asset framing), and snapshot status. Show the same session with a new `Camera(session_id)` native widget kind that binds a host-owned id the way `Scene3d` and `Video` bind theirs (requires an accessible label or `aria_hidden`).

Ask for camera access through the `permissions` plugin; this plugin returns `PermissionDenied` when capture lacks a grant. iOS and macOS use `AVCaptureSession` / `AVCaptureVideoPreviewLayer`; Android uses CameraX / `PreviewView`; the browser uses `getUserMedia` into HTML `<video>`. Windows, Linux, and HarmonyOS return `Unavailable`. Video recording remains out of scope for this release.

- **Add `leptoskit_plugin_live_photo` with create/start/stop/status/release and a `LivePhoto` surface bound by session id (`PHLivePhotoView`).** Android and browser remain Unavailable; editing remains later scope. _Owner:_ Ifiok Jr. · _Introduced in:_ [13da9d6](https://github.com/leptoskit/leptoskit/commit/13da9d61f3252c056816f1da31516fd4e5040f9c)
- **Add `leptoskit_plugin_maps` with create/camera/markers/status/release and a `Map` surface bound by session id (MapKit / Google Maps).** User location and shapes remain later scope. _Owner:_ Ifiok Jr. · _Introduced in:_ [10ddb6f](https://github.com/leptoskit/leptoskit/commit/10ddb6fef86dc107d73054a9e2f2542f6e922da8)

#### Screen state and queries survive a rebuild

_Owner:_ Ifiok Jr. · _Introduced in:_ [c7a85ef](https://github.com/leptoskit/leptoskit/commit/c7a85ef05c4a1a510f6136b444ad4a0c2ef1af5e)

leptoskit builds a screen again whenever a signal it read changes. Two things broke under that: a `RwSignal::new(..)` inside a screen ran again with every rebuild, so a chosen filter or a typed field reset as soon as the screen redrew, and a `use_query` in a screen refetched in a loop, because the rebuild its data caused mounted the query again while the data already counted as stale.

- `leptoskit_native`: `remember(|| initial)` returns the same `RwSignal` on every build of the screen, told apart by its place in the source and by order. `remember_keyed(&key, || initial)` names the value by a key instead, for rows that move. A value is released once a whole build no longer reaches it, such as after navigating to another route. `guest::mount` keeps the values for native apps. In the browser, `web::screen(move || ..)` replaces `move || web::render(..)` and does the same. `ScreenState` is the store underneath, for hosts of their own. Both functions are in `leptoskit::prelude`, and the module is `leptoskit::remember`.
- `leptoskit_query`: a query mounted within two seconds of this client's last fetch of it takes that fetch's data or error instead of fetching again, like SWR's deduping interval. The window covers `RefetchOn::Always` and `retry_on_mount`, and not invalidation, `refetch()`, focus, reconnect, or intervals. Data from `set_query_data`, initial data, or hydration doesn't count as a fetch. Set it with `with_mount_dedupe` on `QueryDefaults`, `QueryOptions`, or `QueryObserverOptions`; `Duration::ZERO` gives TanStack's behaviour. `QueryOptions`, `QueryDefaults`, and `ResolvedQueryOptions` gain a `mount_dedupe` field, so code that builds them with struct literals needs it too.

An app with its own keyed store and a long stale time to work around this can switch to `remember` and drop both.

#### Scroll an element into view with `Reveal`

_Owner:_ Ifiok Jr. · _Introduced in:_ [6aef48a](https://github.com/leptoskit/leptoskit/commit/6aef48a580ac38f8dc06b281c49c9036faf04151)

A screen can now bring one element into view without knowing where layout put it. Create a `Reveal`, give it to the element with `reveal_on(&reveal, ScrollAlign::Nearest)`, and call `reveal.request()` from any handler. Each request scrolls once, after the next update, and the person scrolls freely afterwards. `ScrollAlign` is `Start`, `Center`, `End` or `Nearest`, and `Nearest` does nothing when the element is already in view.

On the web this is the browser's `scrollIntoView`, smooth unless the person asked for reduced motion. In native apps the surface works out the offset from layout and scrolls the nearest `ScrollView` around the element. Only that one view scrolls, and an element outside any `ScrollView` is left alone.

The GTK, Windows, AppKit and UIKit backend crates now apply `scroll_offset`, which they ignored before, so `Reveal` and `VirtualList`'s programmatic scrolling work in the generated Windows and Linux apps. GTK, Windows and AppKit jump to the offset; UIKit animates when asked. The terminal and HarmonyOS still don't scroll from code.

`NativeProps` gains a `reveal` field, which the surface answers itself and never sends to a backend.

#### TextArea: a field for several lines of text

_Owner:_ Ifiok Jr. · _Introduced in:_ [ce9d470](https://github.com/leptoskit/leptoskit/commit/ce9d470863cd327cdd87a36a12738c0fd356986c)

`TextArea()` is a new component, with its own `NativeKind::TextArea`, for notes, comments and messages. Enter adds a line instead of submitting, long lines wrap, and the field scrolls its own content. It is controlled like `TextInput`: `on_change` receives the proposed text with lines separated by `\n`, and `placeholder`, `read_only`, `max_length`, `capitalize`, `autocorrect` and `autocomplete` apply. It never raises `Submit`.

The web renders a `<textarea>`; macOS an `NSTextView` in an `NSScrollView`; iOS a `UITextView`; Android a multi-line `EditText`; Linux a `gtk::TextView` in a `gtk::ScrolledWindow`; Windows a multi-line `EDIT` control; the terminal a wrapping editor with line-wise arrow keys. The Swift and Kotlin renderers that generated apps use draw it too. GTK and Windows show no placeholder, and HarmonyOS and the watch surfaces do not draw the component.

`aria_multiline(true)` on a `TextInput` or `PasswordInput` now fails validation, because a single-line field that calls itself multi-line misreports itself to assistive technology. Use `TextArea` instead.

- **Typed text-input hints.** `TextInput` now carries the keyboard and text-editing hints that hand-written JavaScript used to provide, so every interface stays in leptoskit's own Wasm tree: `keyboard` (Text, Email, Number, Decimal, Phone, Url, Search), `read_only`, `max_length`, `capitalize` (None, Words, Sentences, Characters), `autocorrect`, and `submit_label` (Done, Go, Next, Search, Send). Each platform maps them to its own controls: the web to `type`, `inputmode`, `readonly`, `maxlength`, `autocapitalize`, `autocorrect`, `spellcheck`, and `enterkeyhint`; iOS to `keyboardType`, `autocapitalizationType`, `autocorrectionType`, `spellCheckingType`, and `returnKeyType`; Android to `inputType`, `imeOptions`, `InputFilter.LengthFilter`, and a cleared key listener for read-only fields; GTK to `set_input_purpose`, `set_editable`, `set_max_length`, and `InputHints`; macOS to `isEditable` and delegate truncation plus the field editor's correction and spell checking; Windows to `EM_SETREADONLY` and `EM_LIMITTEXT`. `read_only` implies `aria-readonly`. The Swift and Kotlin bridge renderers and the terminal backend honour the same props. _Owner:_ Ifiok Jr. · _Introduced in:_ [6d42b06](https://github.com/leptoskit/leptoskit/commit/6d42b068273560b572fa4c7f5f4475b33a03563b)
- **Ding-inspired toast pills.** Status toasts arrive as a 44px shape, spring open into a pill, morph in place when a newer message arrives, and shrink before they leave — the same phase model and spring tunings as [Ding](https://github.com/YusufCeng1z/ding). Hold a `Toaster`, call `success` / `error` / `warning` / `info` / `loading`, rebuild from `toaster.state()`, and advance phases with `poll`. `ToastViewport` draws the stack from shared builders, so every backend that already draws `View`, `Text`, `Button`, and `Vector` can show a toast. _Owner:_ Ifiok Jr. · _Introduced in:_ [ab77e98](https://github.com/leptoskit/leptoskit/commit/ab77e9807ae2c781c3d7a80ff5e00c270cfac4b6)

#### Video players and Video surface

_Owner:_ Ifiok Jr. · _Introduced in:_ [130c61f](https://github.com/leptoskit/leptoskit/commit/130c61f9f2fa514b14122055e7c1021735ce2a3b)

`leptoskit_plugin_video` is a new runtime plugin, `video`, for host-owned URL video players. Create or replace a source, play, pause, seek, set muted/loop/rate, snapshot status, and release the player. Show the same player with a new `Video(player_id)` native widget kind that binds a host-owned id the way `Scene3d` binds a scene (requires an accessible label or `aria_hidden`).

iOS and macOS use `AVPlayer` with an `AVPlayerLayer` surface; Android uses Media3 ExoPlayer with a `PlayerView`; the browser uses an HTML `<video>` element. Windows, Linux, and HarmonyOS return `Unavailable`. Tracks, caching, AirPlay, fullscreen controls, and iOS picture-in-picture enter against an attached player remain out of scope for this release.

### Fixes

- **Keep Android builds warning-free: omit the unused stderr fallback on Logcat, simplify pointer decisions, and account for Clippy's Android TLS false positive while retaining const initialization.** _Owner:_ Ifiok Jr. · _Introduced in:_ [7237130](https://github.com/leptoskit/leptoskit/commit/723713098b125431f6e762cbf4b6f3ebe5506a23)
- **Hydrated web screens refresh their stylesheet when classes change.** Switching a theme or opening a panel could introduce new style scopes while leaving the initial CSS in place, so controls lost their layout and colours. The browser regression hydrates server HTML and changes padding in both directions. _Owner:_ Ifiok Jr. · _Introduced in:_ [9189d6a](https://github.com/leptoskit/leptoskit/commit/9189d6a89173a9ff009bf0a7d30b0639fd356fa9)

#### Regions committed together keep their own widgets

_Owner:_ Ifiok Jr. · _Introduced in:_ [7e61e9b](https://github.com/leptoskit/leptoskit/commit/7e61e9bceefea4db7c8e3a70fdd31deb21597898)

`NativeSurface::try_patch` applies patches in tree order, and it answered in that order too, though its contract (and the wasm host, which pairs each answer with the patch it sent) is the order given. A commit that changed two regions whose wire ids ran against the tree, such as a label above a list that gained a row, paired one region's wire ids with the other's widgets. A debug host stopped with "surface and wire tree diverged"; a release host could send a widget's events to another node. On a Solana Seeker, buying anything in Reapers stopped the app.

`try_patch` now answers in the order given. The wasm host's incremental tests check, after every commit, that each widget pairs with the wire node it was built from.

#### Native props read the same whatever serde_json's features

_Owner:_ Ifiok Jr. · _Introduced in:_ [736759c](https://github.com/leptoskit/leptoskit/commit/736759cc6082c21aa05025aab6823b785a0c3a21)

An Android app that also used Solana's clients drew nothing: its first vector failed in the Wasm host with `invalid type: map, expected f32`. Those clients turn on serde_json's `arbitrary_precision` for the whole build, and under it a buffered number with a fraction is a map. The host read a node's props through `#[serde(flatten)]`, which buffers every value, and so did each animation `Keyframe`.

The host now reads a node with `WireNode::from_json`: the props and the keys beside them in two passes, with nothing buffered, and `Keyframe` lists its look's `opacity` and `transform` itself. Its JSON is unchanged.

The guest and the host now share `leptoskit_native::wire`: one table of kind codes and one `WireNode`. That fixes two more ways the sides disagreed:

- The host's own kind table stopped at `Camera`, so a `Map` or a `LivePhoto` showed as a plain view.
- An image's `source` was taken by the DevTools location, which used the same key, so native images had no source. A `DevTools` guest now sends its location as `source_location`.

`test:wire` runs the guest-to-host tests under `arbitrary_precision` too.

- **App shortcuts wait while a modal is open.** On the web, an app shortcut such as Escape or W no longer runs while a `Modal` is open, and it no longer takes the key from the modal. Escape now closes the modal, as it should. Before, an app with its own Escape shortcut kept the key, so the modal stayed open. Shortcuts work again as soon as the modal closes. _Owner:_ Ifiok Jr. · _Introduced in:_ [535aa35](https://github.com/leptoskit/leptoskit/commit/535aa353b341e4e4479d5c9ce19f2b407dbf7b55)

#### Keep focus across web rebuilds and inside modals

_Owner:_ Ifiok Jr. · _Introduced in:_ [a1dd169](https://github.com/leptoskit/leptoskit/commit/a1dd169266d9fc0c2bc17705cdcc0afe846fb584)

On the web, a screen that rebuilt replaced every landmark (`main`, `nav`, `header`, `footer`, `aside`, `article`, `section`, `search`), every heading, list, paragraph and label with a new element, instead of updating the one already there. Anything inside went with it: a text field lost its focus and its selection, and a person typing was dropped back to the top of the page.

Those elements now update in place, as `div` and `span` always did, so focus stays where it was after any rebuild.

A rebuild also no longer takes a node's `id`, `role`, `aria-*` and `tabindex` attributes off and puts them back: it sets the ones whose value changed and removes the ones that are gone. Before, a focused element that is only focusable through `tabindex`, such as a dialog's heading, lost focus to the page on every rebuild.

A `Modal` on the web now wraps Tab: past its last control focus comes back to its first, and Shift+Tab past its first goes to its last, instead of leaving the page for the browser's toolbar.

- **The web launch screen lifts once the app is drawn.** The HTML splash that `[app.launch_screen]` injects into `index.html` used to stay over the app for good: nothing called its `leptoskitDismissSplash()`. The web renderer now fades it out once the app's first render is painted, as iOS and Android end theirs at the first frame. An app that calls `leptoskit::splash::hold()` keeps it until `leptoskit::splash::dismiss()`, which now lifts it on the web too. _Owner:_ Ifiok Jr. · _Introduced in:_ [44cfd11](https://github.com/leptoskit/leptoskit/commit/44cfd118a6ab5aa6e98eb641048fa307c91e47d2)

#### Smaller server-rendered pages

_Owner:_ Ifiok Jr. · _Introduced in:_ [f8440e7](https://github.com/leptoskit/leptoskit/commit/f8440e78ff6fad3cf23ac1257bf146d31778eddd)

Server-rendered and static pages no longer repeat leptoskit's style reset for every differently styled element. Each element's typed styles start from about sixty declarations, such as `display:flex`, `flex-shrink:0`, and zeroed margins and padding, and every distinct look carried its own copy of all of them, about a kilobyte each. `leptoskit::web::render` now writes those defaults once per stylesheet, and each style scope's sheet holds only what differs from them, often a single declaration. Elements look the same: the defaults keep the specificity and order they had, below utility classes. A list of two dozen rows that differ only in width used to carry 27 copies of the reset and now carries one, and its markup is under 9 KB. Style scopes are also 16 hex digits instead of 64, since every element carries its scope.

The root element of each render now carries `data-lk-root`, which keeps one render's defaults from resetting the elements of a render earlier in the page. `leptoskit_testing_library` leaves it out of the markup it prints, as it does `data-lk-style`.

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

### Breaking changes

#### leptoskit_native: components, Tailwind, and the renderer

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_native` is the native side of the shared tree: builders for every component, from `View`, `Text`, and `Button` to `TextInput`, `PasswordInput`, `Slider`, and `ScrollView`, the Tailwind compiler, the retained element tree, and the diff that turns a commit into a mutation batch.

Every native backend implements `NativeBackend` and consumes the mutations this crate produces; the web build uses its optional `web` feature to render the same builders through leptos. App code normally reaches it through `leptoskit::prelude`, not as a direct dependency, so one screen works on every target.

### Features

- **Theme tokens in leptoskit.toml and deeper CSS var().** `[theme.colors]` and `[theme.lengths]` in `leptoskit.toml` name design tokens that `Theme::install_maps` loads into the process-wide Theme map. Arbitrary color and length utilities accept `var(--name)`, one nested `var()` fallback, and `/opacity` on color vars. The web adapter prepends `Theme::css()` so the browser sees the same `:root` custom properties native resolves through Theme. Tailwind `@theme` / `tailwind.config` files remain unsupported. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #538](https://github.com/leptoskit/leptoskit/pull/538)
- **CSS color variables and a Theme map.** Arbitrary color utilities can use `var(--name)` or `var(--name, fallback)`. The web stylesheet keeps the CSS function. Native paint resolves the name through `Theme::set_color` (or the fallback when the theme has no entry). `Theme::css` emits a `:root` sheet of the same names for the browser. Nested `var()`, non-color variables, and opacity modifiers on `var()` stay unsupported. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #536](https://github.com/leptoskit/leptoskit/pull/536)

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
- **Paint italic text and per-edge borders on Linux and Android.** `border-t`, `border-l-2`, and the other edge border utilities now get the same default stroke color as a uniform `border`, and GTK and Android paint each edge instead of leaving those strokes undrawn. The `italic` utility also draws through GTK labels and Android text views, matching the web and the terminal. AppKit, UIKit, and Win32 still leave per-edge borders undrawn and italic upright. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #534](https://github.com/leptoskit/leptoskit/pull/534)
- **Yoga LayoutUnit rounding for Chrome grid parity.** Fractional grid tracks and percentage lengths resolve through Blink-style 1/64 CSS-pixel layout units, so Yoga frames match Chrome `getBoundingClientRect` for the shared Tailwind class trees. The grid parity probe covers nested grids, dense auto-flow, minmax, flex edges, absolute insets, percentages, and overflow. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #540](https://github.com/leptoskit/leptoskit/pull/540)
