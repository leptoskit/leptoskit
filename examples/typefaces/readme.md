# typefaces

Typefaces sets one line in the system font and in two families the app bundles: Skribble Noto Sketch, a hand-drawn face, and Skribble Gentle, a variable face that draws every weight from 300 to 900. The same page draws them in the browser, on iOS, on Android, and on macOS.

The fonts are declared once, in `leptoskit.toml`:

```toml
[theme.fonts.hand]
family = "Skribble Noto Sketch"
faces = [{ file = "fonts/SkribbleNotoSketch-Latin.ttf" }]

[theme.fonts.gentle]
family = "Skribble Gentle"
faces = [
	{ file = "fonts/SkribbleVariableGentle-Latin.ttf", weight = [300, 900] },
]
```

and chosen with a class, which every text inside the element inherits:

```rust,ignore
view! {
	<View class="font-[var(--hand)]">
		<Text class="text-2xl">"Reap the dead market"</Text>
	</View>
}
```

The example is a standalone workspace in the shape `leptoskit new` produces, with runner projects for iOS, Android, and macOS:

```bash
cd examples/typefaces
leptoskit dev                        # the web app, rebuilding on change
leptoskit build --platform ios       # or android, macos
cargo test                           # the page, laid out in the declared faces
```

See [Fonts](../../docs/src/guide/42-fonts.md) for how declaring a font works on each platform.

## The fonts

Both files are cut from the Skribble fonts at commit [`20af150`](https://github.com/openbudgetfun/skribble/tree/20af15049b3af35885dac03d546306b7a3e33c3a/packages), with fonttools 4.60.1, down to printable ASCII and a little punctuation:

```bash
U="U+0020-007E,U+00A0,U+00B7,U+2013-2014,U+2018-201D,U+2022,U+2026"
pyftsubset skribble_font_noto_sketch/assets/fonts/SkribbleNotoSketch-Regular.ttf \
	--output-file=SkribbleNotoSketch-Latin.ttf --unicodes="$U" --layout-features='kern,liga,calt'
fonttools varLib.instancer skribble_font_recursive/assets/fonts/SkribbleVariableGentle-Regular.ttf \
	MONO=0 CASL=0 slnt=0 CRSV=0.5 wght=300:900 -o gentle-wght.ttf
pyftsubset gentle-wght.ttf \
	--output-file=SkribbleVariableGentle-Latin.ttf --unicodes="$U" --layout-features='kern,liga,calt'
```

The instancer pins every axis of the Recursive-based face but weight, so the file keeps one `wght` axis from 300 to 900. Recursive keeps the same advance widths at every weight, so its weights differ in stroke, never in width.

Both fonts are under the SIL Open Font License 1.1, in `assets/fonts/OFL-noto-sketch.txt` and `assets/fonts/OFL-recursive.txt`. The same files are the fixtures of the macOS font tests in `crates/leptoskit_appkit` and `crates/leptoskit_bridge_ffi`.
