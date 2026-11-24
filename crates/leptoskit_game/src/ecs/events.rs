//! Typed double-buffered events (Bevy-style) and synchronous reactive signals /
//! observers (Godot-style).
//!
//! - [`Events<E>`] retains events across two frames in a pair of reusable
//!   buffers (`current` and `previous`) so readers never miss an event and zero
//!   allocations happen after warm-up.
//! - [`ObserverRegistry`] lets gameplay objects and UI bind callbacks directly
//!   to typed signals (`world.observe::<OnDamage>(...)`) globally or scoped to
//!   a specific [`Entity`] (`world.observe_entity::<OnClick>(entity, ...)`).

use std::any::Any;
use std::any::TypeId;
use std::marker::PhantomData;

use crate::ecs::entity::Entity;
use crate::ecs::hash::FastMap;
use crate::world::World;

/// A monotonic sequence number assigned to each sent event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct EventId(pub u64);

/// Double-buffered storage for events of type `E`.
///
/// Call [`Events::update`] once per frame (done automatically by
/// [`GameApp`](crate::ecs::schedule::GameApp) for registered events) to rotate
/// buffers while preserving allocations.
#[derive(Debug)]
pub struct Events<E: 'static> {
	previous: Vec<E>,
	current: Vec<E>,
	/// Monotonic sequence number of `previous[0]`.
	previous_start: u64,
	/// Monotonic sequence number of `current[0]`.
	current_start: u64,
}

impl<E: 'static> Default for Events<E> {
	fn default() -> Self {
		Self::new()
	}
}

impl<E: 'static> Events<E> {
	/// Creates an empty event channel.
	pub fn new() -> Self {
		Self {
			previous: Vec::new(),
			current: Vec::new(),
			previous_start: 0,
			current_start: 0,
		}
	}

	/// Appends an event to the current frame's buffer and returns its sequence
	/// ID.
	#[inline]
	pub fn send(&mut self, event: E) -> EventId {
		let id = self.current_start + self.current.len() as u64;
		self.current.push(event);
		EventId(id)
	}

	/// Appends a batch of events to the current frame's buffer.
	pub fn send_batch(&mut self, events: impl IntoIterator<Item = E>) {
		self.current.extend(events);
	}

	/// Rotates buffers: drops `previous`, moves `current` into `previous`, and
	/// reuses the old `previous` allocation as the new empty `current`.
	pub fn update(&mut self) {
		std::mem::swap(&mut self.previous, &mut self.current);
		self.previous_start = self.current_start;
		self.current_start = self.previous_start + self.previous.len() as u64;
		self.current.clear();
	}

	/// Clears both buffers immediately.
	pub fn clear(&mut self) {
		self.current_start += self.current.len() as u64;
		self.previous_start = self.current_start;
		self.previous.clear();
		self.current.clear();
	}

	/// Total events currently retained across both buffers.
	#[inline]
	pub fn len(&self) -> usize {
		self.previous.len() + self.current.len()
	}

	/// Whether zero events are retained in either buffer.
	#[inline]
	pub fn is_empty(&self) -> bool {
		self.previous.is_empty() && self.current.is_empty()
	}

	/// Iterates over all currently retained events (previous frame, then
	/// current frame) without advancing a cursor.
	pub fn iter_all(&self) -> impl Iterator<Item = &E> {
		self.previous.iter().chain(self.current.iter())
	}

	/// Iterates over events sent in the current frame only.
	#[inline]
	pub fn iter_current(&self) -> std::slice::Iter<'_, E> {
		self.current.iter()
	}

	/// Next event sequence number that will be assigned.
	#[inline]
	pub fn next_id(&self) -> u64 {
		self.current_start + self.current.len() as u64
	}
}

/// Tracks which events of type `E` a reader has already seen.
#[derive(Debug)]
pub struct EventCursor<E: 'static> {
	last_read: u64,
	marker: PhantomData<fn() -> E>,
}

impl<E: 'static> Default for EventCursor<E> {
	fn default() -> Self {
		Self::new()
	}
}

impl<E: 'static> Clone for EventCursor<E> {
	fn clone(&self) -> Self {
		Self {
			last_read: self.last_read,
			marker: PhantomData,
		}
	}
}

impl<E: 'static> EventCursor<E> {
	/// Starts reading from the oldest retained event.
	pub const fn new() -> Self {
		Self {
			last_read: 0,
			marker: PhantomData,
		}
	}

	/// Iterates over every unread event in `events` and advances this cursor
	/// past them.
	pub fn read<'a>(&mut self, events: &'a Events<E>) -> impl Iterator<Item = &'a E> {
		let prev_skip = self
			.last_read
			.saturating_sub(events.previous_start)
			.min(events.previous.len() as u64) as usize;
		let curr_skip = self
			.last_read
			.saturating_sub(events.current_start)
			.min(events.current.len() as u64) as usize;
		self.last_read = events.next_id();
		events.previous[prev_skip..]
			.iter()
			.chain(events.current[curr_skip..].iter())
	}

	/// Number of unread events currently available in `events`.
	#[inline]
	pub fn unread_len(&self, events: &Events<E>) -> usize {
		let oldest = events.previous_start;
		let cursor = self.last_read.max(oldest);
		events.next_id().saturating_sub(cursor) as usize
	}
}

/// Type-erased wrapper so [`World`] can rotate every registered event channel
/// in one pass.
pub(crate) trait AnyEventChannel: 'static {
	fn as_any(&self) -> &dyn Any;
	fn as_any_mut(&mut self) -> &mut dyn Any;
	fn update(&mut self);
}

impl<E: 'static> AnyEventChannel for Events<E> {
	#[inline]
	fn as_any(&self) -> &dyn Any {
		self
	}

	#[inline]
	fn as_any_mut(&mut self) -> &mut dyn Any {
		self
	}

	#[inline]
	fn update(&mut self) {
		Self::update(self);
	}
}

/// Registry of double-buffered event channels owned by a [`World`].
#[derive(Default)]
pub(crate) struct EventRegistry {
	channels: FastMap<TypeId, Box<dyn AnyEventChannel>>,
}

impl EventRegistry {
	/// Ensures an event channel for `E` exists and returns it mutably.
	pub fn get_or_insert_mut<E: 'static>(&mut self) -> &mut Events<E> {
		let entry = self
			.channels
			.entry(TypeId::of::<E>())
			.or_insert_with(|| Box::new(Events::<E>::new()));
		entry
			.as_any_mut()
			.downcast_mut::<Events<E>>()
			.expect("event channel type mismatch")
	}

	/// Borrows an already-registered event channel for `E`.
	pub fn get<E: 'static>(&self) -> Option<&Events<E>> {
		self.channels
			.get(&TypeId::of::<E>())?
			.as_any()
			.downcast_ref::<Events<E>>()
	}

	/// Rotates all registered event channels once per frame.
	pub fn update_all(&mut self) {
		for channel in self.channels.values_mut() {
			channel.update();
		}
	}
}

/// Identifier returned when registering an observer, allowing unsubscription.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ObserverId(pub u64);

/// Payload passed to an observer callback: the triggered signal plus the
/// optional target [`Entity`].
#[derive(Debug, Clone, Copy)]
pub struct Trigger<'a, E> {
	/// The signal payload.
	pub event: &'a E,
	/// Target entity when triggered with [`World::trigger_for`], or `None` for
	/// global signals.
	pub target: Option<Entity>,
}

type ErasedObserver = Box<dyn FnMut(&dyn Any, Option<Entity>, &mut World)>;

pub(crate) struct ObserverEntry {
	id: ObserverId,
	target: Option<Entity>,
	callback: ErasedObserver,
}

/// Registry of synchronous Godot-style signal observers.
#[derive(Default)]
pub(crate) struct ObserverRegistry {
	next_id: u64,
	by_type: FastMap<TypeId, Vec<ObserverEntry>>,
}

impl ObserverRegistry {
	/// Registers a callback for concrete `TypeId`.
	pub fn register<E: 'static>(
		&mut self,
		target: Option<Entity>,
		mut callback: impl FnMut(Trigger<'_, E>, &mut World) + 'static,
	) -> ObserverId {
		self.next_id += 1;
		let id = ObserverId(self.next_id);
		let entry = ObserverEntry {
			id,
			target,
			callback: Box::new(move |raw, trigger_target, world| {
				if let Some(event) = raw.downcast_ref::<E>() {
					callback(
						Trigger {
							event,
							target: trigger_target,
						},
						world,
					);
				}
			}),
		};
		self.by_type
			.entry(TypeId::of::<E>())
			.or_default()
			.push(entry);
		id
	}

	/// Removes an observer by [`ObserverId`]. Returns `true` if found.
	pub fn remove(&mut self, id: ObserverId) -> bool {
		for list in self.by_type.values_mut() {
			if let Some(pos) = list.iter().position(|entry| entry.id == id) {
				let _ = list.swap_remove(pos);
				return true;
			}
		}
		false
	}

	/// Removes all entity-scoped observers bound to `entity` (called on
	/// despawn so dead entities never leak listeners).
	pub fn remove_entity(&mut self, entity: Entity) {
		for list in self.by_type.values_mut() {
			list.retain(|entry| entry.target != Some(entity));
		}
	}

	/// Temporarily takes out the observer list for `E` so observers can freely
	/// mutate `World` (including triggering nested signals or registering new
	/// observers) without borrow conflicts.
	pub fn take_entries<E: 'static>(&mut self) -> Vec<ObserverEntry> {
		self.by_type.remove(&TypeId::of::<E>()).unwrap_or_default()
	}

	/// Restores entries after dispatch, appending any newly registered
	/// observers that arrived during dispatch.
	pub fn restore_entries<E: 'static>(&mut self, mut entries: Vec<ObserverEntry>) {
		if let Some(added) = self.by_type.remove(&TypeId::of::<E>()) {
			entries.extend(added);
		}
		if !entries.is_empty() {
			self.by_type.insert(TypeId::of::<E>(), entries);
		}
	}
}

/// Dispatches `event` to all matching observers in `entries`.
pub(crate) fn run_observer_entries<E: 'static>(
	entries: &mut [ObserverEntry],
	event: &E,
	target: Option<Entity>,
	world: &mut World,
) {
	for entry in entries {
		let matches = match (entry.target, target) {
			(None, _) => true,
			(Some(bound), Some(actual)) => bound == actual,
			(Some(_), None) => false,
		};
		if matches {
			(entry.callback)(event, target, world);
		}
	}
}
