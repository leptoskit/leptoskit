//! Safe type-erased contiguous `SoA` columns for archetype tables.
//!
//! Each column stores values in a tightly packed `Vec<T>` with **lazy
//! per-row change-detection ticks**: when every row in a column shares the
//! same `(added, changed)` tick (the common case for `spawn_batch` and
//! whole-column systems), `ticks` stays completely unallocated (`0 bytes`) and
//! `uniform_ticks` records the column's tick in O(1). Per-row ticks are
//! materialized lazily only when an individual row diverges.

use std::any::Any;

use crate::ecs::component::Component;
use crate::ecs::component::ComponentTicks;

/// Type-erased column operations shared across archetype tables.
pub(crate) trait Column: Send + Sync {
	fn as_any(&self) -> &dyn Any;
	fn as_any_mut(&mut self) -> &mut dyn Any;
	fn swap_remove_drop(&mut self, row: usize);
	/// Moves the element at `row` from `self` (using O(1) swap-remove) into
	/// `dst` without boxing the value.
	fn move_row_to(&mut self, row: usize, dst: &mut dyn Column);
	/// Column-wide highest added tick.
	fn max_added_tick(&self) -> u32;
	/// Column-wide highest changed tick.
	fn max_changed_tick(&self) -> u32;
}

/// Borrowed view over a column's per-row ticks — either a uniform tick shared
/// by every row (zero allocation) or an explicit per-row slice.
#[doc(hidden)]
#[derive(Clone, Copy)]
pub enum ColumnTicksRef<'a> {
	Uniform(ComponentTicks),
	PerRow(&'a [ComponentTicks]),
}

impl ColumnTicksRef<'_> {
	#[inline]
	pub fn get(self, row: usize) -> ComponentTicks {
		match self {
			Self::Uniform(ticks) => ticks,
			Self::PerRow(slice) => slice[row],
		}
	}
}

/// A contiguous `SoA` storage column for component `T`.
pub(crate) struct TypedColumn<T: Component> {
	data: Vec<T>,
	/// Empty while every row shares `uniform_ticks`; populated lazily when
	/// rows diverge in tick value.
	ticks: Vec<ComponentTicks>,
	uniform_ticks: ComponentTicks,
	max_added: u32,
	max_changed: u32,
}

impl<T: Component> TypedColumn<T> {
	#[inline]
	pub fn new() -> Self {
		Self {
			data: Vec::new(),
			ticks: Vec::new(),
			uniform_ticks: ComponentTicks::default(),
			max_added: 0,
			max_changed: 0,
		}
	}

	/// Ensures per-row ticks are materialized for `self.data.len()` rows.
	#[inline]
	fn materialize_ticks(&mut self) {
		if self.ticks.is_empty() && !self.data.is_empty() {
			self.ticks.resize(self.data.len(), self.uniform_ticks);
		}
	}

	#[inline]
	pub fn push(&mut self, value: T, tick: u32) {
		let entry = ComponentTicks::new(tick);
		if self.data.is_empty() {
			self.uniform_ticks = entry;
		} else if !self.ticks.is_empty() {
			self.ticks.push(entry);
		} else if self.uniform_ticks != entry {
			self.ticks.reserve(self.data.len() + 1);
			self.ticks.resize(self.data.len(), self.uniform_ticks);
			self.ticks.push(entry);
		}
		self.data.push(value);
		self.max_added = tick;
		self.max_changed = tick;
	}

	/// Reserves capacity in the contiguous data vector only (for bulk batch
	/// spawning paired with [`TypedColumn::finish_batch`]).
	#[inline]
	pub fn reserve_data(&mut self, additional: usize) {
		self.data.reserve(additional);
	}

	/// Appends a value without touching ticks; caller MUST invoke
	/// [`TypedColumn::finish_batch`] once after the loop.
	#[inline]
	pub fn push_data_only(&mut self, value: T) {
		self.data.push(value);
	}

	/// Finalizes ticks for a bulk batch added since `prev_len` in O(1) when
	/// uniform, or a single bulk `resize` if per-row ticks already exist.
	#[inline]
	pub fn finish_batch(&mut self, prev_len: usize, tick: u32) {
		if self.data.len() == prev_len {
			return;
		}
		let entry = ComponentTicks::new(tick);
		if prev_len == 0 {
			self.uniform_ticks = entry;
		} else if !self.ticks.is_empty() {
			self.ticks.resize(self.data.len(), entry);
		} else if self.uniform_ticks != entry {
			self.ticks.reserve(self.data.len());
			self.ticks.resize(prev_len, self.uniform_ticks);
			self.ticks.resize(self.data.len(), entry);
		}
		self.max_added = tick;
		self.max_changed = tick;
	}

	#[inline]
	pub fn push_with_ticks(&mut self, value: T, entry: ComponentTicks) {
		if self.data.is_empty() {
			self.uniform_ticks = entry;
		} else if !self.ticks.is_empty() {
			self.ticks.push(entry);
		} else if self.uniform_ticks != entry {
			self.ticks.reserve(self.data.len() + 1);
			self.ticks.resize(self.data.len(), self.uniform_ticks);
			self.ticks.push(entry);
		}
		self.data.push(value);
		if entry.added > self.max_added {
			self.max_added = entry.added;
		}
		if entry.changed > self.max_changed {
			self.max_changed = entry.changed;
		}
	}

	#[inline]
	pub fn get(&self, row: usize) -> &T {
		&self.data[row]
	}

	#[inline]
	pub fn get_mut(&mut self, row: usize, tick: u32) -> &mut T {
		if self.ticks.is_empty() && self.uniform_ticks.changed != tick {
			self.materialize_ticks();
		}
		if !self.ticks.is_empty() {
			self.ticks[row].changed = tick;
		}
		self.max_changed = tick;
		&mut self.data[row]
	}

	#[inline]
	pub fn set(&mut self, row: usize, value: T, tick: u32) -> T {
		if self.ticks.is_empty() && self.uniform_ticks.changed != tick {
			self.materialize_ticks();
		}
		if !self.ticks.is_empty() {
			self.ticks[row].changed = tick;
		}
		self.max_changed = tick;
		std::mem::replace(&mut self.data[row], value)
	}

	#[inline]
	pub fn swap_remove(&mut self, row: usize) -> (T, ComponentTicks) {
		let value = self.data.swap_remove(row);
		let ticks = if self.ticks.is_empty() {
			self.uniform_ticks
		} else {
			self.ticks.swap_remove(row)
		};
		(value, ticks)
	}

	#[inline]
	pub fn as_slice(&self) -> &[T] {
		&self.data
	}

	/// Returns a raw mutable slice of all components in this column and stamps
	/// the column's changed tick in O(1) when uniform (zero per-entity
	/// overhead, enabling full LLVM AVX2/NEON auto-vectorization).
	#[inline]
	pub fn as_mut_slice_unchecked_ticks(&mut self, tick: u32) -> &mut [T] {
		self.max_changed = tick;
		if self.ticks.is_empty() {
			self.uniform_ticks.changed = tick;
		} else {
			for entry in &mut self.ticks {
				entry.changed = tick;
			}
		}
		&mut self.data
	}

	/// Returns both the mutable component slice and the materialized parallel
	/// ticks slice for fine-grained per-entity change tracking.
	#[inline]
	pub fn as_mut_slice_with_ticks(&mut self, tick: u32) -> (&mut [T], &mut [ComponentTicks]) {
		self.materialize_ticks();
		self.max_changed = tick;
		(&mut self.data, &mut self.ticks)
	}

	#[inline]
	pub fn ticks_ref(&self) -> ColumnTicksRef<'_> {
		if self.ticks.is_empty() {
			ColumnTicksRef::Uniform(self.uniform_ticks)
		} else {
			ColumnTicksRef::PerRow(&self.ticks)
		}
	}
}

impl<T: Component> Column for TypedColumn<T> {
	#[inline]
	fn as_any(&self) -> &dyn Any {
		self
	}

	#[inline]
	fn as_any_mut(&mut self) -> &mut dyn Any {
		self
	}

	#[inline]
	fn swap_remove_drop(&mut self, row: usize) {
		let _ = self.data.swap_remove(row);
		if !self.ticks.is_empty() {
			let _ = self.ticks.swap_remove(row);
		}
	}

	#[inline]
	fn move_row_to(&mut self, row: usize, dst: &mut dyn Column) {
		let (value, ticks) = self.swap_remove(row);
		let dst = dst
			.as_any_mut()
			.downcast_mut::<Self>()
			.expect("archetype transition matched mismatched column types");
		dst.push_with_ticks(value, ticks);
	}

	#[inline]
	fn max_added_tick(&self) -> u32 {
		self.max_added
	}

	#[inline]
	fn max_changed_tick(&self) -> u32 {
		self.max_changed
	}
}

/// Downcasts a type-erased column reference to `&TypedColumn<T>`.
#[inline]
pub(crate) fn downcast_col<T: Component>(col: &dyn Column) -> &TypedColumn<T> {
	col.as_any()
		.downcast_ref::<TypedColumn<T>>()
		.expect("column type mismatch")
}

/// Downcasts a mutable type-erased column reference to `&mut TypedColumn<T>`.
#[inline]
pub(crate) fn downcast_col_mut<T: Component>(col: &mut dyn Column) -> &mut TypedColumn<T> {
	col.as_any_mut()
		.downcast_mut::<TypedColumn<T>>()
		.expect("column type mismatch")
}
