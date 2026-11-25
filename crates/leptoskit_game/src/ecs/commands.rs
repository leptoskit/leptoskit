//! Deferred structural mutation commands (Bevy `Commands` / Godot
//! `call_deferred`).
//!
//! Queue spawns, despawns, component insertions/removals, and custom world
//! closures while iterating queries, then flush them cleanly at a sync point
//! with [`Commands::apply`].

use crate::ecs::bundle::Bundle;
use crate::ecs::component::Component;
use crate::ecs::entity::Entity;
use crate::world::World;

type CommandFn = Box<dyn FnOnce(&mut World) + Send>;

/// A deferred queue of structural [`World`] mutations.
#[derive(Default)]
pub struct Commands {
	queue: Vec<CommandFn>,
}

impl Commands {
	/// An empty command buffer.
	pub fn new() -> Self {
		Self::default()
	}

	/// Queues spawning an entity with `bundle`.
	pub fn spawn<B: Bundle>(&mut self, bundle: B) {
		self.queue.push(Box::new(move |world| {
			let _ = world.spawn(bundle);
		}));
	}

	/// Queues spawning a named entity with `bundle`.
	pub fn spawn_named<B: Bundle>(&mut self, name: impl Into<String>, bundle: B) {
		let name = name.into();
		self.queue.push(Box::new(move |world| {
			let _ = world.spawn_named(name, bundle);
		}));
	}

	/// Queues despawning `entity` (ignored if already despawned).
	pub fn despawn(&mut self, entity: Entity) {
		self.queue.push(Box::new(move |world| {
			let _ = world.despawn(entity);
		}));
	}

	/// Queues inserting or replacing `component` on `entity`.
	pub fn insert<T: Component>(&mut self, entity: Entity, component: T) {
		self.queue.push(Box::new(move |world| {
			let _ = world.insert_component(entity, component);
		}));
	}

	/// Queues removing component `T` from `entity`.
	pub fn remove<T: Component>(&mut self, entity: Entity) {
		self.queue.push(Box::new(move |world| {
			let _ = world.remove_component::<T>(entity);
		}));
	}

	/// Queues adding `entity` to a Godot-style named group.
	pub fn add_to_group(&mut self, entity: Entity, group: impl Into<String>) {
		let group = group.into();
		self.queue.push(Box::new(move |world| {
			world.add_to_group(entity, group);
		}));
	}

	/// Queues an arbitrary world mutation.
	pub fn push(&mut self, command: impl FnOnce(&mut World) + Send + 'static) {
		self.queue.push(Box::new(command));
	}

	/// Number of queued commands waiting to apply.
	#[inline]
	pub fn len(&self) -> usize {
		self.queue.len()
	}

	/// Whether no commands are queued.
	#[inline]
	pub fn is_empty(&self) -> bool {
		self.queue.is_empty()
	}

	/// Executes every queued command in submission order and clears the buffer.
	pub fn apply(&mut self, world: &mut World) {
		for command in self.queue.drain(..) {
			command(world);
		}
	}
}
