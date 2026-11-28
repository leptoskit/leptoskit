---
"leptoskit_config": minor
"leptoskit_native": minor
"leptoskit_appkit": minor
"leptoskit_uikit": minor
"leptoskit_bridge_ffi": minor
"leptoskit_cli": minor
---

# An app's own fonts on every platform

Declare the typefaces an app bundles once, in `leptoskit.toml`, and choose one with the class the web already uses:

```toml
[theme.fonts.hand]
family = "Skribble Noto Sketch"
faces = [
	{ file = "fonts/NotoSketch-Regular.ttf" },
	{ file = "fonts/NotoSketch-Bold.ttf", weight = 700 },
	{ file = "fonts/NotoSketch-Italic.ttf", style = "italic" },
]

[theme.fonts.display]
faces = [{ file = "fonts/Recursive.ttf", weight = [300, 1000] }]
```

`font-[var(--hand)]` now draws the family on the web, iOS, Android, and macOS, and every text inside the element inherits it. Faces are matched by weight and style as `@font-face` rules are, a variable face draws the weight asked for on its `wght` axis, and layout measures each label in the face that draws it.

- The build checks every declared file and stops with an error naming the `[theme.fonts]` entry and the path it looked at when a file is missing or isn't a TrueType or OpenType font.
- Native builds write `fonts.json` beside the app bundle; the generated iOS, Android, and macOS hosts register the files before their first layout. The web build writes and links `leptoskit-fonts.css`.
- `leptoskit_native::fonts` holds the manifest, the face selection, and `install` for hosts written in Rust; `NativeProps::font_family` and `TextStyle::{italic, face}` carry the choice to measurement and drawing. `font-sans`, `font-serif`, and `font-mono` now return to the system font on native platforms.
- The AppKit and UIKit backends draw and measure declared faces once installed. The host text measurer's C ABI gains the face to measure in, and `lk_fonts_install` and `lk_fonts_select` let Swift and Kotlin hosts install the manifest and choose faces exactly as layout does.
- Windows, Linux, HarmonyOS, watchOS, the terminal, and the Rust `leptoskit_android` backend still draw the system font.
