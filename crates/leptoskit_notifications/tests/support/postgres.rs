use std::net::TcpListener;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;

/// A private Postgres for this test run, on the loopback interface, with
/// trust authentication and a data directory that dies with it.
pub struct Postgres {
	binaries: PathBuf,
	directory: tempfile::TempDir,
	port: u16,
}

impl Postgres {
	pub fn start() -> Option<Self> {
		let Some((binaries, server)) = find_binaries() else {
			assert!(
				std::env::var_os("LEPTOSKIT_REQUIRE_POSTGRES").is_none(),
				"LEPTOSKIT_REQUIRE_POSTGRES is set, but no Postgres binaries were found"
			);
			eprintln!("skipping: no Postgres binaries (initdb, pg_ctl, psql) found");

			return None;
		};

		let directory = tempfile::tempdir().unwrap();
		let data = directory.path().join("data");
		let port = TcpListener::bind("127.0.0.1:0")
			.unwrap()
			.local_addr()
			.unwrap()
			.port();

		run(Command::new(binaries.join("initdb")).args([
			"-D",
			data.to_string_lossy().as_ref(),
			"-U",
			"postgres",
			"-A",
			"trust",
			"-E",
			"UTF8",
			"--locale=C",
			"--no-sync",
		]));

		let log = directory.path().join("log");
		let started = Command::new(binaries.join("pg_ctl"))
			.arg("-D")
			.arg(&data)
			.arg("-p")
			.arg(&server)
			.arg("-l")
			.arg(&log)
			.arg("-o")
			.arg(format!(
				"-p {port} -c listen_addresses=127.0.0.1 -F -c unix_socket_directories=''"
			))
			.args(["-w", "start"])
			.stdin(Stdio::null())
			.stdout(Stdio::null())
			.stderr(Stdio::null())
			.status()
			.expect("run pg_ctl");
		assert!(
			started.success(),
			"pg_ctl couldn't start Postgres:\n{}",
			std::fs::read_to_string(&log).unwrap_or_default()
		);

		Some(Self {
			binaries,
			directory,
			port,
		})
	}

	/// A new, empty database on it, as a connection URL.
	pub fn database(&self) -> String {
		static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

		let name = format!(
			"notifications_{}_{}",
			std::process::id(),
			NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
		);
		run(Command::new(self.binaries.join("psql")).args([
			"-X",
			"-q",
			"-v",
			"ON_ERROR_STOP=1",
			"-h",
			"127.0.0.1",
			"-U",
			"postgres",
			"-d",
			"postgres",
			"-p",
			self.port.to_string().as_ref(),
			"-c",
			&format!("DROP DATABASE IF EXISTS {name}"),
		]));
		run(Command::new(self.binaries.join("psql")).args([
			"-X",
			"-q",
			"-v",
			"ON_ERROR_STOP=1",
			"-h",
			"127.0.0.1",
			"-U",
			"postgres",
			"-d",
			"postgres",
			"-p",
			self.port.to_string().as_ref(),
			"-c",
			&format!("CREATE DATABASE {name}"),
		]));

		format!(
			"postgres://postgres@127.0.0.1:{}/{name}?sslmode=disable",
			self.port
		)
	}
}

impl Drop for Postgres {
	fn drop(&mut self) {
		let _ = Command::new(self.binaries.join("pg_ctl"))
			.arg("-D")
			.arg(self.directory.path().join("data"))
			.args(["-m", "immediate", "-w", "stop"])
			.stdin(Stdio::null())
			.stdout(Stdio::null())
			.stderr(Stdio::null())
			.status();
	}
}

fn find_binaries() -> Option<(PathBuf, PathBuf)> {
	let has_initdb = |directory: &Path| directory.join("initdb").exists();
	let from_env = ["LEPTOSKIT_POSTGRES_BIN", "PGBIN"]
		.into_iter()
		.filter_map(std::env::var_os)
		.map(PathBuf::from);
	let from_path = std::env::var_os("PATH")
		.into_iter()
		.flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>());

	let directory = from_env
		.chain(from_path)
		.find(|directory| has_initdb(directory))?;
	let server = std::fs::canonicalize(directory.join("postgres")).ok()?;
	let binaries = std::fs::canonicalize(directory.join("initdb"))
		.ok()?
		.parent()
		.map(Path::to_path_buf)?;

	Some((binaries, server))
}

fn run(command: &mut Command) -> String {
	let output = command
		.output()
		.unwrap_or_else(|error| panic!("couldn't run {command:?}: {error}"));
	assert!(
		output.status.success(),
		"{command:?} failed:\n{}",
		String::from_utf8_lossy(&output.stderr)
	);
	String::from_utf8(output.stdout).unwrap()
}
