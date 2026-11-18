# Build shared screens with Tailwind

Build your screen from native primitives such as `View`, `Text`, `Image`, `ScrollView`, and `Pressable`. These Rust functions return the same `Element` type on native and web. Tailwind expresses layouts and visual composition. The [component catalog](../reference/components.md) lists the 15 primitives and 31 optional helpers.

```rust
use leptoskit::prelude::*;

#[component]
pub fn Counter(count: usize, on_increment: impl Fn() + Send + Sync + 'static) -> Element {
	view! {
		<View class="w-full gap-4 p-6 bg-slate-100 dark:bg-slate-950">
			<Text class="text-3xl font-bold text-indigo-500">{format!("Count: {count}")}</Text>
			<Button class="rounded-lg bg-indigo-500 px-4 py-2 text-white" on:press=on_increment>
				"Increment"
			</Button>
		</View>
	}
}
```

Use `view!` markup for shared screens and `#[component]` with PascalCase names for reusable components. The macros build the same `Element` tree on web and native platforms. Builder calls remain useful for fallible class validation, helpers without tag support, and direct tree manipulation.

## Write the screen as markup

`leptoskit::prelude` also exports a `view!` macro and a `#[component]` attribute. `view!` parses [rstml](https://github.com/rs-tml/rstml) markup, the syntax leptos uses, and expands it into the same builder calls. The markup renders on the web and on every native backend:

```rust
use leptoskit::prelude::*;

#[component]
pub fn Counter(count: usize, on_increment: impl Fn() + Send + Sync + 'static) -> impl IntoView {
	view! {
		<View class="w-full gap-4 p-6 bg-slate-100 dark:bg-slate-950">
			<Text class="text-3xl font-bold text-indigo-500">"Count: " {count}</Text>
			<Button class="rounded-lg bg-indigo-500 px-4 py-2 text-white" on:press=on_increment>
				"Increment"
			</Button>
		</View>
	}
}

let screen = view! { <Counter count=0 on_increment=|| println!("increment") /> };
```

Each tag calls a component, each attribute calls the builder method of the same name, and `on:press` calls `on_press`. `#[component]` keeps the function's signature, so `Counter(0, handler)` still works in builder code. The [`view!` and `#[component]` reference](../reference/view.md) covers props, children, fragments, and errors.

Flutter-inspired compositions remain optional. Import them from `leptoskit::components::helpers` when useful; the default prelude keeps their functions out of your primary component vocabulary. A row can be `View().class("flex-row")`, and layered content can use a relative `View` with absolute children.

A screen whose root is a `View` or `ScrollView` fills the window's width, as its `<div>` does in a browser, so `items-center` centres content on the screen on every platform. A button centres its label unless you align it with `text-left` or `text-right`.

Tailwind utilities are built into leptoskit. You do not install Tailwind, scan source files, or run a Node build. The compiler parses classes into typed declarations. Native rendering applies those declarations to Yoga and widget props. The optional web renderer emits CSS from the same declarations.

## Commit the screen on native

Create a surface and set its viewport before the first commit:

```rust
use leptoskit::native::HeadlessBackend;
use leptoskit::prelude::*;

let mut surface = NativeSurface::new(HeadlessBackend::new());
surface.set_viewport(390.0, 844.0);
surface
	.set_root(&leptoskit::view! { <Counter count=0 on_increment=|| println!("increment") /> });
```

`HeadlessBackend` lays the screen out without drawing it, which is useful in tests. A real host uses a platform backend instead, such as `AppKitBackend` on macOS or `UIKitBackend` on iOS. The apps `leptoskit new` generates do all of this for you; you only need a surface when you write your own host. Rebuild and commit the screen when your application state changes: native event callbacks update your state, and the host decides when to call `set_root` again.

## Render the same screen with Leptos

Enable the optional `web` feature:

```toml
[dependencies]
leptoskit = { version = "0.1", features = ["web"] }
leptos = "0.9.0-rc0"
```

Select Leptos's `csr`, `hydrate`, or `ssr` feature for your browser or server target. Wrap `web::render` in a reactive closure when the screen reads signals:

```rust
use leptos::prelude::Get;
use leptos::prelude::IntoView;
use leptos::prelude::RwSignal;
use leptos::prelude::Update;

#[leptos::component]
pub fn BrowserRoot() -> impl IntoView {
	let count = RwSignal::new(0usize);
	move || {
		leptoskit::web::render(leptoskit::view! {
			<Counter count=count.get() on_increment=move || count.update(|count| *count += 1) />
		})
	}
}
```

`Counter` is the same component used by the native surface. The web renderer creates semantic DOM controls and attaches the existing event handlers. Leptos owns DOM updates, server rendering, and hydration.

Signals and the hooks in `leptoskit::hooks` (the `use` feature) also work in native apps on iOS, Android, macOS, Windows, and Linux. The view you pass to `leptoskit::guest::mount` runs again when it reads a signal that later changes. A value you bind, and a list or a condition you put in `For` or `Show`, updates on its own: writing the signal is enough, including from an event handler, and you do not call `leptoskit::guest::render()` afterwards. `leptoskit::prelude` carries the signal type, `For`, `Show`, and the reading and writing traits: `Get`, `Set`, and their untracked and reference-taking twins. A hook you scope yourself also needs an owner: add `reactive_graph = "0.3.0-rc0"` for `reactive_graph::owner::Owner`:

```rust
use leptoskit::hooks::use_toggle;
use leptoskit::prelude::*;
use reactive_graph::owner::Owner;

let owner = Owner::new();
let switch = owner.with(|| use_toggle(false));
leptoskit::guest::mount(move || {
	// The view keeps the owner, and with it the hooks, alive.
	let _scope = &owner;
	let toggle = switch.toggle.clone();
	let label = if switch.value.get() { "On" } else { "Off" };
	view! { <Button on:press=move || toggle()>{label}</Button> }
});
```

## Update a value without rebuilding the screen

Re-rendering costs every element on the screen. A list of 1,000 cards is about 3,000 elements, so changing 100 labels that way rebuilds 3,000 to change 100. Write the label against the signal instead, and the renderer updates that node:

```rust
use leptoskit::prelude::*;

let count = RwSignal::new(0);

view! {
	<View class="flex-col gap-2 p-4">
		<Text>{move || format!("Count: {}", count.get())}</Text>
		<Text>"A row that does not change"</Text>
	</View>
};
```

`Text` takes a string or a closure. A closure is a binding: `Text` reads it while it builds the element, so the first frame is right, and the native host then reads it again after every turn — a press, the poll that runs your tasks, a plugin reply. Writing `count.set(…)` is the whole change; nothing else to call, and the same code works on every platform. `background`, and the other value props that take a closure, do the same.

Only the nodes that read application state are visited, and only the ones whose value differs are sent, so a screen that changes a hundred rows out of a thousand pays for a hundred. A press that writes one of those signals does not rebuild the screen around them.

A list or a condition is the same idea for structure. `For` rebuilds its own rows when the collection changes, and `Show` swaps its child when the condition changes. The header above either one stays where it was:

```rust
use leptoskit::prelude::*;

let items = RwSignal::new(vec!["Ada".to_owned(), "Grace".to_owned()]);
let show_count = RwSignal::new(true);

view! {
	<View>
		{Show(move || show_count.get(), || view! { <Text>"People"</Text> })}
		{For(
			move || items.get(),
			|name| name.clone(),
			|name| view! { <Text>{name}</Text> },
		)}
	</View>
}
```

`items.set(…)` replaces the rows. `show_count.set(false)` removes the heading. A press handler can do either, and the view function does not run again to make it so.

The view function runs again when it reads a signal. `let n = count.get();` in the body rebuilds the screen, which is how a value chooses the shape of the tree, and it is the slow way to change a label. A read inside `Text`, `For`, or `Show` stays with that piece. A screen that stores its state in a plain value still rebuilds after a press, because that is how the new value reaches the screen.

Keep a signal in application state, or in [`remember`](#keep-a-screens-own-state-across-rebuilds), rather than creating one inside a view function: a signal lives as long as the guest, so one created on every render is never freed.

On the web the same tree shows the value read at build time, and leptos re-runs the screen when the signal changes, so nothing else is needed there. `leptoskit::prelude` exports leptos's own `RwSignal`, so a screen shared with the browser keeps one signal type.

## Keep a screen's own state across rebuilds

A chosen filter or a half-typed search belongs to one screen, not to the whole app, but `RwSignal::new(false)` inside the screen runs again on every rebuild, so the value resets the moment the screen redraws. `remember` hands back the same signal on every build instead:

```rust
use leptoskit::prelude::*;

#[component]
pub fn Tasks() -> Element {
	// Created on the first build; every rebuild after it gets the same signal.
	let open_only = remember(|| false);
	let toggle = move || open_only.update(|open| *open = !*open);
	let label = if open_only.get() { "Open tasks" } else { "All tasks" };
	view! {
		<View class="flex-col gap-2 p-4">
			<Text class="text-xl font-bold">{label}</Text>
			<Button on:press=move || toggle()>"Switch"</Button>
		</View>
	}
}
```

This works the same on the web and on every native platform. Each `remember` call is named by where it is in your code and by how many times that line already ran in the build, so a component used twice keeps two values. Rows that can move, appear, or disappear name their value with `remember_keyed(&task.id, || …)` instead, so the value follows the row rather than its position.

A value lives as long as the screen keeps asking for it. When a whole rebuild finishes without reaching a `remember` call — you navigated to another route, or a branch stopped rendering — its value is released, and coming back starts from the closure again. State that must outlive a screen, such as a signed-in user, still goes in a signal you create before mounting the app.

`leptoskit::guest::mount` keeps the values for a native app. In the browser, render the screen with `leptoskit::web::screen(move || …)` where you would write `move || leptoskit::web::render(…)`: it rebuilds the same way and keeps the values. Outside both, as in a test that builds a screen once, `remember` is `RwSignal::new`.

## Group a state struct into a store

A screen's state outgrows one value: a form has its fields, a session has its messages. Keeping the whole struct in one `RwSignal` works, but the signal cannot tell the fields apart, so every screen that read any of them rebuilds when any field changes. Wrapping each field in its own signal by hand fixes that, at the cost of the struct no longer being a plain value.

A store keeps the struct a plain Rust value and gives each field its own signal. Derive `Store` on the struct, hold it in an `ArcStore`, and read and write fields through the getters the derive adds:

```rust
use leptoskit::prelude::*;
use leptoskit::store::ArcStore;

#[derive(Default, reactive_stores::Store)]
struct Library {
	title: String,
	open: bool,
}

let library = ArcStore::new(Library::default());
let open = library.clone().open();

view! {
	<View class="gap-2">
		<Text>{move || library.clone().title().get()}</Text>
		<Button on:press=move || open.update(|open| *open = !*open)>"Toggle"</Button>
	</View>
};
```

The generated getters take the store by value, so an `ArcStore` is cloned for each field — a reference bump — or kept as the field handle `open` is above. Writing a field notifies the readers of that field, and of the fields that contain it, but never of its siblings: a screen that shows the title does not rebuild because the library opened. The same code runs on every platform — web, iOS, Android, macOS, Windows, Linux, the terminal, and HarmonyOS (experimental) — because a field is a signal like any other: on native a binding over it rewrites its own node, and on the web the render closure runs again when the field it read changes.

Deriving `Patch` on the same structs adds `.patch(next)`: apply a whole new value — a reducer's output, or fresh data from the server — and only the fields that changed notify. Every field type must take part in patching, which the primitives, `String`, `Option`, `Vec`, and tuples do, and any other struct or enum that derives `Patch` does; when field types come from a crate that should stay free of the reactive crates, write the fields directly instead. The `opencode` example runs its whole state this way, with `Patch` for its actions, and `commonplace` writes one field at a time.

The derives come from the `reactive_stores` crate, so add it next to leptoskit (`reactive_stores = "0.5.0-rc0"`), the same way the owner above adds `reactive_graph`; `leptoskit::store` re-exports the types. `ArcStore` lives as long as a clone of it does, like `ArcRwSignal`, which suits state a whole app shares. `Store::new` instead follows the reactive owner that is current where it was created, is disposed with it, and is `Copy`, so its getters need no clone. Two neighbours still apply: handles and values that must never notify, such as a subscription key, stay in `StoredValue`, and state whose names are decided at run time — a document's variables, say — stays one signal per value in a map.

## Animate from app code

Some screens change on every frame: a counter that ticks, a progress value, or a list your code scrolls. A timer drifts against the display, so updates land between frames and the motion stutters. Wait for the display instead. `leptoskit::guest::next_frame` resolves once per display frame, like `requestAnimationFrame` in a browser, and a signal you write before awaiting it again reaches the screen on the next frame. The counter's label is a binding, so the write is the whole update:

```rust
use leptoskit::guest::next_frame;

use crate::Counter;

/// Adds one to the counter on each of the next `frames` display frames.
async fn count_frames(counter: Counter, frames: u32) -> Result<(), leptoskit::guest::Error> {
	for _ in 0..frames {
		next_frame().await?;
		counter.increment();
	}

	Ok(())
}
```

Run it on your app's executor, as you would any other async plugin call. The `Frame` it returns carries `timestamp_ns` and `period_ns`, the frame's time and the display's refresh period, for motion that should follow elapsed time rather than the number of frames. On iOS and macOS, frames come from `CADisplayLink`; on Android, from the `Choreographer`. Your render happens inside that frame's callback, so a render that fits in one frame never waits an extra one. The display link runs only while your code waits for a frame.

On Windows, Linux, and HarmonyOS, `next_frame` returns `leptoskit::guest::Error::Unavailable`, and so do apps built before leptoskit had a frame clock, which a code push update can still reach. Fall back to a timer there rather than treating it as a failure. `leptoskit::guest::sleep(Duration)` waits on the host's real timer across generated macOS, iOS, Android, Windows, Linux, and terminal apps; experimental HarmonyOS and hosts older than the timer plugin answer `Unavailable`, so drive those from input or a host event instead of a frame wait.

## Combine typed styles and classes

Use typed styles for values that come from application data. Classes override the typed base for properties they set, regardless of builder call order:

```rust
use leptoskit::prelude::*;

let card = view! {
	<View
		class="w-20 p-4"
		with_style=|style| style.width = Dimension::Points(200.0)
		class="rounded-lg bg-white"
	/>
};
```

The rendered width is 80 logical pixels. Each `.class(...)` call appends utilities. Duplicate tokens are removed. Class string order does not determine precedence. Broad spacing utilities precede axes and individual edges, responsive rules override base rules, and important utilities win. Avoid competing utilities for the same property and variant. Equal-rank conflicts use deterministic lexical order, which is not an upstream Tailwind compatibility guarantee.

Use a trailing `!` for an important utility, such as `p-4!`. The older leading form, `!p-4`, is also accepted.

Keep the source tree when you need responsive behavior. `NativeSurface` retains the source internally and resolves it again after viewport or appearance changes. `Element::resolved` returns a snapshot with its classes removed and is intended for renderer integration.

## Update responsive styles and dark mode

Use the default `sm`, `md`, `lg`, `xl`, and `2xl` variants. `max-*` variants select widths below a breakpoint. Variants can be combined into a range:

```rust
let panel = view! { <View class="p-4 md:max-xl:p-8 xl:flex-row dark:bg-slate-950" /> };

surface.set_root(&panel);
surface.set_viewport(900.0, 600.0);
surface.set_dark_mode(true);
```

On native, the host supplies the viewport in logical pixels and calls `set_dark_mode` when appearance changes. The browser uses generated width media queries and `prefers-color-scheme`. Browser resizing does not require rebuilding the screen.

## Let people choose light or dark

Many apps offer a setting with three choices: Light, Dark, and Match device. `dark:` utilities follow the device until your app calls `appearance::set`; from then on they follow the choice, without a reload or a restart:

```rust
use leptoskit::appearance::{self, Appearance};
use leptoskit::prelude::*;

#[component]
pub fn AppearancePicker() -> Element {
	view! {
		<View class="flex-row gap-2">
			<Button on:press=move || appearance::set(Appearance::Light)>"Light"</Button>
			<Button on:press=move || appearance::set(Appearance::Dark)>"Dark"</Button>
			<Button on:press=move || appearance::set(Appearance::System)>"Match device"</Button>
		</View>
	}
}
```

`appearance::get()` returns the current choice and is reactive, so a picker can mark the selected option with a binding or `Show`.

- **Web:** the choice marks the page's `<html>` element with `data-lk-appearance="light"` or `"dark"`, which the generated stylesheet reads, and sets its `color-scheme`, so the browser's own form controls and scrollbars match. Match device removes both.
- **macOS, iOS, Android, Linux, Windows, and the terminal:** the next render carries the choice to the host, which resolves `dark:` against it and tells the platform backend, so controls the backend crates draw match too. In the generated macOS, iOS, and Android apps, `dark:` utilities follow the choice, but a label with no colour class and the system's own control chrome still follow the device, so give text an explicit colour, as in `text-slate-900 dark:text-slate-100`.

Remembering the choice is up to your app. Store `Appearance::as_str()` (`"system"`, `"light"`, or `"dark"`) with the [`store` plugin](../platforms/plugin-inventory.md), a cookie, or your own settings, and call `appearance::set` with `stored.parse()` before the first screen renders. A server render can't know the choice, so on the web put the attribute on `<html>` yourself from the stored value to avoid showing the device's appearance first.

The [CSS coverage reference](../reference/css-coverage.md#defaults-and-units) lists breakpoint values, units, and component defaults.

## Style the pressed state

A button that looks the same while you press it feels broken. Add `active:` utilities to a `Button` or `Pressable` to restyle it while it is held down:

```rust
use leptoskit::prelude::*;

#[component]
pub fn AddButton(increment: impl Fn() + Send + Sync + 'static) -> Element {
	view! {
		<Button
			class="rounded-lg bg-indigo-500 px-6 py-3 text-white active:bg-indigo-700"
			on:press=increment
		>
			"Add one"
		</Button>
	}
}
```

This works on every platform except HarmonyOS. On the web, `active:` compiles to CSS `:active`, and the rendered root listens for touches so that iOS Safari applies it too. On macOS, iOS, Android, Linux, and Windows, the native control reports when the platform starts showing it as pressed, and leptoskit applies the `active:` utilities to that control until it is released. Nothing else re-renders, and the control keeps its widget, accessible name, and role. `active:` rules override responsive and dark-mode rules while they apply, and combine with them, as in `dark:active:bg-indigo-300`.

Each platform decides when a press starts and ends, the same way it does for its own highlight. On the web and macOS, a press lasts until the pointer is released, even after the pointer leaves the control. On iOS and Windows, it ends when the finger or pointer slides away and returns if it comes back. On Android, it ends when the finger slides away. Linux follows GTK's pressed state for the button. Keys show the pressed style where the platform draws one for them: while Space is held on Android and Windows and in most browsers, and briefly on Linux. On macOS and iOS, a key press or VoiceOver activates the control without showing its pressed style.

Controls without `active:` utilities still respond. Every `Pressable`, and every `Button` with its own background, border, or corner radius, dims to 70 percent of its opacity while held. A `Button` without any of those keeps the platform's own pressed highlight instead. Browser buttons have no native chrome, so they always dim. Any `active:` utility replaces the dim; to keep a control unchanged while pressed, repeat a resting value, as in `bg-indigo-500 active:bg-indigo-500`.

`active:` is only valid on `Button` and `Pressable`, the controls whose presses every platform reports. On any other element, `.class` panics and `.try_class` returns an error. Inside a `Pressable`, put the pressed styles on the `Pressable` itself. Text color is inherited, so `active:text-white` on the `Pressable` also recolors any child text that sets no color of its own. To check pressed styles in a test, `WidgetTester::hold` holds a control down and `release` lets it go; see [Testing](08-testing.md).

## Clear the status bar and home indicator

Phones draw system UI over the edges of the screen. Padding utilities with a `safe` value pad by the inset the host reports for each edge, and `safe-offset-*` adds a spacing value to that inset. The names follow the `tailwindcss-safe-area` plugin:

```rust
let screen = view! {
	<ScrollView class="w-full h-full bg-white">
		<View class="min-h-full px-6 pt-safe-offset-4 pb-safe-offset-4 gap-6" />
	</ScrollView>
};
```

`p`, `px`, `py`, `pt`, `pr`, `pb`, and `pl` accept both forms. `UIKitHost` reads the controller view's `safeAreaInsets`. Other native hosts call `NativeSurface::set_safe_area` with the edges covered by system UI, in logical pixels; on Android, those are the window insets for system bars and display cutouts. Desktop windows report no insets, so the padding is just the offset. In a browser, the utilities emit `env(safe-area-inset-*)`, which needs `viewport-fit=cover` in the page's viewport meta tag.

## Validate dynamic classes

Use `.class(...)` for class strings you control. Invalid or unsupported tokens cause a panic with the class and its reason. Use `.try_class(...)` for dynamic input:

```rust
use leptoskit::prelude::*;

pub fn panel(classes: &str) -> Result<Element, ClassError> {
	View().try_class(classes)
}

let error = panel("md:hover:bg-red-500").expect_err("hover styles are unsupported");
assert_eq!(error.class, "md:hover:bg-red-500");
```

The compiler validates inactive variants too. An unsupported `2xl:shadow-lg` returns an error even in a narrow viewport. It rejects unknown utilities, invalid values, unsupported variants, and empty breakpoint ranges.

## Use controls and scrolling

Set text input values and accessible names with the shared builders; see the [accessibility guide](15-accessibility.md) for the full ARIA vocabulary:

```rust
let form = view! {
	<View class="gap-3 p-4">
		<TextInput
			value="Ada"
			placeholder="Your name"
			aria_label="Name"
			class="border border-slate-300 rounded-md p-2"
		/>
		<Button enabled=true on:press=|| println!("save")>"Save"</Button>
	</View>
};
```

Register text changes with `.on_change(handler)`. The handler receives the proposed `&str`. `PasswordInput()` has the same single-line API with masked display.

### Help password managers fill a form

A password manager can only fill or save a password when it knows which field holds the account name and which holds the password. Say so with `autocomplete`, using the same words as HTML: `username` or `email` for the account, `current-password` to sign in, and `new-password` when the person picks a password, so the manager can suggest a strong one and save it.

```rust
view! {
	<View class="gap-3">
		<TextInput value=email autocomplete="username" aria_label="Email" on:change=set_email />
		<PasswordInput
			value=password
			autocomplete="current-password"
			aria_label="Password"
			on:change=set_password
		/>
	</View>
}
```

The web gets an `<input type="password" autocomplete="current-password">`. macOS and iOS set the field's content type, Android its autofill hint, and HarmonyOS its input type, which each system's password manager reads. Windows and Linux mask the field, but have no hint to pass on. Everywhere, a `PasswordInput` turns off autocorrection and suggestions.

### Typed text-input hints

A text field can ask its platform for the keyboard, capitalization, correction, and return key that fit what it holds, instead of hand-written JavaScript:

```rust
view! {
    <View class="gap-3">
        <TextInput
            keyboard=Keyboard::Email
            capitalize=Capitalize::None
            autocorrect=false
            max_length=64
            submit_label=SubmitLabel::Send
            aria_label="Email"
        />
        <TextInput
            keyboard=Keyboard::Number
            read_only=true
            aria_label="Account number"
        />
    </View>
}
```

`Keyboard` offers `Text`, `Email`, `Number`, `Decimal`, `Phone`, `Url`, and `Search`. `Capitalize` offers `None`, `Words`, `Sentences`, and `Characters`. `SubmitLabel` offers `Done`, `Go`, `Next`, `Search`, and `Send`. `read_only` refuses edits but keeps the field selectable and copyable, and is announced as `aria-readonly`.

What each platform honours:

- The web gets a matching `type` (`email`, `tel`, `url`, `search`), or an `inputmode` (`numeric`, `decimal`) where HTML has no type, plus `readonly`, `maxlength`, `autocapitalize`, `autocorrect`, `spellcheck`, and `enterkeyhint`.
- iOS sets `keyboardType`, `autocapitalizationType`, `autocorrectionType`, `spellCheckingType`, and `returnKeyType`; a read-only field refuses edits through its delegate.
- Android sets `inputType` variations and flags, `imeOptions`, `InputFilter.LengthFilter`, and clears the key listener for a read-only field.
- GTK sets `input_purpose`, `set_editable`, `set_max_length`, and `InputHints` (capitalization and `NO_SPELLCHECK`).
- macOS keeps the field editable or not, truncates in the delegate on `max_length`, and turns the field editor's correction and spell checking off when `autocorrect` is false.
- Windows sends `EM_SETREADONLY` and `EM_LIMITTEXT`. Other hints are no-ops there.

Text the application sets is never rewritten by capitalization or correction; they shape what the person types.

### Take several lines of text

A `TextInput` holds one line: Enter submits it, and long text scrolls sideways. For a note, a comment, or a message, use `TextArea`. Enter adds a line, long lines wrap, and the field scrolls its own content once the text is taller than the field.

```rust
view! {
	<View class="gap-3 p-4">
		<TextArea
			value=draft
			placeholder="Write a note"
			aria_label="Note"
			max_length=2000
			class="w-full h-32"
			on:change=set_draft
		/>
		<Button on:press=save>"Save"</Button>
	</View>
}
```

It is controlled the same way as `TextInput`: `on:change` receives the proposed text, lines separated by `\n` on every platform, and the field shows the `value` you render next. `placeholder`, `read_only`, `max_length`, `capitalize`, `autocorrect`, and `autocomplete` work as they do on a `TextInput`. Because Enter adds a line, a `TextArea` never raises a `Submit` event; give the form a button, or handle a key chord with `on:key` where the platform delivers key presses.

Give it a height, such as `h-32`. Without one it is as tall as its text, so it starts one line tall and grows as lines are added.

What each platform draws:

- The web renders a `<textarea>`.
- macOS draws an `NSTextView` inside an `NSScrollView`, and iOS a `UITextView`. Neither has a placeholder of its own, so leptoskit draws one over the empty text. On macOS, focus is reported when the person starts editing, not when they click into the field.
- Android draws a multi-line `EditText` whose text starts at the top.
- Linux draws a `gtk::TextView` inside a `gtk::ScrolledWindow`. GTK's text view has no placeholder, so none is shown; name the field with a label.
- Windows draws a multi-line `EDIT` control with a vertical scroll bar. It shows no placeholder either, and honours only `read_only` and `max_length` of the hints.
- The terminal draws a wrapping editor: Up and Down move between lines, Home and End move within one, and the view follows the caret.
- HarmonyOS and the watch surfaces don't draw `TextArea`: HarmonyOS stops with an error and a watch surface refuses the tree.

`keyboard` picks the on-screen keyboard on iOS and Android only, and `submit_label` is ignored, since the return key adds a line. A single-line field can't claim to be multi-line: `aria_multiline(true)` on a `TextInput` or `PasswordInput` fails validation and points you here.

Use typed handlers for boolean and numeric controls. Keep the accepted value in your application state, then rebuild the screen after a change:

```rust
use leptoskit::components::helpers::Card;
use leptoskit::components::helpers::ListTile;
use leptoskit::prelude::*;

#[component]
pub fn Preferences(checked: bool, update: impl Fn(bool) + Send + Sync + 'static) -> Element {
	view! {
		<Card class="gap-4">
			<Text class="text-xl font-semibold">"Preferences"</Text>
			{ListTile(
				None,
				view! { <Text>"Notifications"</Text> },
				Some(view! {
					<Switch checked=checked aria_label="Notifications" on:toggle=update />
				}),
			)}
		</Card>
	}
}
```

`on_toggle` receives a proposed boolean. `on_value_change` receives a proposed slider value. Both renderers restore the committed value if the application rejects the proposal. Related radios share a validated `RadioGroup` and application-owned selection.

Use `Pressable("Accessible name")` for custom decorative button content. Register its action with `on_press`, then add text, images, and layout containers as children. Nested controls, scrolling descendants, and descendant event handlers are invalid. `Pressable` currently provides activation rather than React Native's full press lifecycle.

Use `ScrollView()` for a scroll container. `View().class("overflow-scroll")` also becomes a native scroll widget during resolution. Scroll views scroll on both axes. To react to scrolling, register a handler with `.on(NativeEventSlot::Scroll, handler)`: the web and every native backend report the scroll offset. The generated macOS, iOS, and Android apps don't report scroll events yet.

Use `ListView()` or `GridView(columns)` for eager collections that fill the remainder of a bounded column. Every item is built and laid out. For long lists, `VirtualList` builds only the items on screen; see [virtualized lists](17-virtualized-lists.md).

Add `overflow-hidden` when children must clip to a rounded container on both renderers:

```rust
let photo = view! {
	<View class="size-24 rounded-lg overflow-hidden">
		<Image source="avatar.jpg" class="size-24" aria_label="Ada's avatar" />
	</View>
};
```

Native platforms load images from local files only: a path or a `file://` URL, plus `resource://` names with the iOS backend and `android.resource://` URIs on Android. In the browser, the source is a URL. Give each platform a source it can load.

For supported utility families and their limits, use the [CSS coverage reference](../reference/css-coverage.md). For a complete shared-screen example, see [code reuse across platforms](../platforms/code-reuse.md).

## Show a dialog or a menu above the screen

A confirmation, a "new item" form and an account menu all sit above the rest of the screen. They close on Escape or a press outside, focus goes into them when they open, and it goes back where it was when they close. A `View` styled with `absolute` gives you none of that, and whatever comes later in the tree is drawn over it.

Use `Modal` for something the person must answer before going on, and `Popover` for something that hangs from a control and leaves the rest of the screen usable.

```rust
view! {
	<View class="gap-3 p-4">
		<Button id="delete" on:press=move || set_confirming(true)>"Delete note"</Button>
		<Modal
			open=confirming
			aria_label="Delete the note?"
			initial_focus="keep"
			on:dismiss=move || set_confirming(false)
		>
			<View class="w-80 gap-3 rounded-xl bg-white p-4">
				<Text>"Delete the note? This cannot be undone."</Text>
				<Button on:press=delete>"Delete"</Button>
				<Button id="keep" on:press=move || set_confirming(false)>"Keep it"</Button>
			</View>
		</Modal>
	</View>
}
```

Both stay in the tree while closed and show nothing. Your state says whether one is open. A dismissal is a request: `on:dismiss` runs when the person presses Escape or presses outside, and the overlay closes when you render it with `open` false. Leave the state alone to refuse, for example while a form inside is saving. Without an `on:dismiss` handler, only something inside the overlay can close it.

A `Modal` covers the whole screen, and what it holds sits in the middle. The `Modal` itself is the backdrop: give it a class such as `bg-black/50` to dim the screen, and style the panel inside it. `initial_focus` names the element that takes focus when it opens, by `id`. Without one, the first control inside does, so in a confirmation name the safe choice. Give a `Modal` an accessible name with `aria_label` or `aria_labelledby`; it has the `dialog` role already.

A `Popover` names the element it hangs from by `id`, and `placement` picks the side:

```rust
view! {
	<View class="items-end p-4">
		<Button id="me" on:press=move || set_menu_open(!menu_open)>"Account"</Button>
		<Popover
			anchor="me"
			open=menu_open
			placement=Placement::BelowEnd
			on:dismiss=move || set_menu_open(false)
		>
			<View class="w-48 rounded-lg bg-white p-2">
				<Button on:press=open_profile>"Profile"</Button>
				<Button on:press=sign_out>"Sign out"</Button>
			</View>
		</Popover>
	</View>
}
```

The placements are `BelowStart` (the default), `Below`, `BelowEnd`, `AboveStart`, `Above` and `AboveEnd`. Start and end follow the reading direction. A popover with no room on its side opens on the other side, and one that would leave the screen is moved back inside it. A press on the anchor of an open popover is left to the anchor's own handler, so a button that toggles its menu closes it once. A popover is as large as what it holds, and it has no role of its own: give its content the role that fits, such as `menu`.

In builder code these are `Modal(open)` and `Popover(anchor, open)`, with `.on_dismiss(...)`, `.initial_focus(...)` and `.placement(...)`.

For a confirmation with a title and a description, or a menu of actions with arrow keys, [`Dialog` and `Menu`](16-accessible-components.md#dialog) build on these and do the wiring for you.

What each platform does:

- **Web.** A `Modal` is a `<dialog>` opened with `showModal()`. The browser puts it above the page, makes the page behind it inert, keeps Tab inside it, and gives focus back when it closes. A server writes a modal that starts open as `<dialog open>`, so it is on screen before any script runs. A `Popover` is an element with `popover="auto"`, placed from its anchor's rectangle and kept beside it while the page scrolls. A popover that starts open appears once the page hydrates.
- **Terminal.** Overlays are drawn last, over everything. Tab stays inside an open modal, Escape asks the overlay on top to close, a click on a modal's backdrop or outside a popover asks too, and closing gives focus back. Tab moves between the controls in a popover, and asks it to close when it would carry focus out.
- **macOS.** A `Modal` is a sheet on the app's window, which dims the window, keeps clicks and the keyboard to itself, and gives focus back when it closes. The sheet is as large as what the modal holds, and the modal's own background is not drawn, because the sheet does the dimming. Escape asks to close; a press outside does not, since a sheet has no outside to press. A `Popover` is an `NSPopover` with an arrow pointing at its anchor. Escape and a press outside ask to close. `placement` picks above or below, and the popover is always centred on its anchor. `initial_focus` works for fields; macOS gives buttons keyboard focus only while Keyboard Navigation is on in System Settings.
- **iOS.** A `Modal` is a form sheet: on an iPad a card as large as what the modal holds, and on an iPhone a sheet across the width of the screen with what the modal holds centred at its top. The modal's own background is not drawn, because the sheet dims the screen behind it. A swipe down on the sheet asks to close, as do the Escape key on a hardware keyboard and the VoiceOver escape gesture. A `Popover` is a system popover with an arrow pointing at its anchor, on an iPhone as well as an iPad. A press outside asks to close. `placement` picks above or below, and the popover is always centred on its anchor. `initial_focus` raises the keyboard for a field and moves the VoiceOver cursor for anything else.
- **Android.** A `Modal` is a dialog in a window of its own, as large as what the modal holds and centred on the screen. The modal's own background is not drawn, because the dialog dims the screen behind it. The Back gesture, Escape on a hardware keyboard and a touch outside the dialog ask to close. A `Popover` is a popup window placed where `placement` says, beside its anchor, and it takes touches and focus while it is open. A touch outside it asks to close and goes no further. The Back gesture closes a popover at once and then tells your app: a popover your app keeps open comes back with the next change to the screen. `initial_focus` moves focus, and the TalkBack cursor with it, to the named element.
- **Linux and Windows.** For now an overlay is drawn as a layer in the right place, over what comes before it in the tree, and presses do not pass through an open modal. The rest is not there yet: nothing asks to dismiss it, so close it from a control inside; focus is not moved or kept inside; and a parent that clips or scrolls cuts it off. Put overlays at the top level of the screen until these platforms present them as sheets, popovers and windows of their own.
- **Watch surfaces** refuse a tree that holds a `Modal` or a `Popover`.

## Persist shared application state

Enable the `storage` feature to use `leptoskit::hooks::use_async_storage` with persistent preferences. The browser host opens a `LocalStorage<T>` namespace. A native host opens an application-private data directory. Both implement the same `AsyncStorage<T>` contract and return the same reactive handle for shared screen code.

```rust
let initialize =
	owner.with(|| leptoskit::hooks::use_async_storage(storage, "name", String::from("Guest")));
let preferences = initialize.await?;
```

Read `preferences.value` when constructing the shared tree. A button callback passes `(preferences.set_value)(new_name)` to the host's local executor. The hook updates the displayed value after persistence succeeds and returns errors without replacing the last successful value. Native file operations run on worker threads. Browser local storage completes immediately; custom async adapters can use IndexedDB or a host service.

The owner scopes the model across awaits. Saves, reloads, and removal serialize per handle. Disposing the owner while an operation is pending returns a typed disposal error. The host also supplies viewport dimensions; use `Viewport<f32>` to retain native Tailwind's fractional logical pixels.
