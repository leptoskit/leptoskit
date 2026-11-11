//! State that survives a rebuild of the screen.
//!
//! leptoskit builds a screen again whenever a signal it read changes: the
//! native runtime calls the view function again, and the browser re-runs the
//! closure that renders it. A `RwSignal::new(..)` inside the screen runs again
//! with it, so a chosen filter or a typed field would reset the moment the
//! screen redraws. [`remember`] hands back the same signal on every build
//! instead:
//!
//! ```
//! use leptoskit_native::Element;
//! use leptoskit_native::reactive::Get;
//! use leptoskit_native::reactive::RwSignal;
//! use leptoskit_native::reactive::Set;
//! use leptoskit_native::remember::ScreenState;
//! use leptoskit_native::remember::remember;
//!
//! fn filter() -> RwSignal<&'static str> {
//! 	// The first build runs the closure; every later build gets this signal
//! 	// back, holding whatever the screen wrote to it.
//! 	remember(|| "all")
//! }
//!
//! # reactive_graph::owner::Owner::new().with(|| {
//! // The renderer does this part: one state per mounted screen, one build per
//! // redraw.
//! let state = ScreenState::new();
//! state.provide();
//! state.build(filter).set("open");
//! assert_eq!(state.build(filter).get(), "open");
//! # });
//! ```
//!
//! A call is told apart by where it is in the source and by how many times
//! that line already ran in this build, so a component used twice keeps two
//! values, in the order it was used. Rows that can move, appear, or disappear
//! name their own value with [`remember_keyed`] and the row's key instead, so
//! a value follows its row rather than its position.
//!
//! A value lives as long as the screen keeps asking for it. Once a whole build
//! finishes without reaching it — the route changed, or a branch that held it
//! stopped rendering — the value is released and its signal disposed. State
//! that outlives a screen belongs in a signal created before the app mounts,
//! as before.
//!
//! The renderers supply the [`ScreenState`]: the native runtime's
//! `guest::mount` and the web's `web::screen` wrap every build in one. Outside
//! them, as in a test that renders once, there is nothing to remember into,
//! and [`remember`] returns a new signal each time.

use std::any::Any;
use std::any::TypeId;
use std::collections::HashMap;
use std::hash::Hash;
use std::panic::Location;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::PoisonError;

use reactive_graph::owner::Owner;
use reactive_graph::owner::provide_context;
use reactive_graph::owner::use_context;
use reactive_graph::signal::RwSignal;
use reactive_graph::traits::Dispose;

use crate::reactive::hash_key;

/// The remembered values of one mounted screen, and the builds that keep them.
///
/// Cloning is cheap; clones share the values.
#[derive(Clone)]
pub struct ScreenState {
	inner: Arc<Mutex<Inner>>,
}

impl std::fmt::Debug for ScreenState {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("ScreenState")
			.field("values", &self.len())
			.finish_non_exhaustive()
	}
}

struct Inner {
	/// Owns every remembered signal, so a build's own owner being cleaned up
	/// never disposes one.
	owner: Owner,
	values: HashMap<Slot, Value>,
	/// How many builds have started.
	build: u64,
	/// How many builds are running, counting nested ones.
	depth: u32,
	/// How many times each call site has run in the current build.
	seen: HashMap<(&'static Location<'static>, TypeId), u32>,
}

/// Where a value was asked for.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Slot {
	site: &'static Location<'static>,
	ty: TypeId,
	key: SlotKey,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum SlotKey {
	/// The call site's nth call in a build.
	Nth(u32),
	/// A key the screen chose.
	Keyed(u64),
}

struct Value {
	signal: Box<dyn Any + Send + Sync>,
	dispose: fn(&(dyn Any + Send + Sync)),
	/// The last build that asked for it.
	build: u64,
}

impl ScreenState {
	/// An empty state whose signals belong to an owner below the current one.
	#[must_use]
	pub fn new() -> Self {
		Self {
			inner: Arc::new(Mutex::new(Inner {
				owner: Owner::new(),
				values: HashMap::new(),
				build: 0,
				depth: 0,
				seen: HashMap::new(),
			})),
		}
	}

	/// Makes this state the one [`remember`] uses below the current owner.
	pub fn provide(&self) {
		provide_context(self.clone());
	}

	/// Runs one build of the screen, and afterwards releases every value that
	/// neither this build nor anything since the last one asked for.
	///
	/// A build inside a build is part of the outer one.
	pub fn build<T>(&self, build: impl FnOnce() -> T) -> T {
		{
			let mut inner = self.lock();

			if inner.depth == 0 {
				inner.build += 1;
				inner.seen.clear();
			}

			inner.depth += 1;
		}

		let _end = EndBuild(self);
		build()
	}

	/// How many values are remembered.
	#[must_use]
	pub fn len(&self) -> usize {
		self.lock().values.len()
	}

	/// Whether nothing is remembered.
	#[must_use]
	pub fn is_empty(&self) -> bool {
		self.len() == 0
	}

	fn lock(&self) -> MutexGuard<'_, Inner> {
		self.inner.lock().unwrap_or_else(PoisonError::into_inner)
	}

	fn signal<T>(
		&self,
		site: &'static Location<'static>,
		key: Option<u64>,
		init: impl FnOnce() -> T,
	) -> RwSignal<T>
	where
		T: Send + Sync + 'static,
	{
		let ty = TypeId::of::<T>();
		let slot = {
			let mut inner = self.lock();
			let key = key.map_or_else(
				|| {
					let seen = inner.seen.entry((site, ty)).or_insert(0);
					let nth = *seen;
					*seen += 1;
					SlotKey::Nth(nth)
				},
				SlotKey::Keyed,
			);
			let slot = Slot { site, ty, key };
			let build = inner.build;

			if let Some(value) = inner.values.get_mut(&slot)
				&& let Some(signal) = value.signal.downcast_ref::<RwSignal<T>>()
			{
				value.build = build;
				return *signal;
			}

			slot
		};

		// Not under the lock: `init` may remember values of its own.
		let initial = init();
		let mut inner = self.lock();
		let signal = inner.owner.with(|| RwSignal::new(initial));
		let build = inner.build;
		inner.values.insert(
			slot,
			Value {
				signal: Box::new(signal),
				dispose: dispose::<T>,
				build,
			},
		);
		signal
	}
}

impl Default for ScreenState {
	fn default() -> Self {
		Self::new()
	}
}

/// Ends a build when it returns or unwinds, releasing what it left behind.
struct EndBuild<'a>(&'a ScreenState);

impl Drop for EndBuild<'_> {
	fn drop(&mut self) {
		let released = {
			let mut inner = self.0.lock();
			inner.depth -= 1;

			if inner.depth > 0 {
				return;
			}

			let build = inner.build;
			let mut released = Vec::new();
			inner.values.retain(|_, value| {
				let keep = value.build == build;

				if !keep {
					released.push((
						std::mem::replace(&mut value.signal, Box::new(())),
						value.dispose,
					));
				}

				keep
			});
			released
		};

		// Disposing runs cleanups, which may reach this state again.
		for (signal, dispose) in released {
			dispose(signal.as_ref());
		}
	}
}

fn dispose<T: Send + Sync + 'static>(signal: &(dyn Any + Send + Sync)) {
	if let Some(signal) = signal.downcast_ref::<RwSignal<T>>() {
		signal.dispose();
	}
}

/// The signal this call site holds on the current screen, created from `init`
/// on the first build that reaches it.
///
/// Calls are told apart by their place in the source and by order: the
/// second time a line runs in one build it gets a value of its own. Use
/// [`remember_keyed`] for rows that can reorder. Without a [`ScreenState`] in
/// context, this is `RwSignal::new(init())`.
#[track_caller]
pub fn remember<T>(init: impl FnOnce() -> T) -> RwSignal<T>
where
	T: Send + Sync + 'static,
{
	let site = Location::caller();

	match use_context::<ScreenState>() {
		Some(state) => state.signal(site, None, init),
		None => RwSignal::new(init()),
	}
}

/// The signal this call site holds for `key` on the current screen, created
/// from `init` on the first build that reaches it.
///
/// The key, not the order of calls, names the value, so a row's state follows
/// the row when rows move. Two calls with the same key at the same place share
/// one value. Without a [`ScreenState`] in context, this is
/// `RwSignal::new(init())`.
#[track_caller]
pub fn remember_keyed<K, T>(key: &K, init: impl FnOnce() -> T) -> RwSignal<T>
where
	K: Hash + ?Sized,
	T: Send + Sync + 'static,
{
	let site = Location::caller();

	match use_context::<ScreenState>() {
		Some(state) => state.signal(site, Some(hash_key(key)), init),
		None => RwSignal::new(init()),
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::reactive::Get;
	use crate::reactive::GetUntracked;
	use crate::reactive::Set;

	fn screen() -> (Owner, ScreenState) {
		let owner = Owner::new();
		let state = owner.with(|| {
			let state = ScreenState::new();
			state.provide();
			state
		});
		(owner, state)
	}

	#[test]
	fn a_signal_created_in_the_screen_resets_on_every_build() {
		// What the screen did before `remember`: the bug this module fixes.
		let (owner, state) = screen();
		let build = || state.build(|| RwSignal::new(0));
		owner.with(|| {
			build().set(3);
			assert_eq!(build().get(), 0);
		});
	}

	#[test]
	fn a_remembered_signal_survives_a_rebuild() {
		let (owner, state) = screen();
		let build = || state.build(|| remember(|| 0));
		owner.with(|| {
			build().set(3);
			assert_eq!(build().get(), 3);
			assert_eq!(state.len(), 1);
		});
	}

	#[test]
	fn each_call_of_one_line_keeps_its_own_value_in_order() {
		let (owner, state) = screen();
		let counter = || remember(|| 0);
		let build = || state.build(|| (counter(), counter()));
		owner.with(|| {
			let (first, second) = build();
			first.set(1);
			second.set(2);
			let (first, second) = build();
			assert_eq!((first.get(), second.get()), (1, 2));
			assert_eq!(state.len(), 2);
		});
	}

	#[test]
	fn the_same_line_holds_one_value_per_type() {
		fn field<T: Default + Send + Sync + 'static>() -> RwSignal<T> {
			remember(T::default)
		}

		let (owner, state) = screen();
		owner.with(|| {
			let (number, text) = state.build(|| (field::<u8>(), field::<String>()));
			number.set(4);
			text.set("four".to_owned());
			let (number, text) = state.build(|| (field::<u8>(), field::<String>()));
			assert_eq!((number.get(), text.get()), (4, "four".to_owned()));
		});
	}

	#[test]
	fn a_keyed_value_follows_its_row() {
		let (owner, state) = screen();
		let rows = |order: &[&str]| {
			state.build(|| {
				order
					.iter()
					.map(|row| remember_keyed(row, || (*row).to_owned()))
					.collect::<Vec<_>>()
			})
		};
		owner.with(|| {
			let first = rows(&["a", "b"]);
			first[0].set("edited a".to_owned());
			let moved = rows(&["b", "a"]);
			assert_eq!(moved[1].get(), "edited a");
			assert_eq!(moved[0].get(), "b");
		});
	}

	#[test]
	fn a_value_the_screen_stops_asking_for_is_released() {
		let (owner, state) = screen();
		let draft = || remember(|| "draft".to_owned());
		owner.with(|| {
			let typed = state.build(draft);
			typed.set("typed".to_owned());
			// Another route: nothing asks for the draft.
			state.build(|| ());
			assert!(state.is_empty());
			assert!(
				typed.try_get_untracked().is_none(),
				"its signal is disposed"
			);
			// Coming back starts over.
			assert_eq!(state.build(draft).get(), "draft");
		});
	}

	#[test]
	fn values_asked_for_between_builds_last_until_the_next_build() {
		let (owner, state) = screen();
		let row = || remember_keyed(&7, || 1);
		owner.with(|| {
			state.build(|| ());
			// A list region re-running after a write, outside any build.
			row().set(2);
			assert_eq!(row().get(), 2);
			state.build(|| assert_eq!(row().get(), 2));
			state.build(|| ());
			assert!(state.is_empty());
		});
	}

	#[test]
	fn a_nested_build_belongs_to_the_outer_one() {
		let (owner, state) = screen();
		let build = || {
			state.build(|| {
				let outer = remember(|| 1);
				let inner = state.build(|| remember(|| 2));
				(outer, inner)
			})
		};
		owner.with(|| {
			let (outer, inner) = build();
			outer.set(10);
			inner.set(20);
			let (outer, inner) = build();
			assert_eq!((outer.get(), inner.get()), (10, 20));
			assert_eq!(state.len(), 2);
		});
	}

	#[test]
	fn init_may_remember_values_of_its_own() {
		let (owner, state) = screen();
		owner.with(|| {
			let outer = state.build(|| remember(|| remember(|| 5).get() + 1));
			assert_eq!(outer.get(), 6);
			assert_eq!(state.len(), 2);
		});
	}

	#[test]
	fn the_build_owner_being_cleaned_up_keeps_remembered_values() {
		let (owner, state) = screen();
		// Each build runs under an owner of its own, as a web render does.
		let build = || Owner::new().with(|| state.build(|| remember(|| 1)));
		owner.with(|| {
			build().set(2);
			assert_eq!(build().get(), 2);
		});
	}

	#[test]
	fn a_build_that_panics_still_ends() {
		let (owner, state) = screen();
		owner.with(|| {
			let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
				state.build(|| {
					let _ = remember(|| 1);
					panic!("the screen failed");
				})
			}));
			assert!(panicked.is_err());
			state.build(|| ());
			assert!(state.is_empty(), "the next build is a whole build again");
		});
	}

	#[test]
	fn without_a_screen_state_each_call_is_fresh() {
		let positional = || remember(|| 0);
		let keyed = || remember_keyed("filter", || 0);
		Owner::new().with(|| {
			positional().set(5);
			assert_eq!(positional().get(), 0);
			keyed().set(5);
			assert_eq!(keyed().get(), 0);
		});
	}

	#[test]
	fn the_state_prints_how_much_it_holds() {
		let (owner, state) = screen();
		owner.with(|| {
			state.build(|| remember(|| 0));
			assert_eq!(format!("{state:?}"), "ScreenState { values: 1, .. }");
			assert!(ScreenState::default().is_empty());
		});
	}

	#[test]
	fn disposing_another_type_does_nothing() {
		dispose::<u32>(&());
	}
}
