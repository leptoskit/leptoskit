//! The publication durability barrier.
//!
//! `leptoskit_db`'s file-backed SQLite connections run `journal_mode=WAL`
//! with `synchronous=NORMAL`: a committed, readable transaction is in the
//! WAL, but the WAL need not be on the platter yet. A healthy-machine
//! crash test cannot see the difference — the kernel keeps the page
//! cache — so publication cannot lean on "the commit answered". The
//! barrier is explicit instead: every file under the generation (its
//! databases, their WAL and shared-memory sidecars, the credentials, the
//! project's files) and every directory entry from the generation up to
//! and including the volume root are fsynced, *before* the `published`
//! marker is written. The marker, in turn, is fsynced and renamed into
//! place by [`crate::marker::Published::commit`], so a generation is
//! either published-and-durable or absent.
//!
//! The barrier runs under the build lock while the generation is still
//! unpublished, with every file already at its stable final path. Its
//! failure propagates: nothing is published, the directory stays an
//! abandoned build, and the next image rebuilds it.
//!
//! What this proves and what it doesn't: the fsync calls are issued and
//! their errors propagated in a testable order, and a crash after the
//! barrier leaves a recoverable state. Power-cut behavior itself — that
//! the drive actually honors the syncs — cannot be proven on a healthy
//! machine and is not claimed.

use std::path::Path;

use anyhow::Context as _;
use anyhow::Result;

/// Establishes the durability barrier over a generation: fsyncs every
/// regular file in the tree (children before parents, files before the
/// directory entries that name them), then every directory from the
/// generation up to and including the state volume's root.
///
/// # Errors
///
/// When any file or directory cannot be opened, read, or synced. A
/// failure leaves the generation unpublished; the caller must not
/// publish over it.
pub fn establish(dir: &Path) -> Result<()> {
	let generations = dir
		.parent()
		.context("a generation lives under <volume>/gen/<day>.f<format>")?;
	let volume = generations
		.parent()
		.context("the generations live under the state volume")?;

	sync_tree(dir)?;
	sync_dir(generations)?;
	sync_dir(volume)
}

/// fsyncs every regular file in the tree rooted at `dir`, then the
/// directories of the tree, deepest first: a directory entry is synced
/// only after the entries it names are durable.
fn sync_tree(dir: &Path) -> Result<()> {
	let mut files = Vec::new();
	let mut directories = vec![dir.to_path_buf()];
	let mut walk = vec![dir.to_path_buf()];

	while let Some(current) = walk.pop() {
		for entry in
			std::fs::read_dir(&current).with_context(|| format!("reading {}", current.display()))?
		{
			let entry = entry.with_context(|| format!("reading {}", current.display()))?;
			let path = entry.path();
			// `file_type` does not follow symlinks; a generation holds
			// none, and anything else is synced only if it is a real
			// file.
			let kind = entry
				.file_type()
				.with_context(|| format!("inspecting {}", path.display()))?;
			if kind.is_dir() {
				directories.push(path.clone());
				walk.push(path);
			} else if kind.is_file() {
				files.push(path);
			}
		}
	}

	for file in files {
		sync_file(&file)?;
	}
	for directory in directories.iter().rev() {
		sync_dir(directory)?;
	}
	Ok(())
}

/// fsyncs one file, by a read-only handle: fsync carries the file's
/// dirty pages to the volume whatever the handle's mode is.
fn sync_file(path: &Path) -> Result<()> {
	let handle =
		std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
	handle
		.sync_all()
		.with_context(|| format!("syncing {}", path.display()))
}

/// fsyncs a directory, so the entries in it survive a power cut.
fn sync_dir(dir: &Path) -> Result<()> {
	let handle = std::fs::File::open(dir)
		.with_context(|| format!("opening {} to sync its entries", dir.display()))?;
	handle
		.sync_all()
		.with_context(|| format!("syncing {}", dir.display()))
}

#[cfg(test)]
mod tests {
	use anyhow::ensure;
	use tempfile::TempDir;

	use super::*;

	/// The barrier covers the tree it is handed: nested files (a
	/// database in a subdirectory), sidecars, and the directory entries
	/// up to the volume root — and it touches nothing while doing so.
	#[test]
	fn the_barrier_covers_the_tree_without_changing_it() -> Result<()> {
		let volume = TempDir::new()?;
		let dir = volume.path().join("gen").join("2026-11-15.f1");
		std::fs::create_dir_all(dir.join("admin"))?;
		std::fs::create_dir_all(dir.join("project").join(".git"))?;
		std::fs::write(dir.join("analytics.sqlite3"), b"database")?;
		std::fs::write(dir.join("analytics.sqlite3-wal"), b"wal")?;
		std::fs::write(dir.join("admin").join("admin.sqlite3"), b"admin")?;
		std::fs::write(dir.join("project").join(".git").join("config"), b"git")?;
		std::fs::write(dir.join("secrets.json"), b"secrets")?;

		establish(&dir)?;

		ensure!(dir.join("analytics.sqlite3-wal").is_file(), "nothing moved");
		ensure!(
			dir.join("project/.git/config").is_file(),
			"the nested tree is intact"
		);
		Ok(())
	}

	/// A barrier that cannot sync something is an error, never a silent
	/// pass: publication must not proceed over a failed sync. An
	/// unopenable file — here, one this account cannot read — is the
	/// failure under test.
	#[cfg(unix)]
	#[test]
	fn a_barrier_failure_is_an_error_and_names_the_file() -> Result<()> {
		let volume = TempDir::new()?;
		let dir = volume.path().join("gen").join("2026-11-15.f1");
		std::fs::create_dir_all(&dir)?;
		let unreadable = dir.join("state.sqlite3");
		std::fs::write(&unreadable, b"state")?;
		use std::os::unix::fs::PermissionsExt as _;
		std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o000))?;

		let outcome = establish(&dir);

		std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o600))?;
		ensure!(
			outcome.is_err(),
			"a file the barrier cannot sync must fail the barrier, not publish over it"
		);
		let error = outcome.expect_err("checked above");
		ensure!(
			error.to_string().contains("state.sqlite3"),
			"the failure names the file: {error:#}"
		);
		Ok(())
	}
}
