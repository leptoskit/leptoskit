//! The editor's theme, read from the styles of the element it lives in.
//!
//! The canvas cannot inherit CSS, so the host reads what the container's
//! text would look like (font, size, line height, colours) and has canvist
//! lay out and paint with the same values. Utility classes and `var(--…)`
//! tokens on the container therefore style the editor as they would a
//! `<textarea>`.

use std::sync::Arc;

use canvist_layout::BlockPatch;
use canvist_layout::BlockRule;
use canvist_layout::Chrome;
use canvist_layout::Color;
use canvist_layout::Decorations;
use canvist_layout::EmbedRule;
use canvist_layout::EmbedStyle;
use canvist_layout::FontSpec;
use canvist_layout::Motion;
use canvist_layout::TextStyle;
use canvist_layout::Theme;
use wasm_bindgen::JsCast;
use web_sys::CanvasRenderingContext2d;
use web_sys::CssStyleDeclaration;
use web_sys::Document;
use web_sys::Element;
use web_sys::HtmlCanvasElement;
use web_sys::HtmlElement;
use web_sys::Window;

use super::dom;

/// The space above and below a paragraph, as a multiple of the font size:
/// what canvist's `StarterKit` theme uses.
const PARAGRAPH_SPACE: f32 = 0.75;

/// The line height when the container's cannot be read.
const FALLBACK_LINE_HEIGHT: f32 = 1.5;

/// The font size when the container's cannot be read.
const FALLBACK_SIZE: f32 = 16.0;

/// The page background browsers paint when nothing sets one.
const LIGHT_PAGE: Color = Color::rgb(0xff, 0xff, 0xff);
const DARK_PAGE: Color = Color::rgb(0x12, 0x12, 0x12);

/// Everything read from the container that the theme depends on.
#[derive(Clone, Debug, PartialEq)]
struct Style {
	family: String,
	size: f32,
	weight: u16,
	italic: bool,
	line_height: f32,
	letter_spacing: f32,
	color: Color,
	background: Color,
	caret: Color,
}

/// Reads the theme, and remembers the last one so an unchanged style costs
/// no relayout.
pub(crate) struct ThemeReader {
	/// A 1×1 canvas that turns any CSS colour into its sRGB bytes.
	scratch: Option<CanvasRenderingContext2d>,
	last: Option<Style>,
}

impl ThemeReader {
	pub(crate) fn new(document: &Document) -> Self {
		let scratch = document
			.create_element("canvas")
			.ok()
			.and_then(|canvas| canvas.dyn_into::<HtmlCanvasElement>().ok())
			.and_then(|canvas| {
				canvas.set_width(1);
				canvas.set_height(1);
				canvas.get_context("2d").ok().flatten()
			})
			.and_then(|context| context.dyn_into::<CanvasRenderingContext2d>().ok());

		Self {
			scratch,
			last: None,
		}
	}

	/// The theme `container`'s styles ask for, or `None` when they ask for
	/// the one read last.
	pub(crate) fn read(
		&mut self,
		window: &Window,
		document: &Document,
		container: &HtmlElement,
	) -> Option<Arc<Theme>> {
		let style = self.style(window, document, container);

		if self.last.as_ref() == Some(&style) {
			return None;
		}

		let theme = theme(&style);
		self.last = Some(style);
		Some(Arc::new(theme))
	}

	fn style(&self, window: &Window, document: &Document, container: &HtmlElement) -> Style {
		let Some(computed) = dom::computed(window, container) else {
			return Style {
				family: "sans-serif".into(),
				size: FALLBACK_SIZE,
				weight: 400,
				italic: false,
				line_height: FALLBACK_LINE_HEIGHT,
				letter_spacing: 0.0,
				color: Color::rgb(0, 0, 0),
				background: LIGHT_PAGE,
				caret: Color::rgb(0, 0, 0),
			};
		};
		let family = Some(dom::property(&computed, "font-family"))
			.filter(|family| !family.trim().is_empty())
			.unwrap_or_else(|| "sans-serif".into());
		let size = Some(dom::pixels(&computed, "font-size") as f32)
			.filter(|size| *size > 0.0)
			.unwrap_or(FALLBACK_SIZE);
		let weight = dom::property(&computed, "font-weight")
			.trim()
			.parse::<f32>()
			.map_or(400, |weight| weight.clamp(1.0, 1000.0).round() as u16);
		let italic = matches!(
			dom::property(&computed, "font-style").split(' ').next(),
			Some("italic" | "oblique")
		);
		let font = FontSpec {
			family: Arc::from(family.as_str()),
			size,
			weight,
			italic,
		};
		let color = self
			.color(&dom::property(&computed, "color"))
			.unwrap_or(Color::rgb(0, 0, 0));

		Style {
			line_height: line_height(document, &computed, &font),
			letter_spacing: dom::pixels(&computed, "letter-spacing") as f32,
			color,
			background: self.background(window, document, container),
			caret: self
				.color(&dom::property(&computed, "caret-color"))
				.unwrap_or(color),
			family,
			size,
			weight,
			italic,
		}
	}

	/// The opaque colour behind `container`'s text: its own background over
	/// each ancestor's, down to the page's.
	///
	/// The canvas is opaque, so the browser can antialias its text as it
	/// does the text around it; it therefore needs a colour to fill. A
	/// background image or gradient behind the editor cannot show through.
	fn background(&self, window: &Window, document: &Document, container: &HtmlElement) -> Color {
		let mut seen = Color::TRANSPARENT;
		let mut element: Option<Element> = Some(container.clone().into());

		while let Some(current) = element {
			let behind = dom::computed(window, &current)
				.and_then(|style| self.color(&dom::property(&style, "background-color")))
				.unwrap_or(Color::TRANSPARENT);
			seen = over(seen, behind);

			if seen.a == u8::MAX {
				return seen;
			}

			element = current.parent_element();
		}

		over(seen, page(window, document))
	}

	/// A computed CSS colour as sRGB bytes.
	fn color(&self, css: &str) -> Option<Color> {
		let css = css.trim();

		if css.is_empty() || css == "auto" {
			return None;
		}

		parse_rgb(css).or_else(|| self.rasterized(css))
	}

	/// Any colour the browser can paint, read back from a pixel it painted:
	/// computed styles keep `oklch()`, `color()` and the like as written.
	fn rasterized(&self, css: &str) -> Option<Color> {
		let context = self.scratch.as_ref()?;
		context.clear_rect(0.0, 0.0, 1.0, 1.0);
		// An unreadable colour leaves this one in place.
		context.set_fill_style_str("rgba(0, 0, 0, 0)");
		context.set_fill_style_str(css);
		context.fill_rect(0.0, 0.0, 1.0, 1.0);
		let pixel = context.get_image_data(0.0, 0.0, 1.0, 1.0).ok()?.data();

		match pixel.as_slice() {
			[r, g, b, a] => Some(Color::rgba(*r, *g, *b, *a)),
			_ => None,
		}
	}
}

/// The canvist theme for `style`: paragraphs and line breaks, which is all a
/// plain-paragraph editor holds.
fn theme(style: &Style) -> Theme {
	let base = Theme::builder(TextStyle {
		font: FontSpec {
			family: Arc::from(style.family.as_str()),
			size: style.size,
			weight: style.weight,
			italic: style.italic,
		},
		line_height: style.line_height,
		color: style.color,
		background: None,
		decorations: Decorations::default(),
		letter_spacing: style.letter_spacing,
		baseline_shift: 0.0,
		motion: Motion::None,
	})
	.build();
	let chrome = Chrome {
		background: style.background,
		caret: style.caret,
		..*base.chrome()
	};
	let space = PARAGRAPH_SPACE * style.size;

	base.to_builder()
		.chrome(chrome)
		.block(BlockRule::new(
			"paragraph",
			BlockPatch::new().space(space, space).tag("p"),
		))
		.embed(EmbedRule::new("hardBreak", EmbedStyle::LineBreak))
		.build()
}

/// The container's line height as a multiple of its font size. CSS computes
/// a length, or `normal`, which depends on the font and has to be measured.
fn line_height(document: &Document, computed: &CssStyleDeclaration, font: &FontSpec) -> f32 {
	let length = dom::pixels(computed, "line-height") as f32;

	if length > 0.0 {
		return length / font.size;
	}

	let style = format!(
		"position:absolute;left:0;top:0;visibility:hidden;white-space:pre;font:{};line-height:\
		 normal",
		font.css()
	);

	dom::probe(document, "div", &style)
		.and_then(|probe| {
			probe.set_text_content(Some("x"));
			dom::laid_out(document, &probe, || {
				probe.get_bounding_client_rect().height()
			})
		})
		.map(|height| height as f32 / font.size)
		.filter(|ratio| *ratio > 0.0)
		.unwrap_or(FALLBACK_LINE_HEIGHT)
}

/// The colour the browser paints behind the page when no element sets one:
/// dark only when the page opted into a dark scheme and is using it.
fn page(window: &Window, document: &Document) -> Color {
	let scheme = document
		.document_element()
		.and_then(|root| dom::computed(window, &root))
		.map(|style| dom::property(&style, "color-scheme"))
		.unwrap_or_default();
	let allows = |name: &str| scheme.split_whitespace().any(|word| word == name);
	let prefers_dark = window
		.match_media("(prefers-color-scheme: dark)")
		.ok()
		.flatten()
		.is_some_and(|query| query.matches());

	if allows("dark") && (prefers_dark || !allows("light")) {
		DARK_PAGE
	} else {
		LIGHT_PAGE
	}
}

/// `top` painted over `bottom`, both with straight alpha.
fn over(top: Color, bottom: Color) -> Color {
	let (top_alpha, bottom_alpha) = (f32::from(top.a) / 255.0, f32::from(bottom.a) / 255.0);
	let alpha = top_alpha + bottom_alpha * (1.0 - top_alpha);

	if alpha <= 0.0 {
		return Color::TRANSPARENT;
	}

	let channel = |top: u8, bottom: u8| {
		let mixed =
			f32::from(top) * top_alpha + f32::from(bottom) * bottom_alpha * (1.0 - top_alpha);
		(mixed / alpha).round() as u8
	};

	Color::rgba(
		channel(top.r, bottom.r),
		channel(top.g, bottom.g),
		channel(top.b, bottom.b),
		(alpha * 255.0).round() as u8,
	)
}

/// A colour as computed styles write sRGB ones: `rgb(r, g, b)` or
/// `rgba(r, g, b, a)`.
fn parse_rgb(css: &str) -> Option<Color> {
	let numbers = css
		.strip_prefix("rgba(")
		.or_else(|| css.strip_prefix("rgb("))?
		.strip_suffix(')')?;
	let mut parts = numbers.split(',').map(|part| part.trim().parse::<f32>());
	let mut channel = || Some(parts.next()?.ok()?.round().clamp(0.0, 255.0) as u8);
	let (r, g, b) = (channel()?, channel()?, channel()?);
	let alpha = match parts.next() {
		Some(alpha) => (alpha.ok()?.clamp(0.0, 1.0) * 255.0).round() as u8,
		None => u8::MAX,
	};

	parts
		.next()
		.is_none()
		.then_some(Color::rgba(r, g, b, alpha))
}
