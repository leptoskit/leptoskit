# Accessible components

The [accessibility guide](15-accessibility.md) covers the declarative model: every element carries WAI-ARIA roles, states, relationships, and live regions, and every backend maps them onto its platform. What that model leaves to each app is the widget behavior — the expanded state of a disclosure, the selection of a tab list, the label, help, and error wiring of a field. `leptoskit_aria` is that layer, in the spirit of react-aria and [leptos_aria](https://github.com/ifiokjr/leptos_aria): the component owns the semantics, the app owns the styling through the same Tailwind classes every leptoskit component takes.

```toml
[dependencies]
leptoskit_aria = "0.1"
```

Or enable `leptoskit`'s `aria` feature and import `leptoskit::aria::*`.

## The contract

Components are plain functions, or small values like `Field`, `Tabs`, `Dialog`, and `Menu`, built from current state plus callbacks. A screen owns its state — signals, atomics, or otherwise — and rebuilds the tree when it changes; each component renders the state it is given. The parts of one widget are separate elements, so layout stays the app's choice, and compound parts share a caller-chosen id prefix: the button and panel of a disclosure both derive their ids from the same `"faq"` prefix.

The renderer's validation enforces the wiring these components declare. A field whose input is labelled by an id no element carries, or a dialog without its title, is a tree that fails to commit. Wrong accessibility is a visible error, not a screen-reader-only surprise.

```rust,ignore
use leptoskit::component;
use leptoskit::view;
use leptoskit_aria::DisclosureButton;
use leptoskit_aria::DisclosurePanel;
use leptoskit_native::components::Text;
use leptoskit_native::components::View;

#[component]
fn Faq(open: bool, toggle: impl Fn(bool) + Send + Sync + 'static) -> leptoskit_native::Element {
	view! {
		<View>
			{DisclosureButton("faq", "Frequently asked questions", open, toggle)}
			{DisclosurePanel("faq", open).child(view! {
				<Text>"Only visible once expanded"</Text>
			})}
		</View>
	}
}
```

## The components

| Component                              | Contract                                                                    |
| -------------------------------------- | --------------------------------------------------------------------------- |
| `DisclosureButton` / `DisclosurePanel` | Expand and collapse a region, wired by `aria-expanded` and `aria-controls`. |
| `ToggleButton`                         | A pressed-state button, including the mixed state.                          |
| `Tabs`                                 | Tabs with selection, arrow keys, set position, and panel relationships.     |
| `RadioGroup`                           | A choice of one among cards or rows, as one Tab stop with arrow keys.       |
| `Field`                                | Label, help text, and error wiring for any input element.                   |
| `Dialog`                               | A dialog named by its title and described by its description.               |
| `Menu`                                 | A button that opens a list of actions, with arrow keys and focus return.    |

### Disclosure

The button names the action, carries `aria-expanded`, and references its panel with `aria-controls`; the panel is a region labelled by the button that stays mounted while collapsed, so the relationship always resolves and collapsed content is unreachable rather than absent. Pressing the button calls `on_toggle` with the proposed state; keeping it is the caller's, like every controlled control in leptoskit.

### ToggleButton

`aria-pressed` on a plain button, with the WAI-ARIA mixed-state convention: pressing an unpressed or mixed button proposes `true`, pressing a pressed one proposes `false`. For a two-state control that also works with switch announcements, `Switch` remains the native primitive.

### Tabs

A row of tabs should be one stop for the Tab key. Otherwise a keyboard user presses Tab once per tab to get past the row, on every page that has one. The convention is that Tab lands on the selected tab, and the arrow keys move between tabs.

One `Tabs` describes a tab set (its id prefix, how many tabs it has, and which one is selected) and builds every part:

```rust
#[component]
fn Settings() -> impl IntoView {
	let selected = remember(|| 0_usize);
	let tabs = Tabs::new("settings", 2, selected.get());

	view! {
		<View class="gap-4">
			{tabs
				.list(move |index| selected.set(index))
				.class("flex-row gap-2")
				.child(tabs.tab(0, "General"))
				.child(tabs.tab(1, "Notifications"))}
			{tabs.panel(0).child(view! { <Text>"General settings"</Text> })}
			{tabs.panel(1).child(view! { <Text>"Notification choices"</Text> })}
		</View>
	}
}
```

`tabs.list(on_select)` is the `tablist` container, `tabs.tab(index, label)` is each tab, and `tabs.panel(index)` is the panel it selects. `on_select` receives a tab's index when the tab is pressed or reached with a key. Because every part comes from the same value, exactly one tab is selected, every tab reports the same set size, and each tab's `aria-controls` names its own panel; an index outside the set panics. Panels stay mounted with the inactive ones collapsed, so switching keeps panel state.

Left and Right move to the previous and next tab and wrap at the ends. Home and End go to the first and last. Moving selects the tab and takes focus with it. Enter and Space activate the focused tab. For tabs stacked in a column, call `.orientation(Orientation::Vertical)` on the `Tabs`; Up and Down move instead, and the list reports `aria-orientation`.

- **Web and terminal:** as described. Only the selected tab is in the Tab order.
- **macOS, iOS, Android, Windows and Linux (GTK):** these backends do not deliver key presses to the app yet, so the arrow keys do nothing there. Every tab stays reachable with Tab, as before.

### RadioGroup

The native `Radio` is a small circle next to a label. A plan picker or a persona chooser wants the whole card to be the option, and still wants a screen reader to say "Ada, radio button, 1 of 2, checked".

`RadioGroup` takes one label per option and the index of the checked one, or `None` while nobody has answered. Give each option any content with `.option(index, content)`:

```rust
#[component]
fn Persona() -> impl IntoView {
	let chosen = remember(|| None::<usize>);
	let group = RadioGroup::new("persona", ["Ada", "Grace"], chosen.get())
		.option(0, view! { <Card><Text>"Ada"</Text><Text>"Writes the engine"</Text></Card> })
		.option(1, view! { <Card><Text>"Grace"</Text><Text>"Writes the compiler"</Text></Card> })
		.on_change(move |index| chosen.set(Some(index)));

	view! {
		<View class="gap-2">
			<Text id="persona-label">"Who are you today?"</Text>
			{Element::from(group).class("flex-row gap-3").aria_labelled_by(["persona-label"])}
		</View>
	}
}
```

Each option is a pressable with the `radio` role, named by its label, with its checked state and its position in the group. An option given no content shows its label as text. `on_change` receives the index of the option that was pressed or reached with a key; a group that is rebuilt with its old state refuses the choice.

The group is one stop for the Tab key: Tab lands on the checked option, or on the first when none is checked. Any arrow key moves to the next or previous option and wraps at the ends, Home and End go to the first and last, and moving chooses the option and takes focus with it. Enter and Space choose the focused option.

- **Web and terminal:** as described.
- **macOS, iOS, Android, Windows and Linux (GTK):** the arrow keys do nothing yet, and every option stays reachable with Tab.

The prelude already exports a `RadioGroup`: the name that ties native `Radio` controls together. Import this one by name, `use leptoskit::aria::RadioGroup;`, so the two do not collide.

Use the native `Radio` when a plain circle and label is what you want: it is the platform's own control on every backend.

### Field

One `Field` names an input and carries its help text and error. Place the label, help, and error elements around the input; `field.wire(input)` gives the input the field's id and connects them with `aria-labelledby` and `aria-describedby` — appended to any description the input already carries — and marks the value `aria-invalid` while an error is set. Build a fresh `Field` each render from current state so an error that appears rewires the input; announce the new error with an `Alert` when it should interrupt.

### Dialog

A confirmation should stop the person until they answer it: nothing behind it can be pressed, Tab stays inside it, and a screen reader announces its title when it opens. `Dialog` names the dialog, and `dialog.modal(open)` puts it above the screen:

```rust
#[component]
fn Draft() -> impl IntoView {
	let asking = remember(|| false);
	let dialog = Dialog::new("discard", "Discard draft?").description("This cannot be undone.");

	view! {
		<View>
			<Button on:press=move || asking.set(true)>"Discard"</Button>
			{dialog
				.modal(asking.get())
				.on_dismiss(move || asking.set(false))
				.initial_focus("keep")
				.child(view! {
					<View class="gap-3 rounded-xl bg-white p-6">
						{dialog.title_element(2)}
						{dialog.description_element()}
						<Button id="keep" on:press=move || asking.set(false)>"Keep writing"</Button>
						<Button on:press=discard>"Discard"</Button>
					</View>
				})}
		</View>
	}
}
```

One `Dialog` carries the id, the title, and an optional description. `dialog.title_element(level)` is the heading and `dialog.description_element()` is the description, and the dialog is labelled by the first and described by the second, so the references cannot drift. Validation rejects a dialog whose title element is missing. The dialog stays in the tree while closed, so its references always resolve.

`dialog.modal(open)` is a [`Modal`](07-shared-components.md#show-a-dialog-or-a-menu-above-the-screen), so everything on that page applies: `.on_dismiss(...)` hears Escape and a press outside, `.initial_focus(id)` names what takes focus, and focus goes back to where it was when the dialog closes. Put the safe answer first or name it with `initial_focus`, so Enter on a freshly opened dialog does not destroy anything.

- **Web:** a `<dialog>` opened with `showModal()`. The browser makes the page behind it inert, keeps Tab inside, gives focus back, and tells assistive technology that the dialog is modal.
- **Terminal:** drawn over the screen, with Tab kept inside and focus given back.
- **macOS, iOS, Android, Windows and Linux (GTK):** drawn over the screen, but the screen behind still takes presses and Tab still leaves the dialog. Close it from its own buttons.

`dialog.container(open)` is the older form: a dialog laid out in the page where you put it, shown or collapsed, with no top layer and no focus handling. Use it for a panel that is a dialog in name only, such as an inline confirmation inside a card. It is not marked `aria-modal`, because nothing keeps focus inside it.

### Menu

An account menu is a button that opens a short list of actions. The keyboard convention is strict: opening puts focus on the first action, the arrow keys move through the list, Escape closes it, and focus returns to the button. Getting any of that wrong leaves a keyboard user stranded inside the list or dropped at the top of the page.

`Menu` takes the button, the actions, and whether it is open:

```rust
#[component]
fn Account() -> impl IntoView {
	let open = remember(|| false);
	let menu = Menu::new("account", open.get())
		.button(view! { <Button>"Account"</Button> })
		.item("Profile", open_profile)
		.item("Sign out", sign_out)
		.placement(Placement::BelowEnd)
		.on_toggle(move |next| open.set(next));

	view! { <View class="items-end p-4">{Element::from(menu)}</View> }
}
```

`.button(...)` is any control: a `Button`, or a `Pressable` around an avatar. The menu gives it the id `account-button`, tells assistive technology that it opens a menu and whether the menu is open, and toggles on a press. Each `.item(label, on_select)` is one action. Follow an item with `.content(...)` to show an icon beside the words or a second line; the label still names it. `.placement(...)` picks the side of the button the list opens on, and it opens below the button unless told otherwise.

Your screen owns whether the menu is open. `on_toggle` is called with `true` when the button is pressed while closed, and with `false` when an action is chosen, when the button is pressed while open, and when the person dismisses the menu. Store the value and the next build shows it.

Opening moves focus to the first action. Up and Down move between actions and wrap at the ends, and Home and End go to the first and last. Enter or Space runs the focused action. Choosing an action, Escape, Tab, or a press outside closes the menu, and focus goes back to the button. Moving through a menu never runs an action. The actions are out of the Tab order: Tab reaches the button, and the arrows reach the rest.

The list is a [`Popover`](07-shared-components.md#show-a-dialog-or-a-menu-above-the-screen) with the `menu` role, and each action is a pressable with the `menuitem` role.

- **Web and terminal:** as described.
- **macOS, iOS, Android, Windows and Linux (GTK):** the list shows beside the button and a press on an action runs it and closes the list. The arrow keys, Escape and a press outside do nothing yet, so the button and the actions are how it closes.

A menu is for actions. For a choice that stays chosen, such as a theme or a sort order, use a `RadioGroup`.

## Testing

The components are ordinary element trees, so the testing guide applies directly:

- `Finder::aria_role(Role::Tab, "Notifications")` finds tabs by role and name, and `tester.tap` drives selection through the real event dispatch.
- `WidgetTester::semantics()` prints what assistive technology sees — each tab's selection and position in the set show up in the tree.
- Wiring failures are `Element::validate()` errors: a test that builds a dialog without its title asserts the invalid tree up front.

## What comes next

More widget families — list boxes with selection, accordion (a disclosure group with exclusive opening), tooltips, and announcements outside a live region — follow the same pattern. Each one is behavior and semantics over the same shared tree, not a new platform surface.
