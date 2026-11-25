//! The game [`World`]: archetype ECS storage, O(1) resources, events,
//! Godot-style signals/names/groups, and named cube props.

use std::any::Any;
use std::any::TypeId;
use std::collections::BTreeMap;

use crate::ecs::Bundle;
use crate::ecs::Commands;
use crate::ecs::Component;
use crate::ecs::EcsStore;
use crate::ecs::Entity;
use crate::ecs::EventId;
use crate::ecs::Events;
use crate::ecs::ObserverId;
use crate::ecs::QueryFilter;
use crate::ecs::QueryPlan;
use crate::ecs::ReadQuery;
use crate::ecs::Trigger;
use crate::ecs::WriteQuery;
use crate::ecs::events::EventRegistry;
use crate::ecs::events::ObserverRegistry;
use crate::ecs::events::run_observer_entries;
use crate::ecs::hash::FastMap;

/// Steps per second. [`World::dt`] is `1 / TICK_HZ` and does not follow the
/// display.
pub const TICK_HZ: u32 = 60;

/// Where a body sits, in world units. Y is up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
	/// The body's origin.
	pub position: [f32; 3],
}

impl Default for Transform {
	fn default() -> Self {
		Self {
			position: [0.0, 0.0, 0.0],
		}
	}
}

/// A box the host draws. Scale is the full size on each axis, in world units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mesh {
	/// Full width, height, and depth.
	pub scale: [f32; 3],
	/// Straight sRGB, 8 bits per channel, alpha last.
	pub color: [u8; 4],
}

impl Default for Mesh {
	fn default() -> Self {
		Self {
			scale: [1.0, 1.0, 1.0],
			color: [255, 255, 255, 255],
		}
	}
}

/// One saved value a menu can read by name.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
	/// A number.
	Number(f64),
	/// Text.
	Text(String),
	/// A boolean.
	Flag(bool),
}

impl From<f64> for Value {
	fn from(value: f64) -> Self {
		Self::Number(value)
	}
}

impl From<f32> for Value {
	fn from(value: f32) -> Self {
		Self::Number(f64::from(value))
	}
}

impl From<bool> for Value {
	fn from(value: bool) -> Self {
		Self::Flag(value)
	}
}

impl From<String> for Value {
	fn from(value: String) -> Self {
		Self::Text(value)
	}
}

impl From<&str> for Value {
	fn from(value: &str) -> Self {
		Self::Text(value.to_owned())
	}
}

/// The saved world. Gameplay mutates it through ECS queries, systems, or
/// ergonomic named helpers; the host reads it to draw.
#[derive(Default)]
pub struct World {
	ecs: EcsStore,
	resources: FastMap<TypeId, Box<dyn Any>>,
	events: EventRegistry,
	observers: ObserverRegistry,
	commands: Commands,
	published: BTreeMap<String, Value>,
	tick: u64,
}

impl World {
	/// An empty world at tick 0.
	pub fn new() -> Self {
		Self::default()
	}

	/// Completed ticks. The tick currently running still reports the previous
	/// count.
	#[inline]
	pub fn tick(&self) -> u64 {
		self.tick
	}

	/// The fixed step, in seconds.
	#[inline]
	pub fn dt(&self) -> f32 {
		1.0 / TICK_HZ as f32
	}

	pub(crate) fn advance(&mut self) {
		self.tick += 1;
	}

	/// Current ECS change-detection tick.
	#[inline]
	pub fn change_tick(&self) -> u32 {
		self.ecs.change_tick
	}

	/// Advances the ECS change-detection tick counter.
	#[inline]
	pub fn increment_change_tick(&mut self) -> u32 {
		self.ecs.bump_tick()
	}

	// -------------------------------------------------------------------------
	// High-performance Archetype ECS API
	// -------------------------------------------------------------------------

	/// Spawns a new entity with `bundle` and returns its generational
	/// [`Entity`] handle.
	#[inline]
	pub fn spawn<B: Bundle>(&mut self, bundle: B) -> Entity {
		self.ecs.spawn(bundle)
	}

	/// Spawns an entity with `bundle` and assigns it a unique Godot-style
	/// `name` for O(1) lookup via [`World::named`].
	pub fn spawn_named<B: Bundle>(&mut self, name: impl Into<String>, bundle: B) -> Entity {
		let entity = self.ecs.spawn(bundle);
		self.ecs.set_name(entity, name);
		entity
	}

	/// Spawns an iterator of identical bundles in bulk with a single archetype
	/// lookup and single column reservation.
	pub fn spawn_batch<B: Bundle>(&mut self, iter: impl IntoIterator<Item = B>) -> Vec<Entity> {
		self.ecs.spawn_batch(iter)
	}

	/// Bulk-spawns an iterator of identical bundles without allocating a
	/// returned `Vec<Entity>`, returning the count spawned.
	pub fn spawn_batch_discard<B: Bundle>(&mut self, iter: impl IntoIterator<Item = B>) -> usize {
		self.ecs.spawn_batch_discard(iter)
	}

	/// Despawns `entity`, removing its components, name, group memberships,
	/// and entity-scoped signal observers.
	pub fn despawn(&mut self, entity: Entity) -> bool {
		if self.ecs.despawn(entity) {
			self.observers.remove_entity(entity);
			true
		} else {
			false
		}
	}

	/// Returns whether `entity` is currently alive.
	#[inline]
	pub fn is_alive(&self, entity: Entity) -> bool {
		self.ecs.entities.is_alive(entity)
	}

	/// Number of live entities in this world.
	#[inline]
	pub fn entity_count(&self) -> usize {
		self.ecs.entities.len()
	}

	/// Number of distinct component-signature archetypes in this world.
	#[inline]
	pub fn archetype_count(&self) -> usize {
		self.ecs.archetype_count()
	}

	/// Number of registered component types in this world.
	#[inline]
	pub fn component_type_count(&self) -> usize {
		self.ecs.component_type_count()
	}

	/// Inserts or replaces component `T` on `entity` (returns `false` if
	/// `entity` is not alive).
	#[inline]
	pub fn insert_component<T: Component>(&mut self, entity: Entity, component: T) -> bool {
		self.ecs.insert_component(entity, component)
	}

	/// Removes component `T` from `entity`, returning it if present.
	#[inline]
	pub fn remove_component<T: Component>(&mut self, entity: Entity) -> Option<T> {
		self.ecs.remove_component(entity)
	}

	/// Borrows component `T` on `entity` in O(1).
	#[inline]
	pub fn get_component<T: Component>(&self, entity: Entity) -> Option<&T> {
		self.ecs.get_component(entity)
	}

	/// Mutably borrows component `T` on `entity` in O(1), marking it changed.
	#[inline]
	pub fn get_component_mut<T: Component>(&mut self, entity: Entity) -> Option<&mut T> {
		self.ecs.get_component_mut(entity)
	}

	/// Returns whether `entity` currently holds component `T`.
	#[inline]
	pub fn has_component<T: Component>(&self, entity: Entity) -> bool {
		self.ecs.has_component::<T>(entity)
	}

	// -------------------------------------------------------------------------
	// Godot-style Names & Groups
	// -------------------------------------------------------------------------

	/// Assigns or replaces the unique name for `entity`.
	pub fn set_name(&mut self, entity: Entity, name: impl Into<String>) -> bool {
		self.ecs.set_name(entity, name)
	}

	/// Looks up an entity by its unique name in O(1).
	#[inline]
	pub fn named(&self, name: &str) -> Option<Entity> {
		self.ecs.named(name)
	}

	/// Returns the unique name assigned to `entity`, if any.
	#[inline]
	pub fn name_of(&self, entity: Entity) -> Option<&str> {
		self.ecs.name_of(entity)
	}

	/// Adds `entity` to a Godot-style named group (such as `"enemies"`).
	pub fn add_to_group(&mut self, entity: Entity, group: impl Into<String>) {
		self.ecs.add_to_group(entity, group);
	}

	/// Removes `entity` from `group`.
	pub fn remove_from_group(&mut self, entity: Entity, group: &str) {
		self.ecs.remove_from_group(entity, group);
	}

	/// Returns all live entities in `group`.
	#[inline]
	pub fn group(&self, group: &str) -> &[Entity] {
		self.ecs.group(group)
	}

	// -------------------------------------------------------------------------
	// Queries & SIMD-friendly Chunk Iteration
	// -------------------------------------------------------------------------

	/// Visits every entity matching read-only query `Q`.
	#[inline]
	pub fn for_each_read<Q: ReadQuery>(&self, f: impl FnMut(Q::Item<'_>)) {
		self.ecs.for_each_read::<Q, ()>(0, f);
	}

	/// Visits every entity matching read-only query `Q` and filter `F` since
	/// `since_tick`.
	#[inline]
	pub fn for_each_read_filtered<Q: ReadQuery, F: QueryFilter>(
		&self,
		since_tick: u32,
		f: impl FnMut(Q::Item<'_>),
	) {
		self.ecs.for_each_read::<Q, F>(since_tick, f);
	}

	/// Visits every entity matching mutable query `Q`.
	#[inline]
	pub fn for_each<Q: WriteQuery>(&mut self, f: impl FnMut(Q::Item<'_>)) {
		self.ecs.for_each_write::<Q, ()>(0, f);
	}

	/// Visits every entity matching mutable query `Q` and filter `F` since
	/// `since_tick`.
	#[inline]
	pub fn for_each_filtered<Q: WriteQuery, F: QueryFilter>(
		&mut self,
		since_tick: u32,
		f: impl FnMut(Q::Item<'_>),
	) {
		self.ecs.for_each_write::<Q, F>(since_tick, f);
	}

	/// Executes a cached [`QueryPlan`] over read-only query `Q` and filter `F`.
	#[inline]
	pub fn for_each_read_cached<Q: ReadQuery, F: QueryFilter>(
		&self,
		plan: &mut QueryPlan,
		since_tick: u32,
		f: impl FnMut(Q::Item<'_>),
	) {
		self.ecs.for_each_read_cached::<Q, F>(plan, since_tick, f);
	}

	/// Executes a cached [`QueryPlan`] over mutable query `Q` and filter `F`.
	#[inline]
	pub fn for_each_cached<Q: WriteQuery, F: QueryFilter>(
		&mut self,
		plan: &mut QueryPlan,
		since_tick: u32,
		f: impl FnMut(Q::Item<'_>),
	) {
		self.ecs.for_each_write_cached::<Q, F>(plan, since_tick, f);
	}

	/// Contiguous slice chunk iteration for `(&mut A, &B)`, exposing raw
	/// slices per archetype for full LLVM auto-vectorization.
	#[inline]
	pub fn for_each_chunk_2<A: Component, B: Component>(&mut self, f: impl FnMut(&mut [A], &[B])) {
		self.ecs.for_each_chunk_2::<A, B>(f);
	}

	/// Parallel multi-core contiguous slice chunk iteration for `(&mut A, &B)`
	/// (powered by `rayon` on native targets, deterministic sequential on
	/// `wasm32`).
	#[inline]
	pub fn par_for_each_chunk_2<A: Component, B: Component>(
		&mut self,
		f: impl Fn(&mut [A], &[B]) + Send + Sync,
	) {
		self.ecs.par_for_each_chunk_2::<A, B>(f);
	}

	// -------------------------------------------------------------------------
	// Deferred Commands
	// -------------------------------------------------------------------------

	/// Borrows the world's deferred [`Commands`] buffer.
	#[inline]
	pub fn commands(&mut self) -> &mut Commands {
		&mut self.commands
	}

	/// Flushes all queued deferred commands into `self`.
	pub fn flush_commands(&mut self) {
		if self.commands.is_empty() {
			return;
		}
		let mut commands = std::mem::take(&mut self.commands);
		commands.apply(self);
	}

	// -------------------------------------------------------------------------
	// Double-Buffered Events & Godot-style Signal Observers
	// -------------------------------------------------------------------------

	/// Registers a double-buffered event channel for `E`.
	pub fn register_event<E: 'static>(&mut self) {
		let _ = self.events.get_or_insert_mut::<E>();
	}

	/// Sends an event of type `E` into its double-buffered channel.
	#[inline]
	pub fn send_event<E: 'static>(&mut self, event: E) -> EventId {
		self.events.get_or_insert_mut::<E>().send(event)
	}

	/// Borrows the double-buffered [`Events<E>`] channel, if any event of type
	/// `E` has been registered or sent.
	#[inline]
	pub fn events<E: 'static>(&self) -> Option<&Events<E>> {
		self.events.get::<E>()
	}

	/// Rotates every registered event buffer once per frame.
	pub fn update_events(&mut self) {
		self.events.update_all();
	}

	/// Registers a global Godot-style signal observer for signal type `E`.
	pub fn observe<E: 'static>(
		&mut self,
		callback: impl FnMut(Trigger<'_, E>, &mut Self) + 'static,
	) -> ObserverId {
		self.observers.register::<E>(None, callback)
	}

	/// Registers a signal observer scoped to `entity`. Automatically cleaned up
	/// if `entity` is despawned.
	pub fn observe_entity<E: 'static>(
		&mut self,
		entity: Entity,
		callback: impl FnMut(Trigger<'_, E>, &mut Self) + 'static,
	) -> ObserverId {
		self.observers.register::<E>(Some(entity), callback)
	}

	/// Unregisters an observer by [`ObserverId`].
	pub fn remove_observer(&mut self, id: ObserverId) -> bool {
		self.observers.remove(id)
	}

	/// Synchronously triggers a global signal `event`, invoking all global
	/// observers of `E`.
	#[allow(clippy::needless_pass_by_value)]
	pub fn trigger<E: 'static>(&mut self, event: E) {
		let mut entries = self.observers.take_entries::<E>();
		if entries.is_empty() {
			return;
		}
		run_observer_entries(&mut entries, &event, None, self);
		self.observers.restore_entries::<E>(entries);
	}

	/// Synchronously triggers signal `event` targeting `entity`, invoking both
	/// global observers and observers bound to `entity`.
	#[allow(clippy::needless_pass_by_value)]
	pub fn trigger_for<E: 'static>(&mut self, entity: Entity, event: E) {
		let mut entries = self.observers.take_entries::<E>();
		if entries.is_empty() {
			return;
		}
		run_observer_entries(&mut entries, &event, Some(entity), self);
		self.observers.restore_entries::<E>(entries);
	}

	// -------------------------------------------------------------------------
	// Legacy & Quick-Start Ergonomic API (100% backwards-compatible)
	// -------------------------------------------------------------------------

	/// Spawns a unit white cube, or returns the one already named `name`.
	pub fn cube(&mut self, name: impl Into<String>) -> Prop<'_> {
		let name = name.into();
		let entity = if let Some(existing) = self.ecs.named(&name) {
			if !self.ecs.has_component::<Transform>(existing) {
				self.ecs.insert_component(existing, Transform::default());
			}
			if !self.ecs.has_component::<Mesh>(existing) {
				self.ecs.insert_component(existing, Mesh::default());
			}
			existing
		} else {
			let entity = self.ecs.spawn((Transform::default(), Mesh::default()));
			self.ecs.set_name(entity, name);
			entity
		};
		Prop {
			world: self,
			entity,
		}
	}

	/// The position of a named body.
	pub fn position(&self, name: &str) -> Option<[f32; 3]> {
		let entity = self.ecs.named(name)?;
		self.ecs
			.get_component::<Transform>(entity)
			.map(|transform| transform.position)
	}

	/// A mutable position. `None` when `name` was never spawned.
	pub fn position_mut(&mut self, name: &str) -> Option<&mut [f32; 3]> {
		let entity = self.ecs.named(name)?;
		self.ecs
			.get_component_mut::<Transform>(entity)
			.map(|transform| &mut transform.position)
	}

	/// Stores `value` under `name`, replacing a previous publication.
	pub fn publish(&mut self, name: impl Into<String>, value: impl Into<Value>) {
		self.published.insert(name.into(), value.into());
	}

	/// The publication named `name`, when it is a number.
	pub fn number(&self, name: &str) -> Option<f64> {
		match self.published.get(name) {
			Some(Value::Number(value)) => Some(*value),
			_ => None,
		}
	}

	/// The publication named `name`, when it is text.
	pub fn text(&self, name: &str) -> Option<&str> {
		match self.published.get(name) {
			Some(Value::Text(value)) => Some(value),
			_ => None,
		}
	}

	/// The publication named `name`, when it is a flag.
	pub fn flag(&self, name: &str) -> Option<bool> {
		match self.published.get(name) {
			Some(Value::Flag(value)) => Some(*value),
			_ => None,
		}
	}

	/// Every number publication, in name order.
	pub fn numbers(&self) -> Vec<(String, f64)> {
		self.published
			.iter()
			.filter_map(|(name, value)| {
				match value {
					Value::Number(number) => Some((name.clone(), *number)),
					Value::Text(_) | Value::Flag(_) => None,
				}
			})
			.collect()
	}

	/// Inserts or replaces a resource in O(1). Gameplay state that is not an
	/// entity component lives here.
	pub fn insert<T: 'static>(&mut self, value: T) {
		self.resources.insert(TypeId::of::<T>(), Box::new(value));
	}

	/// The resource of type `T` in O(1).
	#[inline]
	pub fn get<T: 'static>(&self) -> Option<&T> {
		self.resources.get(&TypeId::of::<T>())?.downcast_ref()
	}

	/// A mutable resource of type `T` in O(1).
	#[inline]
	pub fn get_mut<T: 'static>(&mut self) -> Option<&mut T> {
		self.resources.get_mut(&TypeId::of::<T>())?.downcast_mut()
	}

	/// Returns a mutable reference to resource `T`, inserting `T::default()`
	/// if absent.
	pub fn get_or_insert_default<T: Default + 'static>(&mut self) -> &mut T {
		self.resources
			.entry(TypeId::of::<T>())
			.or_insert_with(|| Box::new(T::default()))
			.downcast_mut()
			.expect("resource type mismatch")
	}

	/// Removes and returns resource `T` if present.
	pub fn remove_resource<T: 'static>(&mut self) -> Option<T> {
		self.resources
			.remove(&TypeId::of::<T>())
			.and_then(|boxed| boxed.downcast::<T>().ok())
			.map(|boxed| *boxed)
	}

	/// Visits every legacy cube `(Transform, Mesh)` in the world.
	#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
	pub(crate) fn for_each_body(&self, mut f: impl FnMut(&Transform, &Mesh)) {
		self.for_each_read::<(&Transform, &Mesh)>(|(transform, mesh)| f(transform, mesh));
	}
}

/// A cube that was just spawned or looked up. Place it, then colour it.
pub struct Prop<'a> {
	world: &'a mut World,
	entity: Entity,
}

impl Prop<'_> {
	/// The underlying ECS [`Entity`] handle for this prop.
	#[inline]
	pub const fn entity(&self) -> Entity {
		self.entity
	}

	/// Moves the cube's origin to `(x, y, z)`.
	#[must_use]
	pub fn at(self, x: f32, y: f32, z: f32) -> Self {
		if let Some(transform) = self.world.get_component_mut::<Transform>(self.entity) {
			transform.position = [x, y, z];
		}
		self
	}

	/// Sets the full size on each axis.
	#[must_use]
	pub fn scale(self, x: f32, y: f32, z: f32) -> Self {
		if let Some(mesh) = self.world.get_component_mut::<Mesh>(self.entity) {
			mesh.scale = [x, y, z];
		}
		self
	}

	/// Sets an opaque `#rrggbb` or an `#rrggbbaa` colour.
	///
	/// # Panics
	///
	/// Panics when `hex` is not one of those forms.
	#[must_use]
	pub fn color(self, hex: &str) -> Self {
		let parsed = parse_hex(hex).unwrap_or_else(|| panic!("invalid colour `{hex}`"));
		if let Some(mesh) = self.world.get_component_mut::<Mesh>(self.entity) {
			mesh.color = parsed;
		}
		self
	}
}

/// Parses `#rrggbb` or `#rrggbbaa` into straight RGBA bytes.
pub fn parse_hex(value: &str) -> Option<[u8; 4]> {
	let digits = value.strip_prefix('#')?;
	let packed = u32::from_str_radix(digits, 16).ok()?;
	match digits.len() {
		6 => Some([(packed >> 16) as u8, (packed >> 8) as u8, packed as u8, 255]),
		8 => {
			Some([
				(packed >> 24) as u8,
				(packed >> 16) as u8,
				(packed >> 8) as u8,
				packed as u8,
			])
		}
		_ => None,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	struct Score(u32);

	#[test]
	fn cubes_move_publish_and_store_resources() {
		let mut world = World::new();
		assert!((world.dt() - 1.0 / 60.0).abs() < f32::EPSILON);
		let _ = world
			.cube("lantern")
			.at(1.0, 2.0, 3.0)
			.scale(2.0, 1.0, 1.0)
			.color("#e7b15a");
		let _ = world.cube("lantern").at(4.0, 2.0, 3.0);
		assert_eq!(world.position("lantern"), Some([4.0, 2.0, 3.0]));
		let mut count = 0;
		world.for_each_body(|_, _| count += 1);
		assert_eq!(count, 1);
		world.position_mut("lantern").unwrap()[0] = 5.0;
		assert_eq!(world.position("missing"), None);
		assert!(world.position_mut("missing").is_none());

		world.publish("x", 5.0_f32);
		world.publish("label", "hi");
		world.publish("paused", true);
		world.publish("x", 6.0_f64);
		assert_eq!(world.number("x"), Some(6.0));
		assert_eq!(world.text("label"), Some("hi"));
		assert_eq!(world.flag("paused"), Some(true));
		assert_eq!(world.number("label"), None);
		assert_eq!(world.text("x"), None);
		assert_eq!(world.flag("x"), None);
		assert_eq!(world.numbers(), vec![("x".to_owned(), 6.0)]);

		world.insert(Score(1));
		world.insert(Score(2));
		assert_eq!(world.get::<Score>().map(|score| score.0), Some(2));
		world.get_mut::<Score>().unwrap().0 = 3;
		assert_eq!(world.get::<Score>().unwrap().0, 3);
		assert!(world.get::<u8>().is_none());
		world.advance();
		assert_eq!(world.tick(), 1);
	}

	#[test]
	fn hex_colours() {
		assert_eq!(parse_hex("#112233"), Some([0x11, 0x22, 0x33, 255]));
		assert_eq!(parse_hex("#11223344"), Some([0x11, 0x22, 0x33, 0x44]));
		assert_eq!(parse_hex("112233"), None);
		assert_eq!(parse_hex("#xyzxyz"), None);
		assert_eq!(parse_hex("#fff"), None);
	}

	#[test]
	#[should_panic(expected = "invalid colour")]
	fn a_bad_colour_panics() {
		let mut world = World::new();
		let _ = world.cube("a").color("nope");
	}
}
