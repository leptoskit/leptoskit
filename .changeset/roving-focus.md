---
"leptoskit_aria": minor
"leptoskit_testing": minor
"leptoskit": minor
---

# Arrow keys move through tabs and radio cards

`leptoskit_aria::Tabs` is now one stop for the Tab key. Only the selected tab is in the Tab order; Left and Right (Up and Down after `.orientation(Orientation::Vertical)`), Home and End move to another tab, select it, and take focus with it.

**Breaking:** the selection callback moved from each tab to the list. Write `tabs.list(move |index| …)` and `tabs.tab(index, label)` where you wrote `tabs.list()` and `tabs.tab(index, label, move |index| …)`. `Tabs` no longer implements `PartialEq`.

`leptoskit_aria::RadioGroup` is new: a choice of one option among several where each option shows any content, such as a card. `RadioGroup::new(id, labels, selected).option(index, content).on_change(…)` becomes an element with `Element::from`. Options carry the `radio` role, their checked state and their position; the group is one Tab stop, and the arrow keys, Home and End move, choose and take focus along.

The arrow keys work on the web and in the terminal. macOS, iOS, Android, Windows and GTK do not deliver key presses to the app yet, so every tab and option stays reachable with Tab there, as before.

`leptoskit_testing::WidgetTester::press_key(finder, key)` presses a key on a widget and bubbles it to the nearest `on_key` handler.
