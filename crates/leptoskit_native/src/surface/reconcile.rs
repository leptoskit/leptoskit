//! Reconciles source elements into the existing shadow tree and mutation
//! stream.

use leptoskit_yoga::NodeId;
use leptoskit_yoga::Style;

use super::HitArea;
use super::NativeSurface;
use super::ShadowNode;
use super::measured_content;
use crate::backend::Mutation;
use crate::backend::MutationList;
use crate::backend::NativeBackend;
use crate::element::Element;
use crate::element::InheritedProps;
use crate::element::ResolvedOwn;
use crate::props::NativeKind;
use crate::props::NativeProps;
use crate::tailwind::StyleContext;

impl<B: NativeBackend> NativeSurface<B> {
	pub(super) fn commit_source(&mut self) {
		self.adopt_root_appearance();

		let Some(source) = self.source.take() else {
			return;
		};
		// Reconciliation is positional, so the held node's path in the
		// committed tree names its element in the new source.
		let held = self
			.pressed
			.filter(|&id| self.is_committed(id))
			.map(|id| self.path_to(id));
		let context = StyleContext {
			viewport: self.viewport,
			dark: self.dark,
			safe_area: self.safe_area,
			active: false,
		};
		self.uses_viewport = false;
		self.uses_safe_area = false;
		self.uses_dark = false;

		let mut mutations = if self.root.is_none() {
			let count = count_nodes(&source);
			self.reserve_nodes(count);
			MutationList::with_capacity(count.saturating_mul(2))
		} else {
			MutationList::new()
		};
		let new_root = if source.tree_declares_relationships() {
			self.record_tree_context_flags(&source);
			let (element, relationships) = source.resolved_tracking(&context, held.as_deref());
			self.relationships = relationships;

			match self.root {
				None => self.build_subtree_owned(None, element, &mut mutations),
				Some(root) => self.update_node_owned(root, element, &mut mutations),
			}
		} else {
			self.relationships = false;

			match self.root {
				None => {
					self.build_source_subtree(
						None,
						&source,
						&context,
						InheritedProps::default(),
						held.as_deref(),
						&mut mutations,
					)
				}
				Some(root) => {
					self.update_source_node(
						root,
						&source,
						&context,
						InheritedProps::default(),
						held.as_deref(),
						&mut mutations,
					)
				}
			}
		};

		self.source = Some(source);
		self.root = Some(new_root);
		self.apply_and_lay_out(mutations);

		// A held control that was replaced, removed, disabled, or hidden by
		// this commit is no longer pressed, and no release will arrive for a
		// replacement built in its slot. Render that slot unpressed.
		if let Some(id) = self.pressed
			&& (!self.is_committed(id) || !self.event_path_is_active(id))
		{
			self.pressed = None;
			self.commit_source();
		}
	}

	/// Records whether `element` depends on viewport, safe area, or dark mode.
	#[inline]
	pub(super) fn record_element_context_flags(&mut self, element: &Element) {
		self.uses_viewport |=
			element.classes().uses_viewport() || element.kind == NativeKind::Modal;
		self.uses_safe_area |= element.classes().uses_safe_area();
		self.uses_dark |= element.classes().uses_dark();
	}

	/// Records context dependency flags across an entire subtree.
	pub(super) fn record_tree_context_flags(&mut self, element: &Element) {
		self.record_element_context_flags(element);

		for child in &element.children {
			self.record_tree_context_flags(child);
		}
	}

	/// Marks `id` and its ancestors as having layout-independent metadata
	/// changes (structure, inert, clip, or overlay state) so the next
	/// `collect_frames` walk recomputes their hit areas and overlay list.
	#[inline]
	pub(super) fn mark_meta_dirty(&mut self, id: NodeId) {
		let mut current = Some(id);

		while let Some(node) = current {
			let idx = node.index();

			if self.meta_dirty[idx] {
				break;
			}

			self.meta_dirty[idx] = true;
			current = self.shadows[idx].parent;
		}
	}

	/// Hands a reconciled batch to the backend, then lays out and pushes the
	/// changed frames.
	pub(super) fn apply_and_lay_out(&mut self, mutations: MutationList) {
		#[cfg(all(feature = "rive", not(target_arch = "wasm32")))]
		let mutations = self.sync_rive(mutations);
		#[cfg(feature = "devtools")]
		let source_start = std::time::Instant::now();
		#[cfg(feature = "devtools")]
		let mutation_count = mutations.len();
		self.backend.apply_owned(mutations);
		#[cfg(feature = "devtools")]
		self.devtools.source(mutation_count, source_start);

		self.commit_layout();
	}

	fn reserve_nodes(&mut self, additional: usize) {
		self.yoga.reserve(additional);
		self.shadows.reserve(additional);
		self.frames.reserve(additional);
		self.hit_areas.reserve(additional);
		self.meta_dirty.reserve(additional);
		self.subtree_has_overlay.reserve(additional);
	}

	pub(super) fn allocate(&mut self, kind: NativeKind) -> NodeId {
		let id = self.yoga.create_node();
		debug_assert_eq!(id.index(), self.shadows.len());
		self.shadows.push(ShadowNode {
			kind,
			props: NativeProps::default(),
			handlers: Vec::new(),
			gesture: None,
			children: Vec::new(),
			parent: None,
			text_cache: None,
			#[cfg(feature = "devtools")]
			source: None,
			#[cfg(feature = "devtools")]
			component: None,
			#[cfg(feature = "devtools")]
			classes: Vec::new(),
		});
		self.frames.push(None);
		self.hit_areas.push(HitArea::DETACHED);
		self.meta_dirty.push(true);
		self.subtree_has_overlay.push(false);
		id
	}

	/// Streaming build from an unresolved source element, avoiding the
	/// intermediate resolved `Element` tree allocation.
	fn build_source_subtree(
		&mut self,
		slot: Option<(NodeId, usize)>,
		source: &Element,
		context: &StyleContext,
		parent_inherited: InheritedProps,
		held: Option<&[usize]>,
		out: &mut MutationList,
	) -> NodeId {
		self.record_element_context_flags(source);
		let resolved = source.resolve_own_into(context, parent_inherited, held);
		let child_inherited = InheritedProps::from_props(&resolved.props);
		let id = self.create_resolved_node(slot, resolved, source, out);

		for (index, child) in source.children.iter().enumerate() {
			let child_held = match held {
				Some([first, rest @ ..]) if *first == index => Some(rest),
				_ => None,
			};
			self.build_source_subtree(
				Some((id, index)),
				child,
				context,
				child_inherited,
				child_held,
				out,
			);
		}

		id
	}

	/// Streaming update from an unresolved source element, avoiding the
	/// intermediate resolved `Element` tree allocation.
	pub(super) fn update_source_node(
		&mut self,
		id: NodeId,
		source: &Element,
		context: &StyleContext,
		parent_inherited: InheritedProps,
		held: Option<&[usize]>,
		out: &mut MutationList,
	) -> NodeId {
		self.record_element_context_flags(source);
		let resolved = source.resolve_own_into(context, parent_inherited, held);

		if !self.keeps_identity_parts(id, resolved.kind, &resolved.props) {
			let slot = self.node_slot(id);
			self.teardown(id, out);
			let child_inherited = InheritedProps::from_props(&resolved.props);
			let new_id = self.create_resolved_node(slot, resolved, source, out);

			for (index, child) in source.children.iter().enumerate() {
				let child_held = match held {
					Some([first, rest @ ..]) if *first == index => Some(rest),
					_ => None,
				};
				self.build_source_subtree(
					Some((new_id, index)),
					child,
					context,
					child_inherited,
					child_held,
					out,
				);
			}

			return new_id;
		}

		let child_inherited = InheritedProps::from_props(&resolved.props);
		self.update_own_resolved(id, resolved, source, out);

		let old_len = self.shadows[id.index()].children.len();
		let new_len = source.children.len();
		let common = old_len.min(new_len);

		for index in 0..common {
			let child_id = self.shadows[id.index()].children[index];
			let child_held = match held {
				Some([first, rest @ ..]) if *first == index => Some(rest),
				_ => None,
			};
			self.update_source_node(
				child_id,
				&source.children[index],
				context,
				child_inherited,
				child_held,
				out,
			);
		}

		for (offset, child) in source.children[common..new_len].iter().enumerate() {
			let index = common + offset;
			let child_held = match held {
				Some([first, rest @ ..]) if *first == index => Some(rest),
				_ => None,
			};
			self.build_source_subtree(
				Some((id, index)),
				child,
				context,
				child_inherited,
				child_held,
				out,
			);
		}

		for index in (new_len..old_len).rev() {
			let child_id = self.shadows[id.index()].children[index];
			self.teardown(child_id, out);
		}

		id
	}

	/// Creates one committed node from its resolved kind/style/props and source
	/// metadata (handlers, gesture, devtools).
	fn create_resolved_node(
		&mut self,
		slot: Option<(NodeId, usize)>,
		resolved: ResolvedOwn,
		meta: &Element,
		out: &mut MutationList,
	) -> NodeId {
		let id = self.allocate(resolved.kind);
		let kind = resolved.kind;
		self.yoga.set_style(id, resolved.style);

		if measured_content(kind, &resolved.props).is_some() {
			self.attach_text_measure(id, kind, &resolved.props);
		}

		let send_props = self.props_to_send(id, &resolved.props);
		let shadow = &mut self.shadows[id.index()];
		shadow.props = resolved.props;

		if !meta.handlers().is_empty() {
			shadow.handlers = meta.handlers().to_vec();
		}

		shadow.gesture = meta.attached_gesture().cloned();
		#[cfg(feature = "devtools")]
		{
			shadow.source = meta.source_location();
			shadow.component = meta.component_name();
			let classes = if meta.applied_classes().classes().is_empty() {
				meta.classes().classes()
			} else {
				meta.applied_classes().classes()
			};
			shadow.classes = classes.split_whitespace().map(str::to_owned).collect();
		}

		// A backend with no overlays of its own shows one as a container.
		let overlay = matches!(kind, NativeKind::Modal | NativeKind::Popover);
		let backend_kind = if overlay && !self.backend.presents_overlays() {
			NativeKind::View
		} else {
			kind
		};
		out.push(Mutation::Create {
			id,
			kind: backend_kind,
			props: send_props,
		});

		if let Some((parent, index)) = slot {
			self.attach_child(parent, id, index, out);
		}

		id
	}

	/// Builds a fresh subtree from an already-resolved owned `Element` (used
	/// when tree-wide ARIA resolution ran or when reconciling a patch subtree).
	pub(super) fn build_subtree_owned(
		&mut self,
		slot: Option<(NodeId, usize)>,
		mut element: Element,
		out: &mut MutationList,
	) -> NodeId {
		let children = std::mem::take(&mut element.children);
		let resolved = ResolvedOwn {
			kind: element.kind,
			style: std::mem::take(&mut element.style),
			props: std::mem::take(&mut element.props),
		};
		let id = self.create_resolved_node(slot, resolved, &element, out);

		for (index, child) in children.into_iter().enumerate() {
			self.build_subtree_owned(Some((id, index)), child, out);
		}

		id
	}

	/// Inserts a detached node into `parent` at `index` (clamped to the
	/// current child count) and records the `InsertChild` mutation.
	pub(super) fn attach_child(
		&mut self,
		parent: NodeId,
		child: NodeId,
		index: usize,
		out: &mut MutationList,
	) {
		debug_assert!(self.shadows[child.index()].parent.is_none());
		let children = &mut self.shadows[parent.index()].children;
		let index = index.min(children.len());
		children.insert(index, child);
		self.shadows[child.index()].parent = Some(parent);
		self.yoga.insert_child(parent, child, index);
		self.mark_meta_dirty(parent);
		out.push(Mutation::InsertChild {
			parent,
			child,
			index,
		});
	}

	fn node_slot(&self, id: NodeId) -> Option<(NodeId, usize)> {
		self.shadows[id.index()].parent.map(|parent| {
			let index = self.shadows[parent.index()]
				.children
				.iter()
				.position(|&child| child == id)
				.expect("a child is listed under its parent");
			(parent, index)
		})
	}

	pub(super) fn update_node_owned(
		&mut self,
		id: NodeId,
		mut element: Element,
		out: &mut MutationList,
	) -> NodeId {
		// A different widget kind is a different widget, and so is a different
		// accessibility role: platforms fix the role when the widget is
		// created (GTK's accessible role is construct-only), so a role change
		// tears the old subtree down and builds the replacement in the same
		// slot.
		if !self.keeps_identity_parts(id, element.kind, &element.props) {
			let slot = self.node_slot(id);
			self.teardown(id, out);

			return self.build_subtree_owned(slot, element, out);
		}

		let children = std::mem::take(&mut element.children);
		let resolved = ResolvedOwn {
			kind: element.kind,
			style: std::mem::take(&mut element.style),
			props: std::mem::take(&mut element.props),
		};
		self.update_own_resolved(id, resolved, &element, out);

		let old_len = self.shadows[id.index()].children.len();
		let new_len = children.len();
		let common = old_len.min(new_len);
		let mut new_iter = children.into_iter();

		for index in 0..common {
			let child_id = self.shadows[id.index()].children[index];
			let child_el = new_iter.next().expect("index < new_len");
			self.update_node_owned(child_id, child_el, out);
		}

		for (offset, child_el) in new_iter.enumerate() {
			self.build_subtree_owned(Some((id, common + offset)), child_el, out);
		}

		for index in (new_len..old_len).rev() {
			let child_id = self.shadows[id.index()].children[index];
			self.teardown(child_id, out);
		}

		id
	}

	/// Whether `kind` and `props` keep the identity of committed node `id`: the
	/// same widget kind and accessibility role.
	#[inline]
	pub(super) fn keeps_identity_parts(
		&self,
		id: NodeId,
		kind: NativeKind,
		props: &NativeProps,
	) -> bool {
		let shadow = &self.shadows[id.index()];
		shadow.kind == kind && shadow.props.aria().role == props.aria().role
	}

	/// The props a backend receives. A focus request travels once, with the
	/// commit that makes it: every later update leaves it out, and so does a
	/// widget created while it still holds a request already carried out.
	/// Backends then move focus once and the person can move on.
	///
	/// A reveal request never travels. The surface answers it once layout
	/// knows where the widget is, by scrolling the scroll view that holds it.
	fn props_to_send(&mut self, id: NodeId, props: &NativeProps) -> NativeProps {
		let mut props = props.clone();

		match props.focus_request {
			Some(serial) if serial > self.focus_answered => self.focus_answered = serial,
			_ => props.focus_request = None,
		}

		if let Some(reveal) = props.reveal.take()
			&& reveal.serial > self.reveal_answered
		{
			self.reveals.push((id, reveal));
		}

		props
	}

	/// Updates a node that keeps its identity, leaving its children alone.
	pub(super) fn update_own_resolved(
		&mut self,
		id: NodeId,
		resolved: ResolvedOwn,
		meta: &Element,
		out: &mut MutationList,
	) {
		let idx = id.index();

		if self.shadows[idx].props != resolved.props {
			let prev_inert = self.shadows[idx].is_inert();
			let prev_clip = self.shadows[idx].props.clip == Some(true);
			let prev_overlay = self.shadows[idx].is_open_overlay();
			let content_changed = measured_content(resolved.kind, &self.shadows[idx].props)
				!= measured_content(resolved.kind, &resolved.props);
			let send_props = self.props_to_send(id, &resolved.props);
			self.shadows[idx].props = resolved.props;

			let new_inert = self.shadows[idx].is_inert();
			let new_clip = self.shadows[idx].props.clip == Some(true);
			let new_overlay = self.shadows[idx].is_open_overlay();

			if prev_inert != new_inert || prev_clip != new_clip || prev_overlay != new_overlay {
				self.mark_meta_dirty(id);
			}

			self.update_text_measure(id, resolved.kind, content_changed);
			out.push(Mutation::UpdateProps {
				id,
				props: send_props,
			});
		}

		let handlers = meta.handlers();
		let shadow_handlers = &mut self.shadows[idx].handlers;

		if handlers.is_empty() {
			shadow_handlers.clear();
		} else {
			shadow_handlers.clear();
			shadow_handlers.extend_from_slice(handlers);
		}

		let gesture_removed =
			self.shadows[idx].gesture.is_some() && meta.attached_gesture().is_none();
		self.shadows[idx].gesture = meta.attached_gesture().cloned();

		if gesture_removed {
			let effects = {
				let mut session = self.gestures.borrow_mut();
				session.cancel_node(id, 0);
				session.take_effects()
			};
			self.fire_gestures(effects);
		}

		#[cfg(feature = "devtools")]
		{
			self.shadows[idx].source = meta.source_location();
			self.shadows[idx].component = meta.component_name();
			let classes = if meta.applied_classes().classes().is_empty() {
				meta.classes().classes()
			} else {
				meta.applied_classes().classes()
			};
			self.shadows[idx].classes = classes.split_whitespace().map(str::to_owned).collect();
		}
		self.yoga.set_style(id, resolved.style);
	}

	/// Removes a subtree, post-order: every node is detached from its parent
	/// (`RemoveChild`) and then deleted (`Delete`), so a widget never outlives
	/// its children and is always parentless when deleted.
	///
	/// The shadow slot stays allocated (ids are never reused), but it is left
	/// inert: no handlers, no parent, no children, and no measure function, so
	/// a stale id can neither receive events nor keep closures alive.
	///
	/// Children go last to first, as reconciliation removes them, so each
	/// removal finds its child at the end of the parent's list and clearing a
	/// long list costs its length rather than its square.
	pub(super) fn teardown(&mut self, id: NodeId, out: &mut MutationList) {
		let children = std::mem::take(&mut self.shadows[id.index()].children);

		for &child in children.iter().rev() {
			self.teardown(child, out);
		}

		if let Some(parent) = self.shadows[id.index()].parent.take() {
			let siblings = &mut self.shadows[parent.index()].children;

			if let Some(index) = siblings.iter().rposition(|&child| child == id) {
				siblings.remove(index);
			}

			self.yoga.detach(id);
			self.mark_meta_dirty(parent);
			out.push(Mutation::RemoveChild { parent, child: id });
		}

		let effects = {
			let mut session = self.gestures.borrow_mut();
			session.cancel_node(id, 0);
			session.take_effects()
		};
		self.fire_gestures(effects);
		let shadow = &mut self.shadows[id.index()];
		shadow.handlers.clear();
		shadow.gesture = None;
		// Inert arena slots must not retain removed assets or other owned
		// props.
		shadow.props = NativeProps::default();
		self.yoga.set_style(id, Style::default());

		if shadow.text_cache.take().is_some() {
			self.yoga.set_measured(id, false);
		}

		self.frames[id.index()] = None;
		self.hit_areas[id.index()] = HitArea::DETACHED;
		self.meta_dirty[id.index()] = true;
		self.subtree_has_overlay[id.index()] = false;
		self.scrolled.borrow_mut().remove(&id);
		out.push(Mutation::Delete { id });
	}
}

fn count_nodes(element: &Element) -> usize {
	let mut count = 1usize;

	for child in &element.children {
		count += count_nodes(child);
	}

	count
}
