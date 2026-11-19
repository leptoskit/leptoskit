//! `use_clock` on the platform's own timer.
#![cfg(not(target_arch = "wasm32"))]

use std::sync::Arc;
use std::time::Duration;

use leptoskit_use::use_clock;
use leptoskit_use::use_timeout_fn;
use parking_lot::Mutex;
use reactive_graph::owner::Owner;
use reactive_graph::prelude::*;

fn ms(n: u64) -> Duration {
	Duration::from_millis(n)
}

/// Waits until `done` holds, for a few seconds at most.
async fn eventually(done: impl Fn() -> bool) -> bool {
	for _ in 0..400 {
		if done() {
			return true;
		}

		tokio::time::sleep(ms(10)).await;
	}

	done()
}

#[tokio::test]
async fn the_clock_advances_drives_a_timeout_and_stops_with_its_owner() {
	let _ = any_spawner::Executor::init_tokio();
	let owner = Owner::new();
	let fired = Arc::new(Mutex::new(0));
	let clock = owner.with(|| {
		let clock = use_clock(ms(5));
		let fired = Arc::clone(&fired);
		use_timeout_fn(clock, move || *fired.lock() += 1, ms(30));

		clock
	});
	assert_eq!(clock.get_untracked(), Duration::ZERO);

	assert!(eventually(|| clock.get_untracked() >= ms(30)).await);
	assert!(eventually(|| *fired.lock() == 1).await);

	// Time is measured, so it cannot run ahead of the real clock.
	let before = clock.get_untracked();
	tokio::time::sleep(ms(40)).await;
	let after = clock.get_untracked();
	assert!(after > before, "{before:?} then {after:?}");
	assert!(
		after.saturating_sub(before) < ms(2_000),
		"{before:?} then {after:?}"
	);

	// Cleanup stops the timer: nothing touches the signal afterwards.
	drop(owner);
	tokio::time::sleep(ms(40)).await;
	assert_eq!(*fired.lock(), 1);
}

#[tokio::test]
async fn an_interval_of_zero_does_not_spin() {
	let _ = any_spawner::Executor::init_tokio();
	let owner = Owner::new();
	let clock = owner.with(|| use_clock(Duration::ZERO));

	assert!(eventually(|| clock.get_untracked() >= ms(5)).await);
	drop(owner);
}
