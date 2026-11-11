# Gestures

A button already knows how to be pressed. A canvas, a card, or a photo does not know how to be dragged, pinched, or held. `gesture` attaches that recognition to any element, and the same recognizers run on Android and Linux.

```rust
use leptoskit::prelude::*;

#[component]
pub fn Stage() -> Element {
	view! {
		<View
			class="h-64 w-full bg-slate-100"
			gesture=Gesture::pan().on_update(|event| {
				let _ = event.translation_x;
			})
		>
			<Text>"Drag"</Text>
		</View>
	}
}
```

`translation_x` and `translation_y` are how far the finger has moved from where it went down, in layout units. `on_begin` runs when the drag wins, `on_update` while it moves, and `on_end` when the finger lifts. `on_cancel` runs if the platform takes the pointer away, or the element leaves the tree, after the drag has started.

## Recognizers

| Gesture                 | Wins when                                                      | What you read             |
| ----------------------- | -------------------------------------------------------------- | ------------------------- |
| `Gesture::tap()`        | The pointer lifts within 18 layout units of where it went down | The point, on `on_end`    |
| `Gesture::long_press()` | The pointer stays inside that distance for 500 milliseconds    | `on_begin`, then `on_end` |
| `Gesture::pan()`        | The pointer travels 18 layout units                            | Translation and velocity  |
| `Gesture::pinch()`      | A second pointer lands on the same element                     | `scale`, starting at 1    |
| `Gesture::rotation()`   | A second pointer lands on the same element                     | `rotation`, in radians    |
| `Gesture::fling()`      | The pointer lifts faster than 800 layout units per second      | Velocity, on `on_end`     |

`max_distance`, `max_duration`, `min_duration`, `min_velocity`, `horizontal`, and `vertical` change those thresholds. A horizontal pan fails if the drag is clearly vertical, so a scroll beside it can take the pointer.

## One pointer, several recognizers

Recognizers on the element under the pointer, and on its ancestors, enter one arena. The deepest one that accepts wins, and the others fail. That is the same order hit testing already uses.

`simultaneous` lets two recognizers win together. A pinch and a rotation are the usual pair:

```rust
use leptoskit::prelude::*;

#[component]
pub fn Photo() -> Element {
	view! {
		<View
			class="h-64 w-64"
			gesture=Gesture::pinch().simultaneous(Gesture::rotation())
		>
			<Text>"Pinch"</Text>
		</View>
	}
}
```

`exclusive` gives the first gesture priority. The second can win only after the first fails, so a pan takes a drag and a tap takes a lift that never moved:

```rust
use leptoskit::prelude::*;

#[component]
pub fn Card() -> Element {
	view! {
		<View class="h-24 w-full" gesture=Gesture::pan().exclusive(Gesture::tap())>
			<Text>"Card"</Text>
		</View>
	}
}
```

`race` gives the gesture to whichever accepts first. A long press wins a hold, and a tap wins if the finger lifts before the hold is long enough.

Calling `gesture` twice keeps both. The second runs at the same time as the first.

## Buttons, scrolling, and sliders

A tap does not take the pointer away from the control underneath it, so a `Button` inside a tapped view still presses. A pan, long press, pinch, or rotation does take it: once the gesture wins, the button does not also click and a scroll does not also scroll.

A scroll view or a slider on the path wins a drag before an ancestor pan does. `blocks_native` on the pan reverses that, when the pan should move the screen instead of the scroller:

```rust
use leptoskit::prelude::*;

#[component]
pub fn Drawer() -> Element {
	view! {
		<View class="h-full w-full" gesture=Gesture::pan().horizontal().blocks_native()>
			<ScrollView class="h-full w-full">
				<Text>"Rows"</Text>
			</ScrollView>
		</View>
	}
}
```

`Gesture::native()` is the platform control's own claim, for a composition that wants to name it.

## Where it runs

Android's widget backend and Linux (GTK 4) deliver every pointer into this arena, in layout units, with the same rules. A pointer that is lifted, cancelled, or whose element leaves the tree always ends, so the next touch starts clean.

On the web, iOS, macOS, Windows, the terminal, and HarmonyOS, and in the generated Swift and Kotlin renderers, `gesture` is remembered on the element and does not yet receive a pointer stream. Presses and clicks on those targets are unchanged.
