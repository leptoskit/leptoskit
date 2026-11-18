//! Small DOM helpers the host's pieces share.

use wasm_bindgen::JsCast;
use web_sys::CssStyleDeclaration;
use web_sys::Document;
use web_sys::Element;
use web_sys::HtmlElement;
use web_sys::Window;

/// A new `tag` element styled with `css`, not yet in the page.
///
/// The style goes through the CSS object model, not a `style` attribute, so
/// a page whose content security policy forbids inline styles still gets
/// it.
pub(crate) fn element(document: &Document, tag: &str, css: &str) -> Option<HtmlElement> {
	let element = document
		.create_element(tag)
		.ok()?
		.dyn_into::<HtmlElement>()
		.ok()?;
	element.style().set_css_text(css);
	Some(element)
}

/// A probe: an element only laid out to be measured, which assistive
/// technology should not hear of.
pub(crate) fn probe(document: &Document, tag: &str, css: &str) -> Option<HtmlElement> {
	let probe = element(document, tag, css)?;
	let _ = probe.set_attribute("aria-hidden", "true");
	Some(probe)
}

/// Put `probe` in the page for as long as `read` runs. `None` before the
/// page has a body to lay it out in.
pub(crate) fn laid_out<T>(
	document: &Document,
	probe: &HtmlElement,
	read: impl FnOnce() -> T,
) -> Option<T> {
	document.body()?.append_child(probe).ok()?;
	let value = read();
	probe.remove();
	Some(value)
}

/// The styles the browser resolved for `element`.
pub(crate) fn computed(window: &Window, element: &Element) -> Option<CssStyleDeclaration> {
	window.get_computed_style(element).ok().flatten()
}

/// A computed property of `style`, or an empty string.
pub(crate) fn property(style: &CssStyleDeclaration, name: &str) -> String {
	style.get_property_value(name).unwrap_or_default()
}

/// A computed length in CSS px, or 0 for anything that is not one.
pub(crate) fn pixels(style: &CssStyleDeclaration, name: &str) -> f64 {
	property(style, name)
		.trim()
		.strip_suffix("px")
		.and_then(|number| number.parse().ok())
		.unwrap_or(0.0)
}
