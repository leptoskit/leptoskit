//! Every Android view that says why an app stopped is a `NativeFailureView`,
//! which brings its own colors. A plain `TextView` took the theme's dark
//! text, so a stopped app with a dark background looked like a black screen.
//!
//! `android-tests/NativeFailureViewTest.kt` draws the view on a device; this
//! holds the sources to it on every machine.

use std::path::Path;

const MOBILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/mobile");

fn kotlin_sources() -> Vec<(String, String)> {
	let mut sources: Vec<(String, String)> = std::fs::read_dir(MOBILE)
		.expect("the mobile sources")
		.map(|entry| entry.expect("a mobile source").path())
		.filter(|path| path.extension().is_some_and(|extension| extension == "kt"))
		.map(|path| {
			let name = path
				.file_name()
				.and_then(|name| name.to_str())
				.expect("a file name")
				.to_owned();
			(
				name,
				std::fs::read_to_string(&path).expect("a Kotlin source"),
			)
		})
		.collect();
	sources.sort();
	sources
}

/// The body of `object NativeFailureView`, up to the next top-level item.
fn failure_view() -> String {
	let source = std::fs::read_to_string(Path::new(MOBILE).join("LeptosKitView.kt"))
		.expect("LeptosKitView.kt")
		// Windows runners check the sources out with CRLF; the boundary
		// markers below are written against LF.
		.replace("\r\n", "\n");
	let start = source
		.find("internal object NativeFailureView {")
		.expect("LeptosKitView.kt defines NativeFailureView");
	let end = source[start..]
		.find("\n}\n")
		.expect("NativeFailureView ends");
	source[start..start + end].to_owned()
}

fn color(view: &str, name: &str) -> u32 {
	let line = view
		.lines()
		.find(|line| line.contains(&format!("const val {name}: Int = 0x")))
		.unwrap_or_else(|| panic!("NativeFailureView defines {name} as a literal color"));
	let hex = line
		.split("0x")
		.nth(1)
		.and_then(|rest| rest.get(..8))
		.expect("an ARGB literal");
	u32::from_str_radix(hex, 16).expect("hexadecimal")
}

fn luminance(argb: u32) -> f64 {
	let channel = |shift: u32| {
		let value = f64::from((argb >> shift) & 0xFF) / 255.0;
		if value <= 0.039_28 {
			value / 12.92
		} else {
			((value + 0.055) / 1.055).powf(2.4)
		}
	};
	0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
}

#[test]
fn every_android_failure_message_is_a_native_failure_view() {
	let mut descriptions = Vec::new();

	for (name, source) in kotlin_sources() {
		for line in source.lines() {
			if line.contains("\"leptoskit-error\"") {
				descriptions.push(format!("{name}: {}", line.trim()));
			}
		}
	}

	assert_eq!(
		descriptions,
		["LeptosKitView.kt: const val DESCRIPTION = \"leptoskit-error\"".to_owned()],
		"build failure messages with NativeFailureView.create, not a TextView of their own"
	);
}

#[test]
fn the_failure_view_brings_its_own_readable_colors_and_fills_the_app() {
	let view = failure_view();

	for required in [
		"setTextColor(TEXT)",
		"setBackgroundColor(BACKGROUND)",
		"FrameLayout.LayoutParams.MATCH_PARENT,\n                    \
		 FrameLayout.LayoutParams.MATCH_PARENT,",
	] {
		assert!(
			view.contains(required),
			"NativeFailureView lacks `{required}`"
		);
	}

	let (text, background) = (color(&view, "TEXT"), color(&view, "BACKGROUND"));
	assert_eq!(text >> 24, 0xFF, "the text is opaque");
	assert_eq!(background >> 24, 0xFF, "the background is opaque");
	let (light, dark) = (luminance(text), luminance(background));
	let contrast = (light.max(dark) + 0.05) / (light.min(dark) + 0.05);
	assert!(
		contrast >= 7.0,
		"contrast {contrast:.1}:1 is under WCAG AAA's 7:1"
	);
}

#[test]
fn an_app_that_cannot_start_shows_why_instead_of_closing() {
	let source = std::fs::read_to_string(Path::new(MOBILE).join("LeptosKitApplicationView.kt"))
		.expect("LeptosKitApplicationView.kt");

	// Every place the application view makes a guest's view catches what
	// its construction throws: a plugin that fails to register throws there.
	let mut constructions = 0;
	for (index, _) in source.match_indices("LeptosKitView(") {
		let before = &source[..index];
		let try_at = before
			.rfind("try {")
			.expect("a try before the construction");
		let closed = before[try_at..].matches('}').count();
		let opened = before[try_at..].matches('{').count();
		assert!(
			opened > closed,
			"LeptosKitApplicationView.kt line {}: construct the view inside a try",
			before.lines().count()
		);
		constructions += 1;
	}
	assert!(constructions >= 4, "found {constructions} constructions");
}
