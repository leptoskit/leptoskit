//! Exact static artifact membership, shared by website and graph releases.
//! Capturing bytes proves integrity only; reproduction is a separate step.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;
use anyhow::ensure;
use serde::Deserialize;
use serde::Serialize;

#[cfg(test)]
mod tests;
#[cfg(unix)]
mod unix;

pub(crate) const MAX_ENTRIES: usize = 100_000;
pub(crate) const MAX_BYTES: u64 = 16 * 1024 * 1024 * 1024;
pub(crate) const MAX_DEPTH: usize = 128;
pub(crate) const MAX_METADATA_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Member {
	Directory {
		mode: u32,
	},
	File {
		mode: u32,
		size: u64,
		sha256: String,
	},
}

pub(crate) type Tree = BTreeMap<String, Member>;

pub(crate) fn safe_path(path: &str) -> bool {
	!path.is_empty()
		&& !path.contains('\\')
		&& !path.chars().any(char::is_control)
		&& path.split('/').all(|part| !matches!(part, "" | "." | ".."))
		&& path.split('/').count() <= MAX_DEPTH
}

pub(crate) fn validate(tree: &Tree) -> Result<()> {
	ensure!(tree.len() <= MAX_ENTRIES, "asset tree is too large");
	let mut total = 0_u64;

	for (path, member) in tree {
		ensure!(safe_path(path), "unsafe asset path");

		if let Some((parent, _)) = path.rsplit_once('/') {
			ensure!(
				matches!(tree.get(parent), Some(Member::Directory { .. })),
				"missing asset parent directory"
			);
		}

		match member {
			Member::Directory { mode } => {
				ensure!(*mode == 0o755, "asset directories require mode 0755");
			}
			Member::File { mode, size, sha256 } => {
				ensure!(
					*mode == 0o644 && digest(sha256),
					"asset files require mode 0644 and SHA-256"
				);
				total = total
					.checked_add(*size)
					.filter(|total| *total <= MAX_BYTES)
					.ok_or_else(|| anyhow::anyhow!("asset tree exceeds its byte limit"))?;
			}
		}
	}

	Ok(())
}

fn digest(value: &str) -> bool {
	value.len() == 64
		&& value
			.bytes()
			.all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// A root directory held open throughout inventory and copying. On Unix,
/// every descendant is opened relative to its parent without following links.
pub(crate) struct Root {
	#[cfg(unix)]
	file: std::fs::File,
}

#[cfg_attr(
	not(unix),
	allow(
		clippy::unused_self,
		reason = "non-Unix refusals retain the descriptor capability API without opening files"
	)
)]
impl Root {
	/// Bounded transport read using retained no-follow descriptors.
	pub(crate) fn release_bytes(&self, relative: &str, limit: u64) -> Result<Vec<u8>> {
		ensure!(
			safe_path(relative) && limit <= 64 * 1024 * 1024,
			"invalid release transport bound"
		);
		#[cfg(unix)]
		return unix::read_file(self, relative, limit);
		#[cfg(not(unix))]
		anyhow::bail!("release transport requires Unix descriptors");
	}

	/// Retain a no-follow descriptor for bounded, non-executing format
	/// inspection.
	pub(crate) fn open_executable(&self, relative: &str) -> Result<std::fs::File> {
		ensure!(safe_path(relative), "invalid executable path");
		#[cfg(unix)]
		return unix::open_executable(self, relative);
		#[cfg(not(unix))]
		anyhow::bail!("exact executable inspection requires Unix permission metadata");
	}

	/// Read pinned artifact metadata with a separate bounded document budget.
	pub(crate) fn metadata(&self, relative: &str) -> Result<Vec<u8>> {
		ensure!(safe_path(relative), "invalid artifact metadata path");
		#[cfg(unix)]
		return unix::read_file(self, relative, MAX_METADATA_BYTES);
		#[cfg(not(unix))]
		anyhow::bail!("exact asset capture requires Unix permission metadata");
	}

	/// List the immediate directory membership without following entries.
	pub(crate) fn names(&self) -> Result<Vec<String>> {
		#[cfg(unix)]
		return unix::root_names(self);
		#[cfg(not(unix))]
		anyhow::bail!("exact asset capture requires Unix permission metadata");
	}

	/// Check executable bytes and mode without executing them.
	pub(crate) fn executable(&self, relative: &str) -> Result<Member> {
		ensure!(safe_path(relative), "invalid executable path");
		#[cfg(unix)]
		return unix::executable(self, relative);
		#[cfg(not(unix))]
		anyhow::bail!("exact executable capture requires Unix permission metadata");
	}

	/// Read a small declared input without following relative links.
	pub(crate) fn read_file(&self, relative: &str, limit: u64) -> Result<Vec<u8>> {
		ensure!(
			safe_path(relative) && limit <= 1024 * 1024,
			"invalid bounded artifact input"
		);

		#[cfg(unix)]
		return unix::read_file(self, relative, limit);
		#[cfg(not(unix))]
		anyhow::bail!("exact asset capture requires Unix permission metadata");
	}

	/// Capture an explicitly declared executable separately from public assets.
	pub(crate) fn copy_executable(&self, relative: &str, destination: &Path) -> Result<Member> {
		ensure!(safe_path(relative), "invalid executable path");

		#[cfg(unix)]
		return unix::copy_executable(self, relative, destination);
		#[cfg(not(unix))]
		{
			let _ = destination;
			anyhow::bail!("exact executable capture requires Unix permission metadata");
		}
	}

	/// Opens a trusted root path; its final component must be a real directory.
	pub(crate) fn open(path: &Path) -> Result<Self> {
		#[cfg(unix)]
		return unix::open(path);
		#[cfg(not(unix))]
		{
			let _ = path;
			anyhow::bail!("exact asset capture requires Unix permission metadata");
		}
	}

	/// Resolve a declared output from a trusted project directory, refusing
	/// links at every relative component. An empty path denotes this root.
	pub(crate) fn beneath(&self, relative: &str) -> Result<Self> {
		ensure!(
			relative.is_empty() || safe_path(relative),
			"invalid asset directory"
		);

		#[cfg(unix)]
		return unix::beneath(self, relative);
		#[cfg(not(unix))]
		anyhow::bail!("exact asset capture requires Unix permission metadata");
	}

	pub(crate) fn tree(&self, excluded: &[String]) -> Result<Tree> {
		ensure!(
			excluded.iter().all(|path| safe_path(path)),
			"invalid excluded asset path"
		);

		#[cfg(unix)]
		return unix::tree(self, excluded);
		#[cfg(not(unix))]
		anyhow::bail!("exact asset capture requires Unix permission metadata");
	}

	/// Copy only the independently captured tree, rejecting changed bytes,
	/// modes or membership. The caller owns the new destination and discards
	/// it on error; an incomplete copy must never become a successful release.
	pub(crate) fn copy(
		&self,
		destination: &Path,
		excluded: &[String],
		expected: &Tree,
	) -> Result<()> {
		validate(expected)?;
		ensure!(
			self.tree(excluded)? == *expected,
			"asset membership changed before copying"
		);
		#[cfg(unix)]
		unix::copy(self, destination, expected)?;
		#[cfg(not(unix))]
		{
			let _ = destination;
			anyhow::bail!("exact asset capture requires Unix permission metadata");
		}
		#[cfg(unix)]
		{
			ensure!(
				Self::open(destination)?.tree(&[])? == *expected,
				"copied asset tree differs"
			);
			ensure!(
				self.tree(excluded)? == *expected,
				"asset membership changed while copying"
			);
			Ok(())
		}
	}
}
