//! Compiled guest acceptance for the native analytics client: started,
//! recording, and flushing in the interpreter, with the host playing the
//! HTTP and filesystem plugins
//! (`crates/leptoskit_wasm/tests/analytics_guest.rs`).
#![cfg(target_arch = "wasm32")]
#![allow(unsafe_code)] // Exported C names form the interpreter's entry points.

use std::cell::Cell;
use std::cell::RefCell;

use futures::executor::LocalPool;
use futures::executor::LocalSpawner;
use futures::task::LocalSpawnExt as _;
use leptoskit_analytics::client::native;
use leptoskit_analytics::props;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::guest::Client;

/// The plugin the test registers to hear what the guest reports.
const REPORT: &str = "analytics-test-report";

thread_local! {
	static POOL: RefCell<LocalPool> = RefCell::new(LocalPool::new());
	static SPAWNER: LocalSpawner = POOL.with_borrow(LocalPool::spawner);
	static DONE: Cell<bool> = const { Cell::new(false) };
}

fn report(method: u32, message: &[u8]) {
	if let Ok(client) = Client::new(REPORT)
		&& let Ok(call) = client.call(MethodId(method), message)
	{
		std::mem::forget(call);
	}
}

#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_main() {
	std::panic::set_hook(Box::new(|info| report(1, info.to_string().as_bytes())));
	any_spawner::Executor::init_local_custom_executor(Executor).expect("install executor");
	SPAWNER.with(|spawner| {
		spawner
			.spawn_local(async {
				let mut options = native::Options::new("https://app.test");
				// Only the explicit flush sends, so the test sees one request
				// per run.
				options.interval = std::time::Duration::from_secs(3_600);
				options.flush_at = 1_000;
				native::init(options).await;
				report(0, native::distinct_id().unwrap_or_default().as_bytes());
				native::capture("opened", props! { "screen" => "home" });
				native::flush().await;
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
	leptoskit_poll();
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
			.expect("spawn task");
	}

	fn poll_local(&self) {}
}
