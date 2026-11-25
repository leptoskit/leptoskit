use std::collections::HashMap;
use std::collections::HashSet;

use serde::Deserialize;

use super::region::RegionSlot;
use super::region::drain_dirty;
use crate::Element;
use crate::EventOutcome;
use crate::NativeEvent;
use crate::NativeKind;
use crate::element::HandlerEntry;
use crate::hash::FastHash;
use crate::reactive::Binding;

/// The UI imports, separated from tree reconciliation so it can run in tests.
pub(super) trait Host {
	fn create(&mut self, id: i64, kind: i32);
	fn props(&mut self, id: i64, bytes: &[u8]);
	fn insert(&mut self, parent: i64, child: i64, index: i32);
	fn delete(&mut self, id: i64);
	fn commit(&mut self);
}

/// A committed element without duplicate copies of its descendants.
#[derive(Clone)]
struct Node {
	element: Element,
	parent: Option<i64>,
	children: Vec<i64>,
	/// The key a [`For`](crate::reactive::For) or
	/// [`Show`](crate::reactive::Show) reuses this node by. `None` for a
	/// node that was not built as one of their children.
	key: Option<u64>,
}

/// Staged host mutation metadata recorded in build order so `publish`/`splice`
/// stream mutations linearly without repeated hash-map probes.
struct BuildOp {
	id: i64,
	kind: i32,
	is_new: bool,
	children_changed: bool,
	wire: Option<(u32, u32)>,
}

/// The guest retains handlers and the host retains the corresponding wire tree.
#[derive(Default)]
pub(super) struct Tree {
	nodes: HashMap<i64, Node, FastHash>,
	/// Node ids in build order, parents before children: the order the host
	/// hears about them, independent of the map's.
	order: Vec<i64>,
	/// Node ids evicted during the current [`Self::update`], recorded in
	/// forward build order so [`Self::publish_in_place`] sends `host.delete`
	/// in reverse order (leaves and trailing siblings first).
	removed: Vec<i64>,
	/// Reusable DFS stack for [`Self::evict_subtree`] and
	/// [`Self::evict_slice`].
	evict_stack: Vec<i64>,
	/// Node ids that read application state: what a refresh visits, so it
	/// costs the values that changed rather than the size of the tree.
	bound: Vec<i64>,
	/// Shared staging buffer for serialized wire payloads during one commit.
	wire_arena: Vec<u8>,
	/// Staged build-order operations for the current `update` or `splice`.
	build_ops: Vec<BuildOp>,
	root: Option<i64>,
	next_id: i64,
	/// Regions subscribed on this tree, keyed by the node that owns them.
	regions: HashMap<i64, std::sync::Arc<RegionSlot>, FastHash>,
}

/// Whether a node kept in place is now built somewhere else in the source,
/// which the host's `DevTools` agent shows. Always false without the
/// feature.
fn moved(previous: &Element, element: &Element) -> bool {
	#[cfg(feature = "devtools")]
	{
		previous.source_location() != element.source_location()
			|| previous.component_name() != element.component_name()
	}
	#[cfg(not(feature = "devtools"))]
	{
		let _ = (previous, element);
		false
	}
}

/// The style to send: none at all when it is the default.
fn sent_style(style: &leptoskit_yoga::Style) -> Option<&leptoskit_yoga::Style> {
	thread_local! {
		static DEFAULT: leptoskit_yoga::Style = leptoskit_yoga::Style::default();
	}
	DEFAULT.with(|default| style != default).then_some(style)
}

/// Encodes one node's wire payload into `wire_arena` and returns its byte
/// range.
fn encode_node_wire(
	wire_arena: &mut Vec<u8>,
	element: &Element,
	prev_rive: Option<&leptoskit_rive::Playback>,
) -> (u32, u32) {
	// Reuse only this node's previously transmitted scene. The host
	// never looks up global asset identifiers or retains a separate
	// file cache.
	let retained = element
		.props
		.rive
		.as_ref()
		.filter(|content| prev_rive.is_some_and(|old| old.same_scene(content)));
	#[cfg(feature = "devtools")]
	let source = element.source_location().map(|location| {
		format!(
			"{}:{}:{}",
			location.file(),
			location.line(),
			location.column()
		)
	});
	#[cfg(not(feature = "devtools"))]
	let source: Option<String> = None;
	#[cfg(feature = "devtools")]
	let component = element.component_name();
	#[cfg(not(feature = "devtools"))]
	let component: Option<&str> = None;

	let start = wire_arena.len() as u32;
	let rive_inputs = retained.map(|content| content.inputs());
	crate::wire::encode_binary_into(
		wire_arena,
		&element.props,
		sent_style(&element.style),
		element.classes().classes(),
		retained.is_some(),
		rive_inputs,
		source.as_deref(),
		component,
	);
	let end = wire_arena.len() as u32;
	(start, end)
}

impl Tree {
	/// Whether a refresh or a region can update this tree without rebuilding
	/// it.
	///
	/// The wasm runtime is the caller. Host tests build the tree without it,
	/// so the method looks unused there.
	#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
	pub(super) fn has_dynamic(&self) -> bool {
		!self.bound.is_empty() || !self.regions.is_empty()
	}

	/// Validates and prepares the complete tree before mutating the host.
	pub(super) fn update(&mut self, element: Element, host: &mut impl Host) {
		element
			.validate()
			.unwrap_or_else(|error| panic!("invalid leptoskit element tree: {error}"));
		self.order.clear();
		self.removed.clear();
		self.bound.clear();
		self.wire_arena.clear();
		self.build_ops.clear();
		self.regions.clear();
		let prev_root = self.root;
		self.root = Some(self.build_in_place(element, None, prev_root, 1));
		self.publish_in_place(host);
		self.wire_arena.clear();
		self.build_ops.clear();
		self.removed.clear();
	}

	/// Applies regions a signal write queued, then the bindings.
	///
	/// A list or a condition rebuilds only its own children. Bindings then
	/// rewrite the nodes whose value differs, in the same turn, so a handler
	/// that changes both pays for both and for nothing around them.
	pub(super) fn settle(&mut self, host: &mut impl Host) {
		// A region that runs while settling can queue another. A handful of
		// passes covers a list whose items are themselves lists; past that the
		// closure is writing a signal it reads, and stopping beats looping.
		for _ in 0..16 {
			let dirty = drain_dirty();

			if dirty.is_empty() {
				break;
			}

			for id in dirty {
				let Some(slot) = self.regions.get(&id).cloned() else {
					continue;
				};
				let children = slot.evaluate();
				self.splice(id, children, host);
			}
		}

		self.refresh_reactive(host);
	}

	/// Replaces the children of `parent` with `children`, reusing a child that
	/// keeps its key and its kind.
	///
	/// Only the region's own subtree is moved out and rebuilt; the rest of the
	/// tree stays where it is, so a list that changes costs its own rows
	/// rather than the size of the screen around it.
	fn splice(&mut self, parent: i64, children: Vec<(u64, Element)>, host: &mut impl Host) {
		let Some(previous_children) = self.nodes.get(&parent).map(|node| node.children.clone())
		else {
			return;
		};

		// The replaced subtree, parents before children, moved rather than
		// copied into a tree of its own for the rebuild to reuse nodes from.
		let replaced = descendants(self, parent);
		let mut old = Self {
			nodes: HashMap::with_capacity_and_hasher(replaced.len(), FastHash::default()),
			..Self::default()
		};

		for &id in &replaced {
			if let Some(node) = self.nodes.remove(&id) {
				old.nodes.insert(id, node);
			}

			// A reused node subscribes again if it is still a region, and a
			// dropped slot unsubscribes.
			self.regions.remove(&id);
		}

		if !replaced.is_empty() {
			let gone: HashSet<i64, FastHash> = replaced.iter().copied().collect();
			self.order.retain(|id| !gone.contains(id));
			self.bound.retain(|id| !gone.contains(id));
		}

		let old_keys: HashMap<u64, i64, FastHash> = previous_children
			.iter()
			.filter_map(|id| {
				old.nodes
					.get(id)
					.and_then(|node| node.key.map(|key| (key, *id)))
			})
			.collect();
		let child_depth = depth_of(self, parent) + 1;
		self.build_ops.clear();
		self.wire_arena.clear();
		let mut child_ids = Vec::with_capacity(children.len());

		for (key, child) in children {
			let previous = old_keys.get(&key).copied().filter(|id| {
				old.nodes
					.get(id)
					.is_some_and(|node| node.element.kind == child.kind)
			});
			let id = self.build(child, Some(parent), previous, &old, child_depth);

			if let Some(node) = self.nodes.get_mut(&id) {
				node.key = Some(key);
			}

			child_ids.push(id);
		}

		// Delete leaves and trailing siblings first so the host detaches each
		// removed child from the tail of its parent's list in O(1).
		for id in replaced
			.iter()
			.rev()
			.filter(|id| !self.nodes.contains_key(id))
		{
			host.delete(*id);
		}

		for op in &self.build_ops {
			if op.is_new {
				host.create(op.id, op.kind);
			}

			if let Some((start, end)) = op.wire {
				host.props(op.id, &self.wire_arena[start as usize..end as usize]);
			}
		}

		self.wire_arena.clear();

		if previous_children != child_ids {
			for (index, &child) in child_ids.iter().enumerate() {
				host.insert(parent, child, index as i32);
			}
		}

		if let Some(node) = self.nodes.get_mut(&parent) {
			node.children = child_ids;
		}

		for op in &self.build_ops {
			if !op.children_changed {
				continue;
			}

			let node = &self.nodes[&op.id];

			for (index, &child) in node.children.iter().enumerate() {
				host.insert(op.id, child, index as i32);
			}
		}

		self.build_ops.clear();
		host.commit();
	}

	/// Immediately removes an un-reused subtree from `self.nodes` in forward
	/// build order and records its ids in `self.removed` so `self.nodes`
	/// reclaims dead slots before new siblings trigger a table growth.
	fn evict_subtree(&mut self, root: i64) {
		self.evict_stack.clear();
		self.evict_stack.push(root);
		self.drain_evict_stack();
	}

	/// Evicts a slice of trailing sibling subtrees in forward build order,
	/// reusing `self.evict_stack` with zero per-sibling allocations.
	fn evict_slice(&mut self, roots: &[i64]) {
		self.evict_stack.clear();
		self.evict_stack.extend(roots.iter().rev().copied());
		self.drain_evict_stack();
	}

	fn drain_evict_stack(&mut self) {
		while let Some(id) = self.evict_stack.pop() {
			if let Some(node) = self.nodes.remove(&id) {
				self.removed.push(id);
				self.evict_stack.extend(node.children.iter().rev().copied());
			}
		}
	}

	/// Sends evicted deletes (leaves and trailing siblings first so the host
	/// detaches in O(1)), then staged creates, prop updates, and child
	/// insertions before committing.
	fn publish_in_place(&mut self, host: &mut impl Host) {
		for &id in self.removed.iter().rev() {
			host.delete(id);
		}

		for op in &self.build_ops {
			if op.is_new {
				host.create(op.id, op.kind);
			}

			if let Some((start, end)) = op.wire {
				host.props(op.id, &self.wire_arena[start as usize..end as usize]);
			}
		}

		for op in &self.build_ops {
			if !op.children_changed {
				continue;
			}

			let node = &self.nodes[&op.id];

			for (index, &child) in node.children.iter().enumerate() {
				host.insert(op.id, child, index as i32);
			}
		}

		host.commit();
	}

	/// Sends the nodes whose bound value changed, without rebuilding the tree.
	///
	/// This is the update path for
	/// [`Element::text_from`](crate::Element::text_from)
	/// and [`Element::background_from`](crate::Element::background_from):
	/// reading each binding is a signal read, and a node is encoded only when
	/// its value actually differs, so a frame costs the nodes that changed
	/// rather than every element in the tree.
	pub(super) fn refresh_reactive(&mut self, host: &mut impl Host) -> bool {
		let mut changed = false;

		for index in 0..self.bound.len() {
			let id = self.bound[index];
			let node = self.nodes.get_mut(&id).expect("every bound id has a node");
			let text = node.element.bindings().text.as_ref().map(Binding::read);
			let background = node
				.element
				.bindings()
				.background
				.as_ref()
				.map(Binding::read);
			let mut node_changed = false;

			if let Some(text) = text
				&& node.element.props.text.as_deref() != Some(text.as_str())
			{
				node.element.props.text = Some(text);
				node_changed = true;
			}

			if let Some(background) = background
				&& node.element.props.background != Some(background)
			{
				node.element.props.background = Some(background);
				node_changed = true;
			}

			if !node_changed {
				continue;
			}

			self.wire_arena.clear();
			crate::wire::encode_binary_into(
				&mut self.wire_arena,
				&node.element.props,
				sent_style(&node.element.style),
				node.element.classes().classes(),
				false,
				None,
				// The host kept the source from the node's first render.
				None,
				None,
			);
			host.props(id, &self.wire_arena);
			changed = true;
		}

		if changed {
			self.wire_arena.clear();
			host.commit();
		}

		changed
	}

	/// Reconciles `element` into `self.nodes` in place, reusing existing
	/// same-kind node slots and child `Vec`s without allocating a replacement
	/// map.
	fn build_in_place(
		&mut self,
		mut element: Element,
		parent: Option<i64>,
		previous: Option<i64>,
		depth: usize,
	) -> i64 {
		assert!(depth <= 256, "the native guest tree exceeds 256 levels");
		let kind = kind_code(element.kind);

		// Fast path for reused leaf nodes (e.g. Text): one map lookup, zero Vec
		// allocations, and no staged BuildOp unless props/style/classes
		// changed.
		if element.children.is_empty()
			&& element.region.is_none()
			&& let Some(prev_id) = previous
			&& let Some(node) = self.nodes.get_mut(&prev_id)
			&& node.element.kind == element.kind
			&& node.children.is_empty()
		{
			node.parent = parent;
			node.key = None;
			self.order.push(prev_id);

			if !element.bindings().is_empty() {
				self.bound.push(prev_id);
			}

			let changed = !node.element.classes().same_classes(element.classes())
				|| node.element.props != element.props
				|| node.element.style != element.style
				|| moved(&node.element, &element);

			if changed {
				let prev_rive = node.element.props.rive.as_deref();
				let wire = encode_node_wire(&mut self.wire_arena, &element, prev_rive);
				self.build_ops.push(BuildOp {
					id: prev_id,
					kind,
					is_new: false,
					children_changed: false,
					wire: Some(wire),
				});
			}

			node.element = element;
			return prev_id;
		}

		let reused = previous.filter(|id| {
			self.nodes
				.get(id)
				.is_some_and(|node| node.element.kind == element.kind)
		});
		let is_new = reused.is_none();

		if is_new && let Some(old_id) = previous {
			self.evict_subtree(old_id);
		}

		let id = if let Some(id) = reused {
			id
		} else {
			self.next_id = self
				.next_id
				.checked_add(1)
				.expect("guest node ids exhausted");
			self.next_id
		};
		self.order.push(id);
		let op_index = self.build_ops.len();
		self.build_ops.push(BuildOp {
			id,
			kind,
			is_new,
			children_changed: false,
			wire: None,
		});

		if !element.bindings().is_empty() {
			self.bound.push(id);
		}

		let mut child_keys = Vec::new();

		if let Some(region) = element.region.clone() {
			let slot = RegionSlot::new(id, region);
			let produced = slot.evaluate();
			child_keys = produced.iter().map(|(key, _)| *key).collect();
			element.children = produced.into_iter().map(|(_, child)| child).collect();
			self.regions.insert(id, slot);
		}

		let mut children = if is_new {
			Vec::new()
		} else {
			let node = self.nodes.get_mut(&id).expect("reused node exists");
			std::mem::take(&mut node.children)
		};
		let raw_children = std::mem::take(&mut element.children);
		let old_len = children.len();
		let new_len = raw_children.len();
		let mut children_changed = is_new || old_len != new_len;
		children.reserve(new_len.saturating_sub(old_len));

		for (index, child) in raw_children.into_iter().enumerate() {
			let previous_child = (index < old_len).then(|| children[index]);
			let child_id = self.build_in_place(child, Some(id), previous_child, depth + 1);

			if index < old_len {
				if children[index] != child_id {
					children_changed = true;
					children[index] = child_id;
				}
			} else {
				children.push(child_id);
			}
		}

		if old_len > new_len {
			self.evict_slice(&children[new_len..old_len]);
		}

		children.truncate(new_len);

		if children.is_empty() {
			children_changed = false;
		}

		for (child, key) in children.iter().zip(child_keys) {
			if let Some(node) = self.nodes.get_mut(child) {
				node.key = Some(key);
			}
		}

		if is_new {
			let wire = Some(encode_node_wire(&mut self.wire_arena, &element, None));
			self.build_ops[op_index].children_changed = children_changed;
			self.build_ops[op_index].wire = wire;
			self.nodes.insert(
				id,
				Node {
					element,
					parent,
					children,
					key: None,
				},
			);
		} else {
			let node = self.nodes.get_mut(&id).expect("reused node exists");
			let changed = !node.element.classes().same_classes(element.classes())
				|| node.element.props != element.props
				|| node.element.style != element.style
				|| moved(&node.element, &element);
			let wire = changed.then(|| {
				let prev_rive = node.element.props.rive.as_deref();
				encode_node_wire(&mut self.wire_arena, &element, prev_rive)
			});
			self.build_ops[op_index].children_changed = children_changed;
			self.build_ops[op_index].wire = wire;
			node.element = element;
			node.parent = parent;
			node.children = children;
			node.key = None;
		}

		id
	}

	/// Reuses the host renderer's positional, same-kind identity convention
	/// when splicing a region subtree out of `old`.
	fn build(
		&mut self,
		mut element: Element,
		parent: Option<i64>,
		previous: Option<i64>,
		old: &Self,
		depth: usize,
	) -> i64 {
		assert!(depth <= 256, "the native guest tree exceeds 256 levels");
		// Custom kinds have no code in this ABI; failing here avoids silently
		// rendering an unrelated container on the host.
		let kind = kind_code(element.kind);
		let previous = previous
			.and_then(|id| old.nodes.get(&id).map(|node| (id, node)))
			.filter(|(_, node)| node.element.kind == element.kind);
		let is_new = previous.is_none();
		let id = if let Some((id, _)) = previous {
			id
		} else {
			self.next_id = self
				.next_id
				.checked_add(1)
				.expect("guest node ids exhausted");
			self.next_id
		};
		self.order.push(id);
		let op_index = self.build_ops.len();
		self.build_ops.push(BuildOp {
			id,
			kind,
			is_new,
			children_changed: false,
			wire: None,
		});

		if !element.bindings().is_empty() {
			self.bound.push(id);
		}

		// A region subscribes here, so a later write queues this node. The
		// read is its own observer: it must not subscribe the screen that is
		// being built around it.
		let mut child_keys = Vec::new();

		if let Some(region) = element.region.clone() {
			let slot = RegionSlot::new(id, region);
			let produced = slot.evaluate();
			child_keys = produced.iter().map(|(key, _)| *key).collect();
			element.children = produced.into_iter().map(|(_, child)| child).collect();
			self.regions.insert(id, slot);
		}

		let children: Vec<i64> = std::mem::take(&mut element.children)
			.into_iter()
			.enumerate()
			.map(|(index, child)| {
				let previous_child = previous.and_then(|(_, node)| node.children.get(index));
				self.build(child, Some(id), previous_child.copied(), old, depth + 1)
			})
			.collect();

		for (child, key) in children.iter().zip(child_keys) {
			if let Some(node) = self.nodes.get_mut(child) {
				node.key = Some(key);
			}
		}

		let children_changed =
			previous.is_none_or(|(_, node)| node.children != children) && !children.is_empty();

		// Comparing shared drawings avoids serializing unchanged paths.
		let changed = previous.is_none_or(|(_, node)| {
			!node.element.classes().same_classes(element.classes())
				|| node.element.props != element.props
				|| node.element.style != element.style
				|| moved(&node.element, &element)
		});
		let wire = changed.then(|| {
			let prev_rive = previous.and_then(|(_, node)| node.element.props.rive.as_deref());
			encode_node_wire(&mut self.wire_arena, &element, prev_rive)
		});
		self.build_ops[op_index].children_changed = children_changed;
		self.build_ops[op_index].wire = wire;
		self.nodes.insert(
			id,
			Node {
				element,
				parent,
				children,
				key: None,
			},
		);
		id
	}

	/// The link a press at `id` is on: the nearest pressable around it that
	/// has one.
	pub(super) fn pressed_link(&self, id: i64) -> Option<String> {
		let mut current = Some(id);

		while let Some(id) = current {
			let node = self.nodes.get(&id)?;

			if node.element.kind == NativeKind::Pressable {
				return node.element.props.href().map(str::to_owned);
			}

			current = node.parent;
		}

		None
	}

	/// Snapshots callbacks so application handlers may request another render.
	pub(super) fn handlers(
		&self,
		id: i64,
		event: NativeEvent,
	) -> Option<(NativeEvent, Vec<Vec<HandlerEntry>>)> {
		let target = self.nodes.get(&id)?;
		let event = normalize_event(&target.element, event)?;
		let mut current = Some(id);
		let mut handlers = Vec::new();

		while let Some(id) = current {
			let node = &self.nodes[&id];

			if node.element.props.hidden == Some(true)
				|| node.element.props.enabled == Some(false)
				|| (node.element.kind == NativeKind::ActivityIndicator
					&& node.element.props.running == Some(false))
			{
				return None;
			}

			handlers.push(node.element.handlers().to_vec());
			current = node.parent;
		}

		Some((event, handlers))
	}
}

/// Runs every matching handler at a node before deciding whether to bubble.
pub(super) fn invoke(event: &NativeEvent, handlers: &[Vec<HandlerEntry>]) -> EventOutcome {
	for entries in handlers {
		let mut handled = false;

		for entry in entries {
			if entry.event.matches(event) && (entry.handler)(event) == EventOutcome::Handled {
				handled = true;
			}
		}

		if handled {
			return EventOutcome::Handled;
		}
	}

	EventOutcome::Unhandled
}

/// Interprets only the payload schema belonging to the selected ABI event.
pub(super) fn decode_event(kind: i32, payload: &[u8]) -> Option<NativeEvent> {
	#[derive(Deserialize)]
	struct Text {
		value: String,
	}

	#[derive(Deserialize)]
	struct Toggle {
		checked: bool,
	}

	#[derive(Deserialize)]
	struct Value {
		value: f64,
	}

	#[derive(Deserialize)]
	struct Scroll {
		x: f32,
		y: f32,
	}

	match kind {
		0 => Some(NativeEvent::Press),
		1 | 2 => {
			let Text { value } = serde_json::from_slice(payload).ok()?;

			Some(if kind == 1 {
				NativeEvent::Change { value }
			} else {
				NativeEvent::Submit { value }
			})
		}
		3 => Some(NativeEvent::Focus),
		4 => Some(NativeEvent::Blur),
		5 => {
			let Scroll { x, y } = serde_json::from_slice(payload).ok()?;

			(x.is_finite() && y.is_finite()).then_some(NativeEvent::Scroll {
				offset_x: x,
				offset_y: y,
			})
		}
		6 => {
			let Toggle { checked } = serde_json::from_slice(payload).ok()?;
			Some(NativeEvent::Toggle { checked })
		}
		7 => {
			let Value { value } = serde_json::from_slice(payload).ok()?;
			value
				.is_finite()
				.then_some(NativeEvent::ValueChange { value })
		}
		8 => serde_json::from_slice(payload).ok().map(NativeEvent::Key),
		9 => Some(NativeEvent::Dismiss),
		_ => None,
	}
}

/// Applies controlled-widget proposal rules before application handlers run.
fn normalize_event(element: &Element, event: NativeEvent) -> Option<NativeEvent> {
	match event {
		NativeEvent::Toggle { checked } => {
			(matches!(
				element.kind,
				NativeKind::Switch | NativeKind::Checkbox | NativeKind::Radio
			) && (element.kind != NativeKind::Radio || checked))
				.then_some(event)
		}
		NativeEvent::ValueChange { value } => {
			if element.kind != NativeKind::Slider {
				return None;
			}

			element
				.props
				.slider
				.unwrap_or_default()
				.proposed_value(value)
				.map(|value| NativeEvent::ValueChange { value })
		}
		_ => Some(event),
	}
}

/// Every node under `parent`, not including it, in the order a build visits
/// them: each node before its children, siblings first to last. The list is
/// the subtree a region replaces, and the order its nodes are deleted in.
fn descendants(tree: &Tree, parent: i64) -> Vec<i64> {
	let mut out = Vec::new();
	let mut stack: Vec<i64> = tree
		.nodes
		.get(&parent)
		.map(|node| node.children.iter().rev().copied().collect())
		.unwrap_or_default();

	while let Some(id) = stack.pop() {
		if let Some(node) = tree.nodes.get(&id) {
			stack.extend(node.children.iter().rev().copied());
		}

		out.push(id);
	}

	out
}

/// How deep `id` sits, the root being one. The guest refuses a tree past 256.
fn depth_of(tree: &Tree, id: i64) -> usize {
	let mut depth = 1;
	let mut current = tree.nodes.get(&id).and_then(|node| node.parent);

	while let Some(id) = current {
		depth += 1;
		current = tree.nodes.get(&id).and_then(|node| node.parent);
	}

	depth
}

/// The codes of the native UI ABI, independent of enum declaration order.
fn kind_code(kind: NativeKind) -> i32 {
	crate::wire::kind_code(kind)
		.unwrap_or_else(|| panic!("custom widget `{kind}` has no native guest ABI code"))
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod wire_tests;
