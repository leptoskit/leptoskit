//! What differs between browsers and screens, and has to be detected.
//!
//! Ported from canvist's `packages/canvist-web/src/editor.ts` and
//! `platform.ts`.

use canvist_headless::canvist_core::Platform;
use wasm_bindgen::JsValue;
use web_sys::Document;
use web_sys::Window;

use super::dom;
use super::paint::PixelGrid;

/// How close, in CSS px, a measured border width must be to a snapped one to
/// count as snapped.
const BORDER_SNAP_TOLERANCE: f64 = 1.0 / 128.0;

/// Whether `Mod` in key bindings is Cmd or Ctrl in this browser.
pub(crate) fn platform() -> Platform {
	if web_sys::window().is_some_and(|window| is_apple(&window)) {
		Platform::Apple
	} else {
		Platform::Other
	}
}

/// Whether the browser runs on an Apple platform, where `Mod` is Cmd and
/// Option copies a drag.
pub(crate) fn is_apple(window: &Window) -> bool {
	let navigator = window.navigator();
	// `userAgentData` is the newer source, and only Chromium has it.
	let hinted = js_sys::Reflect::get(&navigator, &"userAgentData".into())
		.ok()
		.filter(JsValue::is_object)
		.and_then(|data| js_sys::Reflect::get(&data, &"platform".into()).ok())
		.and_then(|platform| platform.as_string());
	let platform = hinted
		.or_else(|| navigator.platform().ok())
		.unwrap_or_default();

	let platform = platform.to_ascii_lowercase();

	["mac", "iphone", "ipad", "ipod"]
		.iter()
		.any(|name| platform.contains(name))
}

/// Whether lines break by Blink's tailoring of the Unicode rules, not ICU's
/// defaults, which Firefox and WebKit follow. CSS leaves the details to each
/// engine and there is no feature test for them, so this reads the user
/// agent.
pub(crate) fn chromium(window: &Window) -> bool {
	let agent = window.navigator().user_agent().unwrap_or_default();

	["Chrome/", "Chromium/", "Edg/"]
		.iter()
		.any(|name| agent.contains(name))
}

/// The grid this browser snaps DOM text baselines and backgrounds to.
///
/// Firefox and WebKit snap to device pixels. Chromium does too when it lays
/// out in device units, as it does on real high-density screens; when it
/// lays out in CSS units (as under device emulation), it snaps to whole CSS
/// pixels and scales. Layout in device units can represent a box 1/64 of a
/// device pixel wide, which CSS-unit layout rounds to zero.
pub(crate) fn pixel_grid(window: &Window, document: &Document) -> PixelGrid {
	let scale = window.device_pixel_ratio();

	#[allow(clippy::float_cmp, reason = "a ratio of exactly one needs no probe")]
	if scale == 1.0 || !chromium(window) {
		return PixelGrid::Device;
	}

	let width = 1.0 / (64.0 * scale);
	let style =
		format!("position:absolute;left:0;top:0;height:1px;width:{width}px;visibility:hidden");
	let measured = dom::probe(document, "div", &style).and_then(|probe| {
		dom::laid_out(document, &probe, || {
			probe.get_bounding_client_rect().width()
		})
	});

	if measured.is_some_and(|width| width <= 0.0) {
		PixelGrid::Css
	} else {
		PixelGrid::Device
	}
}

/// How many units per CSS pixel this browser snaps border widths down to:
/// the device pixel ratio, 1 when it lays out in CSS pixels, or `None` when
/// it keeps exact widths. A 1.7 px border tells them apart.
pub(crate) fn border_scale(window: &Window, document: &Document) -> Option<f32> {
	const WIDTH: f64 = 1.7;

	let outer = dom::probe(
		document,
		"div",
		&format!(
			"position:absolute;left:0;top:0;visibility:hidden;border-left:{WIDTH}px \
			 solid;width:10px;height:1px"
		),
	)?;
	let inner = dom::probe(document, "div", "width:1px;height:1px")?;
	outer.append_child(&inner).ok()?;
	let used = dom::laid_out(document, &outer, || {
		inner.get_bounding_client_rect().left() - outer.get_bounding_client_rect().left()
	})?;

	[window.device_pixel_ratio(), 1.0]
		.into_iter()
		.find(|scale| {
			let snapped = (WIDTH * scale).floor().max(1.0) / scale;
			(used - snapped).abs() < BORDER_SNAP_TOLERANCE
		})
		.map(|scale| scale as f32)
}

/// How many device pixels a box spanning `size` CSS px from `start` covers.
/// Browsers snap a box's edges to device pixels, not its size, so at
/// fractional ratios this can differ from `round(size * scale)` by one.
/// Computed here rather than observed with `device-pixel-content-box`,
/// which Firefox and Safari don't support, so the size is exact in every
/// engine and only the element needs observing.
pub(crate) fn device_pixels(start: f64, size: f64, scale: f64) -> u32 {
	(((start + size) * scale).round() - (start * scale).round()).max(0.0) as u32
}
