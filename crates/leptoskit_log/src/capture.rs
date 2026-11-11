//! Records kept in memory instead of written, so tests can read what the
//! runtime logged.
//!
//! A test binary has one logger, shared by its tests, which may run at once.
//! Every test therefore reads the same records, and looks for its own by
//! something only it logs: a plugin name, a node id, a payload.
//!
//! ```
//! let records = leptoskit_log::capture::start();
//! log::warn!(target: "leptoskit/plugin/doc-example", "call #1 failed");
//! assert!(records.find("doc-example", "call #1").is_some());
//! ```

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use log::Level;
use log::Record;

/// The most records kept; the oldest go first.
const MAX_RECORDS: usize = 10_000;

static CAPTURING: AtomicBool = AtomicBool::new(false);
static RECORDS: Mutex<VecDeque<Captured>> = Mutex::new(VecDeque::new());

/// One record as it was logged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Captured {
	/// Its level.
	pub level: Level,
	/// Its target, such as `leptoskit/plugin/http`.
	pub target: String,
	/// Its formatted message.
	pub message: String,
}

/// Reads the records kept since [`start`].
#[derive(Debug, Clone, Copy)]
pub struct Records;

/// Installs leptoskit's logger at `trace` and keeps every record in memory
/// rather than writing it.
pub fn start() -> Records {
	crate::install();
	CAPTURING.store(true, Ordering::Release);
	log::set_max_level(log::LevelFilter::Trace);
	Records
}

/// Keeps `record` when capturing, and says whether it did.
pub(crate) fn keep(record: &Record<'_>) -> bool {
	if !CAPTURING.load(Ordering::Acquire) {
		return false;
	}

	let mut records = RECORDS.lock().unwrap_or_else(PoisonError::into_inner);

	if records.len() == MAX_RECORDS {
		records.pop_front();
	}

	records.push_back(Captured {
		level: record.level(),
		target: record.target().to_owned(),
		message: record.args().to_string(),
	});
	true
}

impl Records {
	/// Every record kept so far, oldest first.
	pub fn all(self) -> Vec<Captured> {
		RECORDS
			.lock()
			.unwrap_or_else(PoisonError::into_inner)
			.iter()
			.cloned()
			.collect()
	}

	/// The newest record whose target contains `target` and whose message
	/// contains `text`.
	pub fn find(self, target: &str, text: &str) -> Option<Captured> {
		RECORDS
			.lock()
			.unwrap_or_else(PoisonError::into_inner)
			.iter()
			.rev()
			.find(|record| record.target.contains(target) && record.message.contains(text))
			.cloned()
	}

	/// Like [`Self::find`], but fails the test with every record kept when
	/// there is none.
	#[track_caller]
	pub fn expect(self, target: &str, text: &str) -> Captured {
		self.find(target, text).unwrap_or_else(|| {
			let all: Vec<String> = self
				.all()
				.iter()
				.map(|record| format!("{} {}: {}", record.level, record.target, record.message))
				.collect();
			panic!(
				"no record under `{target}` says `{text}`; the records are:\n{}",
				all.join("\n")
			)
		})
	}
}
