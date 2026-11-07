//! Browser information from User-Agent Client Hints.
//!
//! [`snapshot`] reads only the low-entropy hints every page already gets
//! (`brands`, `mobile`, `platform`), the logical processor count and the
//! maximum touch points. [`high_entropy_values`] asks for the detailed hints
//! — model, architecture, full versions — and runs only when you call it.
//! The user-agent string is never parsed, and the coarse, privacy-reduced
//! `navigator.deviceMemory` is not presented as total RAM.

use js_sys::Array;
use js_sys::Reflect;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;

use super::*;

/// The high-entropy hints [`high_entropy_values`] requests.
pub const HIGH_ENTROPY_HINTS: [&str; 7] = [
	"architecture",
	"bitness",
	"formFactors",
	"fullVersionList",
	"model",
	"platformVersion",
	"wow64",
];

/// Detailed client hints. The browser may withhold or reduce any of them;
/// withheld values are `None` or empty.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct HighEntropyValues {
	/// CPU architecture, such as `arm` or `x86`.
	pub architecture: Option<String>,
	/// CPU bitness, such as `64`.
	pub bitness: Option<String>,
	/// Device model; usually only reported on Android.
	pub model: Option<String>,
	/// OS version, such as `15.1.0`.
	pub platform_version: Option<String>,
	/// A 32-bit browser on 64-bit Windows.
	pub wow64: Option<bool>,
	/// Form factors such as `Desktop` or `Mobile`.
	pub form_factors: Vec<String>,
	/// Brands with their full versions.
	pub full_version_list: Vec<Brand>,
}

fn hints() -> Result<Option<JsValue>, Error> {
	let navigator = web_sys::window().ok_or(Error::Unavailable)?.navigator();
	let hints = Reflect::get(navigator.as_ref(), &JsValue::from_str("userAgentData"))
		.map_err(|_| Error::Native)?;
	Ok(hints.is_object().then_some(hints))
}

fn property(object: &JsValue, name: &str) -> JsValue {
	Reflect::get(object, &JsValue::from_str(name)).unwrap_or(JsValue::UNDEFINED)
}

/// A non-empty string within `max` bytes.
fn text(value: &JsValue, max: usize) -> Option<String> {
	value
		.as_string()
		.filter(|value| !value.is_empty() && value.len() <= max)
}

/// A whole number in `u32` range.
fn whole(value: &JsValue) -> Option<u32> {
	value
		.as_f64()
		.filter(|value| value.fract() == 0.0 && (0.0..=f64::from(u32::MAX)).contains(value))
		.map(|value| value as u32)
}

/// Named brands within the wire bounds; others are skipped.
fn brands(value: &JsValue) -> Vec<Brand> {
	let Some(array) = value.dyn_ref::<Array>() else {
		return Vec::new();
	};

	array
		.iter()
		.filter_map(|entry| {
			Some(Brand {
				brand: text(&property(&entry, "brand"), MAX_SHORT_ITEM_BYTES)?,
				version: text(&property(&entry, "version"), MAX_SHORT_ITEM_BYTES)
					.unwrap_or_default(),
			})
		})
		.take(MAX_SHORT_LIST)
		.collect()
}

/// Reads the low-entropy hints, logical processor count and touch points.
/// Without client hints (Firefox, Safari) only the processor count and touch
/// points are known.
pub fn snapshot() -> Result<Snapshot, Error> {
	let navigator = web_sys::window().ok_or(Error::Unavailable)?.navigator();
	let hints = hints()?;
	let (os_name, mobile, brands) = match &hints {
		Some(hints) => {
			(
				text(&property(hints, "platform"), MAX_TEXT_BYTES),
				property(hints, "mobile").as_bool(),
				brands(&property(hints, "brands")),
			)
		}
		None => (None, None, Vec::new()),
	};

	Ok(Snapshot {
		os_name,
		cpu_count: whole(&property(navigator.as_ref(), "hardwareConcurrency"))
			.and_then(NonZeroU32::new),
		platform: Platform::Web(Web {
			mobile,
			max_touch_points: whole(&property(navigator.as_ref(), "maxTouchPoints")),
			brands,
		}),
		..Snapshot::default()
	})
}

/// Requests the high-entropy client hints in [`HIGH_ENTROPY_HINTS`].
///
/// `Unavailable` without client hints; `PermissionDenied` when the browser
/// refuses (for example through a `Permissions-Policy`). Dropping the future
/// stops waiting; the browser's own request still finishes.
pub async fn high_entropy_values() -> Result<HighEntropyValues, Error> {
	let hints = hints()?.ok_or(Error::Unavailable)?;
	let method = property(&hints, "getHighEntropyValues")
		.dyn_into::<js_sys::Function>()
		.map_err(|_| Error::Unavailable)?;
	let names = HIGH_ENTROPY_HINTS
		.iter()
		.map(|name| JsValue::from_str(name))
		.collect::<Array>();
	let promise = method
		.call1(&hints, &names)
		.map_err(|_| Error::Native)?
		.dyn_into::<js_sys::Promise>()
		.map_err(|_| Error::Native)?;
	let values = wasm_bindgen_futures::JsFuture::from(promise)
		.await
		.map_err(|error| {
			if property(&error, "name").as_string().as_deref() == Some("NotAllowedError") {
				Error::PermissionDenied
			} else {
				Error::Native
			}
		})?;

	if !values.is_object() {
		return Err(Error::Native);
	}

	let form_factors = property(&values, "formFactors")
		.dyn_ref::<Array>()
		.map(|array| {
			array
				.iter()
				.filter_map(|value| text(&value, MAX_SHORT_ITEM_BYTES))
				.take(MAX_SHORT_LIST)
				.collect()
		})
		.unwrap_or_default();

	Ok(HighEntropyValues {
		architecture: text(&property(&values, "architecture"), MAX_TEXT_BYTES),
		bitness: text(&property(&values, "bitness"), MAX_TEXT_BYTES),
		model: text(&property(&values, "model"), MAX_TEXT_BYTES),
		platform_version: text(&property(&values, "platformVersion"), MAX_TEXT_BYTES),
		wow64: property(&values, "wow64").as_bool(),
		form_factors,
		full_version_list: brands(&property(&values, "fullVersionList")),
	})
}
