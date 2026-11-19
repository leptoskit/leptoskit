---
"leptoskit_native": minor
"leptoskit_aria": patch
"leptoskit_tui": patch
"leptoskit_wasm": patch
---

# Hear the ⌘ key in key handlers on the web

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
