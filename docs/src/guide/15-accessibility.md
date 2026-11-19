# Accessibility

Accessibility in leptoskit is ARIA. The props are the WAI-ARIA names, the web renderer emits them as real attributes and semantic elements, and every native backend maps the same properties onto its platform: VoiceOver on iOS and macOS, TalkBack on Android, UI Automation on Windows, and AT-SPI through GTK. React Native converged on the same `aria-*` vocabulary, and GTK's accessible API mirrors it almost one to one, so one shared model covers every platform without a web-only escape hatch.

<Callout kind="note" title="Generated apps">

The full mapping below is what the web and the platform backend crates do. The macOS, iOS, and Android apps that `leptoskit new` generates currently pass on only each element's accessible name (`aria_label`) to VoiceOver and TalkBack, not its role or state. See [platforms](../platforms/readme.md#generated-apps-and-backend-crates).

</Callout>

## The props

Every element carries the same builder methods:

| Builder                                   | Web attribute                    | What it does                                                                       |
| ----------------------------------------- | -------------------------------- | ---------------------------------------------------------------------------------- |
| `aria_label`                              | `aria-label`                     | The accessible name, announced first. Overrides visible text.                      |
| `aria_labelled_by(["id"])`                | `aria-labelledby`                | Names the element after other elements, by `id`.                                   |
| `aria_description`                        | `aria-description`               | A description beyond the name.                                                     |
| `aria_described_by(["id"])`               | `aria-describedby`               | Describes the element with other elements; the usual home of help text and errors. |
| `role(Role::Heading)`                     | `role`                           | Overrides the widget kind's implicit role.                                         |
| `aria_level(n)`                           | `aria-level`                     | Heading, list, or tree depth, from one.                                            |
| `aria_hidden(true)`                       | `aria-hidden`                    | Hides the subtree from assistive technology while it stays visible.                |
| `aria_checked(Tristate::Mixed)`           | `aria-checked`                   | Check state of a custom checkbox, option, or switch.                               |
| `aria_pressed(true)`                      | `aria-pressed`                   | Toggle-button state.                                                               |
| `aria_selected` / `aria_expanded`         | same                             | Selection and expansion state.                                                     |
| `aria_current(Current::Page)`             | `aria-current`                   | Marks the current page, step, or item.                                             |
| `aria_value_now/min/max`                  | `aria-valuenow` and friends      | Range values for custom range widgets.                                             |
| `aria_value_text("50 percent")`           | `aria-valuetext`                 | A human-readable range value.                                                      |
| `aria_live(Live::Polite)`                 | `aria-live`                      | Makes the element a live region whose text changes are announced.                  |
| `aria_atomic` / `aria_busy`               | same                             | Whether the whole region is announced; whether it is mid-change.                   |
| `aria_modal`                              | `aria-modal`                     | A dialog that blocks the rest of the page.                                         |
| `aria_required` / `aria_invalid`          | same                             | Field requirements and validity.                                                   |
| `aria_read_only` / `aria_multiline`       | same                             | Field editing traits.                                                              |
| `aria_has_popup(HasPopup::Menu)`          | `aria-haspopup`                  | What a control opens.                                                              |
| `aria_controls(["id"])`                   | `aria-controls`                  | Links a control to what it controls.                                               |
| `aria_pos_in_set(n)` / `aria_set_size(n)` | `aria-posinset` / `aria-setsize` | Position in a set; radios fill these automatically.                                |
| `aria_orientation`                        | `aria-orientation`               | A widget's axis.                                                                   |
| `aria_role_description("slide")`          | `aria-roledescription`           | A human-readable role label.                                                       |
| `tab_index(0)`                            | `tabindex`                       | Joins or leaves sequential keyboard focus.                                         |
| `id("name")`                              | `id`                             | A stable identifier, for relationships and the DOM.                                |
| `aria_attr("aria-rowindex", "3")`         | verbatim                         | Any other `aria-*` attribute, emitted on the web, ignored natively.                |

`accessibility_label` remains as an alias of `aria_label`.

## Roles become semantic elements

The first rule of ARIA is to prefer native HTML, and the web renderer does: structural roles upgrade the element instead of adding a redundant `role` attribute.

```rust,ignore
view! { <Text role=Role::Heading aria_level=2>"Profile"</Text> } // <h2>Profile</h2>
view! { <Heading aria_level=2>"Profile"</Heading> } // the same, as a component
view! { <View role=Role::List /> } // <ul>
view! { <View role=Role::Navigation /> } // <nav>
view! { <View role=Role::Banner /> } // <header>
view! { <View role=Role::Main /> } // <main>
view! { <View role=Role::Separator /> } // <hr> without children
view! { <Divider /> } // a separator with orientation
```

Every `NativeKind` implies a role — a `Button` is a `button`, a `TextInput` a `textbox`, a `Switch` a `switch` — and an explicit `role` overrides it. A container that declares a control role joins the tab order and activates from Enter and Space, because no native element does it for it:

```rust,ignore
view! { <View role=Role::Button aria_label="Archive order" on:press=archive /> }
```

On native platforms, the same declarations map onto VoiceOver traits, TalkBack class names, NSAccessibility roles, UIA control types, and GTK accessible roles. Unmapped roles read as a group with the ARIA name as the role description.

## Naming and validation

Some contracts are checked before any tree commits, like radio groups and pressable labels:

- Text inputs need an accessible name: a label, a labelling reference, or at least a placeholder. A placeholder alone is the documented fallback name on every platform, but it disappears when typing — prefer a real label.
- Ids are unique within a tree, and relationship references point at ids that exist.
- A presentational role (`none`/`presentation`) cannot carry a name.
- `aria_level` starts at one, and passthrough `aria_attr` names must be lowercase `aria-*` attributes without a typed builder.

## Relationships and merging

`aria_labelled_by` and `aria_described_by` reference element ids. On the web the renderer keeps the id references and browsers compute the name live. Native accessibility APIs have no id references, so the renderer resolves them: referenced text is folded into the label before backends see it, and radio options learn their position in the group automatically.

### Labels that focus their field

On a web page, people expect a click on a field's label to put the caret in the field, and a click on "Remember me" to tick its checkbox. That needs no handler and no script. Give the field an `id`, give the `Text` beside it an `id`, and name the field after the text:

```rust
view! {
	<View class="gap-2">
		<Text id="email-label">"Email"</Text>
		<TextInput id="email" aria_labelled_by=["email-label"] on:change=set_email />
		<View class="flex-row gap-2">
			<Checkbox id="remember" checked=remember aria_labelled_by=["remember-label"] on:toggle=set_remember />
			<Text id="remember-label">"Remember me"</Text>
		</View>
	</View>
}
```

The [`Field`](16-accessible-components.md#field) helper and `leptoskit::form` already build this shape, so their labels work the same way.

- **Web:** the text renders as `<label for="email">`, so the browser focuses a text field and toggles a `Checkbox`, `Switch` or `Radio` when its label is clicked. It applies to `TextInput`, `PasswordInput`, `TextArea`, `Checkbox`, `Switch`, `Radio` and `Slider`.
- **Android:** the label's view is marked with `labelFor`, so TalkBack knows which field it names. A tap on the label does not move focus.
- **macOS, iOS, Windows, Linux and the terminal:** nothing changes. Clicking a label is not a convention there, and the field already carries the label's text as its name.

The text becomes a label only when the relationship is unambiguous: the field has an `id`, it is named by exactly one element, that element is a plain `Text` with no `role`, and no other field is named after the same text. A label and its field inside the same `For` or `Show` are matched; a label outside a `For` or `Show` whose field is inside it stays a plain text.

### Move focus once

Some actions should carry the person to the next thing to do. After "Send code", the caret belongs in the code field. After a menu closes, focus belongs back on the button that opened it. After a form fails, a screen reader should land on the error summary. Without that, a keyboard or screen-reader user has to hunt for where the screen changed.

Create a `FocusRequest`, give it to the widget with `focus_on`, and call `request()` from any handler:

```rust
#[component]
fn SignIn() -> impl IntoView {
	let to_code = FocusRequest::new();

	view! {
		<View class="gap-2">
			<Button on:press=move || { send_code(); to_code.request(); }>"Send code"</Button>
			<TextInput aria_label="Code" focus_on=to_code />
		</View>
	}
}
```

Each call to `request()` moves focus once, after the screen next updates. The person can then tab or click away, and later updates leave focus where they put it. That is the difference from `focused(true)`, which pulls focus back after every update and suits a widget that should always hold it, such as a chat prompt.

The handle is `Copy`, so any number of handlers can capture it. Give one handle to one widget; when two requests arrive in the same update, the newest wins. A widget that leaves the screen and comes back does not take focus again until `request()` is called again.

- **Web:** the element is focused with `focus()`. A `Text`, a heading or a container takes focus too: it gets `tabindex="-1"`, so it can be focused from code without joining the Tab order.
- **Terminal:** focus moves to any widget in the focus ring, and the widget hears `Focus` as it would from Tab.
- **Linux (GTK):** the widget grabs focus if GTK lets it take focus.
- **macOS:** text fields and text areas become first responder. Buttons, switches, checkboxes and sliders take focus only while Keyboard Navigation is turned on in System Settings, which is how macOS treats them everywhere. Text and containers are left alone.
- **Windows:** buttons, toggles, sliders and text fields take focus. Text and containers are left alone.
- **iOS:** a text field or text area takes the keyboard. Any other widget takes the VoiceOver cursor instead, because iOS has no keyboard focus for it without a hardware keyboard.
- **Android:** a text field takes input focus. Other widgets take focus when the device is driven by a keyboard or remote, and take the TalkBack cursor either way. The on-screen keyboard is not raised for you.

On macOS, Windows, iOS and Android the widget does not hear a `Focus` event for a move the app asked for, because events are held back while an update is applied. The web, the terminal and GTK deliver it.

### Keyboard shortcuts for the whole app

`on_key` hears keys pressed inside an element, which suits a field or a list. A shortcut such as ⌘K to search should work wherever focus is, even when nothing has focus, so it belongs to the app. Call `use_shortcut` in a component:

```rust
#[component]
fn Notes() -> impl IntoView {
	let search = FocusRequest::new();
	let drawer = RwSignal::new(false);

	use_shortcut(Shortcut::primary("k"), move || search.request());
	use_shortcut(Shortcut::key("w").unless_typing(), move || drawer.update(|open| *open = !*open));
	use_shortcut(Shortcut::key("w").shift().unless_typing(), move || drawer.set(false));

	view! {
		<TextInput aria_label="Search" focus_on=search />
	}
}
```

`Shortcut::primary` holds the platform's command key: ⌘ on Apple devices and Ctrl everywhere else, the same rule as `KeyPress::is_primary`. Add `.shift()` or `.alt()` for more modifiers. A letter matches in either case, so `Shortcut::key("w").shift()` is Shift+W. `unless_typing()` leaves the key alone while the person types in a text field, so a plain letter stays a letter there.

A shortcut lasts as long as the component that made it, and the latest `run` replaces the last one when the screen builds again. When two shortcuts match the same key, the one made last runs, so a screen's own shortcut wins over the app's. A key an element's `on_key` handles never reaches a shortcut, so a dialog that takes Enter or Escape keeps it. A shortcut that runs takes the key from the browser, so ⌘K does not also open the browser's search.

- **Web:** shortcuts listen on the window.
- **Native and terminal:** hosts don't deliver keys to shortcuts yet, so a shortcut does nothing there for now.

A plain container with a `role` or an `aria_label` becomes one accessibility element, merging its content the way React Native's `accessible` does — VoiceOver and TalkBack read the card below as a single item:

```rust,ignore
view! {
	<View aria_label="Order card">
		<Text>"Order #42"</Text>
	</View>
}
```

## Live regions

Live regions announce their changing text: politely once the reader is idle, assertively by interrupting.

```rust,ignore
view! { <Status>"3 items in your cart"</Status> } // role=status: polite, atomic
view! { <Alert>"Connection lost"</Alert> } // role=alert: assertive
```

The web emits the attributes and the browser announces changes. Android uses its native live region. iOS, macOS, and Windows announce when the resolved region text changes, at high priority for assertive regions. GTK exposes the text through the label; announcements need GTK 4.14, above this crate's current 4.6 minimum.

## Images

`Image` is decorative until named: the web renders `alt=""` and native backends leave it out of the accessibility tree. Name informative images with `aria_label`, and hide purely decorative subtrees with `aria_hidden(true)`.

## Testing

- `Finder::aria_role(Role::Button, "Continue")` finds elements by effective role and accessible name, like Testing Library's `getByRole` — the role a widget kind implies counts.
- `WidgetTester::semantics()` prints the semantics tree a screen reader would expose — roles, names, states, values, live politeness — for assertions and snapshots, like Flutter's semantics tree dump.
- Server-rendered HTML tests assert the real attributes; the browser suite exercises keyboard activation; the AppKit suite asserts NSAccessibility state; and the native testing harness drives real devices through their accessibility drivers.

## Limits and what comes next

Widget behaviors — disclosure, toggle buttons, tabs, field wiring, dialogs — now ship as headless components in `leptoskit_aria`; see the [accessible components guide](16-accessible-components.md). Focus scopes, traps, and restoration — the prerequisite for containing dialog focus — are still pending, as are custom accessibility actions (React Native's `accessibilityActions`, Flutter's `customSemanticsActions`). Runtime services such as announcements outside a live region, screen-reader detection, reduce-motion, and font-scale preferences belong in a follow-up `leptoskit_plugin_accessibility` runtime plugin, modelled on the speech plugin; the declarative model needs no extra library.
