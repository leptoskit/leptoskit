//! A queue checkpoint is committed only after every new segment is on disk.
//! Two banks keep the previous checkpoint intact if a write or commit fails.

use std::cell::Cell;
use std::cell::RefCell;
use std::rc::Rc;
use std::task::Poll;
use std::task::Waker;

pub(super) const DIRECTORY: &str = "leptoskit_analytics";
const MANIFEST: &str = "leptoskit_analytics/queue-manifest";
const MAX_FILES: usize = 1_024;

pub(super) trait Store {
	async fn mkdir(&self, path: &str) -> Result<(), ()>;
	async fn list(&self, path: &str) -> Result<Vec<String>, ()>;
	async fn read(&self, path: &str) -> Result<Vec<u8>, ()>;
	/// Replaces a file atomically, as the filesystem plugin guarantees.
	async fn write(&self, path: &str, bytes: &[u8]) -> Result<(), ()>;
	async fn remove(&self, path: &str) -> Result<(), ()>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Checkpoint {
	bank: u8,
	files: usize,
}

impl Checkpoint {
	fn path(self, index: usize) -> String {
		format!("{DIRECTORY}/queue-{}-{index}", self.bank)
	}

	fn encode(self) -> Vec<u8> {
		format!("1:{}:{}", self.bank, self.files).into_bytes()
	}

	fn decode(bytes: &[u8]) -> Result<Self, ()> {
		let text = std::str::from_utf8(bytes).map_err(|_| ())?;
		let mut parts = text.split(':');
		if parts.next() != Some("1") {
			return Err(());
		}
		let bank = parts.next().ok_or(())?.parse::<u8>().map_err(|_| ())?;
		let files = parts.next().ok_or(())?.parse::<usize>().map_err(|_| ())?;
		if bank > 1 || files > MAX_FILES || parts.next().is_some() {
			return Err(());
		}
		Ok(Self { bank, files })
	}
}

/// Reads the whole committed checkpoint, or refuses it without changing disk.
/// Before the first checkpoint, the original sequential files are still read.
pub(super) async fn load(store: &impl Store) -> Result<(Option<Checkpoint>, Vec<Vec<u8>>), ()> {
	store.mkdir(DIRECTORY).await?;
	let names = store.list(DIRECTORY).await?;
	let checkpoint = if names.iter().any(|name| name == "queue-manifest") {
		Some(Checkpoint::decode(&store.read(MANIFEST).await?)?)
	} else {
		None
	};
	let paths = if let Some(checkpoint) = checkpoint {
		(0..checkpoint.files)
			.map(|index| checkpoint.path(index))
			.collect::<Vec<_>>()
	} else {
		let mut indices: Vec<usize> = names
			.iter()
			.filter_map(|name| name.strip_prefix("queue-")?.parse().ok())
			.collect();
		indices.sort_unstable();
		if indices.len() > MAX_FILES || indices.iter().copied().ne(0..indices.len()) {
			return Err(());
		}
		indices
			.into_iter()
			.map(|index| format!("{DIRECTORY}/queue-{index}"))
			.collect()
	};
	let mut segments = Vec::with_capacity(paths.len());
	for path in paths {
		segments.push(store.read(&path).await?);
	}
	Ok((checkpoint, segments))
}

/// Writes the inactive bank, then atomically commits its manifest. Cleanup is
/// best effort and happens only after the new checkpoint can be recovered.
/// The caller serializes saves and advances its saved version only on success.
pub(super) async fn save(
	store: &impl Store,
	previous: Option<Checkpoint>,
	segments: &[Vec<u8>],
) -> Result<Checkpoint, ()> {
	if segments.len() > MAX_FILES {
		return Err(());
	}
	store.mkdir(DIRECTORY).await?;
	// A commit may have reached disk even if its caller did not observe the
	// acknowledgement. Choose the inactive bank from disk before each save.
	let names = store.list(DIRECTORY).await?;
	let previous = if names.iter().any(|name| name == "queue-manifest") {
		Some(Checkpoint::decode(&store.read(MANIFEST).await?)?)
	} else {
		previous
	};
	let next = Checkpoint {
		bank: previous.map_or(0, |checkpoint| checkpoint.bank ^ 1),
		files: segments.len(),
	};
	for (index, segment) in segments.iter().enumerate() {
		store.write(&next.path(index), segment).await?;
	}
	store.write(MANIFEST, &next.encode()).await?;
	if let Ok(names) = store.list(DIRECTORY).await {
		for name in names {
			if name == "queue-manifest" || !name.starts_with("queue-") {
				continue;
			}
			let path = format!("{DIRECTORY}/{name}");
			if !(0..next.files).any(|index| path == next.path(index)) {
				let _ = store.remove(&path).await;
			}
		}
	}
	Ok(next)
}

/// Saves may be started by the interval, flush, and consent changes. Hold the
/// gate across I/O so none can overwrite another save's inactive bank.
#[derive(Default)]
pub(super) struct Gate {
	busy: Cell<bool>,
	waiters: RefCell<Vec<Waker>>,
}

impl Gate {
	pub async fn lock(self: &Rc<Self>) -> Guard {
		std::future::poll_fn(|cx| {
			if !self.busy.replace(true) {
				return Poll::Ready(());
			}
			let mut waiters = self.waiters.borrow_mut();
			if !waiters.iter().any(|waker| waker.will_wake(cx.waker())) {
				waiters.push(cx.waker().clone());
			}
			Poll::Pending
		})
		.await;
		Guard(self.clone())
	}
}

pub(super) struct Guard(Rc<Gate>);

impl Drop for Guard {
	fn drop(&mut self) {
		self.0.busy.set(false);
		let waiters = std::mem::take(&mut *self.0.waiters.borrow_mut());
		for waker in waiters {
			waker.wake();
		}
	}
}

#[cfg(test)]
mod tests {
	use std::collections::BTreeMap;

	use super::*;

	#[derive(Default)]
	struct Disk {
		files: RefCell<BTreeMap<String, Vec<u8>>>,
		writes: Cell<usize>,
		fail_write: Cell<Option<usize>>,
		fail_cleanup: Cell<bool>,
	}

	impl Store for Disk {
		fn mkdir(&self, _: &str) -> impl Future<Output = Result<(), ()>> {
			std::future::ready(Ok(()))
		}

		fn list(&self, _: &str) -> impl Future<Output = Result<Vec<String>, ()>> {
			std::future::ready(Ok(self
				.files
				.borrow()
				.keys()
				.map(|path| {
					path.strip_prefix("leptoskit_analytics/")
						.unwrap()
						.to_owned()
				})
				.collect()))
		}

		fn read(&self, path: &str) -> impl Future<Output = Result<Vec<u8>, ()>> {
			std::future::ready(self.files.borrow().get(path).cloned().ok_or(()))
		}

		fn write(&self, path: &str, bytes: &[u8]) -> impl Future<Output = Result<(), ()>> {
			let write = self.writes.get();
			self.writes.set(write + 1);
			if self.fail_write.get() == Some(write) {
				return std::future::ready(Err(()));
			}
			self.files.borrow_mut().insert(path.into(), bytes.to_vec());
			std::future::ready(Ok(()))
		}

		fn remove(&self, path: &str) -> impl Future<Output = Result<(), ()>> {
			if self.fail_cleanup.get() {
				return std::future::ready(Err(()));
			}
			self.files.borrow_mut().remove(path);
			std::future::ready(Ok(()))
		}
	}

	fn segments(values: &[&str]) -> Vec<Vec<u8>> {
		values
			.iter()
			.map(|value| value.as_bytes().to_vec())
			.collect()
	}

	#[tokio::test]
	async fn every_failed_segment_or_manifest_write_preserves_the_previous_checkpoint() {
		for failed in 0..=3 {
			let disk = Disk::default();
			let old = segments(&["old-first", "old-second"]);
			let next = segments(&["new-first", "new-second", "new-third"]);
			let checkpoint = save(&disk, None, &old).await.unwrap();
			disk.fail_write.set(Some(disk.writes.get() + failed));
			assert!(save(&disk, Some(checkpoint), &next).await.is_err());
			assert_eq!(load(&disk).await.unwrap(), (Some(checkpoint), old));
			disk.fail_write.set(None);
			let committed = save(&disk, Some(checkpoint), &next).await.unwrap();
			assert_eq!(load(&disk).await.unwrap(), (Some(committed), next));
		}
	}

	#[tokio::test]
	async fn an_unacknowledged_commit_is_still_protected_by_the_next_save() {
		let disk = Disk::default();
		let stale = save(&disk, None, &segments(&["first"])).await.unwrap();
		let committed = save(&disk, Some(stale), &segments(&["second", "second-tail"]))
			.await
			.unwrap();
		disk.fail_write.set(Some(disk.writes.get() + 1));
		assert!(
			save(&disk, Some(stale), &segments(&["third", "third-tail"]))
				.await
				.is_err()
		);
		assert_eq!(
			load(&disk).await.unwrap(),
			(Some(committed), segments(&["second", "second-tail"]))
		);
	}

	#[tokio::test]
	async fn legacy_files_survive_failed_migration_and_are_removed_only_after_commit() {
		let disk = Disk::default();
		disk.files
			.borrow_mut()
			.insert(format!("{DIRECTORY}/queue-0"), b"legacy".to_vec());
		disk.fail_write.set(Some(1));
		assert!(
			save(&disk, None, &segments(&["replacement"]))
				.await
				.is_err()
		);
		assert_eq!(load(&disk).await.unwrap(), (None, segments(&["legacy"])));
		disk.fail_write.set(None);
		let checkpoint = save(&disk, None, &segments(&["replacement"]))
			.await
			.unwrap();
		assert_eq!(
			load(&disk).await.unwrap(),
			(Some(checkpoint), segments(&["replacement"]))
		);
		assert!(
			!disk
				.files
				.borrow()
				.contains_key(&format!("{DIRECTORY}/queue-0"))
		);
	}

	#[tokio::test]
	async fn failed_cleanup_keeps_the_new_checkpoint_and_empty_commits_do_not_resurrect_old_events()
	{
		let disk = Disk::default();
		let old = save(&disk, None, &segments(&["old"])).await.unwrap();
		disk.fail_cleanup.set(true);
		let empty = save(&disk, Some(old), &[]).await.unwrap();
		assert_eq!(load(&disk).await.unwrap(), (Some(empty), vec![]));
		assert!(disk.files.borrow().contains_key(&old.path(0)));
	}

	#[tokio::test]
	async fn incomplete_or_invalid_checkpoints_are_refused_without_rewriting_disk() {
		for manifest in ["1:0:2", "1:2:0", "1:0:1025", "invalid"] {
			let disk = Disk::default();
			disk.files
				.borrow_mut()
				.insert(MANIFEST.into(), manifest.as_bytes().to_vec());
			assert!(load(&disk).await.is_err());
			assert_eq!(disk.writes.get(), 0);
		}
	}

	#[tokio::test]
	async fn concurrent_saves_wait_for_the_first_commit() {
		let gate = Rc::new(Gate::default());
		let order = RefCell::new(Vec::new());
		let first = async {
			let _guard = gate.lock().await;
			order.borrow_mut().push(1);
			tokio::task::yield_now().await;
			order.borrow_mut().push(2);
		};
		let second = async {
			let _guard = gate.lock().await;
			order.borrow_mut().push(3);
		};
		tokio::join!(first, second);
		assert_eq!(*order.borrow(), [1, 2, 3]);
	}
}
