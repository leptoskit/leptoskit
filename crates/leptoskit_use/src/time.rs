use std::sync::Arc;
use std::time::Duration;

use reactive_graph::owner::on_cleanup;
use reactive_graph::prelude::*;
use reactive_graph::signal::ReadSignal;
use reactive_graph::signal::RwSignal;
use reactive_graph::wrappers::read::Signal;

use crate::Action;
use crate::watch_with_options;

/// A host-provided monotonic clock, measured from any fixed origin.
///
/// Advance the signal from your event loop, animation frame, server scheduler,
/// or test. Time must never go backwards. Hooks schedule no work on their own.
/// Using one signal lets an application drive all its timers with one host
/// timer.
pub type Clock = Signal<Duration>;

/// A [`Clock`] that advances by itself, about once every `interval`, on the
/// platform's own timer: `setTimeout` in the browser, the timer plugin in a
/// native app, and tokio or a timer thread elsewhere.
///
/// Use it for a countdown, a "Copied" label that clears itself, or anything
/// else that needs time to pass without the host driving a signal. The value
/// is the time since the hook was called. It is measured, not counted, so a
/// tab the browser put to sleep catches up in one step when it wakes, and
/// intervals built on it [coalesce](use_interval_fn) what they missed.
///
/// ```no_run
/// use std::time::Duration;
///
/// use leptoskit_use::use_clock;
/// use leptoskit_use::use_timeout_fn;
/// use reactive_graph::prelude::*;
/// use reactive_graph::signal::RwSignal;
///
/// let copied = RwSignal::new(true);
/// let clock = use_clock(Duration::from_millis(100));
/// use_timeout_fn(
/// 	clock,
/// 	move || copied.set(false),
/// 	Duration::from_millis(1200),
/// );
/// ```
///
/// The clock stops when the owner it was created under is cleaned up, so a
/// screen that leaves takes its timer with it. An `interval` under one
/// millisecond is treated as one millisecond.
///
/// The app's task executor runs the timer, which every app target leptoskit
/// builds sets up. Where the host has no timer, such as a native host built
/// before the timer plugin existed, the clock stays at zero. While a page
/// renders on the server the clock reads zero and stops when the render's
/// owner is cleaned up.
#[cfg(feature = "clock")]
#[must_use]
pub fn use_clock(interval: Duration) -> Clock {
	let interval = interval.max(Duration::from_millis(1));
	let elapsed = RwSignal::new(Duration::ZERO);
	let (ticking, stop) = futures::future::abortable(async move {
		let started = leptoskit_query::now_ms();
		let mut latest = Duration::ZERO;

		while leptoskit_query::try_sleep(interval).await.is_ok() {
			// A wall clock can be set back. This one never goes back.
			let now = Duration::from_millis(leptoskit_query::now_ms().saturating_sub(started));
			latest = latest.max(now);

			if elapsed.try_set(latest).is_some() {
				// The signal is gone, and so is whoever was reading it.
				return;
			}
		}
	});
	let ticking = async move {
		let _ = ticking.await;
	};
	#[cfg(not(target_family = "wasm"))]
	any_spawner::Executor::spawn(ticking);
	#[cfg(target_family = "wasm")]
	any_spawner::Executor::spawn_local(ticking);
	on_cleanup(move || stop.abort());

	elapsed.into()
}

/// Restartable one-shot timer controls.
#[derive(Clone)]
pub struct UseTimeoutFnReturn {
	/// Whether a deadline is scheduled.
	pub is_pending: Signal<bool>,
	/// Start or restart from the clock's current value.
	pub start: Action,
	/// Cancel the deadline.
	pub stop: Action,
}

/// Run once on the first clock update at or after the deadline.
///
/// Starts immediately. A zero delay fires on the next clock change. Stopping,
/// restarting, or owner cleanup cancels the old deadline. Deadline arithmetic
/// saturates at `Duration::MAX`.
pub fn use_timeout_fn(
	clock: impl Into<Clock>,
	callback: impl Fn() + Send + Sync + 'static,
	delay: Duration,
) -> UseTimeoutFnReturn {
	let clock = clock.into();
	let deadline = RwSignal::new(Some(clock.get_untracked().saturating_add(delay)));
	watch_with_options(
		move || clock.get(),
		move |now, _| {
			if deadline.get_untracked().is_some_and(|at| *now >= at) {
				deadline.set(None);
				callback();
			}
		},
		false,
	);
	on_cleanup(move || {
		deadline.try_set(None);
	});
	UseTimeoutFnReturn {
		is_pending: Signal::derive(move || deadline.with(Option::is_some)),
		start: Arc::new(move || deadline.set(Some(clock.get_untracked().saturating_add(delay)))),
		stop: Arc::new(move || deadline.set(None)),
	}
}

/// Repeating timer controls.
#[derive(Clone)]
pub struct UseIntervalFnReturn {
	/// Whether the interval is running.
	pub is_active: Signal<bool>,
	/// Cancel future ticks.
	pub pause: Action,
	/// Restart a full interval from the current clock value.
	pub resume: Action,
}

/// Run at most once per host clock update, coalescing missed intervals.
///
/// Starts immediately, with the first callback after `interval`. Resuming
/// starts a fresh interval. A zero interval runs once per clock change. There
/// is no catch-up loop after an application resumes from the background.
pub fn use_interval_fn(
	clock: impl Into<Clock>,
	callback: impl Fn() + Send + Sync + 'static,
	interval: Duration,
) -> UseIntervalFnReturn {
	let clock = clock.into();
	let deadline = RwSignal::new(Some(clock.get_untracked().saturating_add(interval)));
	watch_with_options(
		move || clock.get(),
		move |now, _| {
			if deadline.get_untracked().is_some_and(|at| *now >= at) {
				deadline.set(Some(now.saturating_add(interval)));
				callback();
			}
		},
		false,
	);
	on_cleanup(move || {
		deadline.try_set(None);
	});
	UseIntervalFnReturn {
		is_active: Signal::derive(move || deadline.with(Option::is_some)),
		pause: Arc::new(move || deadline.set(None)),
		resume: Arc::new(move || {
			deadline.set(Some(clock.get_untracked().saturating_add(interval)));
		}),
	}
}

/// Count interval ticks. Missed ticks are coalesced, and the count saturates.
pub fn use_interval(clock: impl Into<Clock>, interval: Duration) -> ReadSignal<u64> {
	let count = RwSignal::new(0_u64);
	use_interval_fn(
		clock,
		move || count.update(|n| *n = n.saturating_add(1)),
		interval,
	);
	count.read_only()
}

/// A trailing-edge debounced function. Repeated calls restart the deadline.
/// The callback has no arguments: capture signals and read them when it runs.
pub fn use_debounce_fn(
	clock: impl Into<Clock>,
	callback: impl Fn() + Send + Sync + 'static,
	delay: Duration,
) -> Action {
	let timer = use_timeout_fn(clock, callback, delay);
	(timer.stop)();
	timer.start
}

/// A leading-edge throttled function; calls inside the interval are dropped.
/// Zero duration permits every call. No trailing invocation is scheduled.
pub fn use_throttle_fn(
	clock: impl Into<Clock>,
	callback: impl Fn() + Send + Sync + 'static,
	interval: Duration,
) -> Action {
	let clock = clock.into();
	let next = RwSignal::new(Duration::ZERO);
	Arc::new(move || {
		let now = clock.get_untracked();
		if now >= next.get_untracked() {
			next.set(now.saturating_add(interval));
			callback();
		}
	})
}

/// Debounce changes to a signal, retaining the initial value immediately.
///
/// Subscribers are notified only when the debounced value really changes: a
/// source that changes and changes back within the delay notifies nobody.
pub fn signal_debounced<T>(
	source: impl Get<Value = T> + Send + Sync + 'static,
	clock: impl Into<Clock>,
	delay: Duration,
) -> ReadSignal<T>
where
	T: Clone + PartialEq + Send + Sync + 'static,
{
	let source = Signal::derive(move || source.get());
	let output = RwSignal::new(source.get_untracked());
	let trigger = use_debounce_fn(clock, move || set_if_changed(output, source), delay);
	watch_with_options(move || source.get(), move |_, _| trigger(), false);
	output.read_only()
}

/// Throttle changes to a signal on the leading edge, without a trailing update.
///
/// Subscribers are notified only when the throttled value really changes.
pub fn signal_throttled<T>(
	source: impl Get<Value = T> + Send + Sync + 'static,
	clock: impl Into<Clock>,
	interval: Duration,
) -> ReadSignal<T>
where
	T: Clone + PartialEq + Send + Sync + 'static,
{
	let source = Signal::derive(move || source.get());
	let output = RwSignal::new(source.get_untracked());
	let trigger = use_throttle_fn(clock, move || set_if_changed(output, source), interval);
	watch_with_options(move || source.get(), move |_, _| trigger(), false);
	output.read_only()
}

/// Elapsed time on the host clock since this hook was created.
///
/// The value is zero at creation and follows every clock update; it never
/// goes negative even if the host supplies an earlier reading.
pub fn use_timestamp(clock: impl Into<Clock>) -> Signal<Duration> {
	let clock = clock.into();
	let origin = clock.get_untracked();
	Signal::derive(move || clock.get().saturating_sub(origin))
}

/// Copies `source` into `output` unless they are already equal, so a value
/// that changed and changed back wakes no subscriber.
fn set_if_changed<T>(output: RwSignal<T>, source: Signal<T>)
where
	T: Clone + PartialEq + Send + Sync + 'static,
{
	let next = source.get_untracked();

	if output.with_untracked(|current| *current != next) {
		output.set(next);
	}
}
