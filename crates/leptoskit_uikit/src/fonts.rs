//! The app's declared fonts, loaded with Core Text from the files the host
//! [installed](leptoskit_native::fonts::install).

use std::cell::RefCell;

use leptoskit_native::fonts::Face;
use objc2::Message;
use objc2::rc::Retained;
use objc2_core_foundation::CFArray;
use objc2_core_foundation::CFNumber;
use objc2_core_foundation::CFRetained;
use objc2_core_foundation::CFURL;
use objc2_core_text::CTFont;
use objc2_core_text::CTFontDescriptor;
use objc2_core_text::CTFontManagerCreateFontDescriptorsFromURL;
use objc2_ui_kit::UIFont;

/// The OpenType tag of the weight axis, `wght`, as Core Text names axes.
const WEIGHT_AXIS: i32 = i32::from_be_bytes(*b"wght");

/// The declared face at `size` points, or `None` when its file cannot be
/// read: the build checks every declared file, so that means the bundle was
/// changed after it was built.
pub(crate) fn declared(face: Face, size: f32) -> Option<Retained<UIFont>> {
	let descriptor = descriptor(face)?;
	// SAFETY: a descriptor Core Text made from a font file, and a null
	// matrix, which Core Text documents as the identity.
	let font =
		unsafe { CTFont::with_font_descriptor(&descriptor, f64::from(size), std::ptr::null()) };
	let font: &UIFont = (*font).as_ref();

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
	use super::WEIGHT_AXIS;

	#[test]
	fn the_weight_axis_is_tagged_wght() {
		assert_eq!(WEIGHT_AXIS, 0x7767_6874);
	}
}
