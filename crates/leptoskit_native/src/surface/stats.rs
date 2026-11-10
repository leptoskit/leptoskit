//! Opt-in commit cost accounting, for frame logs and headless tooling.

use std::time::Duration;

/// What the surface's commits cost since [`take_commit_stats`] last read
/// them.
///
/// Durations stay zero on `wasm32`, which has no monotonic clock in the
/// standard library; counts are always kept.
///
/// [`take_commit_stats`]: super::NativeSurface::take_commit_stats
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CommitStats {
	/// Commits made, including the extra pass that releases a held control.
	pub commits: u32,
	/// Mutations sent to the backend, frame updates included.
	pub mutations: usize,
	/// Time spent resolving classes, diffing, and applying mutations.
	pub reconcile: Duration,
	/// Time spent computing layout and pushing changed frames.
	pub layout: Duration,
}

/// A stopwatch that runs only while stats are recorded.
pub(super) struct Timer {
	#[cfg(not(target_arch = "wasm32"))]
	started: Option<std::time::Instant>,
}

impl Timer {
	pub(super) fn start(recording: bool) -> Self {
		#[cfg(not(target_arch = "wasm32"))]
		{
			Self {
				started: recording.then(std::time::Instant::now),
			}
		}
		#[cfg(target_arch = "wasm32")]
		{
			let _ = recording;
			Self {}
		}
	}

	pub(super) fn elapsed(&self) -> Duration {
		#[cfg(not(target_arch = "wasm32"))]
		{
			self.started
				.map_or(Duration::ZERO, |started| started.elapsed())
		}
		#[cfg(target_arch = "wasm32")]
		{
			Duration::ZERO
		}
	}
}
