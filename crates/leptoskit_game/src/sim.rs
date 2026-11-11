//! The fixed step, and the actions and messages a tick reads.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::marker::PhantomData;

use crate::world::World;

// Already `const`. Android's OS thread-local backend still emits a non-const
// init function, which this lint treats as a missing const.
#[cfg_attr(target_os = "android", allow(clippy::missing_const_for_thread_local))]
thread_local! {
	#[cfg_attr(target_os = "android", expect(clippy::missing_const_for_thread_local, reason = "Android fallback TLS: rust-clippy#13422"))]
	static INBOX: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// Queues a message for the next tick.
///
/// A button handler calls this. The tick that is already running does not see
/// it; the following tick does, through [`Input::messages`].
pub fn post(message: impl Into<String>) {
	INBOX.with(|inbox| inbox.borrow_mut().push(message.into()));
}

/// Drops messages still queued. Tests use this so one case cannot leak into the
/// next.
pub fn clear_posts() {
	INBOX.with(|inbox| inbox.borrow_mut().clear());
}

fn drain_posts() -> Vec<String> {
	INBOX.with(|inbox| std::mem::take(&mut *inbox.borrow_mut()))
}

/// What a tick can read: held and newly pressed actions, and messages posted
/// since the last tick.
#[derive(Debug, Default)]
pub struct Input {
	held: BTreeSet<String>,
	pressed: BTreeSet<String>,
	messages: Vec<String>,
}

impl Input {
	/// Whether `action` is down at the end of this tick.
	pub fn held(&self, action: &str) -> bool {
		self.held.contains(action)
	}

	/// Whether `action` went down since the previous tick.
	pub fn pressed(&self, action: &str) -> bool {
		self.pressed.contains(action)
	}

	/// Messages posted since the previous tick, in arrival order.
	pub fn messages(&self) -> &[String] {
		&self.messages
	}
}

/// Gameplay. The body of `setup` and `tick` is ordinary Rust; it does not go
/// through the UI compiler.
pub trait Game {
	/// A short name printed by the run report.
	const ID: &'static str;

	/// Builds the world once.
	fn setup(world: &mut World);

	/// Advances one fixed step.
	fn tick(world: &mut World, input: &Input);
}

/// A running game. Rendering never changes it.
pub struct Sim<G: Game> {
	world: World,
	input: Input,
	game: PhantomData<G>,
}

impl<G: Game> Sim<G> {
	/// Runs [`Game::setup`].
	pub fn new() -> Self {
		let mut world = World::new();
		G::setup(&mut world);
		Self {
			world,
			input: Input::default(),
			game: PhantomData,
		}
	}

	/// The world `setup` and `tick` have built.
	pub fn world(&self) -> &World {
		&self.world
	}

	/// Holds `action` from now on. The next tick sees [`Input::pressed`] as
	/// well as [`Input::held`].
	pub fn hold(&mut self, action: impl Into<String>) {
		let action = action.into();
		if self.input.held.insert(action.clone()) {
			self.input.pressed.insert(action);
		}
	}

	/// Replaces the held set. Newly held names are also pressed for the next
	/// tick.
	pub fn set_held(&mut self, actions: &[String]) {
		let desired: BTreeSet<String> = actions.iter().cloned().collect();
		for action in &desired {
			if self.input.held.insert(action.clone()) {
				self.input.pressed.insert(action.clone());
			}
		}
		self.input.held.retain(|action| desired.contains(action));
		self.input
			.pressed
			.retain(|action| self.input.held.contains(action));
	}

	/// Releases `action` without a tick.
	pub fn release(&mut self, action: &str) {
		self.input.held.remove(action);
		self.input.pressed.remove(action);
	}

	/// One fixed step. Messages posted since the previous step arrive here.
	pub fn step(&mut self) {
		self.input.messages = drain_posts();
		G::tick(&mut self.world, &self.input);
		self.input.pressed.clear();
		self.input.messages.clear();
		self.world.advance();
	}

	/// Runs every step that fits in `milliseconds`.
	pub fn run(&mut self, milliseconds: f32) {
		let steps = (milliseconds * crate::world::TICK_HZ as f32 / 1000.0).floor();
		let steps = steps.max(0.0) as u32;
		for _ in 0..steps {
			self.step();
		}
	}
}

impl<G: Game> Default for Sim<G> {
	fn default() -> Self {
		Self::new()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	struct Drift;

	impl Game for Drift {
		const ID: &'static str = "drift";

		fn setup(world: &mut World) {
			let _ = world.cube("player").at(0.0, 0.0, 0.0);
		}

		fn tick(world: &mut World, input: &Input) {
			if input.messages().iter().any(|message| message == "restart") {
				world.position_mut("player").unwrap()[0] = 0.0;
			} else if input.held("move") {
				world.position_mut("player").unwrap()[0] += world.dt();
			}
			if input.pressed("jump") {
				let jumps = world.number("jumps").unwrap_or(0.0);
				world.publish("jumps", jumps + 1.0);
			}
		}
	}

	#[test]
	fn hold_is_an_edge_and_a_message_waits_for_the_next_tick() {
		clear_posts();
		let mut sim = Sim::<Drift>::new();
		sim.hold("move");
		sim.hold("move");
		sim.step();
		let once = sim.world().position("player").unwrap()[0];
		assert!((once - sim.world().dt()).abs() < 1e-5);
		sim.step();
		let twice = sim.world().position("player").unwrap()[0];
		assert!((twice - 2.0 * sim.world().dt()).abs() < 1e-5);

		sim.release("move");
		sim.hold("jump");
		sim.step();
		sim.step();
		assert_eq!(sim.world().number("jumps"), Some(1.0));

		post("restart");
		sim.step();
		assert_eq!(sim.world().position("player").unwrap()[0], 0.0);
		assert_eq!(sim.world().tick(), 5);

		sim.set_held(&["move".to_owned()]);
		sim.set_held(&["move".to_owned()]);
		sim.run(1000.0);
		assert!(sim.world().tick() > 5);
		sim.run(-1.0);
		clear_posts();
	}
}
