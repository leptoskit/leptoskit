---
"leptoskit_router": minor
---

# Your own tab bar and sheet surface

`Tabs::bar` draws the tab bar with your own components instead of the built-in row of titles. It receives a `TabBar` with each tab and whether it is selected, and `TabBar::link` wraps whatever you draw in a link that switches tabs the same way the built-in bar does. `Stack::surface` draws the layer a modal or sheet route sits on, so it can use your app's paper and corners instead of plain white. Both keep the built-in test ids. `Tabs`, `Navigator`, and `IntoNavigator` now require `'static` route types, as `Router` already did.
