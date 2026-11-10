//! Per-commit cost records and the opt-in frame log.
//!
//! Every guest commit produces a [`FrameStats`]. Setting
//! [`FRAME_LOG_ENV`] before a host is created also writes each one as a JSON
//! line, to standard error for `stderr` or `-`, otherwise appended to the file
//! the variable names.

use std::io::Write;
use std::time::Duration;

/// The environment variable that turns the frame log on.
pub const FRAME_LOG_ENV: &str = "LEPTOSKIT_FRAME_LOG";

/// The guest entry point whose work produced a commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Entry {
	/// `leptoskit_main`, the initial render.
	Main,
	/// `leptoskit_event`, an event's rebuild.
	Event,
	/// `leptoskit_viewport`, a viewport change the guest heard.
	Viewport,
	/// `leptoskit_plugins_ready`, plugin replies or async work.
	Plugins,
}

/// What one guest commit cost. Durations are zero on `wasm32`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct FrameStats {
	/// The host's commit count after this commit.
	pub commit: u32,
	/// The entry point that committed.
	pub entry: Entry,
	/// Milliseconds since the host was created.
	pub at_ms: u64,
	/// Microseconds in the guest entry point, including the host calls it
	/// made to build and assemble the tree.
	pub guest_us: u64,
	/// Microseconds the host spent resolving classes, diffing the tree, and
	/// applying mutations.
	pub commit_us: u64,
	/// Microseconds the host spent laying out and pushing frames.
	pub layout_us: u64,
	/// Mutations sent to the backend, frame updates included.
	pub mutations: usize,
	/// Nodes in the committed tree.
	pub nodes: usize,
}

/// A monotonic stopwatch that reads zero where the standard library has no
/// clock.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Stopwatch {
	#[cfg(not(target_arch = "wasm32"))]
	started: std::time::Instant,
}

impl Stopwatch {
	pub(crate) fn start() -> Self {
		Self {
			#[cfg(not(target_arch = "wasm32"))]
			started: std::time::Instant::now(),
		}
	}

	pub(crate) fn elapsed(self) -> Duration {
		#[cfg(not(target_arch = "wasm32"))]
		{
			self.started.elapsed()
		}
		#[cfg(target_arch = "wasm32")]
		{
			Duration::ZERO
		}
	}
}

/// The last commit's record and where the log goes.
pub(crate) struct Recorder {
	created: Stopwatch,
	last: Option<FrameStats>,
	sink: Option<Box<dyn Write + Send>>,
}

impl Recorder {
	/// A recorder that logs where [`FRAME_LOG_ENV`] says, if it is set.
	pub(crate) fn from_env() -> Self {
		Self {
			created: Stopwatch::start(),
			last: None,
			sink: std::env::var_os(FRAME_LOG_ENV).and_then(|target| open(&target)),
		}
	}

	pub(crate) fn last(&self) -> Option<&FrameStats> {
		self.last.as_ref()
	}

	pub(crate) fn set_sink(&mut self, sink: Option<Box<dyn Write + Send>>) {
		self.sink = sink;
	}

	pub(crate) fn since_created(&self) -> u64 {
		micros(self.created.elapsed()) / 1_000
	}

	/// Keeps a commit's record and logs it. A sink that fails is dropped, so a
	/// full disk stops the log rather than the app.
	pub(crate) fn record(&mut self, stats: FrameStats) {
		if let Some(sink) = &mut self.sink {
			let line = serde_json::to_string(&stats).expect("frame stats serialize");

			if writeln!(sink, "{line}")
				.and_then(|()| sink.flush())
				.is_err()
			{
				self.sink = None;
			}
		}

		self.last = Some(stats);
	}
}

/// Opens the log target an environment value names.
fn open(target: &std::ffi::OsStr) -> Option<Box<dyn Write + Send>> {
	if target.is_empty() {
		return None;
	}

	if target == "stderr" || target == "-" {
		return Some(Box::new(std::io::stderr()));
	}

	let file = std::fs::OpenOptions::new()
		.create(true)
		.append(true)
		.open(target)
		.ok()?;
	Some(Box::new(file))
}

pub(crate) fn micros(duration: Duration) -> u64 {
	u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}
