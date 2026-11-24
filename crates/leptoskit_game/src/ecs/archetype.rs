//! Archetype tables and pooled zero-clone O(1) structural transition edges.
//!
//! Entities sharing the exact same set of component types live in one
//! [`Archetype`], where each component type occupies a contiguous
//! [`TypedColumn`](crate::ecs::column::TypedColumn). Adding or removing a
//! component follows a cached pooled transition edge containing a precomputed
//! column index map, avoiding repeated sorting, hashing, cloning, or binary
//! searches.

use crate::ecs::column::Column;
use crate::ecs::column::downcast_col;
use crate::ecs::column::downcast_col_mut;
use crate::ecs::component::Component;
use crate::ecs::component::ComponentId;
use crate::ecs::component::ComponentMask;
use crate::ecs::component::ComponentRegistry;
use crate::ecs::entity::Entity;
use crate::ecs::hash::FastMap;

/// Dense index into the world's archetype vector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ArchetypeId(pub u32);

impl ArchetypeId {
	/// The empty archetype holding entities with zero components.
	pub const EMPTY: Self = Self(0);
	/// Sentinel for unassigned entity slots.
	pub const INVALID: Self = Self(u32::MAX);
}

/// Cached transition when inserting one new component into an archetype.
#[derive(Debug, Clone)]
pub(crate) struct AddEdge {
	pub target: ArchetypeId,
	/// Pairs of `(src_col_idx, dst_col_idx)` for all existing components.
	pub column_map: Box<[(usize, usize)]>,
	/// Column index in `target` where the newly added component goes.
	pub added_col: usize,
}

/// Cached transition when removing one component from an archetype.
#[derive(Debug, Clone)]
pub(crate) struct RemoveEdge {
	pub target: ArchetypeId,
	/// Pairs of `(src_col_idx, dst_col_idx)` for all surviving components.
	pub column_map: Box<[(usize, usize)]>,
	/// Column index in `src` of the component being removed.
	pub removed_col: usize,
}

/// A dense columnar table storing all entities with one exact component set.
#[doc(hidden)]
pub struct Archetype {
	pub(crate) id: ArchetypeId,
	pub(crate) components: Box<[ComponentId]>,
	pub(crate) mask: ComponentMask,
	/// Direct `ComponentId -> column index` table (`u16::MAX` if absent).
	col_lookup: Vec<u16>,
	pub(crate) entities: Vec<Entity>,
	pub(crate) columns: Vec<Box<dyn Column>>,
	pub(crate) add_edges: FastMap<ComponentId, usize>,
	pub(crate) remove_edges: FastMap<ComponentId, usize>,
}

impl Archetype {
	/// Creates a new archetype for a sorted, deduplicated list of `components`.
	pub(crate) fn new(
		id: ArchetypeId,
		components: Box<[ComponentId]>,
		registry: &ComponentRegistry,
	) -> Self {
		let mut mask = ComponentMask::default();
		let max_id = components.last().map_or(0, |c| c.0 as usize + 1);
		let mut col_lookup = vec![u16::MAX; max_id];
		let mut columns = Vec::with_capacity(components.len());
		for (col_idx, &comp_id) in components.iter().enumerate() {
			mask.set(comp_id);
			col_lookup[comp_id.0 as usize] =
				u16::try_from(col_idx).expect("archetype column count exceeded u16::MAX");
			columns.push(registry.make_column(comp_id));
		}
		Self {
			id,
			components,
			mask,
			col_lookup,
			entities: Vec::new(),
			columns,
			add_edges: FastMap::default(),
			remove_edges: FastMap::default(),
		}
	}

	/// Number of live entities currently stored in this archetype.
	#[inline]
	pub(crate) fn len(&self) -> usize {
		self.entities.len()
	}

	/// Whether this archetype currently holds zero entities.
	#[inline]
	pub(crate) fn is_empty(&self) -> bool {
		self.entities.is_empty()
	}

	/// O(1) direct lookup of the column index for `id`.
	#[inline]
	pub(crate) fn column_index(&self, id: ComponentId) -> Option<usize> {
		let idx = *self.col_lookup.get(id.0 as usize)?;
		(idx != u16::MAX).then_some(usize::from(idx))
	}

	/// Immutable typed slice of a column by its pre-resolved `col_idx`.
	#[inline]
	pub(crate) fn column_slice<T: Component>(&self, col_idx: usize) -> &[T] {
		downcast_col::<T>(self.columns[col_idx].as_ref()).as_slice()
	}

	/// Mutable typed slice of a column by `col_idx`, stamping column change
	/// tick once.
	#[inline]
	pub(crate) fn column_slice_mut<T: Component>(&mut self, col_idx: usize, tick: u32) -> &mut [T] {
		downcast_col_mut::<T>(self.columns[col_idx].as_mut()).as_mut_slice_unchecked_ticks(tick)
	}

	/// Swap-removes the entity at `row` and drops all its components.
	///
	/// Returns the entity that was moved into `row` (if `row` was not the last
	/// slot) so the allocator can update its row index.
	pub(crate) fn swap_remove_drop(&mut self, row: usize) -> Option<Entity> {
		let _ = self.entities.swap_remove(row);
		for col in &mut self.columns {
			col.swap_remove_drop(row);
		}
		self.entities.get(row).copied()
	}
}

/// Collection of all archetypes in a world, indexed by sorted component
/// signature.
pub(crate) struct Archetypes {
	list: Vec<Archetype>,
	by_signature: FastMap<Box<[ComponentId]>, ArchetypeId>,
	add_edges_pool: Vec<AddEdge>,
	remove_edges_pool: Vec<RemoveEdge>,
}

impl Archetypes {
	/// Initializes the archetype store with [`ArchetypeId::EMPTY`].
	pub fn new(registry: &ComponentRegistry) -> Self {
		let empty = Archetype::new(ArchetypeId::EMPTY, Box::from([]), registry);
		let mut by_signature: FastMap<Box<[ComponentId]>, ArchetypeId> = FastMap::default();
		by_signature.insert(Box::from([]), ArchetypeId::EMPTY);
		Self {
			list: vec![empty],
			by_signature,
			add_edges_pool: Vec::new(),
			remove_edges_pool: Vec::new(),
		}
	}

	/// Returns the archetype matching sorted, unique `components`, creating it
	/// if it does not yet exist.
	pub fn get_or_create(
		&mut self,
		components: &[ComponentId],
		registry: &ComponentRegistry,
	) -> ArchetypeId {
		if let Some(&id) = self.by_signature.get(components) {
			return id;
		}
		let id =
			ArchetypeId(u32::try_from(self.list.len()).expect("archetype count exceeded u32::MAX"));
		let boxed: Box<[ComponentId]> = components.into();
		let archetype = Archetype::new(id, boxed.clone(), registry);
		self.by_signature.insert(boxed, id);
		self.list.push(archetype);
		id
	}

	/// Resolves (and caches) the pooled transition edge index for adding
	/// `comp` to `src` with zero heap allocation on cache hit.
	#[inline]
	pub fn resolve_add_edge(
		&mut self,
		src: ArchetypeId,
		comp: ComponentId,
		registry: &ComponentRegistry,
	) -> usize {
		if let Some(&edge_idx) = self.list[src.0 as usize].add_edges.get(&comp) {
			return edge_idx;
		}
		let (next, pos, src_len) = {
			let src_components = &self.list[src.0 as usize].components;
			let src_len = src_components.len();
			let mut next = Vec::with_capacity(src_len + 1);
			let pos = src_components.partition_point(|&id| id < comp);
			next.extend_from_slice(&src_components[..pos]);
			next.push(comp);
			next.extend_from_slice(&src_components[pos..]);
			(next, pos, src_len)
		};

		let target = self.get_or_create(&next, registry);
		let column_map: Box<[(usize, usize)]> = (0..src_len)
			.map(|src_col| {
				let dst_col = if src_col < pos { src_col } else { src_col + 1 };
				(src_col, dst_col)
			})
			.collect();
		let edge_idx = self.add_edges_pool.len();
		self.add_edges_pool.push(AddEdge {
			target,
			column_map,
			added_col: pos,
		});
		self.list[src.0 as usize].add_edges.insert(comp, edge_idx);
		edge_idx
	}

	/// Resolves (and caches) the pooled transition edge index for removing
	/// `comp` from `src` with zero heap allocation on cache hit.
	#[inline]
	pub fn resolve_remove_edge(
		&mut self,
		src: ArchetypeId,
		comp: ComponentId,
		registry: &ComponentRegistry,
	) -> Option<usize> {
		if let Some(&edge_idx) = self.list[src.0 as usize].remove_edges.get(&comp) {
			return Some(edge_idx);
		}
		let (next, removed_col, src_len) = {
			let src_components = &self.list[src.0 as usize].components;
			let src_len = src_components.len();
			let removed_col = src_components.binary_search(&comp).ok()?;
			let mut next = Vec::with_capacity(src_len - 1);
			next.extend_from_slice(&src_components[..removed_col]);
			next.extend_from_slice(&src_components[removed_col + 1..]);
			(next, removed_col, src_len)
		};

		let target = self.get_or_create(&next, registry);
		let column_map: Box<[(usize, usize)]> = (0..src_len)
			.filter(|&src_col| src_col != removed_col)
			.enumerate()
			.map(|(dst_col, src_col)| (src_col, dst_col))
			.collect();
		let edge_idx = self.remove_edges_pool.len();
		self.remove_edges_pool.push(RemoveEdge {
			target,
			column_map,
			removed_col,
		});
		self.list[src.0 as usize]
			.remove_edges
			.insert(comp, edge_idx);
		Some(edge_idx)
	}

	/// Borrows an [`AddEdge`] and the `(src, dst)` archetype pair
	/// simultaneously with zero clones.
	#[inline]
	pub fn add_transition_mut(
		&mut self,
		src: ArchetypeId,
		edge_idx: usize,
	) -> (&AddEdge, &mut Archetype, &mut Archetype) {
		let edge = &self.add_edges_pool[edge_idx];
		let [first, second] = self
			.list
			.get_disjoint_mut([src.0 as usize, edge.target.0 as usize])
			.expect("archetype transition requires two distinct archetypes");
		(edge, first, second)
	}

	/// Borrows a [`RemoveEdge`] and the `(src, dst)` archetype pair
	/// simultaneously with zero clones.
	#[inline]
	pub fn remove_transition_mut(
		&mut self,
		src: ArchetypeId,
		edge_idx: usize,
	) -> (&RemoveEdge, &mut Archetype, &mut Archetype) {
		let edge = &self.remove_edges_pool[edge_idx];
		let [first, second] = self
			.list
			.get_disjoint_mut([src.0 as usize, edge.target.0 as usize])
			.expect("archetype transition requires two distinct archetypes");
		(edge, first, second)
	}

	#[inline]
	pub fn get(&self, id: ArchetypeId) -> &Archetype {
		&self.list[id.0 as usize]
	}

	#[inline]
	pub fn get_mut(&mut self, id: ArchetypeId) -> &mut Archetype {
		&mut self.list[id.0 as usize]
	}

	#[inline]
	pub fn len(&self) -> usize {
		self.list.len()
	}

	#[inline]
	pub fn as_slice(&self) -> &[Archetype] {
		&self.list
	}

	#[inline]
	pub fn as_mut_slice(&mut self) -> &mut [Archetype] {
		&mut self.list
	}
}
