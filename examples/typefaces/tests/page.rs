//! The page lays every line out in the face its classes choose, as a native
//! host does once it has installed the build's `fonts.json`.

use std::sync::Arc;
use std::sync::Mutex;

use leptoskit_native::HeadlessBackend;
use leptoskit_native::NativeSurface;
use leptoskit_native::TextMeasurer;
use leptoskit_native::TextStyle;
use leptoskit_native::fonts;
use leptoskit_native::fonts::Face;
use leptoskit_native::fonts::Fonts;
use leptoskit_yoga::AvailableSpace;
use leptoskit_yoga::Size;
use typefaces_example::LINE;
use typefaces_example::Page;

/// What `leptoskit build` writes for this example's `[theme.fonts]`: the
/// families in key order, so `gentle` is family 0 and `hand` family 1.
const MANIFEST: &str = r#"{"families": [
	{"name": "gentle", "family": "Skribble Gentle", "faces": [
		{"file": "fonts/SkribbleVariableGentle-Latin.ttf", "weight": [300, 900], "italic": false}]},
	{"name": "hand", "family": "Skribble Noto Sketch", "faces": [
		{"file": "fonts/SkribbleNotoSketch-Latin.ttf", "weight": [400, 400], "italic": false}]}
]}"#;

/// Records the style each text is measured in.
#[derive(Default)]
struct Recorder(Mutex<Vec<(String, TextStyle)>>);

impl TextMeasurer for Recorder {
	fn measure(&self, text: &str, style: TextStyle, _: Size<AvailableSpace>) -> Size<f32> {
		self.0.lock().unwrap().push((text.to_owned(), style));
		Size::new(10.0 * text.chars().count() as f32, style.font_size)
	}
}

#[test]
fn every_line_is_measured_in_the_face_its_classes_choose() {
	let manifest = Fonts::from_json(MANIFEST.as_bytes()).expect("the manifest");
	fonts::install(manifest, concat!(env!("CARGO_MANIFEST_DIR"), "/assets")).expect("the fonts");
	let recorder = Arc::new(Recorder::default());
	let measurer: Arc<dyn TextMeasurer> = recorder.clone();
	let mut surface = NativeSurface::new(HeadlessBackend::new().with_text_measurer(measurer));
	surface.set_viewport(390.0, 844.0);
	surface.set_root(&leptoskit::view! { <Page /> });

	let styles = recorder.0.lock().unwrap().clone();
	let faces = |text: &str| {
		let mut faces = styles
			.iter()
			.filter(|(line, _)| line == text)
			.map(|(_, style)| style.face)
			.collect::<Vec<_>>();
		faces.dedup();
		faces
	};
	let hand = Some(Face {
		index: 1,
		weight: 400,
	});
	let gentle = |weight| Some(Face { index: 0, weight });

	assert_eq!(faces("Typefaces"), [hand]);
	assert_eq!(faces("A button takes the family too"), [hand]);
	assert_eq!(faces("--HAND · SKRIBBLE NOTO SKETCH"), [None]);

	let mut line = faces(LINE);
	line.sort_by_key(|face| face.map(|face| (face.index, face.weight)));
	line.dedup();
	assert_eq!(line, [None, gentle(300), gentle(400), gentle(900), hand]);
}
