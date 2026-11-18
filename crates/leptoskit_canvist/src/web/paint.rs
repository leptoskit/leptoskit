use canvist_layout::Color;
use canvist_layout::DecorationKind;
use canvist_layout::DisplayItem;
use canvist_layout::DisplayList;
use canvist_layout::FontId;
use canvist_layout::Rect;
use canvist_layout::TextRun;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::prelude::wasm_bindgen;
use web_sys::CanvasRenderingContext2d;
use web_sys::ContextAttributes2d;
use web_sys::HtmlCanvasElement;

const OVERHANG_PER_FONT_SIZE: f32 = 0.5;
const SQUIGGLE_AMPLITUDE_PER_THICKNESS: f32 = 1.5;
const SQUIGGLE_WAVELENGTH_PER_AMPLITUDE: f32 = 4.0;

#[wasm_bindgen]
extern "C" {
	#[wasm_bindgen(extends = CanvasRenderingContext2d)]
	pub type TextContext;
	#[wasm_bindgen(method, setter = fontKerning)]
	pub fn set_font_kerning(this: &TextContext, value: &str);
	#[wasm_bindgen(method, setter = fontVariantCaps)]
	pub fn set_font_variant_caps(this: &TextContext, value: &str);
	#[wasm_bindgen(method, setter = textRendering)]
	pub fn set_text_rendering(this: &TextContext, value: &str);
	#[wasm_bindgen(method, setter = wordSpacing)]
	pub fn set_word_spacing(this: &TextContext, value: &str);
	#[wasm_bindgen(method, setter = letterSpacing)]
	pub fn set_letter_spacing(this: &TextContext, value: &str);
	#[wasm_bindgen(method, setter = direction)]
	pub fn set_direction(this: &TextContext, value: &str);
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Surface {
	pub width: f32,
	pub height: f32,
	pub scale: f32,
	pub pixel_width: u32,
	pub pixel_height: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PixelGrid {
	#[default]
	Device,
	Css,
}

pub enum Region<'a> {
	All,
	Rects(&'a [Rect]),
}

#[derive(Default)]
struct State {
	font: Option<FontId>,
	fill: Option<Color>,
	letter_spacing: Option<u32>,
	rtl: Option<bool>,
}

pub struct Painter {
	canvas: HtmlCanvasElement,
	context: CanvasRenderingContext2d,
	surface: Surface,
	fonts: Vec<String>,
	hidden_families: Vec<String>,
	hidden: Vec<bool>,
	grid: PixelGrid,
	colors: std::collections::HashMap<Color, String>,
	state: State,
	drawn: u32,
}

impl Painter {
	pub fn new(canvas: HtmlCanvasElement) -> Result<Self, JsValue> {
		let attributes = ContextAttributes2d::new();
		attributes.set_alpha(false);
		let context = canvas
			.get_context_with_context_options("2d", &attributes)?
			.ok_or_else(|| JsValue::from_str("no 2d context"))?
			.dyn_into::<CanvasRenderingContext2d>()?;
		let mut painter = Self {
			canvas,
			context: context.clone(),
			surface: Surface { width: 0.0, height: 0.0, scale: 1.0, pixel_width: 0, pixel_height: 0 },
			fonts: Vec::new(),
			hidden_families: Vec::new(),
			hidden: Vec::new(),
			grid: PixelGrid::default(),
			colors: std::collections::HashMap::new(),
			state: State::default(),
			drawn: 0,
		};
		painter.reset_context();
		Ok(painter)
	}

	pub fn surface(&self) -> Surface { self.surface }

	pub fn hide_families(&mut self, families: Vec<String>) -> bool {
		if families == self.hidden_families { return false; }
		self.hidden_families = families;
		self.fonts.clear();
		self.hidden.clear();
		true
	}

	pub fn forget_fonts(&mut self) {
		self.fonts.clear();
		self.hidden.clear();
		self.state.font = None;
	}

	pub fn set_grid(&mut self, grid: PixelGrid) -> bool {
		let changed = grid != self.grid;
		self.grid = grid;
		changed
	}

	pub fn set_pixel_grid_str(&mut self, grid: &str) -> bool {
		let g = match grid {
			"device" => PixelGrid::Device,
			"css" => PixelGrid::Css,
			_ => return false,
		};
		self.set_grid(g)
	}

	pub fn drawn(&self) -> u32 { self.drawn }

	pub fn resize(&mut self, surface: Surface) -> bool {
		if surface == self.surface { return false; }
		if (surface.pixel_width, surface.pixel_height) != (self.surface.pixel_width, self.surface.pixel_height) {
			self.canvas.set_width(surface.pixel_width);
			self.canvas.set_height(surface.pixel_height);
		}
		self.surface = surface;
		self.reset_context();
		true
	}

	fn reset_context(&mut self) {
		let context = &self.context;
		let text: &TextContext = context.unchecked_ref();
		text.set_font_kerning("normal");
		text.set_font_variant_caps("normal");
		text.set_text_rendering("auto");
		text.set_word_spacing("0px");
		context.set_text_align("left");
		context.set_text_baseline("alphabetic");
		self.state = State::default();
	}

	pub fn paint(
		&mut self,
		list: &DisplayList,
		top: f32,
		background: Color,
		region: &Region<'_>,
		font: &dyn Fn(FontId) -> String,
	) {
		self.sync_fonts(list, font);
		let context = self.context.clone();
		let scale = f64::from(self.surface.scale);
		let _ = context.set_transform(scale, 0.0, 0.0, scale, 0.0, 0.0);
		self.state = State::default();
		self.drawn = 0;
		match region {
			Region::All => self.paint_all(list, top, background),
			Region::Rects(rects) => self.paint_rects(list, top, background, rects),
		}
	}

	fn paint_all(&mut self, list: &DisplayList, top: f32, background: Color) {
		self.fill_surface(background);
		for item in &list.items {
			self.item(list, item, top);
		}
	}

	fn paint_rects(&mut self, list: &DisplayList, top: f32, background: Color, rects: &[Rect]) {
		let overhang = overhang(list);
		let rects: Vec<Rect> = rects.iter().map(|r| inflate(*r, overhang)).collect();
		let context = self.context.clone();
		context.save();
		self.clip_to(&rects, top);
		self.fill_surface(background);
		for item in &list.items {
			let bounds = inflate(item.bounds(), overhang);
			if rects.iter().any(|r| r.intersects(&bounds)) {
				self.item(list, item, top);
			}
		}
		context.restore();
		self.state = State::default();
	}

	fn clip_to(&self, rects: &[Rect], top: f32) {
		let context = &self.context;
		context.begin_path();
		for rect in rects {
			let snapped = self.snap_out(Rect::new(rect.x, rect.y - top, rect.width, rect.height));
			context.rect(f64::from(snapped.x), f64::from(snapped.y), f64::from(snapped.width), f64::from(snapped.height));
		}
		context.clip();
	}

	fn fill_surface(&mut self, background: Color) {
		let scale = f64::from(self.surface.scale);
		self.fill_color(background);
		self.context.fill_rect(
			0.0,
			0.0,
			f64::from(self.surface.width).max(f64::from(self.surface.pixel_width) / scale),
			f64::from(self.surface.height).max(f64::from(self.surface.pixel_height) / scale),
		);
	}

	fn item(&mut self, list: &DisplayList, item: &DisplayItem, top: f32) {
		self.drawn += 1;
		match item {
			DisplayItem::Rect { rect, color, radius, .. } => {
				let rect = self.snap(Rect::new(rect.x, rect.y - top, rect.width, rect.height));
				self.fill_color(*color);
				self.fill_rect(rect, *radius);
			}
			DisplayItem::Text(run) => self.text(run, top),
			DisplayItem::Decoration { kind: DecorationKind::Squiggle, x, y, width, thickness, color, .. } => {
				self.squiggle((*x, *y - top), *width, *thickness, *color);
			}
			DisplayItem::Decoration { x, y, width, thickness, color, .. } => {
				let line = self.snap(Rect::new(*x, y - top - thickness / 2.0, *width, *thickness));
				self.fill_color(*color);
				self.fill_rect(line, 0.0);
			}
			DisplayItem::Stroke { points, width, color, .. } => {
				self.stroke(list.stroke_points(points), top, *width, *color);
			}
			DisplayItem::Slot { .. } => {}
		}
	}

	fn text(&mut self, run: &TextRun, top: f32) {
		let index = run.font.index();
		if index >= self.fonts.len() || self.hidden.get(index).copied().unwrap_or(false) {
			return;
		}
		let context = self.context.clone();
		let text: &TextContext = context.unchecked_ref();
		if self.state.font != Some(run.font) {
			context.set_font(&self.fonts[index]);
			self.state.font = Some(run.font);
		}
		let spacing = run.letter_spacing.to_bits();
		if self.state.letter_spacing != Some(spacing) {
			text.set_letter_spacing(&format!("{}px", canvist_layout::css_number(run.letter_spacing)));
			self.state.letter_spacing = Some(spacing);
		}
		if self.state.rtl != Some(run.rtl) {
			text.set_direction(if run.rtl { "rtl" } else { "ltr" });
			self.state.rtl = Some(run.rtl);
		}
		self.fill_color(run.color);
		let baseline = self.round(run.baseline - top);
		let _ = context.fill_text(run.text(), f64::from(run.x), f64::from(baseline));
	}

	fn squiggle(&mut self, (x, y): (f32, f32), width: f32, thickness: f32, color: Color) {
		let amplitude = thickness * SQUIGGLE_AMPLITUDE_PER_THICKNESS;
		let wave = amplitude * SQUIGGLE_WAVELENGTH_PER_AMPLITUDE;
		let context = self.context.clone();
		context.begin_path();
		context.move_to(f64::from(x), f64::from(y));
		let mut position = 0.0;
		let mut up = true;
		while position < width {
			let step = (wave / 2.0).min(width - position);
			let peak = if up { -amplitude } else { amplitude };
			context.quadratic_curve_to(
				f64::from(x + position + step / 2.0),
				f64::from(y + peak),
				f64::from(x + position + step),
				f64::from(y),
			);
			position += step;
			up = !up;
		}
		let color = self.color(color);
		context.set_stroke_style_str(&color);
		context.set_line_width(f64::from(thickness));
		context.stroke();
	}

	fn stroke(&mut self, points: &[f32], top: f32, width: f32, color: Color) {
		let context = self.context.clone();
		context.begin_path();
		for (index, point) in points.as_chunks::<2>().0.iter().enumerate() {
			let (x, y) = (f64::from(point[0]), f64::from(point[1] - top));
			if index == 0 { context.move_to(x, y); }
			context.line_to(x, y);
		}
		let color = self.color(color);
		context.set_stroke_style_str(&color);
		context.set_line_width(f64::from(width));
		context.set_line_cap("round");
		context.set_line_join("round");
		context.stroke();
	}

	fn fill_rect(&self, rect: Rect, radius: f32) {
		if radius <= 0.0 {
			self.context.fill_rect(f64::from(rect.x), f64::from(rect.y), f64::from(rect.width), f64::from(rect.height));
			return;
		}
		self.rounded_path(rect, radius);
		self.context.fill();
	}

	fn rounded_path(&self, rect: Rect, radius: f32) {
		let radius = f64::from(radius.min(rect.width / 2.0).min(rect.height / 2.0).max(0.0));
		let (x, y) = (f64::from(rect.x), f64::from(rect.y));
		let (right, bottom) = (f64::from(rect.right()), f64::from(rect.bottom()));
		let context = &self.context;
		context.begin_path();
		context.move_to(x + radius, y);
		let _ = context.arc_to(right, y, right, bottom, radius);
		let _ = context.arc_to(right, bottom, x, bottom, radius);
		let _ = context.arc_to(x, bottom, x, y, radius);
		let _ = context.arc_to(x, y, right, y, radius);
		context.close_path();
	}

	fn fill_color(&mut self, color: Color) {
		if self.state.fill == Some(color) { return; }
		let css = self.color(color);
		self.context.set_fill_style_str(&css);
		self.state.fill = Some(color);
	}

	fn color(&mut self, color: Color) -> String {
		self.colors.entry(color).or_insert_with(|| color.to_string()).clone()
	}

	fn sync_fonts(&mut self, list: &DisplayList, css: &dyn Fn(FontId) -> String) {
		for (index, font) in list.fonts.iter().enumerate().skip(self.fonts.len()) {
			self.fonts.push(css(FontId::from_index(index)));
			let hidden = self.hidden_families.iter().any(|f| font.uses_family(f));
			self.hidden.push(hidden);
		}
	}

	fn grid_scale(&self) -> f32 {
		match self.grid {
			PixelGrid::Device => self.surface.scale,
			PixelGrid::Css => 1.0,
		}
	}

	fn round(&self, value: f32) -> f32 {
		let scale = self.grid_scale();
		(value * scale).round() / scale
	}

	fn snap(&self, rect: Rect) -> Rect {
		let (left, top) = (self.round(rect.x), self.round(rect.y));
		Rect::new(left, top, self.round(rect.right()) - left, self.round(rect.bottom()) - top)
	}

	fn snap_out(&self, rect: Rect) -> Rect {
		let scale = self.surface.scale;
		let (left, top) = ((rect.x * scale).floor() / scale, (rect.y * scale).floor() / scale);
		let right = (rect.right() * scale).ceil() / scale;
		let bottom = (rect.bottom() * scale).ceil() / scale;
		Rect::new(left, top, right - left, bottom - top)
	}
}

fn overhang(list: &DisplayList) -> f32 {
	list.fonts.iter().map(|f| f.size).fold(0.0f32, f32::max) * OVERHANG_PER_FONT_SIZE
}

fn inflate(rect: Rect, by: f32) -> Rect {
	Rect::new(rect.x - by, rect.y - by, rect.width + 2.0 * by, rect.height + 2.0 * by)
}