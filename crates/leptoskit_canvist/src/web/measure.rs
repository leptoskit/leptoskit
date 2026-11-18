use canvist_layout::FontId;
use canvist_layout::FontMetrics;
use canvist_layout::MeasureBatch;
use canvist_layout::MeasureOutput;
use canvist_layout::TextMeasurer;
use unicode_segmentation::UnicodeSegmentation;
use wasm_bindgen::JsCast;
use web_sys::CanvasRenderingContext2d;
use web_sys::HtmlCanvasElement;

const RESOLUTION_SAMPLE: &str = "The quick brown fox jumps over the lazy dog 0123456789";
const SAME_FACE_TOLERANCE: f32 = 0.05;
const LIGATURE_CANDIDATES: [&str; 8] = ["ffi", "ffl", "ff", "fi", "fl", "fj", "ft", "Th"];
const LIGATURE_PROBE_PX: f32 = 200.0;
const LIGATURE_TOLERANCE: f32 = 0.02;

pub struct CanvasMeasurer {
	document: web_sys::Document,
	context: CanvasRenderingContext2d,
	fonts: Vec<String>,
	resolved: Vec<Option<Resolution>>,
	font: String,
	singles: std::collections::HashMap<String, f32>,
	segments: u64,
	calls: u64,
}

#[derive(Clone)]
struct Resolution {
	font: String,
	dom_widths: bool,
	ligatures: Vec<Ligature>,
}

#[derive(Clone)]
struct Ligature {
	text: String,
	edges: Vec<f32>,
}

impl CanvasMeasurer {
	pub fn new(document: web_sys::Document) -> Self {
		let canvas: HtmlCanvasElement = document.create_element("canvas").unwrap().dyn_into().unwrap();
		let context: CanvasRenderingContext2d = canvas.get_context("2d").unwrap().unwrap().dyn_into().unwrap();
		let text: &crate::web::paint::TextContext = context.unchecked_ref();
		text.set_font_kerning("normal");
		text.set_font_variant_caps("normal");
		text.set_text_rendering("auto");
		text.set_word_spacing("0px");
		context.set_text_align("left");
		context.set_text_baseline("alphabetic");
		Self {
			document,
			context,
			fonts: Vec::new(),
			resolved: Vec::new(),
			font: String::new(),
			singles: std::collections::HashMap::new(),
			segments: 0,
			calls: 0,
		}
	}

	pub fn paint_font(&self, id: FontId) -> String {
		self.resolved
			.get(id.index())
			.and_then(|r| r.as_ref()).map_or_else(|| self.css(id.index()), |r| r.font.clone())
	}

	pub fn forget(&mut self) {
		self.singles.clear();
		self.resolved.clear();
		self.font.clear();
	}

	pub fn segments(&self) -> u64 {
		self.segments
	}

	pub fn calls(&self) -> u64 {
		self.calls
	}

	fn css(&self, id: usize) -> String {
		self.fonts.get(id).cloned().unwrap_or_default()
	}

	fn resolution(&mut self, id: usize) -> Resolution {
		if id >= self.resolved.len() {
			self.resolved.resize(id + 1, None);
		}
		if self.resolved[id].is_none() {
			let css = self.css(id);
			let r = self.resolve(&css);
			self.resolved[id] = Some(r);
		}
		self.resolved[id].clone().unwrap()
	}

	// WebKit fix: canvas and DOM resolve a family list through the same fonts
	// but not always to the same face. When the two disagree on a sample's
	// width, the shortest tail of the family list that canvas draws as wide
	// as the DOM draws the whole list stands in for it. When no tail matches,
	// the face is right but its size is not (Firefox snaps fractional sizes);
	// then widths come from the DOM.
	fn resolve(&mut self, css: &str) -> Resolution {
		// parse "style weight sizepx families"
		let (prefix, list) = parse_font(css);
		let dom = self.dom_width(css, RESOLUTION_SAMPLE);
		let families = split_families(list);
		for start in 0..families.len() {
			let candidate = format!("{} {}", prefix, families[start..].join(", "));
			self.context.set_font(&candidate);
			let canvas_w = self.context.measure_text(RESOLUTION_SAMPLE).map_or(0.0, |m| m.width() as f32);
			if (canvas_w - dom).abs() <= SAME_FACE_TOLERANCE {
				self.font = String::new();
				let ligatures = self.ligatures(&candidate, css);
				return Resolution { font: candidate, dom_widths: false, ligatures };
			}
		}
		self.font = String::new();
		let ligatures = self.ligatures(css, css);
		Resolution { font: css.to_owned(), dom_widths: true, ligatures }
	}

	fn dom_width(&self, font: &str, text: &str) -> f32 {
		let span: web_sys::HtmlElement = self.document.create_element("span").unwrap().dyn_into().unwrap();
		span.set_attribute("aria-hidden", "true").ok();
		span.style().set_css_text(&format!(
			"position:absolute;left:0;top:0;visibility:hidden;white-space:pre;font:{font};font-kerning:normal;font-variant:normal;letter-spacing:0;word-spacing:0"
		));
		span.set_text_content(Some(text));
		self.document.body().unwrap().append_child(&span).ok();
		let w = span.get_bounding_client_rect().width() as f32;
		span.remove();
		w
	}

	fn ligatures(&mut self, canvas_font: &str, css: &str) -> Vec<Ligature> {
		let document = &self.document;
		let host: web_sys::HtmlElement = document.create_element("div").unwrap().dyn_into().unwrap();
		host.set_attribute("aria-hidden", "true").ok();
		host.style().set_css_text("position:absolute;left:0;top:0;width:0;height:0;overflow:hidden;visibility:hidden;pointer-events:none");
		let mut rows = Vec::new();
		for cand in &LIGATURE_CANDIDATES {
			let row: web_sys::HtmlElement = document.create_element("div").unwrap().dyn_into().unwrap();
			row.style().set_css_text(&format!(
				"font:{};white-space:pre;width:max-content;font-kerning:normal;font-variant:normal;letter-spacing:0;word-spacing:0",
				at_size(css, LIGATURE_PROBE_PX)
			));
			row.set_text_content(Some(cand));
			host.append_child(&row).ok();
			rows.push(row);
		}
		document.body().unwrap().append_child(&host).ok();
		self.context.set_font(&at_size(canvas_font, LIGATURE_PROBE_PX));
		self.font.clear();
		let mut found = Vec::new();
		for (i, cand) in LIGATURE_CANDIDATES.iter().enumerate() {
			let row = &rows[i];
			let node = row.first_child();
			if node.is_none() { continue; }
			let dom_width = row.get_bounding_client_rect().width() as f32;
			let canvas_width = self.context.measure_text(cand).map_or(0.0, |m| m.width() as f32);
			let mut edges = Vec::new();
			let mut ligated = false;
			let node = node.unwrap();
			for split in 1..cand.len() {
				let range = document.create_range().unwrap();
				range.set_start(&node, 0).ok();
				range.set_end(&node, split as u32).ok();
				let dom = range.get_bounding_client_rect().width() as f32 / dom_width.max(1.0);
				let canvas = (self.context.measure_text(&cand[..=split]).map_or(0.0, |m| m.width() as f32)
					- self.context.measure_text(&cand[split..=split]).map_or(0.0, |m| m.width() as f32))
					/ canvas_width.max(1.0);
				if (dom - canvas).abs() > LIGATURE_TOLERANCE {
					ligated = true;
				}
				edges.push(dom);
			}
			if ligated && dom_width > 0.0 && canvas_width > 0.0 {
				found.push(Ligature { text: cand.to_string(), edges });
			}
		}
		host.remove();
		found
	}

	pub fn add_font(&mut self, id: usize, css: &str) {
		if id >= self.fonts.len() {
			self.fonts.resize(id + 1, String::new());
			self.resolved.resize(id + 1, None);
		}
		self.fonts[id] = css.to_owned();
	}
}

impl TextMeasurer for CanvasMeasurer {
	fn measure(&mut self, batch: &MeasureBatch<'_>, out: &mut MeasureOutput) {
		let fonts = batch.fonts.fonts();

		// Register fonts.
		for (i, font) in fonts.iter().enumerate() {
			if i >= self.fonts.len() || self.fonts[i].is_empty() {
				self.add_font(i, &font.css());
			}
		}

		// Metrics via canvas TextMetrics (DOM probe ported partially).
		for &id in batch.metrics {
			let _css = self.fonts[id.index()].clone();
			let resolved = self.resolution(id.index());
			self.context.set_font(&resolved.font);
			self.font = resolved.font.clone();
			let m = self.context.measure_text("x").ok();
			let ascent = m.as_ref().map_or(resolved.font.split_whitespace().nth(2).unwrap_or("16px").trim_end_matches("px").parse::<f32>().unwrap_or(16.0) * 0.8, |mm| mm.font_bounding_box_ascent() as f32);
			let descent = m.as_ref().map_or(ascent * 0.2, |mm| mm.font_bounding_box_descent() as f32);
			out.metrics.push(FontMetrics { ascent, descent });
		}

		// Line boxes: leave empty -> layout derives from metrics (CSS 2.1).

		// Advances.
		let mut start = 0usize;
		// We need text slice boundaries; batch.segments gives SegmentRequest with text.
		// Collect joined/dom segments.
		let mut from_dom: Vec<(usize, String, usize, bool, Option<f32>)> = Vec::new(); // (font_idx, segment, out_index, joined, width)

		for segment in batch.segments {
			let font_idx = segment.font.index();
			let seg_text = segment.text;
			let res = self.resolution(font_idx);
			let out_index = out.advances.len();
			let joined = is_joining(seg_text);
			let dom_widths = res.dom_widths;
			self.push_advances(seg_text, joined, &mut out.advances, &res.ligatures);

			if joined || dom_widths {
				let canvas_w = if dom_widths {
					None
				} else {
					self.context.set_font(&res.font);
					self.font = res.font.clone();
					Some(self.context.measure_text(seg_text).map_or(0.0, |m| m.width() as f32))
				};
				from_dom.push((font_idx, seg_text.to_owned(), out_index, joined, canvas_w));
			}
			self.segments += 1;
			let _ = start;
			start += seg_text.len();
		}

		// fromDom: DOM-derived advances
		if !from_dom.is_empty() {
			// Build a host div with one span per segment.
			let document = &self.document;
			let host: web_sys::HtmlElement = document.create_element("div").unwrap().dyn_into().unwrap();
			host.style().set_css_text("position:absolute;left:0;top:0;width:0;height:0;overflow:hidden;visibility:hidden;pointer-events:none");
			let mut spans = Vec::new();
			for (font_idx, seg, _, _, _) in &from_dom {
				let span: web_sys::HtmlElement = document.create_element("div").unwrap().dyn_into().unwrap();
				let css = self.fonts[*font_idx].clone();
				span.style().set_css_text(&format!(
					"font:{css};white-space:pre;width:max-content;font-kerning:normal;font-variant:normal;letter-spacing:0;word-spacing:0"
				));
				span.set_text_content(Some(seg));
				host.append_child(&span).ok();
				spans.push(span);
			}
			document.body().unwrap().append_child(&host).ok();

			for (i, (font_idx, seg, at, joined, canvas_w)) in from_dom.iter().enumerate() {
				let span = &spans[i];
				let node = span.first_child();
				if node.is_none() { continue; }
				let node = node.unwrap();
				let total = canvas_w.unwrap_or_else(|| span.get_bounding_client_rect().width() as f32);
				if *joined {
					// joined advances via Range edges
					let mut scalar = 0usize;
					let mut previous = 0.0f32;
					for (offset, cluster) in seg.grapheme_indices(true) {
						let end = offset + cluster.len();
						let range = document.create_range().unwrap();
						range.set_start(&node, 0).ok();
						range.set_end(&node, end as u32).ok();
						let edge = if end == seg.len() {
							total
						} else {
							range.get_bounding_client_rect().width() as f32
						};
						if *at + scalar < out.advances.len() {
							out.advances[*at + scalar] = edge - previous;
						}
						previous = edge;
						scalar += cluster.chars().count();
					}
				}
				// scale
				let count = seg.chars().count();
				let mut sum = 0.0f32;
				for j in 0..count {
					if *at + j < out.advances.len() {
						sum += out.advances[*at + j];
					}
				}
				if sum > 0.0 {
					let factor = total / sum;
					for j in 0..count {
						if *at + j < out.advances.len() {
							out.advances[*at + j] *= factor;
						}
					}
				}
				let _ = font_idx;
			}
			host.remove();
		}

		self.calls += 1;
	}
}

fn is_joining(text: &str) -> bool {
	text.chars().any(|c| matches!(c as u32, 0x0600..=0x07ff | 0x0800..=0x08ff | 0x1800..=0x18af | 0xfb50..=0xfdff | 0xfe70..=0xfeff))
}

fn parse_font(css: &str) -> (String, &str) {
	// "style weight sizepx family, ..."
	let parts: Vec<&str> = css.splitn(4, ' ').collect();
	if parts.len() >= 4 {
		(format!("{} {} {}", parts[0], parts[1], parts[2]), parts[3])
	} else {
		(css.to_owned(), "")
	}
}

fn split_families(list: &str) -> Vec<String> {
	let mut out = Vec::new();
	let mut cur = String::new();
	let mut quote: Option<char> = None;
	for ch in list.chars() {
		if quote.is_none() && ch == ',' {
			let t = cur.trim().to_owned();
			if !t.is_empty() { out.push(t); }
			cur.clear();
			continue;
		}
		cur.push(ch);
		if let Some(q) = quote {
			if ch == q { quote = None; }
		} else if ch == '"' || ch == '\'' {
			quote = Some(ch);
		}
	}
	let t = cur.trim().to_owned();
	if !t.is_empty() { out.push(t); }
	out
}

impl CanvasMeasurer {
	fn push_advances(&mut self, segment: &str, joined: bool, out: &mut Vec<f32>, ligatures: &[Ligature]) {
		if joined {
			let scalars = segment.chars().count();
			let start = out.len();
			out.resize(start + scalars, 0.0);
			return;
		}
		let _ = ();
		// Simpler robust implementation: measure prefixes.
		let chars: Vec<char> = segment.chars().collect();
		let _n = chars.len();
		// Build edges: edge[i] = width after cluster i (grapheme). Use graphemes.
		let graphemes: Vec<&str> = segment.graphemes(true).collect();
		let mut edges: Vec<f32> = Vec::with_capacity(graphemes.len());
		let mut end_acc = 0usize; // byte end
		for (gi, g) in graphemes.iter().enumerate() {
			end_acc += g.len();
			let next = graphemes.get(gi + 1).copied();
			let edge = if next.is_none() {
				self.measure_text(segment) // whole
			} else {
				let prefix_end = end_acc + next.unwrap().len();
				let whole = self.measure_text(&segment[..prefix_end]);
				let alone = self.measure_text(next.unwrap());
				whole - alone
			};
			edges.push(edge);
		}
		// split ligatures
		split_ligatures(&graphemes, &mut edges, ligatures);
		// emit advances
		let mut previous = 0.0f32;
		for (gi, g) in graphemes.iter().enumerate() {
			let edge = edges[gi];
			out.push(edge - previous);
			previous = edge;
			for _ in 1..g.chars().count() {
				out.push(0.0);
			}
		}
	}
}

fn split_ligatures(clusters: &[&str], edges: &mut [f32], ligatures: &[Ligature]) {
	for i in 0..clusters.len() {
		if ligatures.is_empty() { break; }
		let lig = ligatures.iter().find(|l| {
			l.text.chars().enumerate().all(|(o, ch)| clusters.get(i + o).is_some_and(|c| c == &ch.to_string()))
		});
		if let Some(lig) = lig {
			let start = if i == 0 { 0.0 } else { edges[i - 1] };
			let last = i + lig.text.chars().count() - 1;
			let width = edges[last] - start;
			for (o, share) in lig.edges.iter().enumerate() {
				edges[i + o] = start + share * width;
			}
		}
	}
}

impl CanvasMeasurer {
	fn measure_text(&mut self, text: &str) -> f32 {
		// use resolved font currently set? We'll set font to css of current... 
		// For prefix measures within a segment, use the canvas font already set
		// by caller. Here we just measure with whatever font is current.
		self.context.measure_text(text).map_or(0.0, |m| m.width() as f32)
	}
}

// Fix at_size
fn at_size(font: &str, size: f32) -> String {
	// replace the first "Npx" size token
	let mut out = String::new();
	let mut found = false;
	for token in font.split_whitespace() {
		if !found && token.ends_with("px") && token[..token.len()-2].parse::<f32>().is_ok() {
			out.push_str(&format!("{size}px "));
			found = true;
		} else {
			out.push_str(token);
			out.push(' ');
		}
	}
	out.trim_end().to_owned()
}