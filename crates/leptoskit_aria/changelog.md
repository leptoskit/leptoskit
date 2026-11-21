# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-21)

### Features

#### Menus and modal dialogs in leptoskit_aria

_Owner:_ Ifiok Jr. · _Introduced in:_ [14db9ea](https://github.com/leptoskit/leptoskit/commit/14db9ea30962657d4f38f0d4629fdfa5d6d6df87)

`Menu::new("account", open).button(avatar).item("Sign out", sign_out).on_toggle(set_open)` is a button that opens a list of actions. Opening moves focus to the first action, Up and Down move through them and wrap, Home and End go to the ends, and Enter or Space runs one. Choosing an action, Escape, Tab, or a press outside closes the menu and gives focus back to the button. `.content(...)` gives an action richer content than its label, and `.placement(...)` picks the side it opens on.

`Dialog::modal(open)` is the dialog as a `Modal`: above the screen, named by its title, described by its description, with focus kept inside on the web and in the terminal. `Dialog::container(open)` stays for a dialog laid out in the page.

In the terminal, Tab that would carry focus out of an open popover asks the popover to close, focus requests and `initial_focus` reach controls with a negative `tab_index`, and only the focused `Pressable` is underlined (every one was).

macOS, iOS, Android, Linux and Windows show a menu and run its actions, but the arrow keys, Escape and a press outside do nothing there yet.

#### Arrow keys move through tabs and radio cards

_Owner:_ Ifiok Jr. · _Introduced in:_ [bc7c851](https://github.com/leptoskit/leptoskit/commit/bc7c8510366147ca4e256b1a1f3896717f4c50d4)

`leptoskit_aria::Tabs` is now one stop for the Tab key. Only the selected tab is in the Tab order; Left and Right (Up and Down after `.orientation(Orientation::Vertical)`), Home and End move to another tab, select it, and take focus with it.

**Breaking:** the selection callback moved from each tab to the list. Write `tabs.list(move |index| …)` and `tabs.tab(index, label)` where you wrote `tabs.list()` and `tabs.tab(index, label, move |index| …)`. `Tabs` no longer implements `PartialEq`.

`leptoskit_aria::RadioGroup` is new: a choice of one option among several where each option shows any content, such as a card. `RadioGroup::new(id, labels, selected).option(index, content).on_change(…)` becomes an element with `Element::from`. Options carry the `radio` role, their checked state and their position; the group is one Tab stop, and the arrow keys, Home and End move, choose and take focus along.

The arrow keys work on the web and in the terminal. macOS, iOS, Android, Windows and GTK do not deliver key presses to the app yet, so every tab and option stays reachable with Tab there, as before.

`leptoskit_testing::WidgetTester::press_key(finder, key)` presses a key on a widget and bubbles it to the nearest `on_key` handler.

### Fixes

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

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

### Breaking changes

#### leptoskit_aria: accessible primitives

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_aria` provides headless accessible components for the shared element tree: disclosure, toggle buttons, tabs, labeled fields, and dialogs with roles and state wired for web and native.

It owns behavior, not pixels: app code styles the pieces with Tailwind and gets consistent focus, keyboard, and ARIA semantics on every backend. `leptoskit_form` builds its fields on top of it.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
