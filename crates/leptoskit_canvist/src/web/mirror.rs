//! The accessibility mirror: the document as semantic HTML, visually hidden
//! beside the canvas, so screen readers and find-in-page tools can read
//! what the canvas paints.
//!
//! It is refreshed when the browser is idle after a change rather than on
//! every keystroke. Ported from canvist's
//! `packages/canvist-web/src/mirror.ts`, building elements instead of
//! parsing HTML, so no document text is ever read as markup.

use canvist_headless::Doc;
use canvist_headless::canvist_model::EMBED_CHAR;
use web_sys::Document;
use web_sys::HtmlElement;

use super::dom;

/// Visually hidden, but still read aloud: `display: none` would hide it
/// from assistive technology too.
const HIDDEN: &str = "position:absolute;width:1px;height:1px;margin:-1px;padding:0;border:0;\
                      overflow:hidden;clip-path:inset(50%);white-space:nowrap";

/// A visually hidden copy of the document for assistive technology.
pub(crate) struct Mirror {
	element: HtmlElement,
}

impl Mirror {
	/// An empty mirror labelled `label`, not yet in the page.
	pub(crate) fn new(document: &Document, label: &str) -> Option<Self> {
		let element = dom::element(document, "div", HIDDEN)?;
		let _ = element.set_attribute("data-canvist", "mirror");
		let _ = element.set_attribute("role", "document");
		let _ = element.set_attribute("aria-label", label);

		Some(Self { element })
	}

	pub(crate) fn element(&self) -> &HtmlElement {
		&self.element
	}

	/// Show `doc`: one `<p>` per paragraph, with a `<br>` at each line
	/// break, and an image named "Drawing" for each drawing.
	pub(crate) fn refresh(&self, document: &Document, doc: &Doc) {
		self.element.set_text_content(None);
		let model = doc.document();
		let drawing = model.schema().block_type("ink");

		for block in model.children(None) {
			let is_drawing = drawing.is_some()
				&& model
					.block(*block)
					.map(canvist_headless::canvist_model::Block::kind)
					== drawing;

			if is_drawing && let Ok(image) = document.create_element("div") {
				let _ = image.set_attribute("role", "img");
				let _ = image.set_attribute("aria-label", "Drawing");
				let _ = self.element.append_child(&image);
				continue;
			}

			let (Some(text), Ok(paragraph)) = (model.inline(*block), document.create_element("p"))
			else {
				continue;
			};

			for (index, line) in text.as_str().split(EMBED_CHAR).enumerate() {
				if index > 0
					&& let Ok(line_break) = document.create_element("br")
				{
					let _ = paragraph.append_child(&line_break);
				}

				let _ = paragraph.append_child(&document.create_text_node(line));
			}

			let _ = self.element.append_child(&paragraph);
		}
	}
}
