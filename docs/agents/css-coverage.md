# CSS coverage evidence

Which tests back each row of the public [CSS and Tailwind coverage reference](../src/reference/css-coverage.md), and how to run them.

## Evidence by module

- [Box Model](https://www.w3.org/TR/css-box-3/): The [box tests](https://github.com/leptoskit/leptoskit/blob/main/crates/leptoskit_yoga/tests/css_boxes.rs) cover decoration sizes and percentage padding.
- [Sizing](https://www.w3.org/TR/css-sizing-3/): Tests cover constraints and aspect ratio with padding and borders. A [layout test](https://github.com/leptoskit/leptoskit/blob/main/crates/leptoskit_yoga/tests/layout.rs) covers a root filling the available width within its margins and limits, and a [renderer test](https://github.com/leptoskit/leptoskit/blob/main/crates/leptoskit_native/tests/renderer.rs) covers container roots filling the viewport while text and control roots keep their size.
- [Flexible Box Layout](https://www.w3.org/TR/css-flexbox-1/): Box tests cover fractional flex factors, constraint freezing, and reverse plus RTL.
- [Positioned Layout](https://www.w3.org/TR/css-position-3/): Tests cover signed offsets, direction-dependent edge precedence, grid-area offsets, and explicit dimensions with opposing insets.
- [Display](https://www.w3.org/TR/css-display-3/): [Component tests](https://github.com/leptoskit/leptoskit/blob/main/crates/leptoskit_native/tests/components.rs) cover hidden subtrees and restoration after resizing.
- [Overflow](https://www.w3.org/TR/css-overflow-3/): Component tests cover overflow hit testing.
- [Fonts](https://www.w3.org/TR/css-fonts-3/): Tests compare measured label sizes with actual AppKit cells.
- [Color](https://www.w3.org/TR/css-color-4/): [Compiler tests](https://github.com/leptoskit/leptoskit/blob/main/crates/leptoskit_native/tests/tailwind.rs) check resolved colors and palette entries.
- [Backgrounds and Borders](https://www.w3.org/TR/css-backgrounds-3/): The widget suites that painted radius clipping are not in the tree.
- [Syntax](https://www.w3.org/TR/css-syntax-3/) and [Values and Units](https://www.w3.org/TR/css-values-3/): Tests reject malformed brackets, non-finite numbers, invalid ranges, and unsafe tokens.
- [Media Queries](https://www.w3.org/TR/mediaqueries-4/) and [Conditional Rules](https://www.w3.org/TR/css-conditional-3/): Tests cover boundaries and environment changes.
- [Selectors](https://www.w3.org/TR/selectors-4/) (`active`): [compiler tests](https://github.com/leptoskit/leptoskit/blob/main/crates/leptoskit_native/tests/tailwind.rs) cover precedence against breakpoints, dark mode, and important rules, and the `:active` CSS. [Pressed-state tests](https://github.com/leptoskit/leptoskit/blob/main/crates/leptoskit_native/tests/pressed_state.rs) drive holds through the headless surface: in-place updates, the default dim, disabled and hidden targets, replacement, and validation. The [web tests](https://github.com/leptoskit/leptoskit/blob/main/crates/leptoskit_native/tests/web.rs) check the scoped `:active` rules, and the [Wasm host tests](https://github.com/leptoskit/leptoskit/blob/main/crates/leptoskit_wasm/tests/controls.rs) check that press phases never enter the guest. Headless presses live in the pressed-state tests. UIKit touch events are in `native_probe`, and the Android button's pressed state is in `ProbeActivity`. The AppKit, GTK, and Win32 widget suites and the browser interaction suite are not in the tree.

## Sources, tests, and commands

The relevant source and tests are:

- [Utility parser and declaration compiler](https://github.com/leptoskit/leptoskit/tree/main/crates/leptoskit_native/src/tailwind).
- [Utility parsing, precedence, CSS output, and real-layout tests](https://github.com/leptoskit/leptoskit/blob/main/crates/leptoskit_native/tests/tailwind.rs).
- [Shared component, inheritance, responsive layout, visibility, and event tests](https://github.com/leptoskit/leptoskit/blob/main/crates/leptoskit_native/tests/components.rs).
- [Yoga layout tests](https://github.com/leptoskit/leptoskit/blob/main/crates/leptoskit_yoga/tests/layout.rs) and [CSS box tests](https://github.com/leptoskit/leptoskit/blob/main/crates/leptoskit_yoga/tests/css_boxes.rs).
- [Web server-rendering tests](https://github.com/leptoskit/leptoskit/blob/main/crates/leptoskit_native/tests/web.rs).

Run the implementation checks with:

```bash
cargo test --package leptoskit_yoga
cargo test --package leptoskit_native --all-features
cargo test --package leptoskit_appkit --test appkit
```

The AppKit tests run only on macOS. Browser tests use the workspace's `wasm-bindgen-test-runner` and an installed ChromeDriver:

```bash
CHROMEDRIVER=/path/to/chromedriver cargo test --package leptoskit_native --features web --target wasm32-unknown-unknown --test web_browser
```

These suites exercise the named contracts. They are not a CSS conformance suite or evidence of every possible combination of supported declarations.
