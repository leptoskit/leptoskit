//! Generational entity identifiers and O(1) entity allocation.
//!
//! Each [`Entity`] packs a 32-bit slot index and a 32-bit non-zero generation
//! into a single `u64`, eliminating ABA stale-handle bugs with zero heap
//! indirection.

use std::fmt;

use crate::ecs::archetype::ArchetypeId;

/// A generational handle to an entity inside a [`World`](crate::World).
///
/// Copying, comparing, and hashing an `Entity` operate on a single `u64`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Entity(u64);

impl Entity {
	/// A sentinel handle that never refers to any live entity.
	pub const PLACEHOLDER: Self = Self(u64::MAX);

	/// Constructs an entity from its raw slot `index` and `generation`.
	#[inline]
	pub(crate) const fn from_parts(index: u32, generation: u32) -> Self {
		Self(((generation as u64) << 32) | (index as u64))
	}

	/// The dense slot index.
	#[inline]
	pub const fn index(self) -> u32 {
		self.0 as u32
	}

	/// The slot generation, incremented each time the slot is recycled.
	#[inline]
	pub const fn generation(self) -> u32 {
		(self.0 >> 32) as u32
	}

	/// Packed 64-bit representation for compact serialization or keys.
	#[inline]
	pub const fn to_bits(self) -> u64 {
		self.0
	}

	/// Reconstructs an entity handle from [`Entity::to_bits`].
	#[inline]
	pub const fn from_bits(bits: u64) -> Self {
		Self(bits)
	}
}

impl fmt::Debug for Entity {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "Entity({}v{})", self.index(), self.generation())
	}
}

impl fmt::Display for Entity {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}v{}", self.index(), self.generation())
	}
}

/// Where a live entity's components are stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EntityLocation {
	pub archetype: ArchetypeId,
	pub row: u32,
}

impl EntityLocation {
	pub const UNASSIGNED: Self = Self {
		archetype: ArchetypeId::INVALID,
		row: u32::MAX,
	};

	#[inline]
	pub const fn is_alive(self) -> bool {
		self.archetype.0 != ArchetypeId::INVALID.0
	}
}

/// Per-slot metadata in the dense entity directory.
#[derive(Debug, Clone, Copy)]
struct Slot {
	generation: u32,
	location: EntityLocation,
}

/// Dense generational entity table with O(1) free-list recycling and bulk
/// allocation.
#[derive(Debug, Default)]
pub(crate) struct EntityAllocator {
	slots: Vec<Slot>,
	free: Vec<u32>,
	alive: usize,
}

impl EntityAllocator {
	/// Allocates a single entity handle and records its initial `location`.
	#[inline]
	pub fn alloc(&mut self, location: EntityLocation) -> Entity {
		self.alive += 1;
		if let Some(index) = self.free.pop() {
			let slot = &mut self.slots[index as usize];
			slot.location = location;
			Entity::from_parts(index, slot.generation)
		} else {
			let index = u32::try_from(self.slots.len()).expect("entity capacity exceeded u32::MAX");
			self.slots.push(Slot {
				generation: 1,
				location,
			});
			Entity::from_parts(index, 1)
		}
	}

	/// Allocates `count` entities consecutively placed into `archetype`
	/// starting at `start_row`, appending them into `out`.
	pub fn alloc_batch(
		&mut self,
		archetype: ArchetypeId,
		start_row: u32,
		count: usize,
		out: &mut Vec<Entity>,
	) {
		if count == 0 {
			return;
		}
		out.reserve(count);
		self.alive += count;

		let reuse = self.free.len().min(count);
		let mut row = start_row;
		for _ in 0..reuse {
			let index = self.free.pop().expect("reuse bounded by free.len()");
			let slot = &mut self.slots[index as usize];
			slot.location = EntityLocation { archetype, row };
			out.push(Entity::from_parts(index, slot.generation));
			row += 1;
		}

		let fresh = count - reuse;
		if fresh > 0 {
			let base_index = self.slots.len();
			self.slots.reserve(fresh);
			for offset in 0..fresh {
				let index =
					u32::try_from(base_index + offset).expect("entity capacity exceeded u32::MAX");
				self.slots.push(Slot {
					generation: 1,
					location: EntityLocation { archetype, row },
				});
				out.push(Entity::from_parts(index, 1));
				row += 1;
			}
		}
	}

	/// Frees `entity` if it is alive, returning its previous location and
	/// advancing its slot generation so stale handles are rejected.
	#[inline]
	pub fn free(&mut self, entity: Entity) -> Option<EntityLocation> {
		let index = entity.index() as usize;
		let slot = self.slots.get_mut(index)?;
		if slot.generation != entity.generation() || !slot.location.is_alive() {
			return None;
		}
		let previous = slot.location;
		slot.location = EntityLocation::UNASSIGNED;
		slot.generation = slot.generation.wrapping_add(1).max(1);
		self.free.push(entity.index());
		self.alive -= 1;
		Some(previous)
	}

	/// Returns the location of `entity` if it is currently alive.
	#[inline]
	pub fn location(&self, entity: Entity) -> Option<EntityLocation> {
		let slot = self.slots.get(entity.index() as usize)?;
		if slot.generation == entity.generation() && slot.location.is_alive() {
			Some(slot.location)
		} else {
			None
		}
	}

	/// Updates the recorded location of an already-live entity (e.g. after a
	/// swap-remove or archetype migration).
	#[inline]
	pub fn set_location(&mut self, entity: Entity, location: EntityLocation) {
		if let Some(slot) = self.slots.get_mut(entity.index() as usize)
			&& slot.generation == entity.generation()
		{
			slot.location = location;
		}
	}

	/// Whether `entity` refers to a currently live entity.
	#[inline]
	pub fn is_alive(&self, entity: Entity) -> bool {
		self.location(entity).is_some()
	}

	/// Number of live entities.
	#[inline]
	pub fn len(&self) -> usize {
		self.alive
	}
}
