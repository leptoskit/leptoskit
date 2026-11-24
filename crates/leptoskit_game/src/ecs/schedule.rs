//! Modular [`GameApp`], [`GamePlugin`], fixed-timestep [`Time`], finite
//! [`State<S>`] transitions, and DAG-ordered [`Schedule`] execution.
//!
//! Combines Bevy's plugin/schedule modularity with Godot's clear lifecycle
//! stages (`Startup`, `PreUpdate`, `FixedUpdate`, `Update`, `PostUpdate`,
//! `RenderSync`), deterministic fixed-step accumulation, and access-conflict
//! wave planning.

use std::any::TypeId;
use std::collections::VecDeque;
use std::hash::Hash;

use crate::ecs::hash::FastMap;
use crate::ecs::hash::FastSet;
use crate::world::TICK_HZ;
use crate::world::World;

/// Lifecycle stages executed by [`GameApp`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Stage {
	/// Runs once before the first frame.
	Startup,
	/// Runs at the start of every frame (input polling, network intake).
	PreUpdate,
	/// Runs zero or more times per frame at a deterministic fixed step
	/// (`Time::fixed_dt`). Gameplay and physics belong here.
	FixedUpdate,
	/// Runs once per rendered frame with variable `Time::delta`. Camera
	/// smoothing, visual tweens, and HUD updates belong here.
	Update,
	/// Runs after `Update` (hierarchy transform propagation, bounds updates).
	PostUpdate,
	/// Runs last before rendering extracts the frame snapshot.
	RenderSync,
}

/// Frame and fixed-step clock resource stored in [`World`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Time {
	/// Variable frame delta for the current [`Stage::Update`] pass, in seconds,
	/// scaled by [`Time::time_scale`].
	pub delta: f32,
	/// Unscaled raw frame delta clamped by [`Time::max_delta`], in seconds.
	pub unscaled_delta: f32,
	/// Deterministic fixed step length, in seconds (default `1 / 60`).
	pub fixed_dt: f32,
	/// Total scaled elapsed simulation time, in seconds.
	pub elapsed: f64,
	/// Remaining fractional time in the fixed-step accumulator, in seconds.
	pub accumulator: f32,
	/// Interpolation factor in `[0.0, 1.0)` between the previous fixed step
	/// and the current fixed step (`accumulator / fixed_dt`), for silky-smooth
	/// 120/240 Hz rendering from 60 Hz physics.
	pub alpha: f32,
	/// Time multiplier (`1.0` normal, `0.0` paused, `0.5` slow-motion).
	pub time_scale: f32,
	/// Upper bound on a single frame's delta to prevent spiral-of-death stalls.
	pub max_delta: f32,
	/// Maximum fixed steps executed in a single frame.
	pub max_fixed_steps: u32,
	/// Total rendered frames completed.
	pub frames: u64,
	/// Total fixed steps completed.
	pub fixed_ticks: u64,
}

impl Default for Time {
	fn default() -> Self {
		Self::with_fixed_hz(TICK_HZ)
	}
}

impl Time {
	/// Creates a clock with a custom fixed simulation frequency (e.g. 60 or
	/// 120 Hz).
	pub fn with_fixed_hz(hz: u32) -> Self {
		let hz = hz.max(1);
		Self {
			delta: 0.0,
			unscaled_delta: 0.0,
			fixed_dt: 1.0 / hz as f32,
			elapsed: 0.0,
			accumulator: 0.0,
			alpha: 0.0,
			time_scale: 1.0,
			max_delta: 0.1,
			max_fixed_steps: 8,
			frames: 0,
			fixed_ticks: 0,
		}
	}

	/// Advances the frame clock by `raw_dt` seconds and returns how many fixed
	/// steps should run this frame.
	pub fn advance_frame(&mut self, raw_dt: f32) -> u32 {
		let clamped = if raw_dt.is_finite() && raw_dt > 0.0 {
			raw_dt.min(self.max_delta)
		} else {
			0.0
		};
		let scale = if self.time_scale.is_finite() && self.time_scale >= 0.0 {
			self.time_scale
		} else {
			0.0
		};
		self.unscaled_delta = clamped;
		self.delta = clamped * scale;
		self.elapsed += f64::from(self.delta);
		self.accumulator += self.delta;

		let mut steps = 0_u32;
		while self.accumulator >= self.fixed_dt && steps < self.max_fixed_steps {
			self.accumulator -= self.fixed_dt;
			steps += 1;
		}
		if steps == self.max_fixed_steps && self.accumulator > self.fixed_dt {
			self.accumulator = self.fixed_dt * 0.999;
		}
		self.alpha = (self.accumulator / self.fixed_dt).clamp(0.0, 1.0);
		self.frames += 1;
		steps
	}
}

/// Active finite state machine value stored in [`World`] (e.g. `Menu`,
/// `Playing`, `Paused`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State<S: Eq + Hash + Clone + 'static> {
	current: S,
}

impl<S: Eq + Hash + Clone + 'static> State<S> {
	/// Wraps an initial state value.
	pub const fn new(initial: S) -> Self {
		Self { current: initial }
	}

	/// The active state.
	#[inline]
	pub const fn get(&self) -> &S {
		&self.current
	}
}

/// Queued state transition stored in [`World`]. Call [`NextState::set`] from
/// any system or observer to trigger `OnExit(old)` -> `OnEnter(new)` at the
/// end of the current stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NextState<S: Eq + Hash + Clone + 'static> {
	pending: Option<S>,
}

impl<S: Eq + Hash + Clone + 'static> Default for NextState<S> {
	fn default() -> Self {
		Self { pending: None }
	}
}

impl<S: Eq + Hash + Clone + 'static> NextState<S> {
	/// Requests a transition to `next` on the next transition flush.
	pub fn set(&mut self, next: S) {
		self.pending = Some(next);
	}

	/// Clears any queued transition.
	pub fn reset(&mut self) {
		self.pending = None;
	}

	/// The pending state, if any.
	#[inline]
	pub fn pending(&self) -> Option<&S> {
		self.pending.as_ref()
	}
}

/// Component and resource access signature declared by a system for conflict
/// analysis and wave grouping.
#[derive(Debug, Default, Clone)]
pub struct SystemAccess {
	reads: FastSet<TypeId>,
	writes: FastSet<TypeId>,
	exclusive: bool,
}

impl SystemAccess {
	/// Marks the system as requiring exclusive `&mut World` access.
	pub fn exclusive() -> Self {
		Self {
			reads: FastSet::default(),
			writes: FastSet::default(),
			exclusive: true,
		}
	}

	/// Declares read access to type `T`.
	#[must_use]
	pub fn read<T: 'static>(mut self) -> Self {
		self.reads.insert(TypeId::of::<T>());
		self
	}

	/// Declares write access to type `T`.
	#[must_use]
	pub fn write<T: 'static>(mut self) -> Self {
		self.writes.insert(TypeId::of::<T>());
		self
	}

	/// Returns `true` if `self` and `other` cannot execute in the same wave.
	pub fn conflicts_with(&self, other: &Self) -> bool {
		if self.exclusive || other.exclusive {
			return true;
		}
		self.writes
			.iter()
			.any(|id| other.writes.contains(id) || other.reads.contains(id))
			|| other.writes.iter().any(|id| self.reads.contains(id))
	}
}

type SystemFn = Box<dyn FnMut(&mut World)>;
type ConditionFn = Box<dyn Fn(&World) -> bool>;

/// Descriptor for a single scheduled system, supporting named ordering
/// constraints (`.before()`, `.after()`) and run conditions (`.run_if()`).
pub struct SystemConfig {
	name: String,
	func: SystemFn,
	condition: Option<ConditionFn>,
	before: Vec<String>,
	after: Vec<String>,
	access: SystemAccess,
}

impl SystemConfig {
	/// Creates a system configuration from a name and closure/function.
	pub fn new(name: impl Into<String>, func: impl FnMut(&mut World) + 'static) -> Self {
		Self {
			name: name.into(),
			func: Box::new(func),
			condition: None,
			before: Vec::new(),
			after: Vec::new(),
			access: SystemAccess::exclusive(),
		}
	}

	/// Orders this system strictly before the system named `target`.
	#[must_use]
	pub fn before(mut self, target: impl Into<String>) -> Self {
		self.before.push(target.into());
		self
	}

	/// Orders this system strictly after the system named `target`.
	#[must_use]
	pub fn after(mut self, target: impl Into<String>) -> Self {
		self.after.push(target.into());
		self
	}

	/// Runs this system only when `predicate(world)` returns `true`.
	#[must_use]
	pub fn run_if(mut self, predicate: impl Fn(&World) -> bool + 'static) -> Self {
		self.condition = Some(Box::new(predicate));
		self
	}

	/// Runs this system only while [`State<S>`] equals `expected`.
	#[must_use]
	pub fn in_state<S: Eq + Hash + Clone + 'static>(self, expected: S) -> Self {
		self.run_if(move |world| {
			world
				.get::<State<S>>()
				.is_some_and(|state| state.get() == &expected)
		})
	}

	/// Declares fine-grained component/resource access metadata for conflict
	/// wave inspection and scheduling.
	#[must_use]
	pub fn with_access(mut self, access: SystemAccess) -> Self {
		self.access = access;
		self
	}
}

/// Ordered collection of systems executed within one [`Stage`].
#[derive(Default)]
pub struct Schedule {
	systems: Vec<SystemConfig>,
	order: Vec<usize>,
	waves: Vec<Vec<usize>>,
	dirty: bool,
}

impl Schedule {
	/// Adds a system to this schedule.
	pub fn add_system(&mut self, config: SystemConfig) {
		self.systems.push(config);
		self.dirty = true;
	}

	/// Compiles the dependency graph into a deterministic topological order
	/// and conflict-free execution waves.
	///
	/// # Panics
	///
	/// Panics if `before`/`after` constraints form a cycle.
	pub fn build(&mut self) {
		if !self.dirty {
			return;
		}
		let count = self.systems.len();
		let mut by_name: FastMap<&str, usize> = FastMap::default();
		for (idx, sys) in self.systems.iter().enumerate() {
			by_name.insert(sys.name.as_str(), idx);
		}
		let mut edges: Vec<Vec<usize>> = vec![Vec::new(); count];
		let mut indegree = vec![0_usize; count];

		for (idx, sys) in self.systems.iter().enumerate() {
			for target in &sys.before {
				if let Some(&dst) = by_name.get(target.as_str())
					&& dst != idx
				{
					edges[idx].push(dst);
					indegree[dst] += 1;
				}
			}
			for dep in &sys.after {
				if let Some(&src) = by_name.get(dep.as_str())
					&& src != idx
				{
					edges[src].push(idx);
					indegree[idx] += 1;
				}
			}
		}

		let mut ready: VecDeque<usize> = indegree
			.iter()
			.enumerate()
			.filter_map(|(i, &deg)| (deg == 0).then_some(i))
			.collect();
		let mut order = Vec::with_capacity(count);
		while let Some(node) = ready.pop_front() {
			order.push(node);
			for &next in &edges[node] {
				indegree[next] -= 1;
				if indegree[next] == 0 {
					ready.push_back(next);
				}
			}
		}
		assert_eq!(
			order.len(),
			count,
			"cycle detected in game schedule system ordering constraints"
		);

		// Group into conflict-free waves while respecting topological order.
		let mut waves: Vec<Vec<usize>> = Vec::new();
		let mut system_wave = vec![0_usize; count];
		let mut parents: Vec<Vec<usize>> = vec![Vec::new(); count];
		for (u, outs) in edges.iter().enumerate() {
			for &v in outs {
				parents[v].push(u);
			}
		}
		for &idx in &order {
			let min_wave = parents[idx]
				.iter()
				.map(|&p| system_wave[p] + 1)
				.max()
				.unwrap_or(0);
			let mut placed = min_wave;
			loop {
				if placed == waves.len() {
					waves.push(vec![idx]);
					break;
				}
				let conflict = waves[placed].iter().any(|&other| {
					self.systems[idx]
						.access
						.conflicts_with(&self.systems[other].access)
				});
				if !conflict {
					waves[placed].push(idx);
					break;
				}
				placed += 1;
			}
			system_wave[idx] = placed;
		}

		self.order = order;
		self.waves = waves;
		self.dirty = false;
	}

	/// Runs all enabled systems in compiled order and flushes deferred commands
	/// after each system.
	pub fn run(&mut self, world: &mut World) {
		self.build();
		for &idx in &self.order {
			world.increment_change_tick();
			let should_run = self.systems[idx]
				.condition
				.as_ref()
				.is_none_or(|cond| cond(world));
			if should_run {
				(self.systems[idx].func)(world);
				world.flush_commands();
			}
		}
	}

	/// Number of conflict-free execution waves in the compiled plan.
	pub fn wave_count(&mut self) -> usize {
		self.build();
		self.waves.len()
	}

	/// System names in compiled execution order (useful for diagnostics and
	/// tests).
	pub fn execution_order(&mut self) -> Vec<&str> {
		self.build();
		self.order
			.iter()
			.map(|&idx| self.systems[idx].name.as_str())
			.collect()
	}
}

/// Modular feature bundle installed into a [`GameApp`] (Bevy-style plugins).
pub trait GamePlugin {
	/// Registers resources, events, states, and systems on `app`.
	fn build(&self, app: &mut GameApp);
}

type TransitionFn = Box<dyn FnMut(&mut World)>;

struct StateHook<S: Eq + Hash + Clone + 'static> {
	on_enter: FastMap<S, Vec<SystemFn>>,
	on_exit: FastMap<S, Vec<SystemFn>>,
}

impl<S: Eq + Hash + Clone + 'static> Default for StateHook<S> {
	fn default() -> Self {
		Self {
			on_enter: FastMap::default(),
			on_exit: FastMap::default(),
		}
	}
}

/// Complete modular game application combining an ECS [`World`], staged
/// [`Schedule`]s, finite state transitions, fixed-timestep accumulation, and
/// plugins.
pub struct GameApp {
	world: World,
	schedules: FastMap<Stage, Schedule>,
	state_drivers: FastMap<TypeId, TransitionFn>,
	started: bool,
}

impl Default for GameApp {
	fn default() -> Self {
		Self::new()
	}
}

impl GameApp {
	/// Creates a new app initialized with a default 60 Hz [`Time`] resource.
	pub fn new() -> Self {
		let mut world = World::new();
		world.insert(Time::default());
		Self {
			world,
			schedules: FastMap::default(),
			state_drivers: FastMap::default(),
			started: false,
		}
	}

	/// Sets the fixed-step simulation frequency in Hz (default 60).
	#[must_use]
	pub fn with_fixed_hz(mut self, hz: u32) -> Self {
		self.world.insert(Time::with_fixed_hz(hz));
		self
	}

	/// Installs a [`GamePlugin`].
	#[allow(clippy::needless_pass_by_value)]
	pub fn add_plugin(&mut self, plugin: impl GamePlugin) -> &mut Self {
		plugin.build(self);
		self
	}

	/// Inserts or replaces a resource on the app's [`World`].
	pub fn insert_resource<T: 'static>(&mut self, resource: T) -> &mut Self {
		self.world.insert(resource);
		self
	}

	/// Registers a double-buffered event type `E` so its buffers rotate
	/// automatically every frame.
	pub fn add_event<E: 'static>(&mut self) -> &mut Self {
		self.world.register_event::<E>();
		self
	}

	/// Initializes a finite state machine of type `S` with `initial`.
	pub fn init_state<S: Eq + Hash + Clone + 'static>(&mut self, initial: S) -> &mut Self {
		let type_id = TypeId::of::<S>();
		self.world.insert(State::new(initial));
		self.world.insert(NextState::<S>::default());
		self.world.get_or_insert_default::<StateHook<S>>();
		self.state_drivers.insert(type_id, make_state_driver::<S>());
		self
	}

	/// Registers a system executed whenever [`State<S>`] enters `state`.
	pub fn on_enter<S: Eq + Hash + Clone + 'static>(
		&mut self,
		state: S,
		system: impl FnMut(&mut World) + 'static,
	) -> &mut Self {
		let entry = self.world.get_or_insert_default::<StateHook<S>>();
		entry
			.on_enter
			.entry(state)
			.or_default()
			.push(Box::new(system));
		self
	}

	/// Registers a system executed whenever [`State<S>`] exits `state`.
	pub fn on_exit<S: Eq + Hash + Clone + 'static>(
		&mut self,
		state: S,
		system: impl FnMut(&mut World) + 'static,
	) -> &mut Self {
		let entry = self.world.get_or_insert_default::<StateHook<S>>();
		entry
			.on_exit
			.entry(state)
			.or_default()
			.push(Box::new(system));
		self
	}

	/// Adds a configured system to `stage`.
	pub fn add_system(&mut self, stage: Stage, config: SystemConfig) -> &mut Self {
		self.schedules.entry(stage).or_default().add_system(config);
		self
	}

	/// Convenience helper to add an unnamed system function to `stage`.
	pub fn add_fn(
		&mut self,
		stage: Stage,
		name: impl Into<String>,
		func: impl FnMut(&mut World) + 'static,
	) -> &mut Self {
		self.add_system(stage, SystemConfig::new(name, func))
	}

	/// Adds a sequence of systems chained strictly in the order listed.
	pub fn add_chain(
		&mut self,
		stage: Stage,
		systems: impl IntoIterator<Item = SystemConfig>,
	) -> &mut Self {
		let mut previous: Option<String> = None;
		for mut config in systems {
			if let Some(prev) = previous.take() {
				config = config.after(prev);
			}
			previous = Some(config.name.clone());
			self.add_system(stage, config);
		}
		self
	}

	/// Immutable reference to the underlying [`World`].
	#[inline]
	pub fn world(&self) -> &World {
		&self.world
	}

	/// Mutable reference to the underlying [`World`].
	#[inline]
	pub fn world_mut(&mut self) -> &mut World {
		&mut self.world
	}

	/// Runs [`Stage::Startup`] once if it has not yet run.
	pub fn startup(&mut self) {
		if self.started {
			return;
		}
		self.started = true;
		self.run_stage(Stage::Startup);
		self.flush_state_transitions();
	}

	/// Advances one complete frame by `raw_dt` seconds:
	/// 1. Runs [`Stage::Startup`] on the first frame.
	/// 2. Rotates double-buffered event channels.
	/// 3. Runs [`Stage::PreUpdate`].
	/// 4. Runs [`Stage::FixedUpdate`] `0..N` times according to [`Time`].
	/// 5. Runs [`Stage::Update`], [`Stage::PostUpdate`], [`Stage::RenderSync`].
	/// 6. Flushes any queued [`NextState`] transitions.
	pub fn update(&mut self, raw_dt: f32) {
		if !self.started {
			self.startup();
		}
		self.world.update_events();
		let fixed_steps = self
			.world
			.get_mut::<Time>()
			.map_or(1, |time| time.advance_frame(raw_dt));

		self.run_stage(Stage::PreUpdate);
		self.flush_state_transitions();

		for _ in 0..fixed_steps {
			self.run_stage(Stage::FixedUpdate);
			self.world.advance();
			if let Some(time) = self.world.get_mut::<Time>() {
				time.fixed_ticks += 1;
			}
			self.flush_state_transitions();
		}

		self.run_stage(Stage::Update);
		self.flush_state_transitions();
		self.run_stage(Stage::PostUpdate);
		self.run_stage(Stage::RenderSync);
	}

	/// Runs exactly one fixed step (`raw_dt = fixed_dt`), ideal for
	/// deterministic headless tests and simulations.
	pub fn step_fixed(&mut self) {
		let fixed_dt = self
			.world
			.get::<Time>()
			.map_or(1.0 / TICK_HZ as f32, |t| t.fixed_dt);
		self.update(fixed_dt);
	}

	/// Returns the schedule for `stage`, if any systems were added.
	pub fn schedule_mut(&mut self, stage: Stage) -> Option<&mut Schedule> {
		self.schedules.get_mut(&stage)
	}

	fn run_stage(&mut self, stage: Stage) {
		if let Some(mut schedule) = self.schedules.remove(&stage) {
			schedule.run(&mut self.world);
			self.schedules.insert(stage, schedule);
		}
	}

	fn flush_state_transitions(&mut self) {
		let keys: Vec<TypeId> = self.state_drivers.keys().copied().collect();
		for key in keys {
			if let Some(mut driver) = self.state_drivers.remove(&key) {
				driver(&mut self.world);
				self.state_drivers.insert(key, driver);
			}
		}
	}
}

fn make_state_driver<S: Eq + Hash + Clone + 'static>() -> TransitionFn {
	Box::new(|world: &mut World| {
		let pending = world
			.get_mut::<NextState<S>>()
			.and_then(|next| next.pending.take());
		let Some(next) = pending else {
			return;
		};
		let prev = world.get::<State<S>>().map(|s| s.current.clone());
		if prev.as_ref() == Some(&next) {
			return;
		}
		// Run OnExit hooks for previous state.
		if let Some(prev_state) = prev {
			let mut exit_hooks = world
				.get_mut::<StateHook<S>>()
				.and_then(|hooks| hooks.on_exit.remove(&prev_state))
				.unwrap_or_default();
			for hook in &mut exit_hooks {
				hook(world);
				world.flush_commands();
			}
			if !exit_hooks.is_empty() {
				world
					.get_or_insert_default::<StateHook<S>>()
					.on_exit
					.insert(prev_state, exit_hooks);
			}
		}
		// Commit new state.
		world.insert(State::new(next.clone()));
		// Run OnEnter hooks for new state.
		let mut enter_hooks = world
			.get_mut::<StateHook<S>>()
			.and_then(|hooks| hooks.on_enter.remove(&next))
			.unwrap_or_default();
		for hook in &mut enter_hooks {
			hook(world);
			world.flush_commands();
		}
		if !enter_hooks.is_empty() {
			world
				.get_or_insert_default::<StateHook<S>>()
				.on_enter
				.insert(next, enter_hooks);
		}
	})
}
