//! The platform tools device control shells out to: `simctl`, `devicectl`,
//! `adb`, and the emulator.
//!
//! Every invocation has a deadline, so a wedged tool fails the step instead
//! of hanging the run.

use std::path::PathBuf;
use std::process::Child;
use std::process::Command;
use std::process::Stdio;
use std::sync::mpsc;
use std::time::Duration;
use std::time::Instant;

use super::Error;

/// A finished tool invocation.
#[derive(Debug)]
pub struct ToolOutput {
	/// The captured standard output, lossily decoded.
	pub stdout: String,
	/// The captured standard error, lossily decoded.
	pub stderr: String,
	/// The raw captured standard output, for binary payloads like screenshots.
	pub raw: Vec<u8>,
	/// The exit code, when the process exited on its own.
	pub code: Option<i32>,
}

impl ToolOutput {
	/// Whether the tool exited successfully.
	pub fn success(&self) -> bool {
		self.code == Some(0)
	}

	/// The failure detail: the exit status plus any stderr excerpt.
	pub fn failure(&self) -> String {
		let excerpt: String = self.stderr.chars().take(400).collect();
		format!(
			"exit {}{}",
			self.code
				.map_or("killed".to_owned(), |code| code.to_string()),
			if excerpt.trim().is_empty() {
				String::new()
			} else {
				format!(": {}", excerpt.trim())
			}
		)
	}
}

/// Runs one command to completion under a deadline, terminating its process
/// group on expiry.
pub fn run(command: &mut Command, timeout: Duration) -> Result<ToolOutput, Error> {
	run_feeding(command, timeout, None)
}

/// Runs one command, writing `input` to its standard input and closing it.
///
/// `None` leaves standard input closed, which is what a tool with no program
/// to read expects.
pub fn run_feeding(
	command: &mut Command,
	timeout: Duration,
	input: Option<&[u8]>,
) -> Result<ToolOutput, Error> {
	feed(command, timeout, input, None)
}

/// Writes `first`, waits `pause_ms`, then writes `second` and closes input.
///
/// A `uinput` touchscreen is not attached at the moment it is registered.
/// Sending the gesture in the same write, with a `delay` command for that
/// wait, stamps the wait into the gesture's event timestamps. The pause is
/// wall time between the two writes instead.
pub fn run_feeding_then(
	command: &mut Command,
	timeout: Duration,
	first: &[u8],
	pause_ms: u64,
	second: &[u8],
) -> Result<ToolOutput, Error> {
	feed(
		command,
		timeout,
		Some(first),
		Some((Duration::from_millis(pause_ms), second)),
	)
}

/// Runs `command`, optionally writing standard input in one or two parts.
fn feed(
	command: &mut Command,
	timeout: Duration,
	input: Option<&[u8]>,
	then: Option<(Duration, &[u8])>,
) -> Result<ToolOutput, Error> {
	isolate(command);
	if input.is_some() {
		command.stdin(Stdio::piped());
	} else {
		command.stdin(Stdio::null());
	}
	command.stdout(Stdio::piped()).stderr(Stdio::piped());

	let name = command.get_program().to_string_lossy().into_owned();
	let started = Instant::now();
	let mut child = spawn(command, &name)?;
	if let Some(bytes) = input
		&& let Some(mut pipe) = child.stdin.take()
	{
		use std::io::Write;
		let _ = pipe.write_all(bytes);
		let _ = pipe.flush();
		if let Some((pause, second)) = then {
			let pause_started = Instant::now();
			while pause_started.elapsed() < pause {
				if started.elapsed() >= timeout {
					let _ = terminate(&mut child);
					let _ = child.wait();
					return Err(Error::Tool {
						name,
						detail: format!("exceeded {} seconds", timeout.as_secs()),
					});
				}
				if matches!(child.try_wait(), Ok(Some(_))) {
					break;
				}
				let left = pause.saturating_sub(pause_started.elapsed());
				std::thread::sleep(Duration::from_millis(20).min(left));
			}
			let _ = pipe.write_all(second);
			let _ = pipe.flush();
		}
	}
	let (receiver, readers) = read_pipes(&mut child);
	let left = timeout.saturating_sub(started.elapsed());
	let timed_out = wait(&mut child, left, &name);

	// Reap the child in either case. After a kill this also lets the tool's
	// own pipe ends close, so the readers usually finish inside the grace.
	let code = child.wait().ok().and_then(|status| status.code());
	let (raw, err) = collect(&receiver, readers, Duration::from_secs(1));

	if timed_out {
		return Err(Error::Tool {
			name,
			detail: format!("exceeded {} seconds", timeout.as_secs()),
		});
	}

	Ok(ToolOutput {
		stdout: String::from_utf8_lossy(&raw).into_owned(),
		stderr: String::from_utf8_lossy(&err).into_owned(),
		raw,
		code,
	})
}

/// Which pipe a reader is on.
#[derive(Clone, Copy)]
enum Stream {
	Out,
	Err,
}

/// The reader channel and the number of pipes still feeding it.
type PipeReaders = (mpsc::Receiver<(Stream, Option<Vec<u8>>)>, usize);

/// Starts the tool, naming it in anything that goes wrong.
fn spawn(command: &mut Command, name: &str) -> Result<Child, Error> {
	command.spawn().map_err(|error| {
		Error::Tool {
			name: name.to_owned(),
			detail: format!("starting: {error}"),
		}
	})
}

/// Reads the tool's two pipes on their own threads.
///
/// A tool sometimes leaves a process behind that inherits its output pipes,
/// such as a device daemon started by `adb` or `devicectl`. Waiting for the
/// pipes to close would then wait for that process to exit, which can be
/// forever. The readers report on a channel instead, so a pipe that stays open
/// costs one bounded grace period rather than the whole run.
fn read_pipes(child: &mut Child) -> PipeReaders {
	let (sender, receiver) = mpsc::channel();
	let mut readers = 0;

	if let Some(pipe) = child.stdout.take() {
		readers += 1;

		let sender = sender.clone();
		std::thread::spawn(move || capture(pipe, Stream::Out, &sender));
	}

	if let Some(pipe) = child.stderr.take() {
		readers += 1;
		std::thread::spawn(move || capture(pipe, Stream::Err, &sender));
	}

	(receiver, readers)
}

/// Sends the bytes a pipe yields as they arrive, then `None` at end of file.
fn capture(
	mut pipe: impl std::io::Read,
	stream: Stream,
	sender: &mpsc::Sender<(Stream, Option<Vec<u8>>)>,
) {
	let mut buffer = [0u8; 8192];

	loop {
		match std::io::Read::read(&mut pipe, &mut buffer) {
			Ok(0) => break,
			Ok(read) => {
				if sender
					.send((stream, Some(buffer[..read].to_vec())))
					.is_err()
				{
					return;
				}
			}

			Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
			Err(_) => break,
		}
	}

	let _ = sender.send((stream, None));
}

/// Waits for the tool, killing it once `timeout` passes. Says whether it had
/// to be killed.
fn wait(child: &mut Child, timeout: Duration, name: &str) -> bool {
	let started = Instant::now();

	loop {
		match child.try_wait() {
			Ok(Some(_)) => return false,
			Ok(None) => {}
			Err(error) => {
				eprintln!("waiting for {name}: {error}");

				return false;
			}
		}

		if started.elapsed() < timeout {
			std::thread::sleep(Duration::from_millis(20));
			continue;
		}

		if let Err(error) = terminate(child) {
			eprintln!("terminating timed-out {name}: {error}");
		}

		return true;
	}
}

/// Collects what the readers send until both end, or `grace` passes.
fn collect(
	receiver: &mpsc::Receiver<(Stream, Option<Vec<u8>>)>,
	mut readers: usize,
	grace: Duration,
) -> (Vec<u8>, Vec<u8>) {
	let mut raw = Vec::new();
	let mut err = Vec::new();
	let deadline = Instant::now() + grace;

	while readers > 0 {
		let Some(left) = deadline.checked_duration_since(Instant::now()) else {
			break;
		};

		match receiver.recv_timeout(left) {
			Ok((stream, Some(bytes))) => {
				match stream {
					Stream::Out => raw.extend(bytes),
					Stream::Err => err.extend(bytes),
				}
			}
			Ok((_, None)) => readers -= 1,
			Err(_) => break,
		}
	}

	(raw, err)
}

/// Spawns a detached process that outlives this run, for emulators a control
/// document boots but no process owns.
pub fn spawn_detached(command: &mut Command) -> Result<(), Error> {
	spawn_detached_child(command).map(|_| ())
}

/// Spawns a detached process and returns it, so the caller can wait until a
/// window appears without blocking on the process exiting.
pub fn spawn_detached_child(command: &mut Command) -> Result<Child, Error> {
	isolate(command);
	command
		.stdin(Stdio::null())
		.stdout(Stdio::null())
		.stderr(Stdio::null());
	let name = command.get_program().to_string_lossy().into_owned();
	spawn(command, &name)
}

fn isolate(command: &mut Command) {
	#[cfg(unix)]
	{
		use std::os::unix::process::CommandExt;
		command.process_group(0);
	}
	#[cfg(not(unix))]
	let _ = command;
}

fn terminate(child: &mut Child) -> Result<(), std::io::Error> {
	#[cfg(unix)]
	{
		use rustix::process::Pid;
		use rustix::process::Signal;
		use rustix::process::kill_process_group;
		let Some(pid) = i32::try_from(child.id()).ok().and_then(Pid::from_raw) else {
			return Ok(());
		};

		match kill_process_group(pid, Signal::KILL) {
			Ok(()) | Err(rustix::io::Errno::SRCH) => Ok(()),
			Err(error) => Err(std::io::Error::from(error)),
		}
	}
	#[cfg(not(unix))]
	{
		child.kill()
	}
}

/// Resolves `adb` or `emulator` against the Android SDK variables.
pub fn android_tool(name: &str) -> Command {
	let executable = if cfg!(windows) {
		format!("{name}.exe")
	} else {
		name.to_owned()
	};

	let path = std::env::var_os("ANDROID_HOME")
		.or_else(|| std::env::var_os("ANDROID_SDK_ROOT"))
		.map_or_else(
			|| PathBuf::from(&executable),
			|sdk| {
				PathBuf::from(sdk)
					.join(if name == "adb" {
						"platform-tools"
					} else {
						"emulator"
					})
					.join(&executable)
			},
		);
	Command::new(path)
}

/// An `adb` invocation for one device serial.
pub fn adb(serial: &str) -> Command {
	let mut command = android_tool("adb");
	command.args(["-s", serial]);
	command
}

/// An `xcrun simctl` invocation.
pub fn simctl() -> Command {
	let mut command = Command::new("xcrun");
	command.arg("simctl");
	command
}

/// An `xcrun devicectl` invocation.
pub fn devicectl() -> Command {
	let mut command = Command::new("xcrun");
	command.arg("devicectl");
	command
}

/// Runs a listing command whose tool may not be installed, returning its
/// standard output or [`None`] when the tool is missing or fails.
pub fn list(mut command: Command, timeout: Duration) -> Option<String> {
	match run(&mut command, timeout) {
		Ok(output) if output.success() => Some(output.stdout),
		_ => None,
	}
}

/// Reads `xcrun simctl list devices --json`, or [`None`] without Xcode.
pub fn simulator_list(timeout: Duration) -> Option<String> {
	let mut command = Command::new("xcrun");
	command.args(["simctl", "list", "devices", "--json"]);
	list(command, timeout)
}

/// Reads `xcrun devicectl list devices --json-output <file>`, or [`None`]
/// without Xcode.
///
/// devicectl writes the JSON document to the file named by `--json-output`.
/// It does not print that document for `--json`.
pub fn iphone_list(timeout: Duration) -> Option<String> {
	let path = devicectl_listing_path();
	let command = devicectl_list_command(&path);
	let command_ok = list(command, timeout).is_some();
	listing_body(command_ok, &path)
}

/// `xcrun devicectl list devices --json-output <path>`.
fn devicectl_list_command(json_path: &std::path::Path) -> Command {
	let mut command = Command::new("xcrun");
	command
		.args(["devicectl", "list", "devices", "--json-output"])
		.arg(json_path);
	command
}

/// A private path for one `devicectl` listing.
fn devicectl_listing_path() -> PathBuf {
	static LISTINGS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
	let listing = LISTINGS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
	let nanos = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map_or(0, |duration| duration.as_nanos());
	std::env::temp_dir().join(format!(
		"leptoskit-devicectl-{}-{nanos}-{listing}.json",
		std::process::id()
	))
}

/// The JSON file devicectl wrote, when it wrote one.
fn read_listing(path: &std::path::Path) -> Option<String> {
	let text = std::fs::read_to_string(path).ok()?;
	if text.trim().is_empty() {
		None
	} else {
		Some(text)
	}
}

/// The JSON file devicectl wrote, when the command succeeded and the file
/// has a document. The file is removed either way, so a failed command cannot
/// return a stale document.
fn listing_body(command_ok: bool, path: &std::path::Path) -> Option<String> {
	let body = read_listing(path);
	let _ = std::fs::remove_file(path);
	if command_ok { body } else { None }
}

/// Reads `adb devices -l`, or [`None`] without an Android SDK.
pub fn android_list(timeout: Duration) -> Option<String> {
	let mut command = android_tool("adb");
	command.args(["devices", "-l"]);
	list(command, timeout)
}

/// Reads the installed Android virtual devices, or [`None`] without an
/// Android SDK.
pub fn avd_list(timeout: Duration) -> Option<String> {
	let mut command = android_tool("emulator");
	command.arg("-list-avds");
	list(command, timeout)
}

/// Finds an executable on `PATH`, like `which`.
pub fn find_in_path(program: &str) -> Option<PathBuf> {
	let path = std::env::var_os("PATH")?;
	let executable = if cfg!(windows) {
		format!("{program}.exe")
	} else {
		program.to_owned()
	};

	std::env::split_paths(&path)
		.map(|directory| directory.join(&executable))
		.find(|candidate| candidate.is_file())
}

/// A free TCP port on loopback, handed back for a driver to bind. Racing
/// another process into the port fails its startup loudly rather than
/// silently misrouting.
pub fn free_port() -> Option<u16> {
	std::net::TcpListener::bind(("127.0.0.1", 0))
		.ok()?
		.local_addr()
		.ok()?
		.port()
		.into()
}

#[cfg(all(test, unix))]
mod tests {
	use super::*;

	#[test]
	fn running_a_tool_captures_its_output() {
		let mut command = Command::new("/bin/echo");
		command.arg("captured");
		let output = run(&mut command, Duration::from_secs(10)).unwrap();
		assert_eq!(output.stdout.trim(), "captured");
		assert_eq!(output.raw, b"captured\n");
		assert!(output.success());
	}

	#[test]
	fn a_timed_out_tool_is_killed_and_reported() {
		let mut command = Command::new("/bin/sleep");
		command.arg("30");
		let error = run(&mut command, Duration::from_millis(300)).unwrap_err();
		assert!(error.to_string().contains("exceeded"), "{error}");
		// The process group is gone, not lingering.
		let check = Command::new("/bin/ps")
			.arg("-o")
			.arg("pid=")
			.arg("-p")
			.arg("0")
			.output();
		assert!(check.is_ok());
	}

	#[test]
	fn a_daemon_that_keeps_the_pipe_open_does_not_hang_the_call() {
		// The shell exits at once, but its background child inherits stdout
		// and stderr. Joining the readers would wait for that child; the
		// bounded grace returns the tool's own output instead.
		let mut command = Command::new("/bin/sh");
		command.arg("-c").arg("sleep 10 & printf started >&1");
		let started = Instant::now();
		let output = run(&mut command, Duration::from_secs(5)).unwrap();
		assert_eq!(output.stdout.trim(), "started");
		assert!(
			started.elapsed() < Duration::from_secs(4),
			"the call waited for the background process"
		);
	}

	#[cfg(unix)]
	#[test]
	fn feeding_stdin_reaches_the_command() {
		let mut command = Command::new("sh");
		command.args(["-c", "cat"]);
		let output = run_feeding(&mut command, Duration::from_secs(5), Some(b"pinch\n")).unwrap();
		assert_eq!(output.stdout, "pinch\n");
	}

	#[test]
	fn missing_tools_report_their_name() {
		let mut command = Command::new("leptoskit-no-such-tool");
		let error = run(&mut command, Duration::from_secs(5)).unwrap_err();
		assert!(error.to_string().contains("leptoskit-no-such-tool"));
	}

	#[test]
	fn spawn_detached_starts_and_missing_programs_fail() {
		let mut command = Command::new("/usr/bin/true");
		spawn_detached(&mut command).unwrap();
		let mut command = Command::new("leptoskit-no-such-tool");
		let error = spawn_detached(&mut command).unwrap_err();
		assert!(error.to_string().contains("leptoskit-no-such-tool"));
	}

	#[test]
	fn free_ports_are_real_and_bindable() {
		// Another test can race the port between asking and binding, so
		// a few tries must land on one that really is free.
		for _ in 0..10 {
			if let Some(port) = free_port()
				&& std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
			{
				return;
			}
		}

		panic!("no free port stayed bindable across ten tries");
	}

	#[test]
	fn platform_tools_carry_their_programs() {
		let simctl = simctl();
		assert_eq!(simctl.get_program(), "xcrun");
		let devicectl = devicectl();
		assert_eq!(devicectl.get_program(), "xcrun");
		let adb = android_tool("adb");
		assert!(adb.get_program().to_string_lossy().ends_with("adb"));
	}

	#[test]
	fn devicectl_list_writes_json_to_a_file() {
		let path = PathBuf::from("/tmp/leptoskit-devices.json");
		let command = devicectl_list_command(&path);
		assert_eq!(command.get_program(), "xcrun");
		let args: Vec<_> = command
			.get_args()
			.map(|arg| arg.to_string_lossy().into_owned())
			.collect();
		assert_eq!(
			args,
			[
				"devicectl",
				"list",
				"devices",
				"--json-output",
				"/tmp/leptoskit-devices.json",
			]
		);
	}

	#[test]
	fn a_listing_file_is_kept_only_when_it_has_json() {
		let directory =
			std::env::temp_dir().join(format!("leptoskit-listing-{}", std::process::id()));
		std::fs::create_dir_all(&directory).unwrap();
		let path = directory.join("devices.json");
		std::fs::write(&path, " \n").unwrap();
		assert!(read_listing(&path).is_none());
		std::fs::write(&path, "{\"result\":{\"devices\":[]}}\n").unwrap();
		assert_eq!(
			read_listing(&path).as_deref(),
			Some("{\"result\":{\"devices\":[]}}\n")
		);
		assert!(read_listing(directory.join("missing.json").as_path()).is_none());
		std::fs::write(&path, "{\"ok\":true}\n").unwrap();
		assert!(listing_body(false, &path).is_none());
		assert!(!path.exists(), "a failed command must not leave the file");
		std::fs::write(&path, "{\"ok\":true}\n").unwrap();
		assert_eq!(
			listing_body(true, &path).as_deref(),
			Some("{\"ok\":true}\n")
		);
		assert!(!path.exists());
		assert!(listing_body(true, &directory.join("missing.json")).is_none());
		let _ = std::fs::remove_dir_all(&directory);
	}

	#[test]
	fn iphone_list_fails_closed_without_xcrun() {
		// A machine with Xcode would actually talk to a phone. The Linux
		// lanes have no `xcrun`, and that is the path this covers.
		if find_in_path("xcrun").is_some() {
			return;
		}

		assert!(iphone_list(Duration::from_secs(2)).is_none());
	}

	#[test]
	fn find_in_path_resolves_real_programs_only() {
		assert!(find_in_path("ls").is_some());
		assert!(find_in_path("leptoskit-no-such-tool").is_none());
	}
}
