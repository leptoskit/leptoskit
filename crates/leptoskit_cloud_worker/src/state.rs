//! What a worker keeps on disk: who it is, the job it is running, and the
//! outcome it has yet to report.
//!
//! The directory is the worker's alone (`0700`), its files `0600`, and
//! every write lands whole: a temporary file, synced, renamed over the old
//! one, and the directory synced. One worker runs from a directory at a
//! time, held by a lock on `lock`.

use std::fs;
use std::io;
use std::io::Write as _;
use std::os::unix::fs::MetadataExt as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::path::PathBuf;

use leptoskit_cloud::workers::JobKind;
use leptoskit_cloud::workers::JobOutcome;
use leptoskit_cloud::workers::WorkerRole;
use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;
use uuid::Uuid;

const IDENTITY: &str = "worker.json";
const MARKER: &str = "job.json";
const RESULT: &str = "result.json";
const LOCK: &str = "lock";

/// The most of a state file that is read.
const MAX_FILE: u64 = 4 * 1024 * 1024;

/// Why the state directory couldn't be used.
#[derive(Debug, thiserror::Error)]
pub enum StateError {
	#[error("couldn't use the state directory {}: {source}", path.display())]
	Io {
		path: PathBuf,
		#[source]
		source: io::Error,
	},
	#[error("{} isn't what this worker wrote; move it away to start again", path.display())]
	Unreadable { path: PathBuf },
	#[error(
		"{} must be a directory of this user's, not a link and not one another user can write",
		.0.display()
	)]
	NotPrivate(PathBuf),
	#[error("another worker is running from {}", .0.display())]
	Locked(PathBuf),
	#[error("{} already holds a paired worker; use another directory, or remove it", .0.display())]
	AlreadyPaired(PathBuf),
	#[error("no worker is paired in {}; run `leptoskit-cloud-worker pair` first", .0.display())]
	NotPaired(PathBuf),
}

/// Who the worker is to its control plane.
#[derive(Clone, Serialize, Deserialize)]
pub struct Identity {
	/// The control plane's origin.
	pub origin: String,
	/// The path its operations are under.
	pub rpc: String,
	/// The worker's key.
	pub key: String,
	pub worker: Uuid,
	pub name: String,
	pub role: WorkerRole,
	/// The capabilities it declared when it paired.
	#[serde(default)]
	pub capabilities: Vec<String>,
}

impl std::fmt::Debug for Identity {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_struct("Identity")
			.field("origin", &self.origin)
			.field("key", &"[redacted]")
			.field("worker", &self.worker)
			.field("name", &self.name)
			.field("role", &self.role)
			.finish_non_exhaustive()
	}
}

/// The job the worker started, written before it runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Marker {
	pub job: Uuid,
	pub attempt: u32,
	pub kind: JobKind,
}

/// How a job ended, written before the control plane is told.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Saved {
	pub job: Uuid,
	pub attempt: u32,
	pub outcome: JobOutcome,
}

/// A worker's state directory, locked for as long as this is held.
#[derive(Debug)]
pub struct State {
	directory: PathBuf,
	_lock: fs::File,
}

impl State {
	/// Opens `directory`, making it if need be, and locks it.
	///
	/// # Errors
	///
	/// [`StateError::Locked`] while another worker holds it, or an I/O
	/// failure.
	pub fn open(directory: &Path) -> Result<Self, StateError> {
		let io = |source| {
			StateError::Io {
				path: directory.to_owned(),
				source,
			}
		};
		private_directory(directory)?;
		let lock = private(&directory.join(LOCK), false).map_err(io)?;
		rustix::fs::flock(&lock, rustix::fs::FlockOperation::NonBlockingLockExclusive)
			.map_err(|_| StateError::Locked(directory.to_owned()))?;
		Ok(Self {
			directory: directory.to_owned(),
			_lock: lock,
		})
	}

	/// The directory.
	#[must_use]
	pub fn directory(&self) -> &Path {
		&self.directory
	}

	fn read<T: DeserializeOwned>(&self, name: &str) -> Result<Option<T>, StateError> {
		let path = self.directory.join(name);
		let io = |source| {
			StateError::Io {
				path: path.clone(),
				source,
			}
		};
		let length = match fs::metadata(&path) {
			Ok(metadata) => metadata.len(),
			Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
			Err(error) => return Err(io(error)),
		};

		if length > MAX_FILE {
			return Err(StateError::Unreadable { path });
		}

		let bytes = fs::read(&path).map_err(io)?;
		serde_json::from_slice(&bytes)
			.map(Some)
			.map_err(|_| StateError::Unreadable { path })
	}

	fn write<T: Serialize>(&self, name: &str, value: &T) -> Result<(), StateError> {
		let path = self.directory.join(name);
		let io = |source| {
			StateError::Io {
				path: path.clone(),
				source,
			}
		};
		let bytes = serde_json::to_vec_pretty(value)
			.map_err(|error| io(io::Error::new(io::ErrorKind::InvalidData, error)))?;
		let temporary = self.directory.join(format!("{name}.new"));
		// What a worker that died while writing left, or anything else:
		// the new file is made afresh, never written through.
		match fs::remove_file(&temporary) {
			Ok(()) => {}
			Err(error) if error.kind() == io::ErrorKind::NotFound => {}
			Err(error) => return Err(io(error)),
		}
		let mut file = private(&temporary, true).map_err(io)?;
		file.write_all(&bytes).map_err(io)?;
		file.sync_all().map_err(io)?;
		fs::rename(&temporary, &path).map_err(io)?;
		fs::File::open(&self.directory)
			.and_then(|directory| directory.sync_all())
			.map_err(io)
	}

	fn remove(&self, name: &str) -> Result<(), StateError> {
		let path = self.directory.join(name);

		match fs::remove_file(&path) {
			Ok(()) => {}
			Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
			Err(source) => return Err(StateError::Io { path, source }),
		}

		fs::File::open(&self.directory)
			.and_then(|directory| directory.sync_all())
			.map_err(|source| StateError::Io { path, source })
	}

	/// Who the worker is.
	///
	/// # Errors
	///
	/// [`StateError::NotPaired`] before `pair`.
	pub fn identity(&self) -> Result<Identity, StateError> {
		self.read(IDENTITY)?
			.ok_or_else(|| StateError::NotPaired(self.directory.clone()))
	}

	/// Keeps who the worker is. A directory holds one worker for good.
	///
	/// # Errors
	///
	/// [`StateError::AlreadyPaired`] when it already holds one.
	pub fn pair(&self, identity: &Identity) -> Result<(), StateError> {
		self.ensure_unpaired()?;
		self.write(IDENTITY, identity)
	}

	/// Checks nothing is paired here yet, before a pairing code is spent.
	///
	/// # Errors
	///
	/// [`StateError::AlreadyPaired`] when a worker is.
	pub fn ensure_unpaired(&self) -> Result<(), StateError> {
		if self.directory.join(IDENTITY).exists() {
			return Err(StateError::AlreadyPaired(self.directory.clone()));
		}

		Ok(())
	}

	/// The job that was started and not yet cleared.
	///
	/// # Errors
	///
	/// When the file can't be read.
	pub fn marker(&self) -> Result<Option<Marker>, StateError> {
		self.read(MARKER)
	}

	/// Notes that `marker`'s job is about to run.
	///
	/// # Errors
	///
	/// When it can't be written: the job must not run then.
	pub fn start(&self, marker: &Marker) -> Result<(), StateError> {
		self.write(MARKER, marker)
	}

	/// The outcome that was saved and not yet acknowledged.
	///
	/// # Errors
	///
	/// When the file can't be read.
	pub fn saved(&self) -> Result<Option<Saved>, StateError> {
		self.read(RESULT)
	}

	/// Keeps `saved` until the control plane has it.
	///
	/// # Errors
	///
	/// When it can't be written.
	pub fn save(&self, saved: &Saved) -> Result<(), StateError> {
		self.write(RESULT, saved)
	}

	/// Forgets the job: its outcome first, then its marker, so a crash
	/// between the two leaves a marker, never an outcome without one.
	///
	/// # Errors
	///
	/// When a file can't be removed.
	pub fn clear(&self) -> Result<(), StateError> {
		self.remove(RESULT)?;
		self.remove(MARKER)
	}
}

/// Opens `path` for writing, readable by its owner only, and never
/// through a link. With `fresh`, the file must not be there yet.
fn private(path: &Path, fresh: bool) -> io::Result<fs::File> {
	let mut options = fs::OpenOptions::new();
	options
		.write(true)
		.mode(0o600)
		.custom_flags(rustix::fs::OFlags::NOFOLLOW.bits().cast_signed());

	if fresh {
		options.create_new(true);
	} else {
		options.create(true).truncate(false);
	}

	options.open(path)
}

/// Makes `directory` if need be and keeps it to this user: it must be a
/// directory, not a link to one, owned by this user, and no one else may
/// read or write it.
///
/// # Errors
///
/// [`StateError::NotPrivate`] for a directory that isn't this user's, or
/// an I/O failure.
pub(crate) fn private_directory(directory: &Path) -> Result<(), StateError> {
	let io = |source| {
		StateError::Io {
			path: directory.to_owned(),
			source,
		}
	};
	fs::create_dir_all(directory).map_err(io)?;
	let metadata = fs::symlink_metadata(directory).map_err(io)?;

	if !metadata.is_dir() || metadata.uid() != rustix::process::geteuid().as_raw() {
		return Err(StateError::NotPrivate(directory.to_owned()));
	}

	fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).map_err(io)
}

#[cfg(test)]
mod tests {
	use leptoskit_cloud::workers::BuildOutcome;

	use super::*;

	fn identity() -> Identity {
		Identity {
			origin: "https://cloud.example.com".to_owned(),
			rpc: "/rpc".to_owned(),
			key: "lkw_example".to_owned(),
			worker: Uuid::nil(),
			name: "builder-1".to_owned(),
			role: WorkerRole::Build,
			capabilities: Vec::new(),
		}
	}

	#[test]
	fn a_directory_holds_one_worker_and_only_its_owner_reads_it() {
		let directory = tempfile::tempdir().expect("a directory");
		let path = directory.path().join("state");
		let state = State::open(&path).expect("opens");
		assert!(matches!(state.identity(), Err(StateError::NotPaired(_))));
		state.pair(&identity()).expect("pairs");
		assert_eq!(state.identity().expect("an identity").name, "builder-1");
		assert!(matches!(
			state.pair(&identity()),
			Err(StateError::AlreadyPaired(_))
		));
		assert!(!format!("{:?}", state.identity()).contains("secret"));

		let mode = |path: &Path| fs::metadata(path).expect("metadata").permissions().mode() & 0o777;
		assert_eq!(mode(&path), 0o700);
		assert_eq!(mode(&path.join(IDENTITY)), 0o600);

		// A second worker can't run from the same directory.
		assert!(matches!(State::open(&path), Err(StateError::Locked(_))));
		drop(state);
		State::open(&path).expect("opens once the first is gone");
	}

	#[test]
	fn a_job_is_marked_saved_and_cleared() {
		let directory = tempfile::tempdir().expect("a directory");
		let state = State::open(directory.path()).expect("opens");
		assert_eq!(state.marker().expect("reads"), None);
		let marker = Marker {
			job: Uuid::from_u128(7),
			attempt: 2,
			kind: JobKind::BuildServer,
		};
		state.start(&marker).expect("marks");
		assert_eq!(state.marker().expect("reads"), Some(marker));
		assert_eq!(state.saved().expect("reads"), None);
		let saved = Saved {
			job: marker.job,
			attempt: marker.attempt,
			outcome: JobOutcome::BuildServer(BuildOutcome::Stopped),
		};
		state.save(&saved).expect("saves");
		assert_eq!(state.saved().expect("reads"), Some(saved));
		state.clear().expect("clears");
		assert_eq!(state.marker().expect("reads"), None);
		assert_eq!(state.saved().expect("reads"), None);
		state.clear().expect("clearing nothing is fine");
		// Nothing temporary is left behind.
		let left: Vec<_> = fs::read_dir(directory.path())
			.expect("lists")
			.map(|entry| entry.expect("an entry").file_name())
			.collect();
		assert_eq!(left, [LOCK]);
	}

	#[test]
	fn a_file_that_is_not_ours_is_not_guessed_at() {
		let directory = tempfile::tempdir().expect("a directory");
		let state = State::open(directory.path()).expect("opens");
		fs::write(directory.path().join(MARKER), b"{ not json").expect("written");
		assert!(matches!(state.marker(), Err(StateError::Unreadable { .. })));
	}

	#[test]
	fn a_state_directory_must_be_a_directory_of_this_users() {
		let root = tempfile::tempdir().expect("a directory");
		let real = root.path().join("real");
		fs::create_dir(&real).expect("a directory");
		let link = root.path().join("link");
		std::os::unix::fs::symlink(&real, &link).expect("a link");
		assert!(matches!(State::open(&link), Err(StateError::NotPrivate(_))));

		// A file left where a new one is written is replaced, not written
		// through.
		let path = root.path().join("state");
		let state = State::open(&path).expect("opens");
		let elsewhere = root.path().join("elsewhere");
		fs::write(&elsewhere, "untouched").expect("a file");
		std::os::unix::fs::symlink(&elsewhere, path.join(format!("{IDENTITY}.new")))
			.expect("a link");
		state.pair(&identity()).expect("pairs");
		assert_eq!(fs::read_to_string(&elsewhere).expect("reads"), "untouched");
		assert!(
			!fs::symlink_metadata(path.join(IDENTITY))
				.expect("written")
				.is_symlink()
		);
	}
}
