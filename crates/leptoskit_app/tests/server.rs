//! The operator binary: every subcommand works offline against a temporary
//! data directory, and a missing key or billing configuration is refused
//! before any state is touched.
#![cfg(all(feature = "server", feature = "site"))]
use std::io::Read as _;
use std::io::Write as _;
use std::net::TcpListener;
use std::net::TcpStream;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::body::to_bytes;
use axum::http::Request;
use axum::http::StatusCode;
use leptoskit_deploy_console::hosted::Billing;
use leptoskit_deploy_console::hosted::Hosted;
use tower::ServiceExt as _;

/// The encryption key the broker opens its data with.
const KEY: &str = "test-only-hosted-encryption-secret-0123456789";

/// The broker executable, built for this test, with no Stripe configuration
/// unless a test adds it.
fn broker() -> Command {
	let mut command = Command::new(env!("CARGO_BIN_EXE_leptoskit-cloud"));
	for name in [
		"STRIPE_SECRET_KEY",
		"STRIPE_WEBHOOK_SECRET",
		"STRIPE_CONTROL_PRICE",
		"STRIPE_TEAM_PRICE",
	] {
		command.env_remove(name);
	}
	command
}

/// The broker pointing at one data directory with its key set.
fn operator(data: &Path) -> Command {
	let mut command = broker();
	command
		.env("LEPTOSKIT_HOSTED_KEY", KEY)
		.arg("--data")
		.arg(data);
	command
}

fn checked(output: &Output) -> &Output {
	assert!(output.status.success(), "{}", stderr(output));
	output
}

fn stderr(output: &Output) -> String {
	String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout(output: &Output) -> String {
	String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Enrolls `alice` with an existing Stripe customer and returns the account
/// key the rotate command printed.
fn enrol(data: &Path) -> String {
	let created = operator(data)
		.args(["account", "--id", "alice", "--customer", "cus_alice"])
		.output()
		.unwrap();
	assert_ne!(stdout(checked(&created)).trim(), "");
	let rotated = operator(data)
		.args(["rotate-key", "--account", "alice"])
		.output()
		.unwrap();
	let account_key = stdout(checked(&rotated)).trim().to_owned();
	assert_ne!(account_key, "");
	account_key
}

/// Applies the paid subscription managed capacity needs, the way the
/// operator's Stripe webhook would.
fn activate(data: &Path) {
	let host = Hosted::open(data, KEY.into(), "http://localhost:4701", None).unwrap();
	let billing = Billing::new(
		"test".into(),
		"test-hook".into(),
		"price_control".into(),
		"price_team".into(),
	)
	.unwrap();
	billing
		.apply_subscription(
			&host,
			"alice",
			&serde_json::json!({
				"id": "sub_alice",
				"customer": "cus_alice",
				"status": "active",
				"items": {"data": [{
					"quantity": 1,
					"price": {"id": "price_team"},
					"current_period_end": std::time::SystemTime::now()
						.duration_since(std::time::UNIX_EPOCH)
						.unwrap()
						.as_secs() + 3600,
				}]},
			}),
		)
		.unwrap();
}

/// Registers the managed installation and checks its pairing code printed.
fn managed(data: &Path) {
	let output = operator(data)
		.args(["managed", "--account", "alice", "--project", "notes"])
		.output()
		.unwrap();
	assert_ne!(stdout(checked(&output)).trim(), "");
}

/// A data directory with an enrolled, subscribed account and a managed
/// installation, as the hosted pilot has.
fn prepared(directory: &Path) -> (PathBuf, String) {
	let data = directory.join("cloud");
	let account_key = enrol(&data);
	activate(&data);
	managed(&data);
	(data, account_key)
}

/// One request against the hosted API.
async fn call(
	app: &Router,
	method: &str,
	path: &str,
	key: Option<&str>,
	body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
	let mut request = Request::builder()
		.method(method)
		.uri(path)
		.header("content-type", "application/json");
	if let Some(key) = key {
		request = request.header("authorization", format!("Bearer {key}"));
	}
	let request = request
		.body(body.map_or_else(Body::empty, |value| {
			Body::from(serde_json::to_vec(&value).unwrap())
		}))
		.unwrap();
	let response = app.clone().oneshot(request).await.unwrap();
	let status = response.status();
	let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
	(
		status,
		serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
	)
}

#[test]
fn the_broker_without_its_key_says_how_to_set_one() {
	let output = broker()
		.env_remove("LEPTOSKIT_HOSTED_KEY")
		.args(["backup", "--out", "unused"])
		.output()
		.unwrap();
	assert!(!output.status.success());
	assert!(
		stderr(&output).contains("LEPTOSKIT_HOSTED_KEY"),
		"{}",
		stderr(&output)
	);
}

#[test]
fn the_operator_subcommands_manage_one_data_directory() {
	let directory = tempfile::tempdir().unwrap();
	let (data, _account_key) = prepared(directory.path());
	let backup = directory.path().join("backup");
	let rollback = directory.path().join("rollback");

	// Backup copies every journal; the schema rollback needs its backup and
	// an explicit --yes.
	let output = operator(&data)
		.args(["backup", "--out", backup.to_str().unwrap()])
		.output()
		.unwrap();
	checked(&output);
	assert!(backup.exists());
	let output = operator(&data)
		.args(["rollback-schema", "--backup", rollback.to_str().unwrap()])
		.output()
		.unwrap();
	assert!(!output.status.success());
	assert!(stderr(&output).contains("--yes"), "{}", stderr(&output));
	let output = operator(&data)
		.args([
			"rollback-schema",
			"--backup",
			rollback.to_str().unwrap(),
			"--yes",
		])
		.output()
		.unwrap();
	checked(&output);

	// Serving needs Stripe's keys and both recurring prices first.
	let output = operator(&data)
		.args(["serve", "--assets", data.to_str().unwrap()])
		.output()
		.unwrap();
	assert!(!output.status.success());
	assert!(
		stderr(&output).contains("Stripe keys"),
		"{}",
		stderr(&output)
	);
}

#[tokio::test]
async fn a_running_operation_is_reconciled_once_its_worker_is_gone() {
	let directory = tempfile::tempdir().unwrap();
	let (data, account_key) = prepared(directory.path());
	let host = Hosted::open(&data, KEY.into(), "http://localhost:4701", None).unwrap();
	let app = host.router();

	// Queue a plan against the managed installation.
	let (status, overview) = call(&app, "GET", "/api/v1/overview", Some(&account_key), None).await;
	assert_eq!(status, StatusCode::OK, "{overview}");
	let revision = overview["projects"]
		.as_array()
		.unwrap()
		.iter()
		.find(|project| project["id"] == "notes")
		.unwrap()["revision"]
		.as_u64()
		.unwrap();
	let (status, operation) = call(
		&app,
		"POST",
		"/api/v1/projects/notes/operations",
		Some(&account_key),
		Some(serde_json::json!({
			"expected_revision": revision,
			"idempotency_key": "operator-test-1",
			"action": {"kind": "plan"},
		})),
	)
	.await;
	assert_eq!(status, StatusCode::OK, "{operation}");
	let operation_id = operation["id"].as_str().unwrap().to_owned();

	// A worker pairs and claims it; reconciling is only allowed after that
	// worker is revoked.
	let code = host.accounts().pairing("alice", "notes").unwrap();
	let (status, paired) = call(
		&app,
		"POST",
		"/api/v1/worker/pair",
		None,
		Some(serde_json::json!({"code": code})),
	)
	.await;
	assert_eq!(status, StatusCode::OK, "{paired}");
	let worker = paired["key"].as_str().unwrap().to_owned();
	let (status, claimed) = call(
		&app,
		"POST",
		"/api/v1/worker/claim",
		Some(&worker),
		Some(serde_json::json!({})),
	)
	.await;
	assert_eq!(status, StatusCode::OK, "{claimed}");
	assert_eq!(claimed["operation"]["phase"], "running");
	host.accounts().revoke_worker("alice", "notes").unwrap();
	drop(host);

	let output = operator(&data)
		.args([
			"interrupt",
			"--account",
			"alice",
			"--operation",
			&operation_id,
		])
		.output()
		.unwrap();
	assert!(
		stdout(checked(&output)).contains("Marked interrupted"),
		"{}",
		stdout(&output)
	);

	// An operation that does not exist is refused rather than pretending
	// it was interrupted.
	let output = operator(&data)
		.args([
			"interrupt",
			"--account",
			"alice",
			"--operation",
			"00000000-0000-4000-8000-000000000099",
		])
		.output()
		.unwrap();
	assert!(!output.status.success());
	assert!(!stderr(&output).contains("panicked"), "{}", stderr(&output));
}

/// The website and documentation the broker serves, each file present with
/// the same names `leptoskit build` and the docs build write.
fn website(assets: &Path, docs: &Path) {
	for (path, bytes) in [
		("index.html", "<!doctype html><title>console</title>"),
		(
			"console/index.html",
			"<!doctype html><title>console</title>",
		),
		("bootstrap.js", "fixture bootstrap"),
		("pkg/leptoskit_app.js", "fixture module"),
		("pkg/leptoskit_app_bg.wasm", "fixture wasm"),
		(
			"pkg/__wasm_split.______________________.js",
			"fixture loader",
		),
		(
			"pkg/__wasm_split_manifest.json",
			r#"{"mount_console_1":["split_mount_console_1"]}"#,
		),
		("pkg/split_mount_console_1.wasm", "fixture chunk"),
		("landing.css", "fixture styles"),
		("landing.mjs", "fixture script"),
		("play/index.html", "fixture playground"),
		("play/playground.mjs", "fixture playground module"),
		("play/frame.html", "fixture player frame"),
		("play/frame.mjs", "fixture player"),
		("playground.css", "fixture playground styles"),
		("console.css", "fixture console styles"),
		("rive/rive.js", "fixture runtime"),
		("rive/rive.wasm", "fixture runtime wasm"),
		("rive/brand.riv", "fixture scene"),
	] {
		let path = assets.join(path);
		std::fs::create_dir_all(path.parent().unwrap()).unwrap();
		std::fs::write(path, bytes).unwrap();
	}
	for page in [
		"index.html",
		"guide/01-installation.html",
		"guide/29-hosted-deployments.html",
		"schema.json",
		"_leptoskit/theme.js",
		"_leptoskit/site.js",
		"_leptoskit/site.css",
		"_leptoskit/search.json",
		"theme/skribble.css",
		"assets/fonts/skribble/SkribbleNotoSketch-Regular.woff2",
		"assets/fonts/skribble/SkribbleMonoPlayful-Regular.woff2",
		"assets/fonts/skribble/SkribbleMonoPlayful-Bold.woff2",
		"_leptoskit/generated/card-1.svg",
		"_leptoskit/generated/brand-mint.svg",
	] {
		let path = docs.join(page);
		std::fs::create_dir_all(path.parent().unwrap()).unwrap();
		std::fs::write(path, "fixture page").unwrap();
	}
}

/// Reads one HTTP response from the serving broker.
fn get(address: std::net::SocketAddr) -> String {
	let mut response = String::new();
	for attempt in 0..100 {
		match TcpStream::connect_timeout(&address, Duration::from_millis(100)) {
			Ok(mut stream) => {
				stream
					.set_read_timeout(Some(Duration::from_secs(5)))
					.unwrap();
				stream
					.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
					.unwrap();
				stream.read_to_string(&mut response).unwrap();
				break;
			}
			Err(_) if attempt < 99 => std::thread::sleep(Duration::from_millis(100)),
			Err(error) => panic!("the broker never accepted: {error}"),
		}
	}
	response
}

/// Stops the broker through its graceful-shutdown signal, and checks it
/// exited cleanly.
fn stop(mut child: std::process::Child) {
	let killed = Command::new("kill")
		.args(["-TERM", &child.id().to_string()])
		.status()
		.unwrap();
	assert!(killed.success());
	for _ in 0..100 {
		if let Some(status) = child.try_wait().unwrap() {
			assert!(status.success(), "{status}");
			return;
		}
		std::thread::sleep(Duration::from_millis(100));
	}
	child.kill().unwrap();
	panic!("the broker ignored SIGTERM");
}

#[test]
fn the_broker_serves_the_site_until_it_is_asked_to_stop() {
	let directory = tempfile::tempdir().unwrap();
	let data = directory.path().join("cloud");
	let assets = directory.path().join("assets");
	let docs = directory.path().join("docs");
	website(&assets, &docs);

	// A free port, closed again for the broker to take.
	let listener = TcpListener::bind("127.0.0.1:0").unwrap();
	let address = listener.local_addr().unwrap();
	drop(listener);

	let child = broker()
		.env("LEPTOSKIT_HOSTED_KEY", KEY)
		.env("STRIPE_SECRET_KEY", "sk_test_offline")
		.env("STRIPE_WEBHOOK_SECRET", "whsec_offline")
		.env("STRIPE_CONTROL_PRICE", "price_control")
		.env("STRIPE_TEAM_PRICE", "price_team")
		.args([
			"--data",
			data.to_str().unwrap(),
			"serve",
			"--assets",
			assets.to_str().unwrap(),
			"--docs",
			docs.to_str().unwrap(),
			"--bind",
			&address.to_string(),
		])
		.stdout(std::process::Stdio::null())
		.stderr(std::process::Stdio::piped())
		.spawn()
		.unwrap();
	let response = get(address);
	assert!(response.contains("200 OK"), "{response}");
	assert!(response.contains("LeptosKit"), "{response}");
	stop(child);

	// A backup restores into a new data directory with the same key.
	let backup = directory.path().join("restore-source");
	let output = operator(&data)
		.args(["backup", "--out", backup.to_str().unwrap()])
		.output()
		.unwrap();
	checked(&output);
	let restored = directory.path().join("restored");
	let output = operator(&restored)
		.args(["restore", "--from", backup.to_str().unwrap()])
		.output()
		.unwrap();
	checked(&output);
	assert!(restored.join("accounts.sqlite3").is_file());
}

#[test]
fn invalid_billing_configuration_is_refused_before_serving() {
	let directory = tempfile::tempdir().unwrap();
	let data = directory.path().join("cloud");
	let output = broker()
		.env("LEPTOSKIT_HOSTED_KEY", KEY)
		.env("STRIPE_SECRET_KEY", "sk_test_offline")
		.env("STRIPE_WEBHOOK_SECRET", "whsec_offline")
		.env("STRIPE_CONTROL_PRICE", "control")
		.env("STRIPE_TEAM_PRICE", "price_team")
		.args([
			"--data",
			data.to_str().unwrap(),
			"serve",
			"--assets",
			directory.path().to_str().unwrap(),
		])
		.output()
		.unwrap();
	assert!(!output.status.success());
	assert!(
		stderr(&output).contains("invalid billing configuration"),
		"{}",
		stderr(&output)
	);
}
