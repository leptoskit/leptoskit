//! The lifecycle's proofs, against the real binary.
//!
//! Every test spawns `CARGO_BIN_EXE_demo_service` with its own temporary
//! volume and ephemeral port — never the development demo's paths or
//! ports — and drives it over raw HTTP with explicit `Host` headers. The
//! debug-build `DEMO_TEST_*` hooks make the clock, a deterministic
//! initialization failure, a crash mid-build, and a paused operation
//! reproducible; the release binary ignores all of them.
//!
//! Proof numbering matches the brief: (1) cold start, (2) restart, (3)
//! reset, (4) SIGTERM, (5) overlap and handoff, (6) midnight overlap, (7)
//! crash during a build, (8) failed initialization, (9) dispatch
//! boundaries, (10) schema drift refused, (11) cleanup and budget.

#![cfg(unix)]

mod support;

use std::fs::File;
use std::os::fd::AsFd as _;
use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use anyhow::ensure;
use leptoskit_deploy_console::control::Action;
use leptoskit_deploy_console::control::Intent;
use leptoskit_deploy_console::control::Phase;
use leptoskit_deploy_console::control::Store;
use rustix::fs::FlockOperation;
use support::ADMIN_HOST;
use support::ADMIN_HOST_V6;
use support::CONNECT_V4;
use support::CONNECT_V6;
use support::DRAIN;
use support::READY;
use support::RETRY;
use support::SIGTERM_BUDGET;
use support::SITE_HOST;
use support::Sse;
use support::appears_within;
use support::clock_hooks;
use support::date_of;
use support::free_port;
use support::generation_dir;
use support::generation_names;
use support::get;
use support::get_on;
use support::noon;
use support::request_raw;
use support::request_with_headers;
use support::retry_hook;
use support::secrets;
use support::set_clock;
use support::spawn;
use support::spawn_env;
use support::spawn_ipv6;
use support::today;
use support::tomorrow;
use support::volume;
use support::wait_bound;
use support::wait_ready;
use support::worker_owner;

// ─── (1) cold start ──────────────────────────────────────────────────

#[test]
fn cold_start_builds_one_generation_with_private_modes() {
	let root = volume();
	let mut server = spawn(&root, free_port(), &[]);
	wait_ready(&mut server, READY);

	let generation = generation_dir(&root);
	let published = generation.join("published");
	assert!(
		published.exists(),
		"the generation is published: {}",
		published.display()
	);
	assert_eq!(generation_names(&root).len(), 1, "exactly one generation");

	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt as _;
		let root_mode = std::fs::metadata(&root).expect("root").permissions().mode();
		let generation_mode = std::fs::metadata(&generation)
			.expect("generation")
			.permissions()
			.mode();
		let secrets_mode = std::fs::metadata(generation.join("secrets.json"))
			.expect("secrets")
			.permissions()
			.mode();
		assert_eq!(root_mode & 0o777, 0o700, "the volume stays 0700");
		assert_eq!(generation_mode & 0o777, 0o700, "the generation stays 0700");
		assert_eq!(secrets_mode & 0o777, 0o600, "secrets.json stays 0600");
	}
}

#[test]
fn bad_roots_are_refused_with_a_nonzero_exit() {
	// A symlinked root would point the checks at a directory they never
	// saw.
	#[cfg(unix)]
	{
		let target = volume();
		let holder = tempfile::tempdir().expect("a temporary directory").keep();
		let link = holder.join("demo-state");
		std::os::unix::fs::symlink(&target, &link).expect("the symlink is created");
		let mut server = spawn(&link, free_port(), &[]);
		let status = server
			.wait_within(Duration::from_secs(15))
			.expect("the process refuses a symlinked root and exits");
		assert!(!status.success(), "a symlinked root is refused: {status:?}");
	}

	// A world-readable root fails the 0700 check.
	let loose = volume();
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt as _;
		std::fs::set_permissions(&loose, std::fs::Permissions::from_mode(0o755))
			.expect("the mode changes");
		let mut server = spawn(&loose, free_port(), &[]);
		let status = server
			.wait_within(Duration::from_secs(15))
			.expect("the process refuses a loose root and exits");
		assert!(!status.success(), "a 0755 root is refused: {status:?}");
	}
}

// ─── (2) same-day restart ────────────────────────────────────────────

#[test]
fn a_killed_process_reopens_the_same_generation_without_reseeding() -> Result<()> {
	let root = volume();
	let clock = root.join("clock");
	let mut server = spawn(&root, free_port(), &clock_hooks(&clock));
	wait_ready(&mut server, READY);

	let before_secrets = secrets(&root);
	let before_jwks = get(server.port, SITE_HOST, "/oauth/jwks").1;
	let before_count = count_events_in_db(&root)?;

	server.kill();
	assert!(!server.alive(), "the process is dead after SIGKILL");

	let port = server.port;
	let mut restarted = spawn(&root, port, &clock_hooks(&clock));
	wait_ready(&mut restarted, READY);

	let after_secrets = secrets(&root);
	assert_eq!(
		before_secrets, after_secrets,
		"a restart keeps the generation's credentials byte for byte"
	);
	assert_eq!(
		before_jwks,
		get(restarted.port, SITE_HOST, "/oauth/jwks").1,
		"the identity provider's keys are unchanged"
	);
	let after_count = count_events_in_db(&root)?;
	assert_eq!(
		before_count, after_count,
		"the seeded month is not re-sent on reopen"
	);
	assert_eq!(
		generation_names(&root).len(),
		1,
		"no second generation is built for the same day"
	);
	Ok(())
}

/// Counts the `events` rows in the generation's analytics database,
/// read-only: the durable side of the seed barrier, as the service's own
/// read route would report it.
fn count_events_in_db(root: &Path) -> Result<i64> {
	let dir = generation_dir(root);
	let connection = rusqlite::Connection::open_with_flags(
		dir.join("analytics.sqlite3"),
		rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
	)?;
	let count: i64 = connection.query_row("SELECT count(*) FROM events", [], |row| row.get(0))?;
	Ok(count)
}

// ─── (2b) a mutable day ──────────────────────────────────────────────

/// Ingests one real visitor event through the live service's own batch
/// route, signed with the generation's real token, and returns the
/// service's typed answer: `(status, the capture result as JSON)`.
fn ingest_one_visit(port: u16, token: &str) -> (u16, serde_json::Value) {
	let now = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.expect("the clock is sane")
		.as_millis() as i64;
	let batch = leptoskit_analytics::wire::Batch {
		sent_at: now,
		contexts: vec![vec![(
			"$lib".to_owned(),
			leptoskit_analytics::wire::Value::Text("demo-service-test".to_owned()),
		)]],
		entries: vec![leptoskit_analytics::wire::Entry {
			uuid: leptoskit_analytics::wire::uuid_v7(now, [9_u8; 10]),
			distinct_id: "restart-proof-visitor".to_owned(),
			session_id: Some(leptoskit_analytics::wire::uuid_v7(now, [7_u8; 10])),
			context: 0,
			body: leptoskit_analytics::wire::Body::Event {
				name: "$pageview".to_owned(),
				properties: vec![(
					"$pathname".to_owned(),
					leptoskit_analytics::wire::Value::Text("/restart-proof".to_owned()),
				)],
			},
		}],
	};
	let path = "/_leptoskit/services/analytics/v1/batch";
	let (status, body) = support::post(
		port,
		&format!("{ADMIN_HOST}:{port}"),
		path,
		&[
			("Authorization", &format!("Bearer {token}")),
			("Content-Type", "application/octet-stream"),
		],
		&leptoskit_analytics::wire::encode(&batch),
	);
	let receipt = serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
	(status, receipt)
}

/// The published marker is a historical initialization receipt, not an
/// invariant over today's editable analytics: visitor events land, an
/// overlapping candidate opens the changed day, a same-day SIGKILL
/// restart keeps keys, extra rows, and the marker without reseeding, and
/// a legitimate analytics deletion does not make the day unopenable.
#[test]
fn a_published_generation_survives_visitor_changes_and_deletions_across_restarts() -> Result<()> {
	let root = volume();
	let clock = root.join("clock");
	set_clock(&clock, noon(0));

	let port = free_port();
	let mut server = spawn(&root, port, &clock_hooks(&clock));
	wait_ready(&mut server, READY);

	let marker_path = generation_dir(&root).join("published");
	let marker_before = std::fs::read(&marker_path)?;
	let secrets_before = secrets(&root);
	let seeded = count_events_in_db(&root)?;

	// A visitor's event, through the live service's own ingestion.
	let token = secrets_before["analytics_token"]
		.as_str()
		.expect("the analytics token is in the secrets")
		.to_owned();
	let (status, receipt) = ingest_one_visit(port, &token);
	assert_eq!(status, 200, "the live service accepts the visit: {receipt}");
	assert_eq!(
		receipt["stored"].as_i64(),
		Some(1),
		"the service's own answer says it stored exactly the one event: {receipt}"
	);
	assert_eq!(
		count_events_in_db(&root)?,
		seeded + 1,
		"the visit is durably in the events table"
	);

	// An overlapping candidate opens the same, now-changed generation.
	let mut candidate = spawn(&root, free_port(), &clock_hooks(&clock));
	assert_eq!(
		wait_ready(&mut candidate, READY),
		200,
		"a candidate opens the day a visitor already changed"
	);
	candidate.terminate_within(SIGTERM_BUDGET);

	// Same-day SIGKILL and restart: keys, the extra row, and the marker
	// are all kept, and nothing reseeds.
	server.kill();
	assert!(!server.alive(), "the process is dead after SIGKILL");
	let mut restarted = spawn(&root, port, &clock_hooks(&clock));
	assert_eq!(
		wait_ready(&mut restarted, READY),
		200,
		"a restart reopens the changed generation without reseeding"
	);
	assert_eq!(
		secrets(&root),
		secrets_before,
		"the credentials are unchanged"
	);
	assert_eq!(
		std::fs::read(&marker_path)?,
		marker_before,
		"the published marker is untouched: a receipt, not a live count"
	);
	assert_eq!(
		count_events_in_db(&root)?,
		seeded + 1,
		"neither a reseed nor a loss"
	);

	// A legitimate analytics deletion lowers the count below the receipt:
	// the day must still open. The test arranges the durable post-deletion
	// state directly, as the service's own delete path leaves it.
	restarted.kill();
	let connection = rusqlite::Connection::open(generation_dir(&root).join("analytics.sqlite3"))?;
	let deleted = connection.execute(
		"DELETE FROM events WHERE rowid IN (SELECT rowid FROM events ORDER BY rowid LIMIT 2)",
		[],
	)?;
	ensure!(deleted == 2, "the deletion removed the rows it names");
	drop(connection);

	let mut after_deletion = spawn(&root, port, &clock_hooks(&clock));
	assert_eq!(
		wait_ready(&mut after_deletion, READY),
		200,
		"a day holding fewer events than its receipt still opens"
	);
	after_deletion.terminate_within(SIGTERM_BUDGET);
	Ok(())
}

// ─── (3) reset ───────────────────────────────────────────────────────

#[test]
fn a_reset_replaces_the_image_and_collects_what_nobody_holds() -> Result<()> {
	let root = volume();
	let clock = root.join("clock");
	set_clock(&clock, noon(0));

	let port = free_port();
	let mut server = spawn(&root, port, &clock_hooks(&clock));
	wait_ready(&mut server, READY);

	// The generation event stream, open while the day turns.
	let (mut stream, first_event) = Sse::open(port, SITE_HOST, "/demo/generation");
	assert_eq!(first_event.0, "generation", "the stream opens with the day");

	// What the old day looks like, so retention and fresh credentials
	// have something to be checked against.
	let old_day = today();
	let old_generation = generation_dir(&root);
	let old_secrets = secrets(&root);

	// Plant a day-before-yesterday generation, published but unheld, and
	// a legacy unpublished leftover: both are collectable.
	plant_generation(&root, &format!("{}.f1", day_before(2)), true);
	plant_generation(
		&root,
		&format!("{}.building-leftover", day_before(2)),
		false,
	);
	assert_eq!(generation_names(&root).len(), 3, "today plus two plants");

	// Midnight.
	set_clock(&clock, noon(1));

	let pid = server.pid();
	assert!(
		stream.closes_within(DRAIN),
		"the open generation stream closes during the reset"
	);

	// The same process image serves the new day: execve, not a restart.
	assert!(
		server.alive(),
		"the process is still running after the reset"
	);
	assert_eq!(server.pid(), pid, "the PID survives the exec");

	// The new day builds and publishes in the same PID.
	let new_generation = root.join("gen").join(format!(
		"{}.f{}",
		tomorrow_name(),
		demo_service::marker::GENERATION_FORMAT
	));
	ensure!(
		appears_within(&new_generation.join("published"), Duration::from_secs(90)),
		"the new day's generation is published: {}",
		new_generation.display()
	);
	assert_eq!(wait_ready(&mut server, READY), 200, "the new day is ready");
	assert_ne!(
		old_generation, new_generation,
		"a new generation directory is built"
	);
	assert_ne!(
		old_secrets,
		serde_json::from_slice::<serde_json::Value>(
			&std::fs::read(new_generation.join("secrets.json")).expect("the new day's secrets")
		)
		.expect("the new secrets parse"),
		"the new day runs on fresh credentials"
	);

	let names = generation_names(&root);
	assert!(
		names.iter().any(|name| name.starts_with(&old_day)),
		"yesterday is retained: {names:?}"
	);
	assert_eq!(
		names.len(),
		2,
		"the plants are collected, today and yesterday remain: {names:?}"
	);

	// The stream reconnects into the new day.
	assert_eq!(
		wait_ready(&mut server, READY),
		200,
		"ready before reconnect"
	);
	let (_reconnected, (_, data)) = Sse::open(port, SITE_HOST, "/demo/generation");
	assert!(
		data.contains(&tomorrow_name()),
		"the stream reports the new day: {data}"
	);

	// On Linux, no descriptor survives exec pointing into the old day.
	#[cfg(target_os = "linux")]
	assert_no_descriptors_into(&old_generation, pid);

	server.terminate_within(SIGTERM_BUDGET);
	Ok(())
}

fn tomorrow_name() -> String {
	tomorrow()
}

fn day_before(days: i64) -> String {
	date_of(noon(0) - days * 86_400_000)
}

/// Plants a generation directory for collection proofs: a lifetime lock
/// file, some bytes, and optionally a valid marker. `name` is the full
/// directory name, `<day>.f<format>`.
fn plant_generation(root: &Path, name: &str, published: bool) -> std::path::PathBuf {
	let dir = root.join("gen").join(name);
	std::fs::create_dir_all(&dir).expect("the plant is created");
	std::fs::write(dir.join("lifetime.lock"), b"").expect("the lock file exists");
	std::fs::write(dir.join("leftover.sqlite3"), vec![0_u8; 4096]).expect("the bytes exist");

	if published {
		let marker = demo_service::marker::Published {
			format: demo_service::marker::GENERATION_FORMAT,
			fingerprint: demo_service::marker::Published::current_fingerprint()
				.expect("the fingerprint is computable"),
			day: name.split_once(".f").expect("a day.f name").0.to_owned(),
			seed_events: 1,
		};
		let text = serde_json::to_vec_pretty(&marker).expect("the marker serializes");
		std::fs::write(dir.join("published"), text).expect("the marker is written");
	}
	dir
}

/// Linux only: every descriptor the live process holds must be outside
/// the retired generation's directory.
#[cfg(target_os = "linux")]
fn assert_no_descriptors_into(old_generation: &Path, pid: u32) {
	let descriptors = std::fs::read_dir(format!("/proc/{pid}/fd"))
		.expect("the live process's descriptors are listable");
	for entry in descriptors.filter_map(Result::ok) {
		let target = std::fs::read_link(entry.path()).unwrap_or_default();
		let target = target.to_string_lossy();
		assert!(
			!target.starts_with(&old_generation.to_string_lossy()),
			"descriptor {} still points into the retired generation: {target}",
			entry.path().display()
		);
	}
}

// ─── (4) SIGTERM ─────────────────────────────────────────────────────

#[test]
fn a_sigterm_with_a_stream_open_exits_zero_within_eight_seconds() {
	let root = volume();
	let clock = root.join("clock");
	set_clock(&clock, noon(0));

	let port = free_port();
	let mut server = spawn(&root, port, &clock_hooks(&clock));
	wait_ready(&mut server, READY);

	let (mut stream, _) = Sse::open(port, SITE_HOST, "/demo/generation");

	let started = std::time::Instant::now();
	let status = server.terminate_within(SIGTERM_BUDGET);
	let elapsed = started.elapsed();

	let status = status.expect("SIGTERM ends the process within the budget");
	assert!(
		status.success(),
		"SIGTERM exits 0, got {status:?} after {elapsed:?}"
	);
	assert!(
		elapsed <= SIGTERM_BUDGET,
		"the stop took {elapsed:?}, over the {SIGTERM_BUDGET:?} budget"
	);
	assert!(
		stream.closes_within(DRAIN),
		"the stream is closed by the drain"
	);
}

// ─── (5) overlap and handoff ─────────────────────────────────────────

#[test]
fn an_overlap_hands_the_worker_over_and_completes_operations() -> Result<()> {
	let root = volume();
	let clock = root.join("clock");
	set_clock(&clock, noon(0));
	let hooks = [
		clock_hooks(&clock),
		vec![
			(
				"DEMO_TEST_PAUSE_OP",
				root.join("pause").display().to_string(),
			),
			(
				"DEMO_TEST_OP_STARTED",
				root.join("op-started").display().to_string(),
			),
		],
	]
	.concat();

	let port_a = free_port();
	let mut active = spawn(&root, port_a, &hooks);
	wait_ready(&mut active, READY);

	let port_b = free_port();
	let mut candidate = spawn(&root, port_b, &hooks);
	wait_ready(&mut candidate, READY);

	// One generation, two processes, one worker.
	assert_eq!(generation_names(&root).len(), 1, "one shared generation");
	let worker_pid = worker_owner(&root).expect("the worker owner is written");
	assert_eq!(
		worker_pid,
		active.pid().to_string(),
		"the first server runs the worker"
	);

	// Pause operations, then enqueue one through the shared store: the
	// active process claims it and holds it mid-flight.
	std::fs::write(root.join("pause"), b"").expect("the pause file is written");
	let store = Store::open(&generation_dir(&root).join("console.sqlite3"))?;
	let abandoned = store.enqueue(
		"notes-production",
		&Intent {
			expected_revision: 0,
			idempotency_key: "handoff-abandoned".to_owned(),
			action: Action::Status,
		},
	)?;
	ensure!(
		appears_within(&root.join("op-started"), Duration::from_secs(10)),
		"the active process claims the operation and pauses mid-flight"
	);

	// The active process stops. The handoff happens only after its exit:
	// the kernel released the worker lock when the process died.
	let status = active
		.terminate_within(SIGTERM_BUDGET)
		.expect("the active process stops within the budget");
	assert!(status.success(), "the stop is clean: {status:?}");

	std::fs::remove_file(root.join("pause")).expect("operations resume");

	let deadline = std::time::Instant::now() + Duration::from_secs(5);
	while worker_owner(&root).as_deref() != Some(candidate.pid().to_string().as_str()) {
		ensure!(
			std::time::Instant::now() < deadline,
			"the candidate takes the worker lock within seconds of the exit; owner is {:?}",
			worker_owner(&root)
		);
		std::thread::sleep(Duration::from_millis(100));
	}

	// The abandoned operation is interrupted by the new worker, and the
	// next queued operation completes.
	let finished = store
		.operation(&abandoned.id)?
		.expect("the abandoned operation is in the store");
	assert_eq!(
		finished.phase,
		Phase::Interrupted,
		"the abandoned operation ends interrupted, not half-applied"
	);

	let next = store.enqueue(
		"notes-production",
		&Intent {
			expected_revision: finished.revision,
			idempotency_key: "handoff-next".to_owned(),
			action: Action::Status,
		},
	)?;
	let deadline = std::time::Instant::now() + Duration::from_secs(15);
	let next_operation = loop {
		let operation = store
			.operation(&next.id)?
			.expect("the next operation is in the store");
		if !matches!(operation.phase, Phase::Queued | Phase::Running) {
			break operation;
		}
		ensure!(
			std::time::Instant::now() < deadline,
			"the next operation completes under the new worker"
		);
		std::thread::sleep(Duration::from_millis(100));
	};
	assert_eq!(
		next_operation.phase,
		Phase::Succeeded,
		"the candidate's worker runs the next operation: {}",
		next_operation.message
	);

	// The terminal state is durable: a brand-new reader of the shared
	// store sees both outcomes, which is exactly what an EventSource
	// reconnect reads as its snapshot. Cross-process live broadcasts are
	// not claimed; see the stream-window proof below for the runtime's
	// reconnection mechanism.
	let fresh = Store::open(&generation_dir(&root).join("console.sqlite3"))?;
	assert_eq!(
		fresh.operation(&abandoned.id)?.expect("abandoned").phase,
		Phase::Interrupted,
		"the interrupted terminal state is durable"
	);
	assert_eq!(
		fresh.operation(&next.id)?.expect("next").phase,
		Phase::Succeeded,
		"the completed terminal state is durable"
	);

	candidate.terminate_within(SIGTERM_BUDGET);
	Ok(())
}

// ─── (6) midnight during an overlap ──────────────────────────────────

#[test]
fn a_simultaneous_midnight_builds_exactly_one_new_generation() -> Result<()> {
	let root = volume();
	let clock = root.join("clock");
	set_clock(&clock, noon(0));
	let hooks = clock_hooks(&clock);

	let mut first = spawn(&root, free_port(), &hooks);
	wait_ready(&mut first, READY);
	let mut second = spawn(&root, free_port(), &hooks);
	wait_ready(&mut second, READY);

	let first_pid = first.pid();
	let second_pid = second.pid();

	set_clock(&clock, noon(1));

	assert!(
		first.alive() && second.alive(),
		"both images survive the reset in place"
	);
	assert_eq!(first.pid(), first_pid, "the first image keeps its PID");
	assert_eq!(second.pid(), second_pid, "the second image keeps its PID");

	// Exactly one builder publishes the new day; the other opens it.
	let new_day = tomorrow();
	let published = root
		.join("gen")
		.join(format!(
			"{}.f{}",
			new_day,
			demo_service::marker::GENERATION_FORMAT
		))
		.join("published");
	ensure!(
		appears_within(&published, Duration::from_secs(120)),
		"the new day's generation is published: {:?}",
		generation_names(&root)
	);

	assert!(
		wait_ready(&mut first, READY) == 200 && wait_ready(&mut second, READY) == 200,
		"both processes serve the new day"
	);

	let names = generation_names(&root);
	let new_generations = names.iter().filter(|name| name.contains(&new_day)).count();
	assert_eq!(
		new_generations, 1,
		"the build lock serializes midnight: exactly one new generation: {names:?}"
	);
	assert_eq!(names.len(), 2, "the old day is retained");

	first.terminate_within(SIGTERM_BUDGET);
	second.terminate_within(SIGTERM_BUDGET);
	Ok(())
}

// ─── (7) crash during a build ────────────────────────────────────────

#[test]
fn a_crash_during_a_build_publishes_nothing_and_the_next_image_recovers() -> Result<()> {
	let root = volume();
	let clock = root.join("clock");
	set_clock(&clock, noon(0));
	let mut hooks = clock_hooks(&clock);
	hooks.push(("DEMO_TEST_CRASH_AFTER_SEED", "1".to_owned()));

	let mut server = spawn(&root, free_port(), &hooks);
	let status = server
		.wait_within(Duration::from_secs(60))
		.expect("the crashed build ends the process");
	assert!(
		!status.success(),
		"an abort is not a clean exit: {status:?}"
	);

	// Nothing was published: the marker never existed. The crash happened
	// *after* the committed seed and its read-back, so the rows are on
	// disk — a crash any earlier would leave nothing to recover from and
	// nothing worth preserving.
	let generation = generation_dir(&root);
	assert!(
		!generation.join("published").exists(),
		"a crashed build leaves no published marker"
	);
	let committed = count_events_in_db(&root)?;
	assert!(
		committed > 0,
		"the deliberate crash happened after the committed seed: {committed} rows are on disk"
	);

	// The next image recovers: the unpublished state is rebuilt only
	// once the crashed initializer's lifetime lock is gone, which it is.
	let mut recovery = spawn(&root, free_port(), &clock_hooks(&clock));
	assert_eq!(
		wait_ready(&mut recovery, READY),
		200,
		"the next image recovers"
	);
	assert!(
		generation_dir(&root).join("published").exists(),
		"the rebuilt generation is published"
	);
	assert_eq!(
		generation_names(&root).len(),
		1,
		"one generation for the day, rebuilt in place"
	);

	recovery.terminate_within(SIGTERM_BUDGET);
	Ok(())
}

// ─── (8) failed initialization ───────────────────────────────────────

#[test]
fn a_failed_initialization_stays_unready_and_retries_in_a_fresh_image() {
	let root = volume();
	let clock = root.join("clock");
	set_clock(&clock, noon(0));
	let fail_file = root.join("fail-init");
	std::fs::write(&fail_file, b"").expect("the fail file is written");
	let mut hooks = clock_hooks(&clock);
	hooks.push(("DEMO_TEST_FAIL_INIT", fail_file.display().to_string()));
	hooks.extend(retry_hook());

	let port = free_port();
	let mut server = spawn(&root, port, &hooks);
	wait_bound(&mut server, READY);

	// The fallback serves: alive, not ready, honest about why.
	let (status, body) = get(port, SITE_HOST, "/readyz");
	assert_eq!(status, 503, "the fallback is not ready");
	assert!(
		body.contains("generation"),
		"the failing check is named: {body}"
	);
	let (health, _) = get(port, SITE_HOST, "/healthz");
	assert_eq!(health, 200, "the process stays up and alive");
	let (page, _) = get(port, SITE_HOST, "/");
	assert_eq!(page, 503, "ordinary pages answer 503 during the failure");

	// Long enough for at least one bounded retry, the fallback holds.
	std::thread::sleep(Duration::from_secs(3));
	assert!(
		server.alive(),
		"the process keeps the fallback rather than exiting"
	);
	assert!(
		!generation_dir(&root).join("published").exists(),
		"nothing publishes during the failure"
	);

	// The failure is lifted; a fresh image — same PID, re-exec — succeeds.
	std::fs::remove_file(&fail_file).expect("the fail file is removed");
	assert_eq!(
		wait_ready(&mut server, RETRY + READY),
		200,
		"the retry becomes ready"
	);
	assert!(
		generation_dir(&root).join("published").exists(),
		"the retry publishes a generation"
	);

	server.terminate_within(SIGTERM_BUDGET);
}

// ─── (9) dispatch boundaries ─────────────────────────────────────────

/// A request that names two authorities — the site's host and a
/// stranger's, here — must not be resolved by taking one of its Host
/// values. The HTTP server may refuse the wire syntax itself (400); the
/// application must refuse the request (421). Either way it is not
/// served, and the proof runs before and after readiness.
fn duplicated_host_is_not_served(port: u16) {
	let duplicate = format!(
		"GET /healthz HTTP/1.1\r\nHost: {SITE_HOST}\r\nHost: stranger.example\r\nUser-Agent: \
		 demo-service-test\r\nAccept: */*\r\nConnection: close\r\n\r\n"
	);
	let (status, body) = request_raw(CONNECT_V4, port, &duplicate);
	assert!(
		status == 400 || status == 421,
		"a duplicated Host is refused, not served: {status} {body}"
	);
}

#[test]
fn dispatch_answers_its_own_authorities_and_refuses_everyone_else() {
	let root = volume();
	let clock = root.join("clock");
	set_clock(&clock, noon(0));

	let port = free_port();
	let mut server = spawn(&root, port, &clock_hooks(&clock));
	wait_bound(&mut server, READY);

	// Before any generation: alive, not ready, Retry-After on pages, and
	// foreign hosts refused even here.
	assert_eq!(get(port, SITE_HOST, "/healthz").0, 200);
	assert_eq!(get(port, SITE_HOST, "/readyz").0, 503);
	assert_eq!(get(port, ADMIN_HOST, "/readyz").0, 503);
	let (_, headers, _) = request_with_headers(port, SITE_HOST, "/");
	assert!(
		headers
			.iter()
			.any(|(name, _)| name.eq_ignore_ascii_case("retry-after")),
		"the fallback answers with Retry-After: {headers:?}"
	);
	assert_eq!(get(port, "stranger.example", "/").0, 421);
	assert_eq!(get(port, "stranger.example", "/healthz").0, 421);
	duplicated_host_is_not_served(port);

	wait_ready(&mut server, READY);

	// After: public hosts serve, the loopback serves only health and the
	// token-authenticated analytics prefix, the analytics prefix is not
	// on the public site, and strangers still get 421.
	assert_eq!(get(port, SITE_HOST, "/").0, 200);
	assert_eq!(get(port, SITE_HOST, "/healthz").0, 200);
	assert_eq!(get(port, SITE_HOST, "/readyz").0, 200);
	assert_eq!(get(port, ADMIN_HOST, "/healthz").0, 200);
	assert_eq!(get(port, ADMIN_HOST, "/readyz").0, 200);
	let admin_home = get(port, ADMIN_HOST, "/").0;
	assert!(
		admin_home == 200 || admin_home == 303,
		"the admin serves its home: {admin_home}"
	);
	let loopback = format!("{ADMIN_HOST}:{port}");
	assert_eq!(
		get(port, &loopback, "/").0,
		404,
		"the loopback host answers no pages"
	);
	assert_eq!(
		get(port, &loopback, "/_leptoskit/admin").0,
		404,
		"the loopback host answers no admin"
	);
	assert_eq!(
		get(
			port,
			SITE_HOST,
			"/_leptoskit/services/analytics/rpc/event_definitions"
		)
		.0,
		404,
		"the analytics prefix is not on the public site"
	);
	assert_eq!(
		get(
			port,
			&loopback,
			"/_leptoskit/services/analytics/rpc/event_definitions"
		)
		.0,
		405,
		"the loopback serves the analytics prefix (405: this was a GET, the route wants POST)"
	);
	assert_eq!(
		get(port, SITE_HOST, "/demo/generation-splash").0,
		404,
		"the site router answers its own 404s"
	);
	assert_eq!(get(port, "stranger.example", "/").0, 421);
	duplicated_host_is_not_served(port);

	server.terminate_within(SIGTERM_BUDGET);
}

/// A misconfigured service refuses before the listener exists: nonzero
/// exit, nothing bound, nothing built. The colliding cases differ from a
/// good configuration only in the spelling the contract says must fold
/// to the same authority — casing, a trailing dot, a default port, a
/// bracketed IPv6 literal — so a refusal proves the normalization
/// happened before the check.
#[test]
fn invalid_configuration_exits_nonzero_without_listening_or_writing_a_generation() {
	let cases: Vec<(&str, Vec<(&str, String)>)> = vec![
		(
			"an ftp site origin on loopback",
			vec![("DEMO_SITE_ORIGIN", "ftp://localhost".to_owned())],
		),
		(
			"a plain-http admin origin off loopback",
			vec![("DEMO_ADMIN_ORIGIN", "http://admin.example".to_owned())],
		),
		(
			"credentials in the site origin",
			vec![("DEMO_SITE_ORIGIN", "https://user@site.example".to_owned())],
		),
		(
			"a path in the site origin",
			vec![("DEMO_SITE_ORIGIN", "https://site.example/app".to_owned())],
		),
		(
			"repeated trailing dots",
			vec![("DEMO_SITE_ORIGIN", "https://site.example..".to_owned())],
		),
		(
			"a bind off loopback",
			vec![("DEMO_BIND", "10.0.0.1:8080".to_owned())],
		),
		(
			"the site and the admin fold to one authority",
			vec![
				("DEMO_SITE_ORIGIN", "https://NOTES.example.:443".to_owned()),
				("DEMO_ADMIN_ORIGIN", "https://notes.example".to_owned()),
			],
		),
	];

	for (name, overrides) in cases {
		let root = volume();
		let port = free_port();
		let mut server = spawn_env(&root, port, &overrides);
		let status = server
			.wait_within(Duration::from_secs(15))
			.unwrap_or_else(|| panic!("{name}: the process refuses the configuration and exits"));
		assert!(!status.success(), "{name} is refused: {status:?}");
		assert!(
			std::net::TcpStream::connect_timeout(
				&std::net::SocketAddr::new(CONNECT_V4, port),
				Duration::from_millis(250)
			)
			.is_err(),
			"{name}: nothing listens"
		);
		assert!(
			generation_names(&root).is_empty(),
			"{name}: no generation is built"
		);
	}
}

/// The site and the bind fold to one IPv6 authority: a bracketed origin
/// and the same `[::1]` bind must collide after normalization, on pain
/// of one authority answering for two. Refused before the listener.
#[test]
fn an_ipv6_site_origin_colliding_with_the_bind_is_refused() {
	let root = volume();
	let port = free_port();
	let overrides = vec![
		("DEMO_BIND", format!("[::1]:{port}")),
		("DEMO_SITE_ORIGIN", format!("http://[::1]:{port}")),
	];
	let mut server = spawn_env(&root, port, &overrides);
	let status = server
		.wait_within(Duration::from_secs(15))
		.expect("the process refuses the configuration and exits");
	assert!(!status.success(), "the collision is refused: {status:?}");
	assert!(
		std::net::TcpStream::connect_timeout(
			&std::net::SocketAddr::new(CONNECT_V4, port),
			Duration::from_millis(250)
		)
		.is_err()
			&& std::net::TcpStream::connect_timeout(
				&std::net::SocketAddr::new(CONNECT_V6, port),
				Duration::from_millis(250)
			)
			.is_err(),
		"nothing listens on the collided authority"
	);
	assert!(generation_names(&root).is_empty(), "no generation is built");
}

/// Whether the machine can bind IPv6 loopback at all. The IPv6 proofs
/// report a missing facility rather than pretend to pass.
fn ipv6_loopback_available() -> bool {
	std::net::TcpListener::bind(std::net::SocketAddr::from((
		std::net::Ipv6Addr::LOCALHOST,
		0,
	)))
	.is_ok()
}

/// Real IPv6 loopback: the bind, an IPv6 admin origin, and bracketed
/// Host headers agree on one canonical spelling — `[::1]`, whatever the
/// header wrote — before and after readiness.
#[test]
fn ipv6_loopback_authorities_route_and_foreign_hosts_are_refused() {
	if !ipv6_loopback_available() {
		eprintln!("skipping: this machine has no IPv6 loopback to bind; the proof needs one");
		return;
	}
	let root = volume();
	let clock = root.join("clock");
	set_clock(&clock, noon(0));

	let port = free_port();
	let mut server = spawn_ipv6(&root, port, &clock_hooks(&clock));
	wait_bound(&mut server, READY);

	let bind_host = format!("[::1]:{port}");
	// The bind authority answers health checks under every bracketed
	// spelling of ::1, before a generation exists.
	assert_eq!(get_on(CONNECT_V6, port, &bind_host, "/healthz").0, 200);
	assert_eq!(
		get_on(
			CONNECT_V6,
			port,
			&format!("[0:0:0:0:0:0:0:1]:{port}"),
			"/healthz"
		)
		.0,
		200,
		"a bracketed equivalent canonicalizes to the same authority"
	);
	assert_eq!(
		get_on(CONNECT_V6, port, ADMIN_HOST_V6, "/readyz").0,
		503,
		"the bare literal names the admin, not ready yet"
	);
	assert_eq!(get_on(CONNECT_V6, port, "stranger.example", "/").0, 421);

	wait_ready(&mut server, READY);

	// After readiness: the bind answers no pages, the admin serves, the
	// site serves through the IPv6 listener, strangers stay refused.
	assert_eq!(get_on(CONNECT_V6, port, &bind_host, "/").0, 404);
	assert_eq!(get_on(CONNECT_V6, port, ADMIN_HOST_V6, "/readyz").0, 200);
	assert_eq!(get_on(CONNECT_V6, port, SITE_HOST, "/").0, 200);
	assert_eq!(get_on(CONNECT_V6, port, "stranger.example", "/").0, 421);

	server.terminate_within(SIGTERM_BUDGET);
}

// ─── (10) schema drift ───────────────────────────────────────────────

#[test]
fn schema_drift_is_refused_before_any_database_changes() -> Result<()> {
	let root = volume();
	let clock = root.join("clock");
	set_clock(&clock, noon(0));
	let port = free_port();
	let mut server = spawn(&root, port, &clock_hooks(&clock));
	wait_ready(&mut server, READY);
	server.terminate_within(SIGTERM_BUDGET);

	// Rewrite the marker as if the schemas had drifted without a format
	// bump: same format, a different fingerprint.
	let generation = generation_dir(&root);
	let marker_path = generation.join("published");
	let mut marker: demo_service::marker::Published =
		serde_json::from_slice(&std::fs::read(&marker_path)?)?;
	marker.fingerprint = "0".repeat(64);
	std::fs::write(&marker_path, serde_json::to_vec_pretty(&marker)?)?;

	let databases = ["site.sqlite3", "analytics.sqlite3", "console.sqlite3"];
	let before = databases
		.iter()
		.map(|database| (*database, file_hash(&generation.join(database))))
		.collect::<Vec<_>>();

	// The process refuses the marker and stays unready; it never opens
	// the databases, so no migration touches them.
	let mut refused = spawn(&root, free_port(), &clock_hooks(&clock));
	std::thread::sleep(Duration::from_secs(4));
	assert_eq!(
		get(refused.port, SITE_HOST, "/readyz").0,
		503,
		"the drifted generation is refused, not opened"
	);
	assert!(
		refused.alive(),
		"the process keeps the fallback and reports"
	);
	let stderr = support::stderr_of(&mut refused);
	assert!(
		stderr.contains("fingerprint"),
		"the refusal names the fingerprint contract: {stderr}"
	);

	for (database, hash) in before {
		assert_eq!(
			file_hash(&generation.join(database)),
			hash,
			"{database} is byte-identical after the refusal"
		);
	}
	// The tampered marker is still the marker: nothing rebuilt over it.
	let after_marker: demo_service::marker::Published =
		serde_json::from_slice(&std::fs::read(&marker_path)?)?;
	assert_eq!(
		after_marker, marker,
		"nothing rebuilt the refused generation"
	);
	Ok(())
}

fn file_hash(path: &Path) -> [u8; 32] {
	use sha2::Digest as _;
	let bytes = std::fs::read(path).expect("the database file is readable");
	sha2::Sha256::digest(&bytes).into()
}

// ─── (10b) a corrupt published marker ────────────────────────────────

/// A published generation whose marker cannot be read back — truncated
/// JSON here — is a fail-closed fault: the process stays unready and
/// reports it, and no database, credential, or marker byte is touched.
/// Only a genuinely absent marker permits the unpublished build path.
#[test]
fn a_corrupt_published_marker_fails_closed_without_touching_the_generation() -> Result<()> {
	let root = volume();
	let clock = root.join("clock");
	set_clock(&clock, noon(0));

	let port = free_port();
	let mut server = spawn(&root, port, &clock_hooks(&clock));
	wait_ready(&mut server, READY);
	server.terminate_within(SIGTERM_BUDGET);

	let generation = generation_dir(&root);
	let marker_path = generation.join("published");
	let intact = std::fs::read(&marker_path)?;
	let truncated = intact[..intact.len() / 2].to_vec();
	std::fs::write(&marker_path, &truncated)?;

	let state = [
		"site.sqlite3",
		"analytics.sqlite3",
		"console.sqlite3",
		"secrets.json",
	];
	let before: Vec<(&str, [u8; 32])> = state
		.iter()
		.map(|name| (*name, file_hash(&generation.join(name))))
		.collect();

	let mut refused = spawn(&root, free_port(), &clock_hooks(&clock));

	// Long enough that a rebuild — the old behavior — would have served.
	// This process must keep the fallback instead, through at least one
	// bounded re-exec retry.
	let deadline = std::time::Instant::now() + Duration::from_secs(70);
	loop {
		assert!(refused.alive(), "the process keeps the fallback");
		assert_ne!(
			get(refused.port, SITE_HOST, "/readyz").0,
			200,
			"a corrupt marker must never serve, and never rebuild over the published generation"
		);
		if std::time::Instant::now() > deadline {
			break;
		}
		std::thread::sleep(Duration::from_millis(250));
	}
	let stderr = support::stderr_of(&mut refused);
	assert!(
		stderr.contains("published") || stderr.contains("marker"),
		"the refusal names the marker: {stderr}"
	);

	for (name, hash) in before {
		assert_eq!(
			file_hash(&generation.join(name)),
			hash,
			"{name} is byte-identical after the refusal"
		);
	}
	assert_eq!(
		std::fs::read(&marker_path)?,
		truncated,
		"nothing rewrote or removed the corrupt marker"
	);
	Ok(())
}

// ─── (11) cleanup and budget ─────────────────────────────────────────

/// Retention recognizes a published generation structurally — a marker
/// that names its own directory with seed evidence — regardless of the
/// format or fingerprint this binary serves. A recent foreign-format
/// generation is the same-day rollback target after a format bump, so it
/// survives collection with no live holder; a marker that cannot be read
/// is never a deletion reason; only genuinely expired published
/// generations and genuinely unpublished leftovers are collected.
#[test]
fn collection_keeps_recent_published_generations_and_never_deletes_what_it_cannot_read()
-> Result<()> {
	let root = volume();
	let clock = root.join("clock");
	set_clock(&clock, noon(0));

	// Yesterday in a foreign format: published, recent, unheld.
	let foreign_name = format!("{}.f2", day_before(1));
	let foreign = plant_generation(&root, &foreign_name, false);
	let foreign_marker = serde_json::to_vec_pretty(&demo_service::marker::Published {
		format: 2,
		fingerprint: "0".repeat(64),
		day: day_before(1),
		seed_events: 900,
	})?;
	std::fs::write(foreign.join("published"), &foreign_marker)?;

	// A recent generation of this format whose marker is truncated:
	// unreadable means kept, counted, and reported — never deleted.
	let corrupt_name = format!("{}.f1", day_before(1));
	let corrupt = plant_generation(&root, &corrupt_name, false);
	let truncated_marker = b"{\"format\": 1, \"fingerprint\": \"aa".to_vec();
	std::fs::write(corrupt.join("published"), &truncated_marker)?;

	// An expired published generation of this format and an unpublished
	// leftover: both genuinely collectable.
	plant_generation(&root, &format!("{}.f1", day_before(2)), true);
	plant_generation(
		&root,
		&format!("{}.building-leftover", day_before(2)),
		false,
	);

	let mut server = spawn(&root, free_port(), &clock_hooks(&clock));
	wait_ready(&mut server, READY);

	let names = generation_names(&root);
	assert!(
		names.contains(&foreign_name),
		"the foreign-format rollback target survives collection: {names:?}"
	);
	assert!(
		names.contains(&corrupt_name),
		"the unreadable marker survives collection: {names:?}"
	);
	assert!(
		!names.iter().any(|name| name.starts_with(&day_before(2))),
		"the expired published generation and the leftover are collected: {names:?}"
	);
	assert_eq!(
		std::fs::read(foreign.join("published"))?,
		foreign_marker,
		"the foreign marker is untouched"
	);
	assert_eq!(
		std::fs::read(corrupt.join("published"))?,
		truncated_marker,
		"the truncated marker is untouched"
	);

	server.terminate_within(SIGTERM_BUDGET);
	Ok(())
}

#[test]
fn cleanup_refuses_a_held_lifetime_lock_and_the_budget_fails_honestly() {
	let root = volume();
	let clock = root.join("clock");
	set_clock(&clock, noon(0));

	// Two old generations whose lifetime locks the test holds: a live
	// holder somewhere else is exactly what these stand in for.
	let held_a = plant_generation(&root, &format!("{}.f1", day_before(2)), true);
	let held_b = plant_generation(&root, &format!("{}.f1", day_before(3)), true);
	let locks = [hold_shared(&held_a), hold_shared(&held_b)];

	let mut server = spawn(&root, free_port(), &clock_hooks(&clock));
	wait_ready(&mut server, READY);

	let names = generation_names(&root);
	assert!(
		names.iter().any(|name| name.starts_with(&day_before(2))),
		"the held generation survives collection: {names:?}"
	);
	assert!(
		names.iter().any(|name| name.starts_with(&day_before(3))),
		"the second held generation survives: {names:?}"
	);

	server.terminate_within(SIGTERM_BUDGET);
	drop(locks);

	// With the locks gone, the next start collects them.
	let mut next = spawn(&root, free_port(), &clock_hooks(&clock));
	assert_eq!(
		wait_ready(&mut next, READY),
		200,
		"the volume is servable again"
	);
	let names = generation_names(&root);
	assert!(
		!names.iter().any(|name| name.starts_with(&day_before(2))),
		"the released generation is collected: {names:?}"
	);
	next.terminate_within(SIGTERM_BUDGET);
}

#[test]
fn an_uncollectable_volume_stays_unready_with_an_honest_error() -> Result<()> {
	let root = volume();
	let clock = root.join("clock");
	set_clock(&clock, noon(0));

	// Five old generations, all held: more than the documented budget of
	// four directories, and unremovable by their locks alone.
	let mut held = Vec::new();
	for days in 2..=6 {
		let dir = plant_generation(&root, &format!("{}.f1", day_before(days)), true);
		held.push(hold_shared(&dir));
	}

	let mut server = spawn(&root, free_port(), &clock_hooks(&clock));

	// The process stays up, unready, and says why rather than unlinking
	// live databases or growing without bound.
	let published = root
		.join("gen")
		.join(format!(
			"{}.f{}",
			today(),
			demo_service::marker::GENERATION_FORMAT
		))
		.join("published");
	ensure!(
		appears_within(&published, Duration::from_secs(90)),
		"today's generation builds before the collection verdict"
	);
	std::thread::sleep(Duration::from_secs(2));
	assert_eq!(get(server.port, SITE_HOST, "/readyz").0, 503, "not ready");
	assert!(server.alive(), "the process keeps the fallback");
	let stderr = support::stderr_of(&mut server);
	assert!(
		stderr.contains("budget") || stderr.contains("cannot be collected"),
		"the honest budget error is reported: {stderr}"
	);

	for day in (2..=6).map(day_before) {
		assert!(
			generation_names(&root)
				.iter()
				.any(|name| name.starts_with(&day)),
			"the held generation {day} was not unlinked"
		);
	}

	drop(held);
	Ok(())
}

/// Holds a generation's lifetime lock shared, as a live server elsewhere
/// would.
fn hold_shared(dir: &Path) -> File {
	let mut options = std::fs::OpenOptions::new();
	options.read(true).write(true).create(true);
	let file = options
		.open(dir.join("lifetime.lock"))
		.expect("the lifetime lock opens");
	rustix::fs::flock(file.as_fd(), FlockOperation::LockShared).expect("the shared lock is taken");
	file
}
