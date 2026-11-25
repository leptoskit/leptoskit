//! A builder that is a program the operator names.
//!
//! The program is run once for each build, in the unpacked source, with a
//! cleared environment and these variables:
//!
//! - `LEPTOSKIT_BUILD_SOURCE`: the source directory, also where it starts;
//! - `LEPTOSKIT_BUILD_INPUT`: a JSON file holding a [`BuildRequest`];
//! - `LEPTOSKIT_BUILD_OUTPUT`: where it writes how the build ended, as JSON:
//!   `{"result": "succeeded", …}` with a `BuildServerOutput`'s fields, or
//!   `{"result": "failed", "reason": "…", "steps": […]}`;
//! - `HOME` and `TMPDIR`: empty directories deleted with the source;
//! - `PATH`, and any variable the operator named to pass on.
//!
//! What it prints, on either stream, is the build's log. It is stopped,
//! with what it started in its own process group, when it ends, when the
//! build is cancelled, or when it runs past its time. A process that
//! leaves the group, by `setsid` or by asking a daemon such as Docker's to
//! start it, is not stopped: a build command that starts one must stop it
//! when it is asked to end.
//!
//! Nothing here isolates the program from the machine: see the crate's
//! documentation.

use std::future::Future;
use std::path::Path;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::Stdio;
use std::time::Duration;

use leptoskit_cloud::archive::Digest;
use leptoskit_cloud::deployments::AppFacts;
use leptoskit_cloud::workers::BuildOutcome;
use leptoskit_cloud::workers::BuildServerInput;
use leptoskit_cloud::workers::DeploymentRef;
use leptoskit_cloud::workers::MAX_LOG_LINE_BYTES;
use serde::Deserialize;
use serde::Serialize;
use tokio::io::AsyncBufReadExt as _;
use tokio::io::AsyncRead;
use tokio::io::AsyncReadExt as _;

use crate::build::BuildJob;
use crate::build::Builder;
use crate::logs::LogSink;

/// The most of the program's result that is read.
const MAX_OUTPUT: u64 = 1024 * 1024;

/// How long the program has to end once it is asked to, before it is
/// killed.
const GRACE: Duration = Duration::from_secs(10);

/// The longest a failure's reason is.
const MAX_REASON: usize = 500;

/// What the program is asked to build: a job's input without the
/// archive's address, which is a credential the program has no use for.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BuildRequest {
	pub deployment: DeploymentRef,
	/// The source archive's SHA-256.
	pub digest: Digest,
	pub toolchain: String,
	pub targets: Vec<String>,
	/// Where the image is pushed. The result must name the same.
	pub image_repository: String,
	pub app: AppFacts,
}

impl From<&BuildServerInput> for BuildRequest {
	fn from(input: &BuildServerInput) -> Self {
		Self {
			deployment: input.deployment.clone(),
			digest: input.digest.clone(),
			toolchain: input.toolchain.clone(),
			targets: input.targets.clone(),
			image_repository: input.image_repository.clone(),
			app: input.app.clone(),
		}
	}
}

/// Builds by running a program.
#[derive(Clone, Debug)]
pub struct CommandBuilder {
	program: PathBuf,
	arguments: Vec<String>,
	/// Names of the worker's own variables the program is given too.
	pass: Vec<String>,
	/// Optional persistent cache directory shared across builds (not deleted
	/// with the per-build workspace). Exposes `CARGO_HOME`, `SCCACHE_DIR`, and
	/// `LEPTOSKIT_BUILD_CACHE_DIR` to the build command.
	cache_dir: Option<PathBuf>,
	/// Optional sandbox wrapper command prefix (e.g. `["bwrap",
	/// "--unshare-pid", ...]`). When non-empty, `sandbox[0]` is spawned
	/// with `sandbox[1..]`, followed by `program` and `arguments`.
	sandbox: Vec<String>,
	timeout: Duration,
}

impl CommandBuilder {
	/// Runs `program` with `arguments` for each build, for an hour at
	/// most.
	#[must_use]
	pub fn new(program: impl Into<PathBuf>, arguments: Vec<String>) -> Self {
		Self {
			program: program.into(),
			arguments,
			pass: Vec::new(),
			cache_dir: None,
			sandbox: Vec::new(),
			timeout: Duration::from_hours(1),
		}
	}

	/// Also gives the program the worker's own variables named `names`,
	/// such as a registry's credentials.
	#[must_use]
	pub fn pass(mut self, names: Vec<String>) -> Self {
		self.pass = names;
		self
	}

	/// Persists Cargo, `sccache`, and `BuildKit` caches under `dir` across
	/// builds instead of discarding them with each ephemeral workspace.
	#[must_use]
	pub fn cache_dir(mut self, dir: impl Into<PathBuf>) -> Self {
		self.cache_dir = Some(dir.into());
		self
	}

	/// Wraps the build program with a sandbox runner prefix (such as `bwrap`
	/// or a container runner). Empty disables wrapping.
	#[must_use]
	pub fn sandbox(mut self, wrapper: Vec<String>) -> Self {
		self.sandbox = wrapper;
		self
	}

	/// Stops a build that runs longer than `timeout`.
	#[must_use]
	pub fn timeout(mut self, timeout: Duration) -> Self {
		self.timeout = timeout;
		self
	}
}

fn failed(reason: impl Into<String>) -> BuildOutcome {
	BuildOutcome::Failed {
		reason: shortened(&reason.into()),
		steps: Vec::new(),
	}
}

/// `reason`, held to a reason's length, with its control characters as
/// spaces: it is shown to an operator as well as to the organisation.
fn shortened(reason: &str) -> String {
	let mut reason: String = reason
		.chars()
		.map(|c| if c.is_control() { ' ' } else { c })
		.collect();

	if reason.len() > MAX_REASON {
		let mut end = MAX_REASON;

		while !reason.is_char_boundary(end) {
			end -= 1;
		}

		reason.truncate(end);
	}

	reason
}

/// Copies `stream`'s lines to the log. A line longer than a log's line
/// is sent in pieces, so a build that never ends a line holds no more
/// than one line's worth of the worker's memory.
async fn copy(stream: impl AsyncRead + Unpin, log: LogSink, redact: &Redactions) {
	let mut reader = tokio::io::BufReader::new(stream);
	let mut line = Vec::new();

	let send = |line: &mut Vec<u8>| {
		// Read by bytes: a build may print what isn't UTF-8.
		let text = String::from_utf8_lossy(line);
		log.line(redact.apply(text.trim_end_matches(['\n', '\r'])));
		line.clear();
	};

	while let Ok(buffer) = reader.fill_buf().await {
		if buffer.is_empty() {
			break;
		}

		let room = MAX_LOG_LINE_BYTES - line.len();
		let (taken, ended) = match buffer.iter().take(room).position(|&byte| byte == b'\n') {
			Some(at) => (at + 1, true),
			None => (buffer.len().min(room), false),
		};
		line.extend_from_slice(&buffer[..taken]);
		reader.consume(taken);

		if ended || line.len() >= MAX_LOG_LINE_BYTES {
			send(&mut line);
		}
	}

	if !line.is_empty() {
		send(&mut line);
	}
}

/// What replaces a value no log line may show.
const REDACTED: &str = "[redacted]";

/// The shortest run of a credential's characters that is hidden on its
/// own: shorter runs are ordinary words as often as a key's.
const SHORTEST_RUN: usize = 16;

/// The shortest value hidden whole: a credential shorter than this, such
/// as an alias, would hide ordinary words.
const SHORTEST_VALUE: usize = 8;

/// The shortest password hidden whole.
const SHORTEST_PASSWORD: usize = 4;

/// Whether `c` belongs to a run of a key's characters: base64, hex, and
/// the like.
fn in_run(c: char) -> bool {
	c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=' | '_' | '-')
}

/// What no log line of a job may show: its credentials' values.
#[derive(Clone, Debug, Default)]
pub(crate) struct Redactions {
	/// Each value, whole.
	values: Vec<String>,
	/// Each value's runs of a key's characters, [`SHORTEST_RUN`] or longer.
	runs: Vec<String>,
}

impl Redactions {
	/// What hides `values`, by their names: a password is hidden whole
	/// at any length of four or more, since a short one is still a
	/// password.
	pub(crate) fn new<'a>(values: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
		let mut redactions = Self::default();

		for (name, value) in values {
			let shortest = if name.contains("PASSWORD") {
				SHORTEST_PASSWORD
			} else {
				SHORTEST_VALUE
			};

			if value.trim().len() >= shortest {
				redactions.values.push(value.trim().to_owned());
			}

			redactions.runs.extend(
				value
					.split(|c: char| !in_run(c))
					.filter(|run| run.len() >= SHORTEST_RUN)
					.map(str::to_owned),
			);
		}

		redactions
	}

	/// `line` with each value in it, and each long run of characters that
	/// is part of one, replaced. A line is cut into pieces of the
	/// protocol's length, and a key's run printed whole may be cut with it:
	/// a run is hidden when it is part of one of a value's, not only all of
	/// one.
	pub(crate) fn apply(&self, line: &str) -> String {
		if self.values.is_empty() && self.runs.is_empty() {
			return line.to_owned();
		}

		let mut line = line.to_owned();

		for value in &self.values {
			if line.contains(value.as_str()) {
				line = line.replace(value.as_str(), REDACTED);
			}
		}

		let mut out = String::with_capacity(line.len());
		let mut rest = line.as_str();

		while let Some(start) = rest.find(in_run) {
			out.push_str(&rest[..start]);
			rest = &rest[start..];
			let end = rest.find(|c: char| !in_run(c)).unwrap_or(rest.len());
			let run = &rest[..end];

			if run.len() >= SHORTEST_RUN && self.runs.iter().any(|known| known.contains(run)) {
				out.push_str(REDACTED);
			} else {
				out.push_str(run);
			}

			rest = &rest[end..];
		}

		out.push_str(rest);
		out
	}
}

/// The group of the build command running now, or 0: what a worker that
/// is told to stop at once kills on its way out.
static RUNNING: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// Kills the build command running now, and what it started in its group.
/// For a worker that is about to exit without ending its build.
pub fn kill_running_build() {
	let group = RUNNING.swap(0, std::sync::atomic::Ordering::SeqCst);
	signal(rustix::process::Pid::from_raw(group), true);
}

/// The process group a program leads, taken as it starts: once it has
/// ended its ID is gone, and what it started may not be.
type Group = Option<rustix::process::Pid>;

fn group_of(child: &tokio::process::Child) -> Group {
	child
		.id()
		.and_then(|id| i32::try_from(id).ok())
		.and_then(rustix::process::Pid::from_raw)
}

fn signal(group: Group, kill: bool) {
	if let Some(group) = group {
		let signal = if kill {
			rustix::process::Signal::KILL
		} else {
			rustix::process::Signal::TERM
		};
		// The program leads a process group of its own, so this reaches
		// whatever it started and nothing of the worker's.
		let _ = rustix::process::kill_process_group(group, signal);
	}
}

/// Asks the program and everything it started to end, then makes them.
async fn end(child: &mut tokio::process::Child, group: Group) {
	signal(group, false);

	if tokio::time::timeout(GRACE, child.wait()).await.is_err() {
		let _ = child.kill().await;
	}

	// What outlived the program itself.
	gone(group);
}

/// Kills what is left of the program's group, if anything is: a group
/// with no one left in it has an ID that may be another's by now.
fn gone(group: Group) {
	if let Some(id) = group
		&& rustix::process::test_kill_process_group(id).is_ok()
	{
		signal(group, true);
	}
}

/// What the program wrote at `path`, checked against what it was asked.
async fn result(path: &Path, input: &BuildServerInput) -> Option<BuildOutcome> {
	let bytes = written(path).await?;
	let outcome: BuildOutcome = serde_json::from_slice(&bytes).ok()?;

	match outcome {
		BuildOutcome::Succeeded(output)
			if std::iter::once(&output.image)
				.chain(
					output
						.processes
						.iter()
						.filter_map(|process| process.image.as_ref()),
				)
				.any(|image| image.repository != input.image_repository) =>
		{
			Some(failed(
				"the build command pushed its image somewhere other than where it was told to",
			))
		}
		// Only the worker says a build was stopped.
		BuildOutcome::Stopped => None,
		// The build's own words, held to a reason's length.
		BuildOutcome::Failed { reason, steps } => {
			Some(BuildOutcome::Failed {
				reason: shortened(&reason),
				steps,
			})
		}
		outcome @ BuildOutcome::Succeeded(_) => Some(outcome),
	}
}

/// How a program the worker ran ended.
pub(crate) enum Exit {
	/// It ended by itself, with this status.
	Exited(std::io::Result<std::process::ExitStatus>),
	/// It was told to stop, and was.
	Stopped,
	/// It ran past `minutes` and was stopped.
	TimedOut { minutes: u64 },
	/// It couldn't be started.
	Unstarted,
}

/// Derives an isolated cache subdirectory for one `(organization, project)`
/// pair so writable Cargo / sccache / `BuildKit` caches are never shared across
/// tenants on a multi-tenant build worker.
fn scoped_cache_root(base: &Path, organization: &str, project: &str) -> PathBuf {
	let sanitize = |segment: &str| -> String {
		let cleaned: String = segment
			.chars()
			.filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
			.take(64)
			.collect();
		if cleaned.is_empty() {
			"default".to_owned()
		} else {
			cleaned
		}
	};
	base.join("tenants")
		.join(sanitize(organization))
		.join(sanitize(project))
}

/// One run of an operator's program: in `source`, with a cleared
/// environment but for `PATH`, what the operator passes, and `env`.
pub(crate) struct Launch<'a, S: AsRef<std::ffi::OsStr> = String> {
	pub(crate) program: &'a Path,
	pub(crate) arguments: &'a [S],
	/// Names of the worker's own variables the program is given too.
	pub(crate) pass: &'a [String],
	pub(crate) timeout: Duration,
	pub(crate) source: &'a Path,
	pub(crate) env: Vec<(String, std::ffi::OsString)>,
	pub(crate) log: LogSink,
	pub(crate) stop: crate::build::Stop,
	/// What no log line may show.
	pub(crate) redact: std::sync::Arc<Redactions>,
	/// What the program is, in the log: `build` or `publish`.
	pub(crate) what: &'static str,
}

impl<S: AsRef<std::ffi::OsStr>> Launch<'_, S> {
	/// Runs the program to its end, and stops what it started.
	pub(crate) async fn execute(self) -> Exit {
		let mut command = tokio::process::Command::new(self.program);
		command
			.args(self.arguments)
			.current_dir(self.source)
			.env_clear()
			.stdin(Stdio::null())
			.stdout(Stdio::piped())
			.stderr(Stdio::piped())
			.kill_on_drop(true);

		for name in std::iter::once("PATH").chain(self.pass.iter().map(String::as_str)) {
			if let Some(value) = std::env::var_os(name) {
				command.env(name, value);
			}
		}

		// The job's own, after the operator's: a job's name wins.
		for (name, value) in &self.env {
			command.env(name, value);
		}

		command.process_group(0);
		let child = match command.spawn() {
			Ok(child) => child,
			Err(error) => {
				self.log.line(format!(
					"worker: couldn't start the {} command: {}",
					self.what,
					error.kind()
				));
				return Exit::Unstarted;
			}
		};
		let group = group_of(&child);
		RUNNING.store(
			rustix::process::Pid::as_raw(group),
			std::sync::atomic::Ordering::SeqCst,
		);
		let exit = self.wait(child, group).await;
		RUNNING.store(0, std::sync::atomic::Ordering::SeqCst);
		exit
	}

	async fn wait(self, mut child: tokio::process::Child, group: Group) -> Exit {
		let mut stop = self.stop;
		let mut streams = tokio::spawn({
			let out = child.stdout.take();
			let err = child.stderr.take();
			let log = self.log.clone();
			let redact = std::sync::Arc::clone(&self.redact);
			async move {
				let out = async {
					if let Some(out) = out {
						copy(out, log.clone(), &redact).await;
					}
				};
				let err = async {
					if let Some(err) = err {
						copy(err, log.clone(), &redact).await;
					}
				};
				tokio::join!(out, err);
			}
		});

		enum Ended {
			Exited(std::io::Result<std::process::ExitStatus>),
			Stopped,
			TimedOut,
		}

		let ended = tokio::select! {
			status = child.wait() => Ended::Exited(status),
			() = stop.stopped() => Ended::Stopped,
			() = tokio::time::sleep(self.timeout) => Ended::TimedOut,
		};

		let status = match ended {
			Ended::Exited(status) => {
				// Whatever the program left running goes with it.
				gone(group);
				status
			}
			Ended::Stopped => {
				end(&mut child, group).await;
				streams.abort();
				return Exit::Stopped;
			}
			Ended::TimedOut => {
				end(&mut child, group).await;
				streams.abort();
				let minutes = self.timeout.as_secs().div_ceil(60).max(1);
				self.log.line(format!(
					"worker: the {} ran past its limit of {minutes} min and was stopped",
					self.what
				));
				return Exit::TimedOut { minutes };
			}
		};
		// The streams close once every process holding them is gone; one
		// that left the group may hold them open for good.
		if tokio::time::timeout(GRACE, &mut streams).await.is_err() {
			streams.abort();
		}

		Exit::Exited(status)
	}
}

/// The bytes the program wrote at `path`, if it is a regular file of
/// [`MAX_OUTPUT`] bytes at most.
///
/// The file is opened without following a link and without waiting on a
/// pipe: what is left of a program may have put anything there.
pub(crate) async fn written(path: &Path) -> Option<Vec<u8>> {
	let flags = rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK;
	let file = tokio::fs::OpenOptions::new()
		.read(true)
		.custom_flags(flags.bits().cast_signed())
		.open(path)
		.await
		.ok()?;
	let metadata = file.metadata().await.ok()?;

	if !metadata.is_file() || metadata.len() > MAX_OUTPUT {
		return None;
	}

	let mut bytes = Vec::new();
	file.take(MAX_OUTPUT + 1)
		.read_to_end(&mut bytes)
		.await
		.ok()?;

	if bytes.len() as u64 > MAX_OUTPUT {
		return None;
	}

	Some(bytes)
}

/// Makes `HOME` and `TMPDIR` for a program under `scratch`, and writes
/// `request` to `input` there.
pub(crate) async fn prepare(
	scratch: &Path,
	input: &Path,
	request: &impl Serialize,
) -> std::io::Result<(PathBuf, PathBuf)> {
	let home = scratch.join("home");
	let temporary = scratch.join("tmp");
	tokio::fs::create_dir_all(&home).await?;
	tokio::fs::create_dir_all(&temporary).await?;
	let json = serde_json::to_vec_pretty(request).map_err(std::io::Error::other)?;
	tokio::fs::write(input, json).await?;
	Ok((home, temporary))
}

impl CommandBuilder {
	async fn run(&self, job: BuildJob<'_>) -> BuildOutcome {
		let BuildJob {
			input,
			source,
			scratch,
			log,
			stop,
		} = job;
		let request = scratch.join("input.json");
		let output = scratch.join("output.json");
		let Ok((home, temporary)) = prepare(scratch, &request, &BuildRequest::from(input)).await
		else {
			return failed("the worker couldn't prepare the build's directory");
		};
		let mut env = vec![
			("LEPTOSKIT_BUILD_SOURCE".to_owned(), source.into()),
			("LEPTOSKIT_BUILD_INPUT".to_owned(), request.into()),
			("LEPTOSKIT_BUILD_OUTPUT".to_owned(), output.clone().into()),
			("HOME".to_owned(), home.into()),
			("TMPDIR".to_owned(), temporary.into()),
		];
		if let Some(cache_root) = &self.cache_dir {
			let scoped = scoped_cache_root(
				cache_root,
				&input.deployment.organization,
				&input.deployment.project,
			);
			let cargo_home = scoped.join("cargo");
			let sccache_dir = scoped.join("sccache");
			let buildkit_dir = scoped.join("buildkit");
			if crate::state::private_directory(&scoped).is_err()
				|| crate::state::private_directory(&cargo_home).is_err()
				|| crate::state::private_directory(&sccache_dir).is_err()
				|| crate::state::private_directory(&buildkit_dir).is_err()
			{
				return failed("the worker couldn't prepare its persistent build cache directory");
			}
			env.push(("LEPTOSKIT_BUILD_CACHE_DIR".to_owned(), scoped.into()));
			env.push(("CARGO_HOME".to_owned(), cargo_home.into()));
			env.push(("SCCACHE_DIR".to_owned(), sccache_dir.into()));
		}

		let (program, arguments): (PathBuf, Vec<std::ffi::OsString>) =
			if let Some((head, rest)) = self.sandbox.split_first() {
				if head.trim().is_empty() {
					return failed("the build sandbox program is empty");
				}
				let mut args: Vec<std::ffi::OsString> =
					rest.iter().map(std::ffi::OsString::from).collect();
				args.push(self.program.as_os_str().to_owned());
				args.extend(self.arguments.iter().map(std::ffi::OsString::from));
				(PathBuf::from(head), args)
			} else {
				(
					self.program.clone(),
					self.arguments
						.iter()
						.map(std::ffi::OsString::from)
						.collect(),
				)
			};

		let exit = Launch {
			program: &program,
			arguments: &arguments,
			pass: &self.pass,
			timeout: self.timeout,
			source,
			env,
			log,
			stop,
			redact: std::sync::Arc::default(),
			what: "build",
		}
		.execute()
		.await;
		let status = match exit {
			Exit::Exited(status) => status,
			Exit::Stopped => return BuildOutcome::Stopped,
			Exit::TimedOut { minutes } => {
				return failed(format!("the build ran past its limit of {minutes} min"));
			}
			Exit::Unstarted => return failed("the worker couldn't start its build command"),
		};
		let written = result(&output, input).await;

		match (status, written) {
			(Ok(status), Some(outcome)) if status.success() => outcome,
			(Ok(status), _) if status.success() => {
				failed("the build command ended without writing what it built")
			}
			// A build that says why it failed is believed over its status.
			(_, Some(outcome @ BuildOutcome::Failed { .. })) => outcome,
			(Ok(status), _) => {
				failed(match status.code() {
					Some(code) => format!("the build command exited with status {code}"),
					None => "the build command was killed".to_owned(),
				})
			}
			(Err(_), _) => failed("the worker lost track of its build command"),
		}
	}
}

impl Builder for CommandBuilder {
	fn build<'a>(
		&'a self,
		job: BuildJob<'a>,
	) -> Pin<Box<dyn Future<Output = BuildOutcome> + Send + 'a>> {
		Box::pin(self.run(job))
	}
}

#[cfg(test)]
mod tests {
	use leptoskit_cloud::workers::Download;

	use super::*;
	use crate::build::Stop;

	const REPOSITORY: &str = "registry.example.test/leptoskit/ana/notes";

	fn input() -> BuildServerInput {
		BuildServerInput {
			deployment: DeploymentRef {
				organization: "ana".to_owned(),
				project: "notes".to_owned(),
				environment: "production".to_owned(),
				number: 3,
			},
			archive: Download {
				url: "https://store.example.test/secret-address".to_owned(),
				expires_at: "2026-11-19T00:00:00Z".parse().expect("a time"),
			},
			digest: Digest::of(b"source"),
			size: 6,
			toolchain: "stable".to_owned(),
			targets: vec!["x86_64-unknown-linux-gnu".to_owned()],
			image_repository: REPOSITORY.to_owned(),
			app: AppFacts::default(),
		}
	}

	/// What a build of `script`, a shell script, ended as, and its log.
	async fn build(
		script: &str,
		builder: impl FnOnce(CommandBuilder) -> CommandBuilder,
		stop: Stop,
	) -> (BuildOutcome, Vec<String>, tempfile::TempDir) {
		let directory = tempfile::tempdir().expect("a directory");
		let source = directory.path().join("source");
		let scratch = directory.path().join("scratch");
		std::fs::create_dir_all(&source).expect("a source");
		std::fs::create_dir_all(&scratch).expect("a scratch");
		std::fs::write(source.join("Cargo.toml"), "[package]\n").expect("a file");
		let (log, mut lines) = LogSink::collecting();
		let input = input();
		let command = builder(CommandBuilder::new(
			"/bin/sh",
			vec!["-c".to_owned(), script.to_owned()],
		));
		let outcome = command
			.build(BuildJob {
				input: &input,
				source: &source,
				scratch: &scratch,
				log,
				stop,
			})
			.await;
		let mut log = Vec::new();

		while let Ok(line) = lines.try_recv() {
			log.push(line);
		}

		(outcome, log, directory)
	}

	fn succeeded(repository: &str) -> String {
		format!(
			r#"{{"result":"succeeded","image":{{"repository":"{repository}","digest":"{}"}},"processes":[{{"name":"app","command":["/app/server"],"port":8080,"ready_path":"/readyz","live_path":"/healthz"}}],"migrations":[],"steps":[{{"name":"cargo build","millis":1200}}]}}"#,
			Digest::of(b"image").as_str()
		)
	}

	#[tokio::test]
	async fn a_build_runs_in_the_source_and_answers_in_a_file() {
		// Compared as real paths: the shell's `$PWD` is one, and macOS's
		// temporary directory is reached through a link.
		let script = format!(
			"test -f Cargo.toml && test \"$(pwd -P)\" = \"$(cd \"$LEPTOSKIT_BUILD_SOURCE\" && pwd \
			 -P)\" && echo building && echo careful >&2 && grep -q '\"image_repository\"' \
			 \"$LEPTOSKIT_BUILD_INPUT\" && ! grep -q secret-address \"$LEPTOSKIT_BUILD_INPUT\" && \
			 cat > \"$LEPTOSKIT_BUILD_OUTPUT\" <<'JSON'\n{}\nJSON",
			succeeded(REPOSITORY)
		);
		let (outcome, mut log, _directory) = build(&script, |builder| builder, Stop::never()).await;
		let BuildOutcome::Succeeded(output) = outcome else {
			panic!("{outcome:?} {log:?}");
		};
		assert_eq!(output.image.repository, REPOSITORY);
		assert_eq!(output.processes[0].name, "app");
		assert_eq!(output.steps[0].millis, 1200);
		log.sort();
		assert_eq!(log, ["building", "careful"]);
	}

	#[tokio::test]
	async fn the_environment_is_cleared_but_for_what_is_named() {
		// Cargo sets both for a test's process.
		let script = "echo \"hidden=${CARGO_PKG_VERSION:-none} passed=${CARGO_PKG_NAME:-none}\"; \
		              test -d \"$HOME\" && test -d \"$TMPDIR\" && echo dirs; exit 3";
		let (outcome, log, _directory) = build(
			script,
			|builder| builder.pass(vec!["CARGO_PKG_NAME".to_owned()]),
			Stop::never(),
		)
		.await;
		assert_eq!(log, ["hidden=none passed=leptoskit_cloud_worker", "dirs"]);
		assert_eq!(
			outcome,
			BuildOutcome::Failed {
				reason: "the build command exited with status 3".to_owned(),
				steps: Vec::new(),
			}
		);
	}

	#[tokio::test]
	async fn a_build_says_why_it_failed_or_is_told() {
		let script = "printf '%s' '{\"result\":\"failed\",\"reason\":\"src/main.rs:4: expected \
		              `;`\",\"steps\":[{\"name\":\"cargo build\",\"millis\":10}]}' > \
		              \"$LEPTOSKIT_BUILD_OUTPUT\"; exit 1";
		let (outcome, _, _directory) = build(script, |builder| builder, Stop::never()).await;
		let BuildOutcome::Failed { reason, steps } = outcome else {
			panic!("{outcome:?}");
		};
		assert_eq!(reason, "src/main.rs:4: expected `;`");
		assert_eq!(steps.len(), 1);

		for (script, expected) in [
			("true", "ended without writing what it built"),
			(
				"echo nonsense > \"$LEPTOSKIT_BUILD_OUTPUT\"",
				"ended without writing",
			),
			// Only the worker says a build was stopped.
			(
				"echo '{\"result\":\"stopped\"}' > \"$LEPTOSKIT_BUILD_OUTPUT\"",
				"ended without writing",
			),
			("kill -9 $$", "was killed"),
		] {
			let (outcome, _, _directory) = build(script, |builder| builder, Stop::never()).await;
			let BuildOutcome::Failed { reason, .. } = outcome else {
				panic!("{script}: {outcome:?}");
			};
			assert!(reason.contains(expected), "{script}: {reason}");
		}

		// An image pushed somewhere else isn't the one that was asked for.
		let script = format!(
			"cat > \"$LEPTOSKIT_BUILD_OUTPUT\" <<'JSON'\n{}\nJSON",
			succeeded("registry.example.test/leptoskit/bob/other")
		);
		let (outcome, _, _directory) = build(&script, |builder| builder, Stop::never()).await;
		let BuildOutcome::Failed { reason, .. } = outcome else {
			panic!("{outcome:?}");
		};
		assert!(reason.contains("somewhere other"), "{reason}");

		// A program that isn't there is the worker's failure to say.
		let directory = tempfile::tempdir().expect("a directory");
		let (log, _lines) = LogSink::collecting();
		let outcome = CommandBuilder::new("/nonexistent/build", Vec::new())
			.build(BuildJob {
				input: &input(),
				source: directory.path(),
				scratch: directory.path(),
				log,
				stop: Stop::never(),
			})
			.await;
		assert!(
			matches!(outcome, BuildOutcome::Failed { .. }),
			"{outcome:?}"
		);
	}

	/// Whether the process whose ID `file` holds is gone, given a moment:
	/// a signal is delivered soon, not at once.
	async fn gone(file: &Path) -> bool {
		for _ in 0..50 {
			if !alive(file) {
				return true;
			}

			tokio::time::sleep(Duration::from_millis(100)).await;
		}

		false
	}

	/// Whether the process whose ID `file` holds is still running.
	fn alive(file: &Path) -> bool {
		let id: i32 = std::fs::read_to_string(file)
			.expect("a process ID")
			.trim()
			.parse()
			.expect("a number");
		// A process that was killed and not yet collected by whoever
		// inherited it still has an ID, and isn't running.
		if let Ok(stat) = std::fs::read_to_string(format!("/proc/{id}/stat")) {
			return !stat
				.rsplit_once(')')
				.is_some_and(|(_, rest)| rest.trim_start().starts_with('Z'));
		}

		let process = rustix::process::Pid::from_raw(id).expect("a process ID");
		rustix::process::test_kill_process(process).is_ok()
	}

	#[tokio::test]
	async fn a_build_past_its_time_is_stopped_with_what_it_started() {
		let marks = tempfile::tempdir().expect("a directory");
		let child = marks.path().join("child");
		// The program starts another that would outlive it, and ignores
		// being asked to end.
		let script = format!(
			"trap '' TERM; sleep 300 & echo $! > {}; echo started; sleep 300",
			child.display()
		);
		let started = std::time::Instant::now();
		let (outcome, log, _directory) = build(
			&script,
			|builder| builder.timeout(Duration::from_millis(600)),
			Stop::never(),
		)
		.await;
		let BuildOutcome::Failed { reason, .. } = outcome else {
			panic!("{outcome:?}");
		};
		assert!(reason.contains("ran past its limit of 1 min"), "{reason}");
		assert!(log.iter().any(|line| line == "started"), "{log:?}");
		assert!(started.elapsed() < Duration::from_secs(60));
		assert!(
			gone(&child).await,
			"what the build started is still running"
		);
	}

	#[tokio::test]
	async fn a_build_told_to_stop_stops() {
		let marks = tempfile::tempdir().expect("a directory");
		let child = marks.path().join("child");
		let script = format!("sleep 300 & echo $! > {}; sleep 300", child.display());
		let (stopper, stop) = Stop::new();
		tokio::spawn(async move {
			tokio::time::sleep(Duration::from_millis(500)).await;
			let _ = stopper.send(true);
			// Held until the build has seen it.
			tokio::time::sleep(Duration::from_secs(30)).await;
		});
		let started = std::time::Instant::now();
		let (outcome, _, _directory) = build(&script, |builder| builder, stop).await;
		assert_eq!(outcome, BuildOutcome::Stopped);
		assert!(started.elapsed() < Duration::from_secs(10));
		assert!(
			gone(&child).await,
			"what the build started is still running"
		);
	}

	#[tokio::test]
	async fn a_line_that_never_ends_is_sent_in_pieces() {
		// Three log lines' worth, with no newline at all.
		let script = format!("head -c {} /dev/zero | tr '\\0' x", MAX_LOG_LINE_BYTES * 3);
		let (_, log, _directory) = build(&script, |builder| builder, Stop::never()).await;
		assert_eq!(
			log.len(),
			3,
			"{:?}",
			log.iter().map(String::len).collect::<Vec<_>>()
		);
		assert!(log.iter().all(|line| line.len() == MAX_LOG_LINE_BYTES));
	}

	#[tokio::test]
	async fn only_a_regular_file_is_read_as_the_result() {
		// A link to a file holding a good result, and a pipe no one writes.
		for setup in [
			format!(
				"printf '%s' '{}' > \"$HOME/real.json\" && ln -s \"$HOME/real.json\" \
				 \"$LEPTOSKIT_BUILD_OUTPUT\"",
				succeeded(REPOSITORY)
			),
			"mkfifo \"$LEPTOSKIT_BUILD_OUTPUT\"".to_owned(),
		] {
			let (outcome, log, _directory) = tokio::time::timeout(
				Duration::from_secs(30),
				build(&setup, |builder| builder, Stop::never()),
			)
			.await
			.expect("a pipe isn't waited on");
			assert_eq!(
				outcome,
				failed("the build command ended without writing what it built"),
				"{log:?}"
			);
		}
	}

	#[tokio::test]
	async fn every_image_must_be_in_the_repository_it_was_given() {
		let elsewhere = succeeded(REPOSITORY).replace(
			r#""name":"app","#,
			&format!(
				r#""name":"app","image":{{"repository":"registry.example.test/someone/else","digest":"{}"}},"#,
				Digest::of(b"theirs").as_str()
			),
		);
		let script = format!("printf '%s' '{elsewhere}' > \"$LEPTOSKIT_BUILD_OUTPUT\"");
		let (outcome, _, _directory) = build(&script, |builder| builder, Stop::never()).await;
		assert_eq!(
			outcome,
			failed("the build command pushed its image somewhere other than where it was told to")
		);
	}

	#[tokio::test]
	async fn a_builds_own_reason_is_held_to_a_reasons_length() {
		let script = format!(
			"printf '{{\"result\":\"failed\",\"reason\":\"%s\\\\u001b[2Jfake\"}}' {} > \
			 \"$LEPTOSKIT_BUILD_OUTPUT\"; exit 1",
			"x".repeat(2000)
		);
		let (outcome, log, _directory) = build(&script, |builder| builder, Stop::never()).await;
		let BuildOutcome::Failed { reason, .. } = outcome else {
			panic!("{outcome:?} {log:?}");
		};
		assert_eq!(reason.len(), MAX_REASON);
		assert!(!reason.contains(char::is_control));
	}

	#[tokio::test]
	async fn cache_dir_exposes_persistent_cache_env_and_sandbox_wraps_command() {
		let cache = tempfile::tempdir().expect("cache dir");
		let cache_path = cache.path().to_path_buf();
		let script = format!(
			"test -d \"$CARGO_HOME\" && test -d \"$SCCACHE_DIR\" && test -d \
			 \"$LEPTOSKIT_BUILD_CACHE_DIR\" && echo cached > \"$CARGO_HOME/marker\" && cat > \
			 \"$LEPTOSKIT_BUILD_OUTPUT\" <<'JSON'\n{}\nJSON",
			succeeded(REPOSITORY)
		);
		let (outcome, log, _directory) = build(
			&script,
			|builder| {
				builder
					.cache_dir(&cache_path)
					.sandbox(vec!["/usr/bin/env".to_owned()])
			},
			Stop::never(),
		)
		.await;
		assert!(
			matches!(outcome, BuildOutcome::Succeeded(_)),
			"{outcome:?} {log:?}"
		);
		assert!(cache_path.join("tenants/ana/notes/cargo/marker").is_file());
		assert!(cache_path.join("tenants/ana/notes/buildkit").is_dir());
	}

	#[test]
	fn scoped_cache_root_isolates_tenants_and_sanitizes_segments() {
		let base = Path::new("/var/cache/leptoskit");
		let ana = scoped_cache_root(base, "ana", "notes");
		let bob = scoped_cache_root(base, "bob", "notes");
		assert_ne!(ana, bob);
		assert_eq!(ana, base.join("tenants/ana/notes"));

		let escaped = scoped_cache_root(base, "../etc", "///");
		assert_eq!(escaped, base.join("tenants/etc/default"));
	}
}
