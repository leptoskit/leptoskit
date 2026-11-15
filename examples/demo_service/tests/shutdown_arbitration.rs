//! The stop arbitration's proofs, against the real binary.
//!
//! Termination is authoritative: a SIGTERM wins over the daily reset
//! whichever order they arrive in. These proofs drive the races with
//! real signals and the debug clock — never by calling the service's
//! own request helpers, which would prove nothing about observation.
//!
//! - **Mid-drain**: the reset has started and holds its drain on an open
//!   stream; the SIGTERM lands inside that drain, and the process must exit 0
//!   instead of re-executing into tomorrow.
//! - **Final handoff boundary**: `run` has already reported a reset and parked,
//!   debug-only, after the runtime has stopped; the SIGTERM lands there, and
//!   the final choice must still read it as the last word. The gate is compiled
//!   out of release builds, off unless its environment variables name local
//!   files, bounded, and unreachable over HTTP.

#![cfg(unix)]

mod support;

use std::path::Path;
use std::time::Instant;

use anyhow::Result;
use anyhow::ensure;
use support::DRAIN;
use support::READY;
use support::SIGTERM_BUDGET;
use support::SITE_HOST;
use support::Sse;
use support::appears_within;
use support::clock_hooks;
use support::free_port;
use support::generation_names;
use support::noon;
use support::secrets;
use support::set_clock;
use support::spawn;
use support::volume;
use support::wait_ready;
use support::wait_unready;

/// One day's generation exists, with exactly the credentials it started
/// with: the cancelled reset never published tomorrow.
fn nothing_of_the_new_day(root: &Path, old_secrets: &serde_json::Value) {
	let names = generation_names(root);
	assert_eq!(names.len(), 1, "only today's generation exists: {names:?}");
	assert_eq!(
		secrets(root),
		*old_secrets,
		"the credentials are unchanged: no new day was published"
	);
}

/// The reset is mid-drain, held open by the SSE stream the proof holds,
/// when the operator's SIGTERM arrives — twice: an observed termination
/// is terminal, and repeating it cannot regress the decision. The
/// process exits 0 within the eight-second budget and never re-executes
/// into a new day.
#[test]
fn a_sigterm_during_the_reset_drain_exits_zero_and_publishes_no_new_day() {
	let root = volume();
	let clock = root.join("clock");
	set_clock(&clock, noon(0));

	let port = free_port();
	let mut server = spawn(&root, port, &clock_hooks(&clock));
	wait_ready(&mut server, READY);
	let pid = server.pid();

	// The generation stream stays open: the drain waits for it, up to
	// the grace period, so the SIGTERM below lands mid-drain for certain
	// rather than racing the re-exec.
	let (mut stream, first_event) = Sse::open(port, SITE_HOST, "/demo/generation");
	assert_eq!(first_event.0, "generation", "the stream opens with the day");
	let old_secrets = secrets(&root);

	// Midnight. Readiness fails first; the drain then holds on the open
	// stream.
	set_clock(&clock, noon(1));
	assert_ne!(
		wait_unready(&mut server, READY),
		200,
		"the reset fails readiness before it drains"
	);
	assert_eq!(server.pid(), pid, "the drain does not replace the image");

	// Real signals, mid-drain.
	let started = Instant::now();
	server.signal_stop();
	server.signal_stop();

	let status = server
		.wait_within(SIGTERM_BUDGET)
		.expect("SIGTERM ends the process within the budget");
	let elapsed = started.elapsed();
	assert!(
		status.success(),
		"the mid-drain SIGTERM exits 0, got {status:?} after {elapsed:?}"
	);
	assert!(
		!server.alive(),
		"the process is gone, not re-executed into tomorrow"
	);
	assert!(
		stream.closes_within(DRAIN),
		"the stream closes as the process ends"
	);
	nothing_of_the_new_day(&root, &old_secrets);
}

/// `run` has reported a reset and parked — debug-only — after the
/// runtime has stopped and before the exit-or-exec choice. A SIGTERM
/// observed exactly there is the last word: the final choice re-reads
/// the stop state and exits 0 instead of re-execing.
#[test]
fn a_sigterm_at_the_final_handoff_boundary_exits_zero_instead_of_re_execing() -> Result<()> {
	let root = volume();
	let clock = root.join("clock");
	set_clock(&clock, noon(0));
	let arrived = root.join("handoff-arrived");
	let gate = root.join("handoff-gate");
	let hooks = [
		clock_hooks(&clock),
		vec![
			("DEMO_TEST_HANDOFF_ARRIVED", arrived.display().to_string()),
			("DEMO_TEST_HANDOFF_GATE", gate.display().to_string()),
		],
	]
	.concat();

	let port = free_port();
	let mut server = spawn(&root, port, &hooks);
	wait_ready(&mut server, READY);
	let pid = server.pid();
	let old_secrets = secrets(&root);

	// Midnight: the reset drains with nothing in flight, `run` reports a
	// reset, and the process parks at the boundary.
	set_clock(&clock, noon(1));
	ensure!(
		appears_within(&arrived, READY),
		"the process parks at the final handoff boundary: {}",
		arrived.display()
	);
	assert!(
		server.alive(),
		"the process is parked, neither exited nor re-executed"
	);
	assert_eq!(server.pid(), pid, "the boundary keeps the PID");

	// Real signals, parked at the boundary — repeated, safely.
	server.signal_stop();
	server.signal_stop();
	std::fs::write(&gate, b"").expect("the gate opens");

	let started = Instant::now();
	let status = server
		.wait_within(SIGTERM_BUDGET)
		.expect("the process leaves the boundary within the budget");
	assert!(
		status.success(),
		"the boundary SIGTERM exits 0, got {status:?} after {:?}",
		started.elapsed()
	);
	assert!(
		!server.alive(),
		"the process is gone, not re-executed into tomorrow"
	);
	nothing_of_the_new_day(&root, &old_secrets);
	Ok(())
}
