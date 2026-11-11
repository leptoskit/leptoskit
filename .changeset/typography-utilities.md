---
"release": minor
---

# Every font weight, and line height

Text takes all nine of Tailwind's weights: `font-thin`, `font-extralight`, `font-light`, `font-normal`, `font-medium`, `font-semibold`, `font-bold`, `font-extrabold`, and `font-black`. Before, anything outside `font-normal` to `font-bold` stopped the app as an unsupported utility. `FontWeight` gains the five new variants and `FontWeight::value`, the CSS number from 100 to 900. A platform whose system font lacks a weight draws the nearest one, as browsers do.

Line height is a text property now. `leading-none`, `leading-tight`, `leading-snug`, `leading-normal`, `leading-relaxed`, and `leading-loose` set it as a multiple of the font size, `leading-6` (any step on the spacing scale) as a fixed height, and `leading-[1.4]`, `leading-[150%]`, or `leading-[20px]` as you write it; `text-sm/6` sets a size and a line height together. Descendants inherit it, and a multiple follows each one's own font size. Every line takes exactly the line height with its glyphs centered, as in CSS, and layout reserves that height, on the web, macOS, iOS, Android, Linux, and HarmonyOS. Windows and the terminal keep the font's own line height, and native text fields keep their single-line height.

`NativeProps::line_height`, `LineHeight`, `Element::line_height`, and `TextStyle::line_height` carry it. A custom `TextMeasurer` whose platform cannot set line height can call `TextStyle::fit_line_height` to reserve what the renderer draws. `TextStyle` gains a field, so a struct literal needs `line_height: None` or `..TextStyle::new(…)`.
