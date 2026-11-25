//! Declared fonts on macOS: the host installs the build's manifest, a class
//! chooses the family, and layout measures in the face AppKit draws.
#![cfg(all(feature = "app", target_os = "macos"))]
// The fonts are installed through the C ABI the Swift host calls.
#![allow(unsafe_code)]

use std::fmt::Write as _;
use std::path::PathBuf;

use leptoskit_appkit::AppKitTextMeasurer;
use leptoskit_bridge_ffi::app::App;
use leptoskit_bridge_ffi::app::lk_fonts_install;
use leptoskit_native::FontWeight;
use leptoskit_native::TextMeasurer;
use leptoskit_native::TextStyle;
use leptoskit_native::fonts::Face;
use leptoskit_yoga::AvailableSpace;
use leptoskit_yoga::Size;

/// The manifest the build writes for the typefaces example's `[theme.fonts]`.
const MANIFEST: &str = r#"{"families":[
	{"name":"hand","family":"Skribble Noto Sketch","faces":[
		{"file":"fonts/SkribbleNotoSketch-Latin.ttf","weight":[400,400],"italic":false}]},
	{"name":"gentle","family":"Skribble Gentle","faces":[
		{"file":"fonts/SkribbleVariableGentle-Latin.ttf","weight":[300,900],"italic":false}]}]}"#;
const TEXT: &str = "Reap the dead market";

/// The example's bundled `assets` folder, which holds the fixtures.
fn assets() -> PathBuf {
	PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/typefaces/assets")
}

/// Installs the fonts as the Swift host does, through the C ABI.
fn install() {
	let root = assets();
	let root = root.to_str().unwrap();
	// SAFETY: Both byte ranges outlive the call.
	let status =
		unsafe { lk_fonts_install(MANIFEST.as_ptr(), MANIFEST.len(), root.as_ptr(), root.len()) };

	assert_eq!(status, 0);
}

fn width(face: Option<Face>) -> f32 {
	let style = TextStyle {
		face,
		..TextStyle::new(20.0, FontWeight::Regular)
	};

	AppKitTextMeasurer::new()
		.measure(
			TEXT,
			style,
			Size::new(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
		)
		.width
}

#[test]
fn each_declared_face_measures_with_its_own_metrics() {
	install();
	let system = width(None);
	let hand = width(Some(Face {
		index: 0,
		weight: 400,
	}));
	let light = width(Some(Face {
		index: 1,
		weight: 300,
	}));
	let black = width(Some(Face {
		index: 1,
		weight: 900,
	}));

	for (name, measured) in [("hand", hand), ("light", light), ("black", black)] {
		assert!(measured > 0.0, "{name} measured {measured}");
		assert!(
			(measured - system).abs() > 1.0,
			"{name} measured {measured}, as wide as the system font's {system}"
		);
	}

	assert!(
		(hand - light).abs() > 1.0,
		"the two families measure alike: {hand} and {light}"
	);
	// Recursive keeps every weight's advances, so its weights set alike.
	assert_eq!(black.to_bits(), light.to_bits());
}

#[test]
fn a_class_lays_its_label_out_in_the_declared_face() {
	install();

	for (class, face) in [
		(
			"font-[var(--hand)]",
			Some(Face {
				index: 0,
				weight: 400,
			}),
		),
		(
			"font-[var(--gentle)] font-black",
			Some(Face {
				index: 1,
				weight: 900,
			}),
		),
		("font-mono", None),
	] {
		let guest = guest(class);
		let mut app = App::new(guest.as_bytes()).unwrap();
		app.with_host(|host| host.start(800.0, 400.0)).unwrap();
		let laid_out = app.with_host(|host| {
			let surface = host.surface();
			let root = surface.root().unwrap();
			let label = surface.children(root)[0];
			surface.frame(label).unwrap().width
		});
		let weight = if class.contains("font-black") {
			FontWeight::Black
		} else {
			FontWeight::Regular
		};
		let measured = AppKitTextMeasurer::new()
			.measure(
				TEXT,
				TextStyle {
					face,
					..TextStyle::new(20.0, weight)
				},
				Size::new(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
			)
			.width;

		assert_eq!(laid_out.to_bits(), measured.to_bits(), "{class}");
	}
}

fn guest(class: &str) -> String {
	let props = serde_json::json!({ "text": TEXT, "font_size": 20.0, "class": class }).to_string();
	let data = escape(&props);
	let root = r#"{"class":"flex flex-col items-start"}"#;
	let root_data = escape(root);
	format!(
		r#"(module
		(import "leptoskit" "node_create" (func $create (param i64 i32)))
		(import "leptoskit" "node_set_props" (func $props (param i64 i32 i32)))
		(import "leptoskit" "node_insert" (func $insert (param i64 i64 i32)))
		(import "leptoskit" "commit" (func $commit))
		(memory (export "memory") 1)
		(data (i32.const 0) "{root_data}")
		(data (i32.const 256) "{data}")
		(func (export "leptoskit_main")
		  (call $create (i64.const 1) (i32.const 0))
		  (call $create (i64.const 2) (i32.const 1))
		  (call $props (i64.const 1) (i32.const 0) (i32.const {root_length}))
		  (call $props (i64.const 2) (i32.const 256) (i32.const {length}))
		  (call $insert (i64.const 1) (i64.const 2) (i32.const 0))
		  (call $commit))
		(func (export "leptoskit_event") (param i64 i32 i32 i32) (result i32) i32.const 0))"#,
		root_length = root.len(),
		length = props.len()
	)
}

fn escape(text: &str) -> String {
	let mut escaped = String::new();

	for byte in text.bytes() {
		write!(escaped, "\\{byte:02x}").unwrap();
	}

	escaped
}
