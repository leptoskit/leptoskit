//! Read-only queries over a committed surface, shared by the widget tester
//! and the headless guest runner.

use std::fmt::Write;

use leptoskit_native::Frame;
use leptoskit_native::NativeBackend;
use leptoskit_native::NativeKind;
use leptoskit_native::NativeProps;
use leptoskit_native::NativeSurface;
use leptoskit_native::Role;
use leptoskit_yoga::NodeId;

use crate::Finder;
use crate::Widget;

/// A committed surface seen through a viewport.
pub(crate) struct Inspector<'a, B: NativeBackend> {
	pub(crate) surface: &'a NativeSurface<B>,
	pub(crate) viewport: (f32, f32),
}

/// The facts one accessibility line is built from.
struct Line<'a> {
	id: NodeId,
	depth: usize,
	kind: NativeKind,
	aria: &'a leptoskit_native::Aria,
	name: Option<&'a str>,
	checked: Option<leptoskit_native::Tristate>,
	value: Option<f64>,
	live: leptoskit_native::Live,
}

/// Whether a node reports any of its states.
fn has_states(aria: &leptoskit_native::Aria, checked: Option<leptoskit_native::Tristate>) -> bool {
	checked.is_some()
		|| aria.pressed.is_some()
		|| aria.selected.is_some()
		|| aria.expanded.is_some()
		|| aria.busy.is_some()
		|| aria.required.is_some()
		|| aria
			.invalid
			.is_some_and(|invalid| invalid != leptoskit_native::Invalid::False)
		|| aria.current.is_some()
		|| aria.modal.is_some()
		|| aria.read_only.is_some()
}

/// Writes the states a node sets, in a stable order, as ` [state=value]`.
fn write_states(
	entry: &mut String,
	aria: &leptoskit_native::Aria,
	checked: Option<leptoskit_native::Tristate>,
) {
	let flag = |value: Option<bool>| value.filter(|value| *value).map(|_| "true".to_owned());
	let text =
		|value: Option<leptoskit_native::Tristate>| value.map(|value| value.as_str().to_owned());

	for (state, value) in [
		("checked", text(checked)),
		("pressed", text(aria.pressed)),
		("selected", flag(aria.selected)),
		("expanded", flag(aria.expanded)),
		("busy", flag(aria.busy)),
		("modal", flag(aria.modal)),
		("required", flag(aria.required)),
		("read-only", flag(aria.read_only)),
		(
			"invalid",
			aria.invalid
				.filter(|invalid| *invalid != leptoskit_native::Invalid::False)
				.map(|invalid| invalid.as_str().to_owned()),
		),
		(
			"current",
			aria.current
				.filter(|current| *current != leptoskit_native::Current::False)
				.map(|current| current.as_str().to_owned()),
		),
	] {
		if let Some(value) = value {
			write!(entry, " [{state}={value}]").expect("writing to a string");
		}
	}
}

/// Whether a frame is finite and overlaps the viewport at all.
fn within_viewport(frame: Frame, viewport: (f32, f32)) -> bool {
	let finite = [frame.x, frame.y, frame.width, frame.height]
		.into_iter()
		.all(f32::is_finite);

	finite
		&& frame.width > 0.0
		&& frame.height > 0.0
		&& frame.x < viewport.0
		&& frame.y < viewport.1
		&& frame.x + frame.width > 0.0
		&& frame.y + frame.height > 0.0
}

/// Whether a node hides itself and its subtree: a hidden node, a stopped
/// activity indicator, or a transparent one.
pub(crate) fn hides(props: &NativeProps, kind: NativeKind) -> bool {
	props.hidden == Some(true)
		|| (kind == NativeKind::ActivityIndicator && props.running == Some(false))
		|| props
			.opacity
			.is_some_and(|opacity| !opacity.is_finite() || opacity <= 0.0)
}

/// Whether a node clips its subtree to its own frame.
pub(crate) fn clips(props: &NativeProps, kind: NativeKind) -> bool {
	props.clip == Some(true) || kind == NativeKind::ScrollView
}

/// The overlap of two frames.
fn intersect(frame: Frame, clip: Frame) -> Frame {
	let right = (frame.x + frame.width).min(clip.x + clip.width);
	let bottom = (frame.y + frame.height).min(clip.y + clip.height);
	let x = frame.x.max(clip.x);
	let y = frame.y.max(clip.y);

	Frame {
		x,
		y,
		width: right - x,
		height: bottom - y,
	}
}

impl<B: NativeBackend> Inspector<'_, B> {
	/// A stable, readable tree, including hidden widgets, without allocation
	/// IDs.
	pub(crate) fn tree(&self) -> String {
		fn diagnostic_value(kind: NativeKind, value: Option<&str>) -> Option<&str> {
			if kind == NativeKind::PasswordInput {
				value.map(|_| "<redacted>")
			} else {
				value
			}
		}

		let mut output = String::new();
		let mut pending = self
			.surface
			.root()
			.map(|id| (id, 0))
			.into_iter()
			.collect::<Vec<_>>();

		while let Some((id, depth)) = pending.pop() {
			let props = self.surface.props(id);
			let frame = self.absolute_frame(id);
			let kind = self.surface.kind(id);
			// Writing to a String cannot fail.
			writeln!(
				output,
				"{}{} text={:?} label={:?} value={:?} frame=({:.1},{:.1},{:.1},{:.1}) visible={} \
				 enabled={}",
				"  ".repeat(depth),
				kind,
				diagnostic_value(kind, props.text.as_deref()),
				props.accessibility_label,
				diagnostic_value(kind, props.value.as_deref()),
				frame.x,
				frame.y,
				frame.width,
				frame.height,
				self.visible(id),
				self.enabled(id)
			)
			.expect("writing to a string");
			pending.extend(
				self.surface
					.children(id)
					.iter()
					.rev()
					.map(|&child| (child, depth + 1)),
			);
		}

		output
	}

	/// What assistive technology sees, as an indented outline.
	///
	/// One line per accessibility-relevant element: its role, accessible
	/// name, states, values, and live-region politeness. Elements hidden
	/// with `aria_hidden` disappear with their subtrees, control content
	/// merges into the control, and unnamed plain containers are skipped,
	/// mirroring what a screen reader exposes. Use this for accessibility
	/// assertions and snapshots, like Flutter's semantics tree dump.
	pub(crate) fn semantics(&self) -> String {
		let mut lines = Vec::new();

		if let Some(root) = self.surface.root() {
			self.semantics_walk(root, 0, &mut lines);
		}

		let mut output = lines.join("\n");

		if !output.is_empty() {
			output.push('\n');
		}

		output
	}

	fn semantics_walk(&self, id: NodeId, depth: usize, lines: &mut Vec<String>) {
		let props = self.surface.props(id);
		let aria = props.aria();

		if aria.is_hidden() {
			return;
		}

		let kind = self.surface.kind(id);
		let role = aria.effective_role(kind);
		let live = aria.effective_live(kind);
		// The resolver filled `live_text` with what the region announces, so
		// an unnamed live region still shows its read-aloud content.
		let name = props.accessible_name().or(aria.live_text.as_deref());
		let checked = aria.checked.or(match kind {
			NativeKind::Switch | NativeKind::Checkbox | NativeKind::Radio => {
				props.checked.map(leptoskit_native::Tristate::from)
			}
			_ => None,
		});
		let value = aria.value_now.or_else(|| {
			(kind == NativeKind::Slider).then(|| props.slider.unwrap_or_default().value())
		});
		let included = role.is_some()
			|| name.is_some()
			|| aria.description.is_some()
			|| aria.level.is_some()
			|| has_states(aria, checked)
			|| value.is_some()
			|| aria.value_text.is_some()
			|| live != leptoskit_native::Live::Off;

		if included {
			let facts = Line {
				id,
				depth,
				kind,
				aria,
				name,
				checked,
				value,
				live,
			};
			lines.push(self.describe(&facts));
		}

		// A control is one accessibility element with its content merged, and
		// so are a named plain container (like React Native's `accessible`)
		// and a live region, whose announced unit is the whole subtree.
		let merged = role.is_some_and(Role::is_interactive)
			|| live != leptoskit_native::Live::Off
			|| (aria.role.is_none()
				&& name.is_some()
				&& matches!(
					kind,
					NativeKind::View
						| NativeKind::ScrollView
						| NativeKind::Text
						| NativeKind::TextRun
				));

		if !merged {
			for &child in self.surface.children(id) {
				self.semantics_walk(child, depth + usize::from(included), lines);
			}
		}
	}

	/// One node's line: its role, name, states, and value.
	fn describe(&self, line: &Line<'_>) -> String {
		let Line {
			id,
			depth,
			kind,
			aria,
			name,
			checked,
			value,
			live,
		} = *line;
		let mut entry = format!(
			"{}{}",
			"  ".repeat(depth),
			aria.effective_role(kind)
				.map_or(kind.wire_name(), Role::wire_name)
		);

		if let Some(level) = aria.level {
			write!(entry, "({level})").expect("writing to a string");
		}

		if let Some(name) = name {
			write!(entry, " {name:?}").expect("writing to a string");
		}

		if !self.enabled(id) {
			entry.push_str(" [disabled]");
		}

		write_states(&mut entry, aria, checked);

		if let Some(value) = value {
			write!(entry, " value={value}").expect("writing to a string");
		}

		if let Some(text) = &aria.value_text {
			write!(entry, " {text:?}").expect("writing to a string");
		}

		if let Some(description) = &aria.description {
			write!(entry, " description={description:?}").expect("writing to a string");
		}

		if live != leptoskit_native::Live::Off {
			write!(entry, " live={}", live.as_str()).expect("writing to a string");
			let atomic = aria
				.atomic
				.unwrap_or_else(|| aria.effective_role(kind).is_some_and(Role::implicit_atomic));

			if atomic {
				entry.push_str(" atomic");
			}
		}

		entry
	}

	pub(crate) fn matches(&self, finder: &Finder) -> Vec<NodeId> {
		let mut found = Vec::new();
		let mut pending = self.surface.root().into_iter().collect::<Vec<_>>();

		while let Some(id) = pending.pop() {
			if self.visible(id) && finder.matches(self.surface.kind(id), self.surface.props(id)) {
				found.push(id);
			}

			pending.extend(self.surface.children(id).iter().rev().copied());
		}

		found
	}

	pub(crate) fn widget(&self, id: NodeId) -> Widget {
		Widget {
			kind: self.surface.kind(id),
			props: self.surface.props(id).clone(),
			frame: self.absolute_frame(id),
		}
	}

	pub(crate) fn absolute_frame(&self, id: NodeId) -> Frame {
		let mut frame = self.surface.frame(id).expect("reachable nodes have frames");
		let mut parent = self.surface.parent(id);

		while let Some(id) = parent {
			let offset = self
				.surface
				.frame(id)
				.expect("reachable parents have frames");
			frame.x += offset.x;
			frame.y += offset.y;
			parent = self.surface.parent(id);
		}

		frame
	}

	pub(crate) fn visible(&self, id: NodeId) -> bool {
		self.visible_frame(id).is_some()
	}

	pub(crate) fn visible_frame(&self, id: NodeId) -> Option<Frame> {
		let frame = self.absolute_frame(id);

		if !within_viewport(frame, self.viewport) {
			return None;
		}

		self.clip_through_ancestors(id, frame)
	}

	/// Clips a frame by every ancestor that clips, and rejects one that is
	/// hidden, collapsed, or clipped away entirely.
	fn clip_through_ancestors(&self, id: NodeId, frame: Frame) -> Option<Frame> {
		let mut visible = frame;
		let mut current = Some(id);

		while let Some(node) = current {
			let props = self.surface.props(node);
			let kind = self.surface.kind(node);

			if hides(props, kind) {
				return None;
			}

			let layout = self.surface.layout(node);

			if layout.width <= 0.0 || layout.height <= 0.0 {
				return None;
			}

			if node != id && clips(props, kind) {
				visible = intersect(visible, self.absolute_frame(node));

				if visible.width <= 0.0 || visible.height <= 0.0 {
					return None;
				}
			}

			current = self.surface.parent(node);
		}

		Some(visible)
	}

	pub(crate) fn enabled(&self, id: NodeId) -> bool {
		let mut current = Some(id);

		while let Some(node) = current {
			if self.surface.props(node).enabled == Some(false) {
				return false;
			}

			current = self.surface.parent(node);
		}

		true
	}

	pub(crate) fn is_descendant(&self, child: NodeId, ancestor: NodeId) -> bool {
		let mut current = Some(child);

		while let Some(node) = current {
			if node == ancestor {
				return true;
			}

			current = self.surface.parent(node);
		}

		false
	}
}
