//! Declared fonts: the family `font-[var(--name)]` chooses, how text
//! inherits it, the face each style selects, and the style every
//! text-bearing widget is measured in.
//!
//! The manifest is process-wide, so every test here installs the same one.

#[cfg(feature = "serde")]
use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::Mutex;

use leptoskit_native::Element;
use leptoskit_native::FontFamily;
use leptoskit_native::FontWeight;
use leptoskit_native::HeadlessBackend;
#[cfg(feature = "serde")]
use leptoskit_native::NativeKind;
use leptoskit_native::NativeProps;
use leptoskit_native::NativeSurface;
use leptoskit_native::StyleContext;
use leptoskit_native::Tailwind;
use leptoskit_native::TextMeasurer;
use leptoskit_native::TextStyle;
use leptoskit_native::fonts;
use leptoskit_native::fonts::DeclaredFamily;
use leptoskit_native::fonts::Face;
use leptoskit_native::fonts::FontFace;
use leptoskit_native::fonts::Fonts;
#[cfg(feature = "serde")]
use leptoskit_native::wire::KINDS;
use leptoskit_yoga::AvailableSpace;
use leptoskit_yoga::Size;
use leptoskit_yoga::Style;

fn manifest() -> Fonts {
	let face = |file: &str, weight: u16, italic: bool| {
		FontFace {
			file: file.to_owned(),
			weight: [weight, weight],
			italic,
		}
	};

	Fonts {
		families: vec![
			DeclaredFamily {
				name: "hand".to_owned(),
				family: "Skribble Noto Sketch".to_owned(),
				faces: vec![
					face("fonts/Sketch-Regular.ttf", 400, false),
					face("fonts/Sketch-Bold.ttf", 700, false),
					face("fonts/Sketch-Italic.ttf", 400, true),
				],
			},
			DeclaredFamily {
				name: "figures".to_owned(),
				family: "Skribble Variable".to_owned(),
				faces: vec![FontFace {
					file: "fonts/Variable.ttf".to_owned(),
					weight: [300, 1000],
					italic: false,
				}],
			},
		],
	}
}

fn install() -> &'static Fonts {
	fonts::install(manifest(), "bundle/assets").expect("one manifest per process")
}

fn props(classes: &str) -> NativeProps {
	install();
	let mut style = Style::default();
	let mut props = NativeProps::default();
	Tailwind::parse(classes)
		.unwrap()
		.apply(&mut style, &mut props, &StyleContext::default());
	props
}

#[test]
fn installing_is_once_per_process() {
	let installed = install();

	assert_eq!(installed, &manifest());
	assert_eq!(fonts::installed(), Some(installed));
	assert_eq!(
		fonts::install(manifest(), "bundle/assets").map(|fonts| fonts.families.len()),
		Ok(2)
	);
	assert_eq!(
		fonts::install(Fonts::default(), "bundle/assets"),
		Err(fonts::InstallError)
	);
	assert_eq!(
		fonts::install(manifest(), "elsewhere"),
		Err(fonts::InstallError)
	);
	assert_eq!(
		fonts::path(1),
		Some(
			std::path::Path::new("bundle/assets")
				.join("fonts")
				.join("Sketch-Bold.ttf")
		)
	);
	assert_eq!(fonts::path(9), None);
}

#[test]
fn a_theme_variable_chooses_the_declared_family() {
	assert_eq!(
		props("font-[var(--hand)]").font_family,
		Some(FontFamily::Declared(0))
	);
	assert_eq!(
		props("font-[var(--figures)]").font_family,
		Some(FontFamily::Declared(1))
	);
	// A variable no font declares leaves the family to inheritance.
	assert_eq!(props("font-[var(--brand)]").font_family, None);
	assert_eq!(props("font-mono").font_family, Some(FontFamily::System));
	// The web stylesheet still leaves the variable to the browser.
	assert!(
		Tailwind::parse("font-[var(--hand)]")
			.unwrap()
			.css()
			.contains("font-family:var(--hand);")
	);
}

#[test]
fn the_text_style_carries_the_face_for_its_weight_and_style() {
	let face = |classes: &str| props(classes).text_style().face;

	assert_eq!(
		face("font-[var(--hand)]"),
		Some(Face {
			index: 0,
			weight: 400
		})
	);
	assert_eq!(
		face("font-[var(--hand)] font-bold"),
		Some(Face {
			index: 1,
			weight: 700
		})
	);
	assert_eq!(
		face("font-[var(--hand)] font-black"),
		Some(Face {
			index: 1,
			weight: 700
		})
	);
	assert_eq!(
		face("font-[var(--hand)] italic"),
		Some(Face {
			index: 2,
			weight: 400
		})
	);
	assert_eq!(
		face("font-[var(--figures)] font-semibold"),
		Some(Face {
			index: 3,
			weight: 600
		})
	);
	assert_eq!(
		face("font-[var(--figures)] font-thin"),
		Some(Face {
			index: 3,
			weight: 300
		})
	);
	assert_eq!(face("font-sans font-bold"), None);
	assert_eq!(face("font-bold"), None);

	let italic = props("italic font-[var(--hand)]").text_style();
	assert!(italic.italic);
	assert!(!TextStyle::default().italic);
	assert_eq!(TextStyle::default().face, None);
	assert_eq!(
		props("font-[var(--hand)]").font_face(FontWeight::Bold.value(), true),
		Some(Face {
			index: 2,
			weight: 400
		})
	);
}

/// Records every style a surface measures text in, by the text measured.
#[derive(Default)]
struct Recorder {
	measured: Mutex<Vec<(String, TextStyle)>>,
}

impl TextMeasurer for Recorder {
	fn measure(&self, text: &str, style: TextStyle, _: Size<AvailableSpace>) -> Size<f32> {
		self.measured.lock().unwrap().push((text.to_owned(), style));

		// Wider for a declared face, so layout shows which one measured.
		let width = if style.face.is_some() { 20.0 } else { 10.0 };
		Size::new(width * text.chars().count() as f32, style.font_size)
	}
}

fn surface(recorder: &Arc<Recorder>) -> NativeSurface<HeadlessBackend> {
	let measurer: Arc<dyn TextMeasurer> = recorder.clone();
	let mut surface = NativeSurface::new(HeadlessBackend::new().with_text_measurer(measurer));
	surface.set_viewport(4000.0, 4000.0);
	surface
}

#[test]
fn descendants_inherit_the_family_and_the_stacks_reset_it() {
	install();
	let recorder = Arc::new(Recorder::default());
	let mut surface = surface(&recorder);

	surface.set_root(
		&Element::view()
			.class("font-[var(--hand)] items-start")
			.child(Element::text("inherits"))
			.child(Element::text("resets").class("font-mono"))
			.child(Element::text("bold").class("font-bold")),
	);

	let measured = recorder.measured.lock().unwrap().clone();
	let face = |text: &str| {
		measured
			.iter()
			.find(|(measured, _)| measured == text)
			.map(|(_, style)| style.face)
			.expect("measured")
	};

	assert_eq!(face("inherits").map(|face| face.index), Some(0));
	assert_eq!(face("resets"), None);
	assert_eq!(face("bold").map(|face| face.index), Some(1));

	let root = surface.root().unwrap();
	let children = surface.children(root).to_vec();
	let width = |index: usize| surface.frame(children[index]).unwrap().width;

	// The declared face measures twice as wide as the system face here.
	assert_eq!(width(0), 160.0);
	assert_eq!(width(1), 60.0);
}

#[test]
fn changing_the_family_remeasures_the_text() {
	install();
	let recorder = Arc::new(Recorder::default());
	let mut surface = surface(&recorder);
	let tree = |class: &str| {
		Element::view()
			.class(class)
			.class("items-start")
			.child(Element::text("Reap"))
	};

	surface.set_root(&tree("font-sans"));
	let root = surface.root().unwrap();
	let text = surface.children(root)[0];
	assert_eq!(surface.frame(text).unwrap().width, 40.0);

	surface.set_root(&tree("font-[var(--hand)]"));
	let text = surface.children(surface.root().unwrap())[0];
	assert_eq!(surface.frame(text).unwrap().width, 80.0);
}

/// Every widget kind that draws text is measured in the declared face.
///
/// A new kind that measures text must join `TEXT_KINDS`, and every platform
/// renderer must draw it with the face its style carries, which
/// `leptoskit_bridge_ffi`'s `font_guard` test holds the renderers to.
#[cfg(feature = "serde")]
#[test]
fn every_text_bearing_kind_measures_in_the_declared_face() {
	const TEXT_KINDS: [NativeKind; 6] = [
		NativeKind::Text,
		NativeKind::TextRun,
		NativeKind::TextInput,
		NativeKind::Button,
		NativeKind::PasswordInput,
		NativeKind::TextArea,
	];

	install();
	let recorder = Arc::new(Recorder::default());
	let mut unmounted = BTreeSet::new();

	for kind in KINDS {
		let label = format!("{kind:?}");
		let mut element = Element::new(kind);
		element.props.text = Some(label.clone());
		element.props.value = Some(label.clone());
		element.props.placeholder = Some(label.clone());
		element.props.accessibility_label = Some(label.clone());
		// The kinds that show something the app owns need a handle to it.
		match kind {
			NativeKind::Scene3d => element.props.scene3d = Some(1),
			NativeKind::Video => element.props.video = Some(1),
			NativeKind::Camera => element.props.camera = Some(1),
			NativeKind::Map => element.props.map = Some(1),
			NativeKind::LivePhoto => element.props.live_photo = Some(1),
			NativeKind::CanvistEditor => element.props.canvist_editor = Some(1),
			NativeKind::Radio => {
				element.props.radio_group =
					Some(leptoskit_native::RadioGroup::new("group").unwrap());
			}
			_ => {}
		}

		let root = Element::view()
			.class("font-[var(--hand)] font-bold")
			.child(element);

		if let Err(error) = surface(&recorder).try_set_root(&root) {
			unmounted.insert((label, error.to_string()));
		}
	}

	// Rive needs real playback content, and draws its text inside the
	// artboard, never through the platform's text system.
	assert_eq!(
		unmounted
			.iter()
			.map(|(kind, _)| kind.as_str())
			.collect::<Vec<_>>(),
		["Rive"],
		"{unmounted:?}"
	);

	let measured = recorder.measured.lock().unwrap().clone();
	let kinds: BTreeSet<String> = measured.iter().map(|(text, _)| text.clone()).collect();
	let expected: BTreeSet<String> = TEXT_KINDS.iter().map(|kind| format!("{kind:?}")).collect();

	assert_eq!(kinds, expected, "the kinds that measure text");

	for (text, style) in &measured {
		assert_eq!(
			style.face,
			Some(Face {
				index: 1,
				weight: 700
			}),
			"{text} measures in the declared bold face"
		);
	}
}

#[cfg(feature = "serde")]
#[test]
fn the_manifest_reads_the_json_the_build_writes() {
	let json = br#"{"families":[{"name":"hand","family":"Skribble Noto Sketch","faces":[
		{"file":"fonts/Sketch-Regular.ttf","weight":[400,400]},
		{"file":"fonts/Sketch-Bold.ttf","weight":[700,700],"italic":false},
		{"file":"fonts/Sketch-Italic.ttf","weight":[400,400],"italic":true}]},
		{"name":"figures","family":"Skribble Variable","faces":[
		{"file":"fonts/Variable.ttf","weight":[300,1000]}]}]}"#;

	assert_eq!(Fonts::from_json(json).unwrap(), manifest());
	assert!(Fonts::from_json(b"{").is_err());
	assert_eq!(
		serde_json::to_value(FontFamily::Declared(3)).unwrap(),
		serde_json::json!({ "declared": 3 })
	);
	assert_eq!(
		serde_json::to_value(FontFamily::System).unwrap(),
		serde_json::json!("system")
	);
}
