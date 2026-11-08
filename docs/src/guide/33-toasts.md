# Toasts

A toast is a short status message that appears over the screen, confirms an action, or reports an error without taking the user away from what they were doing. leptoskit's toasts follow [Ding](https://github.com/YusufCeng1z/ding): one pill at a time that arrives as a compact shape, springs open into a pill with text, morphs in place when a newer message arrives, and shrinks before it leaves.

```rust,ignore
use std::sync::Arc;
use std::time::Instant;

use leptoskit::prelude::*;

// ToastViewport takes shared callbacks, so this sample uses builders.
fn screen(toaster: &Toaster) -> Element {
	View()
		.class("flex-1")
		.child(
			Button("Save")
				.class("rounded-lg bg-emerald-600 px-4 py-2 text-white")
				.on_press(|| { /* toaster.success("Saved", ToastOptions::new()) in app state */ }),
		)
		.child(ToastViewport(
			toaster.state(),
			Arc::new(|_id| { /* dismiss */ }),
			Arc::new(|_id, _secondary| { /* act */ }),
		))
}

// Advance phases from a host timer: toaster.poll(Instant::now()).
let _ = (screen, Instant::now());
```

Hold a `Toaster` in app state. Call `success`, `error`, `warning`, `info`, or `loading`, then rebuild the tree (or a binding) from `toaster.state()` and keep calling `poll` with a monotonic clock so enter → open → closing → exit advances on Ding's timings (450 ms to open, 340 ms to shrink, 220 ms to leave, 5 s default life).

## What the pill does

| Phase     | Look                                                              | Motion                                              |
| --------- | ----------------------------------------------------------------- | --------------------------------------------------- |
| `Enter`   | 44 px circle with the status icon                                 | Enter spring 320 / 26 from off-screen at half scale |
| `Open`    | Expanded pill with title, optional description, actions, progress | Morph spring 340 / 27; body fades and slides in     |
| `Closing` | Compact circle again                                              | Morph spring, firmer close                          |
| `Exit`    | Gone                                                              | 150 ms ease-out leave (scale 0.92, slight drift)    |

Stack shuffle uses spring 400 / 36. Errors and loading toasts, and any toast with a button, stay until dismissed. Hover or focus should call `pause` / `resume` so the life timer waits, as Ding does.

A second call while a toast is live morphs that toast instead of stacking a second pill. Only the newest caller may `update` it, so a slow upload cannot overwrite a newer message. Repeating the exact same toast bumps a ×N counter.

## Actions and updates

```rust,ignore
use leptoskit::components::toast::ToastAction;
use leptoskit::components::toast::ToastKind;
use leptoskit::components::toast::ToastOptions;
use leptoskit::components::toast::ToastPatch;

let id = toaster.loading(
	"Uploading photo…",
	ToastOptions::new().id("upload").progress(0.0),
);
toaster.update(
	&id,
	ToastPatch::new()
		.progress(Some(0.6)),
);
toaster.update(
	&id,
	ToastPatch::new()
		.kind(ToastKind::Success)
		.title("Photo uploaded")
		.progress(None),
);

toaster.success(
	"Message deleted",
	ToastOptions::new().action(ToastAction::new("Undo", || {
		Some(ToastPatch::new().kind(ToastKind::Info).title("Restored"))
	})),
);
```

Returning a patch from an action morphs the pill; returning `None` dismisses it.

## Platforms

The toast tree is ordinary `View`, `Text`, `Button`, and `Vector` builders, so every backend that draws those kinds can show a toast. Spring enter, morph, and stack motion run where [animations](32-animations.md) already run: strongest on the generated iOS, Android, and macOS apps. The web plays entering springs as CSS `linear()` easings; `layout_transition` and `exiting` are not wired in the browser yet, so size morph and leave may jump there. Windows, Linux, the terminal, and HarmonyOS show the correct resting phase without interpolating. Pair the viewport with the polite and assertive live regions it already mounts for screen readers.
