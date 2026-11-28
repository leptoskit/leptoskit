# Fonts

An app's typeface carries much of its look, and a browser loads one from a stylesheet. A phone app has no stylesheet: the font has to ship inside the app, and iOS, Android, and macOS each register and draw a bundled font in their own way. Put the font files in your project's `assets/` folder, name them once in `leptoskit.toml`, and choose a family with the same `font-[var(--name)]` class on every platform.

## Bundle a font

With the files at `assets/fonts/` in your project, declare each family under `[theme.fonts]`:

```toml
[theme.fonts.hand]
family = "Skribble Noto Sketch"
faces = [
	{ file = "fonts/NotoSketch-Regular.ttf" },
	{ file = "fonts/NotoSketch-Bold.ttf", weight = 700 },
	{ file = "fonts/NotoSketch-Italic.ttf", style = "italic" },
]

# A variable font: one file draws every weight its `wght` axis reaches.
[theme.fonts.display]
faces = [{ file = "fonts/Recursive.ttf", weight = [300, 1000] }]
```

Then choose a family with its name:

```rust
use leptoskit::prelude::*;

#[component]
pub fn Title() -> impl IntoView {
	view! {
		<View class="font-[var(--hand)]">
			<Text class="text-3xl font-bold">"Reap the dead market"</Text>
			<Button class="italic">"Hunt"</Button>
		</View>
	}
}
```

A family set on an element reaches every text inside it: labels, buttons, text inputs, password inputs, and text areas. `font-sans`, `font-serif`, and `font-mono` return to the system font for an element and everything inside it.

| Key            | Default        | Meaning                                                                                                                         |
| -------------- | -------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| table name     |                | The custom property the family is chosen with: `[theme.fonts.hand]` is `font-[var(--hand)]`. It may not name a color or length. |
| `family`       | the table name | The family name the web stylesheet declares.                                                                                    |
| `faces`        |                | The family's files, at least one.                                                                                               |
| `faces.file`   |                | The file's path inside `assets/`, as an `asset://` path is written. TrueType (`.ttf`) or OpenType (`.otf`).                     |
| `faces.weight` | `400`          | The weight the file draws, from 1 to 1000, or `[min, max]` for a variable font's `wght` axis.                                   |
| `faces.style`  | `"normal"`     | `"normal"` or `"italic"`.                                                                                                       |

The [typefaces example](https://github.com/leptoskit/leptoskit/tree/main/examples/typefaces) draws a static and a variable family beside the system font on the web, iOS, Android, and macOS.

## Which face draws

Each piece of text asks for a weight (`font-light`, `font-bold`, …) and a style (`italic`), and the family's faces are matched as browsers match `@font-face` rules:

1. The style comes first: the faces in the style asked for, or the other style when the family has none.
2. Then the weight: a face that draws it. Otherwise, for a weight from 400 to 500, the nearest heavier face up to 500, then the nearest lighter one, then the nearest heavier one; below 400 the nearest lighter face, then heavier; above 500 the nearest heavier face, then lighter.
3. A variable face draws the weight asked for, held within its `[min, max]`.

Native platforms never fake a weight or a slant: a family with no bold face draws bold text in its nearest weight, and a family with no italic face draws italic text upright. Browsers may thicken or slant a face that is missing, so declare the bold and italic files when they matter.

Layout measures each piece of text in the face that draws it, so a label in a wide hand-drawn face gets the room it needs, and changing a family lays the text out again.

## What each platform does

The build checks every declared file, then each platform loads the faces before its first layout:

| Platform                                         | How the fonts load                                                                                                                                                                                                                                       |
| ------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Web                                              | The build writes `leptoskit-fonts.css` beside `index.html` and links it there: an `@font-face` rule per face, with `font-display: swap`, and a `:root` rule naming each family (`--hand: "Skribble Noto Sketch", ui-sans-serif, system-ui, sans-serif`). |
| iOS                                              | The app bundles `fonts.json` and the `assets` folder. At start-up the host registers each file with Core Text for the app's process, and draws and measures with the same Core Text font.                                                                |
| Android                                          | The APK carries `fonts.json` and the files under its assets. The host loads each face with `Typeface.Builder`, setting the `wght` axis of a variable face, and measures with the same typeface it draws.                                                 |
| macOS                                            | As on iOS, from `Contents/Resources/fonts.json` and `Contents/Resources/assets`.                                                                                                                                                                         |
| Windows, Linux, HarmonyOS, watchOS, the terminal | The system font. The class is accepted and the build still checks the files, but nothing loads them yet.                                                                                                                                                 |

Wear OS and Meta Quest builds use the Android host, so they load the fonts the same way; they have not been checked on a device.

## A missing file

A declared file that is not in `assets/`, or that is not a TrueType or OpenType font, stops the build instead of drawing the system font:

```text
error: [theme.fonts.hand] faces[1] declares `fonts/NotoSketch-Bold.ttf`, which must be a font file in the app's assets folder
caused by: no font file at /path/to/app/assets/fonts/NotoSketch-Bold.ttf
```

A WOFF or WOFF2 file loads only in browsers, so `leptoskit.toml` rejects one with the reason, as it does a font collection (`.ttc`).

## Fonts and code push

Fonts ship with the store build, as every asset does. A code push update carries the guest alone: it can use any family the installed app declares, but a release that adds a font needs a new store build.

## Hosts you write yourself

The apps `leptoskit new` generates install the fonts for you. An app that drives a native backend from its own Rust host, such as `leptoskit_appkit` or `leptoskit_uikit`, installs them before its first layout:

```rust,no_run
use leptoskit_native::fonts;

let manifest = std::fs::read("fonts.json")?;
fonts::install(fonts::Fonts::from_json(&manifest)?, "assets")?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

The AppKit and UIKit backends then draw and measure the declared faces. The Android backend written in Rust, `leptoskit_android`, still draws the system font.

## Limits

- Of a variable font's axes, only `wght` follows the text. Declare italics as their own files: the `ital` and `slnt` axes stay at the file's defaults.
- Native platforms don't synthesize bold or italic, as described above.
- `tabular-nums`, `tracking-*`, and OpenType feature controls still apply only on the web. See [CSS coverage](../reference/css-coverage.md).
