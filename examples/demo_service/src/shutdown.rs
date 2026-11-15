//! How this process image stops, and what it means.
//!
//! There are exactly two ways to stop serving: a supervisor's SIGTERM, or
//! this image's own reset — the day changed, or a failed initialization's
//! bounded retry is due. Both fail readiness first, then drain in-flight
//! requests and streams for at most three seconds; after that the runtime
//! gets at most two more seconds, and then the process either exits 0
//! (SIGTERM) or replaces itself with the same binary through `execve`
//! (reset), which keeps the PID and closes every descriptor.
//!
//! Termination is authoritative. A reset moves a running image to reset;
//! a SIGTERM moves a running *or* resetting image to terminate, and
//! terminate is terminal: neither a retry nor a repeated reset can
//! replace an observed termination. The SIGTERM listener is installed
//! once, before anything serves, and stays armed for the whole process
//! life, so a signal is still observed through the server's drain and
//! the runtime's shutdown; the final exit-or-exec choice reads the
//! recorded reason after the runtime has stopped. A signal delivered
//! after that last read is past the boundary — the decision is already
//! made, and nothing claims otherwise.
//!
//! The reason is recorded once, in an atomic, before anything waits on
//! it — so a worker task that sees its `JoinHandle` end can tell "the
//! process is stopping" from "the worker died on its own", which is the
//! difference between a normal stop and a truthful nonzero failure.

use std::sync::OnceLock;
use std::sync::atomic::AtomicU8;
use std::sync::atomic::Ordering;
use std::time::Duration;

use tokio::sync::watch;

use crate::Exit;
use crate::clock;

const RUNNING: u8 = 0;
const IS_TERMINATE: u8 = 1;
const IS_RESET: u8 = 2;

/// Why the process is stopping. A reset moves a running image to reset;
/// termination moves a running or resetting image to terminate, and
/// nothing moves a recorded reason back or past it.
struct StopReason {
	state: AtomicU8,
}

impl StopReason {
	const fn new() -> Self {
		Self {
			state: AtomicU8::new(RUNNING),
		}
	}

	/// The operator's stop: recorded over anything already decided,
	/// because termination is terminal.
	fn record_terminate(&self) {
		self.state.store(IS_TERMINATE, Ordering::SeqCst);
	}

	/// This image's own reset: recorded only while running, so it can
	/// never replace a termination. Returns whether it was recorded.
	fn record_reset(&self) -> bool {
		self.state
			.compare_exchange(RUNNING, IS_RESET, Ordering::SeqCst, Ordering::SeqCst)
			.is_ok()
	}

	/// Why the process is stopping, once it is.
	fn reason(&self) -> Option<Exit> {
		match self.state.load(Ordering::SeqCst) {
			IS_TERMINATE => Some(Exit::Terminate),
			IS_RESET => Some(Exit::Reset),
			_ => None,
		}
	}
}

/// The one stop state of this process image.
static STOP: StopReason = StopReason::new();

/// Wakes everything that waits for the stop, once the reason is set.
static TERMINATED: OnceLock<watch::Sender<bool>> = OnceLock::new();

/// The stopping side every task holds.
#[derive(Clone)]
pub struct Terminated {
	receiver: watch::Receiver<bool>,
}

/// Initializes the stop signal. Idempotent, so tests that start several
/// lifecycles in one process share it: every caller gets a fresh receiver
/// on the same channel. A reason recorded before this call — a SIGTERM
/// during startup — is announced here, so the process still stops.
pub fn init() -> Terminated {
	let sender = TERMINATED.get_or_init(|| watch::channel(false).0);
	if reason().is_some() {
		announce();
	}
	Terminated {
		receiver: sender.subscribe(),
	}
}

impl Terminated {
	/// Resolves when the process starts to stop.
	pub async fn wait(&self) {
		let mut receiver = self.receiver.clone();
		while !*receiver.borrow_and_update() {
			if receiver.changed().await.is_err() {
				// The sender is a static; this can't happen, but a stuck
				// wait would be worse than a spurious return.
				return;
			}
		}
	}

	/// Whether the stop has started: the reason is already recorded.
	#[must_use]
	pub fn is_terminating(&self) -> bool {
		reason().is_some()
	}

	/// The cancellation future every lock wait and timer selects against,
	/// so SIGTERM stops a waiter instead of orphaning it.
	pub fn cancellable(&self) -> crate::locks::Cancel {
		let receiver = self.receiver.clone();
		Box::pin(async move {
			let mut receiver = receiver;
			while !*receiver.borrow_and_update() {
				if receiver.changed().await.is_err() {
					return;
				}
			}
		})
	}
}

/// Why the process is stopping, once it is. Reasons never regress: a
/// recorded termination stays the answer no matter what else requests.
#[must_use]
pub fn reason() -> Option<Exit> {
	STOP.reason()
}

/// A SIGTERM, straight from the listener thread. Recorded over any reset
/// already decided: termination is terminal.
pub fn request_terminate() {
	STOP.record_terminate();
	announce();
}

/// This image's own stop: the day changed, or a bounded retry is due. The
/// image will be replaced by `execve`, not exited — unless a SIGTERM has
/// already been observed, which stays the decision.
pub fn request_reset() {
	if STOP.record_reset() {
		announce();
	}
}

fn announce() {
	if let Some(sender) = TERMINATED.get() {
		let _ = sender.send(true);
	}
}

/// The future `GracefulShutdown` stops the listener on: the stop has been
/// requested, by SIGTERM or by a reset. The SIGTERM listener was
/// installed before anything served (see [`install_terminate_handler`])
/// and outlives this future — a signal that arrives during the drain it
/// starts is still recorded, and the final choice re-reads the reason
/// after the runtime stops. Readiness fails before the drain, so
/// `/readyz` turns 503 at once.
pub async fn serve_signal() {
	init().wait().await;
}

/// Installs the SIGTERM listener once, before anything serves, on a
/// dedicated thread that lives as long as the process. `signal-hook`'s
/// iterator parks a normal thread on a self-pipe: the raw handler only
/// writes a byte, and everything here — the record, the watch — runs on
/// the thread, so nothing unsafe and no async work ever runs in signal
/// context. The thread is never cancelled: a SIGTERM during the server's
/// drain, during the runtime's shutdown, or right up to the final
/// decision is still recorded.
///
/// Lifetime contract: the exit-or-exec decision reads the recorded
/// reason once, after the runtime has stopped; a signal delivered after
/// that read is past the last decision, and this module claims no
/// coverage of it.
///
/// Fails honestly when the handler cannot be installed, before anything
/// serves: without it, a SIGTERM would kill the process mid-write.
#[cfg(unix)]
pub fn install_terminate_handler() -> std::io::Result<()> {
	use signal_hook::consts::SIGTERM;
	use signal_hook::iterator::Signals;

	let mut signals = Signals::new([SIGTERM])?;
	std::thread::Builder::new()
		.name("demo-sigterm".to_owned())
		.spawn(move || {
			// SIGTERM is the only signal registered above; record it and
			// keep listening until the process ends.
			for signal in signals.forever() {
				debug_assert_eq!(signal, SIGTERM, "only SIGTERM is registered");
				request_terminate();
			}
		})
		.map(|_| ())
}

/// Non-Unix: there is no SIGTERM, so there is nothing to install — the
/// stop comes from the image's own reset.
#[cfg(not(unix))]
pub fn install_terminate_handler() -> std::io::Result<()> {
	Ok(())
}

/// Watches the UTC day, at most [`clock::watch_interval`] between checks,
/// and requests the reset when it differs from the generation's.
pub async fn watch_day(day: String, terminated: Terminated) {
	loop {
		tokio::select! {
			() = tokio::time::sleep(clock::watch_interval()) => {}
			() = terminated.wait() => return,
		}

		if clock::day_of(clock::now_millis()) != day {
			request_reset();
			return;
		}
	}
}

/// The bounded retry: after a failed initialization, keep the fallback
/// for `delay`, then reset into a fresh image that tries again. SIGTERM
/// cancels the wait.
pub async fn retry_later(delay: Duration, terminated: Terminated) {
	tokio::select! {
		() = tokio::time::sleep(delay) => {
			eprintln!("retrying the generation in a fresh image");
			request_reset();
		}
		() = terminated.wait() => {}
	}
}

/// How long the whole stop may take, once the signal arrives: at most the
/// drain for requests and streams, then this much for the runtime's
/// pending tasks. The brief's bound for SIGTERM is eight seconds in all.
#[must_use]
pub const fn runtime_shutdown_timeout() -> Duration {
	Duration::from_secs(2)
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Terminate, then a late reset: the operator's stop is terminal.
	#[test]
	fn termination_outranks_a_late_reset() {
		let stop = StopReason::new();
		stop.record_terminate();
		assert_eq!(stop.reason(), Some(Exit::Terminate));
		assert!(
			!stop.record_reset(),
			"a reset after an observed termination is refused"
		);
		assert_eq!(stop.reason(), Some(Exit::Terminate));
	}

	/// Reset, then terminate: the day turned, then the operator stopped
	/// the image; the end state is the operator's.
	#[test]
	fn termination_outranks_an_earlier_reset() {
		let stop = StopReason::new();
		assert!(stop.record_reset());
		assert_eq!(stop.reason(), Some(Exit::Reset));
		stop.record_terminate();
		assert_eq!(stop.reason(), Some(Exit::Terminate));
	}

	/// A repeated reset is not a new decision, and it never retires a
	/// termination.
	#[test]
	fn a_repeated_reset_is_not_a_new_decision() {
		let stop = StopReason::new();
		assert!(stop.record_reset());
		assert!(!stop.record_reset(), "the second reset adds nothing");
		assert_eq!(stop.reason(), Some(Exit::Reset));
		stop.record_terminate();
		assert!(!stop.record_reset(), "termination stays terminal");
		assert_eq!(stop.reason(), Some(Exit::Terminate));
	}
}
