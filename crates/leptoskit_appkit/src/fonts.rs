//! The app's declared fonts, loaded with Core Text from the files the host
//! [installed](leptoskit_native::fonts::install).

use std::cell::RefCell;

use leptoskit_native::fonts::Face;
use objc2::Message;
use objc2::rc::Retained;
use objc2_app_kit::NSFont;
use objc2_core_foundation::CFArray;
use objc2_core_foundation::CFNumber;
use objc2_core_foundation::CFRetained;
use objc2_core_foundation::CFURL;
use objc2_core_text::CTFont;
use objc2_core_text::CTFontDescriptor;
use objc2_core_text::CTFontManagerCreateFontDescriptorsFromURL;

/// The OpenType tag of the weight axis, `wght`, as Core Text names axes.
const WEIGHT_AXIS: i32 = i32::from_be_bytes(*b"wght");

/// The declared face at `size` points, or `None` when its file cannot be
/// read: the build checks every declared file, so that means the bundle was
/// changed after it was built.
pub(crate) fn declared(face: Face, size: f32) -> Option<Retained<NSFont>> {
	let descriptor = descriptor(face)?;
	// SAFETY: a descriptor Core Text made from a font file, and a null
	// matrix, which Core Text documents as the identity.
	let font =
		unsafe { CTFont::with_font_descriptor(&descriptor, f64::from(size), std::ptr::null()) };
	let font: &NSFont = (*font).as_ref();

	Some(font.retain())
}

/// The descriptor of `face`, read from its file once per thread and weight.
fn descriptor(face: Face) -> Option<CFRetained<CTFontDescriptor>> {
	thread_local! {
		static CACHE: RefCell<Vec<(Face, Option<CFRetained<CTFontDescriptor>>)>> =
			const { RefCell::new(Vec::new()) };
	}

	if let Some(found) = CACHE.with_borrow(|cache| {
		cache
			.iter()
			.find(|(cached, _)| *cached == face)
			.map(|(_, descriptor)| descriptor.clone())
	}) {
		return found;
	}

	let descriptor = load(face);
	CACHE.with_borrow_mut(|cache| cache.push((face, descriptor.clone())));
	descriptor
}

fn load(face: Face) -> Option<CFRetained<CTFontDescriptor>> {
	let declared = leptoskit_native::fonts::installed()?.face(face.index)?;
	let url = CFURL::from_file_path(leptoskit_native::fonts::path(face.index)?)?;
	// SAFETY: the URL names a local file; Core Text returns an array of the
	// descriptors of the fonts in it, or null when it holds none.
	let descriptors = unsafe { CTFontManagerCreateFontDescriptorsFromURL(&url) }?;
	// SAFETY: Core Text documents every element as a `CTFontDescriptor`.
	let descriptors =
		unsafe { CFRetained::cast_unchecked::<CFArray<CTFontDescriptor>>(descriptors) };
	let descriptor = descriptors.get(0)?;

	if !declared.is_variable() {
		return Some(descriptor);
	}

	// SAFETY: a weight axis tag and a weight within the axis the app
	// declared; Core Text clamps a value the font's axis does not reach.
	Some(unsafe {
		descriptor.copy_with_variation(&CFNumber::new_i32(WEIGHT_AXIS), f64::from(face.weight))
	})
}

#[cfg(test)]
mod tests {
	use leptoskit_native::fonts::DeclaredFamily;
	use leptoskit_native::fonts::Face;
	use leptoskit_native::fonts::FontFace;
	use leptoskit_native::fonts::Fonts;
	use objc2_core_foundation::CFDictionary;
	use objc2_core_foundation::CFNumber;
	use objc2_core_foundation::CFRetained;
	use objc2_core_text::CTFont;

	use super::WEIGHT_AXIS;
	use super::declared;

	/// The typefaces example's fonts: a static face and a variable one. This
	/// crate's only tests install them, once for the process.
	fn install() {
		let family = |name: &str, file: &str, weight: [u16; 2]| {
			DeclaredFamily {
				name: name.to_owned(),
				family: name.to_owned(),
				faces: vec![FontFace {
					file: file.to_owned(),
					weight,
					italic: false,
				}],
			}
		};
		let fonts = Fonts {
			families: vec![
				family("hand", "fonts/SkribbleNotoSketch-Latin.ttf", [400, 400]),
				family(
					"gentle",
					"fonts/SkribbleVariableGentle-Latin.ttf",
					[300, 900],
				),
				family("missing", "fonts/Missing.ttf", [400, 400]),
			],
		};
		let assets = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
			.join("../../examples/typefaces/assets");

		leptoskit_native::fonts::install(fonts, assets).expect("one set of fonts");
	}

	fn weight(font: &CTFont) -> Option<f64> {
		// SAFETY: Core Text returns the axis values keyed by `CFNumber` axis
		// identifiers, with `CFNumber` values.
		let variation = unsafe { font.variation() }?;
		let variation =
			unsafe { CFRetained::cast_unchecked::<CFDictionary<CFNumber, CFNumber>>(variation) };

		variation.get(&CFNumber::new_i32(WEIGHT_AXIS))?.as_f64()
	}

	#[test]
	fn the_weight_axis_is_tagged_wght() {
		assert_eq!(WEIGHT_AXIS, 0x7767_6874);
	}

	#[test]
	fn declared_faces_load_from_the_installed_files() {
		install();
		let hand = declared(
			Face {
				index: 0,
				weight: 400,
			},
			20.0,
		)
		.expect("the static face");
		let hand: &CTFont = (*hand).as_ref();

		// SAFETY: Every font has a PostScript name.
		assert_eq!(
			unsafe { hand.post_script_name() }.to_string(),
			"SkribbleNotoSketch-Regular"
		);
		assert!((unsafe { hand.size() } - 20.0).abs() < f64::EPSILON);
		assert_eq!(
			declared(
				Face {
					index: 2,
					weight: 400
				},
				20.0
			),
			None
		);
		assert_eq!(
			declared(
				Face {
					index: 7,
					weight: 400
				},
				20.0
			),
			None
		);
	}

	#[test]
	fn variable_faces_draw_at_the_selected_weight() {
		install();

		for drawn in [300_u16, 650, 900] {
			let font = declared(
				Face {
					index: 1,
					weight: drawn,
				},
				16.0,
			)
			.expect("the variable face");
			let font: &CTFont = (*font).as_ref();

			assert_eq!(weight(font), Some(f64::from(drawn)), "drawn at {drawn}");
		}
	}
}
