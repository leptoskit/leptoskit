//! Compiled guest acceptance for the native runtime timer.
//!
//! The host in `crates/leptoskit_wasm/tests/timers.rs` enables the reserved
//! timer plugin, routes this guest's sleeps, and checks that they really wait.
//! The guest also abandons a one-second timer, which must reach the host as a
//! cancellation rather than a completion.
#![cfg(target_arch = "wasm32")]
#![allow(unsafe_code)] // Exported C names form the interpreter's entry points.

use std::cell::Cell;
use std::cell::RefCell;
use std::time::Duration;

use leptoskit_bridge::guest::Client;
use leptoskit_bridge::timer::SLEEP;
use leptoskit_bridge::timer::TIMER_PLUGIN;
use leptoskit_bridge::timer::encode;

mod queries;

use futures::executor::LocalPool;
use futures::executor::LocalSpawner;
use futures::task::LocalSpawnExt as _;
use reactive_graph::owner::Owner;

thread_local! {
	static POOL: RefCell<LocalPool> = RefCell::new(LocalPool::new());
	static SPAWNER: LocalSpawner = POOL.with_borrow(LocalPool::spawner);
	static OWNER: Owner = Owner::new();
	static DONE: Cell<bool> = const { Cell::new(false) };
}

#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_main() {
	std::panic::set_hook(Box::new(|info| {
		if let Ok(client) = Client::new("timer-test-panic")
			&& let Ok(call) =
				client.call(leptoskit_bridge::MethodId(0), info.to_string().as_bytes())
		{
			std::mem::forget(call);
		}
	}));
	SPAWNER.with(|_| ());
	OWNER.with(Owner::set);
	any_spawner::Executor::init_local_custom_executor(Executor).expect("install executor");
	SPAWNER.with(|spawner| {
		spawner
			.spawn_local(async {
				if !leptoskit_bridge::guest::timer_available() {
					assert_eq!(
						leptoskit_query::try_sleep(Duration::from_millis(1)).await,
						Err(leptoskit_query::SleepError::Unavailable)
					);
					assert!(leptoskit_query::try_sleep(Duration::ZERO).await.is_ok());
					queries::without_timer().await;
					DONE.with(|done| done.set(true));
					return;
				}

				// Submit a long sleep, keep it across a shorter wait, then drop
				// it while the host already runs it. The host
				// must cancel it rather than let it keep the
				// guest alive or complete later.
				let client = Client::new(TIMER_PLUGIN).expect("timer plugin");
				let payload = encode(Duration::from_secs(1)).expect("encode");
				let abandoned = client.call(SLEEP, &payload).expect("submit");

				leptoskit_query::try_sleep(Duration::from_millis(15))
					.await
					.expect("first sleep");
				drop(abandoned);
				leptoskit_query::try_sleep(Duration::from_millis(1))
					.await
					.expect("second sleep");
				queries::with_timer().await;
				DONE.with(|done| done.set(true));
			})
			.expect("spawn acceptance");
	});
	leptoskit_poll();
}

/// The host calls this after each plugin reply, through `WasmHost`.
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_poll() {
	POOL.with_borrow_mut(LocalPool::run_until_stalled);
}

#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_event(_id: i64, _kind: i32, _ptr: i32, _len: i32) -> i32 {
	DONE.with(|done| i32::from(done.get()))
}

struct Executor;

impl any_spawner::CustomExecutor for Executor {
	fn spawn(&self, task: any_spawner::PinnedFuture<()>) {
		self.spawn_local(task);
	}

	fn spawn_local(&self, task: any_spawner::PinnedLocalFuture<()>) {
		SPAWNER
			.with(|spawner| spawner.spawn_local(task))
			.expect("spawn query");
	}

	fn poll_local(&self) {}
}
