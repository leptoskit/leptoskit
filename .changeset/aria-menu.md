---
"leptoskit_aria": minor
"leptoskit_tui": patch
---

# Menus and modal dialogs in leptoskit_aria

`Menu::new("account", open).button(avatar).item("Sign out", sign_out).on_toggle(set_open)` is a button that opens a list of actions. Opening moves focus to the first action, Up and Down move through them and wrap, Home and End go to the ends, and Enter or Space runs one. Choosing an action, Escape, Tab, or a press outside closes the menu and gives focus back to the button. `.content(...)` gives an action richer content than its label, and `.placement(...)` picks the side it opens on.

`Dialog::modal(open)` is the dialog as a `Modal`: above the screen, named by its title, described by its description, with focus kept inside on the web and in the terminal. `Dialog::container(open)` stays for a dialog laid out in the page.

In the terminal, Tab that would carry focus out of an open popover asks the popover to close, focus requests and `initial_focus` reach controls with a negative `tab_index`, and only the focused `Pressable` is underlined (every one was).

macOS, iOS, Android, Linux and Windows show a menu and run its actions, but the arrow keys, Escape and a press outside do nothing there yet.
