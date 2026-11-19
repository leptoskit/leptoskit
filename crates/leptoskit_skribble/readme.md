<h1 align="center">leptoskit_skribble</h1>

<p align="center">
  <strong>leptoskit_skribble</strong> is hand-drawn components, charts, and maps for leptoskit, drawn by <a href="https://openbudgetfun.github.io/skribble/">skribble</a>'s framework-free Rust core.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://github.com/leptoskit/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_skribble"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_skribble"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_skribble.svg"></a>
  <a href="https://docs.rs/leptoskit_skribble"><img alt="docs.rs" src="https://docs.rs/leptoskit_skribble/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

Rough borders, sketched fills, pen marks, doodles, charts, and maps for apps written once with leptoskit. `SkribblePage`, `WiredBox`, `WiredCard`, `WiredCircle`, `WiredBadge`, `WiredButton`, `WiredInput`, `WiredCheckbox`, `WiredRadio`, `WiredSwitch`, `WiredProgress`, `WiredHighlight`, `WiredDivider`, and `Sketch` cover a screen; `charts` adds candlestick, line, and bar charts with technical indicators; and `maps` adds a hand-drawn map with pressable pins and zoom controls.

The ink is leptoskit `Vector` elements, which the web renderer shows as inline SVG and every native backend draws through its own path API, so every platform gets the same strokes. Ink paints with the element's text colour, and a `text-[…] dark:text-[…]` class picks it, so dark mode follows the host without redrawing.

The default `Theme` is skribble's sunny colourway. Colour-code cards with `theme.accent_at(i)`, keep `ButtonVariant::Primary` for the one loud action, and pick another colourway with `Theme::colorway(Colorway::Berry)`.

## How it fits

<!-- {=skribbleSummary} -->

It turns the drawings of [skribble](https://openbudgetfun.github.io/skribble/), a hand-drawn design system, into leptoskit components. `skribble_core` decides what to draw; this crate lays the ink out as `Vector` elements, so the same rough strokes render on the web and every native backend.

<!-- {/skribbleSummary} -->

skribble owns [`skribble_core`](https://github.com/openbudgetfun/skribble/tree/main/rust/core): the rough engine, the theme and colourways, and every drawing as framework-neutral path data (`InkDrawing`). This crate owns everything that is leptoskit's: elements, Tailwind classes, layout, events, and accessibility. When leptoskit changes, only this crate changes; when skribble draws something new, `leptoskit_skribble::ink::layer` renders it before a component exists.

## Use it

```rust
use leptoskit::prelude::*;
// leptoskit's prelude has a `Theme` of its own, so name skribble's.
use leptoskit_skribble::Theme;
use leptoskit_skribble::*;

pub fn welcome(on_start: impl Fn() + Send + Sync + 'static) -> Element {
	let theme = Theme::SKRIBBLE;
	view! {
		<SkribblePage class="items-center gap-6 p-6">
			<WiredCard seed=3 fill_style=FillStyle::Solid fill=theme.accent_at(0) class="gap-4">
				<WiredHighlight kind=HighlightKind::Marker seed=5>
					<Text class="text-2xl font-bold">"Draw it differently"</Text>
				</WiredHighlight>
				<WiredButton label="Start sketching" on_press=on_start />
			</WiredCard>
		</SkribblePage>
	}
}
```

Three examples render a static HTML page each, which doubles as a visual check of the ink in light and dark mode:

```sh
cargo run -p leptoskit_skribble --example notebook -- notebook.html
cargo run -p leptoskit_skribble --example charts -- charts.html
cargo run -p leptoskit_skribble --example map -- map.html
```

## Learn more

- [skribble for Rust](https://openbudgetfun.github.io/skribble/guides/rust)
- [skribble_core](https://github.com/openbudgetfun/skribble/tree/main/rust/core)
- [leptoskit documentation](https://leptoskit.dev/docs/)
- [Repository](https://github.com/leptoskit/leptoskit)
