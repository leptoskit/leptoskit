//! The inherited-descriptor guard's proofs, against the real binary.
//!
//! On Linux, the process must refuse to start while a nonstandard file
//! descriptor survives into it — including one whose target merely ends
//! in `/fd`: a spare open of `/proc/self/fd` itself, or a file or
//! directory named `fd`. Every proof runs an isolated helper process — a
//! `sh` whose fixed source inherits one descriptor and then `exec`s the
//! real binary, with the path and the binary passed as positional
//! arguments, never interpolated into the source. The test holds the
//! child's bind for the child's whole life, so a process that passes the
//! guard exits at the bind and one that fails it exits before the bind:
//! the refusal message tells the two outcomes apart without ever
//! seeding a volume.
//!
//! The proofs are Linux-only, where `/proc/self/fd` exists; the harness
//! they drive compiles on Unix, so the macOS compiler and lints
//! still check it.

#![cfg(unix)]
#![allow(
	dead_code,
	reason = "the proofs run on Linux only; the harness still typechecks on Unix"
)]

mod support;

use std::io::Read as _;
use std::net::SocketAddr;
use std::net::TcpListener;
use std::path::Path;
use std::path::PathBuf;
use std::process::Child;
use std::process::ChildStderr;
use std::process::Command;
use std::process::ExitStatus;
use std::process::Stdio;
use std::thread;
use std::time::Duration;
use std::time::Instant;

#[cfg(target_os = "linux")]
use support::volume;

/// The fixed helper source: open `$1` as descriptor 3, then replace the
/// shell with `$2` — the real binary — which inherits the descriptor.
const INHERIT_AND_EXEC: &str = "exec 3< \"$1\"; exec \"$2\"";

/// How long a child has to refuse or to reach the held bind, the bound
/// the other refusal proofs use.
const START: Duration = Duration::from_secs(15);

/// One child's outcome: how it exited and everything it wrote to
/// standard error.
#[derive(Debug)]
struct Start {
	status: ExitStatus,
	stderr: String,
}

/// Plants `name` inside the volume — a file, or a directory — for a
/// proof to hand to the helper shell as the handle to inherit.
fn plant(root: &Path, name: &str, directory: bool) -> PathBuf {
	let handle = root.join(name);
	if directory {
		std::fs::create_dir(&handle).expect("the directory is created");
	} else {
		std::fs::write(&handle, b"not a descriptor exemption").expect("the file is written");
	}
	handle
}

/// The bind the test holds, at an address no other proof uses.
fn held_bind() -> (TcpListener, SocketAddr) {
	let bind = TcpListener::bind(("127.0.0.1", 0)).expect("the test holds a bind");
	let address = bind.local_addr().expect("the held bind has an address");
	(bind, address)
}

/// Runs the real binary in an isolated process, with `inherited` opened
/// as an extra descriptor 3 when given. The test holds the bind
/// throughout, so a passing guard dies at the bind and a failing guard
/// dies before it.
fn exec_with_descriptor(root: &Path, address: SocketAddr, inherited: Option<&Path>) -> Start {
	let variables = support::env(root, address, &[]);
	let mut command = match inherited {
		Some(inherited) => {
			let mut command = Command::new("sh");
			command
				.arg("-c")
				.arg(INHERIT_AND_EXEC)
				.arg("demo-descriptor-guard")
				.arg(inherited)
				.arg(env!("CARGO_BIN_EXE_demo_service"));
			command
		}
		None => support::command_with_clean_fds(env!("CARGO_BIN_EXE_demo_service")),
	};
	command.envs(variables).stderr(Stdio::piped());
	let mut child = command.spawn().expect("the child process spawns");
	let stderr = child.stderr.take().expect("the child's stderr is piped");
	let status = wait_within(&mut child);
	Start {
		status,
		stderr: read_all(stderr),
	}
}

/// Waits for the child within the bound; a child that does neither
/// refuse nor reach the bind has hung, and the proof fails rather than
/// wait forever.
fn wait_within(child: &mut Child) -> ExitStatus {
	let deadline = Instant::now() + START;
	loop {
		match child.try_wait().expect("the child's status is readable") {
			Some(status) => return status,
			None if Instant::now() < deadline => thread::sleep(Duration::from_millis(50)),
			None => {
				let _ = child.kill();
				let _ = child.wait();
				panic!("the child neither refused nor reached the bind within 15 seconds");
			}
		}
	}
}

/// Reads a pipe to its end, after its writer has exited.
fn read_all(mut stderr: ChildStderr) -> String {
	let mut collected = String::new();
	stderr
		.read_to_string(&mut collected)
		.expect("the child's stderr is readable");
	collected
}

/// Asserts the child refused at the guard, before the bind: a nonzero
/// exit, the guard's message naming the inherited descriptor 3 with its
/// target, and no bind error.
fn assert_refused_at_the_guard(started: &Start, address: SocketAddr) {
	assert!(
		!started.status.success(),
		"an inherited descriptor refuses to start: {started:?}"
	);
	assert!(
		started
			.stderr
			.contains("inherited file descriptors would survive exec"),
		"the refusal names the guard, not a later failure: {started:?}"
	);
	assert!(
		started.stderr.contains("(3, \""),
		"the refusal names descriptor 3 with its actual target: {started:?}"
	);
	assert!(
		!started.stderr.contains(&format!("binding {address}")),
		"the refusal happens before anything serves: {started:?}"
	);
}

/// A normal process — no inherited nonstandard descriptor — gets past
/// the guard and dies at the bind the test holds: its refusal names the
/// bind, never the guard. The enumeration's own `/proc/self/fd` handle
/// closed with its `read_dir`, so it is not among what the guard
/// inspects, and nothing this check opens can trip it.
#[cfg(target_os = "linux")]
#[test]
fn a_normal_process_gets_past_the_descriptor_guard() {
	let root = volume();
	let (_bind, address) = held_bind();

	let started = exec_with_descriptor(&root, address, None);

	assert!(
		!started.status.success(),
		"the child exits at the held bind: {started:?}"
	);
	assert!(
		started.stderr.contains(&format!("binding {address}")),
		"the child got all the way to the bind: {started:?}"
	);
	assert!(
		!started.stderr.contains("inherited file descriptors"),
		"the guard passed: {started:?}"
	);
}

/// A spare ordinary file is refused before anything serves.
#[cfg(target_os = "linux")]
#[test]
fn an_inherited_ordinary_file_is_refused_before_serving() {
	let root = volume();
	let (_bind, address) = held_bind();
	let file = plant(&root, "note.txt", false);

	let started = exec_with_descriptor(&root, address, Some(&file));

	assert_refused_at_the_guard(&started, address);
	assert!(
		started.stderr.contains(file.to_string_lossy().as_ref()),
		"the refusal names the actual handle: {started:?}"
	);
}

/// A file whose final name is `fd` is refused like any other handle: no
/// target string is an exemption.
#[cfg(target_os = "linux")]
#[test]
fn an_inherited_file_named_fd_is_refused_before_serving() {
	let root = volume();
	let (_bind, address) = held_bind();
	let file = plant(&root, "fd", false);

	let started = exec_with_descriptor(&root, address, Some(&file));

	assert_refused_at_the_guard(&started, address);
	assert!(
		started.stderr.contains(file.to_string_lossy().as_ref()),
		"the refusal names the actual handle: {started:?}"
	);
}

/// A directory whose final name is `fd` is refused like any other
/// handle: no target string is an exemption.
#[cfg(target_os = "linux")]
#[test]
fn an_inherited_directory_named_fd_is_refused_before_serving() {
	let root = volume();
	let (_bind, address) = held_bind();
	let directory = plant(&root, "fd", true);

	let started = exec_with_descriptor(&root, address, Some(&directory));

	assert_refused_at_the_guard(&started, address);
	assert!(
		started
			.stderr
			.contains(directory.to_string_lossy().as_ref()),
		"the refusal names the actual handle: {started:?}"
	);
}

/// A spare open of `/proc/self/fd` — the very directory the guard
/// enumerates — is refused: the exemption is the closure of the
/// enumeration's own handle, never a target string, so another handle
/// to the same directory gets no exemption.
#[cfg(target_os = "linux")]
#[test]
fn an_inherited_handle_to_the_descriptor_directory_is_refused_before_serving() {
	let root = volume();
	let (_bind, address) = held_bind();

	let started = exec_with_descriptor(&root, address, Some(Path::new("/proc/self/fd")));

	assert_refused_at_the_guard(&started, address);
}
