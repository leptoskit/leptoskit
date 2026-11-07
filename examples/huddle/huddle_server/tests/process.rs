//! The `huddle` binary itself: the process that reads its environment,
//! connects the database, applies the migrations, starts the roles, serves
//! the readiness route and the site, and drains on `SIGTERM`.
//!
//! The server runs on a free localhost port with a throwaway SQLite file, so
//! the test exercises the same startup path a deployment does and nothing
//! outside this machine.

#![cfg(unix)]

use std::io::Read;
use std::time::Duration;
use std::time::Instant;

use reqwest::Client;

const WAIT: Duration = Duration::from_secs(30);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_binary_starts_serves_and_stops_on_sigterm() {
	let scratch = tempfile::tempdir().unwrap();
	let database = scratch.path().join("huddle.sqlite3");
	let site = scratch.path().join("dist");
	std::fs::create_dir_all(&site).unwrap();
	std::fs::write(site.join("index.html"), "huddle home").unwrap();
	let port = {
		let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
		listener.local_addr().unwrap().port()
	};
	let base = format!("http://127.0.0.1:{port}");

	let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_huddle"));
	command
		.env("DATABASE_URL", format!("sqlite://{}", database.display()))
		.env("LEPTOS_SITE_ADDR", format!("127.0.0.1:{port}"))
		.env("LEPTOS_SITE_ROOT", &site)
		.env("HUDDLE_ORIGIN", &base)
		.env("HUDDLE_SETTLEMENT_DELAY_MS", "1")
		.env("HUDDLE_SETTLEMENT_FAILURE_RATE", "0")
		.env_remove("AUTH_ENCRYPTION_KEYS")
		.env_remove("DATABASE_REPLICA_URL")
		.env_remove("NATS_URL")
		.stdout(std::process::Stdio::piped())
		.stderr(std::process::Stdio::piped());
	let mut child = command.spawn().expect("the huddle binary starts");
	let mut stdout = child.stdout.take().unwrap();
	let mut stderr = child.stderr.take().unwrap();
	let pid = child.id();
	let http = Client::new();

	// `/readyz` only passes once the migrations applied and the live hub
	// started, which is exactly the startup the binary promises.
	let deadline = Instant::now() + WAIT;
	loop {
		if let Ok(response) = http.get(format!("{base}/readyz")).send().await
			&& response.status().is_success()
		{
			break;
		}
		assert!(Instant::now() < deadline, "the server never became ready");
		tokio::time::sleep(Duration::from_millis(20)).await;
	}

	let mail = http
		.get(format!("{base}/_leptoskit/mail"))
		.send()
		.await
		.unwrap();
	assert_eq!(mail.status(), 200);
	assert!(mail.text().await.unwrap().contains("Huddle mail"));

	let whoami = http
		.post(format!("{base}/api/whoami"))
		.header("origin", &base)
		.header("content-type", "application/json")
		.body("{}")
		.send()
		.await
		.unwrap();
	assert_eq!(whoami.status(), 200);
	assert_eq!(whoami.text().await.unwrap(), "null");

	let home = http.get(format!("{base}/")).send().await.unwrap();
	assert_eq!(home.status(), 200);
	assert_eq!(home.text().await.unwrap(), "huddle home");

	// SIGTERM, as a platform sends when it moves traffic away: the process
	// drains and exits cleanly.
	let signalled = std::process::Command::new("kill")
		.arg("-TERM")
		.arg(pid.to_string())
		.status()
		.expect("kill runs");
	assert!(signalled.success());
	let status = tokio::time::timeout(WAIT, tokio::task::spawn_blocking(move || child.wait()))
		.await
		.expect("the process exits in time")
		.expect("waiting for the process")
		.expect("a process status");
	let mut out = String::new();
	stdout.read_to_string(&mut out).unwrap();
	let mut err = String::new();
	stderr.read_to_string(&mut err).unwrap();
	assert!(status.success(), "exit {status:?}: {err}");
	assert!(out.contains("roles:"), "{out}");
	assert!(out.contains("listening on http://"), "{out}");
	assert!(out.contains("applied migration"), "{out}");
}
