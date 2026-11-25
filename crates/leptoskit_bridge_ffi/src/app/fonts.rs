//! The app's declared fonts, installed by the host before its first layout.
//!
//! The build writes `fonts.json` beside the app bundle: the families
//! `leptoskit.toml` declares under `[theme.fonts]`, with each face's file
//! inside the bundled `assets` folder. The host registers those files with
//! its platform, then hands the manifest and the folder to
//! [`lk_fonts_install`], so `font-[var(--name)]` resolves to the declared
//! family and every measurement carries the face it draws. Renderers that
//! draw a weight or style of their own, such as a styled run, ask
//! [`lk_fonts_select`] which face draws it, so they choose exactly as layout
//! did.

use leptoskit_bridge::Error;
use leptoskit_log::target;
use leptoskit_native::fonts;
use leptoskit_native::fonts::Fonts;

/// Installs the app's declared fonts for this process: `len` bytes of the
/// build's `fonts.json`, and `root_len` UTF-8 bytes naming the bundled
/// `assets` folder the manifest's files are relative to.
///
/// Returns 0, or a negative [`Error`]: `InvalidArgument` for a manifest
/// that does not parse, a root that is not UTF-8, or fonts that differ from
/// the ones this process installed already. Installing the same fonts again
/// succeeds and changes nothing.
///
/// # Safety
/// `manifest` points at `len` readable bytes and `root` at `root_len`, for
/// this call only; either may be null when its length is zero.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_fonts_install(
	manifest: *const u8,
	len: usize,
	root: *const u8,
	root_len: usize,
) -> i32 {
	leptoskit_log::install();
	// SAFETY: The caller lends both byte ranges for this call.
	let (manifest, root) = unsafe { (bytes(manifest, len), bytes(root, root_len)) };

	match install(manifest, root) {
		Ok(()) => 0,
		Err(reason) => {
			log::error!(target: target::BRIDGE, "the app's fonts were not installed: {reason}");
			-(Error::InvalidArgument as i32)
		}
	}
}

fn install(manifest: Option<&[u8]>, root: Option<&[u8]>) -> Result<(), String> {
	let manifest = manifest.ok_or("the manifest is missing")?;
	let root = std::str::from_utf8(root.ok_or("the assets folder is missing")?)
		.map_err(|_| "the assets folder is not UTF-8")?;
	let declared = Fonts::from_json(manifest)
		.map_err(|error| format!("fonts.json is not a manifest: {error}"))?;

	fonts::install(declared, root)
		.map(|_| ())
		.map_err(|error| error.to_string())
}

/// Chooses the face of declared `family` that draws `weight` (1 to 1000)
/// and `italic`, as layout does: the face's position in the manifest, and the
/// weight to draw it at.
///
/// Returns 1 and writes `out[0]` and `out[1]`, or 0 when no fonts are
/// installed or `family` names none of them: the text then draws in the
/// system font.
///
/// # Safety
/// `out` points at two writable `u32`s.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_fonts_select(
	family: u32,
	weight: u32,
	italic: bool,
	out: *mut u32,
) -> i32 {
	let Some(face) = select(family, weight, italic) else {
		return 0;
	};

	if out.is_null() {
		return -(Error::InvalidArgument as i32);
	}

	// SAFETY: The caller lends two writable `u32`s.
	unsafe {
		out.write(u32::from(face.index));
		out.add(1).write(u32::from(face.weight));
	}
	1
}

fn select(family: u32, weight: u32, italic: bool) -> Option<fonts::Face> {
	let family = u16::try_from(family).ok()?;
	let weight = u16::try_from(weight.clamp(1, 1000)).ok()?;

	fonts::installed()?.select(family, weight, italic)
}

/// The bytes at `data`, or `None` for a null pointer with a length.
///
/// # Safety
/// A nonnull `data` points at `len` readable bytes for the returned
/// lifetime.
unsafe fn bytes<'a>(data: *const u8, len: usize) -> Option<&'a [u8]> {
	if len == 0 {
		return Some(&[]);
	}

	// SAFETY: See above.
	(!data.is_null()).then(|| unsafe { std::slice::from_raw_parts(data, len) })
}

#[cfg(test)]
mod tests {
	use super::lk_fonts_install;
	use super::lk_fonts_select;

	/// What the build writes for a static family with an italic, and a
	/// variable one. The process installs it once, for every test here.
	const MANIFEST: &str = r#"{"families":[
		{"name":"hand","family":"Hand","faces":[
			{"file":"fonts/Hand.ttf","weight":[400,400],"italic":false},
			{"file":"fonts/Hand-Italic.ttf","weight":[400,400],"italic":true}]},
		{"name":"gentle","family":"Gentle","faces":[
			{"file":"fonts/Gentle.ttf","weight":[300,900],"italic":false}]}]}"#;
	const ROOT: &str = "bundle/assets";
	const INVALID_ARGUMENT: i32 = -3;

	fn install(manifest: &str, root: &str) -> i32 {
		// SAFETY: Both byte ranges outlive the call.
		unsafe { lk_fonts_install(manifest.as_ptr(), manifest.len(), root.as_ptr(), root.len()) }
	}

	fn select(family: u32, weight: u32, italic: bool) -> (i32, [u32; 2]) {
		assert_eq!(install(MANIFEST, ROOT), 0);
		let mut out = [0_u32; 2];
		// SAFETY: `out` holds the two `u32`s the call writes.
		let found = unsafe { lk_fonts_select(family, weight, italic, out.as_mut_ptr()) };

		(found, out)
	}

	#[test]
	fn hosts_choose_faces_as_layout_does() {
		assert_eq!(select(0, 700, false), (1, [0, 400]));
		assert_eq!(select(0, 400, true), (1, [1, 400]));
		assert_eq!(select(1, 650, false), (1, [2, 650]));
		// Weights outside 1 to 1000 are held to it, then to the axis.
		assert_eq!(select(1, 0, false), (1, [2, 300]));
		assert_eq!(select(1, 5000, false), (1, [2, 900]));
		// The family has no italic, so its upright face draws italic text.
		assert_eq!(select(1, 400, true), (1, [2, 400]));
		assert_eq!(select(2, 400, false).0, 0);
		assert_eq!(select(u32::MAX, 400, false).0, 0);
	}

	#[test]
	fn a_null_output_is_refused_only_when_a_face_would_be_written() {
		assert_eq!(install(MANIFEST, ROOT), 0);
		// SAFETY: No face is found, so nothing is written through the null.
		assert_eq!(
			unsafe { lk_fonts_select(9, 400, false, std::ptr::null_mut()) },
			0
		);
		// SAFETY: A face is found, and the null output is refused.
		assert_eq!(
			unsafe { lk_fonts_select(0, 400, false, std::ptr::null_mut()) },
			INVALID_ARGUMENT
		);
	}

	#[test]
	fn a_process_installs_one_set_of_fonts() {
		assert_eq!(install(MANIFEST, ROOT), 0);
		assert_eq!(install(MANIFEST, ROOT), 0);
		assert_eq!(install(r#"{"families":[]}"#, ROOT), INVALID_ARGUMENT);
		assert_eq!(install(MANIFEST, "elsewhere"), INVALID_ARGUMENT);
		assert_eq!(install("{", ROOT), INVALID_ARGUMENT);
		// SAFETY: A null pointer with a length is refused before it is read.
		assert_eq!(
			unsafe { lk_fonts_install(std::ptr::null(), 4, ROOT.as_ptr(), ROOT.len()) },
			INVALID_ARGUMENT
		);
		// SAFETY: A null root with no length is read as an empty one, which
		// differs from the folder installed.
		assert_eq!(
			unsafe { lk_fonts_install(MANIFEST.as_ptr(), MANIFEST.len(), std::ptr::null(), 0) },
			INVALID_ARGUMENT
		);
		let invalid = [0xff_u8, 0xfe];
		// SAFETY: Both byte ranges outlive the call.
		assert_eq!(
			unsafe {
				lk_fonts_install(
					MANIFEST.as_ptr(),
					MANIFEST.len(),
					invalid.as_ptr(),
					invalid.len(),
				)
			},
			INVALID_ARGUMENT
		);
	}
}
