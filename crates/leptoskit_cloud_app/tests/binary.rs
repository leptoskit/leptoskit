//! `leptoskit-cloud-control`, run as a process: what it refuses, and what it
//! serves once its database is migrated.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;
use serde_json::json;
use tokio::io::AsyncBufReadExt as _;
use tokio::io::BufReader;
use tokio::process::Child;
use tokio::process::Command;

const BINARY: &str = env!("CARGO_BIN_EXE_leptoskit-cloud-control");
const KEYS: &str = "k1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
const SMTP_PASSWORD: &str = "hunter2";
const S3_SECRET: &str = "wJalrXUtnFEMI-hunter3";

/// The binary with only `vars` in its environment.
fn control(vars: &[(&str, &str)], args: &[&str]) -> Command {
	let mut command = Command::new(BINARY);
	command
		.env_clear()
		.envs(vars.iter().copied())
		.args(args)
		.kill_on_drop(true);
	command
}

/// A production environment over the SQLite file at `path`, listening on a
/// port of the system's choosing.
fn production(path: &Path) -> Vec<(&'static str, String)> {
	vec![
		(
			"LEPTOSKIT_CLOUD_DATABASE_URL",
			format!("sqlite://{}", path.display()),
		),
		(
			"LEPTOSKIT_CLOUD_ORIGIN",
			"https://cloud.example.com".to_owned(),
		),
		("LEPTOSKIT_CLOUD_AUTH_KEYS", KEYS.to_owned()),
		(
			"LEPTOSKIT_CLOUD_SMTP_URL",
			format!("smtp://cloud:{SMTP_PASSWORD}@127.0.0.1:2525"),
		),
		(
			"LEPTOSKIT_CLOUD_MAIL_FROM",
			"Cloud <cloud@example.com>".to_owned(),
		),
		("LEPTOSKIT_CLOUD_BIND", "127.0.0.1:0".to_owned()),
		("LEPTOSKIT_CLOUD_APPS_DOMAIN", "apps.example.com".to_owned()),
		(
			"LEPTOSKIT_CLOUD_REGISTRY",
			"registry.example.com/leptoskit".to_owned(),
		),
		// Never contacted: nothing in these tests uploads, and nothing expires.
		(
			"LEPTOSKIT_CLOUD_S3_ENDPOINT",
			"https://s3.example.com".to_owned(),
		),
		("LEPTOSKIT_CLOUD_S3_SOURCES_BUCKET", "sources".to_owned()),
		("LEPTOSKIT_CLOUD_S3_LOGS_BUCKET", "logs".to_owned()),
		("LEPTOSKIT_CLOUD_S3_ACCESS_KEY_ID", "AKIDEXAMPLE".to_owned()),
		("LEPTOSKIT_CLOUD_S3_SECRET_ACCESS_KEY", S3_SECRET.to_owned()),
	]
}

fn borrowed<'a>(vars: &'a [(&'static str, String)]) -> Vec<(&'static str, &'a str)> {
	vars.iter()
		.map(|(name, value)| (*name, value.as_str()))
		.collect()
}

/// Runs to completion: whether it succeeded, and its stdout and stderr.
async fn finish(mut command: Command) -> (bool, String, String) {
	let output = tokio::time::timeout(Duration::from_secs(120), command.output())
		.await
		.expect("finished in time")
		.expect("ran");
	(
		output.status.success(),
		String::from_utf8_lossy(&output.stdout).into_owned(),
		String::from_utf8_lossy(&output.stderr).into_owned(),
	)
}

/// A running server, and the origin it answers on.
struct Running {
	_child: Child,
	origin: String,
}

/// Starts `serve` and waits for it to say where it listens.
async fn start(mut command: Command) -> Running {
	command.stdout(Stdio::piped()).stderr(Stdio::inherit());
	let mut child = command.spawn().expect("started");
	let stdout = child.stdout.take().expect("stdout");
	let mut lines = BufReader::new(stdout).lines();
	let line = tokio::time::timeout(Duration::from_secs(120), lines.next_line())
		.await
		.expect("announced in time")
		.expect("read stdout")
		.expect("a line");
	let address = line
		.strip_prefix("listening on ")
		.and_then(|rest| rest.split(' ').next())
		.unwrap_or_else(|| panic!("an address in {line:?}"))
		.to_owned();
	tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });
	Running {
		_child: child,
		origin: format!("http://{address}"),
	}
}

#[tokio::test]
async fn development_serves_identity_protocol_and_readiness_routes() {
	let server = start(control(
		&[
			("LEPTOSKIT_CLOUD_DATABASE_URL", "sqlite::memory:"),
			("LEPTOSKIT_CLOUD_BIND", "127.0.0.1:0"),
		],
		&["serve", "--dev"],
	))
	.await;
	let client = reqwest::Client::builder()
		.no_proxy()
		.build()
		.expect("client");
	for path in [
		"/.well-known/openid-configuration",
		"/.well-known/leptoskit-cloud",
		"/readyz",
		"/healthz",
	] {
		let response = client
			.get(format!("{}{path}", server.origin))
			.send()
			.await
			.expect("route responds");
		assert_eq!(response.status().as_u16(), 200, "{path}");
	}
	let discovery: Value = client
		.get(format!("{}/.well-known/leptoskit-cloud", server.origin))
		.send()
		.await
		.expect("cloud discovery")
		.json()
		.await
		.expect("JSON");
	let device = client
		.post(format!("{}/oauth/device_authorization", server.origin))
		.form(&[
			(
				"client_id",
				discovery["cli_client_id"].as_str().expect("client ID"),
			),
			("scope", discovery["scope"].as_str().expect("scopes")),
		])
		.send()
		.await
		.expect("device authorization");
	assert_eq!(device.status().as_u16(), 200);
	// `--dev` keeps objects in a local store the control plane serves: the
	// route is reached, and refuses an address it didn't sign.
	let forged = client
		.get(format!(
			"{}/_leptoskit/storage/sources/forged/archive?expires=0&sig=00",
			server.origin
		))
		.send()
		.await
		.expect("local storage responds");
	assert_eq!(forged.status().as_u16(), 403);
	let body = forged.text().await.expect("body");
	assert!(body.starts_with("signature_mismatch"), "{body}");
}

#[tokio::test]
async fn development_listens_on_this_machine_only() {
	let (ok, _, stderr) = finish(control(
		&[
			("LEPTOSKIT_CLOUD_DATABASE_URL", "sqlite::memory:"),
			("LEPTOSKIT_CLOUD_BIND", "0.0.0.0:8080"),
		],
		&["serve", "--dev"],
	))
	.await;
	assert!(!ok);
	assert!(stderr.contains("loopback"), "{stderr}");
}

#[tokio::test]
async fn serving_without_keys_is_refused_without_repeating_secrets() {
	let directory = tempfile::tempdir().expect("a directory");
	let vars: Vec<_> = production(&directory.path().join("cloud.db"))
		.into_iter()
		.filter(|(name, _)| *name != "LEPTOSKIT_CLOUD_AUTH_KEYS")
		.collect();
	let (ok, stdout, stderr) = finish(control(&borrowed(&vars), &["serve"])).await;
	assert!(!ok);
	assert!(stderr.contains("set LEPTOSKIT_CLOUD_AUTH_KEYS"), "{stderr}");
	assert!(!stderr.contains(SMTP_PASSWORD) && !stdout.contains(SMTP_PASSWORD));
	assert!(!stderr.contains(S3_SECRET) && !stdout.contains(S3_SECRET));

	let mut vars = vars;
	vars.push((
		"LEPTOSKIT_CLOUD_AUTH_KEYS",
		"k1:secretish-but-wrong".to_owned(),
	));
	let (ok, _, stderr) = finish(control(&borrowed(&vars), &["serve"])).await;
	assert!(!ok);
	assert!(
		stderr.contains("LEPTOSKIT_CLOUD_AUTH_KEYS is invalid"),
		"{stderr}"
	);
	assert!(!stderr.contains("secretish"), "{stderr}");
}

#[tokio::test]
async fn a_migrated_database_serves_and_is_ready() {
	let directory = tempfile::tempdir().expect("a directory");
	let vars = production(&directory.path().join("cloud.db"));
	let vars = borrowed(&vars);

	let (ok, _, stderr) = finish(control(&vars, &["serve"])).await;
	assert!(!ok, "a database that hasn't migrated isn't served");
	assert!(stderr.contains("pending migration"), "{stderr}");

	let (ok, stdout, stderr) = finish(control(&vars, &["migrate"])).await;
	assert!(ok, "{stderr}");
	assert!(stdout.contains("applied"), "{stdout}");
	let (ok, stdout, _) = finish(control(&vars, &["migrate"])).await;
	assert!(ok);
	assert!(stdout.contains("up to date"), "{stdout}");

	let server = start(control(&vars, &["serve"])).await;
	let client = reqwest::Client::new();
	let ready = client
		.get(format!("{}/readyz", server.origin))
		.send()
		.await
		.expect("/readyz");
	assert_eq!(ready.status().as_u16(), 200);
	let healthy = client
		.get(format!("{}/healthz", server.origin))
		.send()
		.await
		.expect("/healthz");
	assert_eq!(healthy.status().as_u16(), 200);
	let discovery: Value = client
		.get(format!("{}/.well-known/leptoskit-cloud", server.origin))
		.send()
		.await
		.expect("discovery")
		.json()
		.await
		.expect("JSON");
	assert_eq!(discovery["rpc"], "/rpc");
	let whoami = client
		.post(format!("{}/rpc/whoami", server.origin))
		.json(&json!({}))
		.send()
		.await
		.expect("whoami");
	assert_eq!(
		whoami.status().as_u16(),
		401,
		"anonymous callers reach nothing"
	);
}

/// The OAuth provider serves discovery, tokens and the device flow from its
/// layer, not from routes, so they answer only if every route and fallback
/// the binary serves sits inside that layer.
#[tokio::test]
async fn the_oauth_provider_answers_beside_the_probes() {
	let vars = [
		("LEPTOSKIT_CLOUD_DATABASE_URL", "sqlite::memory:"),
		("LEPTOSKIT_CLOUD_BIND", "127.0.0.1:0"),
	];
	let server = start(control(&vars, &["serve", "--dev"])).await;
	let client = reqwest::Client::new();
	let get = async |path: &str| {
		client
			.get(format!("{}{path}", server.origin))
			.send()
			.await
			.unwrap_or_else(|error| panic!("GET {path}: {error}"))
	};

	for path in ["/readyz", "/healthz"] {
		assert_eq!(get(path).await.status().as_u16(), 200, "{path}");
	}

	let openid = get("/.well-known/openid-configuration").await;
	assert_eq!(openid.status().as_u16(), 200, "OpenID discovery");
	let openid: Value = openid.json().await.expect("JSON");
	assert!(
		openid["device_authorization_endpoint"]
			.as_str()
			.is_some_and(|endpoint| endpoint.ends_with("/oauth/device_authorization")),
		"{openid}"
	);

	let cloud = get("/.well-known/leptoskit-cloud").await;
	assert_eq!(cloud.status().as_u16(), 200, "the CLI's discovery");
	let cloud: Value = cloud.json().await.expect("JSON");
	let client_id = cloud["cli_client_id"].as_str().expect("the CLI's client");
	let scope = cloud["scope"].as_str().expect("the CLI's scopes");

	let started = client
		.post(format!("{}/oauth/device_authorization", server.origin))
		.form(&[("client_id", client_id), ("scope", scope)])
		.send()
		.await
		.expect("device authorization");
	assert_eq!(started.status().as_u16(), 200, "the device flow");
	let started: Value = started.json().await.expect("JSON");
	assert!(started["user_code"].is_string(), "{started}");
	assert!(started["device_code"].is_string(), "{started}");

	// With the console built in, its pages answer beside them, and an
	// address nothing claims is the console's not-found page.
	#[cfg(feature = "ssr")]
	{
		assert_eq!(get("/sign-in").await.status().as_u16(), 200, "/sign-in");
		let missing = get("/no-such-organisation/or-page").await;
		assert_eq!(missing.status().as_u16(), 404);
		let page = missing.text().await.expect("text");
		assert!(page.contains("<!DOCTYPE html>"), "{page}");
	}
}

#[tokio::test]
async fn operators_are_granted_on_the_command_line() {
	let directory = tempfile::tempdir().expect("a directory");
	let database = format!("sqlite://{}", directory.path().join("cloud.db").display());
	let vars = [
		("LEPTOSKIT_CLOUD_DATABASE_URL", database.as_str()),
		("LEPTOSKIT_CLOUD_BIND", "127.0.0.1:0"),
	];
	let server = start(control(&vars, &["serve", "--dev"])).await;
	let client = reqwest::Client::new();
	let signed_in = client
		.post(format!("{}/auth/dev/sign-in", server.origin))
		.header("origin", &server.origin)
		.json(&json!({ "email": "ana@example.com" }))
		.send()
		.await
		.expect("dev sign-in");
	assert!(signed_in.status().is_success(), "{}", signed_in.status());
	let cookie = signed_in
		.headers()
		.get("set-cookie")
		.expect("a session")
		.to_str()
		.expect("text")
		.split(';')
		.next()
		.expect("a cookie")
		.to_owned();
	let whoami = async || -> Value {
		client
			.post(format!("{}/rpc/whoami", server.origin))
			.header("origin", &server.origin)
			.header("cookie", &cookie)
			.json(&json!({}))
			.send()
			.await
			.expect("whoami")
			.json()
			.await
			.expect("JSON")
	};
	assert_eq!(whoami().await["operator"], false);

	let (ok, _, stderr) =
		finish(control(&vars, &["operator", "grant", "nobody@example.com"])).await;
	assert!(!ok);
	assert!(stderr.contains("sign in with it first"), "{stderr}");

	let (ok, stdout, stderr) =
		finish(control(&vars, &["operator", "grant", "ana@example.com"])).await;
	assert!(ok, "{stderr}");
	assert!(stdout.contains("is an operator"), "{stdout}");
	assert_eq!(whoami().await["operator"], true);
	let (ok, stdout, _) = finish(control(&vars, &["operator", "list"])).await;
	assert!(ok);
	assert_eq!(stdout.trim(), "ana@example.com");

	let (ok, _, stderr) = finish(control(&vars, &["operator", "revoke", "ana@example.com"])).await;
	assert!(ok, "{stderr}");
	assert_eq!(whoami().await["operator"], false);
}
