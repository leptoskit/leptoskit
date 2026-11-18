//! Drawing on an editor's drawings with the pointer.
//!
//! canvist stores and lays out a drawing and leaves the pointer to its host:
//! the host follows a press over the drawing's rectangle and adds the
//! finished stroke as one change. While the pointer is down, the stroke so
//! far is appended to each frame's display list, so the painter draws it,
//! and repaints only what it touched, exactly as it does a stored one.

use canvist_extension_ink::INK_WIDTH;
use canvist_extension_ink::InkStroke;
use canvist_headless::canvist_model::BlockId;
use canvist_layout::Color;
use canvist_layout::DisplayItem;
use canvist_layout::DisplayList;
use canvist_layout::Rect;

use super::Editor;
use crate::CanvistError;
use crate::Change;
use crate::session::Session;
use crate::web::theme::INK_STROKE;

/// A move shorter than this, in drawing units, adds no point. A drawing is
/// 1000 units across, so this is under a pixel on most screens, and keeps a
/// slow stroke from storing hundreds of points on one spot.
const SHORTEST_STEP: f32 = 1.0;

/// A stroke the pointer is drawing: the drawing it is on, and its points so
/// far as `x, y` pairs in that drawing's units.
#[derive(Clone, Debug)]
pub(super) struct Scribble {
	block: BlockId,
	points: Vec<f32>,
}

impl Editor {
	/// The drawing at document coordinates `(x, y)`, with its rectangle.
	fn drawing_at(&self, session: &Session, (x, y): (f32, f32)) -> Option<(BlockId, Rect)> {
		let ink = session.ink()?;

		ink.drawings(session.doc.state())
			.into_iter()
			.filter_map(|block| Some((block, self.layout.block_rect(block)?)))
			.find(|(_, rect)| {
				rect.width > 0.0
					&& (rect.x..=rect.x + rect.width).contains(&x)
					&& (rect.y..=rect.y + rect.height).contains(&y)
			})
	}

	/// Whether a press at `point` would draw: it is on a drawing, and
	/// people may change the document.
	pub(crate) fn draws_at(&self, session: &Session, point: (f32, f32)) -> bool {
		!session.read_only && self.drawing_at(session, point).is_some()
	}

	/// Whether the pointer is drawing a stroke.
	pub(crate) fn is_scribbling(&self) -> bool {
		self.scribble.is_some()
	}

	/// Press the pointer at `point`. Returns whether that started a stroke,
	/// which it does on a drawing unless the editor is read-only.
	pub(crate) fn start_scribble(&mut self, session: &Session, point: (f32, f32)) -> bool {
		if session.read_only {
			return false;
		}

		let Some((block, rect)) = self.drawing_at(session, point) else {
			return false;
		};
		let (x, y) = in_drawing(rect, point);
		self.scribble = Some(Scribble {
			block,
			points: vec![x, y],
		});

		true
	}

	/// Move the pressed pointer to `point`. Returns whether a stroke is
	/// being drawn, whether or not the move was long enough to add to it.
	pub(crate) fn extend_scribble(&mut self, point: (f32, f32)) -> bool {
		let Some(rect) = self
			.scribble
			.as_ref()
			.and_then(|scribble| self.layout.block_rect(scribble.block))
			.filter(|rect| rect.width > 0.0)
		else {
			return self.scribble.is_some();
		};
		let (x, y) = in_drawing(rect, point);

		if let Some(scribble) = &mut self.scribble
			&& let [.., last_x, last_y] = scribble.points[..]
			&& (x - last_x).hypot(y - last_y) >= SHORTEST_STEP
		{
			scribble.points.extend([x, y]);
		}

		true
	}

	/// Release the pointer: the stroke joins its drawing as one change, and
	/// one step of undo. An editor that became read-only while the pointer
	/// was down keeps the stroke out, for the caller to drop.
	pub(crate) fn finish_scribble(
		&mut self,
		session: &mut Session,
	) -> Result<Option<Change>, CanvistError> {
		if session.read_only {
			return Ok(None);
		}

		let (Some(scribble), Some(ink)) = (self.scribble.take(), session.ink()) else {
			return Ok(None);
		};
		let color = session.pen.hex();
		let stroke = InkStroke {
			points: &scribble.points,
			color: color.as_deref(),
			width: session.pen.units(),
		};
		let command = ink
			.add_stroke(scribble.block, &stroke)
			.map_err(|error| crate::session::engine_error(&error))?;

		self.change(session, &mut |engine, env| engine.run(&command, env))
	}

	/// Drop the stroke being drawn, as when the browser takes the pointer
	/// away. Returns whether there was one.
	pub(crate) fn cancel_scribble(&mut self) -> bool {
		self.scribble.take().is_some()
	}

	/// Have layout read the strokes of the drawings a commit `touched`.
	///
	/// canvist's layout reads a drawing's strokes when it lists the
	/// document's blocks, and accounting for a commit does not list a block
	/// again when only its attributes changed, which is all a stroke
	/// changes. Listing them all again keeps every block's lines, so it
	/// costs a walk over the document: once a stroke, never per pointer
	/// move.
	pub(super) fn reread_drawings(&mut self, session: &Session, touched: &[BlockId]) {
		let document = session.doc.document();
		let Some(ink) = document.schema().block_type("ink") else {
			return;
		};
		let drawn = touched
			.iter()
			.filter_map(|block| document.block(*block))
			.any(|block| block.kind() == ink);

		if drawn {
			self.layout.resync(document);
		}
	}

	/// `list` with the stroke being drawn on top, as the drawing will show
	/// it once it is finished.
	pub(super) fn with_scribble(&self, session: &Session, mut list: DisplayList) -> DisplayList {
		let Some((scribble, rect)) = self
			.scribble
			.as_ref()
			.and_then(|scribble| Some((scribble, self.layout.block_rect(scribble.block)?)))
		else {
			return list;
		};
		let scale = rect.width / INK_WIDTH;
		let width = session.pen.units().unwrap_or(INK_STROKE) * scale;
		let color = session.pen.rgb().map_or_else(
			|| self.layout.theme().base().color,
			|[red, green, blue]| Color::rgb(red, green, blue),
		);
		let start = list.points.len() as u32;
		let (mut left, mut top, mut right, mut bottom) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);

		for [x, y] in scribble.points.as_chunks::<2>().0 {
			let (x, y) = (rect.x + x * scale, rect.y + y * scale);
			left = left.min(x);
			right = right.max(x);
			top = top.min(y);
			bottom = bottom.max(y);
			list.points.extend([x, y]);
		}

		let half = width / 2.0;
		list.items.push(DisplayItem::Stroke {
			points: start..list.points.len() as u32,
			width,
			color,
			// The area the stroke paints, with its round ends.
			bounds: Rect::new(
				left - half,
				top - half,
				right - left + width,
				bottom - top + width,
			),
			block: scribble.block,
		});

		list
	}
}

/// Document coordinates `(x, y)` in the units of the drawing that fills
/// `rect`, kept inside it: a stroke dragged past an edge runs along it.
fn in_drawing(rect: Rect, (x, y): (f32, f32)) -> (f32, f32) {
	let scale = rect.width / INK_WIDTH;

	(
		((x - rect.x) / scale).clamp(0.0, INK_WIDTH),
		((y - rect.y) / scale).clamp(0.0, rect.height / scale),
	)
}
