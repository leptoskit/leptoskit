//! A harness for the process tests: the real binary on ephemeral ports
//! and its own temporary volume, driven over raw HTTP so every `Host`
//! header is explicit.

#![allow(dead_code, reason = "each proof uses the pieces it needs")]

use std::fmt::Write as _;
use std::fs::File;
use std::io::Read as _;
use std::io::Write as _;
use std::net::IpAddr;
use std::net::Ipv4Addr;
use std::net::Ipv6Addr;
use std::net::SocketAddr;
use std::net::TcpListener;
use std::net::TcpStream;
use std::os::fd::AsFd as _;
use std::path::Path;
use std::path::PathBuf;
use std::process::Child;
use std::process::Command;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use rustix::fs::FlockOperation;

/// The site's host: loopback HTTP, accepted by every layer for
/// development, and distinct from the admin's.
pub const SITE_HOST: &str = "localhost";
/// The admin's host.
pub const ADMIN_HOST: &str = "127.0.0.1";
/// The admin's host when it is a canonical bracketed IPv6 literal.
pub const ADMIN_HOST_V6: &str = "[::1]";
/// The address the tests reach an IPv4-bound listener through.
pub const CONNECT_V4: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
/// The address the tests reach an IPv6-bound listener through.
pub const CONNECT_V6: IpAddr = IpAddr::V6(Ipv6Addr::LOCALHOST);

/// How long a spawn may take to become ready in a debug build.
pub const READY: Duration = Duration::from_secs(60);
/// How long a bounded retry may take (the hook shortens it).
pub const RETRY: Duration = Duration::from_secs(30);
/// How long a SIGTERM has to finish, per the brief.
pub const SIGTERM_BUDGET: Duration = Duration::from_secs(8);
/// The drain bound for in-flight requests and streams.
pub const DRAIN: Duration = Duration::from_secs(6);

/// One running service process, its port, and its bind.
pub struct Server {
	pub child: Child,
	pub port: u16,
	/// The address family the process binds, so the polling helpers
	/// connect where it actually listens.
	pub addr: IpAddr,
	/// The process's stderr, collected on a thread and readable while
	/// the process runs, so a proof can wait for a log line instead of
	/// sleeping for a guess at when it appears.
	stderr: Option<Arc<Mutex<String>>>,
	/// The collecting thread, joined by [`Server::stderr_of`] once the
	/// process has ended.
	stderr_thread: Option<thread::JoinHandle<()>>,
}

impl Server {
	/// Whether the process is still running.
	pub fn alive(&mut self) -> bool {
		matches!(self.child.try_wait(), Ok(None))
	}

	/// The process's exit status, once it has one.
	pub fn try_wait(&mut self) -> Option<std::process::ExitStatus> {
		self.child
			.try_wait()
			.expect("the child's status is readable")
	}

	/// Waits for the process to exit within `timeout`.
	pub fn wait_within(&mut self, timeout: Duration) -> Option<std::process::ExitStatus> {
		let deadline = Instant::now() + timeout;
		while Instant::now() < deadline {
			if let Some(status) = self.try_wait() {
				return Some(status);
			}
			thread::sleep(Duration::from_millis(50));
		}
		None
	}

	/// The process id, while it runs.
	pub fn pid(&self) -> u32 {
		self.child.id()
	}

	/// Sends SIGTERM and waits, bounded like a supervisor would be.
	pub fn terminate_within(&mut self, timeout: Duration) -> Option<std::process::ExitStatus> {
		// SIGTERM through the harness: `kill` on the child's pid.
		terminate(self.pid());
		self.wait_within(timeout)
	}

	/// Sends SIGTERM without waiting for the exit, so a proof can deliver
	/// it while a drain is in progress and repeat it safely. A no-op where
	/// there is no SIGTERM, like [`Server::terminate_within`].
	pub fn signal_stop(&mut self) {
		terminate(self.pid());
	}

	/// Kills without ceremony, for crash proofs.
	pub fn kill(&mut self) {
		let _ = self.child.kill();
		let _ = self.child.wait();
	}
}

impl Drop for Server {
	fn drop(&mut self) {
		let _ = self.child.kill();
		let _ = self.child.wait();
	}
}

/// SIGTERM, via `kill`, so the process's own handler runs.
#[cfg(unix)]
fn terminate(pid: u32) {
	// No external crate: the shell-free form is `kill(2)` through libc,
	// which the tests reach through the standard `Command` for the same
	// effect without a dependency.
	let _ = Command::new("kill")
		.args(["-TERM", &pid.to_string()])
		.stdout(Stdio::null())
		.stderr(Stdio::null())
		.status();
}

#[cfg(not(unix))]
fn terminate(_pid: u32) {}

/// Distinct, verified-free ports for parallel tests: the counter hands
/// each caller a different port even mid-parallel, and each is checked
/// for a live listener before it is handed out.
///
/// Two facts about the range keep one machine's many copies of this
/// suite apart. First, other checkouts still hand ports out
/// sequentially from 31000 — every copy of the deterministic allocator
/// lives in the first few dozen ports below 31100 — so this sweep
/// starts no lower than that. Second, where this run's sweep starts is
/// random per test binary, so two randomized copies land in different
/// parts of the range instead of racing the same port: both would
/// check a port free, drop the probe, and lose the bind to the other's
/// child, which the service truthfully refuses with "Address already
/// in use". The probe-and-skip loop below still avoids whatever is
/// genuinely held.
pub fn free_port() -> u16 {
	use std::sync::OnceLock;
	use std::sync::atomic::AtomicU16;
	use std::sync::atomic::Ordering;

	static OFFSET: OnceLock<u16> = OnceLock::new();
	let offset = *OFFSET.get_or_init(|| {
		let nanos = std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.expect("the clock is sane")
			.subsec_nanos();
		(std::process::id() ^ nanos) as u16
	});

	static NEXT: AtomicU16 = AtomicU16::new(0);

	for _ in 0..64 {
		let port = 31100 + offset.wrapping_add(NEXT.fetch_add(1, Ordering::SeqCst)) % 7900;
		let Ok(listener) = TcpListener::bind(("127.0.0.1", port)) else {
			continue;
		};
		drop(listener);
		return port;
	}
	panic!("no free port in the test range")
}

/// A port the kernel agrees is free on both loopback families. The IPv6
/// proofs bind `[::1]`, a namespace the IPv4-only probe inside
/// [`free_port`] cannot see: a listener there would both trip a
/// "nothing listens" probe that had checked nothing and steal a child's
/// bind. Every candidate still comes from `free_port`, so the range
/// rules there apply here too.
pub fn free_port_on_both_families() -> u16 {
	for _ in 0..16 {
		let port = free_port();
		if TcpListener::bind((CONNECT_V6, port)).is_ok() {
			return port;
		}
	}
	panic!("no port is free on both loopback families")
}

/// A fresh volume: mode 0700, owned by this account, not a symlink.
pub fn volume() -> PathBuf {
	let root = tempfile::tempdir().expect("a temporary directory").keep();
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt as _;
		std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
			.expect("the volume is restrictable");
	}
	root
}

/// Holds the volume's `build.lock` exclusively, as the service's own
/// builder would. While a proof holds it, a spawned process binds its
/// listener and serves the fallback but cannot start building, which
/// makes "before any generation" a fact rather than a race against the
/// build finishing first; dropping the file lets the process proceed.
#[cfg(unix)]
pub fn hold_build_lock(root: &Path) -> File {
	let file = std::fs::OpenOptions::new()
		.read(true)
		.write(true)
		.create(true)
		.truncate(false)
		.open(root.join("build.lock"))
		.expect("the build lock opens");
	rustix::fs::flock(file.as_fd(), FlockOperation::LockExclusive)
		.expect("the exclusive build lock is taken");
	file
}

/// A site root holding a stand-in for the pages' browser half: the two
/// files the service insists on before it starts, with nothing in them a
/// browser could run. The process proofs read status lines and markup and
/// never open a page, so the real client, which `leptoskit build` makes, is
/// not needed here. One directory serves every proof in the test binary.
pub fn client_root() -> &'static Path {
	static ROOT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

	ROOT.get_or_init(|| {
		let root = tempfile::tempdir().expect("a temporary directory").keep();
		let pkg = root.join("pkg");
		std::fs::create_dir_all(&pkg).expect("the client directory");
		std::fs::write(pkg.join("demo_pages.js"), b"export {};\n").expect("the stand-in script");
		std::fs::write(pkg.join("demo_pages_bg.wasm"), b"\0asm\x01\0\0\0")
			.expect("the stand-in module");
		root
	})
}

/// One server's environment: the given bind, and the loopback HTTP
/// origins the development proofs use.
pub fn env(root: &Path, bind: SocketAddr, extra: &[(&str, String)]) -> Vec<(String, String)> {
	let mut variables = vec![
		("DEMO_BIND".to_owned(), bind.to_string()),
		(
			"LEPTOSKIT_SERVICE_VOLUME_STATE".to_owned(),
			root.display().to_string(),
		),
		("DEMO_SITE_ORIGIN".to_owned(), format!("http://{SITE_HOST}")),
		(
			"DEMO_ADMIN_ORIGIN".to_owned(),
			format!("http://{ADMIN_HOST}"),
		),
		(
			"LEPTOS_SITE_ROOT".to_owned(),
			client_root().display().to_string(),
		),
	];
	for (name, value) in extra {
		variables.push(((*name).to_owned(), value.clone()));
	}
	variables
}

/// Spawns the real binary bound to IPv4 loopback. Every hook is a
/// debug-build env var the test passes in explicitly; the release binary
/// ignores them.
pub fn spawn(root: &Path, port: u16, extra: &[(&str, String)]) -> Server {
	spawn_on(SocketAddr::new(CONNECT_V4, port), root, extra)
}

/// Starts a service on a fresh port, retrying a refused bind.
pub fn spawn_fresh(root: &Path, extra: &[(&str, String)]) -> Server {
	spawn_fresh_with(free_port, |port| spawn(root, port, extra))
}

/// Starts an IPv6 service on a fresh port, retrying a refused bind.
pub fn spawn_fresh_ipv6(root: &Path, extra: &[(&str, String)]) -> Server {
	spawn_fresh_with(free_port_on_both_families, |port| {
		spawn_ipv6(root, port, extra)
	})
}

fn spawn_fresh_with(
	mut choose: impl FnMut() -> u16,
	mut start: impl FnMut(u16) -> Server,
) -> Server {
	let mut refused = Vec::new();
	for _ in 0..5 {
		let port = choose();
		let mut server = start(port);
		let deadline = Instant::now() + READY;
		loop {
			if let Some(exit) = server.try_wait() {
				let stderr = stderr_of(&mut server);
				let address = SocketAddr::new(server.addr, port);
				if stderr.contains(&format!("binding {address}: Address already in use")) {
					eprintln!("port {port} was refused; trying another port");
					refused.push(port);
					break;
				}
				panic!("the service exited with {exit} before binding its listener\n{stderr}");
			}
			let (status, _) = get_on(server.addr, port, SITE_HOST, "/healthz");
			if status != 0 && server.alive() {
				return server;
			}
			if Instant::now() > deadline {
				let stderr = stderr_of(&mut server);
				panic!("the service never bound its listener\n{stderr}");
			}
			thread::sleep(Duration::from_millis(50));
		}
	}
	panic!("the service could not bind after five attempts; refused ports: {refused:?}");
}

/// Spawns the real binary bound to IPv6 loopback, with the admin origin
/// moved onto the same bracketed literal so the IPv6 proofs exercise an
/// IPv6 authority, not just an IPv6 bind.
pub fn spawn_ipv6(root: &Path, port: u16, extra: &[(&str, String)]) -> Server {
	let mut variables = env(root, SocketAddr::new(CONNECT_V6, port), &[]);
	variables.push((
		"DEMO_ADMIN_ORIGIN".to_owned(),
		format!("http://{ADMIN_HOST_V6}"),
	));
	for (name, value) in extra {
		variables.push(((*name).to_owned(), value.clone()));
	}
	spawn_with_variables(variables)
}

/// Spawns the real binary on the given bind. Every hook is a
/// debug-build env var the test passes in explicitly; the release binary
/// ignores them.
pub fn spawn_on(bind: SocketAddr, root: &Path, extra: &[(&str, String)]) -> Server {
	spawn_with_variables(env(root, bind, extra))
}

/// Spawns the real binary with named environment values replaced on top
/// of the usual ones — for configuration-refusal proofs, where the
/// malformed value is the point.
pub fn spawn_env(root: &Path, port: u16, overrides: &[(&str, String)]) -> Server {
	let mut variables = env(root, SocketAddr::new(CONNECT_V4, port), &[]);
	for (name, value) in overrides {
		variables.retain(|(known, _)| known != name);
		variables.push(((*name).to_owned(), value.clone()));
	}
	spawn_with_variables(variables)
}

fn spawn_with_variables(variables: Vec<(String, String)>) -> Server {
	let bind: SocketAddr = variables
		.iter()
		.find(|(name, _)| name == "DEMO_BIND")
		.and_then(|(_, value)| value.parse().ok())
		.expect("the bind parses");
	let mut child = Command::new(env!("CARGO_BIN_EXE_demo_service"))
		.envs(variables)
		.stdout(Stdio::null())
		.stderr(Stdio::piped())
		.spawn()
		.expect("the demo service binary spawns");

	// The stderr tee: a thread reads the pipe into shared text a proof
	// can poll, so "the refusal is on stderr" can be waited for rather
	// than slept past.
	let (stderr, stderr_thread) = child.stderr.take().map_or((None, None), |mut stderr| {
		let collected = Arc::new(Mutex::new(String::new()));
		let sink = Arc::clone(&collected);
		let handle = thread::spawn(move || {
			let mut chunk = [0_u8; 4096];
			loop {
				match stderr.read(&mut chunk) {
					Ok(0) | Err(_) => break,
					Ok(read) => {
						sink.lock()
							.unwrap_or_else(std::sync::PoisonError::into_inner)
							.push_str(&String::from_utf8_lossy(&chunk[..read]));
					}
				}
			}
		});
		(Some(collected), Some(handle))
	});

	Server {
		child,
		port: bind.port(),
		addr: bind.ip(),
		stderr,
		stderr_thread,
	}
}

/// Polls until `/readyz` says ready. Takes the server mutably so a
/// failure can report the process's own stderr.
///
/// A child that has exited never becomes ready: the wait fails at once
/// with the exit status and stderr instead of polling a dead process
/// for the whole budget — which is what turned a refused bind into a
/// minute of silence and a timeout whose cause it never named.
pub fn wait_ready(server: &mut Server, timeout: Duration) -> u16 {
	let deadline = Instant::now() + timeout;
	loop {
		let (status, body) = get_on(server.addr, server.port, SITE_HOST, "/readyz");
		if std::env::var_os("DEMO_TEST_TRACE").is_some() {
			eprintln!(
				"[trace] /readyz on {addr}:{port} as {SITE_HOST} -> {status} {body}",
				addr = server.addr,
				port = server.port
			);
		}
		if status == 200 {
			return status;
		}
		if let Some(exit) = server.try_wait() {
			let stderr = stderr_of(server);
			panic!(
				"the service exited with {exit} before becoming ready (last /readyz was \
				 {status})\n{stderr}"
			);
		}
		if Instant::now() > deadline {
			let stderr = stderr_of(server);
			panic!("the service never became ready (last /readyz was {status})\n{stderr}");
		}
		thread::sleep(Duration::from_millis(100));
	}
}

/// Polls until the process answers at all (it has bound its listener).
/// As [`wait_ready`], a child that has exited fails the wait at once.
pub fn wait_bound(server: &mut Server, timeout: Duration) {
	let deadline = Instant::now() + timeout;
	loop {
		let (status, _) = get_on(server.addr, server.port, SITE_HOST, "/healthz");
		if status != 0 {
			return;
		}
		if let Some(exit) = server.try_wait() {
			let stderr = stderr_of(server);
			panic!("the service exited with {exit} before binding its listener\n{stderr}");
		}
		if Instant::now() > deadline {
			let stderr = stderr_of(server);
			panic!("the service never bound its listener\n{stderr}");
		}
		thread::sleep(Duration::from_millis(50));
	}
}

/// Polls until `/readyz` stops being ready.
pub fn wait_unready(server: &mut Server, timeout: Duration) -> u16 {
	let deadline = Instant::now() + timeout;
	loop {
		let (status, body) = get_on(server.addr, server.port, SITE_HOST, "/readyz");
		if status != 200 {
			return status;
		}
		assert!(
			Instant::now() <= deadline,
			"the service never became unready: {body}"
		);
		thread::sleep(Duration::from_millis(100));
	}
}

/// The generation directory, once exactly one is expected.
pub fn generation_dir(root: &Path) -> PathBuf {
	let mut generations: Vec<PathBuf> = std::fs::read_dir(root.join("gen"))
		.expect("gen/ exists")
		.filter_map(Result::ok)
		.map(|entry| entry.path())
		.filter(|path| path.is_dir())
		.collect();
	generations.sort();
	assert_eq!(
		generations.len(),
		1,
		"exactly one generation directory is expected: {generations:?}"
	);
	generations.remove(0)
}

/// Every generation directory name, for retention proofs.
pub fn generation_names(root: &Path) -> Vec<String> {
	let Ok(entries) = std::fs::read_dir(root.join("gen")) else {
		return Vec::new();
	};
	let mut names: Vec<String> = entries
		.filter_map(Result::ok)
		.map(|entry| entry.file_name().to_string_lossy().into_owned())
		.collect();
	names.sort();
	names
}

/// The secrets file's contents, as the tests use them (they own the
/// machine and the volume; the file stays 0600).
pub fn secrets(root: &Path) -> serde_json::Value {
	let dir = generation_dir(root);
	let bytes = std::fs::read(dir.join("secrets.json")).expect("secrets.json exists");
	serde_json::from_slice(&bytes).expect("secrets.json parses")
}

/// The worker owner's pid, if written.
pub fn worker_owner(root: &Path) -> Option<String> {
	let dir = generation_dir(root);
	std::fs::read_to_string(dir.join("worker.owner"))
		.ok()
		.map(|text| text.trim().to_owned())
}

/// Writes the debug clock file with a given instant.
pub fn set_clock(path: &Path, millis: i64) {
	std::fs::write(path, millis.to_string()).expect("the clock file is writable");
}

/// The debug clock hooks: a file-driven clock, ticked every 50 ms so a
/// reset is deterministic instead of waiting for real midnight.
pub fn clock_hooks(clock: &Path) -> Vec<(&'static str, String)> {
	vec![
		("DEMO_TEST_CLOCK", clock.display().to_string()),
		("DEMO_TEST_CLOCK_TICK_MS", "50".to_owned()),
	]
}

/// Shortens the bounded retry, so a failed initialization's fresh image
/// arrives within a test's patience.
pub fn retry_hook() -> Vec<(&'static str, String)> {
	vec![("DEMO_TEST_RETRY_MS", "1500".to_owned())]
}

/// A plain HTTP request over raw TCP, with an explicit `Host`, answered
/// until the connection closes.
pub fn get(port: u16, host: &str, path: &str) -> (u16, String) {
	get_on(CONNECT_V4, port, host, path)
}

/// The same request, to whatever address the listener bound.
pub fn get_on(ip: IpAddr, port: u16, host: &str, path: &str) -> (u16, String) {
	let (status, _, body) = request_on(ip, port, host, path, Duration::from_secs(5));
	(status, body)
}

pub fn request(port: u16, host: &str, path: &str, timeout: Duration) -> (u16, String) {
	let (status, _, body) = request_on(CONNECT_V4, port, host, path, timeout);
	(status, body)
}

/// A GET with its response headers, so tests can check `Retry-After`.
pub fn request_with_headers(
	port: u16,
	host: &str,
	path: &str,
) -> (u16, Vec<(String, String)>, String) {
	request_on(CONNECT_V4, port, host, path, Duration::from_secs(5))
}

fn request_on(
	ip: IpAddr,
	port: u16,
	host: &str,
	path: &str,
	timeout: Duration,
) -> (u16, Vec<(String, String)>, String) {
	// A server that hasn't bound yet, or is mid-exec, refuses: that is
	// `not ready`, which the polling helpers understand.
	let Ok(mut stream) = TcpStream::connect((ip, port)) else {
		return (0, Vec::new(), String::new());
	};
	stream
		.set_read_timeout(Some(timeout))
		.expect("the read timeout is set");
	stream
		.set_write_timeout(Some(timeout))
		.expect("the write timeout is set");

	let mut wire = String::new();
	let _ = write!(
		wire,
		"GET {path} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: demo-service-test\r\nAccept: \
		 */*\r\nConnection: close\r\n\r\n"
	);
	stream
		.write_all(wire.as_bytes())
		.expect("the request is sent");

	let mut response = Vec::new();
	let _ = stream.read_to_end(&mut response);
	parse(&response)
}

/// A raw request, sent verbatim — for wire-syntax proofs the typed
/// helpers cannot express, such as a duplicated `Host` header. Returns
/// `(status, body)`.
pub fn request_raw(ip: IpAddr, port: u16, raw: &str) -> (u16, String) {
	let Ok(mut stream) = TcpStream::connect((ip, port)) else {
		return (0, String::new());
	};
	stream
		.set_read_timeout(Some(Duration::from_secs(5)))
		.expect("the read timeout is set");
	stream
		.set_write_timeout(Some(Duration::from_secs(5)))
		.expect("the write timeout is set");
	stream
		.write_all(raw.as_bytes())
		.expect("the request is sent");

	let mut response = Vec::new();
	let _ = stream.read_to_end(&mut response);
	let (status, _, body) = parse(&response);
	(status, body)
}

/// A POST with a binary body, answered until the connection closes:
/// `(status, body)`. The callers name every header, so the request
/// carries only what the proof needs.
pub fn post(
	port: u16,
	host: &str,
	path: &str,
	headers: &[(&str, &str)],
	body: &[u8],
) -> (u16, String) {
	let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) else {
		return (0, String::new());
	};
	stream
		.set_read_timeout(Some(Duration::from_secs(5)))
		.expect("the read timeout is set");
	stream
		.set_write_timeout(Some(Duration::from_secs(5)))
		.expect("the write timeout is set");

	let mut wire = format!(
		"POST {path} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: demo-service-test\r\nAccept: \
		 */*\r\nConnection: close\r\n"
	);
	for (name, value) in headers {
		wire.push_str(name);
		wire.push_str(": ");
		wire.push_str(value);
		wire.push_str("\r\n");
	}
	wire.push_str("Content-Length: ");
	let _ = write!(wire, "{}\r\n\r\n", body.len());
	stream
		.write_all(wire.as_bytes())
		.expect("the request is sent");
	stream.write_all(body).expect("the body is sent");

	let mut response = Vec::new();
	let _ = stream.read_to_end(&mut response);
	let (status, _, text) = parse(&response);
	(status, text)
}

/// Parses a full HTTP/1.1 response's status, headers, and body.
fn parse(response: &[u8]) -> (u16, Vec<(String, String)>, String) {
	let text = String::from_utf8_lossy(response);
	let Some(split) = text.find("\r\n\r\n") else {
		return (0, Vec::new(), text.into_owned());
	};
	let head = &text[..split];
	let body = text[split + 4..].to_owned();

	let mut lines = head.lines();
	let status = lines
		.next()
		.and_then(|line| line.split_whitespace().nth(1))
		.and_then(|code| code.parse().ok())
		.unwrap_or(0);
	let headers = lines
		.filter_map(|line| {
			let (name, value) = line.split_once(':')?;
			Some((name.trim().to_owned(), value.trim().to_owned()))
		})
		.collect();

	(status, headers, body)
}

/// An open SSE connection, read incrementally.
pub struct Sse {
	stream: TcpStream,
	buffer: Vec<u8>,
}

impl Sse {
	/// Opens the stream and reads the first event, proving the connection
	/// is live: `(stream, (event, data))`.
	pub fn open(port: u16, host: &str, path: &str) -> (Self, (String, String)) {
		let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("the SSE connection opens");
		stream
			.set_read_timeout(Some(Duration::from_secs(10)))
			.expect("the read timeout is set");
		let mut wire = String::new();
		let _ = write!(
			wire,
			"GET {path} HTTP/1.1\r\nHost: {host}\r\nAccept: text/event-stream\r\n\r\n"
		);
		stream
			.write_all(wire.as_bytes())
			.expect("the SSE request is sent");

		let mut sse = Self {
			stream,
			buffer: Vec::new(),
		};
		let first = sse
			.read_event(Duration::from_secs(5))
			.expect("a first event arrives");
		(sse, first)
	}

	/// Reads one SSE event: `(event, data)`. `None` on close or timeout.
	pub fn read_event(&mut self, timeout: Duration) -> Option<(String, String)> {
		let deadline = Instant::now() + timeout;
		loop {
			if let Some(event) = self.event_in_buffer() {
				return Some(event);
			}
			let remaining = deadline.checked_duration_since(Instant::now())?;
			self.stream.set_read_timeout(Some(remaining)).ok();
			let mut chunk = [0_u8; 4096];
			match self.stream.read(&mut chunk) {
				Ok(0) | Err(_) => return None,
				Ok(read) => self.buffer.extend_from_slice(&chunk[..read]),
			}
		}
	}

	/// Whether the stream closes within `timeout`.
	pub fn closes_within(&mut self, timeout: Duration) -> bool {
		let deadline = Instant::now() + timeout;
		loop {
			if self.event_in_buffer().is_some() {
				// Another event arrived instead of a close.
				return false;
			}
			let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
				return false;
			};
			self.stream.set_read_timeout(Some(remaining)).ok();
			let mut chunk = [0_u8; 4096];
			match self.stream.read(&mut chunk) {
				Ok(0) => return true,
				Ok(_) => {}
				Err(error)
					if error.kind() == std::io::ErrorKind::WouldBlock
						|| error.kind() == std::io::ErrorKind::TimedOut =>
				{
					return false;
				}
				Err(_) => return true,
			}
		}
	}

	/// The next complete `event`/`data` pair in the buffer, if any. SSE
	/// frames end at an empty line, which arrives as `\r\n\r\n` or
	/// `\n\n`; keep-alive comments are skipped.
	fn event_in_buffer(&mut self) -> Option<(String, String)> {
		let end = frame_end(&self.buffer)?;
		let frame = String::from_utf8_lossy(&self.buffer[..end.0]).into_owned();
		self.buffer.drain(..end.1);

		let mut event = None;
		let mut data = None;
		for line in frame.lines() {
			let line = line.trim_end_matches('\r');
			if let Some(value) = line.strip_prefix("event: ") {
				event = Some(value.trim().to_owned());
			}
			if let Some(value) = line.strip_prefix("data: ") {
				data = Some(value.trim().to_owned());
			}
		}
		Some((event?, data?))
	}
}

/// Where the first SSE frame in `buffer` ends: `(frame end, buffer
/// consumed)`, or `None` while no whole frame has arrived.
fn frame_end(buffer: &[u8]) -> Option<(usize, usize)> {
	for (index, window) in buffer.windows(4).enumerate() {
		if window == b"\r\n\r\n" {
			return Some((index, index + 4));
		}
	}
	for (index, window) in buffer.windows(2).enumerate() {
		if window == b"\n\n" && !buffer[..index].ends_with(b"\r") {
			return Some((index, index + 2));
		}
	}
	None
}

/// Whether a path exists after a bounded wait.
pub fn appears_within(path: &Path, timeout: Duration) -> bool {
	let deadline = Instant::now() + timeout;
	while Instant::now() < deadline {
		if path.exists() {
			return true;
		}
		thread::sleep(Duration::from_millis(50));
	}
	false
}

/// The child's stderr so far, read live from the tee thread: what a
/// wait helper sees while the process still runs.
pub fn stderr_so_far(server: &Server) -> String {
	server.stderr.as_ref().map_or_else(
		|| "<no stderr pipe>".to_owned(),
		|collected| {
			collected
				.lock()
				.unwrap_or_else(std::sync::PoisonError::into_inner)
				.clone()
		},
	)
}

/// Waits until the child's stderr holds `needle` at least `times`
/// times, so a proof can wait for the log line it is about to assert
/// on instead of sleeping past a guessed delay. `false` on timeout.
pub fn stderr_contains_within(
	server: &Server,
	needle: &str,
	times: usize,
	timeout: Duration,
) -> bool {
	let deadline = Instant::now() + timeout;
	loop {
		let occurrences = server.stderr.as_ref().map_or(0, |collected| {
			collected
				.lock()
				.unwrap_or_else(std::sync::PoisonError::into_inner)
				.split(needle)
				.count()
				.saturating_sub(1)
		});
		if occurrences >= times {
			return true;
		}
		if Instant::now() > deadline {
			return false;
		}
		thread::sleep(Duration::from_millis(50));
	}
}

/// A concise report of one server's stderr, for failure messages.
/// Completes once the process has exited; a live process is killed so
/// the pipe ends.
pub fn stderr_of(server: &mut Server) -> String {
	let finished = server
		.stderr_thread
		.as_ref()
		.is_none_or(thread::JoinHandle::is_finished);
	if !finished {
		let _ = server.child.kill();
	}
	if let Some(handle) = server.stderr_thread.take() {
		let _ = handle.join();
	}
	stderr_so_far(server)
}

/// Formats a failure context into panic messages consistently.
pub fn context(message: &str, details: &str) -> String {
	let mut text = String::new();
	let _ = writeln!(text, "{message}");
	let _ = writeln!(text, "{details}");
	text
}

/// The Unix milliseconds for a UTC day's noon, offset by whole days from
/// today.
pub fn noon(days_from_now: i64) -> i64 {
	let now = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.expect("the clock is sane")
		.as_millis() as i64;
	let day = 86_400_000;
	let midnight = now - now.rem_euclid(day);
	midnight + days_from_now * day + 12 * 3_600_000
}

/// Today's UTC date, as the service names generations.
pub fn today() -> String {
	date_of(noon(0))
}

/// Tomorrow's UTC date.
pub fn tomorrow() -> String {
	date_of(noon(1))
}

/// `YYYY-MM-DD` from Unix milliseconds, matching the service's clock, so
/// the tests never disagree with the service about what day it is.
pub fn date_of(millis: i64) -> String {
	// Days since the Unix epoch, then the civil-from-days algorithm.
	let days = millis.div_euclid(86_400_000);
	let z = days + 719_468;
	let era = z.div_euclid(146_097);
	let doe = z.rem_euclid(146_097);
	let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
	let year = yoe + era * 400;
	let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
	let mp = (5 * doy + 2) / 153;
	let day = doy - (153 * mp + 2) / 5 + 1;
	let month = if mp < 10 { mp + 3 } else { mp - 9 };
	let year = if month <= 2 { year + 1 } else { year };
	format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn dates_match_the_service() {
		// 2026-11-15T00:00:00Z, and a moment before it.
		assert_eq!(date_of(1_794_700_800_000), "2026-11-15");
		assert_eq!(date_of(1_794_700_799_999), "2026-11-14");
	}

	#[test]
	fn responses_parse() {
		let wire = b"HTTP/1.1 421 Misdirected Request\r\ncontent-length: 5\r\n\r\nhello";
		let (status, headers, body) = parse(wire);
		assert_eq!(status, 421);
		assert_eq!(body, "hello");
		assert_eq!(headers.len(), 1);
	}
}
