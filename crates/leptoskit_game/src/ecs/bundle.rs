//! Component bundles for ergonomic, zero-overhead single and batch spawning.
//!
//! `()` and tuples of 1..=10 distinct [`Component`]s implement [`Bundle`].
//! [`Bundle::push_batch`] uses pre-resolved column indices, bulk data writes,
//! and a single bulk tick fill per column so spawning 100,000 entities runs at
//! raw memory-bandwidth speed.

use crate::ecs::archetype::Archetype;
use crate::ecs::column::downcast_col_mut;
use crate::ecs::component::Component;
use crate::ecs::component::ComponentId;
use crate::ecs::component::ComponentRegistry;

/// Maximum number of components in a built-in tuple bundle.
pub(crate) const MAX_BUNDLE_COLS: usize = 10;

/// A statically known set of components spawned together on an entity.
pub trait Bundle: Send + Sync + 'static {
	/// Registers every component in this bundle and appends its
	/// [`ComponentId`] to `ids`.
	fn register_ids(registry: &mut ComponentRegistry, ids: &mut Vec<ComponentId>);

	/// Pushes one bundle's components into `archetype` using pre-resolved
	/// column indices `cols` at `tick`.
	fn push_into(self, archetype: &mut Archetype, cols: &[usize], tick: u32);

	/// Pushes an iterator of bundles directly into `archetype` using
	/// pre-resolved column indices `cols`, reserving data once and filling
	/// ticks once at the end.
	fn push_batch<I>(iter: I, archetype: &mut Archetype, cols: &[usize], tick: u32) -> usize
	where
		Self: Sized,
		I: IntoIterator<Item = Self>;
}

impl Bundle for () {
	fn register_ids(_registry: &mut ComponentRegistry, _ids: &mut Vec<ComponentId>) {}

	fn push_into(self, _archetype: &mut Archetype, _cols: &[usize], _tick: u32) {}

	fn push_batch<I>(iter: I, _archetype: &mut Archetype, _cols: &[usize], _tick: u32) -> usize
	where
		I: IntoIterator<Item = Self>,
	{
		iter.into_iter().count()
	}
}

impl<T0: Component> Bundle for (T0,) {
	#[inline]
	fn register_ids(registry: &mut ComponentRegistry, ids: &mut Vec<ComponentId>) {
		ids.push(registry.register::<T0>());
	}

	#[inline]
	fn push_into(self, archetype: &mut Archetype, cols: &[usize], tick: u32) {
		downcast_col_mut::<T0>(archetype.columns[cols[0]].as_mut()).push(self.0, tick);
	}

	fn push_batch<I>(iter: I, archetype: &mut Archetype, cols: &[usize], tick: u32) -> usize
	where
		I: IntoIterator<Item = Self>,
	{
		let iter = iter.into_iter();
		let (lower, _) = iter.size_hint();
		let prev_len = archetype.len();
		let col = downcast_col_mut::<T0>(archetype.columns[cols[0]].as_mut());
		col.reserve_data(lower);
		let mut count = 0;
		for (item,) in iter {
			col.push_data_only(item);
			count += 1;
		}
		col.finish_batch(prev_len, tick);
		count
	}
}

macro_rules! impl_bundle_tuple {
	($(($idx:tt, $name:ident, $col:ident, $val:ident)),+ $(,)?) => {
		impl<$($name: Component),+> Bundle for ($($name,)+) {
			#[inline]
			fn register_ids(registry: &mut ComponentRegistry, ids: &mut Vec<ComponentId>) {
				$(ids.push(registry.register::<$name>());)+
			}

			#[inline]
			fn push_into(self, archetype: &mut Archetype, cols: &[usize], tick: u32) {
				let [$($col,)+] = archetype
					.columns
					.get_disjoint_mut([$(cols[$idx],)+])
					.expect("bundle contains duplicate component types");
				$(
					downcast_col_mut::<$name>($col.as_mut()).push(self.$idx, tick);
				)+
			}

			fn push_batch<I>(
				iter: I,
				archetype: &mut Archetype,
				cols: &[usize],
				tick: u32,
			) -> usize
			where
				I: IntoIterator<Item = Self>,
			{
				let iter = iter.into_iter();
				let (lower, _) = iter.size_hint();
				let prev_len = archetype.len();
				let [$($col,)+] = archetype
					.columns
					.get_disjoint_mut([$(cols[$idx],)+])
					.expect("bundle contains duplicate component types");
				$(
					let $col = downcast_col_mut::<$name>($col.as_mut());
					$col.reserve_data(lower);
				)+
				let mut count = 0;
				for ($($val,)+) in iter {
					$($col.push_data_only($val);)+
					count += 1;
				}
				$(
					$col.finish_batch(prev_len, tick);
				)+
				count
			}
		}
	};
}

impl_bundle_tuple!((0, T0, c0, v0), (1, T1, c1, v1));
impl_bundle_tuple!((0, T0, c0, v0), (1, T1, c1, v1), (2, T2, c2, v2));
impl_bundle_tuple!(
	(0, T0, c0, v0),
	(1, T1, c1, v1),
	(2, T2, c2, v2),
	(3, T3, c3, v3),
);
impl_bundle_tuple!(
	(0, T0, c0, v0),
	(1, T1, c1, v1),
	(2, T2, c2, v2),
	(3, T3, c3, v3),
	(4, T4, c4, v4),
);
impl_bundle_tuple!(
	(0, T0, c0, v0),
	(1, T1, c1, v1),
	(2, T2, c2, v2),
	(3, T3, c3, v3),
	(4, T4, c4, v4),
	(5, T5, c5, v5),
);
impl_bundle_tuple!(
	(0, T0, c0, v0),
	(1, T1, c1, v1),
	(2, T2, c2, v2),
	(3, T3, c3, v3),
	(4, T4, c4, v4),
	(5, T5, c5, v5),
	(6, T6, c6, v6),
);
impl_bundle_tuple!(
	(0, T0, c0, v0),
	(1, T1, c1, v1),
	(2, T2, c2, v2),
	(3, T3, c3, v3),
	(4, T4, c4, v4),
	(5, T5, c5, v5),
	(6, T6, c6, v6),
	(7, T7, c7, v7),
);
impl_bundle_tuple!(
	(0, T0, c0, v0),
	(1, T1, c1, v1),
	(2, T2, c2, v2),
	(3, T3, c3, v3),
	(4, T4, c4, v4),
	(5, T5, c5, v5),
	(6, T6, c6, v6),
	(7, T7, c7, v7),
	(8, T8, c8, v8),
);
impl_bundle_tuple!(
	(0, T0, c0, v0),
	(1, T1, c1, v1),
	(2, T2, c2, v2),
	(3, T3, c3, v3),
	(4, T4, c4, v4),
	(5, T5, c5, v5),
	(6, T6, c6, v6),
	(7, T7, c7, v7),
	(8, T8, c8, v8),
	(9, T9, c9, v9),
);

/// Sorts and verifies that a bundle's component IDs contain no duplicates.
///
/// # Panics
///
/// Panics if the bundle lists the same component type twice.
pub(crate) fn canonicalize_bundle_ids(raw: &[ComponentId]) -> Vec<ComponentId> {
	let mut sorted = raw.to_vec();
	sorted.sort_unstable();
	for pair in sorted.windows(2) {
		assert_ne!(
			pair[0], pair[1],
			"a bundle cannot contain duplicate component types"
		);
	}
	sorted
}
