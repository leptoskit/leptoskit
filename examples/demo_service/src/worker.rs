//! The worker: the one process image that runs the control service's
//! worker for a generation.
//!
//! Ownership is a kernel lock with process lifetime. A ready candidate
//! that lost the race polls for `worker.lock` at most once a second; when
//! it wins — because the previous holder exited, and only then — it
//! writes `worker.owner`, constructs the simulated executor from the
//! generation's persisted applied state, and starts the worker, whose
//! first act is to mark operations the dead holder left running as
//! interrupted.
//!
//! The lock is held by the process's retained-lock set, not by the task:
//! neither an aborted async task nor the runtime's shutdown timeout can
//! release it while a blocking executor write might still be in flight.
//! Only exec or exit closes the descriptor, and the kernel releases the
//! flock only then — after every writer has stopped.
//!
//! If the worker task itself ends while the service is up — it returns
//! silently on storage errors — the process fails truthfully with a
//! nonzero exit rather than keep serving a generation whose operations
//! nobody runs.

#[cfg(debug_assertions)]
use std::future::Future;
use std::path::Path;

use admin_demo::executor::Simulated;
use anyhow::Context as _;
use anyhow::Result;
use leptoskit_deploy_console::control::Executor;
#[cfg(debug_assertions)]
use leptoskit_deploy_console::control::Operation;
#[cfg(debug_assertions)]
use leptoskit_deploy_console::control::Project;
#[cfg(debug_assertions)]
use leptoskit_deploy_console::control::RunResult;

use crate::generation::Generation;
use crate::locks;
use crate::shutdown::Terminated;

/// The simulated executor, wrapped in debug builds when the test hooks
/// ask for a pause, so a process test can hold an operation mid-flight.
fn executor_for(generation: &Generation) -> Result<std::sync::Arc<dyn Executor>> {
	let simulated = Simulated::persisted(
		generation.control.clone(),
		COMMITTED,
		&generation.dir.join("applied.toml"),
	)
	.context("loading the simulated executor's applied state")?;

	#[cfg(debug_assertions)]
	if let Some(paused) = paused_for_tests() {
		return Ok(std::sync::Arc::new(PauseForTests {
			started: std::env::var_os("DEMO_TEST_OP_STARTED").map(std::path::PathBuf::from),
			paused,
			inner: std::sync::Arc::new(simulated),
		}));
	}

	Ok(std::sync::Arc::new(simulated))
}

/// The demo project's `leptoskit.toml`, as last committed.
const COMMITTED: &str = include_str!("../project/leptoskit.toml");

/// Runs the worker when — and only while — this process holds the
/// generation's worker lock. Ends when the process starts to stop; a
/// worker that dies on its own fails the process with exit code 1.
pub async fn run(generation: Generation, terminated: Terminated) {
	let dir: &Path = &generation.dir;

	if let Err(error) = locks::worker_lock(dir, terminated.cancellable()).await {
		// The wait was cancelled by shutdown, or the lock file is
		// unusable. Cancelled is normal; anything else is honest
		// failure, because this image can never run operations. The
		// lock itself is retained by the process, not held here.
		if !terminated.is_terminating() {
			eprintln!("the demo's worker lock failed: {error:#}");
			std::process::exit(1);
		}
		return;
	}

	announce_worker(dir);
	let executor = match executor_for(&generation) {
		Ok(executor) => executor,
		Err(error) => {
			eprintln!("the demo's executor didn't start: {error:#}");
			std::process::exit(1);
		}
	};

	// Exclusive ownership is this lock's contract: interrupting running
	// operations is correct exactly because the previous holder is dead.
	let worker = match generation.control.worker(executor) {
		Ok(worker) => worker,
		Err(error) => {
			eprintln!("the demo's worker didn't start: {error:#}");
			std::process::exit(1);
		}
	};

	tokio::select! {
		outcome = worker => {
			if !terminated.is_terminating() {
				eprintln!(
					"the demo's worker stopped on its own ({outcome:?}); the generation's \
					 operations would not run, so this process fails"
				);
				std::process::exit(1);
			}
		}
		() = terminated.wait() => {}
	}
}

/// Tells an operator — and the process tests — who runs the worker.
fn announce_worker(dir: &Path) {
	let owner = dir.join("worker.owner");
	let pid = std::process::id().to_string();
	if let Err(error) = std::fs::write(&owner, pid.clone()) {
		eprintln!("couldn't write {}: {error}", owner.display());
	}
}

/// The pause hook's file, in a debug build: while it exists, every
/// operation waits before it runs. Compiled out of release builds.
#[cfg(debug_assertions)]
fn paused_for_tests() -> Option<std::path::PathBuf> {
	std::env::var_os("DEMO_TEST_PAUSE_OP").map(std::path::PathBuf::from)
}

/// A debug-only executor wrapper: announces that an operation started and
/// waits while the pause file exists, then delegates to the real
/// simulated executor. Never built in release, so no hook file is ever
/// consulted there.
#[cfg(debug_assertions)]
struct PauseForTests {
	started: Option<std::path::PathBuf>,
	paused: std::path::PathBuf,
	inner: std::sync::Arc<Simulated>,
}

#[cfg(debug_assertions)]
impl Executor for PauseForTests {
	fn run(
		&self,
		project: Project,
		operation: Operation,
	) -> std::pin::Pin<Box<dyn Future<Output = RunResult> + Send>> {
		if let Some(started) = &self.started {
			let _ = std::fs::write(started, operation.id.as_bytes());
		}
		let paused = self.paused.clone();
		let inner = self.inner.clone();
		Box::pin(async move {
			while paused.exists() {
				tokio::time::sleep(std::time::Duration::from_millis(20)).await;
			}
			Executor::run(&*inner, project, operation).await
		})
	}
}
