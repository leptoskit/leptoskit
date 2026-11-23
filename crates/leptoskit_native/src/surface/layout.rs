//! Commits changed frames and rebuilds the existing hit-area cache.

use leptoskit_yoga::AvailableSpace;
use leptoskit_yoga::NodeId;
use leptoskit_yoga::Size;

use super::ChildOrder;
use super::Frame;
use super::HitArea;
use super::HitBounds;
use super::NativeSurface;
use crate::backend::Mutation;
use crate::backend::MutationList;
use crate::backend::NativeBackend;
use crate::props::NativeKind;

impl<B: NativeBackend> NativeSurface<B> {
	pub(super) fn commit_layout(&mut self) {
		let Some(root) = self.root else {
			return;
		};
		#[cfg(feature = "devtools")]
		let layout_start = std::time::Instant::now();
		let available = Size::new(
			AvailableSpace::Definite(self.viewport.width.max(0.0)),
			AvailableSpace::Definite(self.viewport.height.max(0.0)),
		);
		// A container root fills the viewport's width, as its `<div>` does on
		// the web, so a screen that centres its content centres it on the
		// screen. Text and controls keep their intrinsic width there too.
		if matches!(self.kind(root), NativeKind::View | NativeKind::ScrollView) {
			self.yoga.calculate_layout_filling_width(root, available);
		} else {
			self.yoga.calculate_layout(root, available);
		}

		let pass = self.yoga.current_pass();
		let mut frames = if self.frames.get(root.index()).copied().flatten().is_none() {
			MutationList::with_capacity(self.shadows.len())
		} else {
			MutationList::new()
		};
		self.overlays.clear();
		self.collect_frames(root, 0.0, 0.0, pass, &mut frames);
		self.place_popovers(pass, &mut frames);
		self.scroll_to_reveals(&mut frames);
		self.backend.apply(&frames);
		#[cfg(feature = "devtools")]
		{
			let commit = self.devtools.finish(frames.len(), layout_start);

			if let Some(observer) = &mut self.devtools.observer {
				observer(&commit);
			}
		}
	}

	/// Answers the reveals this commit asked for, now that every frame is
	/// known: the nearest scroll view around each revealed node is given
	/// the offset that brings the node into view.
	///
	/// A backend scrolls when a scroll view's offset changes. Asking twice
	/// for the same place, with the person scrolling away in between, is
	/// still two reveals, so the offset is cleared first.
	fn scroll_to_reveals(&mut self, out: &mut MutationList) {
		for (id, reveal) in std::mem::take(&mut self.reveals) {
			// Every request made since the last commit is new, whatever
			// order the tree holds them in.
			self.reveal_answered = self.reveal_answered.max(reveal.serial);
			let align = reveal.align;

			if !self.is_committed(id) {
				continue;
			}

			let mut view = self.shadows[id.index()].parent;

			while let Some(candidate) = view
				&& self.shadows[candidate.index()].kind != NativeKind::ScrollView
			{
				view = self.shadows[candidate.index()].parent;
			}

			let Some(view) = view else {
				continue;
			};
			let target = self.hit_areas[id.index()];
			let frame = self.hit_areas[view.index()];
			let known = self.scrolled.borrow().get(&view).copied();
			let (current_x, current_y) = known.unwrap_or_else(|| {
				self.shadows[view.index()]
					.props
					.scroll_offset
					.map_or((0.0, 0.0), |offset| (offset.x, offset.y))
			});
			// Across the way the view scrolls, the least movement is right
			// whatever was asked for along it.
			let x = crate::ScrollAlign::Nearest.offset(
				target.left - frame.left,
				target.width,
				frame.width,
				current_x,
			);
			let y = align.offset(
				target.top - frame.top,
				target.height,
				frame.height,
				current_y,
			);

			if (x, y) == (current_x, current_y) {
				continue;
			}

			let mut props = self.shadows[view.index()].props.clone();
			props.focus_request = None;
			props.reveal = None;
			props.scroll_offset = None;
			out.push(Mutation::UpdateProps {
				id: view,
				props: props.clone(),
			});
			props.scroll_offset = Some(crate::ScrollOffset::new(x, y).animated());
			out.push(Mutation::UpdateProps { id: view, props });
			self.scrolled.borrow_mut().insert(view, (x, y));
		}
	}

	/// Pushes changed frames for a subtree and rebuilds its hit areas.
	/// `origin_x` and `origin_y` are the parent's viewport position, and the
	/// returned tuple is `(subtree_bounds, subtree_has_open_overlay)`.
	///
	/// An open overlay is not part of its parent's subtree here: it is
	/// placed in the viewport (see [`Self::overlay_frame`] for the frame
	/// the backend is given), and [`Self::hit_test`] asks the open overlays
	/// before the rest of the tree. A modal covers the viewport. A
	/// popover waits for [`Self::place_popovers`], since its anchor may come
	/// later in the tree.
	pub(super) fn collect_frames(
		&mut self,
		id: NodeId,
		origin_x: f32,
		origin_y: f32,
		pass: u64,
		out: &mut MutationList,
	) -> (HitBounds, bool) {
		if self.shadows[id.index()].is_open_overlay() {
			self.overlays.push(id);

			if self.shadows[id.index()].kind == NativeKind::Modal {
				let frame = self.overlay_frame((0.0, 0.0), (origin_x, origin_y));
				self.place(id, frame, (0.0, 0.0), pass, out);
			}

			return (HitBounds::EMPTY, true);
		}

		let layout = *self.yoga.layout(id);

		self.place(
			id,
			(layout.x, layout.y),
			(origin_x + layout.x, origin_y + layout.y),
			pass,
			out,
		)
	}

	/// Commits one node at `frame`, its position as the backend sees it,
	/// and `position`, the same corner in viewport coordinates, then its
	/// children.
	fn place(
		&mut self,
		id: NodeId,
		frame: (f32, f32),
		position: (f32, f32),
		pass: u64,
		out: &mut MutationList,
	) -> (HitBounds, bool) {
		let idx = id.index();
		let layout = *self.yoga.layout(id);
		let frame = Frame {
			x: frame.0,
			y: frame.1,
			width: layout.width,
			height: layout.height,
		};
		let had_frame = self.frames.get(idx).copied().flatten().is_some();

		self.push_changed_frame(id, frame, out);

		let (left, top) = position;
		let prev_area = self.hit_areas[idx];

		// If neither layout nor structural/hit metadata changed anywhere in
		// this subtree, its viewport origin and size stayed identical, and it
		// contains no open overlays, every descendant's frame and hit area is
		// guaranteed unchanged.
		if had_frame
			&& !self.meta_dirty[idx]
			&& !self.subtree_has_overlay[idx]
			&& !self.yoga.subtree_changed_in(id, pass)
			&& prev_area.left.to_bits() == left.to_bits()
			&& prev_area.top.to_bits() == top.to_bits()
			&& prev_area.width.to_bits() == layout.width.to_bits()
			&& prev_area.height.to_bits() == layout.height.to_bits()
		{
			return (prev_area.subtree, false);
		}

		let mut subtree = HitBounds {
			left,
			top,
			right: left + layout.width,
			bottom: top + layout.height,
		};
		let shadow = &self.shadows[idx];
		let inert = shadow.is_inert();
		// Clipped overflow is only hit inside the node's own frame, and a
		// pressable's children are decorative, so neither widens the bounds.
		let overflows = shadow.props.clip != Some(true) && shadow.kind != NativeKind::Pressable;
		let child_count = shadow.children.len();
		let mut has_overlay = false;
		let mut mono_y = child_count >= 8;
		let mut mono_x = child_count >= 8;
		let mut prev_bottom = f32::NEG_INFINITY;
		let mut prev_right = f32::NEG_INFINITY;

		for index in 0..child_count {
			let child = self.shadows[idx].children[index];
			let (child_bounds, child_overlay) = self.collect_frames(child, left, top, pass, out);
			has_overlay |= child_overlay;

			if overflows {
				subtree = subtree.union(child_bounds);
			}

			if mono_y || mono_x {
				if child_bounds.left <= child_bounds.right
					&& child_bounds.top <= child_bounds.bottom
				{
					if child_bounds.top < prev_bottom {
						mono_y = false;
					}

					if child_bounds.left < prev_right {
						mono_x = false;
					}

					prev_bottom = child_bounds.bottom;
					prev_right = child_bounds.right;
				} else {
					mono_y = false;
					mono_x = false;
				}
			}
		}

		if inert {
			subtree = HitBounds::EMPTY;
		}

		let child_order = if mono_y {
			ChildOrder::MonotonicY
		} else if mono_x {
			ChildOrder::MonotonicX
		} else {
			ChildOrder::Unordered
		};

		self.hit_areas[idx] = HitArea {
			left,
			top,
			width: layout.width,
			height: layout.height,
			subtree,
			child_order,
		};
		self.subtree_has_overlay[idx] = has_overlay;
		self.meta_dirty[idx] = false;
		(subtree, has_overlay)
	}

	/// Places each open popover beside its anchor, once every node outside
	/// the popovers has a position. A popover inside a popover is found
	/// while its parent is placed, and placed after it.
	fn place_popovers(&mut self, pass: u64, out: &mut MutationList) {
		let mut next = 0;

		while let Some(&id) = self.overlays.get(next) {
			next += 1;

			let shadow = &self.shadows[id.index()];

			if shadow.kind != NativeKind::Popover {
				continue;
			}

			let layout = *self.yoga.layout(id);
			let anchor = shadow
				.props
				.overlay
				.as_ref()
				.and_then(|overlay| overlay.anchor.as_deref())
				.and_then(|anchor| self.node_with_id(anchor))
				.map(|anchor| self.hit_areas[anchor.index()]);
			let placement = shadow
				.props
				.overlay
				.as_ref()
				.map(|overlay| overlay.placement)
				.unwrap_or_default();
			let placement = if shadow.props.direction == Some(leptoskit_yoga::Direction::Rtl) {
				placement.mirrored()
			} else {
				placement
			};
			// Without an anchor on screen the popover opens at the
			// viewport's top start corner.
			let position = anchor.map_or((0.0, 0.0), |anchor| {
				placement.position(
					(anchor.left, anchor.top, anchor.width, anchor.height),
					(layout.width, layout.height),
					(self.viewport.width, self.viewport.height),
				)
			});

			let parent = self.shadows[id.index()]
				.parent
				.map_or((0.0, 0.0), |parent| {
					let area = self.hit_areas[parent.index()];

					(area.left, area.top)
				});
			let frame = self.overlay_frame(position, parent);
			self.place(id, frame, position, pass, out);
		}
	}

	/// The frame origin the backend is given for an overlay at `position`
	/// in the viewport, whose parent is at `parent`: the viewport position
	/// itself for a backend that presents overlays, and the offset from the
	/// parent for one that shows them as containers.
	fn overlay_frame(&self, position: (f32, f32), parent: (f32, f32)) -> (f32, f32) {
		if self.backend.presents_overlays() {
			position
		} else {
			(position.0 - parent.0, position.1 - parent.1)
		}
	}

	/// The committed node whose element has this `id`, in document order.
	fn node_with_id(&self, wanted: &str) -> Option<NodeId> {
		let mut pending = vec![self.root?];

		while let Some(id) = pending.pop() {
			let shadow = &self.shadows[id.index()];

			if shadow.props.id.as_deref() == Some(wanted) {
				return Some(id);
			}

			pending.extend(shadow.children.iter().rev());
		}

		None
	}

	/// Emits each changed frame once, preserving the existing allocated node
	/// slots.
	fn push_changed_frame(&mut self, id: NodeId, frame: Frame, out: &mut MutationList) {
		if self.frames.get(id.index()).copied().flatten() != Some(frame) {
			if self.frames.len() <= id.index() {
				self.frames.resize(id.index() + 1, None);
			}

			self.frames[id.index()] = Some(frame);
			out.push(Mutation::UpdateLayout {
				id,
				x: frame.x,
				y: frame.y,
				width: frame.width,
				height: frame.height,
			});
		}
	}
}
