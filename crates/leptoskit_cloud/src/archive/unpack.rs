//! Unpacking a source archive into an empty directory, refusing anything
//! that could write outside it.
//!
//! The archive comes from a customer, so every entry is treated as hostile:
//! paths must be relative with no `..`, only files, directories, and
//! symlinks are accepted, the bounds are counted from the headers before any
//! byte is written, and symlinks are created last, after every file, so no
//! write can pass through one. Each symlink is followed through the
//! archive's other symlinks, on the archive's own paths, before it is
//! created. Nothing here runs a program.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs::File;
use std::io;
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;

use flate2::read::GzDecoder;

use super::Limits;
use super::paths;

/// What [`unpack`] wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unpacked {
	/// The files and symlinks written, in archive order.
	pub files: Vec<String>,
	/// The files' bytes.
	pub bytes: u64,
}

/// Why an archive couldn't be unpacked. When unpacking fails part of the
/// way, the destination holds what was written so far: discard it.
#[derive(Debug, thiserror::Error)]
pub enum UnpackError {
	/// The destination exists and isn't an empty directory.
	#[error("{} must be an empty directory, or not exist yet", .0.display())]
	DestinationNotEmpty(PathBuf),
	/// Reading the archive or writing the destination failed.
	#[error("could not unpack the archive")]
	Io(#[from] io::Error),
	/// An entry's path isn't UTF-8.
	#[error("an entry's path isn't UTF-8")]
	NotUtf8,
	/// A path the archive may not hold.
	#[error("the entry `{path}` {reason}")]
	Path {
		/// The path, as the archive spells it.
		path: String,
		/// What is wrong with it.
		reason: &'static str,
	},
	/// The same path twice, or a path that is both a directory and a file
	/// or symlink.
	#[error("the entry `{0}` collides with another entry")]
	Collision(String),
	/// An entry under a path the archive makes a symlink.
	#[error("the entry `{path}` would be written through the symlink `{link}`")]
	ThroughLink {
		/// The entry.
		path: String,
		/// The symlink above it.
		link: String,
	},
	/// A symlink that leaves the destination.
	#[error("the symlink `{path}` points at `{target}`, outside the archive")]
	EscapingLink {
		/// The symlink.
		path: String,
		/// Where it points.
		target: String,
	},
	/// A hard link, device, FIFO, or other entry that isn't a file,
	/// directory, or symlink.
	#[error("the entry `{path}` is a {kind}, which a source archive can't hold")]
	Unsupported {
		/// The entry.
		path: String,
		/// What kind of entry it is.
		kind: String,
	},
	/// More entries (files, symlinks, and directories) than
	/// [`Limits::max_files`].
	#[error("the archive has more than {limit} entries")]
	TooManyFiles {
		/// The limit.
		limit: u64,
	},
	/// One file over [`Limits::max_file_bytes`].
	#[error("the file `{path}` is {bytes} bytes, over the {limit}-byte limit for one file")]
	FileTooLarge {
		/// The file.
		path: String,
		/// Its size, from its header.
		bytes: u64,
		/// The limit.
		limit: u64,
	},
	/// More than [`Limits::max_bytes`] in all.
	#[error("the archive's files come to more than {limit} bytes")]
	TooLarge {
		/// The limit.
		limit: u64,
	},
	/// A symlink on a platform that can't make one here.
	#[error("the archive has the symlink `{0}`, and this platform unpacks only on Unix")]
	LinksUnsupported(String),
}

/// Unpacks the gzip-compressed tar `reader` gives into `destination`, which
/// must be empty or not exist.
///
/// # Errors
///
/// An [`UnpackError`] for the first entry that breaks a rule, or an I/O
/// failure.
pub fn unpack(
	reader: impl Read,
	destination: &Path,
	limits: &Limits,
) -> Result<Unpacked, UnpackError> {
	prepare(destination)?;
	let mut archive = tar::Archive::new(GzDecoder::new(reader));
	let mut state = State::default();

	for entry in archive.entries()? {
		let mut entry = entry?;
		let path =
			String::from_utf8(entry.path_bytes().into_owned()).map_err(|_| UnpackError::NotUtf8)?;
		// A directory entry may end with `/`.
		let kind = entry.header().entry_type();
		let path = if kind.is_dir() {
			path.trim_end_matches('/').to_owned()
		} else {
			path
		};

		if let Some(reason) = paths::path_problem(&path) {
			return Err(UnpackError::Path { path, reason });
		}

		state.check_place(&path, kind.is_dir())?;

		match kind {
			tar::EntryType::Directory => {
				state.count_entry(limits)?;
				std::fs::create_dir_all(destination.join(&path))?;
				state.directories.insert(path);
			}
			tar::EntryType::Regular | tar::EntryType::Continuous => {
				let size = entry.header().size()?;
				state.count_file(&path, size, limits)?;
				let target = destination.join(&path);

				if let Some(parent) = target.parent() {
					std::fs::create_dir_all(parent)?;
				}

				// `create_new` refuses an existing path, symlink or not.
				let mut file = File::create_new(&target)?;
				let written = io::copy(&mut (&mut entry).take(size), &mut file)?;

				if written != size {
					return Err(io::Error::new(
						io::ErrorKind::UnexpectedEof,
						format!("`{path}` ended before its {size} bytes"),
					)
					.into());
				}

				set_mode(&file, entry.header().mode()?)?;
				state.bytes += size;
				state.files.push(path);
			}
			tar::EntryType::Symlink => {
				let target = entry
					.link_name_bytes()
					.map(|target| String::from_utf8(target.into_owned()))
					.transpose()
					.map_err(|_| UnpackError::NotUtf8)?
					.unwrap_or_default();
				state.count_entry(limits)?;
				state.links.insert(path.clone(), target);
				state.files.push(path);
			}
			other => {
				return Err(UnpackError::Unsupported {
					path,
					kind: format!("{other:?}"),
				});
			}
		}
	}

	state.finish(destination)
}

/// Makes `destination`, or checks that it is an empty directory.
fn prepare(destination: &Path) -> Result<(), UnpackError> {
	match std::fs::read_dir(destination) {
		Ok(mut entries) => {
			if entries.next().is_some() {
				return Err(UnpackError::DestinationNotEmpty(destination.to_owned()));
			}

			Ok(())
		}
		Err(error) if error.kind() == io::ErrorKind::NotFound => {
			std::fs::create_dir_all(destination)?;
			Ok(())
		}
		Err(error) => Err(error.into()),
	}
}

/// What has been unpacked so far.
#[derive(Default)]
struct State {
	/// Every path an entry named.
	seen: BTreeSet<String>,
	/// Directories, named or implied by a deeper entry.
	directories: BTreeSet<String>,
	/// Symlinks still to create, path to target.
	links: BTreeMap<String, String>,
	/// Files and symlinks, in archive order.
	files: Vec<String>,
	entries: u64,
	bytes: u64,
}

impl State {
	/// Refuses a path seen before, a file or symlink where a directory is,
	/// and anything under a symlink; then records the path's ancestors as
	/// directories.
	fn check_place(&mut self, path: &str, directory: bool) -> Result<(), UnpackError> {
		if !self.seen.insert(path.to_owned()) {
			return Err(UnpackError::Collision(path.to_owned()));
		}

		if !directory && self.directories.contains(path) {
			return Err(UnpackError::Collision(path.to_owned()));
		}

		let components: Vec<&str> = path.split('/').collect();
		let ancestors = components.len().saturating_sub(1);
		let mut ancestor = String::new();

		for component in &components[..ancestors] {
			if !ancestor.is_empty() {
				ancestor.push('/');
			}

			ancestor.push_str(component);

			if self.links.contains_key(&ancestor) {
				return Err(UnpackError::ThroughLink {
					path: path.to_owned(),
					link: ancestor,
				});
			}

			if self.seen.contains(&ancestor) && !self.directories.contains(&ancestor) {
				return Err(UnpackError::Collision(ancestor));
			}

			self.directories.insert(ancestor.clone());
		}

		Ok(())
	}

	fn count_entry(&mut self, limits: &Limits) -> Result<(), UnpackError> {
		self.entries += 1;

		if self.entries > limits.max_files {
			return Err(UnpackError::TooManyFiles {
				limit: limits.max_files,
			});
		}

		Ok(())
	}

	fn count_file(&mut self, path: &str, size: u64, limits: &Limits) -> Result<(), UnpackError> {
		self.count_entry(limits)?;

		if size > limits.max_file_bytes {
			return Err(UnpackError::FileTooLarge {
				path: path.to_owned(),
				bytes: size,
				limit: limits.max_file_bytes,
			});
		}

		if self.bytes.saturating_add(size) > limits.max_bytes {
			return Err(UnpackError::TooLarge {
				limit: limits.max_bytes,
			});
		}

		Ok(())
	}

	/// Checks every symlink against the whole archive, then creates them.
	fn finish(self, destination: &Path) -> Result<Unpacked, UnpackError> {
		for (path, target) in &self.links {
			if self.directories.contains(path) {
				return Err(UnpackError::Collision(path.clone()));
			}

			if !paths::link_stays_inside(path, &self.links) {
				return Err(UnpackError::EscapingLink {
					path: path.clone(),
					target: target.clone(),
				});
			}
		}

		for (path, target) in &self.links {
			let link = destination.join(path);

			if let Some(parent) = link.parent() {
				std::fs::create_dir_all(parent)?;
			}

			make_link(target, &link, path)?;
		}

		Ok(Unpacked {
			files: self.files,
			bytes: self.bytes,
		})
	}
}

#[cfg(unix)]
fn make_link(target: &str, link: &Path, _: &str) -> Result<(), UnpackError> {
	std::os::unix::fs::symlink(target, link)?;
	Ok(())
}

#[cfg(not(unix))]
fn make_link(_: &str, _: &Path, path: &str) -> Result<(), UnpackError> {
	Err(UnpackError::LinksUnsupported(path.to_owned()))
}

/// `0755` when the header gives anyone execute permission, `0644`
/// otherwise; never set-user-ID, set-group-ID, or sticky bits.
#[cfg(unix)]
fn set_mode(file: &File, mode: u32) -> io::Result<()> {
	use std::os::unix::fs::PermissionsExt;

	let mode = if mode & 0o111 == 0 { 0o644 } else { 0o755 };
	file.set_permissions(std::fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_mode(_: &File, _: u32) -> io::Result<()> {
	Ok(())
}
