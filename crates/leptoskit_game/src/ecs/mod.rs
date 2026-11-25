//! High-performance, 100% safe-Rust Archetype ECS, Schedule, Events, and
//! Godot-style Signals/Observers.
//!
//! Entities with identical component sets are stored in contiguous columnar
//! `SoA` [`Archetype`](archetype::Archetype) tables, enabling SIMD-friendly
//! iteration, O(1) cached archetype transitions, column-level change-detection
//! skipping, and parallel multi-core chunk execution on native targets.

#[doc(hidden)]
pub mod archetype;
pub mod bundle;
#[doc(hidden)]
pub mod column;
pub mod commands;
pub mod component;
pub mod entity;
pub mod events;
pub mod hash;
pub mod query;
pub mod schedule;

pub use archetype::ArchetypeId;
pub use bundle::Bundle;
pub use commands::Commands;
pub use component::Component;
pub use component::ComponentId;
pub use component::ComponentTicks;
pub use entity::Entity;
pub use events::EventCursor;
pub use events::EventId;
pub use events::Events;
pub use events::ObserverId;
pub use events::Trigger;
pub use query::Added;
pub use query::Changed;
pub use query::Mut;
pub use query::QueryFilter;
pub use query::QueryPlan;
pub use query::ReadQuery;
pub use query::With;
pub use query::Without;
pub use query::WriteQuery;
pub use schedule::GameApp;
pub use schedule::GamePlugin;
pub use schedule::NextState;
pub use schedule::Schedule;
pub use schedule::Stage;
pub use schedule::State;
pub use schedule::SystemAccess;
pub use schedule::SystemConfig;
pub use schedule::Time;

use crate::ecs::archetype::Archetypes;
use crate::ecs::bundle::MAX_BUNDLE_COLS;
use crate::ecs::bundle::canonicalize_bundle_ids;
use crate::ecs::column::downcast_col;
use crate::ecs::column::downcast_col_mut;
use crate::ecs::component::ComponentRegistry;
use crate::ecs::entity::EntityAllocator;
use crate::ecs::entity::EntityLocation;
use crate::ecs::hash::FastMap;
use crate::ecs::query::FilterMasks;

/// Inline stack-allocated cache entry for a bundle type: zero heap clone and
/// zero per-spawn `column_index` lookup.
#[derive(Clone, Copy)]
struct CachedBundle {
	archetype: ArchetypeId,
	cols: [usize; MAX_BUNDLE_COLS],
	len: usize,
}

/// Core archetype-ECS storage owned by [`World`](crate::World).
pub(crate) struct EcsStore {
	pub entities: EntityAllocator,
	pub registry: ComponentRegistry,
	pub archetypes: Archetypes,
	/// Cached bundle `TypeId -> CachedBundle` for O(1), zero-alloc spawns.
	bundle_cache: FastMap<std::any::TypeId, CachedBundle>,
	/// Godot-style unique entity names (`name -> Entity`) and reverse map.
	name_to_entity: FastMap<String, Entity>,
	entity_to_name: FastMap<Entity, String>,
	/// Godot-style entity groups (`group -> Vec<Entity>`).
	groups: FastMap<String, Vec<Entity>>,
	entity_groups: FastMap<Entity, Vec<String>>,
	/// Monotonic change-detection tick bumped on structural writes and systems.
	pub change_tick: u32,
}

impl Default for EcsStore {
	fn default() -> Self {
		let registry = ComponentRegistry::default();
		let archetypes = Archetypes::new(&registry);
		Self {
			entities: EntityAllocator::default(),
			registry,
			archetypes,
			bundle_cache: FastMap::default(),
			name_to_entity: FastMap::default(),
			entity_to_name: FastMap::default(),
			groups: FastMap::default(),
			entity_groups: FastMap::default(),
			change_tick: 1,
		}
	}
}

impl EcsStore {
	/// Bumps the change-detection tick counter (wrapping past 0 so 0 stays a
	/// clean "never run" sentinel).
	#[inline]
	pub fn bump_tick(&mut self) -> u32 {
		self.change_tick = self.change_tick.wrapping_add(1).max(1);
		self.change_tick
	}

	/// Resolves (and caches) the target [`ArchetypeId`] and pre-resolved column
	/// indices for bundle type `B` with zero heap allocation on hit.
	#[inline]
	fn resolve_bundle<B: Bundle>(&mut self) -> CachedBundle {
		let key = std::any::TypeId::of::<B>();
		if let Some(&cached) = self.bundle_cache.get(&key) {
			return cached;
		}
		let mut raw_ids = Vec::new();
		B::register_ids(&mut self.registry, &mut raw_ids);
		let sorted = canonicalize_bundle_ids(&raw_ids);
		let arch_id = self.archetypes.get_or_create(&sorted, &self.registry);
		let arch = self.archetypes.get(arch_id);
		let mut cols = [0_usize; MAX_BUNDLE_COLS];
		for (idx, &comp_id) in raw_ids.iter().enumerate() {
			cols[idx] = arch
				.column_index(comp_id)
				.expect("bundle component missing from target archetype");
		}
		let cached = CachedBundle {
			archetype: arch_id,
			cols,
			len: raw_ids.len(),
		};
		self.bundle_cache.insert(key, cached);
		cached
	}

	/// Spawns one entity with `bundle`.
	#[inline]
	pub fn spawn<B: Bundle>(&mut self, bundle: B) -> Entity {
		let cached = self.resolve_bundle::<B>();
		let tick = self.change_tick;
		let arch = self.archetypes.get_mut(cached.archetype);
		let row = u32::try_from(arch.len()).expect("archetype row count exceeded u32::MAX");
		let entity = self.entities.alloc(EntityLocation {
			archetype: cached.archetype,
			row,
		});
		arch.entities.push(entity);
		bundle.push_into(arch, &cached.cols[..cached.len], tick);
		entity
	}

	/// Spawns an entire iterator of identical bundles in bulk with a single
	/// archetype resolution, single column reservation, and zero per-entity
	/// lookups.
	pub fn spawn_batch<B: Bundle>(&mut self, iter: impl IntoIterator<Item = B>) -> Vec<Entity> {
		let cached = self.resolve_bundle::<B>();
		let tick = self.change_tick;
		let arch = self.archetypes.get_mut(cached.archetype);
		let start_row = arch.len();
		let count = B::push_batch(iter, arch, &cached.cols[..cached.len], tick);
		if count == 0 {
			return Vec::new();
		}
		let mut spawned = Vec::with_capacity(count);
		self.entities.alloc_batch(
			cached.archetype,
			u32::try_from(start_row).expect("archetype row count exceeded u32::MAX"),
			count,
			&mut spawned,
		);
		arch.entities.extend_from_slice(&spawned);
		spawned
	}

	/// Spawns an entire iterator of identical bundles without allocating a
	/// return `Vec<Entity>`, returning the number of spawned entities.
	pub fn spawn_batch_discard<B: Bundle>(&mut self, iter: impl IntoIterator<Item = B>) -> usize {
		let cached = self.resolve_bundle::<B>();
		let tick = self.change_tick;
		let arch = self.archetypes.get_mut(cached.archetype);
		let start_row = arch.len();
		let count = B::push_batch(iter, arch, &cached.cols[..cached.len], tick);
		if count == 0 {
			return 0;
		}
		self.entities.alloc_batch(
			cached.archetype,
			u32::try_from(start_row).expect("archetype row count exceeded u32::MAX"),
			count,
			&mut arch.entities,
		);
		count
	}

	/// Despawns `entity`, removing its components, name, and group memberships.
	pub fn despawn(&mut self, entity: Entity) -> bool {
		let Some(loc) = self.entities.free(entity) else {
			return false;
		};
		if let Some(name) = self.entity_to_name.remove(&entity) {
			self.name_to_entity.remove(&name);
		}
		if let Some(groups) = self.entity_groups.remove(&entity) {
			for group in groups {
				if let Some(members) = self.groups.get_mut(&group) {
					if let Some(pos) = members.iter().position(|&e| e == entity) {
						let _ = members.swap_remove(pos);
					}
					if members.is_empty() {
						self.groups.remove(&group);
					}
				}
			}
		}
		let arch = self.archetypes.get_mut(loc.archetype);
		if let Some(moved) = arch.swap_remove_drop(loc.row as usize) {
			self.entities.set_location(moved, loc);
		}
		true
	}

	/// Assigns or replaces a unique Godot-style `name` for `entity`.
	pub fn set_name(&mut self, entity: Entity, name: impl Into<String>) -> bool {
		if !self.entities.is_alive(entity) {
			return false;
		}
		let name = name.into();
		if let Some(old_name) = self.entity_to_name.insert(entity, name.clone()) {
			self.name_to_entity.remove(&old_name);
		}
		if let Some(previous_owner) = self.name_to_entity.insert(name, entity)
			&& previous_owner != entity
		{
			self.entity_to_name.remove(&previous_owner);
		}
		true
	}

	/// Looks up an entity by its unique name in O(1).
	#[inline]
	pub fn named(&self, name: &str) -> Option<Entity> {
		self.name_to_entity.get(name).copied()
	}

	/// Returns the name assigned to `entity`, if any.
	#[inline]
	pub fn name_of(&self, entity: Entity) -> Option<&str> {
		self.entity_to_name.get(&entity).map(String::as_str)
	}

	/// Adds `entity` to a named Godot-style group (idempotent).
	pub fn add_to_group(&mut self, entity: Entity, group: impl Into<String>) {
		if !self.entities.is_alive(entity) {
			return;
		}
		let group = group.into();
		let entry = self.entity_groups.entry(entity).or_default();
		if entry.iter().any(|g| g == &group) {
			return;
		}
		entry.push(group.clone());
		self.groups.entry(group).or_default().push(entity);
	}

	/// Removes `entity` from `group`.
	pub fn remove_from_group(&mut self, entity: Entity, group: &str) {
		if let Some(list) = self.entity_groups.get_mut(&entity)
			&& let Some(pos) = list.iter().position(|g| g == group)
		{
			let _ = list.swap_remove(pos);
		}
		if let Some(members) = self.groups.get_mut(group) {
			if let Some(pos) = members.iter().position(|&e| e == entity) {
				let _ = members.swap_remove(pos);
			}
			if members.is_empty() {
				self.groups.remove(group);
			}
		}
	}

	/// Entities currently in `group`.
	#[inline]
	pub fn group(&self, group: &str) -> &[Entity] {
		self.groups.get(group).map_or(&[], Vec::as_slice)
	}

	/// Inserts or replaces `component` on `entity`, migrating its archetype
	/// through a cached zero-clone transition edge when the component is new.
	pub fn insert_component<T: Component>(&mut self, entity: Entity, component: T) -> bool {
		let Some(loc) = self.entities.location(entity) else {
			return false;
		};
		let comp_id = self.registry.register::<T>();
		let tick = self.change_tick;

		// Fast path: entity already has component T -> overwrite in place.
		if let Some(col_idx) = self.archetypes.get(loc.archetype).column_index(comp_id) {
			let arch = self.archetypes.get_mut(loc.archetype);
			let col = downcast_col_mut::<T>(arch.columns[col_idx].as_mut());
			let _ = col.set(loc.row as usize, component, tick);
			return true;
		}

		// Transition path: pooled zero-clone edge with precomputed column map.
		let edge_idx = self
			.archetypes
			.resolve_add_edge(loc.archetype, comp_id, &self.registry);
		let (edge, src, dst) = self.archetypes.add_transition_mut(loc.archetype, edge_idx);
		let target = edge.target;
		let new_row = u32::try_from(dst.len()).expect("archetype row count exceeded u32::MAX");
		let _ = src.entities.swap_remove(loc.row as usize);
		let swapped = src.entities.get(loc.row as usize).copied();
		dst.entities.push(entity);

		for &(src_col, dst_col) in &edge.column_map {
			src.columns[src_col].move_row_to(loc.row as usize, dst.columns[dst_col].as_mut());
		}
		downcast_col_mut::<T>(dst.columns[edge.added_col].as_mut()).push(component, tick);

		if let Some(moved) = swapped {
			self.entities.set_location(moved, loc);
		}
		self.entities.set_location(
			entity,
			EntityLocation {
				archetype: target,
				row: new_row,
			},
		);
		true
	}

	/// Removes component `T` from `entity`, returning it if present.
	pub fn remove_component<T: Component>(&mut self, entity: Entity) -> Option<T> {
		let loc = self.entities.location(entity)?;
		let comp_id = self.registry.id_of::<T>()?;
		let edge_idx =
			self.archetypes
				.resolve_remove_edge(loc.archetype, comp_id, &self.registry)?;
		let (edge, src, dst) = self
			.archetypes
			.remove_transition_mut(loc.archetype, edge_idx);
		let target = edge.target;
		let new_row = u32::try_from(dst.len()).expect("archetype row count exceeded u32::MAX");
		let _ = src.entities.swap_remove(loc.row as usize);
		let swapped = src.entities.get(loc.row as usize).copied();
		dst.entities.push(entity);

		for &(src_col, dst_col) in &edge.column_map {
			src.columns[src_col].move_row_to(loc.row as usize, dst.columns[dst_col].as_mut());
		}
		let (removed, _) = downcast_col_mut::<T>(src.columns[edge.removed_col].as_mut())
			.swap_remove(loc.row as usize);

		if let Some(moved) = swapped {
			self.entities.set_location(moved, loc);
		}
		self.entities.set_location(
			entity,
			EntityLocation {
				archetype: target,
				row: new_row,
			},
		);
		Some(removed)
	}

	/// Borrows component `T` on `entity` in O(1).
	#[inline]
	pub fn get_component<T: Component>(&self, entity: Entity) -> Option<&T> {
		let loc = self.entities.location(entity)?;
		let comp_id = self.registry.id_of::<T>()?;
		let arch = self.archetypes.get(loc.archetype);
		let col = arch.column_index(comp_id)?;
		Some(downcast_col::<T>(arch.columns[col].as_ref()).get(loc.row as usize))
	}

	/// Borrows component `T` on `entity` mutably in O(1), bumping its change
	/// tick.
	#[inline]
	pub fn get_component_mut<T: Component>(&mut self, entity: Entity) -> Option<&mut T> {
		let loc = self.entities.location(entity)?;
		let comp_id = self.registry.id_of::<T>()?;
		let tick = self.change_tick;
		let arch = self.archetypes.get_mut(loc.archetype);
		let col = arch.column_index(comp_id)?;
		Some(downcast_col_mut::<T>(arch.columns[col].as_mut()).get_mut(loc.row as usize, tick))
	}

	/// Whether `entity` is alive and holds component `T`.
	#[inline]
	pub fn has_component<T: Component>(&self, entity: Entity) -> bool {
		let Some(loc) = self.entities.location(entity) else {
			return false;
		};
		let Some(comp_id) = self.registry.id_of::<T>() else {
			return false;
		};
		self.archetypes
			.get(loc.archetype)
			.column_index(comp_id)
			.is_some()
	}

	/// Visits every entity matching read-only query `Q` and filter `F`.
	pub fn for_each_read<Q: ReadQuery, F: QueryFilter>(
		&self,
		since_tick: u32,
		mut f: impl FnMut(Q::Item<'_>),
	) {
		let mut masks = FilterMasks::default();
		let state = Q::init(&self.registry, &mut masks);
		let filter_state = F::init(&self.registry, &mut masks);
		if masks.missing_required_type {
			return;
		}
		for arch in self.archetypes.as_slice() {
			if !arch.is_empty() && masks.matches(arch) {
				Q::visit_archetype::<F>(state, filter_state, arch, since_tick, &mut f);
			}
		}
	}

	/// Visits every entity matching mutable query `Q` and filter `F`.
	pub fn for_each_write<Q: WriteQuery, F: QueryFilter>(
		&mut self,
		since_tick: u32,
		mut f: impl FnMut(Q::Item<'_>),
	) {
		let mut masks = FilterMasks::default();
		let state = Q::init(&self.registry, &mut masks);
		let filter_state = F::init(&self.registry, &mut masks);
		if masks.missing_required_type {
			return;
		}
		let change_tick = self.change_tick;
		for arch in self.archetypes.as_mut_slice() {
			if !arch.is_empty() && masks.matches(arch) {
				Q::visit_archetype_mut::<F>(
					state,
					filter_state,
					arch,
					since_tick,
					change_tick,
					&mut f,
				);
			}
		}
	}

	/// Executes a cached [`QueryPlan`] over read-only query `Q` and filter `F`,
	/// avoiding repeated archetype mask scans across frames.
	pub fn for_each_read_cached<Q: ReadQuery, F: QueryFilter>(
		&self,
		plan: &mut QueryPlan,
		since_tick: u32,
		mut f: impl FnMut(Q::Item<'_>),
	) {
		let mut masks = FilterMasks::default();
		let state = Q::init(&self.registry, &mut masks);
		let filter_state = F::init(&self.registry, &mut masks);
		plan.refresh(&self.archetypes, || masks);
		for &arch_id in plan.archetypes() {
			let arch = self.archetypes.get(arch_id);
			if !arch.is_empty() {
				Q::visit_archetype::<F>(state, filter_state, arch, since_tick, &mut f);
			}
		}
	}

	/// Executes a cached [`QueryPlan`] over mutable query `Q` and filter `F`.
	pub fn for_each_write_cached<Q: WriteQuery, F: QueryFilter>(
		&mut self,
		plan: &mut QueryPlan,
		since_tick: u32,
		mut f: impl FnMut(Q::Item<'_>),
	) {
		let mut masks = FilterMasks::default();
		let state = Q::init(&self.registry, &mut masks);
		let filter_state = F::init(&self.registry, &mut masks);
		plan.refresh(&self.archetypes, || masks);
		let change_tick = self.change_tick;
		for &arch_id in plan.archetypes() {
			let arch = self.archetypes.get_mut(arch_id);
			if !arch.is_empty() {
				Q::visit_archetype_mut::<F>(
					state,
					filter_state,
					arch,
					since_tick,
					change_tick,
					&mut f,
				);
			}
		}
	}

	/// Peak-speed contiguous slice iteration for `(&mut A, &B)`: hands raw
	/// `&mut [A]` and `&[B]` slices per matching archetype directly to `f` so
	/// LLVM emits pure AVX2/SSE/NEON SIMD vector loops with zero per-entity
	/// overhead.
	pub fn for_each_chunk_2<A: Component, B: Component>(
		&mut self,
		mut f: impl FnMut(&mut [A], &[B]),
	) {
		let (Some(id_a), Some(id_b)) = (self.registry.id_of::<A>(), self.registry.id_of::<B>())
		else {
			return;
		};
		let tick = self.change_tick;
		for arch in self.archetypes.as_mut_slice() {
			if arch.is_empty() {
				continue;
			}
			let (Some(ca), Some(cb)) = (arch.column_index(id_a), arch.column_index(id_b)) else {
				continue;
			};
			let [col_a, col_b] = arch
				.columns
				.get_disjoint_mut([ca, cb])
				.expect("chunk query cannot alias identical component types");
			let sa = downcast_col_mut::<A>(col_a.as_mut()).as_mut_slice_unchecked_ticks(tick);
			let sb = downcast_col::<B>(col_b.as_ref()).as_slice();
			f(sa, sb);
		}
	}

	/// Parallel multi-core chunk iteration for `(&mut A, &B)` using `rayon` on
	/// native targets (and deterministic sequential iteration on `wasm32`).
	pub fn par_for_each_chunk_2<A: Component, B: Component>(
		&mut self,
		f: impl Fn(&mut [A], &[B]) + Send + Sync,
	) {
		#[cfg(not(target_arch = "wasm32"))]
		{
			use rayon::prelude::*;
			const PAR_CHUNK_SIZE: usize = 2048;
			let (Some(id_a), Some(id_b)) = (self.registry.id_of::<A>(), self.registry.id_of::<B>())
			else {
				return;
			};
			let tick = self.change_tick;
			for arch in self.archetypes.as_mut_slice() {
				if arch.is_empty() {
					continue;
				}
				let (Some(ca), Some(cb)) = (arch.column_index(id_a), arch.column_index(id_b))
				else {
					continue;
				};
				let [col_a, col_b] = arch
					.columns
					.get_disjoint_mut([ca, cb])
					.expect("chunk query cannot alias identical component types");
				let sa = downcast_col_mut::<A>(col_a.as_mut()).as_mut_slice_unchecked_ticks(tick);
				let sb = downcast_col::<B>(col_b.as_ref()).as_slice();
				if sa.len() < PAR_CHUNK_SIZE * 2 {
					f(sa, sb);
				} else {
					sa.par_chunks_mut(PAR_CHUNK_SIZE)
						.zip(sb.par_chunks(PAR_CHUNK_SIZE))
						.for_each(|(ca_slice, cb_slice)| f(ca_slice, cb_slice));
				}
			}
		}
		#[cfg(target_arch = "wasm32")]
		{
			self.for_each_chunk_2::<A, B>(f);
		}
	}

	/// Number of distinct archetypes currently instantiated in this world.
	#[inline]
	pub fn archetype_count(&self) -> usize {
		self.archetypes.len()
	}

	/// Number of registered component types in this world.
	#[inline]
	pub fn component_type_count(&self) -> usize {
		self.registry.len()
	}
}
