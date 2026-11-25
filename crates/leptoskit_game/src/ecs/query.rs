//! High-performance typed queries, filters, change detection, and cached
//! query plans.
//!
//! Queries operate directly on contiguous `SoA` columns:
//! - `&T` and `&mut T` expose raw slices per archetype (stamping column change
//!   ticks once per table so LLVM auto-vectorizes inner loops with AVX2/NEON).
//! - [`Mut<T>`] tracks fine-grained per-entity mutations lazily on `DerefMut`.
//! - [`Changed<T>`] and [`Added<T>`] skip entire untouched archetypes in O(1)
//!   before scanning individual rows.
//! - [`QueryPlan`] caches matched archetype IDs across frames so fragmented
//!   worlds pay zero per-frame archetype discovery overhead.

use std::marker::PhantomData;
use std::ops::Deref;
use std::ops::DerefMut;

use crate::ecs::archetype::Archetype;
use crate::ecs::archetype::ArchetypeId;
use crate::ecs::archetype::Archetypes;
use crate::ecs::column::downcast_col;
use crate::ecs::column::downcast_col_mut;
use crate::ecs::component::Component;
use crate::ecs::component::ComponentId;
use crate::ecs::component::ComponentMask;
use crate::ecs::component::ComponentRegistry;
use crate::ecs::component::ComponentTicks;
use crate::ecs::entity::Entity;

/// A smart mutable reference that marks the component changed only when
/// [`DerefMut::deref_mut`] is called.
pub struct Mut<'a, T> {
	value: &'a mut T,
	ticks: &'a mut ComponentTicks,
	change_tick: u32,
}

impl<'a, T> Mut<'a, T> {
	#[inline]
	pub(crate) fn new(value: &'a mut T, ticks: &'a mut ComponentTicks, change_tick: u32) -> Self {
		Self {
			value,
			ticks,
			change_tick,
		}
	}

	/// Replaces the value only if it differs from `new_value`, avoiding a
	/// spurious change-detection tick when unchanged.
	#[inline]
	pub fn set_if_neq(&mut self, new_value: T) -> bool
	where
		T: PartialEq,
	{
		if *self.value == new_value {
			false
		} else {
			*self.value = new_value;
			self.ticks.changed = self.change_tick;
			true
		}
	}

	/// Returns whether this component was added since `since_tick`.
	#[inline]
	pub const fn is_added(&self, since_tick: u32) -> bool {
		self.ticks.is_added(since_tick)
	}

	/// Returns whether this component was changed since `since_tick`.
	#[inline]
	pub const fn is_changed(&self, since_tick: u32) -> bool {
		self.ticks.is_changed(since_tick)
	}

	/// Borrows the value mutably without bumping the change tick.
	#[inline]
	pub fn bypass_change_detection(&mut self) -> &mut T {
		self.value
	}
}

impl<T> Deref for Mut<'_, T> {
	type Target = T;

	#[inline]
	fn deref(&self) -> &Self::Target {
		self.value
	}
}

impl<T> DerefMut for Mut<'_, T> {
	#[inline]
	fn deref_mut(&mut self) -> &mut Self::Target {
		self.ticks.changed = self.change_tick;
		self.value
	}
}

/// Filter requiring an entity to have component `T` without fetching it.
pub struct With<T: Component>(PhantomData<fn() -> T>);

/// Filter requiring an entity to NOT have component `T`.
pub struct Without<T: Component>(PhantomData<fn() -> T>);

/// Filter matching entities whose component `T` changed after `since_tick`.
pub struct Changed<T: Component>(PhantomData<fn() -> T>);

/// Filter matching entities whose component `T` was added after `since_tick`.
pub struct Added<T: Component>(PhantomData<fn() -> T>);

/// Compiled archetype matching masks for a query + filter combination.
#[doc(hidden)]
#[derive(Debug, Default, Clone)]
pub struct FilterMasks {
	pub(crate) required: ComponentMask,
	pub(crate) excluded: ComponentMask,
	pub(crate) missing_required_type: bool,
}

impl FilterMasks {
	#[inline]
	pub fn matches(&self, archetype: &Archetype) -> bool {
		!self.missing_required_type
			&& archetype.mask.contains_all(&self.required)
			&& archetype.mask.is_disjoint(&self.excluded)
	}
}

/// Prepared per-archetype row-level filter state.
pub trait QueryFilter {
	/// Resolved component IDs for this filter.
	type State: Copy;
	/// Per-archetype slice borrow used to test individual rows.
	type Fetch<'w>;
	/// `true` when filtering happens purely at the archetype mask level
	/// (e.g. `()`, `With<T>`, `Without<T>`), allowing queries to take the
	/// zero-allocation contiguous SIMD fast path.
	const ARCHETYPE_ONLY: bool;

	/// Resolves component IDs and populates `masks`.
	fn init(registry: &ComponentRegistry, masks: &mut FilterMasks) -> Self::State;

	/// Returns `false` to skip the entire archetype in O(1) (for example, when
	/// a `Changed<T>` column has `max_changed_tick <= since_tick`).
	fn matches_archetype(state: Self::State, archetype: &Archetype, since_tick: u32) -> bool;

	/// Borrows the per-row tick slices needed for `archetype`.
	fn prepare(state: Self::State, archetype: &Archetype) -> Self::Fetch<'_>;

	/// Returns whether `row` passes the filter.
	fn matches_row(fetch: &Self::Fetch<'_>, row: usize, since_tick: u32) -> bool;
}

impl QueryFilter for () {
	type Fetch<'w> = ();
	type State = ();

	const ARCHETYPE_ONLY: bool = true;

	#[inline]
	fn init(_registry: &ComponentRegistry, _masks: &mut FilterMasks) {}

	#[inline]
	fn matches_archetype(_state: (), _archetype: &Archetype, _since_tick: u32) -> bool {
		true
	}

	#[inline]
	fn prepare(_state: (), _archetype: &Archetype) {}

	#[inline]
	fn matches_row(_fetch: &(), _row: usize, _since_tick: u32) -> bool {
		true
	}
}

impl<T: Component> QueryFilter for With<T> {
	type Fetch<'w> = ();
	type State = ();

	const ARCHETYPE_ONLY: bool = true;

	#[inline]
	fn init(registry: &ComponentRegistry, masks: &mut FilterMasks) {
		if let Some(id) = registry.id_of::<T>() {
			masks.required.set(id);
		} else {
			masks.missing_required_type = true;
		}
	}

	#[inline]
	fn matches_archetype(_state: (), _archetype: &Archetype, _since_tick: u32) -> bool {
		true
	}

	#[inline]
	fn prepare(_state: (), _archetype: &Archetype) {}

	#[inline]
	fn matches_row(_fetch: &(), _row: usize, _since_tick: u32) -> bool {
		true
	}
}

impl<T: Component> QueryFilter for Without<T> {
	type Fetch<'w> = ();
	type State = ();

	const ARCHETYPE_ONLY: bool = true;

	#[inline]
	fn init(registry: &ComponentRegistry, masks: &mut FilterMasks) {
		if let Some(id) = registry.id_of::<T>() {
			masks.excluded.set(id);
		}
	}

	#[inline]
	fn matches_archetype(_state: (), _archetype: &Archetype, _since_tick: u32) -> bool {
		true
	}

	#[inline]
	fn prepare(_state: (), _archetype: &Archetype) {}

	#[inline]
	fn matches_row(_fetch: &(), _row: usize, _since_tick: u32) -> bool {
		true
	}
}

impl<T: Component> QueryFilter for Changed<T> {
	type Fetch<'w> = crate::ecs::column::ColumnTicksRef<'w>;
	type State = Option<ComponentId>;

	const ARCHETYPE_ONLY: bool = false;

	#[inline]
	fn init(registry: &ComponentRegistry, masks: &mut FilterMasks) -> Self::State {
		let id = registry.id_of::<T>();
		if let Some(id) = id {
			masks.required.set(id);
		} else {
			masks.missing_required_type = true;
		}
		id
	}

	#[inline]
	fn matches_archetype(state: Self::State, archetype: &Archetype, since_tick: u32) -> bool {
		let Some(id) = state else { return false };
		let Some(col) = archetype.column_index(id) else {
			return false;
		};
		let max = archetype.columns[col].max_changed_tick();
		max.wrapping_sub(since_tick) != 0 && max.wrapping_sub(since_tick) < (u32::MAX / 2)
	}

	#[inline]
	fn prepare(state: Self::State, archetype: &Archetype) -> Self::Fetch<'_> {
		let id = state.expect("Changed filter matched missing component");
		let col = archetype.column_index(id).expect("missing column");
		downcast_col::<T>(archetype.columns[col].as_ref()).ticks_ref()
	}

	#[inline]
	fn matches_row(fetch: &Self::Fetch<'_>, row: usize, since_tick: u32) -> bool {
		fetch.get(row).is_changed(since_tick)
	}
}

impl<T: Component> QueryFilter for Added<T> {
	type Fetch<'w> = crate::ecs::column::ColumnTicksRef<'w>;
	type State = Option<ComponentId>;

	const ARCHETYPE_ONLY: bool = false;

	#[inline]
	fn init(registry: &ComponentRegistry, masks: &mut FilterMasks) -> Self::State {
		let id = registry.id_of::<T>();
		if let Some(id) = id {
			masks.required.set(id);
		} else {
			masks.missing_required_type = true;
		}
		id
	}

	#[inline]
	fn matches_archetype(state: Self::State, archetype: &Archetype, since_tick: u32) -> bool {
		let Some(id) = state else { return false };
		let Some(col) = archetype.column_index(id) else {
			return false;
		};
		let max = archetype.columns[col].max_added_tick();
		max.wrapping_sub(since_tick) != 0 && max.wrapping_sub(since_tick) < (u32::MAX / 2)
	}

	#[inline]
	fn prepare(state: Self::State, archetype: &Archetype) -> Self::Fetch<'_> {
		let id = state.expect("Added filter matched missing component");
		let col = archetype.column_index(id).expect("missing column");
		downcast_col::<T>(archetype.columns[col].as_ref()).ticks_ref()
	}

	#[inline]
	fn matches_row(fetch: &Self::Fetch<'_>, row: usize, since_tick: u32) -> bool {
		fetch.get(row).is_added(since_tick)
	}
}

macro_rules! impl_filter_tuple {
	($(($idx:tt, $name:ident)),+ $(,)?) => {
		impl<$($name: QueryFilter),+> QueryFilter for ($($name,)+) {
			type State = ($($name::State,)+);
			type Fetch<'w> = ($($name::Fetch<'w>,)+);
			const ARCHETYPE_ONLY: bool = $($name::ARCHETYPE_ONLY)&&+;

			#[inline]
			fn init(registry: &ComponentRegistry, masks: &mut FilterMasks) -> Self::State {
				($($name::init(registry, masks),)+)
			}

			#[inline]
			fn matches_archetype(
				state: Self::State,
				archetype: &Archetype,
				since_tick: u32,
			) -> bool {
				$($name::matches_archetype(state.$idx, archetype, since_tick))&&+
			}

			#[inline]
			fn prepare(state: Self::State, archetype: &Archetype) -> Self::Fetch<'_> {
				($($name::prepare(state.$idx, archetype),)+)
			}

			#[inline]
			fn matches_row(fetch: &Self::Fetch<'_>, row: usize, since_tick: u32) -> bool {
				$($name::matches_row(&fetch.$idx, row, since_tick))&&+
			}
		}
	};
}

impl_filter_tuple!((0, F0), (1, F1));
impl_filter_tuple!((0, F0), (1, F1), (2, F2));
impl_filter_tuple!((0, F0), (1, F1), (2, F2), (3, F3));

/// Cached list of matching archetypes for a query, updated incrementally only
/// when new archetypes are created in the world.
#[derive(Debug, Default, Clone)]
pub struct QueryPlan {
	masks: FilterMasks,
	matched: Vec<ArchetypeId>,
	seen_archetypes: usize,
	initialized: bool,
}

impl QueryPlan {
	/// Creates an empty, uninitialized query plan.
	pub fn new() -> Self {
		Self::default()
	}

	/// Incrementally refreshes matched archetypes from `archetypes`.
	pub(crate) fn refresh(
		&mut self,
		archetypes: &Archetypes,
		init_masks: impl FnOnce() -> FilterMasks,
	) {
		if !self.initialized || self.masks.missing_required_type {
			self.masks = init_masks();
			self.matched.clear();
			self.seen_archetypes = 0;
			self.initialized = true;
		}
		if self.masks.missing_required_type {
			return;
		}
		let slice = archetypes.as_slice();
		for archetype in &slice[self.seen_archetypes..] {
			if self.masks.matches(archetype) {
				self.matched.push(archetype.id);
			}
		}
		self.seen_archetypes = slice.len();
	}

	/// Matched archetype IDs.
	#[inline]
	pub(crate) fn archetypes(&self) -> &[ArchetypeId] {
		&self.matched
	}
}

/// Read-only query descriptor for `World::for_each_read` and `World::query`.
pub trait ReadQuery {
	/// Borrowed item yielded for each entity.
	type Item<'w>;
	/// Resolved component IDs.
	type State: Copy;

	/// Registers required components into `masks` and returns resolved IDs.
	fn init(registry: &ComponentRegistry, masks: &mut FilterMasks) -> Self::State;

	/// Visits every matching row in `archetype` passing `filter`.
	fn visit_archetype<F: QueryFilter>(
		state: Self::State,
		filter_state: F::State,
		archetype: &Archetype,
		since_tick: u32,
		f: &mut impl FnMut(Self::Item<'_>),
	);
}

/// Mutable query descriptor for `World::for_each` and
/// `World::for_each_filtered`.
pub trait WriteQuery {
	/// Borrowed item yielded for each entity.
	type Item<'w>;
	/// Resolved component IDs.
	type State: Copy;

	/// Registers required components into `masks` and returns resolved IDs.
	fn init(registry: &ComponentRegistry, masks: &mut FilterMasks) -> Self::State;

	/// Visits every matching row in `archetype` passing `filter`.
	fn visit_archetype_mut<F: QueryFilter>(
		state: Self::State,
		filter_state: F::State,
		archetype: &mut Archetype,
		since_tick: u32,
		change_tick: u32,
		f: &mut impl FnMut(Self::Item<'_>),
	);
}

/// Sealed helper classifying each component slot in a mutable query tuple.
pub trait QueryTerm {
	/// Borrowed item for one row.
	type Item<'w>;
	/// Resolved component ID (or `None` for `Entity`).
	type State: Copy;
	/// Whether this term mutates a column.
	const MUTABLE: bool;

	/// Registers required component into `masks`.
	fn init(registry: &ComponentRegistry, masks: &mut FilterMasks) -> Self::State;

	/// Column index in `archetype`, or `None` if this term does not borrow a
	/// column (e.g. `Entity` or an absent `Option`).
	fn col_index(state: Self::State, archetype: &Archetype) -> Option<usize>;
}

impl QueryTerm for Entity {
	type Item<'w> = Entity;
	type State = ();

	const MUTABLE: bool = false;

	#[inline]
	fn init(_registry: &ComponentRegistry, _masks: &mut FilterMasks) {}

	#[inline]
	fn col_index(_state: (), _archetype: &Archetype) -> Option<usize> {
		None
	}
}

impl<T: Component> QueryTerm for &T {
	type Item<'w> = &'w T;
	type State = Option<ComponentId>;

	const MUTABLE: bool = false;

	#[inline]
	fn init(registry: &ComponentRegistry, masks: &mut FilterMasks) -> Self::State {
		let id = registry.id_of::<T>();
		if let Some(id) = id {
			masks.required.set(id);
		} else {
			masks.missing_required_type = true;
		}
		id
	}

	#[inline]
	fn col_index(state: Self::State, archetype: &Archetype) -> Option<usize> {
		archetype.column_index(state?)
	}
}

impl<T: Component> QueryTerm for &mut T {
	type Item<'w> = &'w mut T;
	type State = Option<ComponentId>;

	const MUTABLE: bool = true;

	#[inline]
	fn init(registry: &ComponentRegistry, masks: &mut FilterMasks) -> Self::State {
		let id = registry.id_of::<T>();
		if let Some(id) = id {
			masks.required.set(id);
		} else {
			masks.missing_required_type = true;
		}
		id
	}

	#[inline]
	fn col_index(state: Self::State, archetype: &Archetype) -> Option<usize> {
		archetype.column_index(state?)
	}
}

impl<T: Component> QueryTerm for Mut<'_, T> {
	type Item<'w> = Mut<'w, T>;
	type State = Option<ComponentId>;

	const MUTABLE: bool = true;

	#[inline]
	fn init(registry: &ComponentRegistry, masks: &mut FilterMasks) -> Self::State {
		let id = registry.id_of::<T>();
		if let Some(id) = id {
			masks.required.set(id);
		} else {
			masks.missing_required_type = true;
		}
		id
	}

	#[inline]
	fn col_index(state: Self::State, archetype: &Archetype) -> Option<usize> {
		archetype.column_index(state?)
	}
}

impl<T: Component> QueryTerm for Option<&T> {
	type Item<'w> = Option<&'w T>;
	type State = Option<ComponentId>;

	const MUTABLE: bool = false;

	#[inline]
	fn init(registry: &ComponentRegistry, _masks: &mut FilterMasks) -> Self::State {
		registry.id_of::<T>()
	}

	#[inline]
	fn col_index(state: Self::State, archetype: &Archetype) -> Option<usize> {
		archetype.column_index(state?)
	}
}

impl<T: Component> ReadQuery for &T {
	type Item<'w> = &'w T;
	type State = Option<ComponentId>;

	#[inline]
	fn init(registry: &ComponentRegistry, masks: &mut FilterMasks) -> Self::State {
		<&T as QueryTerm>::init(registry, masks)
	}

	#[inline]
	fn visit_archetype<F: QueryFilter>(
		state: Self::State,
		filter_state: F::State,
		archetype: &Archetype,
		since_tick: u32,
		f: &mut impl FnMut(Self::Item<'_>),
	) {
		if archetype.is_empty() || !F::matches_archetype(filter_state, archetype, since_tick) {
			return;
		}
		let col = archetype
			.column_index(state.expect("matched missing component"))
			.expect("missing column");
		let slice = archetype.column_slice::<T>(col);
		let filter = F::prepare(filter_state, archetype);
		for (row, item) in slice.iter().enumerate() {
			if F::matches_row(&filter, row, since_tick) {
				f(item);
			}
		}
	}
}

impl ReadQuery for Entity {
	type Item<'w> = Entity;
	type State = ();

	#[inline]
	fn init(_registry: &ComponentRegistry, _masks: &mut FilterMasks) {}

	#[inline]
	fn visit_archetype<F: QueryFilter>(
		_state: (),
		filter_state: F::State,
		archetype: &Archetype,
		since_tick: u32,
		f: &mut impl FnMut(Entity),
	) {
		if archetype.is_empty() || !F::matches_archetype(filter_state, archetype, since_tick) {
			return;
		}
		let filter = F::prepare(filter_state, archetype);
		for (row, &entity) in archetype.entities.iter().enumerate() {
			if F::matches_row(&filter, row, since_tick) {
				f(entity);
			}
		}
	}
}

impl<A: Component> ReadQuery for (Entity, &A) {
	type Item<'w> = (Entity, &'w A);
	type State = Option<ComponentId>;

	#[inline]
	fn init(registry: &ComponentRegistry, masks: &mut FilterMasks) -> Self::State {
		<&A as QueryTerm>::init(registry, masks)
	}

	#[inline]
	fn visit_archetype<F: QueryFilter>(
		state: Self::State,
		filter_state: F::State,
		archetype: &Archetype,
		since_tick: u32,
		f: &mut impl FnMut(Self::Item<'_>),
	) {
		if archetype.is_empty() || !F::matches_archetype(filter_state, archetype, since_tick) {
			return;
		}
		let col = archetype
			.column_index(state.expect("missing component"))
			.expect("missing column");
		let a = archetype.column_slice::<A>(col);
		let filter = F::prepare(filter_state, archetype);
		for (row, (&e, va)) in archetype.entities.iter().zip(a).enumerate() {
			if F::matches_row(&filter, row, since_tick) {
				f((e, va));
			}
		}
	}
}

impl<A: Component, B: Component> ReadQuery for (&A, &B) {
	type Item<'w> = (&'w A, &'w B);
	type State = (Option<ComponentId>, Option<ComponentId>);

	#[inline]
	fn init(registry: &ComponentRegistry, masks: &mut FilterMasks) -> Self::State {
		(
			<&A as QueryTerm>::init(registry, masks),
			<&B as QueryTerm>::init(registry, masks),
		)
	}

	#[inline]
	fn visit_archetype<F: QueryFilter>(
		state: Self::State,
		filter_state: F::State,
		archetype: &Archetype,
		since_tick: u32,
		f: &mut impl FnMut(Self::Item<'_>),
	) {
		if archetype.is_empty() || !F::matches_archetype(filter_state, archetype, since_tick) {
			return;
		}
		let ca = archetype.column_index(state.0.unwrap()).unwrap();
		let cb = archetype.column_index(state.1.unwrap()).unwrap();
		let sa = archetype.column_slice::<A>(ca);
		let sb = archetype.column_slice::<B>(cb);
		let filter = F::prepare(filter_state, archetype);
		for (row, (va, vb)) in sa.iter().zip(sb).enumerate() {
			if F::matches_row(&filter, row, since_tick) {
				f((va, vb));
			}
		}
	}
}

impl<A: Component, B: Component> ReadQuery for (Entity, &A, &B) {
	type Item<'w> = (Entity, &'w A, &'w B);
	type State = (Option<ComponentId>, Option<ComponentId>);

	#[inline]
	fn init(registry: &ComponentRegistry, masks: &mut FilterMasks) -> Self::State {
		(
			<&A as QueryTerm>::init(registry, masks),
			<&B as QueryTerm>::init(registry, masks),
		)
	}

	#[inline]
	fn visit_archetype<F: QueryFilter>(
		state: Self::State,
		filter_state: F::State,
		archetype: &Archetype,
		since_tick: u32,
		f: &mut impl FnMut(Self::Item<'_>),
	) {
		if archetype.is_empty() || !F::matches_archetype(filter_state, archetype, since_tick) {
			return;
		}
		let ca = archetype.column_index(state.0.unwrap()).unwrap();
		let cb = archetype.column_index(state.1.unwrap()).unwrap();
		let sa = archetype.column_slice::<A>(ca);
		let sb = archetype.column_slice::<B>(cb);
		let filter = F::prepare(filter_state, archetype);
		for (row, ((&e, va), vb)) in archetype.entities.iter().zip(sa).zip(sb).enumerate() {
			if F::matches_row(&filter, row, since_tick) {
				f((e, va, vb));
			}
		}
	}
}

impl<A: Component, B: Component, C: Component> ReadQuery for (&A, &B, &C) {
	type Item<'w> = (&'w A, &'w B, &'w C);
	type State = (
		Option<ComponentId>,
		Option<ComponentId>,
		Option<ComponentId>,
	);

	#[inline]
	fn init(registry: &ComponentRegistry, masks: &mut FilterMasks) -> Self::State {
		(
			<&A as QueryTerm>::init(registry, masks),
			<&B as QueryTerm>::init(registry, masks),
			<&C as QueryTerm>::init(registry, masks),
		)
	}

	#[inline]
	fn visit_archetype<F: QueryFilter>(
		state: Self::State,
		filter_state: F::State,
		archetype: &Archetype,
		since_tick: u32,
		f: &mut impl FnMut(Self::Item<'_>),
	) {
		if archetype.is_empty() || !F::matches_archetype(filter_state, archetype, since_tick) {
			return;
		}
		let sa = archetype.column_slice::<A>(archetype.column_index(state.0.unwrap()).unwrap());
		let sb = archetype.column_slice::<B>(archetype.column_index(state.1.unwrap()).unwrap());
		let sc = archetype.column_slice::<C>(archetype.column_index(state.2.unwrap()).unwrap());
		let filter = F::prepare(filter_state, archetype);
		for (row, ((va, vb), vc)) in sa.iter().zip(sb).zip(sc).enumerate() {
			if F::matches_row(&filter, row, since_tick) {
				f((va, vb, vc));
			}
		}
	}
}

impl<A: Component, B: Component, C: Component> ReadQuery for (Entity, &A, &B, &C) {
	type Item<'w> = (Entity, &'w A, &'w B, &'w C);
	type State = (
		Option<ComponentId>,
		Option<ComponentId>,
		Option<ComponentId>,
	);

	#[inline]
	fn init(registry: &ComponentRegistry, masks: &mut FilterMasks) -> Self::State {
		(
			<&A as QueryTerm>::init(registry, masks),
			<&B as QueryTerm>::init(registry, masks),
			<&C as QueryTerm>::init(registry, masks),
		)
	}

	#[inline]
	fn visit_archetype<F: QueryFilter>(
		state: Self::State,
		filter_state: F::State,
		archetype: &Archetype,
		since_tick: u32,
		f: &mut impl FnMut(Self::Item<'_>),
	) {
		if archetype.is_empty() || !F::matches_archetype(filter_state, archetype, since_tick) {
			return;
		}
		let sa = archetype.column_slice::<A>(archetype.column_index(state.0.unwrap()).unwrap());
		let sb = archetype.column_slice::<B>(archetype.column_index(state.1.unwrap()).unwrap());
		let sc = archetype.column_slice::<C>(archetype.column_index(state.2.unwrap()).unwrap());
		let filter = F::prepare(filter_state, archetype);
		for (row, (((&e, va), vb), vc)) in archetype
			.entities
			.iter()
			.zip(sa)
			.zip(sb)
			.zip(sc)
			.enumerate()
		{
			if F::matches_row(&filter, row, since_tick) {
				f((e, va, vb, vc));
			}
		}
	}
}

impl<A: Component> WriteQuery for &mut A {
	type Item<'w> = &'w mut A;
	type State = Option<ComponentId>;

	#[inline]
	fn init(registry: &ComponentRegistry, masks: &mut FilterMasks) -> Self::State {
		<&mut A as QueryTerm>::init(registry, masks)
	}

	#[inline]
	fn visit_archetype_mut<F: QueryFilter>(
		state: Self::State,
		filter_state: F::State,
		archetype: &mut Archetype,
		since_tick: u32,
		change_tick: u32,
		f: &mut impl FnMut(Self::Item<'_>),
	) {
		if archetype.is_empty() || !F::matches_archetype(filter_state, archetype, since_tick) {
			return;
		}
		let col = archetype.column_index(state.unwrap()).unwrap();
		if F::ARCHETYPE_ONLY {
			let slice = archetype.column_slice_mut::<A>(col, change_tick);
			for item in slice {
				f(item);
			}
			return;
		}
		let matched: Vec<usize> = {
			let filter = F::prepare(filter_state, archetype);
			(0..archetype.len())
				.filter(|&row| F::matches_row(&filter, row, since_tick))
				.collect()
		};
		if matched.is_empty() {
			return;
		}
		let (slice, ticks) = downcast_col_mut::<A>(archetype.columns[col].as_mut())
			.as_mut_slice_with_ticks(change_tick);
		for row in matched {
			ticks[row].changed = change_tick;
			f(&mut slice[row]);
		}
	}
}

impl<A: Component> WriteQuery for Mut<'_, A> {
	type Item<'w> = Mut<'w, A>;
	type State = Option<ComponentId>;

	#[inline]
	fn init(registry: &ComponentRegistry, masks: &mut FilterMasks) -> Self::State {
		<Mut<'_, A> as QueryTerm>::init(registry, masks)
	}

	#[inline]
	fn visit_archetype_mut<F: QueryFilter>(
		state: Self::State,
		filter_state: F::State,
		archetype: &mut Archetype,
		since_tick: u32,
		change_tick: u32,
		f: &mut impl FnMut(Self::Item<'_>),
	) {
		if archetype.is_empty() || !F::matches_archetype(filter_state, archetype, since_tick) {
			return;
		}
		let col = archetype.column_index(state.unwrap()).unwrap();
		if F::ARCHETYPE_ONLY {
			let (slice, ticks) = downcast_col_mut::<A>(archetype.columns[col].as_mut())
				.as_mut_slice_with_ticks(change_tick);
			for (val, t) in slice.iter_mut().zip(ticks.iter_mut()) {
				f(Mut::new(val, t, change_tick));
			}
			return;
		}
		let matched: Vec<usize> = {
			let filter = F::prepare(filter_state, archetype);
			(0..archetype.len())
				.filter(|&row| F::matches_row(&filter, row, since_tick))
				.collect()
		};
		if matched.is_empty() {
			return;
		}
		let (slice, ticks) = downcast_col_mut::<A>(archetype.columns[col].as_mut())
			.as_mut_slice_with_ticks(change_tick);
		for row in matched {
			f(Mut::new(&mut slice[row], &mut ticks[row], change_tick));
		}
	}
}

impl<A: Component> WriteQuery for (Entity, &mut A) {
	type Item<'w> = (Entity, &'w mut A);
	type State = Option<ComponentId>;

	#[inline]
	fn init(registry: &ComponentRegistry, masks: &mut FilterMasks) -> Self::State {
		<&mut A as QueryTerm>::init(registry, masks)
	}

	#[inline]
	fn visit_archetype_mut<F: QueryFilter>(
		state: Self::State,
		filter_state: F::State,
		archetype: &mut Archetype,
		since_tick: u32,
		change_tick: u32,
		f: &mut impl FnMut(Self::Item<'_>),
	) {
		if archetype.is_empty() || !F::matches_archetype(filter_state, archetype, since_tick) {
			return;
		}
		let col = archetype.column_index(state.unwrap()).unwrap();
		if F::ARCHETYPE_ONLY {
			let entities = &archetype.entities;
			let sa = downcast_col_mut::<A>(archetype.columns[col].as_mut())
				.as_mut_slice_unchecked_ticks(change_tick);
			for (&e, va) in entities.iter().zip(sa) {
				f((e, va));
			}
			return;
		}
		let matched: Vec<usize> = {
			let filter = F::prepare(filter_state, archetype);
			(0..archetype.len())
				.filter(|&row| F::matches_row(&filter, row, since_tick))
				.collect()
		};
		let entities = &archetype.entities;
		let (sa, ta) = downcast_col_mut::<A>(archetype.columns[col].as_mut())
			.as_mut_slice_with_ticks(change_tick);
		for row in matched {
			ta[row].changed = change_tick;
			f((entities[row], &mut sa[row]));
		}
	}
}

impl<A: Component, B: Component> WriteQuery for (&mut A, &B) {
	type Item<'w> = (&'w mut A, &'w B);
	type State = (Option<ComponentId>, Option<ComponentId>);

	#[inline]
	fn init(registry: &ComponentRegistry, masks: &mut FilterMasks) -> Self::State {
		(
			<&mut A as QueryTerm>::init(registry, masks),
			<&B as QueryTerm>::init(registry, masks),
		)
	}

	#[inline]
	fn visit_archetype_mut<F: QueryFilter>(
		state: Self::State,
		filter_state: F::State,
		archetype: &mut Archetype,
		since_tick: u32,
		change_tick: u32,
		f: &mut impl FnMut(Self::Item<'_>),
	) {
		if archetype.is_empty() || !F::matches_archetype(filter_state, archetype, since_tick) {
			return;
		}
		let ca = archetype.column_index(state.0.unwrap()).unwrap();
		let cb = archetype.column_index(state.1.unwrap()).unwrap();
		if F::ARCHETYPE_ONLY {
			let [col_a, col_b] = archetype
				.columns
				.get_disjoint_mut([ca, cb])
				.expect("query cannot borrow the same component mutably and immutably");
			let sa =
				downcast_col_mut::<A>(col_a.as_mut()).as_mut_slice_unchecked_ticks(change_tick);
			let sb = downcast_col::<B>(col_b.as_ref()).as_slice();
			for (va, vb) in sa.iter_mut().zip(sb) {
				f((va, vb));
			}
			return;
		}
		let matched: Vec<usize> = {
			let filter = F::prepare(filter_state, archetype);
			(0..archetype.len())
				.filter(|&row| F::matches_row(&filter, row, since_tick))
				.collect()
		};
		if matched.is_empty() {
			return;
		}
		let [col_a, col_b] = archetype
			.columns
			.get_disjoint_mut([ca, cb])
			.expect("query cannot borrow the same component mutably and immutably");
		let (sa, ta) = downcast_col_mut::<A>(col_a.as_mut()).as_mut_slice_with_ticks(change_tick);
		let sb = downcast_col::<B>(col_b.as_ref()).as_slice();
		for row in matched {
			ta[row].changed = change_tick;
			f((&mut sa[row], &sb[row]));
		}
	}
}

impl<A: Component, B: Component> WriteQuery for (Entity, &mut A, &B) {
	type Item<'w> = (Entity, &'w mut A, &'w B);
	type State = (Option<ComponentId>, Option<ComponentId>);

	#[inline]
	fn init(registry: &ComponentRegistry, masks: &mut FilterMasks) -> Self::State {
		(
			<&mut A as QueryTerm>::init(registry, masks),
			<&B as QueryTerm>::init(registry, masks),
		)
	}

	#[inline]
	fn visit_archetype_mut<F: QueryFilter>(
		state: Self::State,
		filter_state: F::State,
		archetype: &mut Archetype,
		since_tick: u32,
		change_tick: u32,
		f: &mut impl FnMut(Self::Item<'_>),
	) {
		if archetype.is_empty() || !F::matches_archetype(filter_state, archetype, since_tick) {
			return;
		}
		let ca = archetype.column_index(state.0.unwrap()).unwrap();
		let cb = archetype.column_index(state.1.unwrap()).unwrap();
		if F::ARCHETYPE_ONLY {
			let entities = &archetype.entities;
			let [col_a, col_b] = archetype
				.columns
				.get_disjoint_mut([ca, cb])
				.expect("query cannot borrow the same component mutably and immutably");
			let sa =
				downcast_col_mut::<A>(col_a.as_mut()).as_mut_slice_unchecked_ticks(change_tick);
			let sb = downcast_col::<B>(col_b.as_ref()).as_slice();
			for ((&e, va), vb) in entities.iter().zip(sa).zip(sb) {
				f((e, va, vb));
			}
			return;
		}
		let matched: Vec<usize> = {
			let filter = F::prepare(filter_state, archetype);
			(0..archetype.len())
				.filter(|&row| F::matches_row(&filter, row, since_tick))
				.collect()
		};
		if matched.is_empty() {
			return;
		}
		let entities = &archetype.entities;
		let [col_a, col_b] = archetype
			.columns
			.get_disjoint_mut([ca, cb])
			.expect("query cannot borrow the same component mutably and immutably");
		let (sa, ta) = downcast_col_mut::<A>(col_a.as_mut()).as_mut_slice_with_ticks(change_tick);
		let sb = downcast_col::<B>(col_b.as_ref()).as_slice();
		for row in matched {
			ta[row].changed = change_tick;
			f((entities[row], &mut sa[row], &sb[row]));
		}
	}
}

impl<A: Component, B: Component> WriteQuery for (&mut A, &mut B) {
	type Item<'w> = (&'w mut A, &'w mut B);
	type State = (Option<ComponentId>, Option<ComponentId>);

	#[inline]
	fn init(registry: &ComponentRegistry, masks: &mut FilterMasks) -> Self::State {
		(
			<&mut A as QueryTerm>::init(registry, masks),
			<&mut B as QueryTerm>::init(registry, masks),
		)
	}

	#[inline]
	fn visit_archetype_mut<F: QueryFilter>(
		state: Self::State,
		filter_state: F::State,
		archetype: &mut Archetype,
		since_tick: u32,
		change_tick: u32,
		f: &mut impl FnMut(Self::Item<'_>),
	) {
		if archetype.is_empty() || !F::matches_archetype(filter_state, archetype, since_tick) {
			return;
		}
		let ca = archetype.column_index(state.0.unwrap()).unwrap();
		let cb = archetype.column_index(state.1.unwrap()).unwrap();
		if F::ARCHETYPE_ONLY {
			let [col_a, col_b] = archetype
				.columns
				.get_disjoint_mut([ca, cb])
				.expect("query cannot borrow the same component mutably twice");
			let sa =
				downcast_col_mut::<A>(col_a.as_mut()).as_mut_slice_unchecked_ticks(change_tick);
			let sb =
				downcast_col_mut::<B>(col_b.as_mut()).as_mut_slice_unchecked_ticks(change_tick);
			for (va, vb) in sa.iter_mut().zip(sb) {
				f((va, vb));
			}
			return;
		}
		let matched: Vec<usize> = {
			let filter = F::prepare(filter_state, archetype);
			(0..archetype.len())
				.filter(|&row| F::matches_row(&filter, row, since_tick))
				.collect()
		};
		if matched.is_empty() {
			return;
		}
		let [col_a, col_b] = archetype
			.columns
			.get_disjoint_mut([ca, cb])
			.expect("query cannot borrow the same component mutably twice");
		let (sa, ta) = downcast_col_mut::<A>(col_a.as_mut()).as_mut_slice_with_ticks(change_tick);
		let (sb, tb) = downcast_col_mut::<B>(col_b.as_mut()).as_mut_slice_with_ticks(change_tick);
		for row in matched {
			ta[row].changed = change_tick;
			tb[row].changed = change_tick;
			f((&mut sa[row], &mut sb[row]));
		}
	}
}

impl<A: Component, B: Component, C: Component> WriteQuery for (&mut A, &B, &C) {
	type Item<'w> = (&'w mut A, &'w B, &'w C);
	type State = (
		Option<ComponentId>,
		Option<ComponentId>,
		Option<ComponentId>,
	);

	#[inline]
	fn init(registry: &ComponentRegistry, masks: &mut FilterMasks) -> Self::State {
		(
			<&mut A as QueryTerm>::init(registry, masks),
			<&B as QueryTerm>::init(registry, masks),
			<&C as QueryTerm>::init(registry, masks),
		)
	}

	#[inline]
	fn visit_archetype_mut<F: QueryFilter>(
		state: Self::State,
		filter_state: F::State,
		archetype: &mut Archetype,
		since_tick: u32,
		change_tick: u32,
		f: &mut impl FnMut(Self::Item<'_>),
	) {
		if archetype.is_empty() || !F::matches_archetype(filter_state, archetype, since_tick) {
			return;
		}
		let ca = archetype.column_index(state.0.unwrap()).unwrap();
		let cb = archetype.column_index(state.1.unwrap()).unwrap();
		let cc = archetype.column_index(state.2.unwrap()).unwrap();
		if F::ARCHETYPE_ONLY {
			let [col_a, col_b, col_c] = archetype
				.columns
				.get_disjoint_mut([ca, cb, cc])
				.expect("query has conflicting column borrows");
			let sa =
				downcast_col_mut::<A>(col_a.as_mut()).as_mut_slice_unchecked_ticks(change_tick);
			let sb = downcast_col::<B>(col_b.as_ref()).as_slice();
			let sc = downcast_col::<C>(col_c.as_ref()).as_slice();
			for ((va, vb), vc) in sa.iter_mut().zip(sb).zip(sc) {
				f((va, vb, vc));
			}
			return;
		}
		let matched: Vec<usize> = {
			let filter = F::prepare(filter_state, archetype);
			(0..archetype.len())
				.filter(|&row| F::matches_row(&filter, row, since_tick))
				.collect()
		};
		if matched.is_empty() {
			return;
		}
		let [col_a, col_b, col_c] = archetype
			.columns
			.get_disjoint_mut([ca, cb, cc])
			.expect("query has conflicting column borrows");
		let (sa, ta) = downcast_col_mut::<A>(col_a.as_mut()).as_mut_slice_with_ticks(change_tick);
		let sb = downcast_col::<B>(col_b.as_ref()).as_slice();
		let sc = downcast_col::<C>(col_c.as_ref()).as_slice();
		for row in matched {
			ta[row].changed = change_tick;
			f((&mut sa[row], &sb[row], &sc[row]));
		}
	}
}

impl<A: Component, B: Component, C: Component> WriteQuery for (&mut A, &mut B, &C) {
	type Item<'w> = (&'w mut A, &'w mut B, &'w C);
	type State = (
		Option<ComponentId>,
		Option<ComponentId>,
		Option<ComponentId>,
	);

	#[inline]
	fn init(registry: &ComponentRegistry, masks: &mut FilterMasks) -> Self::State {
		(
			<&mut A as QueryTerm>::init(registry, masks),
			<&mut B as QueryTerm>::init(registry, masks),
			<&C as QueryTerm>::init(registry, masks),
		)
	}

	#[inline]
	fn visit_archetype_mut<F: QueryFilter>(
		state: Self::State,
		filter_state: F::State,
		archetype: &mut Archetype,
		since_tick: u32,
		change_tick: u32,
		f: &mut impl FnMut(Self::Item<'_>),
	) {
		if archetype.is_empty() || !F::matches_archetype(filter_state, archetype, since_tick) {
			return;
		}
		let ca = archetype.column_index(state.0.unwrap()).unwrap();
		let cb = archetype.column_index(state.1.unwrap()).unwrap();
		let cc = archetype.column_index(state.2.unwrap()).unwrap();
		if F::ARCHETYPE_ONLY {
			let [col_a, col_b, col_c] = archetype
				.columns
				.get_disjoint_mut([ca, cb, cc])
				.expect("query has conflicting column borrows");
			let sa =
				downcast_col_mut::<A>(col_a.as_mut()).as_mut_slice_unchecked_ticks(change_tick);
			let sb =
				downcast_col_mut::<B>(col_b.as_mut()).as_mut_slice_unchecked_ticks(change_tick);
			let sc = downcast_col::<C>(col_c.as_ref()).as_slice();
			for ((va, vb), vc) in sa.iter_mut().zip(sb).zip(sc) {
				f((va, vb, vc));
			}
			return;
		}
		let matched: Vec<usize> = {
			let filter = F::prepare(filter_state, archetype);
			(0..archetype.len())
				.filter(|&row| F::matches_row(&filter, row, since_tick))
				.collect()
		};
		if matched.is_empty() {
			return;
		}
		let [col_a, col_b, col_c] = archetype
			.columns
			.get_disjoint_mut([ca, cb, cc])
			.expect("query has conflicting column borrows");
		let (sa, ta) = downcast_col_mut::<A>(col_a.as_mut()).as_mut_slice_with_ticks(change_tick);
		let (sb, tb) = downcast_col_mut::<B>(col_b.as_mut()).as_mut_slice_with_ticks(change_tick);
		let sc = downcast_col::<C>(col_c.as_ref()).as_slice();
		for row in matched {
			ta[row].changed = change_tick;
			tb[row].changed = change_tick;
			f((&mut sa[row], &mut sb[row], &sc[row]));
		}
	}
}
