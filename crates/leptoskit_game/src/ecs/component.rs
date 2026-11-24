//! Component registration, dense integer IDs, inline 256-bit signature masks,
//! and change-detection ticks.

use std::any::TypeId;

use crate::ecs::column::Column;
use crate::ecs::column::TypedColumn;
use crate::ecs::hash::FastMap;

/// Marker trait implemented automatically for every type that can be stored as
/// an ECS component.
///
/// Unlike Godot's single-inheritance node hierarchy, any plain Rust struct or
/// enum implementing `'static + Send + Sync` is a first-class component with
/// zero derive boilerplate.
pub trait Component: 'static + Send + Sync {}

impl<T: 'static + Send + Sync> Component for T {}

/// Dense integer identifier assigned once per component Rust type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ComponentId(pub u32);

/// Per-component change-detection ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ComponentTicks {
	/// World change tick when the component was inserted on this entity.
	pub added: u32,
	/// World change tick when the component was last mutated on this entity.
	pub changed: u32,
}

impl ComponentTicks {
	/// Creates ticks for a newly inserted component at `tick`.
	#[inline]
	pub const fn new(tick: u32) -> Self {
		Self {
			added: tick,
			changed: tick,
		}
	}

	/// Returns `true` if this component was added strictly after `since`.
	#[inline]
	pub const fn is_added(self, since: u32) -> bool {
		self.added.wrapping_sub(since) != 0 && self.added.wrapping_sub(since) < (u32::MAX / 2)
	}

	/// Returns `true` if this component was changed strictly after `since`.
	#[inline]
	pub const fn is_changed(self, since: u32) -> bool {
		self.changed.wrapping_sub(since) != 0 && self.changed.wrapping_sub(since) < (u32::MAX / 2)
	}
}

/// Inline 256-bit component bitset with automatic spill to heap when a world
/// registers more than 256 distinct component types.
///
/// Testing whether an archetype matches a query takes 4 bitwise AND
/// instructions on the stack with zero heap allocations.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub(crate) struct ComponentMask {
	inline: [u64; 4],
	extra: Option<Box<[u64]>>,
}

impl ComponentMask {
	/// Sets the bit for `id`.
	#[inline]
	pub fn set(&mut self, id: ComponentId) {
		let word = (id.0 as usize) >> 6;
		let bit = 1_u64 << (id.0 & 63);
		if word < 4 {
			self.inline[word] |= bit;
			return;
		}
		let extra_idx = word - 4;
		let mut vec = self.extra.take().map_or_else(Vec::new, Vec::from);
		if extra_idx >= vec.len() {
			vec.resize(extra_idx + 1, 0);
		}
		vec[extra_idx] |= bit;
		self.extra = Some(vec.into_boxed_slice());
	}

	/// Returns `true` if every bit in `required` is set in `self`.
	#[inline]
	pub fn contains_all(&self, required: &Self) -> bool {
		let inline_ok = (self.inline[0] & required.inline[0]) == required.inline[0]
			&& (self.inline[1] & required.inline[1]) == required.inline[1]
			&& (self.inline[2] & required.inline[2]) == required.inline[2]
			&& (self.inline[3] & required.inline[3]) == required.inline[3];
		if !inline_ok {
			return false;
		}
		match (&self.extra, &required.extra) {
			(_, None) => true,
			(None, Some(req)) => req.iter().all(|&w| w == 0),
			(Some(own), Some(req)) => {
				for (index, &rw) in req.iter().enumerate() {
					let ow = own.get(index).copied().unwrap_or(0);
					if (ow & rw) != rw {
						return false;
					}
				}
				true
			}
		}
	}

	/// Returns `true` if `self` shares no bits with `excluded`.
	#[inline]
	pub fn is_disjoint(&self, excluded: &Self) -> bool {
		let inline_disjoint = (self.inline[0] & excluded.inline[0]) == 0
			&& (self.inline[1] & excluded.inline[1]) == 0
			&& (self.inline[2] & excluded.inline[2]) == 0
			&& (self.inline[3] & excluded.inline[3]) == 0;
		if !inline_disjoint {
			return false;
		}
		match (&self.extra, &excluded.extra) {
			(Some(own), Some(exc)) => own.iter().zip(exc.iter()).all(|(&a, &b)| (a & b) == 0),
			_ => true,
		}
	}
}

/// Metadata stored for each registered component type.
pub(crate) struct ComponentInfo {
	pub make_column: fn() -> Box<dyn Column>,
}

/// Registry mapping Rust `TypeId`s to dense [`ComponentId`]s and column
/// factories.
#[doc(hidden)]
#[derive(Default)]
pub struct ComponentRegistry {
	by_type: FastMap<TypeId, ComponentId>,
	infos: Vec<ComponentInfo>,
}

impl ComponentRegistry {
	/// Returns the [`ComponentId`] for `T`, registering it on first sight.
	#[inline]
	pub(crate) fn register<T: Component>(&mut self) -> ComponentId {
		let type_id = TypeId::of::<T>();
		if let Some(&id) = self.by_type.get(&type_id) {
			return id;
		}
		let id = ComponentId(
			u32::try_from(self.infos.len()).expect("component count exceeded u32::MAX"),
		);
		self.by_type.insert(type_id, id);
		self.infos.push(ComponentInfo {
			make_column: || Box::new(TypedColumn::<T>::new()),
		});
		id
	}

	/// Looks up an already-registered component id without mutating.
	#[inline]
	pub(crate) fn id_of<T: Component>(&self) -> Option<ComponentId> {
		self.by_type.get(&TypeId::of::<T>()).copied()
	}

	/// Creates a fresh empty typed column for `id`.
	#[inline]
	pub(crate) fn make_column(&self, id: ComponentId) -> Box<dyn Column> {
		(self.infos[id.0 as usize].make_column)()
	}

	/// Number of registered component types.
	#[inline]
	pub(crate) fn len(&self) -> usize {
		self.infos.len()
	}
}
