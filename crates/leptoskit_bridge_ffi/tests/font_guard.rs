//! Every native renderer takes its fonts from one function per platform,
//! and that function tries the declared face first.
//!
//! Layout measures text in the face `font-[var(--name)]` chooses, so a view
//! that makes a system font of its own draws in a face layout did not
//! measure. This test reads the renderers' sources and fails on a system font
//! made anywhere but the designated functions, so a new text-bearing view has
//! to take its font from them, as the tests in `leptoskit_native`'s
//! `tests/fonts.rs` hold every text-bearing widget kind to measuring in it.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

/// Code that makes a font without a declared family.
const SYSTEM_FONTS: &[&str] = &[
	// Swift.
	".systemFont(",
	".boldSystemFont(",
	".italicSystemFont(",
	".monospacedSystemFont(",
	".monospacedDigitSystemFont(",
	".preferredFont(",
	"UIFont(name:",
	"NSFont(name:",
	"CTFontCreateWithName(",
	"CTFontCreateUIFontForLanguage(",
	// Kotlin.
	"Typeface.create(",
	"Typeface.defaultFromStyle(",
	"Typeface.DEFAULT",
	"Typeface.MONOSPACE",
	"Typeface.SANS_SERIF",
	"Typeface.SERIF",
	"typeface =",
	"setTypeface(",
	// Rust.
	"systemFontOfSize",
	"boldSystemFontOfSize",
	"fontWithName",
	"CTFont::with_name",
];

/// Where a system font may be made, how many times, and the call that each
/// such file makes first to draw a declared face instead.
const DESIGNATED: &[(&str, &str, usize, &str)] = &[
	// The font of every UIKit text view.
	(
		"leptoskit_bridge_ffi/mobile/WidgetTextStyle.swift",
		".systemFont(",
		1,
		"NativeFonts.font(",
	),
	// The font of every AppKit text view.
	(
		"leptoskit_bridge_ffi/desktop/MacWidgetRenderer.swift",
		".systemFont(",
		1,
		"NativeFonts.font(",
	),
	// A label's font before its first update, which passes the renderer's.
	(
		"leptoskit_bridge_ffi/desktop/MacTextLabel.swift",
		".systemFont(",
		1,
		"font: NSFont",
	),
	// The development status line, which shows no app text.
	(
		"leptoskit_bridge_ffi/mobile/MobileApplicationChrome.swift",
		".preferredFont(",
		1,
		"status.font",
	),
	// The Android system font, for text that names no declared family.
	(
		"leptoskit_bridge_ffi/mobile/NativeTextMeasurer.kt",
		"Typeface.create(",
		1,
		"NativeFonts.face(",
	),
	(
		"leptoskit_bridge_ffi/mobile/NativeTextMeasurer.kt",
		"Typeface.defaultFromStyle(",
		1,
		"NativeFonts.face(",
	),
	(
		"leptoskit_bridge_ffi/mobile/NativeTextMeasurer.kt",
		"Typeface.DEFAULT",
		1,
		"NativeFonts.face(",
	),
	// The measuring paint's typeface.
	(
		"leptoskit_bridge_ffi/mobile/NativeTextMeasurer.kt",
		"typeface =",
		1,
		"NativeFonts.face(",
	),
	// The typeface of every Android text view.
	(
		"leptoskit_bridge_ffi/mobile/NativeWidgetProps.kt",
		"typeface =",
		1,
		"NativeFonts.typeface(",
	),
	// The font every AppKit and UIKit text view the Rust backends draw, and
	// every measurement, takes.
	(
		"leptoskit_appkit/src/text.rs",
		"systemFontOfSize",
		1,
		"crate::fonts::declared(",
	),
	(
		"leptoskit_uikit/src/measure.rs",
		"systemFontOfSize",
		1,
		"crate::fonts::declared(",
	),
];

/// The folders, under `crates/`, that hold native renderers' sources.
const RENDERERS: &[&str] = &[
	"leptoskit_bridge_ffi/apple",
	"leptoskit_bridge_ffi/android",
	"leptoskit_bridge_ffi/desktop",
	"leptoskit_bridge_ffi/ios-controls",
	"leptoskit_bridge_ffi/mobile",
	"leptoskit_appkit/src",
	"leptoskit_uikit/src",
];

/// The files that load declared faces: they make fonts from the app's files.
const LOADERS: &[&str] = &[
	"leptoskit_bridge_ffi/mobile/NativeFonts.kt",
	"leptoskit_bridge_ffi/mobile/NativeFonts.swift",
	"leptoskit_appkit/src/fonts.rs",
	"leptoskit_uikit/src/fonts.rs",
];

#[test]
fn renderers_make_system_fonts_only_where_declared_faces_come_first() {
	let crates = Path::new(env!("CARGO_MANIFEST_DIR"))
		.parent()
		.expect("crates folder");
	let mut found = BTreeMap::<(String, &str), usize>::new();

	for folder in RENDERERS {
		for file in sources(&crates.join(folder)) {
			let name = file
				.strip_prefix(crates)
				.expect("a renderer source")
				.to_string_lossy()
				.replace('\\', "/");

			if LOADERS.contains(&name.as_str()) {
				continue;
			}

			let source = fs::read_to_string(&file).expect("a renderer source is UTF-8");

			for line in source.lines().filter(|line| !is_comment(line)) {
				for pattern in SYSTEM_FONTS {
					let count = line.matches(pattern).count();

					if count > 0 {
						*found.entry((name.clone(), *pattern)).or_default() += count;
					}
				}
			}
		}
	}

	let expected = DESIGNATED
		.iter()
		.map(|(file, pattern, count, _)| (((*file).to_owned(), *pattern), *count))
		.collect::<BTreeMap<_, _>>();

	assert_eq!(
		found, expected,
		"a renderer makes a font outside the designated font functions: take it from them, so \
		 text draws in the declared face layout measured"
	);

	for (file, _, _, declared_first) in DESIGNATED {
		let source = fs::read_to_string(crates.join(file)).expect("a designated source");

		assert!(
			source.contains(declared_first),
			"{file} no longer calls {declared_first}"
		);
	}
}

#[test]
fn the_scan_recognises_comments() {
	assert!(is_comment("    // UIFont.systemFont(ofSize: 12)"));
	assert!(is_comment("\t/// typeface = x"));
	assert!(is_comment(" * Typeface.DEFAULT"));
	assert!(!is_comment(
		"let font = UIFont.systemFont(ofSize: 12) // the fallback"
	));
}

/// Whether `line` is only a comment.
fn is_comment(line: &str) -> bool {
	let line = line.trim_start();

	line.starts_with("//") || line.starts_with("* ") || line.starts_with("/*")
}

/// Every Swift, Kotlin and Rust file under `folder`.
fn sources(folder: &Path) -> Vec<PathBuf> {
	let mut files = Vec::new();
	let mut pending = vec![folder.to_path_buf()];

	while let Some(folder) = pending.pop() {
		for entry in fs::read_dir(&folder).expect("a renderer folder") {
			let path = entry.expect("a folder entry").path();

			if path.is_dir() {
				pending.push(path);
			} else if path.extension().is_some_and(|extension| {
				["swift", "kt", "rs"]
					.iter()
					.any(|known| extension == *known)
			}) {
				files.push(path);
			}
		}
	}

	files.sort();
	files
}
