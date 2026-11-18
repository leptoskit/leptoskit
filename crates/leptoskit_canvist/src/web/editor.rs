//! One editor on one canvas: a document laid out by `canvist_layout` and
//! painted on an opaque 2D canvas context.
//!
//! A frame is cheap when little changed: layout relays out only the blocks
//! a commit touched, the display list covers only the viewport, and the
//! painter repaints only the rectangles that differ from the last frame.
//!
//! Everything here is told what happened in the page and never looks: the
//! [host](super::host) owns the DOM, and calls in with keys, pointer
//! positions and sizes. Ported from canvist's `crates/canvist_web/src/lib.rs`
//! and `input.rs`.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use canvist_headless::Doc;
use canvist_headless::canvist_core::CommandResult;
use canvist_headless::canvist_core::Editor as Engine;
use canvist_headless::canvist_core::Env;
use canvist_headless::canvist_core::KeyPress;
use canvist_headless::canvist_core::Modifiers;
use canvist_headless::canvist_core::Update;
use canvist_headless::canvist_core::command;
use canvist_headless::canvist_core::commands;
use canvist_headless::canvist_core::position;
use canvist_headless::canvist_model::InlineText;
use canvist_headless::canvist_model::Point;
use canvist_headless::canvist_model::Selection;
use canvist_layout::Caret;
use canvist_layout::DisplayList;
use canvist_layout::DisplayOptions;
use canvist_layout::DocumentLayout;
use canvist_layout::LayoutOptions;
use canvist_layout::Rect;
use canvist_layout::Theme;
use wasm_bindgen::JsValue;
use web_sys::HtmlCanvasElement;

use super::lines::HostLines;
use super::measure::DomMeasurer;
use super::paint::Painter;
use super::paint::PixelGrid;
use super::paint::Region;
use super::paint::Surface;
use crate::CanvistError;
use crate::Change;
use crate::session::Session;
use crate::session::engine_error;
use crate::text::word_around;

/// More dirty rectangles than this repaint the whole canvas instead: past
/// a handful, clipping costs more than it saves.
const MAX_DIRTY_RECTS: usize = 16;

/// Clicks that select a word; more select the whole paragraph.
const WORD_CLICKS: u32 = 2;

/// What one [`Editor::frame`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Frame {
	/// The document's height in CSS pixels, including estimates for blocks
	/// not laid out yet. The host sizes its scroll area to it.
	pub(crate) height: f32,
	/// How far content at the top of the viewport moved because blocks above
	/// it were measured. Add it to the scroll position to keep the content
	/// still.
	pub(crate) shift: f32,
	/// Display items drawn.
	pub(crate) drawn: u32,
	/// Whether the whole canvas was repainted.
	pub(crate) full: bool,
	/// Whether anything was painted at all.
	pub(crate) painted: bool,
}

/// What a pointer drag extends the selection by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Unit {
	Character,
	Word,
	Block,
}

/// A pointer drag in progress: the range the press selected, which every
/// later position extends from.
#[derive(Clone, Copy, Debug)]
struct Drag {
	unit: Unit,
	from: Point,
	to: Point,
}

/// How the page the editor is in lays text out, which layout has to match.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Tailoring {
	/// Pixels beyond the viewport to lay out ahead of scrolling.
	pub(crate) overscan: f32,
	/// Whether lines break by Blink's rules instead of ICU's defaults.
	pub(crate) chromium_breaks: bool,
}

/// The layout, the measurer and the painter of one editor.
pub(crate) struct Editor {
	tailoring: Tailoring,
	layout: DocumentLayout,
	measurer: DomMeasurer,
	painter: Painter,
	/// What the canvas shows now, for computing the next frame's damage.
	painted: Option<DisplayList>,
	/// Damage that the display list diff cannot see, such as glyphs that
	/// changed shape when a font loaded: repainted on the next frame.
	dirty: Vec<Rect>,
	full_repaint: bool,
	/// The document coordinate at the canvas's top edge.
	top: f32,
	/// Whether the caret is shown at all (the editor has focus and takes
	/// typing) and whether it is in the visible half of its blink.
	focused: bool,
	caret_on: bool,
	/// The pointer drag selecting text, while the pointer is pressed.
	drag: Option<Drag>,
}

impl Editor {
	/// An editor painting `doc` on `canvas` in `theme`, measuring with probes
	/// laid out in `document`.
	pub(crate) fn new(
		canvas: HtmlCanvasElement,
		document: &web_sys::Document,
		doc: &Doc,
		theme: Arc<Theme>,
		tailoring: Tailoring,
	) -> Result<Self, JsValue> {
		Ok(Self {
			tailoring,
			layout: DocumentLayout::new(doc.document(), theme, layout_options(tailoring, 0.0)),
			measurer: DomMeasurer::new(document)?,
			painter: Painter::new(canvas)?,
			painted: None,
			dirty: Vec::new(),
			full_repaint: true,
			top: 0.0,
			focused: false,
			caret_on: true,
			drag: None,
		})
	}

	/// The session's document was swapped for `doc`: lay it out from scratch
	/// on the next frame.
	pub(crate) fn replace(&mut self, doc: &Doc) {
		// Keep what the host told the old layout about the screen.
		let LayoutOptions {
			width,
			border_scale,
			..
		} = *self.layout.options();
		let mut options = layout_options(self.tailoring, width);
		options.border_scale = border_scale;
		self.layout = DocumentLayout::new(doc.document(), Arc::clone(self.layout.theme()), options);
		self.painted = None;
		self.drag = None;
		self.full_repaint = true;
	}

	/// Draw in `theme` from the next frame on. Measurements are kept.
	pub(crate) fn set_theme(&mut self, doc: &Doc, theme: Arc<Theme>) {
		self.layout.set_theme(doc.document(), theme);
		self.full_repaint = true;
	}

	/// Size the canvas. The document is laid out to the surface's width.
	pub(crate) fn resize(&mut self, doc: &Doc, surface: Surface) {
		if self.painter.resize(surface) {
			self.full_repaint = true;
		}

		if surface.width.to_bits() != self.layout.options().width.to_bits() {
			self.layout.set_width(doc.document(), surface.width);
			self.full_repaint = true;
		}
	}

	/// Scroll so document coordinate `top` is at the canvas's top edge.
	pub(crate) fn set_top(&mut self, top: f32) {
		if top.to_bits() != self.top.to_bits() {
			self.top = top;
			self.full_repaint = true;
		}
	}

	/// Show or hide the caret.
	pub(crate) fn set_focused(&mut self, focused: bool) {
		self.focused = focused;
		self.caret_on = true;
	}

	/// Blink the caret.
	pub(crate) fn set_caret_on(&mut self, on: bool) {
		self.caret_on = on;
	}

	/// Lay out the viewport and paint whatever changed since the last frame.
	pub(crate) fn frame(&mut self, session: &Session) -> Result<Frame, CanvistError> {
		let surface = self.painter.surface();
		let bottom = self.top + surface.height;
		let update = self
			.layout
			.ensure(session.doc.document(), &mut self.measurer, self.top, bottom)
			.map_err(|error| engine_error(&error))?;
		let list = self
			.layout
			.display_list(&self.display_options(session, bottom));
		let mut frame = Frame {
			height: self.layout.height(),
			shift: update.shift,
			..Frame::default()
		};

		let full = self.full_repaint || update.shift != 0.0;
		let damage = self.damage(&list, full);

		if full || damage.len() > MAX_DIRTY_RECTS {
			self.paint(&list, &Region::All);
			frame.full = true;
			frame.painted = true;
		} else if !damage.is_empty() {
			self.paint(&list, &Region::Rects(&damage));
			frame.painted = true;
		}

		frame.drawn = self.painter.drawn();
		self.dirty.clear();
		self.full_repaint = false;
		self.painted = Some(list);
		Ok(frame)
	}

	/// The fonts measured since the last call, as CSS `font` shorthands,
	/// with the text measured in each.
	pub(crate) fn take_used_fonts(&mut self) -> BTreeMap<String, String> {
		self.measurer.take_used()
	}

	/// Font faces of `families` finished loading: forget only the
	/// measurements of fonts that list one of them, so the blocks using them
	/// lay out again on the next frame. Returns how many measured blocks
	/// were invalidated.
	pub(crate) fn fonts_loaded(&mut self, families: &[String]) -> usize {
		self.measurer.forget();
		let mut invalidation = self
			.layout
			.forget_fonts(|font| families.iter().any(|family| font.uses_family(family)));
		self.dirty.append(&mut invalidation.rects);
		// A family list may now resolve to another face for painting too.
		self.painter.forget_fonts();
		self.full_repaint |= invalidation.fonts > 0;
		invalidation.blocks
	}

	/// Snap text baselines and rectangle edges to the grid the browser snaps
	/// DOM content to.
	pub(crate) fn set_pixel_grid(&mut self, grid: PixelGrid) {
		if self.painter.set_grid(grid) {
			self.full_repaint = true;
		}
	}

	/// Snap border widths down to `scale` device pixels per CSS pixel, as
	/// this browser draws them, or not at all.
	pub(crate) fn set_border_scale(&mut self, doc: &Doc, scale: Option<f32>) {
		if scale.map(f32::to_bits) != self.layout.options().border_scale.map(f32::to_bits) {
			self.layout.set_border_scale(doc.document(), scale);
			self.full_repaint = true;
		}
	}

	/// Forget every measurement, as when the device pixel ratio changes:
	/// browsers round font metrics to device pixels, so line positions can
	/// move. Blocks lay out again as they come into view.
	pub(crate) fn forget_measurements(&mut self) {
		self.measurer.forget();
		self.layout.forget_measurements();
		self.painter.forget_fonts();
		self.full_repaint = true;
	}

	/// Where the caret is, or would be at the moving end of a range, in
	/// document coordinates. The host scrolls it into view and places the
	/// text field there, so input methods open their windows beside it.
	pub(crate) fn caret_rect(&self, doc: &Doc) -> Option<Rect> {
		let Selection::Text { head, .. } = doc.selection() else {
			return None;
		};
		let caret = Caret::with_affinity(head, doc.state().affinity().into());

		self.layout.caret_rect(caret)
	}

	/// Run the key binding for `key`, a DOM `KeyboardEvent.key`. `None` when
	/// no binding applied, and the browser should handle the key.
	pub(crate) fn key_down(
		&mut self,
		session: &mut Session,
		key: &str,
		modifiers: Modifiers,
	) -> Result<Option<Change>, CanvistError> {
		let press = KeyPress::new(key, modifiers);
		self.change(session, &mut |engine, env| engine.handle_key(press, env))
	}

	/// Replace the selection with `text`, as typing does.
	pub(crate) fn insert_text(
		&mut self,
		session: &mut Session,
		text: &str,
	) -> Result<Option<Change>, CanvistError> {
		self.change(session, &mut |engine, env| engine.insert_text(text, env))
	}

	/// Show `text` as an input method's unfinished text at the selection,
	/// replacing any earlier unfinished text.
	pub(crate) fn set_composition(
		&mut self,
		session: &mut Session,
		text: &str,
	) -> Result<Option<Change>, CanvistError> {
		self.change(session, &mut |engine, env| {
			engine.set_composition(text, env)
		})
	}

	/// Finish the input method's text as `text`.
	pub(crate) fn commit_composition(
		&mut self,
		session: &mut Session,
		text: &str,
	) -> Result<Option<Change>, CanvistError> {
		self.change(session, &mut |engine, env| {
			engine.commit_composition(text, env)
		})
	}

	/// Undo the last change, or redo the last one undone.
	pub(crate) fn history(
		&mut self,
		session: &mut Session,
		redo: bool,
	) -> Result<Option<Change>, CanvistError> {
		if redo {
			self.change(session, &mut Engine::redo)
		} else {
			self.change(session, &mut Engine::undo)
		}
	}

	/// Press the pointer at document coordinates `(x, y)`. One click places
	/// the caret, or extends the selection with `extend`; two select a word
	/// and three the whole paragraph, and dragging afterwards extends by
	/// that unit.
	pub(crate) fn pointer_down(
		&mut self,
		session: &mut Session,
		(x, y): (f32, f32),
		extend: bool,
		clicks: u32,
	) -> Result<Option<Change>, CanvistError> {
		let Some(point) = self.point_at(x, y) else {
			return Ok(None);
		};
		let unit = match clicks {
			0 | 1 => Unit::Character,
			WORD_CLICKS => Unit::Word,
			_ => Unit::Block,
		};
		let (from, to) = unit_around(&session.doc, point, unit);

		if extend && unit == Unit::Character {
			let (anchor, _) = selection_points(&session.doc);
			self.drag = Some(Drag {
				unit,
				from: anchor,
				to: anchor,
			});
			return self.extend_to(session, point);
		}

		self.drag = Some(Drag { unit, from, to });
		self.select(session, Selection::text(from, to))
	}

	/// Move a pressed pointer to `(x, y)`, extending the selection from the
	/// press.
	pub(crate) fn pointer_move(
		&mut self,
		session: &mut Session,
		(x, y): (f32, f32),
	) -> Result<Option<Change>, CanvistError> {
		match self.point_at(x, y) {
			Some(point) if self.drag.is_some() => self.extend_to(session, point),
			_ => Ok(None),
		}
	}

	/// Release the pointer.
	pub(crate) fn pointer_up(&mut self) {
		self.drag = None;
	}

	/// Cut the selection, returning its text as [`copy`] does with the
	/// change that removed it.
	pub(crate) fn cut(
		&mut self,
		session: &mut Session,
	) -> Result<Option<(String, Option<Change>)>, CanvistError> {
		let Some(text) = copy(&session.doc)? else {
			return Ok(None);
		};
		let change = self.change(session, &mut |engine, env| {
			engine.run(&command(commands::delete_selection), env)
		})?;

		Ok(Some((text, change)))
	}

	/// Paste plain text over the selection: one paragraph per line.
	pub(crate) fn paste_text(
		&mut self,
		session: &mut Session,
		text: &str,
	) -> Result<Option<Change>, CanvistError> {
		self.change(session, &mut |engine, env| engine.paste_text(text, env))
	}

	fn select(
		&mut self,
		session: &mut Session,
		selection: Selection,
	) -> Result<Option<Change>, CanvistError> {
		self.change(session, &mut |engine, env| engine.select(selection, env))
	}

	/// Extend the dragged selection to cover `point` by the drag's unit.
	fn extend_to(
		&mut self,
		session: &mut Session,
		point: Point,
	) -> Result<Option<Change>, CanvistError> {
		let Some(drag) = self.drag else {
			return Ok(None);
		};
		let (start, end) = unit_around(&session.doc, point, drag.unit);
		let before =
			position::compare_points(session.doc.document(), point, drag.from) == Ordering::Less;
		let selection = if before {
			Selection::text(drag.to, start)
		} else {
			Selection::text(drag.from, end)
		};

		self.select(session, selection)
	}

	/// Make a change through the engine with the wall clock and this
	/// layout's line navigator, let layout account for it, and say what
	/// changed.
	///
	/// It takes the change as a `dyn` closure so the many input methods share
	/// one copy of this in the Wasm module.
	fn change(
		&mut self,
		session: &mut Session,
		change: &mut dyn FnMut(&mut Engine, &mut Env<'_>) -> CommandResult,
	) -> Result<Option<Change>, CanvistError> {
		let now = Duration::from_secs_f64(js_sys::Date::now() / 1000.0);
		let mut lines = HostLines::new(&mut self.layout, &mut self.measurer);
		let update: Option<Update> = session
			.doc
			.edit(now, &mut lines, |engine, env| change(engine, env))
			.map_err(|error| engine_error(&error))?;

		let Some(update) = update else {
			return Ok(None);
		};

		let damage = self.layout.apply(session.doc.document(), &update.commit);

		if damage.from_y.is_some() {
			self.full_repaint = true;
		}

		self.caret_on = true;
		Ok(session.record(&update))
	}

	/// The model point at document coordinates `(x, y)`.
	fn point_at(&self, x: f32, y: f32) -> Option<Point> {
		self.layout.hit_test(x, y).map(|caret| caret.point)
	}

	/// What to draw in the viewport ending at `bottom`: a collapsed selection
	/// is a caret, drawn only while the editor has focus and the caret is in
	/// the visible half of its blink; any other selection is highlighted. An
	/// empty document shows the placeholder.
	fn display_options(&self, session: &Session, bottom: f32) -> DisplayOptions {
		let mut options = DisplayOptions::viewport(self.top, bottom);

		if let ([block], false, true) = (
			session.doc.document().children(None),
			session.placeholder.is_empty(),
			session.is_empty(),
		) {
			options = options.placeholder(*block, &session.placeholder);
		}

		let Some(caret) = caret(&session.doc) else {
			return options.selection(session.doc.selection());
		};

		if self.focused && self.caret_on {
			options.caret(caret)
		} else {
			options
		}
	}

	/// The rectangles that differ between the canvas and `list`, plus damage
	/// the diff cannot see. Empty when the whole canvas repaints anyway.
	fn damage(&mut self, list: &DisplayList, full: bool) -> Vec<Rect> {
		let Some(previous) = self.painted.as_ref().filter(|_| !full) else {
			return Vec::new();
		};

		let mut damage = list.damage(previous);
		damage.append(&mut self.dirty);

		damage
	}

	/// Paint `region` of `list` at the current scroll position.
	fn paint(&mut self, list: &DisplayList, region: &Region<'_>) {
		let measurer = &mut self.measurer;
		let background = self.layout.theme().chrome().background;

		self.painter
			.paint(list, self.top, background, region, &mut |id, css| {
				measurer.paint_font(id, css)
			});
	}
}

/// The selection as plain text for the clipboard, one line per paragraph,
/// or `None` when nothing is selected.
pub(crate) fn copy(doc: &Doc) -> Result<Option<String>, CanvistError> {
	let collapsed =
		position::text_range(doc.document(), doc.selection()).is_none_or(|range| range.is_empty());

	if collapsed {
		return Ok(None);
	}

	let slice = doc.editor().copy().map_err(|error| engine_error(&error))?;

	Ok(Some(canvist_extension_clipboard::to_plain_text(
		doc.document().schema(),
		&slice,
	)))
}

/// The selection as a caret, when it is one.
fn caret(doc: &Doc) -> Option<Caret> {
	match doc.selection() {
		Selection::Text { anchor, head } if anchor == head => {
			Some(Caret::with_affinity(head, doc.state().affinity().into()))
		}
		_ => None,
	}
}

/// The anchor and head of a text selection, or the document's start.
fn selection_points(doc: &Doc) -> (Point, Point) {
	match doc.selection() {
		Selection::Text { anchor, head } => (anchor, head),
		selection @ Selection::Blocks { .. } => {
			let document = doc.document();
			let start = position::text_range(document, selection)
				.map(|range| range.from)
				.or_else(|| {
					match position::start_selection(document) {
						Selection::Text { head, .. } => Some(head),
						Selection::Blocks { .. } => None,
					}
				});
			let point = start.unwrap_or(Point::new(document.children(None)[0], 0));
			(point, point)
		}
	}
}

/// The range `unit` covers around `point`.
fn unit_around(doc: &Doc, point: Point, unit: Unit) -> (Point, Point) {
	let Some(text) = doc.document().inline(point.block).map(InlineText::as_str) else {
		return (point, point);
	};

	match unit {
		Unit::Character => (point, point),
		Unit::Word => {
			let (start, end) = word_around(text, point.offset);
			(Point::new(point.block, start), Point::new(point.block, end))
		}
		Unit::Block => {
			(
				Point::new(point.block, 0),
				Point::new(point.block, text.len() as u32),
			)
		}
	}
}

fn layout_options(tailoring: Tailoring, width: f32) -> LayoutOptions {
	let layout = LayoutOptions::new(width).overscan(tailoring.overscan);

	if tailoring.chromium_breaks {
		layout.break_override(canvist_layout::chromium_breaks)
	} else {
		layout
	}
}
