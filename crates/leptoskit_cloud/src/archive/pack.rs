//! Packing a project into a source archive.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use flate2::Compression;
use flate2::GzBuilder;
use ignore::gitignore::Gitignore;
use ignore::gitignore::GitignoreBuilder;

use super::Digest;
use super::IGNORE_FILE;
use super::Limits;
use super::paths;

/// How [`pack`] chooses and bounds the files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackOptions {
	/// The bounds, normally [`Limits::DEFAULT`].
	pub limits: Limits,
	/// Whether to ask Git for the file list when the root is in a Git
	/// checkout. Off, or when Git isn't installed or the root isn't a
	/// checkout, the walk that honours `.gitignore` chooses.
	pub use_git: bool,
}

impl Default for PackOptions {
	fn default() -> Self {
		Self {
			limits: Limits::DEFAULT,
			use_git: true,
		}
	}
}

/// A packed source archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packed {
	/// The gzip-compressed tar.
	pub bytes: Vec<u8>,
	/// The SHA-256 of `bytes`.
	pub digest: Digest,
	/// Every path in the archive, files and symlinks, in archive order.
	pub files: Vec<String>,
	/// The files' bytes before compression.
	pub uncompressed_bytes: u64,
}

/// Why a project couldn't be packed.
#[derive(Debug, thiserror::Error)]
pub enum PackError {
	/// Reading the project failed.
	#[error("could not read {}", path.display())]
	Io {
		/// The file or directory.
		path: PathBuf,
		/// The underlying error.
		#[source]
		source: io::Error,
	},
	/// Building the archive in memory failed.
	#[error("could not write the archive")]
	Write(#[source] io::Error),
	/// `git ls-files` failed in a checkout.
	#[error("`git ls-files` failed in {}: {message}", root.display())]
	Git {
		/// The project's root.
		root: PathBuf,
		/// What git said.
		message: String,
	},
	/// The ignore walk or `.leptoskitignore` failed.
	#[error("could not read the ignore files: {0}")]
	Ignore(#[from] ignore::Error),
	/// A path isn't UTF-8, so the archive can't name it portably.
	#[error("{} isn't UTF-8; rename it, or add it to .leptoskitignore", path.display())]
	NotUtf8 {
		/// The path, relative to the root.
		path: PathBuf,
	},
	/// A path the archive can't hold.
	#[error("`{path}` {reason}")]
	Path {
		/// The path, relative to the root.
		path: String,
		/// What is wrong with it.
		reason: &'static str,
	},
	/// A Git submodule.
	#[error(
		"`{0}` is a Git submodule, which the source archive doesn't include; vendor it, or add it \
		 to .leptoskitignore"
	)]
	Submodule(String),
	/// A socket, device, or other file that isn't a file, directory, or
	/// symlink.
	#[error("`{0}` isn't a regular file or symlink; add it to .leptoskitignore")]
	Unsupported(String),
	/// A symlink that leaves the root.
	#[error(
		"`{path}` links to `{target}`, outside the project; the source archive keeps only links \
		 that stay inside it"
	)]
	EscapingLink {
		/// The symlink.
		path: String,
		/// Where it points.
		target: String,
	},
	/// Files that usually hold secrets.
	#[error(
		"{} would go into the source archive, and files like these hold secrets; set each value \
		 with `lk cloud secret set <NAME>`, then add the files to .leptoskitignore",
		backticked(paths)
	)]
	Secrets {
		/// The files, relative to the root.
		paths: Vec<String>,
	},
	/// More files than [`Limits::max_files`].
	#[error(
		"the project has more than {limit} files; add what the build doesn't need to \
		 .leptoskitignore"
	)]
	TooManyFiles {
		/// The limit.
		limit: u64,
	},
	/// One file over [`Limits::max_file_bytes`].
	#[error(
		"`{path}` is {bytes} bytes, over the {limit}-byte limit for one file; add it to \
		 .leptoskitignore"
	)]
	FileTooLarge {
		/// The file.
		path: String,
		/// Its size.
		bytes: u64,
		/// The limit.
		limit: u64,
	},
	/// More than [`Limits::max_bytes`] in all.
	#[error(
		"the project's files come to more than {limit} bytes; add what the build doesn't need to \
		 .leptoskitignore"
	)]
	TooLarge {
		/// The limit.
		limit: u64,
	},
}

fn backticked(paths: &[String]) -> String {
	paths
		.iter()
		.map(|path| format!("`{path}`"))
		.collect::<Vec<_>>()
		.join(", ")
}

/// One entry, read from the project.
enum Entry {
	File { data: Vec<u8>, executable: bool },
	Link { target: String },
}

/// Packs the project at `root` into a deterministic source archive.
///
/// # Errors
///
/// A [`PackError`] for the first rule the project breaks, or a file that
/// can't be read.
pub fn pack(root: &Path, options: &PackOptions) -> Result<Packed, PackError> {
	let limits = options.limits;
	let ignore = leptoskit_ignore(root)?;
	let from_git = if options.use_git {
		git_files(root)?
	} else {
		None
	};
	let mut listed = match from_git {
		Some(listed) => listed,
		None => walked_files(root)?,
	};
	listed.retain(|path| !paths::always_excluded(path) && !ignored(&ignore, path));
	listed.sort_unstable();
	listed.dedup();

	if let Some((path, reason)) = listed
		.iter()
		.find_map(|path| paths::path_problem(path).map(|reason| (path, reason)))
	{
		return Err(PackError::Path {
			path: path.clone(),
			reason,
		});
	}

	let present = present_files(root, listed)?;

	if present.len() as u64 > limits.max_files {
		return Err(PackError::TooManyFiles {
			limit: limits.max_files,
		});
	}

	let secrets: Vec<String> = present
		.iter()
		.map(|(path, _)| path)
		.filter(|path| paths::looks_secret(path))
		.cloned()
		.collect();

	if !secrets.is_empty() {
		return Err(PackError::Secrets { paths: secrets });
	}

	let entries = read_entries(root, present, limits)?;
	let links: BTreeMap<String, String> = entries
		.iter()
		.filter_map(|(path, entry)| {
			match entry {
				Entry::Link { target } => Some((path.clone(), target.clone())),
				Entry::File { .. } => None,
			}
		})
		.collect();

	if let Some((path, target)) = links
		.iter()
		.find(|(path, _)| !paths::link_stays_inside(path, &links))
	{
		return Err(PackError::EscapingLink {
			path: path.clone(),
			target: target.clone(),
		});
	}

	write_archive(entries)
}

/// The `.leptoskitignore` matcher, or an empty one when the file is absent.
fn leptoskit_ignore(root: &Path) -> Result<Gitignore, PackError> {
	let mut builder = GitignoreBuilder::new(root);
	let file = root.join(IGNORE_FILE);

	if file.is_file()
		&& let Some(error) = builder.add(&file)
	{
		return Err(error.into());
	}

	Ok(builder.build()?)
}

fn ignored(ignore: &Gitignore, path: &str) -> bool {
	ignore.matched_path_or_any_parents(path, false).is_ignore()
}

/// The variables that point git at a repository whatever directory it runs
/// in. Git sets some of them for hooks, so packing from inside one, such as
/// a pre-push hook's tests, would otherwise list the hook's repository.
const REPOSITORY_VARIABLES: &[&str] = &[
	"GIT_DIR",
	"GIT_WORK_TREE",
	"GIT_INDEX_FILE",
	"GIT_COMMON_DIR",
	"GIT_OBJECT_DIRECTORY",
	"GIT_ALTERNATE_OBJECT_DIRECTORIES",
	"GIT_NAMESPACE",
	"GIT_PREFIX",
];

fn git(root: &Path) -> Command {
	let mut command = Command::new("git");
	command.arg("-C").arg(root);

	for variable in REPOSITORY_VARIABLES {
		command.env_remove(variable);
	}

	command
}

/// The files Git lists, or `None` when Git isn't installed or `root` isn't
/// in a checkout.
fn git_files(root: &Path) -> Result<Option<Vec<String>>, PackError> {
	let inside = match git(root)
		.args(["rev-parse", "--is-inside-work-tree"])
		.output()
	{
		Ok(output) => output,
		Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
		Err(source) => {
			return Err(PackError::Io {
				path: root.to_owned(),
				source,
			});
		}
	};

	if !inside.status.success() || inside.stdout.trim_ascii() != b"true" {
		return Ok(None);
	}

	let listed = git(root)
		.args([
			"ls-files",
			"-z",
			"--cached",
			"--others",
			"--exclude-standard",
		])
		.output()
		.map_err(|source| {
			PackError::Io {
				path: root.to_owned(),
				source,
			}
		})?;

	if !listed.status.success() {
		return Err(PackError::Git {
			root: root.to_owned(),
			message: String::from_utf8_lossy(&listed.stderr).trim().to_owned(),
		});
	}

	listed
		.stdout
		.split(|byte| *byte == 0)
		.filter(|path| !path.is_empty())
		.map(|path| {
			String::from_utf8(path.to_vec()).map_err(|_| {
				PackError::NotUtf8 {
					path: PathBuf::from(String::from_utf8_lossy(path).into_owned()),
				}
			})
		})
		.collect::<Result<_, _>>()
		.map(Some)
}

/// The files a walk finds, honouring `.gitignore` files below the root and
/// skipping [`super::ALWAYS_EXCLUDED`] without descending into them.
fn walked_files(root: &Path) -> Result<Vec<String>, PackError> {
	let mut files = Vec::new();
	let walk = ignore::WalkBuilder::new(root)
		.hidden(false)
		.parents(false)
		.ignore(false)
		.git_ignore(true)
		.git_global(false)
		.git_exclude(false)
		.require_git(false)
		.follow_links(false)
		.filter_entry(|entry| {
			entry.depth() == 0
				|| entry
					.file_name()
					.to_str()
					.is_none_or(|name| !super::ALWAYS_EXCLUDED.contains(&name))
		})
		.build();

	for entry in walk {
		let entry = entry?;

		if entry.depth() == 0 || entry.file_type().is_some_and(|kind| kind.is_dir()) {
			continue;
		}

		let relative = entry.path().strip_prefix(root).map_err(|_| {
			PackError::Path {
				path: entry.path().display().to_string(),
				reason: "is outside the project",
			}
		})?;
		files.push(archive_path(relative)?);
	}

	Ok(files)
}

/// A relative filesystem path as an archive path: UTF-8, `/`-separated.
fn archive_path(relative: &Path) -> Result<String, PackError> {
	let mut path = String::new();

	for component in relative.components() {
		let std::path::Component::Normal(name) = component else {
			return Err(PackError::Path {
				path: relative.display().to_string(),
				reason: "isn't a plain relative path",
			});
		};
		let Some(name) = name.to_str() else {
			return Err(PackError::NotUtf8 {
				path: relative.to_owned(),
			});
		};

		if !path.is_empty() {
			path.push('/');
		}

		path.push_str(name);
	}

	Ok(path)
}

/// The listed paths that exist, with their metadata, not following
/// symlinks. Git still lists a file deleted from the working tree until the
/// deletion is staged; the archive is the working tree, so it is skipped.
fn present_files(
	root: &Path,
	listed: Vec<String>,
) -> Result<Vec<(String, std::fs::Metadata)>, PackError> {
	let mut present = Vec::with_capacity(listed.len());

	for path in listed {
		let full = root.join(&path);

		match std::fs::symlink_metadata(&full) {
			Ok(metadata) => present.push((path, metadata)),
			Err(error) if error.kind() == io::ErrorKind::NotFound => {}
			Err(source) => return Err(PackError::Io { path: full, source }),
		}
	}

	Ok(present)
}

/// Reads each present path, holding the size bounds.
fn read_entries(
	root: &Path,
	present: Vec<(String, std::fs::Metadata)>,
	limits: Limits,
) -> Result<Vec<(String, Entry)>, PackError> {
	let mut entries = Vec::with_capacity(present.len());
	let mut total: u64 = 0;

	for (path, metadata) in present {
		let full = root.join(&path);
		let io_error = |source| {
			PackError::Io {
				path: full.clone(),
				source,
			}
		};
		let kind = metadata.file_type();

		if kind.is_symlink() {
			let target = std::fs::read_link(&full).map_err(io_error)?;
			let target = target_string(&target).ok_or_else(|| {
				PackError::NotUtf8 {
					path: PathBuf::from(&path),
				}
			})?;
			entries.push((path, Entry::Link { target }));
			continue;
		}

		if kind.is_dir() {
			return Err(PackError::Submodule(path));
		}

		if !kind.is_file() {
			return Err(PackError::Unsupported(path));
		}

		let too_large = |bytes| {
			PackError::FileTooLarge {
				path: path.clone(),
				bytes,
				limit: limits.max_file_bytes,
			}
		};

		if metadata.len() > limits.max_file_bytes {
			return Err(too_large(metadata.len()));
		}

		// Read the whole file and size the entry from what was read, so a
		// file that changes while packing can't disagree with its header.
		let data = std::fs::read(&full).map_err(io_error)?;
		let bytes = data.len() as u64;

		if bytes > limits.max_file_bytes {
			return Err(too_large(bytes));
		}

		total = total.saturating_add(bytes);

		if total > limits.max_bytes {
			return Err(PackError::TooLarge {
				limit: limits.max_bytes,
			});
		}

		entries.push((
			path,
			Entry::File {
				data,
				executable: executable(&metadata),
			},
		));
	}

	Ok(entries)
}

/// A symlink's target in `/` form, or `None` when it isn't UTF-8.
fn target_string(target: &Path) -> Option<String> {
	let target = target.to_str()?;

	if cfg!(windows) {
		Some(target.replace('\\', "/"))
	} else {
		Some(target.to_owned())
	}
}

#[cfg(unix)]
fn executable(metadata: &std::fs::Metadata) -> bool {
	use std::os::unix::fs::PermissionsExt;

	metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn executable(_: &std::fs::Metadata) -> bool {
	false
}

/// Writes the entries, already sorted, as a deterministic gzip-compressed
/// tar.
fn write_archive(entries: Vec<(String, Entry)>) -> Result<Packed, PackError> {
	let encoder = GzBuilder::new()
		.mtime(0)
		.operating_system(255)
		.write(Vec::new(), Compression::default());
	let mut builder = tar::Builder::new(encoder);
	let mut files = Vec::with_capacity(entries.len());
	let mut uncompressed_bytes: u64 = 0;

	for (path, entry) in entries {
		let mut header = tar::Header::new_gnu();
		header.set_mtime(0);
		header.set_uid(0);
		header.set_gid(0);

		match entry {
			Entry::File { data, executable } => {
				header.set_entry_type(tar::EntryType::Regular);
				header.set_mode(if executable { 0o755 } else { 0o644 });
				header.set_size(data.len() as u64);
				uncompressed_bytes += data.len() as u64;
				builder
					.append_data(&mut header, &path, data.as_slice())
					.map_err(PackError::Write)?;
			}
			Entry::Link { target } => {
				header.set_entry_type(tar::EntryType::Symlink);
				header.set_mode(0o644);
				header.set_size(0);
				builder
					.append_link(&mut header, &path, &target)
					.map_err(PackError::Write)?;
			}
		}

		files.push(path);
	}

	let bytes = builder
		.into_inner()
		.and_then(flate2::write::GzEncoder::finish)
		.map_err(PackError::Write)?;

	Ok(Packed {
		digest: Digest::of(&bytes),
		bytes,
		files,
		uncompressed_bytes,
	})
}
