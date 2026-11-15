//! The kernel file locks the lifecycle is built on.
//!
//! Every lock here is an `flock` on a file under the state volume, held by
//! an owned [`std::fs::File`]. Ownership rules, all deliberate:
//!
//! - **`build.lock`** serializes every builder and collector on the volume. It
//!   is taken by cancellable non-blocking polling — never by a blocking wait
//!   inside a detached task — so a SIGTERM can always stop a waiter, and no
//!   abandoned waiter is left holding a queue position. It is released as soon
//!   as open/build/collect succeeds, and *kept* by a failed initializer, which
//!   must not let a second process near state its background writers may still
//!   touch.
//! - **A generation's `lifetime.lock`** is held shared by every process with
//!   that generation open, from before the first database is opened until exec
//!   or exit. Cleanup only ever deletes a generation whose lifetime lock it can
//!   take exclusively without blocking: a live holder's shared lock makes that
//!   fail, which is how clock jumps, paused old candidates, and older formats
//!   refuse to have their databases unlinked underneath them.
//! - **`worker.lock`** names the one process whose image runs the control
//!   service's worker. It is polled for at most once a second by a waiting
//!   candidate, and once held it stays held: the file lives in the process's
//!   retained-lock set, so neither an aborted task nor the runtime's shutdown
//!   timeout can drop it while a blocking executor write is still in flight.
//!
//! The retained-lock set is why "the kernel releases it at exec/exit" is
//! true: the descriptors live until the process image is replaced or
//! dies, and `O_CLOEXEC` closes them exactly then — never up to two
//! seconds earlier.

use std::fs::File;
use std::future::Future;
use std::io;
use std::os::fd::AsFd as _;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::Context as _;
use anyhow::Result;
use rustix::fs::FlockOperation;

/// How often a cancellable poll re-tries a lock.
const POLL: Duration = Duration::from_millis(200);

/// The file descriptors whose flocks must live until exec or exit. A
/// process keeps at most one build lock (a failed initializer), one
/// generation lifetime lock, and one worker lock; the vector is small and
/// bounded, and dropping from it is deliberately unsupported.
static RETAINED: Mutex<Vec<(&'static str, File)>> = Mutex::new(Vec::new());

/// Keeps `file` until the process image is replaced or dies.
fn retain(role: &'static str, file: File) {
	RETAINED
		.lock()
		.unwrap_or_else(std::sync::PoisonError::into_inner)
		.push((role, file));
}

/// Whether `shutdown` has resolved: the shared future every lock wait
/// selects against, so SIGTERM stops a waiter instead of orphaning it.
pub type Cancel = std::pin::Pin<Box<dyn Future<Output = ()> + Send>>;

fn flock(file: &File, operation: FlockOperation) -> io::Result<()> {
	rustix::fs::flock(file.as_fd(), operation).map_err(io::Error::from)
}

fn would_block(error: &io::Error) -> bool {
	error.kind() == io::ErrorKind::WouldBlock
}

fn open_lock_file(path: &Path) -> Result<File> {
	let mut options = std::fs::OpenOptions::new();
	options.create(true).truncate(false).read(true).write(true);
	#[cfg(unix)]
	{
		use std::os::unix::fs::OpenOptionsExt as _;
		options.mode(0o600);
	}
	options
		.open(path)
		.with_context(|| format!("opening {}", path.display()))
}

/// Takes the volume's build lock, polling without blocking so a SIGTERM
/// cancels the wait. The returned file is the lock: drop it to release a
/// successful build, or [`retain`] it after a failed one.
///
/// # Errors
///
/// When the file can't be opened or locked, or the wait is cancelled.
pub async fn build_lock(root: &Path, mut cancelled: Cancel) -> Result<File> {
	let file = open_lock_file(&root.join("build.lock"))?;

	loop {
		match flock(&file, FlockOperation::NonBlockingLockExclusive) {
			Ok(()) => return Ok(file),
			Err(error) if would_block(&error) => {}
			Err(error) => {
				return Err(anyhow::Error::new(error)
					.context(format!("locking {}", root.join("build.lock").display())));
			}
		}

		tokio::select! {
			() = tokio::time::sleep(POLL) => {}
			() = &mut cancelled => {
				anyhow::bail!("stopped while waiting for {}", root.join("build.lock").display());
			}
		}
	}
}

/// The lifetime lock file's path inside a generation directory.
fn lifetime_path(dir: &Path) -> PathBuf {
	dir.join("lifetime.lock")
}

/// Opens (creating if absent) a generation's lifetime lock file. Called
/// under the build lock, before any of the generation's databases open.
fn open_lifetime(dir: &Path) -> Result<File> {
	open_lock_file(&lifetime_path(dir))
}

/// Takes a generation's lifetime lock shared, for this process's whole
/// life. Called under the build lock, where the only possible conflict —
/// a cleaner's exclusive try — never blocks, so the poll is short.
///
/// # Errors
///
/// When the file can't be opened or locked, or the wait is cancelled.
pub async fn lifetime_shared(dir: &Path, mut cancelled: Cancel) -> Result<()> {
	let file = open_lifetime(dir)?;

	loop {
		match flock(&file, FlockOperation::NonBlockingLockShared) {
			Ok(()) => {
				retain("generation lifetime", file);
				return Ok(());
			}
			Err(error) if would_block(&error) => {}
			Err(error) => {
				return Err(anyhow::Error::new(error).context(format!(
					"sharing the lifetime lock at {}",
					lifetime_path(dir).display()
				)));
			}
		}

		tokio::select! {
			() = tokio::time::sleep(POLL) => {}
			() = &mut cancelled => {
				anyhow::bail!("stopped while waiting to share {}", lifetime_path(dir).display());
			}
		}
	}
}

/// Takes a generation's lifetime lock exclusively without blocking, for
/// cleanup. `Ok(None)` means a live holder keeps the generation: nothing
/// is deleted and nothing waits. A directory that doesn't exist has no
/// lock to take and nothing to protect.
///
/// # Errors
///
/// Only when the lock file can't be opened (a held lock is `Ok(None)`).
pub fn try_lifetime_exclusive(dir: &Path) -> Result<Option<File>> {
	if !dir.is_dir() {
		return Ok(None);
	}
	let file = open_lifetime(dir)?;

	match flock(&file, FlockOperation::NonBlockingLockExclusive) {
		Ok(()) => Ok(Some(file)),
		Err(error) if would_block(&error) => Ok(None),
		Err(error) => {
			Err(anyhow::Error::new(error).context(format!(
				"taking {} exclusively",
				lifetime_path(dir).display()
			)))
		}
	}
}

/// Waits until a generation's lifetime lock can be taken exclusively:
/// that is, until its previous initializer is dead. Called under the
/// build lock before an unpublished directory is rebuilt, so a crashed
/// initializer's successor never deletes state a live process still
/// holds.
///
/// # Errors
///
/// When the lock file can't be opened, or the wait is cancelled.
pub async fn wait_lifetime_free(dir: &Path, mut cancelled: Cancel) -> Result<()> {
	loop {
		if try_lifetime_exclusive(dir)?.is_some() {
			return Ok(());
		}

		tokio::select! {
			() = tokio::time::sleep(POLL) => {}
			() = &mut cancelled => {
				anyhow::bail!("stopped while waiting for {} to free {}", dir.display(), lifetime_path(dir).display());
			}
		}
	}
}

/// Takes the generation's worker lock, polling at most once a second, for
/// as long as this process runs its worker. The candidate that loses the
/// race keeps serving without a worker; SIGTERM stops the wait.
///
/// # Errors
///
/// When the file can't be opened or locked, or the wait is cancelled.
pub async fn worker_lock(dir: &Path, mut cancelled: Cancel) -> Result<()> {
	let file = open_lock_file(&dir.join("worker.lock"))?;

	loop {
		match flock(&file, FlockOperation::NonBlockingLockExclusive) {
			Ok(()) => {
				retain("worker", file);
				return Ok(());
			}
			Err(error) if would_block(&error) => {}
			Err(error) => {
				return Err(anyhow::Error::new(error)
					.context(format!("locking {}", dir.join("worker.lock").display())));
			}
		}

		tokio::select! {
			() = tokio::time::sleep(Duration::from_secs(1)) => {}
			() = &mut cancelled => {
				anyhow::bail!("stopped while waiting for {}", dir.join("worker.lock").display());
			}
		}
	}
}

/// Keeps a build lock until exec or exit, after a failed initialization:
/// background writers may already exist, so no other process may touch
/// this volume until this image is gone. This is the difference between
/// "returning an async error" and "releasing the lock".
pub fn retain_build(file: File) {
	retain("build", file);
}

/// Whether the worker lock at `dir` is currently free, for tests and for
/// the worker task's own decision to start.
#[cfg(test)]
pub fn worker_lock_free(dir: &Path) -> bool {
	let Ok(file) = open_lock_file(&dir.join("worker.lock")) else {
		return false;
	};
	flock(&file, FlockOperation::NonBlockingLockExclusive).is_ok()
}

/// Rejects a caller that tries to release a retained lock early. Nothing
/// deletes from [`RETAINED`]; this exists so the invariant is stated where
/// a future edit would break it.
///
/// # Errors
///
/// Always, with the reason the locks are process-lifetime.
#[allow(dead_code)]
fn release_retained_early() -> Result<()> {
	anyhow::bail!(
		"worker and lifetime locks are held by the process image, not by tasks: they are released \
		 by exec or exit, after every writer has stopped, never earlier"
	)
}

/// A build lock's guard for tests: proves two builders exclude each other.
#[cfg(test)]
pub fn try_build_lock(root: &Path) -> Result<Option<File>> {
	let file = open_lock_file(&root.join("build.lock"))?;
	match flock(&file, FlockOperation::NonBlockingLockExclusive) {
		Ok(()) => Ok(Some(file)),
		Err(error) if would_block(&error) => Ok(None),
		Err(error) => Err(anyhow::Error::new(error).context("locking build.lock")),
	}
}

/// Every lock file is private, and a held lifetime lock blocks an
/// exclusive taker without blocking the caller.
#[cfg(test)]
mod tests {
	use anyhow::ensure;
	use tempfile::TempDir;

	use super::*;

	#[tokio::test]
	async fn build_lock_excludes_and_releases() -> Result<()> {
		let root = TempDir::new()?;
		let first = build_lock(root.path(), Box::pin(std::future::pending())).await?;
		let second = try_build_lock(root.path())?;
		ensure!(second.is_none(), "a second builder must be excluded");
		drop(first);
		let retry = try_build_lock(root.path())?;
		ensure!(retry.is_some(), "a released build lock must be takable");
		Ok(())
	}

	#[tokio::test]
	async fn lifetime_locks_share_and_exclusive_tries_fail() -> Result<()> {
		let temp = TempDir::new()?;
		let dir = temp.path().to_path_buf();
		lifetime_shared(&dir, Box::pin(std::future::pending())).await?;
		ensure!(
			try_lifetime_exclusive(&dir)?.is_none(),
			"a shared lifetime lock must refuse an exclusive taker"
		);

		let other_temp = TempDir::new()?;
		let other = other_temp.path().join("other");
		std::fs::create_dir_all(&other)?;
		ensure!(
			try_lifetime_exclusive(&other)?.is_some(),
			"an unheld generation's lifetime lock is takable"
		);
		Ok(())
	}

	#[tokio::test]
	async fn cancelled_waits_return_errors() -> Result<()> {
		let temp = TempDir::new()?;
		let dir = temp.path().to_path_buf();
		lifetime_shared(&dir, Box::pin(std::future::pending())).await?;

		let cancelled: Cancel = Box::pin(async {
			tokio::time::sleep(Duration::from_millis(50)).await;
		});
		let waited = wait_lifetime_free(&dir, cancelled).await;
		ensure!(waited.is_err(), "a cancelled wait must return, not hang");
		Ok(())
	}
}
