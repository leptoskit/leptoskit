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
//! The reason is recorded once, in an atomic, before anything waits on
//! it — so a worker task that sees its `JoinHandle` end can tell "the
//! process is stopping" from "the worker died on its own", which is the
//! difference between a normal stop and a truthful nonzero failure.

use std::future::Future;
use std::sync::OnceLock;
use std::sync::atomic::AtomicU8;
use std::sync::atomic::Ordering;
use std::time::Duration;

use tokio::sync::Notify;
use tokio::sync::watch;

use crate::Exit;
use crate::clock;

const RUNNING: u8 = 0;
const IS_TERMINATE: u8 = 1;
const IS_RESET: u8 = 2;

static REASON: AtomicU8 = AtomicU8::new(RUNNING);

/// Wakes everything that waits for the stop, once the reason is set.
static TERMINATED: OnceLock<watch::Sender<bool>> = OnceLock::new();

/// The reset request, from the day watch or the retry timer.
static RESET: std::sync::LazyLock<Notify> = std::sync::LazyLock::new(Notify::new);

/// The reset signal the server's shutdown future awaits.
fn reset_requested() -> impl Future<Output = ()> {
	RESET.notified()
}

/// The stopping side every task holds.
#[derive(Clone)]
pub struct Terminated {
	receiver: watch::Receiver<bool>,
}

/// Initializes the stop signal. Idempotent, so tests that start several
/// lifecycles in one process share it: every caller gets a fresh receiver
/// on the same channel.
pub fn init() -> Terminated {
	let sender = TERMINATED.get_or_init(|| watch::channel(false).0);
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

/// Why the process is stopping, once it is.
#[must_use]
pub fn reason() -> Option<Exit> {
	match REASON.load(Ordering::SeqCst) {
		IS_TERMINATE => Some(Exit::Terminate),
		IS_RESET => Some(Exit::Reset),
		_ => None,
	}
}

/// A SIGTERM, straight from the signal.
pub fn request_terminate() {
	REASON.store(IS_TERMINATE, Ordering::SeqCst);
	announce();
}

/// This image's own stop: the day changed, or a bounded retry is due. The
/// image will be replaced by `execve`, not exited.
pub fn request_reset() {
	REASON.store(IS_RESET, Ordering::SeqCst);
	RESET.notify_one();
	announce();
}

fn announce() {
	if let Some(sender) = TERMINATED.get() {
		let _ = sender.send(true);
	}
}

/// The future `GracefulShutdown` stops the listener on: SIGTERM or the
/// reset request, whichever comes first. Readiness fails before the
/// drain, so `/readyz` turns 503 at once.
pub async fn serve_signal() {
	#[cfg(unix)]
	let terminate = async {
		use tokio::signal::unix::SignalKind;
		use tokio::signal::unix::signal;
		match signal(SignalKind::terminate()) {
			Ok(mut signal) => {
				signal.recv().await;
				request_terminate();
			}
			Err(error) => {
				// Without the handler installed, a SIGTERM would kill the
				// process mid-write; refusing to serve is honest.
				eprintln!("couldn't install the SIGTERM handler: {error}");
				std::process::exit(1);
			}
		}
	};

	#[cfg(not(unix))]
	let terminate = std::future::pending::<()>();

	tokio::select! {
		() = terminate => {}
		() = reset_requested() => {
			// The requester already recorded the reason.
		}
	}
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
