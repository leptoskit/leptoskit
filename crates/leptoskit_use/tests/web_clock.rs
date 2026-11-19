#![cfg(all(
	feature = "clock",
	feature = "web",
	target_arch = "wasm32",
	target_os = "unknown"
))]

use std::time::Duration;

use leptoskit_use::use_clock;
use leptoskit_use::use_interval;
use reactive_graph::owner::Owner;
use reactive_graph::prelude::*;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

async fn wait(ms: u64) {
	leptoskit_query::try_sleep(Duration::from_millis(ms))
		.await
		.expect("the browser has setTimeout");
}

#[wasm_bindgen_test]
async fn the_clock_runs_on_set_timeout_and_stops_with_its_owner() {
	let _ = any_spawner::Executor::init_wasm_bindgen();
	let owner = Owner::new();
	let (clock, ticks) = owner.with(|| {
		let clock = use_clock(Duration::from_millis(10));

		(clock, use_interval(clock, Duration::from_millis(20)))
	});
	assert_eq!(clock.get_untracked(), Duration::ZERO);

	wait(200).await;
	let reached = clock.get_untracked();
	assert!(reached >= Duration::from_millis(100), "{reached:?}");
	assert!(reached <= Duration::from_millis(400), "{reached:?}");
	assert!(ticks.get_untracked() >= 3, "{}", ticks.get_untracked());

	// Cleanup stops the timer. Reading a signal after its owner is gone
	// would panic, so the value is taken through `try_`.
	owner.cleanup();
	wait(60).await;
	assert_eq!(clock.try_get_untracked().unwrap_or(reached), reached);
}
