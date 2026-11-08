//! The seam an AR runtime calls: hand over a camera picture when the worker
//! is ready, and read the newest depth whenever it likes, without ever
//! waiting on the model.

use std::sync::Arc;
use std::sync::Condvar;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::PoisonError;
use std::thread::JoinHandle;
use std::time::Duration;
use std::time::Instant;

use crate::DepthMap;
use crate::DepthModel;
use crate::Error;
use crate::Picture;

/// How often a [`DepthWorker`] takes a picture unless told otherwise: once
/// a second, which the model keeps up with on a phone's spare core and
/// which is often enough for a room that isn't moving.
pub const DEFAULT_PERIOD: Duration = Duration::from_secs(1);

/// Something that reads depth from a picture: a [`DepthModel`], or a stand-in
/// in tests.
pub trait Estimator: Send + 'static {
	/// How far away everything in `picture` is.
	fn estimate(&self, picture: &Picture) -> Result<DepthMap, Error>;

	/// The longest side, in pixels, worth handing over: a bigger picture is
	/// only averaged down further.
	fn max_side(&self) -> u32;
}

impl Estimator for DepthModel {
	fn estimate(&self, picture: &Picture) -> Result<DepthMap, Error> {
		DepthModel::estimate(self, picture)
	}

	/// Twice the model's longer side, so each of its pixels averages a two
	/// by two block of the picture.
	fn max_side(&self) -> u32 {
		let (width, height) = self.input_size();
		width.max(height) * 2
	}
}

/// One camera picture handed to a [`DepthWorker`].
#[derive(Debug, Clone)]
pub struct Frame<T> {
	/// The picture, with its orientation and lens.
	pub picture: Picture,
	/// When it was captured, in seconds on the caller's clock.
	pub time: f64,
	/// Whatever the caller needs back with the depth: where the camera was,
	/// the picture's display transform.
	pub context: T,
}

/// The depth read from one [`Frame`].
#[derive(Debug, Clone)]
pub struct Estimate<T> {
	/// Counts up from 1 with each estimate since the worker started, so a
	/// caller can tell a new one from the one it already converted.
	pub sequence: u64,
	/// The frame's capture time.
	pub time: f64,
	/// The depth, lying the way the frame's picture did.
	pub depth: DepthMap,
	/// The frame's context.
	pub context: T,
}

/// A thread that reads depth from camera pictures, at most one at a time
/// and at most one each `period`, for a caller that must never wait: an AR
/// runtime calls [`DepthWorker::ready`] and [`DepthWorker::submit`] from the
/// thread that drives its frames, and [`DepthWorker::latest`] from anywhere.
///
/// Dropping it stops the thread once the picture it is reading, if any, is
/// done.
pub struct DepthWorker<T> {
	shared: Arc<Shared<T>>,
	thread: Option<JoinHandle<()>>,
	max_side: u32,
}

impl<T> std::fmt::Debug for DepthWorker<T> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("DepthWorker")
			.field("max_side", &self.max_side)
			.finish_non_exhaustive()
	}
}

struct Shared<T> {
	state: Mutex<State<T>>,
	wake: Condvar,
	period: Duration,
}

struct State<T> {
	pending: Option<Frame<T>>,
	busy: bool,
	accepted: Option<Instant>,
	latest: Option<Arc<Estimate<T>>>,
	error: Option<Error>,
	sequence: u64,
	generation: u64,
	stop: bool,
}

impl<T> Shared<T> {
	fn lock(&self) -> MutexGuard<'_, State<T>> {
		self.state.lock().unwrap_or_else(PoisonError::into_inner)
	}
}

impl<T: Send + Sync + 'static> DepthWorker<T> {
	/// Starts a worker that takes a picture at most once a
	/// [`DEFAULT_PERIOD`].
	pub fn spawn(estimator: impl Estimator) -> std::io::Result<Self> {
		Self::with_period(estimator, DEFAULT_PERIOD)
	}

	/// Starts a worker that takes a picture at most once each `period`.
	pub fn with_period(estimator: impl Estimator, period: Duration) -> std::io::Result<Self> {
		let shared = Arc::new(Shared {
			state: Mutex::new(State {
				pending: None,
				busy: false,
				accepted: None,
				latest: None,
				error: None,
				sequence: 0,
				generation: 0,
				stop: false,
			}),
			wake: Condvar::new(),
			period,
		});
		let max_side = estimator.max_side();
		let worker = Arc::clone(&shared);
		let thread = std::thread::Builder::new()
			.name("leptoskit-depth".into())
			.spawn(move || work(&worker, &estimator))?;

		Ok(Self {
			shared,
			thread: Some(thread),
			max_side,
		})
	}

	/// The longest side, in pixels, worth handing over. Shrink a camera
	/// picture to this before copying it.
	pub fn max_side(&self) -> u32 {
		self.max_side
	}

	/// Whether [`DepthWorker::submit`] would take a picture now: nothing is
	/// being read and a period has passed since the last one was taken.
	/// Cheap; ask before preparing a picture.
	pub fn ready(&self) -> bool {
		let state = self.shared.lock();
		ready(&state, self.shared.period)
	}

	/// Hands over a picture to read, unless one is still being read or a
	/// period has not passed, in which case it is dropped and this returns
	/// `false`. Never waits for the model.
	pub fn submit(&self, frame: Frame<T>) -> bool {
		let mut state = self.shared.lock();

		if !ready(&state, self.shared.period) {
			return false;
		}

		state.pending = Some(frame);
		state.accepted = Some(Instant::now());
		self.shared.wake.notify_one();
		true
	}

	/// The newest depth read, or `None` before the first.
	pub fn latest(&self) -> Option<Arc<Estimate<T>>> {
		self.shared.lock().latest.clone()
	}

	/// Why the last picture could not be read, if it could not.
	pub fn last_error(&self) -> Option<Error> {
		self.shared.lock().error.clone()
	}

	/// Forgets the newest depth and any picture waiting, and drops the one
	/// being read when it is done: the session it came from stopped.
	pub fn reset(&self) {
		let mut state = self.shared.lock();
		state.pending = None;
		state.latest = None;
		state.error = None;
		state.accepted = None;
		state.generation += 1;
	}
}

impl<T> Drop for DepthWorker<T> {
	fn drop(&mut self) {
		self.shared.lock().stop = true;
		self.shared.wake.notify_one();

		if let Some(thread) = self.thread.take() {
			let _ = thread.join();
		}
	}
}

fn ready<T>(state: &State<T>, period: Duration) -> bool {
	!state.busy
		&& state.pending.is_none()
		&& state
			.accepted
			.is_none_or(|accepted| accepted.elapsed() >= period)
}

fn work<T>(shared: &Shared<T>, estimator: &impl Estimator) {
	while let Some((frame, generation)) = next(shared) {
		let result = estimator.estimate(&frame.picture);
		let mut state = shared.lock();
		state.busy = false;

		if state.generation != generation {
			continue;
		}

		match result {
			Ok(depth) => {
				state.sequence += 1;
				state.error = None;
				state.latest = Some(Arc::new(Estimate {
					sequence: state.sequence,
					time: frame.time,
					depth,
					context: frame.context,
				}));
			}
			Err(error) => state.error = Some(error),
		}
	}
}

/// Waits for a picture to read, and marks the worker busy with it; `None`
/// once the worker is dropped.
fn next<T>(shared: &Shared<T>) -> Option<(Frame<T>, u64)> {
	let mut state = shared.lock();

	loop {
		if state.stop {
			return None;
		}

		if let Some(frame) = state.pending.take() {
			state.busy = true;
			return Some((frame, state.generation));
		}

		state = shared
			.wake
			.wait(state)
			.unwrap_or_else(PoisonError::into_inner);
	}
}

#[cfg(test)]
mod tests {
	use std::sync::mpsc;

	use super::*;
	use crate::Intrinsics;
	use crate::Pixels;
	use crate::Plane;

	/// Answers every picture with a flat wall at its width in metres, once
	/// the test lets it.
	struct Gate(Mutex<mpsc::Receiver<()>>);

	impl Estimator for Gate {
		fn estimate(&self, picture: &Picture) -> Result<DepthMap, Error> {
			self.0
				.lock()
				.unwrap()
				.recv()
				.map_err(|_| Error::Inference("closed".into()))?;
			let (width, _) = picture.size();
			if width == 0 {
				return Err(Error::Picture("empty"));
			}
			Ok(DepthMap {
				width: 1,
				height: 1,
				depths: vec![width as f32],
				intrinsics: Intrinsics::assumed(1, 1),
				scale: 1.0,
				took: Duration::ZERO,
			})
		}

		fn max_side(&self) -> u32 {
			896
		}
	}

	fn frame(width: u32, time: f64) -> Frame<&'static str> {
		let pixels = Pixels::Rgb8(Plane::packed(width, 1, 3, vec![0; width as usize * 3]));
		Frame {
			picture: Picture::new(pixels),
			time,
			context: "pose",
		}
	}

	fn wait_for(worker: &DepthWorker<&'static str>, sequence: u64) -> Arc<Estimate<&'static str>> {
		for _ in 0..500 {
			if let Some(estimate) = worker
				.latest()
				.filter(|estimate| estimate.sequence >= sequence)
			{
				return estimate;
			}
			std::thread::sleep(Duration::from_millis(2));
		}
		panic!("no estimate {sequence}");
	}

	#[test]
	fn one_picture_at_a_time_and_the_newest_depth_back() {
		let (open, gate) = mpsc::channel();
		let worker = DepthWorker::with_period(Gate(Mutex::new(gate)), Duration::ZERO).unwrap();
		assert_eq!(worker.max_side(), 896);
		assert!(worker.latest().is_none());
		assert!(worker.submit(frame(3, 1.0)));
		// While it reads, more pictures are dropped rather than queued.
		assert!(!worker.ready());
		assert!(!worker.submit(frame(5, 1.1)));
		open.send(()).unwrap();
		let estimate = wait_for(&worker, 1);
		assert_eq!((estimate.time, estimate.context), (1.0, "pose"));
		assert_eq!(estimate.depth.get(0, 0), Some(3.0));

		assert!(worker.submit(frame(7, 2.0)));
		open.send(()).unwrap();
		assert_eq!(wait_for(&worker, 2).depth.get(0, 0), Some(7.0));
	}

	#[test]
	fn a_period_spaces_pictures_out() {
		let (open, gate) = mpsc::channel();
		let worker =
			DepthWorker::with_period(Gate(Mutex::new(gate)), Duration::from_secs(60)).unwrap();
		assert!(worker.submit(frame(3, 1.0)));
		open.send(()).unwrap();
		wait_for(&worker, 1);
		assert!(!worker.ready());
		assert!(!worker.submit(frame(4, 2.0)));
	}

	#[test]
	fn a_reset_drops_the_picture_being_read_and_errors_are_kept() {
		let (open, gate) = mpsc::channel();
		let worker = DepthWorker::with_period(Gate(Mutex::new(gate)), Duration::ZERO).unwrap();
		assert!(worker.submit(frame(3, 1.0)));
		worker.reset();
		open.send(()).unwrap();
		// The reset picture is never published; the next one is.
		while !worker.ready() {
			std::thread::sleep(Duration::from_millis(2));
		}
		assert!(worker.latest().is_none());
		assert!(worker.submit(frame(0, 2.0)));
		open.send(()).unwrap();
		while worker.last_error().is_none() {
			std::thread::sleep(Duration::from_millis(2));
		}
		assert_eq!(worker.last_error(), Some(Error::Picture("empty")));
		assert!(worker.submit(frame(9, 3.0)));
		open.send(()).unwrap();
		let estimate = wait_for(&worker, 1);
		assert_eq!(estimate.time, 3.0);
		drop(open);
	}
}
