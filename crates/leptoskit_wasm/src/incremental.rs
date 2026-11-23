//! Incremental commits: rebuild only the wire nodes the guest changed since
//! its last commit.
//!
//! The guest already sends only what changed, but assembling the whole wire
//! tree on every commit made a one-label update cost as much as rendering
//! every node. The host now records which nodes changed (`dirty_props`) and
//! which child lists changed (`dirty_structure`), and a commit hands the
//! surface one [`SourcePatch`] per changed region.
//!
//! The committed wire tree is mirrored in [`Committed`], apart from the
//! guest's mutable forest, so widgets pair with wire ids even while the guest
//! holds uncommitted edits.

use std::collections::HashMap;
use std::collections::hash_map::Entry;

use leptoskit_native::Element;
use leptoskit_native::NativeBackend;
use leptoskit_native::NativeKind;
use leptoskit_native::NativeSurface;
use leptoskit_native::SourcePatch;
use leptoskit_yoga::NodeId;

use super::FastHash;
use super::HostNode;
use super::HostState;
use super::MAX_TREE_DEPTH;
use super::host_trap;

/// The wire tree the surface last committed.
#[derive(Debug, Default)]
pub(super) struct Committed {
	/// The committed root, or `None` before the first commit and after a
	/// patch is rejected, which forces the next commit to be complete.
	pub root: Option<i64>,
	/// Committed child lists. Leaves have no entry.
	pub children: HashMap<i64, Vec<i64>, FastHash>,
	/// The surface node built from each committed wire node, so its length
	/// is the number of committed nodes.
	pub node_by_wire: HashMap<i64, NodeId, FastHash>,
}

impl Committed {
	/// The committed subtree under `from`, in pre-order.
	pub fn pre_order(&self, from: i64) -> Vec<i64> {
		let mut order = Vec::new();
		let mut stack = vec![from];

		while let Some(id) = stack.pop() {
			order.push(id);

			if let Some(children) = self.children.get(&id) {
				stack.extend(children.iter().rev());
			}
		}

		order
	}

	/// Replaces the committed tree with `structure`, a pre-order list of
	/// every node and its children.
	pub fn reset(&mut self, root: i64, structure: Structure) {
		self.root = Some(root);
		self.children.clear();
		self.children.extend(
			structure
				.into_iter()
				.filter(|(_, children)| !children.is_empty()),
		);
	}
}

/// Every node of an assembled subtree and its children, in pre-order.
pub(super) type Structure = Vec<(i64, Vec<i64>)>;

/// A changed region of the wire tree, assembled when the guest committed.
#[derive(Debug)]
pub(super) struct WirePatch {
	pub wire: i64,
	pub element: Element,
	/// For a subtree patch, every assembled node and its children in
	/// pre-order; `None` for a patch of the node alone.
	pub structure: Option<Structure>,
}

impl WirePatch {
	/// The number of wire nodes the patch renders.
	pub fn nodes(&self) -> usize {
		self.structure.as_ref().map_or(1, Vec::len)
	}
}

impl HostState {
	/// Creates `id` if it is new, as a parentless root until it is inserted.
	pub fn ensure_node(&mut self, id: i64) -> &mut HostNode {
		match self.nodes.entry(id) {
			Entry::Occupied(entry) => entry.into_mut(),
			Entry::Vacant(entry) => {
				self.roots.insert(id);
				entry.insert(HostNode::default())
			}
		}
	}

	pub fn clear_dirty(&mut self) {
		for id in self.dirty_props.drain(..) {
			if let Some(node) = self.nodes.get_mut(&id) {
				node.dirty_props = false;
			}
		}

		for id in self.dirty_structure.drain(..) {
			if let Some(node) = self.nodes.get_mut(&id) {
				node.dirty_structure = false;
			}
		}
	}

	/// Plans the patches for a commit of `root`, or `None` when the commit
	/// must assemble the whole tree: nothing is committed yet, the root
	/// changed, or the tree declares accessibility relationships, which
	/// resolve across every node.
	pub fn plan_patches(&mut self, root: i64) -> Result<Option<Vec<WirePatch>>, wasmi::Error> {
		if self.committed.root != Some(root) || self.relationships {
			return Ok(None);
		}

		let mut subtrees = Vec::with_capacity(self.dirty_structure.len());
		let mut nodes = Vec::with_capacity(self.dirty_props.len());

		for index in 0..self.dirty_structure.len() {
			let id = self.dirty_structure[index];

			if self.is_patchable(id, root)? {
				subtrees.push(id);
			}
		}

		for index in 0..self.dirty_props.len() {
			let id = self.dirty_props[index];

			if !self.nodes.get(&id).is_some_and(|node| node.dirty_structure)
				&& self.is_patchable(id, root)?
			{
				nodes.push(id);
			}
		}

		// A region inside a rebuilt subtree is rebuilt with it.
		let mut regions = Vec::with_capacity(subtrees.len() + nodes.len());

		for (id, subtree) in subtrees
			.iter()
			.map(|&id| (id, true))
			.chain(nodes.iter().map(|&id| (id, false)))
		{
			if !self.has_dirty_structure_ancestor(id) {
				regions.push((id, subtree));
			}
		}

		let mut patches = Vec::with_capacity(regions.len());

		for (id, subtree) in regions {
			let patch = if subtree {
				let (element, structure) = self.assemble_subtree(id, true)?;
				WirePatch {
					wire: id,
					element,
					structure: Some(structure),
				}
			} else {
				self.charge_node(id)?;
				WirePatch {
					wire: id,
					element: self.element_for(id)?,
					structure: None,
				}
			};

			if !patch.element.is_self_contained() {
				return Ok(None);
			}

			// An invalid region traps the commit, as an invalid tree does,
			// before any widget changes.
			patch
				.element
				.validate_within(self.button_ancestor(id))
				.map_err(|error| host_trap(error.to_string()))?;
			patches.push(patch);
		}

		Ok(Some(patches))
	}

	/// Whether a changed node is both committed and still in the tree under
	/// `root`. Nodes outside either are covered by a changed ancestor's
	/// child list, or are not shown at all.
	fn is_patchable(&mut self, id: i64, root: i64) -> Result<bool, wasmi::Error> {
		if !self.committed.node_by_wire.contains_key(&id) || !self.nodes.contains_key(&id) {
			return Ok(false);
		}

		let mut current = id;

		for _ in 0..=MAX_TREE_DEPTH {
			self.budget.charge(8)?;

			if current == root {
				return Ok(true);
			}

			match self.nodes.get(&current).and_then(|node| node.parent) {
				Some(parent) => current = parent,
				None => return Ok(false),
			}
		}

		Ok(false)
	}

	/// The nearest `Button` or `Pressable` above `id`.
	fn button_ancestor(&self, id: i64) -> Option<NativeKind> {
		let mut current = self.nodes.get(&id).and_then(|node| node.parent);

		while let Some(id) = current {
			let Some(node) = self.nodes.get(&id) else {
				break;
			};

			if let Some(kind @ (NativeKind::Button | NativeKind::Pressable)) = node.kind {
				return Some(kind);
			}

			current = node.parent;
		}

		None
	}

	fn has_dirty_structure_ancestor(&self, id: i64) -> bool {
		let mut current = self.nodes.get(&id).and_then(|node| node.parent);

		while let Some(id) = current {
			let Some(node) = self.nodes.get(&id) else {
				break;
			};

			if node.dirty_structure {
				return true;
			}

			current = node.parent;
		}

		false
	}

	fn charge_node(&mut self, id: i64) -> Result<(), wasmi::Error> {
		let bytes = self.nodes.get(&id).map_or(0, |node| node.property_bytes);
		self.budget.charge(256 + bytes)
	}

	/// The element for one wire node, without children.
	pub fn element_for(&self, id: i64) -> Result<Element, wasmi::Error> {
		let Some(node) = self.nodes.get(&id) else {
			return Ok(Element::new(NativeKind::View));
		};
		Self::element_from_node(id, node)
	}

	fn element_from_node(id: i64, node: &HostNode) -> Result<Element, wasmi::Error> {
		let mut element = Element::new(node.kind.unwrap_or(NativeKind::View));
		element.props = node.wire.props.clone();
		element.style = node.wire.style.clone();

		if !node.wire.class.is_empty() {
			element = element
				.try_class(&node.wire.class)
				.map_err(|error| host_trap(format!("node {id}: {error}")))?;
		}

		Ok(element)
	}

	/// Assembles the subtree under `root` with its structure in pre-order,
	/// charging the render budget for each node. The traversal uses heap
	/// storage so an invalid tree is rejected without recursing through its
	/// untrusted depth.
	pub fn assemble_subtree(
		&mut self,
		root: i64,
		charge: bool,
	) -> Result<(Element, Structure), wasmi::Error> {
		let mut stack = vec![(root, 1)];
		let mut structure = Vec::new();
		let mut elements = Vec::new();

		while let Some((id, depth)) = stack.pop() {
			if depth > MAX_TREE_DEPTH {
				return Err(host_trap(format!(
					"the committed tree is deeper than {MAX_TREE_DEPTH} levels"
				)));
			}

			let (bytes, element, children) = match self.nodes.get(&id) {
				Some(node) => {
					(
						node.property_bytes,
						Self::element_from_node(id, node)?,
						node.children.clone(),
					)
				}
				None => (0, Element::new(NativeKind::View), Vec::new()),
			};

			if charge {
				self.budget.charge(256 + bytes)?;
			}

			elements.push((element, children.len()));
			stack.extend(children.iter().rev().map(|&child| (child, depth + 1)));
			structure.push((id, children));
		}

		let mut assembled = Vec::with_capacity(elements.len());

		for (mut element, child_count) in elements.into_iter().rev() {
			if child_count > 0 {
				element.children.reserve(child_count);

				for _ in 0..child_count {
					element
						.children
						.push(assembled.pop().expect("children precede their parent"));
				}
			}

			assembled.push(element);
		}

		let tree = assembled.pop().expect("the traversal includes its root");
		debug_assert!(assembled.is_empty(), "the traversal describes one tree");
		Ok((tree, structure))
	}
}

/// Hands planned patches to the surface. Returns `Ok(false)`, with nothing
/// changed, when the surface asks for a complete commit instead.
pub(super) fn apply<B: NativeBackend>(
	surface: &mut NativeSurface<B>,
	state: &mut HostState,
	patches: Vec<WirePatch>,
) -> Result<bool, wasmi::Error> {
	let mut planned = Vec::with_capacity(patches.len());
	let mut sources = Vec::with_capacity(patches.len());

	for patch in patches {
		let Some(&node) = state.committed.node_by_wire.get(&patch.wire) else {
			return Ok(false);
		};
		sources.push(if patch.structure.is_some() {
			SourcePatch::Subtree {
				node,
				element: patch.element,
			}
		} else {
			SourcePatch::Node {
				node,
				element: patch.element,
			}
		});

		planned.push((patch.wire, node, patch.structure));
	}

	let held = surface.pressed().is_some();
	let Some(replaced) = surface
		.try_patch(sources)
		.map_err(|error| host_trap(error.to_string()))?
	else {
		return Ok(false);
	};

	for ((wire, before, structure), after) in planned.into_iter().zip(replaced) {
		match structure {
			Some(structure) => {
				for old in state.committed.pre_order(wire) {
					state.committed.children.remove(&old);
					unmap(state, old);
				}

				let order: Vec<i64> = structure.iter().map(|(id, _)| *id).collect();
				state.committed.children.extend(
					structure
						.into_iter()
						.filter(|(_, children)| !children.is_empty()),
				);
				map_subtree(surface, state, after, &order);
			}
			// A node that kept its widget keeps every widget below it.
			None if before == after => {}
			None => {
				let order = state.committed.pre_order(wire);

				for &old in &order {
					unmap(state, old);
				}

				map_subtree(surface, state, after, &order);
			}
		}
	}

	// A held control is resolved by a complete commit, which can rebuild
	// widgets anywhere along its path.
	if held {
		super::resync(surface, state);
	}

	Ok(true)
}

fn unmap(state: &mut HostState, wire: i64) {
	if let Some(node) = state.committed.node_by_wire.remove(&wire) {
		state.clear_wire_by_node(node.index());
	}
}

/// Pairs the surface subtree under `node` with wire ids in pre-order.
fn map_subtree<B: NativeBackend>(
	surface: &NativeSurface<B>,
	state: &mut HostState,
	node: NodeId,
	order: &[i64],
) {
	let mut wires = order.iter().copied();
	let mut stack = vec![node];

	while let Some(node) = stack.pop() {
		let wire = wires.next().expect("surface and wire tree diverged");
		state.set_wire_by_node(node.index(), wire);
		state.committed.node_by_wire.insert(wire, node);
		stack.extend(surface.children(node).iter().rev());
	}

	debug_assert!(wires.next().is_none(), "surface and wire tree diverged");
}
