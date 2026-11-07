//! The native guest's executor for requests the query client starts itself.
//!
//! The runtime keeps its own executor for application tasks; this one exists
//! so query, mutation, and sign-in requests have somewhere to run. Only the
//! native Wasm guest builds it.

use std::cell::RefCell;

use futures::executor::LocalPool;
use futures::executor::LocalSpawner;
use futures::task::LocalSpawnExt as _;

thread_local! {
	static POOL: RefCell<LocalPool> = RefCell::new(LocalPool::new());
	static SPAWNER: LocalSpawner = POOL.with_borrow(LocalPool::spawner);
}

struct Pool;

impl any_spawner::CustomExecutor for Pool {
	fn spawn(&self, task: any_spawner::PinnedFuture<()>) {
		self.spawn_local(task);
	}

	fn spawn_local(&self, task: any_spawner::PinnedLocalFuture<()>) {
		// Only fails once the pool is gone, when nothing runs any more.
		let _ = SPAWNER.with(|spawner| spawner.spawn_local(task));
	}

	fn poll_local(&self) {}
}

/// Installs the executor; the first call wins.
pub fn start() {
	// Take the spawner now: tasks spawned while the pool runs can't borrow
	// it then.
	SPAWNER.with(|_| ());
	let _ = any_spawner::Executor::init_local_custom_executor(Pool);
}

/// Runs every task that can make progress, then settles what it wrote.
///
/// The sign-in screen and the notes list read their queries, so a result that
/// arrives rebuilds the view. A bound value, such as the counter, does not.
pub fn poll() {
	POOL.with_borrow_mut(LocalPool::run_until_stalled);
	leptoskit::guest::flush();
}
