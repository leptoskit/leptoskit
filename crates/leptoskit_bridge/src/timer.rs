//! Timers: how a guest waits for a duration.
//!
//! Like the display frame clock in [`crate::frame`], the timer is a reserved
//! plugin on the ordinary bridge rather than a new Wasm import. A bundle that
//! uses it still loads on a host built before it existed, where
//! [`TIMER_PLUGIN`] does not resolve and the guest receives
//! [`Error::Unavailable`](crate::Error::Unavailable) instead of failing to
//! instantiate or pretending a sleep completed at once. Code push can
//! therefore ship it to existing installs.
//!
//! The host schedules a real timer, completes the call when its deadline
//! arrives, and drops it when the guest drops the waiting future. Hosts that
//! cannot run timers must leave the plugin unregistered; they never answer a
//! sleep early.
//!
//! A host that registers the plugin uses [`host::TimerScheduler`] to keep one
//! bounded timer thread, independent of frames or an event loop.

use std::time::Duration;

use crate::Error;
use crate::MethodId;

/// The reserved plugin name a timer-capable host registers.
pub const TIMER_PLUGIN: &str = "leptoskit.timer";

/// Waits for the little-endian `u64` milliseconds in the request payload.
pub const SLEEP: MethodId = MethodId(0);

/// The encoded size of a sleep request: one little-endian `u64`.
pub const SLEEP_BYTES: usize = 8;

/// The longest single sleep a host accepts, in milliseconds.
const MAX_SLEEP_MILLIS: u64 = 86_400_000;

/// The longest single sleep a host accepts.
///
/// This bounds a hostile or mistaken guest without making long cache or
/// backoff waits impossible. A caller that needs longer waits repeatedly.
pub const MAX_SLEEP: Duration = Duration::from_millis(MAX_SLEEP_MILLIS);

/// Encodes a duration as whole milliseconds, rounding up so a sub-millisecond
/// wait never becomes an immediate one.
///
/// # Errors
///
/// [`Error::InvalidArgument`] when the duration is longer than
/// [`MAX_SLEEP`].
pub fn encode(duration: Duration) -> Result<[u8; SLEEP_BYTES], Error> {
	let millis = duration.as_nanos().div_ceil(1_000_000);

	if millis > u128::from(MAX_SLEEP_MILLIS) {
		return Err(Error::InvalidArgument);
	}

	Ok(u64::try_from(millis)
		.map_err(|_| Error::InvalidArgument)?
		.to_le_bytes())
}

/// Decodes a sleep request.
///
/// # Errors
///
/// [`Error::InvalidArgument`] when the payload is not exactly
/// [`SLEEP_BYTES`] long or asks for longer than [`MAX_SLEEP`].
pub fn decode(bytes: &[u8]) -> Result<Duration, Error> {
	let bytes: [u8; SLEEP_BYTES] = bytes.try_into().map_err(|_| Error::InvalidArgument)?;
	let millis = u64::from_le_bytes(bytes);

	if millis > MAX_SLEEP_MILLIS {
		return Err(Error::InvalidArgument);
	}

	Ok(Duration::from_millis(millis))
}

/// Host-side timer scheduling without a browser or an async runtime.
///
/// The scheduler owns one thread. It wakes only at the earliest deadline (or
/// when a host schedules, cancels, or closes), not on every frame. The host
/// polls [`TimerScheduler::take_due`] on its own thread and completes the
/// calls there; a wake callback can ask the host's event loop to poll sooner.
#[cfg(not(target_arch = "wasm32"))]
pub mod host {
	use std::collections::BTreeMap;
	use std::sync::Arc;
	use std::sync::Condvar;
	use std::sync::Mutex;
	use std::sync::MutexGuard;
	use std::sync::PoisonError;
	use std::thread::JoinHandle;
	use std::time::Duration;
	use std::time::Instant;

	use crate::CallId;
	use crate::Error;

	/// The most timers one host may have waiting at once.
	pub const MAX_TIMERS: usize = 256;

	type Wake = Arc<dyn Fn() + Send + Sync + 'static>;

	#[derive(Default)]
	struct State {
		closed: bool,
		timers: BTreeMap<u64, Instant>,
		/// Deadlines the thread already reached. The next `take_due` consumes
		/// them, so a completion always happens on the host's thread.
		due: Vec<CallId>,
	}

	struct Inner {
		state: Mutex<State>,
		changed: Condvar,
		wake: Mutex<Option<Wake>>,
	}

	impl Inner {
		fn notify(&self) {
			let wake = self
				.wake
				.lock()
				.ok()
				.and_then(|slot| slot.as_ref().map(Arc::clone));

			if let Some(wake) = wake {
				wake();
			}
		}
	}

	/// A bounded timer thread shared by every timer of one host.
	pub struct TimerScheduler {
		inner: Arc<Inner>,
		thread: Option<JoinHandle<()>>,
	}

	impl Default for TimerScheduler {
		fn default() -> Self {
			Self::new().expect("spawning the timer thread")
		}
	}

	impl TimerScheduler {
		/// Starts the timer thread with no wake callback.
		///
		/// # Errors
		///
		/// [`Error::Native`] when the thread cannot start, so a host reports
		/// timers unavailable instead of leaving a guest asleep forever.
		pub fn new() -> Result<Self, Error> {
			let inner = Arc::new(Inner {
				state: Mutex::default(),
				changed: Condvar::new(),
				wake: Mutex::new(None),
			});
			let worker = Arc::clone(&inner);
			let thread = std::thread::Builder::new()
				.name("leptoskit timer".to_owned())
				.spawn(move || run(&worker))
				.map_err(|_| Error::Native)?;

			Ok(Self {
				inner,
				thread: Some(thread),
			})
		}

		/// Replaces the callback that wakes the host after timers come due.
		///
		/// It runs on the timer thread and must only ask the host's event loop
		/// to poll; it must not enter the interpreter or borrow host state.
		pub fn set_wake(&self, wake: impl Fn() + Send + Sync + 'static) {
			if let Ok(mut slot) = self.inner.wake.lock() {
				*slot = Some(Arc::new(wake));
			}
		}

		/// Schedules `id` to come due after `duration`.
		///
		/// # Errors
		///
		/// [`Error::Busy`] when [`MAX_TIMERS`] are waiting or `id` is already
		/// scheduled, and [`Error::Cancelled`] once the scheduler is closed.
		pub fn schedule(&self, id: CallId, duration: Duration) -> Result<(), Error> {
			let at = Instant::now()
				.checked_add(duration)
				.ok_or(Error::InvalidArgument)?;
			let mut state = lock(&self.inner.state);

			if state.closed {
				return Err(Error::Cancelled);
			}

			if state.timers.len() + state.due.len() >= MAX_TIMERS
				|| state.timers.contains_key(&id.0)
				|| state.due.contains(&id)
			{
				return Err(Error::Busy);
			}

			state.timers.insert(id.0, at);
			drop(state);
			self.inner.changed.notify_one();
			Ok(())
		}

		/// Cancels a waiting or due timer, returning whether anything was.
		pub fn cancel(&self, id: CallId) -> bool {
			let mut state = lock(&self.inner.state);
			let scheduled = state.timers.remove(&id.0).is_some();
			let due = state.due.iter().position(|candidate| *candidate == id);

			if let Some(index) = due {
				state.due.remove(index);
			}

			let due = due.is_some();
			drop(state);

			if scheduled || due {
				self.inner.changed.notify_one();
			}

			scheduled || due
		}

		/// Takes every timer whose deadline passed. Empty once closed, so a
		/// late callback never revives a closed host.
		pub fn take_due(&self) -> Vec<CallId> {
			let mut state = lock(&self.inner.state);

			if state.closed {
				return Vec::new();
			}

			std::mem::take(&mut state.due)
		}

		/// How long until the host should call [`Self::take_due`], or `None`
		/// when no timer waits. `Some(Duration::ZERO)` means work is due now.
		pub fn next_deadline(&self) -> Option<Duration> {
			let state = lock(&self.inner.state);

			if state.closed {
				return None;
			}

			if !state.due.is_empty() {
				return Some(Duration::ZERO);
			}

			state
				.timers
				.values()
				.min()
				.map(|at| at.saturating_duration_since(Instant::now()))
		}

		/// Whether `id` is waiting or due.
		pub fn contains(&self, id: CallId) -> bool {
			let state = lock(&self.inner.state);
			!state.closed && (state.timers.contains_key(&id.0) || state.due.contains(&id))
		}

		/// Whether no timer is waiting or due.
		pub fn is_empty(&self) -> bool {
			let state = lock(&self.inner.state);
			state.timers.is_empty() && state.due.is_empty()
		}

		/// Stops the thread and drops every waiting timer. Called by `Drop`.
		pub fn close(&self) {
			let mut state = lock(&self.inner.state);
			state.closed = true;
			state.timers.clear();
			state.due.clear();
			drop(state);
			self.inner.changed.notify_all();
		}
	}

	impl Drop for TimerScheduler {
		fn drop(&mut self) {
			self.close();

			if let Ok(mut wake) = self.inner.wake.lock() {
				*wake = None;
			}

			if let Some(thread) = self.thread.take() {
				let _ = thread.join();
			}
		}
	}

	fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
		state.lock().unwrap_or_else(PoisonError::into_inner)
	}

	fn run(inner: &Inner) {
		loop {
			let mut state = lock(&inner.state);

			if state.closed {
				return;
			}

			let now = Instant::now();
			let mut fired = false;

			loop {
				let due = state
					.timers
					.iter()
					.find(|(_, at)| **at <= now)
					.map(|(id, _)| *id);
				let Some(id) = due else {
					break;
				};

				state.timers.remove(&id);
				state.due.push(CallId(id));
				fired = true;
			}

			drop(state);

			if fired {
				inner.notify();
			}

			let mut state = lock(&inner.state);

			if state.closed {
				return;
			}

			let timeout = state
				.timers
				.values()
				.min()
				.map(|at| at.saturating_duration_since(Instant::now()));

			state = match timeout {
				Some(timeout) if timeout.is_zero() => continue,
				Some(timeout) => {
					inner
						.changed
						.wait_timeout(state, timeout)
						.unwrap_or_else(|error| error.into_inner())
						.0
				}
				None => {
					inner
						.changed
						.wait(state)
						.unwrap_or_else(|error| error.into_inner())
				}
			};
			drop(state);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn durations_round_trip_in_whole_milliseconds_rounding_up() {
		assert_eq!(
			decode(&encode(Duration::ZERO).unwrap()).unwrap(),
			Duration::ZERO
		);
		assert_eq!(
			decode(&encode(Duration::from_millis(1)).unwrap()).unwrap(),
			Duration::from_millis(1)
		);
		assert_eq!(
			decode(&encode(Duration::from_nanos(1)).unwrap()).unwrap(),
			Duration::from_millis(1),
			"a tiny wait must not become an immediate one"
		);
		assert_eq!(decode(&encode(MAX_SLEEP).unwrap()).unwrap(), MAX_SLEEP);
		assert_eq!(
			encode(MAX_SLEEP + Duration::from_millis(1)),
			Err(Error::InvalidArgument)
		);
		assert_eq!(decode(&[0; 7]), Err(Error::InvalidArgument));
		assert_eq!(decode(&[0; 9]), Err(Error::InvalidArgument));
	}

	#[cfg(not(target_arch = "wasm32"))]
	mod host {
		use std::sync::Arc;
		use std::sync::atomic::AtomicUsize;
		use std::sync::atomic::Ordering;
		use std::time::Duration;

		use super::super::host::MAX_TIMERS;
		use super::super::host::TimerScheduler;
		use crate::CallId;
		use crate::Error;

		fn wait_until(deadline: std::time::Instant, timers: &TimerScheduler) -> Vec<CallId> {
			loop {
				let due = timers.take_due();

				if !due.is_empty() || std::time::Instant::now() >= deadline {
					return due;
				}

				std::thread::sleep(Duration::from_millis(2));
			}
		}

		#[test]
		fn a_due_timer_is_taken_once_on_the_host_thread() {
			let timers = TimerScheduler::new().unwrap();
			timers
				.schedule(CallId(7), Duration::from_millis(10))
				.unwrap();

			assert!(timers.contains(CallId(7)));
			assert!(timers.next_deadline().is_some());

			let due = wait_until(std::time::Instant::now() + Duration::from_secs(1), &timers);
			assert_eq!(due, [CallId(7)]);
			assert!(!timers.contains(CallId(7)));
			assert!(timers.take_due().is_empty());
			assert!(timers.is_empty());
		}

		#[test]
		fn cancelling_a_timer_prevents_its_completion() {
			let timers = TimerScheduler::new().unwrap();
			timers
				.schedule(CallId(1), Duration::from_millis(20))
				.unwrap();
			assert!(timers.cancel(CallId(1)));
			assert!(!timers.cancel(CallId(1)));
			std::thread::sleep(Duration::from_millis(40));
			assert!(timers.take_due().is_empty());
			assert!(timers.is_empty());
		}

		#[test]
		fn closing_drops_waiting_and_due_timers_without_late_delivery() {
			let timers = TimerScheduler::new().unwrap();
			timers
				.schedule(CallId(1), Duration::from_millis(1))
				.unwrap();
			std::thread::sleep(Duration::from_millis(20));
			timers.close();
			assert!(timers.take_due().is_empty());
			assert_eq!(timers.next_deadline(), None);
			assert!(!timers.contains(CallId(1)));
			assert_eq!(
				timers.schedule(CallId(2), Duration::ZERO),
				Err(Error::Cancelled)
			);
		}

		#[test]
		fn the_scheduler_bounds_waiting_timers() {
			let timers = TimerScheduler::new().unwrap();

			for id in 0..MAX_TIMERS {
				timers
					.schedule(CallId(id as u64 + 1), Duration::from_secs(60))
					.unwrap();
			}

			assert_eq!(
				timers.schedule(CallId(MAX_TIMERS as u64 + 1), Duration::ZERO),
				Err(Error::Busy)
			);
		}

		#[test]
		fn due_timers_keep_their_slots_until_delivered_or_cancelled() {
			let timers = TimerScheduler::new().unwrap();
			let (sender, receiver) = std::sync::mpsc::channel();
			timers.set_wake(move || {
				let _ = sender.send(());
			});

			// Wait for each timer to move into the delivery queue without
			// consuming it. Its slot must remain occupied after its deadline.
			for id in 0..MAX_TIMERS {
				timers.schedule(CallId(id as u64), Duration::ZERO).unwrap();
				receiver.recv_timeout(Duration::from_secs(2)).unwrap();
			}

			let extra = CallId(MAX_TIMERS as u64);
			assert_eq!(timers.schedule(extra, Duration::ZERO), Err(Error::Busy));
			assert!(timers.cancel(CallId(0)));
			timers.schedule(extra, Duration::from_secs(60)).unwrap();
			assert_eq!(timers.take_due().len(), MAX_TIMERS - 1);
			assert!(timers.contains(extra));
		}

		#[test]
		fn the_wake_callback_runs_when_a_timer_comes_due() {
			let timers = TimerScheduler::new().unwrap();
			let wakes = Arc::new(AtomicUsize::new(0));
			let count = Arc::clone(&wakes);
			timers.set_wake(move || {
				count.fetch_add(1, Ordering::AcqRel);
			});
			timers
				.schedule(CallId(3), Duration::from_millis(5))
				.unwrap();

			let deadline = std::time::Instant::now() + Duration::from_secs(1);

			while wakes.load(Ordering::Acquire) == 0 && std::time::Instant::now() < deadline {
				std::thread::sleep(Duration::from_millis(2));
			}

			assert!(wakes.load(Ordering::Acquire) > 0);
			assert_eq!(timers.take_due(), [CallId(3)]);
		}
	}
}
