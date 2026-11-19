//! Keyboard and mouse routing into the shared event model.

use leptoskit_native::NativeEvent;
use leptoskit_native::NativeKind;
use leptoskit_yoga::NodeId;

use crate::backend::TuiBackend;
use crate::focus::nearest_focusable;
use crate::focus::path_active;
use crate::text::CELL_HEIGHT;
use crate::text::CELL_WIDTH;

/// A decoded key press.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Key {
	/// The key.
	pub code: KeyCode,
	/// Whether a control modifier was held.
	pub ctrl: bool,
	/// Whether an alt or option modifier was held.
	pub alt: bool,
}

impl Key {
	/// A plain key press with no modifiers.
	pub const fn plain(code: KeyCode) -> Self {
		Self {
			code,
			ctrl: false,
			alt: false,
		}
	}

	/// A key press with control held.
	pub const fn controlled(code: KeyCode) -> Self {
		Self {
			code,
			ctrl: true,
			alt: false,
		}
	}
}

/// A terminal key as the shared [`KeyPress`](leptoskit_native::KeyPress)
/// the application hears, named the way the web names keys. Keys the
/// terminal could not decode have no name and are not offered.
#[must_use]
pub fn key_press(key: Key) -> Option<leptoskit_native::KeyPress> {
	let (name, shift) = match key.code {
		KeyCode::Char(symbol) => (symbol.to_string(), symbol.is_uppercase()),
		KeyCode::Enter => ("Enter".to_owned(), false),
		KeyCode::Tab => ("Tab".to_owned(), false),
		KeyCode::BackTab => ("Tab".to_owned(), true),
		KeyCode::Backspace => ("Backspace".to_owned(), false),
		KeyCode::Delete => ("Delete".to_owned(), false),
		KeyCode::Left => ("ArrowLeft".to_owned(), false),
		KeyCode::Right => ("ArrowRight".to_owned(), false),
		KeyCode::Up => ("ArrowUp".to_owned(), false),
		KeyCode::Down => ("ArrowDown".to_owned(), false),
		KeyCode::Home => ("Home".to_owned(), false),
		KeyCode::End => ("End".to_owned(), false),
		KeyCode::PageUp => ("PageUp".to_owned(), false),
		KeyCode::PageDown => ("PageDown".to_owned(), false),
		KeyCode::Escape => ("Escape".to_owned(), false),
		KeyCode::Other => return None,
	};

	Some(leptoskit_native::KeyPress {
		key: name,
		ctrl: key.ctrl,
		alt: key.alt,
		shift,
		// A terminal is never told about the command key.
		meta: false,
	})
}

/// The keys the backend routes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyCode {
	/// A character key, including space.
	Char(char),
	/// Return or enter.
	Enter,
	/// Tab.
	Tab,
	/// Shift-tab.
	BackTab,
	/// Backspace.
	Backspace,
	/// Forward delete.
	Delete,
	/// The left arrow.
	Left,
	/// The right arrow.
	Right,
	/// The up arrow.
	Up,
	/// The down arrow.
	Down,
	/// Home.
	Home,
	/// End.
	End,
	/// Page up.
	PageUp,
	/// Page down.
	PageDown,
	/// Escape.
	Escape,
	/// A key the backend does not route, which every widget ignores.
	Other,
}

/// A mouse event in the cell grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mouse {
	/// A primary click at a cell.
	Click {
		/// The column.
		col: u16,
		/// The row.
		row: u16,
	},
	/// The wheel scrolled up at a cell.
	ScrollUp {
		/// The column.
		col: u16,
		/// The row.
		row: u16,
	},
	/// The wheel scrolled down at a cell.
	ScrollDown {
		/// The column.
		col: u16,
		/// The row.
		row: u16,
	},
}

impl Mouse {
	/// The click's column.
	pub const fn col(self) -> u16 {
		match self {
			Self::Click { col, .. } | Self::ScrollUp { col, .. } | Self::ScrollDown { col, .. } => {
				col
			}
		}
	}

	/// The click's row.
	pub const fn row(self) -> u16 {
		match self {
			Self::Click { row, .. } | Self::ScrollUp { row, .. } | Self::ScrollDown { row, .. } => {
				row
			}
		}
	}
}

/// One shared event the backend asks the host to dispatch.
#[derive(Debug, Clone, PartialEq)]
pub struct Dispatch {
	/// The target node.
	pub node: NodeId,
	/// The event.
	pub event: NativeEvent,
}

/// Routes one key through the focused widget and the focus ring.
///
/// Tab and shift-tab always move focus, even inside a text field, so the ring
/// stays predictable in a keyboard-first terminal. Everything else belongs to
/// the focused widget first: fields edit their drafts, controls activate,
/// scroll views scroll.
pub(crate) fn handle_key(backend: &mut TuiBackend, key: Key) -> Vec<Dispatch> {
	if !key.ctrl && !key.alt {
		match key.code {
			KeyCode::Tab => return tab(backend, 1),
			KeyCode::BackTab => return tab(backend, -1),
			// Escape asks the overlay on top to close.
			KeyCode::Escape => {
				if let Some(&node) = backend.open_overlays().last() {
					return vec![Dispatch {
						node,
						event: NativeEvent::Dismiss,
					}];
				}
			}
			_ => {}
		}
	}

	let Some(focused) = backend.focused() else {
		return Vec::new();
	};

	let Some(node) = backend.node(focused) else {
		return Vec::new();
	};

	match node.kind {
		NativeKind::TextInput | NativeKind::TextArea | NativeKind::PasswordInput => {
			edit_field(backend, focused, key)
		}
		NativeKind::Button | NativeKind::Pressable => activate(focused, key),
		NativeKind::Switch | NativeKind::Checkbox => toggle(focused, node.props.checked, key),
		NativeKind::Radio => radio_key(backend, focused, key),
		NativeKind::Slider => slider_key(backend, focused, key),
		NativeKind::ScrollView => scroll_key(backend, focused, key),
		_ => Vec::new(),
	}
}

/// Tab or shift-tab: moves focus, unless that would carry it out of the
/// popover on top, which closes instead. Closing puts focus back where it
/// was before the popover opened.
fn tab(backend: &mut TuiBackend, step: i32) -> Vec<Dispatch> {
	let leaving = backend.focused().and_then(|focused| {
		let top = *backend.open_overlays().last()?;
		let popover = backend.node(top)?.kind == NativeKind::Popover;
		let stays = next_focus(backend, step).is_some_and(|next| backend.within(next, top));

		(popover && backend.within(focused, top) && !stays).then_some(top)
	});

	match leaving {
		Some(node) => {
			vec![Dispatch {
				node,
				event: NativeEvent::Dismiss,
			}]
		}
		None => move_focus(backend, step),
	}
}

/// Where focus lands `step` positions along the ring, wrapping at the ends.
fn next_focus(backend: &TuiBackend, step: i32) -> Option<NodeId> {
	let ring = backend.focusables();

	if ring.is_empty() {
		return None;
	}

	let next = match backend
		.focused()
		.and_then(|current| ring.iter().position(|&id| id == current))
	{
		Some(index) => {
			let len = ring.len();
			let moved = (index as i64 + i64::from(step)).rem_euclid(len as i64) as usize;
			ring[moved]
		}
		// With nothing focused, forward starts at the first focusable and
		// backward at the last, like a browser's first shift-tab.
		None if step >= 0 => ring[0],
		None => ring[ring.len() - 1],
	};

	Some(next)
}

/// Moves focus by `step` positions through the ring, wrapping at the ends.
pub(crate) fn move_focus(backend: &mut TuiBackend, step: i32) -> Vec<Dispatch> {
	let next = next_focus(backend, step);

	backend.set_focus(next)
}

/// Enter or space activates a button or pressable.
fn activate(id: NodeId, key: Key) -> Vec<Dispatch> {
	if matches!(key.code, KeyCode::Enter | KeyCode::Char(' ')) {
		vec![Dispatch {
			node: id,
			event: NativeEvent::Press,
		}]
	} else {
		Vec::new()
	}
}

/// Enter or space proposes the inverse state, like a tap.
fn toggle(id: NodeId, checked: Option<bool>, key: Key) -> Vec<Dispatch> {
	if matches!(key.code, KeyCode::Enter | KeyCode::Char(' ')) {
		vec![Dispatch {
			node: id,
			event: NativeEvent::Toggle {
				checked: !checked.unwrap_or(false),
			},
		}]
	} else {
		Vec::new()
	}
}

/// Radios select on activate; arrows walk their group, wrapping.
fn radio_key(backend: &mut TuiBackend, id: NodeId, key: Key) -> Vec<Dispatch> {
	match key.code {
		KeyCode::Enter | KeyCode::Char(' ') => {
			vec![Dispatch {
				node: id,
				event: NativeEvent::Toggle { checked: true },
			}]
		}
		KeyCode::Up | KeyCode::Left => step_radio_group(backend, id, -1),
		KeyCode::Down | KeyCode::Right => step_radio_group(backend, id, 1),
		_ => Vec::new(),
	}
}

fn step_radio_group(backend: &mut TuiBackend, id: NodeId, step: i64) -> Vec<Dispatch> {
	let Some(group) = backend.node(id).and_then(|node| {
		node.props
			.radio_group
			.as_ref()
			.map(|group| group.name().to_owned())
	}) else {
		return move_focus(backend, i32::try_from(step).unwrap_or(1));
	};

	let members: Vec<NodeId> = collect_group(backend, &group);
	let next = members
		.iter()
		.position(|&member| member == id)
		.map(|index| {
			let len = members.len();
			members[(index as i64 + step).rem_euclid(len as i64) as usize]
		})
		.or(members.first().copied());

	backend.set_focus(next)
}

/// Every radio in a group, in document order; the resolver guarantees one
/// checked member per group, and the tree guarantees groups are tree-scoped.
fn collect_group(backend: &TuiBackend, group: &str) -> Vec<NodeId> {
	let mut members = Vec::new();
	let Some(root) = backend.root() else {
		return members;
	};

	let mut pending = vec![root];

	while let Some(id) = pending.pop() {
		let Some(node) = backend.node(id) else {
			continue;
		};

		if node.kind == NativeKind::Radio
			&& node
				.props
				.radio_group
				.as_ref()
				.is_some_and(|candidate| candidate.name() == group)
		{
			members.push(id);
		}

		pending.extend(node.children.iter().rev().copied());
	}

	members
}

/// Arrows step a slider by its step, or a twentieth of its span when
/// continuous; home and end go to the bounds. The shared surface snaps the
/// proposal onto the configured grid.
fn slider_key(backend: &mut TuiBackend, id: NodeId, key: Key) -> Vec<Dispatch> {
	let Some(node) = backend.node(id) else {
		return Vec::new();
	};

	let state = node.props.slider.unwrap_or_default();
	let span = state.max() - state.min();
	let step = state
		.step()
		.unwrap_or(if span > 0.0 { span / 20.0 } else { 1.0 });
	let value = state.value();

	let proposed = match key.code {
		KeyCode::Left | KeyCode::Down => value - step,
		KeyCode::Right | KeyCode::Up => value + step,
		KeyCode::Home => state.min(),
		KeyCode::End => state.max(),
		_ => return Vec::new(),
	};

	vec![Dispatch {
		node: id,
		event: NativeEvent::ValueChange {
			value: proposed.clamp(state.min(), state.max()),
		},
	}]
}

/// Arrows scroll by a cell, pages by a page, home and end by the whole span.
fn scroll_key(backend: &mut TuiBackend, id: NodeId, key: Key) -> Vec<Dispatch> {
	let page = backend.node(id).map_or(CELL_HEIGHT, |node| {
		(node.frame.height - CELL_HEIGHT).max(CELL_HEIGHT)
	});

	let (dx, dy) = match key.code {
		KeyCode::Left => (-CELL_WIDTH, 0.0),
		KeyCode::Right => (CELL_WIDTH, 0.0),
		KeyCode::Up => (0.0, -CELL_HEIGHT),
		KeyCode::Down => (0.0, CELL_HEIGHT),
		KeyCode::PageUp => (0.0, -page),
		KeyCode::PageDown => (0.0, page),
		KeyCode::Home => return scroll_to(backend, id, 0.0),
		KeyCode::End => return scroll_to(backend, id, f32::INFINITY),
		_ => (0.0, 0.0),
	};

	if dx == 0.0 && dy == 0.0 {
		return Vec::new();
	}

	backend
		.scroll_by(id, dx, dy)
		.map(|event| vec![Dispatch { node: id, event }])
		.unwrap_or_default()
}

/// Scrolls to an absolute offset, with `f32::INFINITY` meaning the far end.
fn scroll_to(backend: &mut TuiBackend, id: NodeId, y: f32) -> Vec<Dispatch> {
	let Some(extent) = backend.content_extent(id) else {
		return Vec::new();
	};

	let current = backend.scroll(id).unwrap_or_default();
	let max_y = (extent.1 - backend.node(id).map_or(0.0, |node| node.frame.height)).max(0.0);
	let target = if y.is_infinite() { max_y } else { y };

	if (current.y - target).abs() < f32::EPSILON {
		return Vec::new();
	}

	let _ = backend.scroll_by(id, 0.0, target - current.y);
	backend
		.scroll(id)
		.map(|scroll| {
			Dispatch {
				node: id,
				event: NativeEvent::Scroll {
					offset_x: scroll.x,
					offset_y: scroll.y,
				},
			}
		})
		.into_iter()
		.collect()
}

/// Edits a field's draft, returning the change or submit event.
///
/// Caret-only keys move within the draft and dispatch nothing.
fn edit_field(backend: &mut TuiBackend, id: NodeId, key: Key) -> Vec<Dispatch> {
	let Some(draft) = backend.drafts.get(id.index()).and_then(Option::as_ref) else {
		return Vec::new();
	};

	let props = backend
		.node(id)
		.map(|node| node.props.clone())
		.unwrap_or_default();

	// A read-only field refuses edits but stays selectable and copyable.
	if props.read_only == Some(true) {
		return Vec::new();
	}

	let max_length = props.max_length;
	let multiline = backend
		.node(id)
		.is_some_and(|node| node.kind == NativeKind::TextArea);

	let mut chars: Vec<char> = draft.value.chars().collect();
	let caret = draft.caret.min(chars.len());
	let mut caret = caret;

	let edited = match key.code {
		KeyCode::Char(ch) if !key.ctrl && !key.alt => {
			// Refuse the insertion past `max_length`, the way HTML's
			// `maxlength` refuses characters past its cap.
			if max_length.is_some_and(|max| chars.len() >= max) {
				return Vec::new();
			}
			chars.insert(caret, ch);
			caret += 1;
			true
		}

		KeyCode::Backspace if caret > 0 => {
			caret -= 1;
			chars.remove(caret);
			true
		}

		KeyCode::Delete if caret < chars.len() => {
			chars.remove(caret);
			true
		}

		// In a multi-line field Enter adds a line and never submits.
		KeyCode::Enter if multiline => {
			if max_length.is_some_and(|max| chars.len() >= max) {
				return Vec::new();
			}
			chars.insert(caret, '\n');
			caret += 1;
			true
		}

		KeyCode::Up | KeyCode::Down | KeyCode::Home | KeyCode::End if multiline => {
			set_caret(backend, id, line_caret(&chars, caret, key.code));
			return Vec::new();
		}

		KeyCode::Left => {
			set_caret(backend, id, caret.saturating_sub(1));
			return Vec::new();
		}
		KeyCode::Right => {
			set_caret(backend, id, (caret + 1).min(chars.len()));
			return Vec::new();
		}
		KeyCode::Home => {
			set_caret(backend, id, 0);
			return Vec::new();
		}
		KeyCode::End => {
			set_caret(backend, id, chars.len());
			return Vec::new();
		}
		KeyCode::Enter => {
			let value: String = chars.iter().collect();
			backend.commit_draft(id, &value);
			return vec![Dispatch {
				node: id,
				event: NativeEvent::Submit { value },
			}];
		}
		_ => false,
	};

	if !edited {
		return Vec::new();
	}

	let value: String = chars.iter().collect();
	backend.commit_draft(id, &value);
	backend.note_emitted(id, &value);
	set_caret(backend, id, caret);
	vec![Dispatch {
		node: id,
		event: NativeEvent::Change { value },
	}]
}

/// Where the caret lands in a multi-line field: up and down keep the
/// column on the neighbouring line, home and end go to the line's ends.
fn line_caret(chars: &[char], caret: usize, code: KeyCode) -> usize {
	let start = chars[..caret]
		.iter()
		.rposition(|&symbol| symbol == '\n')
		.map_or(0, |index| index + 1);
	let end = chars[caret..]
		.iter()
		.position(|&symbol| symbol == '\n')
		.map_or(chars.len(), |index| caret + index);
	let column = caret - start;

	match code {
		KeyCode::Home => start,
		KeyCode::End => end,
		KeyCode::Up if start == 0 => 0,
		KeyCode::Up => {
			let previous = chars[..start - 1]
				.iter()
				.rposition(|&symbol| symbol == '\n')
				.map_or(0, |index| index + 1);

			(previous + column).min(start - 1)
		}
		KeyCode::Down if end == chars.len() => chars.len(),
		KeyCode::Down => {
			let next_end = chars[end + 1..]
				.iter()
				.position(|&symbol| symbol == '\n')
				.map_or(chars.len(), |index| end + 1 + index);

			(end + 1 + column).min(next_end)
		}
		_ => caret,
	}
}

fn set_caret(backend: &mut TuiBackend, id: NodeId, caret: usize) {
	if let Some(draft) = backend.drafts.get_mut(id.index()).and_then(Option::as_mut) {
		draft.caret = caret;
	}
}

/// Routes one mouse event: clicks focus and activate, the wheel scrolls the
/// enclosing scroll view.
pub(crate) fn handle_mouse(backend: &mut TuiBackend, mouse: Mouse) -> Vec<Dispatch> {
	let x = f32::from(mouse.col()) * CELL_WIDTH + CELL_WIDTH / 2.0;
	let y = f32::from(mouse.row()) * CELL_HEIGHT + CELL_HEIGHT / 2.0;

	let hit = backend.hit_test(x, y);
	let mut events = Vec::new();

	if matches!(mouse, Mouse::Click { .. }) {
		match press_outside(backend, hit) {
			Some((node, true)) => {
				return vec![Dispatch {
					node,
					event: NativeEvent::Dismiss,
				}];
			}
			Some((node, false)) => {
				events.push(Dispatch {
					node,
					event: NativeEvent::Dismiss,
				});
			}
			None => {}
		}
	}

	let Some(hit) = hit else {
		return events;
	};

	events.extend(route_mouse(backend, mouse, hit, x, y));
	events
}

/// The overlay a click asks to close, and whether the click ends there.
///
/// A click on a modal's own backdrop goes no further. A click outside the
/// popover on top closes it and still reaches what was clicked, except on
/// the popover's anchor: the control that opened it decides for itself.
fn press_outside(backend: &TuiBackend, hit: Option<NodeId>) -> Option<(NodeId, bool)> {
	let top = *backend.open_overlays().last()?;
	let node = backend.node(top)?;

	if node.kind == NativeKind::Modal {
		return (hit == Some(top)).then_some((top, true));
	}

	let inside = |ancestor: NodeId| hit.is_some_and(|hit| backend.within(hit, ancestor));
	let on_anchor = node
		.props
		.overlay
		.as_ref()
		.and_then(|overlay| overlay.anchor.as_deref())
		.and_then(|anchor| backend.node_with_id(anchor))
		.is_some_and(inside);

	(!inside(top) && !on_anchor).then_some((top, false))
}

fn route_mouse(
	backend: &mut TuiBackend,
	mouse: Mouse,
	hit: NodeId,
	x: f32,
	y: f32,
) -> Vec<Dispatch> {
	match mouse {
		Mouse::ScrollUp { .. } | Mouse::ScrollDown { .. } => {
			let direction = if matches!(mouse, Mouse::ScrollUp { .. }) {
				-1.0
			} else {
				1.0
			};

			scroll_enclosing(backend, hit, direction)
		}
		Mouse::Click { .. } => click(backend, hit, x, y),
	}
}

fn scroll_enclosing(backend: &mut TuiBackend, hit: NodeId, direction: f32) -> Vec<Dispatch> {
	let mut current = Some(hit);

	while let Some(id) = current {
		let Some(node) = backend.node(id) else {
			return Vec::new();
		};

		if node.kind == NativeKind::ScrollView {
			return backend
				.scroll_by(id, 0.0, direction * 3.0 * CELL_HEIGHT)
				.map(|event| vec![Dispatch { node: id, event }])
				.unwrap_or_default();
		}

		current = node.parent;
	}

	Vec::new()
}

fn click(backend: &mut TuiBackend, hit: NodeId, x: f32, y: f32) -> Vec<Dispatch> {
	if !path_active(backend, hit) {
		return Vec::new();
	}

	let mut events = backend.set_focus(nearest_focusable(backend, hit));
	let Some(node) = backend.node(hit) else {
		return events;
	};

	let event = match node.kind {
		NativeKind::Button | NativeKind::Pressable => Some(NativeEvent::Press),
		NativeKind::Switch | NativeKind::Checkbox => {
			Some(NativeEvent::Toggle {
				checked: !node.props.checked.unwrap_or(false),
			})
		}
		NativeKind::Radio => Some(NativeEvent::Toggle { checked: true }),
		NativeKind::Slider => {
			backend.absolute_frame(hit).and_then(|frame| {
				slider_click(
					node.props.slider.unwrap_or_default(),
					x,
					frame.x,
					frame.width,
				)
			})
		}
		NativeKind::TextInput | NativeKind::PasswordInput => {
			place_caret(backend, hit, x);
			None
		}
		NativeKind::TextArea => {
			place_area_caret(backend, hit, x, y);
			None
		}
		_ => None,
	};

	if let Some(event) = event {
		events.push(Dispatch { node: hit, event });
	}

	events
}

/// A slider click proposes the value at the click's position.
fn slider_click(
	state: leptoskit_native::SliderState,
	x: f32,
	frame_x: f32,
	frame_width: f32,
) -> Option<NativeEvent> {
	if frame_width <= 0.0 {
		return None;
	}

	let ratio = f64::from(((x - frame_x) / frame_width).clamp(0.0, 1.0));

	Some(NativeEvent::ValueChange {
		value: state.min() + ratio * (state.max() - state.min()),
	})
}

/// A field click places the caret at the clicked column.
fn place_caret(backend: &mut TuiBackend, id: NodeId, x: f32) {
	let Some(frame) = backend.absolute_frame(id) else {
		return;
	};

	let column = ((x - frame.x) / CELL_WIDTH).floor().max(0.0) as usize;
	set_caret(backend, id, column);
}

/// Puts a multi-line field's caret under a click, on the wrapped line and
/// column the painter drew there.
fn place_area_caret(backend: &mut TuiBackend, id: NodeId, x: f32, y: f32) {
	let Some(frame) = backend.absolute_frame(id) else {
		return;
	};
	let Some(draft) = backend.draft(id) else {
		return;
	};

	let cols = (frame.width / CELL_WIDTH).floor().max(1.0) as usize;
	let rows = (frame.height / CELL_HEIGHT).floor().max(1.0) as usize;
	let layout = crate::text::area_layout(&draft.value, draft.caret, cols, rows);
	let row = ((y - frame.y) / CELL_HEIGHT).floor().max(0.0) as usize;
	let column = ((x - frame.x) / CELL_WIDTH).floor().max(0.0) as usize;
	let caret = crate::text::area_caret(&draft.value, &layout, layout.first + row, column);
	set_caret(backend, id, caret);
}
