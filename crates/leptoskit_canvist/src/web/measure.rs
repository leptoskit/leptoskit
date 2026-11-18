//! Text measurement with the browser's own text engine.
//!
//! Layout asks for every segment it is missing in one batch per relayout,
//! and caches the answers by (font, segment). This measures a batch with
//! `measureText` on a canvas whose text properties match the painting
//! canvas, so a word is as wide here as `fillText` paints it and as the
//! browser lays it out in the page around the editor.
//!
//! Ported from canvist's `packages/canvist-web/src/measure.ts`, with its
//! fixes for the places canvas and DOM text disagree: WebKit resolving a
//! family list to different faces, Firefox snapping fractional font sizes,
//! ligatures, and scripts whose letters join.

use std::collections::BTreeMap;
use std::collections::HashMap;

use canvist_layout::FontId;
use canvist_layout::FontMetrics;
use canvist_layout::LineBox;
use canvist_layout::MeasureBatch;
use canvist_layout::MeasureOutput;
use canvist_layout::TextMeasurer;
use unicode_segmentation::UnicodeSegmentation;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use web_sys::CanvasRenderingContext2d;
use web_sys::Document;
use web_sys::HtmlCanvasElement;
use web_sys::HtmlElement;
use web_sys::Node;
use web_sys::Range;

use super::dom;
use super::paint::pin_text_properties;
use crate::text::Ligature;
use crate::text::at_size;
use crate::text::clusters;
use crate::text::joins;
use crate::text::scale;
use crate::text::split_families;
use crate::text::split_ligatures;
use crate::text::split_shorthand;

/// Text whose width tells which face a family list resolves to.
const RESOLUTION_SAMPLE: &str = "The quick brown fox jumps over the lazy dog 0123456789";

/// How far the DOM and canvas widths of the sample may differ and still be
/// one face.
const SAME_FACE_TOLERANCE: f64 = 0.05;

/// Letter sequences that fonts commonly draw as one ligature glyph, longest
/// first.
const LIGATURE_CANDIDATES: [&str; 8] = ["ffi", "ffl", "ff", "fi", "fl", "fj", "ft", "Th"];

/// The font size ligatures are probed at, in CSS px: large, so whole-pixel
/// DOM rectangles barely round.
const LIGATURE_PROBE_PX: &str = "200px";

/// How far, as a share of the sequence's width, the DOM's caret edge inside
/// a sequence may sit from the canvas's before the sequence counts as a
/// ligature. Ligatures move it by 5% or more; WebKit's rounding at the probe
/// size moves it under 0.5%.
const LIGATURE_TOLERANCE: f64 = 0.02;

/// A box no one sees or can click, for laying probe text out in.
const PROBE_HOST: &str = "position:absolute;left:0;top:0;width:0;height:0;overflow:hidden;\
                          visibility:hidden;pointer-events:none";

/// The text properties every probe pins, matching the painting canvas.
const PROBE_TEXT: &str = "white-space:pre;width:max-content;font-kerning:normal;font-variant:\
                          normal;letter-spacing:0;word-spacing:0";

/// How one layout font is drawn and measured.
#[derive(Clone, Debug)]
struct Resolution {
	/// The canvas `font` string that draws the DOM's face.
	font: String,
	/// Whether segment widths must come from the DOM.
	dom_widths: bool,
	/// The sequences the face draws as ligatures.
	ligatures: Vec<Ligature>,
}

/// A segment whose advances the DOM supplies, at `at` in the output.
struct DomSegment<'a> {
	font: &'a str,
	segment: &'a str,
	at: usize,
	joined: bool,
	/// The total the advances must sum to, or `None` to take the DOM's.
	width: Option<f64>,
}

/// Layout's [`TextMeasurer`], answered by the page the editor is in.
///
/// Per-character advances come from measuring prefixes that end on grapheme
/// boundaries, so they sum to exactly the width `fillText` paints for the
/// whole segment (kerning and ligatures included), and caret stops fall
/// where the browser puts them. Each grapheme's advance goes to its first
/// scalar value; the rest of the cluster advances nothing.
pub(crate) struct DomMeasurer {
	document: Document,
	context: CanvasRenderingContext2d,
	/// The CSS `font` shorthand of each layout font, by id.
	fonts: Vec<String>,
	/// How each font id is drawn and measured, once resolved.
	resolved: HashMap<usize, Resolution>,
	/// Widths of single graphemes, by canvas font and grapheme.
	singles: HashMap<String, f64>,
	/// The font the measuring canvas is set to.
	current: String,
	/// The fonts measured since [`DomMeasurer::take_used`], with the text
	/// measured in each, so the host can load web fonts for that text.
	used: BTreeMap<String, String>,
}

impl DomMeasurer {
	/// A measurer laying its probes out in `document`.
	pub(crate) fn new(document: &Document) -> Result<Self, JsValue> {
		let context = document
			.create_element("canvas")?
			.dyn_into::<HtmlCanvasElement>()?
			.get_context("2d")?
			.ok_or_else(|| JsValue::from_str("no 2D canvas context to measure text with"))?
			.dyn_into::<CanvasRenderingContext2d>()?;
		pin_text_properties(&context);
		let _ = js_sys::Reflect::set(&context, &"letterSpacing".into(), &"0px".into());

		Ok(Self {
			document: document.clone(),
			context,
			fonts: Vec::new(),
			resolved: HashMap::new(),
			singles: HashMap::new(),
			current: String::new(),
			used: BTreeMap::new(),
		})
	}

	/// Forget cached widths and resolved faces, which change when a web font
	/// finishes loading or the device pixel ratio changes.
	pub(crate) fn forget(&mut self) {
		self.singles.clear();
		self.resolved.clear();
		self.current.clear();
	}

	/// The fonts measured since the last call, as CSS `font` shorthands, with
	/// the text measured in each.
	pub(crate) fn take_used(&mut self) -> BTreeMap<String, String> {
		std::mem::take(&mut self.used)
	}

	/// The `font` string to draw font `id` with on a canvas: one the browser
	/// resolves to the face its DOM uses for `css`, which is not always `css`
	/// itself.
	pub(crate) fn paint_font(&mut self, id: FontId, css: &str) -> String {
		let index = id.index();
		self.register(index, css);
		self.resolution(index).font.clone()
	}

	/// Know font `index` as `css`, forgetting how it resolved if it was
	/// another font before.
	fn register(&mut self, index: usize, css: &str) {
		if self.fonts.len() <= index {
			self.fonts.resize(index + 1, String::new());
		}

		if self.fonts[index] != css {
			css.clone_into(&mut self.fonts[index]);
			self.resolved.remove(&index);
		}
	}

	fn resolution(&mut self, index: usize) -> &Resolution {
		if !self.resolved.contains_key(&index) {
			let resolution = self.resolve(&self.fonts[index].clone());
			self.resolved.insert(index, resolution);
		}

		&self.resolved[&index]
	}

	/// A canvas `font` string that draws the face the DOM uses for `css`.
	///
	/// Canvas and DOM text resolve a family list through the same fonts, but
	/// not always to the same face: WebKit's DOM skips `ui-monospace` in a
	/// list that its canvas honours. When the two disagree on a sample's
	/// width, the shortest tail of the family list that canvas draws as wide
	/// as the DOM draws the whole list stands in for it.
	///
	/// When no tail matches, the face is right but its size is not: Firefox's
	/// canvas snaps fractional font sizes to steps its DOM does not use. The
	/// font is then drawn as asked, and measured by the DOM.
	fn resolve(&mut self, css: &str) -> Resolution {
		let Some((prefix, list)) = split_shorthand(css) else {
			return Resolution {
				font: css.to_owned(),
				dom_widths: false,
				ligatures: Vec::new(),
			};
		};

		let (font, dom_widths) = self.face(css, prefix, list);
		let ligatures = self.ligatures(&font, css);

		Resolution {
			font,
			dom_widths,
			ligatures,
		}
	}

	/// The canvas font that draws `css`'s face, and whether its widths come
	/// from the DOM.
	fn face(&mut self, css: &str, prefix: &str, list: &str) -> (String, bool) {
		let dom = self.dom_width(css, RESOLUTION_SAMPLE);
		let families = split_families(list);
		// Whatever happens below leaves another font on the canvas.
		self.current.clear();

		for start in 0..families.len() {
			let candidate = format!("{prefix} {}", families[start..].join(", "));
			self.context.set_font(&candidate);

			if (self.canvas_width(RESOLUTION_SAMPLE) - dom).abs() <= SAME_FACE_TOLERANCE {
				return (candidate, false);
			}
		}

		(css.to_owned(), true)
	}

	/// The width the DOM lays `text` out at in `font`, or 0 before the page
	/// has a body.
	fn dom_width(&self, font: &str, text: &str) -> f64 {
		let style = format!(
			"position:absolute;left:0;top:0;visibility:hidden;white-space:pre;font:{font};\
			 font-kerning:normal;font-variant:normal;letter-spacing:0;word-spacing:0"
		);
		let Some(span) = self.element("span", &style) else {
			return 0.0;
		};
		span.set_text_content(Some(text));

		self.laid_out(&span, || span.get_bounding_client_rect().width())
			.unwrap_or(0.0)
	}

	/// The sequences `canvas_font` draws as ligatures, found by comparing
	/// caret edges inside each candidate in the DOM (in `css`) with the
	/// canvas's, at a large size so whole-pixel rectangles barely round.
	fn ligatures(&mut self, canvas_font: &str, css: &str) -> Vec<Ligature> {
		let (Some(host), Ok(range)) = (
			self.element("div", PROBE_HOST),
			self.document.create_range(),
		) else {
			return Vec::new();
		};
		let style = format!("font:{};{PROBE_TEXT}", at_size(css, LIGATURE_PROBE_PX));
		let rows: Vec<Option<HtmlElement>> = LIGATURE_CANDIDATES
			.iter()
			.map(|text| {
				let row = self.element("div", &style)?;
				row.set_text_content(Some(text));
				host.append_child(&row).ok()?;
				Some(row)
			})
			.collect();
		self.context
			.set_font(&at_size(canvas_font, LIGATURE_PROBE_PX));
		self.current.clear();

		self.laid_out(&host, || {
			LIGATURE_CANDIDATES
				.iter()
				.zip(&rows)
				.filter_map(|(text, row)| self.ligature(text, row.as_ref()?, &range))
				.collect()
		})
		.unwrap_or_default()
	}

	/// `text` as a ligature, when the face laid out in `row` draws it as
	/// one.
	fn ligature(&self, text: &'static str, row: &HtmlElement, range: &Range) -> Option<Ligature> {
		let node = row.first_child()?;
		let dom_width = row.get_bounding_client_rect().width();
		let canvas_width = self.canvas_width(text);
		let mut edges = Vec::new();
		let mut ligated = false;

		// The candidates are ASCII, so bytes, characters and UTF-16 code
		// units all count the same.
		for split in 1..text.len() {
			let dom = range_width(range, &node, split as u32) / dom_width;
			let canvas = (self.canvas_width(&text[..=split])
				- self.canvas_width(&text[split..=split]))
				/ canvas_width;
			ligated |= (dom - canvas).abs() > LIGATURE_TOLERANCE;
			edges.push(dom);
		}

		(ligated && dom_width > 0.0 && canvas_width > 0.0).then_some(Ligature { text, edges })
	}

	/// Read inline boxes from the DOM, in one layout pass: for a font alone,
	/// its content area's ascent and descent; with a unitless line height,
	/// the space its line box keeps above and below the baseline.
	///
	/// Canvas reports font metrics rounded to whole CSS pixels, but browsers
	/// lay out lines with their own rounding: Blink rounds ascent and descent
	/// to device pixels and floors the half-leading, WebKit keeps 1/64 px,
	/// and Firefox keeps floats. Reading the boxes from real DOM text makes
	/// the canvas's line positions the browser's, whatever its rules.
	fn probe(&self, boxes: &[(&str, Option<f32>)]) -> Vec<(f32, f32)> {
		let unmeasured = || vec![(0.0, 0.0); boxes.len()];

		if boxes.is_empty() {
			return Vec::new();
		}

		let Some(host) = self.element("div", PROBE_HOST) else {
			return unmeasured();
		};
		let rows: Vec<Option<(HtmlElement, HtmlElement)>> = boxes
			.iter()
			.map(|(font, line_height)| {
				// Unitless, as CSS around the editor writes it: browsers round
				// a length and a multiple of the font size to layout units
				// differently.
				let line_height =
					line_height.map_or_else(|| "normal".to_owned(), canvist_layout::css_number);
				let row = self.element(
					"div",
					&format!("font:{font};line-height:{line_height};{PROBE_TEXT}"),
				)?;
				let baseline = self.element("span", "display:inline-block;width:0;height:0")?;
				row.append_child(&baseline).ok()?;
				row.append_child(&self.document.create_text_node("x"))
					.ok()?;
				host.append_child(&row).ok()?;
				Some((row, baseline))
			})
			.collect();

		self.laid_out(&host, || {
			boxes
				.iter()
				.zip(&rows)
				.map(|((_, line_height), row)| {
					row.as_ref()
						.and_then(|(row, baseline)| {
							self.read_box(row, baseline, line_height.is_some())
						})
						.unwrap_or((0.0, 0.0))
				})
				.collect()
		})
		.unwrap_or_else(unmeasured)
	}

	/// The space above and below the baseline in `row`: of its line box, or
	/// of its text's content area.
	fn read_box(
		&self,
		row: &HtmlElement,
		baseline: &HtmlElement,
		line_box: bool,
	) -> Option<(f32, f32)> {
		let bounds = row.get_bounding_client_rect();
		let top = bounds.top();
		let base = baseline.get_bounding_client_rect().bottom() - top;

		if line_box {
			return Some((base as f32, (bounds.height() - base) as f32));
		}

		let range = self.document.create_range().ok()?;
		range.select_node_contents(&row.last_child()?).ok()?;
		let content = range.get_bounding_client_rect();

		Some((
			(base - (content.top() - top)) as f32,
			(content.bottom() - top - base) as f32,
		))
	}

	/// Make font `index` current on the measuring canvas, as it is drawn.
	fn select(&mut self, index: usize) {
		let font = self.resolution(index).font.clone();

		if font != self.current {
			self.context.set_font(&font);
			self.current = font;
		}
	}

	/// Push one advance per scalar value of `segment`.
	///
	/// A shaper adds a kerning pair's adjustment to the first glyph's advance,
	/// and the browser places carets and selection edges by those advances.
	/// So the edge after grapheme `k` is the width of the first `k + 1`
	/// graphemes less the next grapheme's own width, and the last edge is the
	/// whole segment's width: the advances sum to what `fillText` paints, and
	/// each edge sits where the DOM puts it. Inside a ligature the browser
	/// splits the glyph's width instead, so edges there follow `ligatures`.
	fn advances(&mut self, segment: &str, ligatures: &[Ligature], out: &mut Vec<f64>) {
		let clusters = clusters(segment);
		let mut edges = Vec::with_capacity(clusters.len());
		let mut end = 0;

		for (index, cluster) in clusters.iter().enumerate() {
			end += cluster.len();
			edges.push(self.edge(segment, end, clusters.get(index + 1).copied()));
		}

		split_ligatures(&clusters, &mut edges, ligatures);
		let mut previous = 0.0;

		for (cluster, edge) in clusters.iter().zip(edges) {
			out.push(edge - previous);
			previous = edge;
			out.extend(cluster.chars().skip(1).map(|_| 0.0));
		}
	}

	/// Where the caret edge after the cluster ending at byte `end` sits: the
	/// segment's whole width after the last cluster, otherwise the width up
	/// to and including `next` less `next` measured alone, so a kerning
	/// pair's adjustment stays with the first glyph.
	fn edge(&mut self, segment: &str, end: usize, next: Option<&str>) -> f64 {
		let Some(next) = next else {
			return self.canvas_width(segment);
		};

		self.canvas_width(&segment[..end + next.len()]) - self.alone(next)
	}

	/// The width of `cluster` on its own in the current font, cached per
	/// font.
	fn alone(&mut self, cluster: &str) -> f64 {
		let key = format!("{}\u{0}{cluster}", self.current);

		if let Some(width) = self.singles.get(&key) {
			return *width;
		}

		let width = self.canvas_width(cluster);
		self.singles.insert(key, width);
		width
	}

	/// Advances the DOM has to supply, in one layout pass.
	///
	/// - In scripts whose letters join, a prefix of an Arabic word measured
	///   alone ends in a final form, not the joined form the letter has in the
	///   word, and canvas ignores a zero width joiner that would keep it
	///   joined. The DOM places each grapheme by its glyph in context, so its
	///   grapheme boxes give the advances.
	/// - Where canvas and DOM disagree on a font's size (see
	///   [`DomMeasurer::resolve`]), the DOM's width of the whole segment is the
	///   one layout must wrap by; the canvas advances are scaled to it.
	fn supply_from_dom(&self, segments: &[DomSegment<'_>], out: &mut [f64]) {
		if segments.is_empty() {
			return;
		}

		let (Some(host), Ok(range)) = (
			self.element("div", PROBE_HOST),
			self.document.create_range(),
		) else {
			return;
		};
		let rows: Vec<Option<HtmlElement>> = segments
			.iter()
			.map(|entry| {
				let row = self.element("div", &format!("font:{};{PROBE_TEXT}", entry.font))?;
				row.set_text_content(Some(entry.segment));
				host.append_child(&row).ok()?;
				Some(row)
			})
			.collect();

		self.laid_out(&host, || {
			for (entry, row) in segments.iter().zip(&rows) {
				let Some((row, node)) =
					row.as_ref().and_then(|row| Some((row, row.first_child()?)))
				else {
					continue;
				};
				let total = entry
					.width
					.unwrap_or_else(|| row.get_bounding_client_rect().width());
				let count = entry.segment.chars().count();
				let advances = &mut out[entry.at..entry.at + count];

				if entry.joined {
					joined_advances(entry.segment, &range, &node, total, advances);
				}

				scale(advances, total);
			}
		});
	}

	/// The width `measureText` gives `text` in the canvas's current font.
	fn canvas_width(&self, text: &str) -> f64 {
		self.context
			.measure_text(text)
			.map_or(0.0, |metrics| metrics.width())
	}

	/// A new probe element styled with `css`, not yet in the page.
	fn element(&self, tag: &str, css: &str) -> Option<HtmlElement> {
		dom::probe(&self.document, tag, css)
	}

	/// Put `probe` in the page for as long as `read` runs. `None` before the
	/// page has a body to lay it out in.
	fn laid_out<T>(&self, probe: &HtmlElement, read: impl FnOnce() -> T) -> Option<T> {
		dom::laid_out(&self.document, probe, read)
	}
}

impl TextMeasurer for DomMeasurer {
	fn measure(&mut self, batch: &MeasureBatch<'_>, out: &mut MeasureOutput) {
		for (index, font) in batch.fonts.fonts().iter().enumerate() {
			self.register(index, &font.css());
		}

		let fonts = self.fonts.clone();
		let css = |id: FontId| fonts.get(id.index()).map_or("", String::as_str);
		let boxes: Vec<(&str, Option<f32>)> = batch
			.metrics
			.iter()
			.map(|id| (css(*id), None))
			.chain(
				batch
					.line_boxes
					.iter()
					.map(|(id, line_height)| (css(*id), Some(*line_height))),
			)
			.collect();
		let probed = self.probe(&boxes);
		let (metrics, line_boxes) = probed.split_at(batch.metrics.len());
		out.metrics.extend(
			metrics
				.iter()
				.map(|&(ascent, descent)| FontMetrics { ascent, descent }),
		);
		out.line_boxes.extend(
			line_boxes
				.iter()
				.map(|&(above, below)| LineBox { above, below }),
		);

		for (font, _) in &boxes {
			self.used.entry((*font).to_owned()).or_default();
		}

		let mut advances = Vec::new();
		let mut from_dom = Vec::new();

		for segment in batch.segments {
			let index = segment.font.index();
			self.select(index);
			let resolution = self.resolution(index).clone();
			let at = advances.len();
			let joined = segment.text.chars().any(joins);

			if joined {
				// The DOM replaces these.
				advances.extend(segment.text.chars().map(|_| 0.0));
			} else {
				self.advances(segment.text, &resolution.ligatures, &mut advances);
			}

			if joined || resolution.dom_widths {
				from_dom.push(DomSegment {
					font: css(segment.font),
					segment: segment.text,
					at,
					joined,
					width: (!resolution.dom_widths).then(|| self.canvas_width(segment.text)),
				});
			}

			self.used
				.entry(css(segment.font).to_owned())
				.or_default()
				.push_str(segment.text);
		}

		self.supply_from_dom(&from_dom, &mut advances);
		out.advances
			.extend(advances.into_iter().map(|advance| advance as f32));
	}
}

/// Write the advances of a segment whose letters join, read from `node`,
/// its laid-out text. Each edge is the width of the prefix that ends there,
/// read in the word's context: a browser that rounds rectangles to whole
/// pixels (WebKit) then misplaces an edge by under a pixel, rather than
/// accumulating each grapheme's rounding.
fn joined_advances(segment: &str, range: &Range, node: &Node, total: f64, out: &mut [f64]) {
	let mut scalar = 0;
	let mut units = 0;
	let mut previous = 0.0;
	let mut clusters = segment.graphemes(true).peekable();

	while let Some(cluster) = clusters.next() {
		units += cluster.encode_utf16().count() as u32;
		let edge = if clusters.peek().is_none() {
			total
		} else {
			range_width(range, node, units)
		};

		if let Some(advance) = out.get_mut(scalar) {
			*advance = edge - previous;
		}

		previous = edge;
		scalar += cluster.chars().count();
	}
}

/// The width of the first `units` UTF-16 code units of the text in `node`,
/// which is how the DOM counts offsets into text.
fn range_width(range: &Range, node: &Node, units: u32) -> f64 {
	if range.set_start(node, 0).is_err() || range.set_end(node, units).is_err() {
		return 0.0;
	}

	range.get_bounding_client_rect().width()
}
