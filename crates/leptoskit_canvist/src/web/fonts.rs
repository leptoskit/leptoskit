//! Keeping measurements in step with the page's web fonts.
//!
//! The canvas resolves font families through `document.fonts`, so every face
//! the page declares with `@font-face` is usable with no configuration. But
//! a face that is still loading measures and paints as its fallback. The
//! watcher asks the browser to load the faces layout actually used, for the
//! characters it used them for (so `unicode-range` subsets load only when
//! needed), and reports each family as its faces finish loading, so layout
//! forgets just those measurements.
//!
//! Ported from canvist's `packages/canvist-web/src/font-watcher.ts`.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashMap;

use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::JsFuture;
use web_sys::FontFace;
use web_sys::FontFaceSet;

/// Watches one page's fonts on behalf of one editor.
pub(crate) struct FontWatcher {
	fonts: FontFaceSet,
	/// Characters already checked or loaded, per CSS font.
	seen: RefCell<HashMap<String, BTreeSet<char>>>,
}

impl FontWatcher {
	pub(crate) fn new(fonts: FontFaceSet) -> Self {
		Self {
			fonts,
			seen: RefCell::new(HashMap::new()),
		}
	}

	/// The page's fonts, whose `loadingdone` event names faces as they
	/// arrive.
	pub(crate) fn set(&self) -> &FontFaceSet {
		&self.fonts
	}

	/// Load whatever faces the fonts in `used` need for their text and do
	/// not have yet, calling `loaded` with their families once they arrive.
	pub(crate) fn request(
		&self,
		used: &BTreeMap<String, String>,
		loaded: &(impl Fn(Vec<String>) + Clone + 'static),
	) {
		for (font, text) in used {
			let fresh = self.unseen(font, text);

			if fresh.is_empty() || self.fonts.check_with_text(font, &fresh).unwrap_or(true) {
				continue;
			}

			let pending = self.fonts.load_with_text(font, &fresh);
			let loaded = loaded.clone();
			wasm_bindgen_futures::spawn_local(async move {
				// A face that fails to load keeps its fallback, as CSS does.
				let Ok(faces) = JsFuture::from(pending).await else {
					return;
				};
				let families = families(&faces);

				if !families.is_empty() {
					loaded(families);
				}
			});
		}
	}

	/// The characters of `text` not yet checked for `font`, marking them
	/// seen. Empty text still checks one character, so the font's primary
	/// face loads.
	fn unseen(&self, font: &str, text: &str) -> String {
		let mut seen = self.seen.borrow_mut();
		let seen = seen.entry(font.to_owned()).or_default();
		let text = if text.is_empty() { "x" } else { text };

		text.chars()
			.filter(|character| seen.insert(*character))
			.collect()
	}
}

/// The family names of an array of `FontFace`s, without the quotes
/// `FontFace.family` may keep.
pub(crate) fn families(faces: &JsValue) -> Vec<String> {
	let Some(faces) = faces.dyn_ref::<js_sys::Array>() else {
		return Vec::new();
	};

	faces
		.iter()
		.filter_map(|face| face.dyn_into::<FontFace>().ok())
		.map(|face| face.family().trim_matches(['"', '\'']).to_owned())
		.collect()
}
