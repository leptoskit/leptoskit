use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::sync::Arc;
use std::sync::Mutex;

use leptoskit_deploy_console::control::Action;
use leptoskit_deploy_console::control::Executor as _;
use leptoskit_deploy_console::control::Intent;
use leptoskit_deploy_console::control::Operation;
use leptoskit_deploy_console::control::Phase;
use serde_json::Value;
use serde_json::json;

use super::*;

pub(super) const CONFIG: &str = r#"
[app]
name = "notes"
render = "ssr"
[db]
dialect = "postgres"
[infra]
name = "notes-production"
domain = "app.example.com"
location = "hel1"
server_type = "cx23"
ssh_key_ids = [123]
ssh_cidrs = ["192.0.2.1/32"]
account_id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
zone_id = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
updates_domain = "updates.example.com"
updates_bucket = "notes-releases"
backup_bucket = "notes-backups"
image = "ghcr.io/team/notes"
registry = "ghcr.io"
registry_username = "team"
"#;

const OWNER: &str = "test-service-owner-key-01234567890123456789";

/// Serves `router` on a loopback port from its own runtime, for the
/// blocking clients under test.
pub(super) fn serve(router: axum::Router) -> String {
	let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
	listener.set_nonblocking(true).unwrap();
	let url = format!("http://{}", listener.local_addr().unwrap());
	std::thread::spawn(move || {
		tokio::runtime::Builder::new_current_thread()
			.enable_all()
			.build()
			.unwrap()
			.block_on(async move {
				let listener = tokio::net::TcpListener::from_std(listener).unwrap();
				axum::serve(listener, router).await.unwrap();
			});
	});
	url
}

/// A registered app, the service's data directory, and a stand-in for the
/// `lk` binary that records each invocation.
struct Fixture {
	_directory: tempfile::TempDir,
	root: PathBuf,
	data: PathBuf,
	service: Service,
	project: Project,
}

impl Fixture {
	/// `rule` is shell run after the engine records its call, so a test can
	/// make chosen invocations fail.
	fn new(config: &str, rule: &str) -> Self {
		let directory = tempfile::tempdir().unwrap();
		let base = directory.path().canonicalize().unwrap();
		let root = base.join("app");
		let data = base.join("service");
		fs::create_dir_all(root.join("bin")).unwrap();
		fs::write(root.join("leptoskit.toml"), config).unwrap();
		let calls = base.join("calls");
		let engine = root.join("bin/lk");
		fs::write(
			&engine,
			format!(
				r#"#!/bin/sh
{{
  printf 'args=%s\n' "$*"
  printf 'cwd=%s\n' "$(pwd -P)"
  printf 'shared=%s\n' "$(readlink infra/notes-production 2>/dev/null)"
  printf 'origin=%s\n' "${{LEPTOSKIT_CONTROL_ORIGIN-unset}}"
  printf 'local=%s\n' "${{LEPTOSKIT_CONTROL_LOCAL-unset}}"
  printf 'control_key=%s\n' "${{LEPTOSKIT_CONTROL_KEY-unset}}"
  printf 'state=%s %s %s %s %s\n' "${{TF_HTTP_ADDRESS-unset}}" "${{TF_HTTP_LOCK_ADDRESS-unset}}" "${{TF_HTTP_USERNAME-unset}}" "${{TF_HTTP_PASSWORD-unset}}" "${{TF_HTTP_LOCK_METHOD-unset}}"
  printf 'read=%s\n' "${{LEPTOSKIT_ANALYTICS_READ_KEY-unset}}"
  printf 'write=%s\n' "${{LEPTOSKIT_ANALYTICS_WRITE_KEY-unset}}"
  printf '\n'
}} >> '{calls}'
echo "engine output for $*"
echo "engine diagnostics: private-tool-detail" >&2
{rule}
exit 0
"#,
				calls = calls.display()
			),
		)
		.unwrap();
		fs::set_permissions(&engine, fs::Permissions::from_mode(0o700)).unwrap();
		let store = Store::open(&data.join("console.sqlite3")).unwrap();
		let project = Project {
			id: "notes-production".into(),
			environment: "production".into(),
			root: root.clone(),
			revision: 0,
		};
		store.register(&project).unwrap();
		let service = Service::new(store, OWNER, "http://localhost:4700").unwrap();
		Self {
			_directory: directory,
			root,
			data,
			service,
			project,
		}
	}

	fn runner(&self) -> runner::CommandRunner {
		runner::CommandRunner::new(
			self.data.clone(),
			self.root.join("bin/lk"),
			self.service.clone(),
		)
	}

	/// Each recorded engine invocation, as its `name=value` lines.
	fn calls(&self) -> Vec<BTreeMap<String, String>> {
		let Ok(text) = fs::read_to_string(self.root.parent().unwrap().join("calls")) else {
			return Vec::new();
		};
		text.split("\n\n")
			.filter(|block| !block.trim().is_empty())
			.map(|block| {
				block
					.lines()
					.filter_map(|line| line.split_once('='))
					.map(|(name, value)| (name.to_owned(), value.to_owned()))
					.collect()
			})
			.collect()
	}

	fn log(&self, name: &str) -> String {
		fs::read_to_string(self.data.join("logs").join(name)).unwrap()
	}
}

fn operation(id: &str, action: Action) -> Operation {
	Operation {
		events: Vec::new(),
		id: id.into(),
		project: "notes-production".into(),
		revision: 1,
		intent: Intent {
			expected_revision: 0,
			idempotency_key: format!("key-{id}"),
			action,
		},
		phase: Phase::Running,
		created: 0,
		updated: 0,
		message: String::new(),
		result: Value::Null,
	}
}

fn deploy(version: &str) -> Action {
	Action::Deploy {
		version: version.into(),
		setup: false,
		maintenance: false,
	}
}

fn git(root: &Path, args: &[&str]) -> String {
	let output = std::process::Command::new("git")
		.args([
			"-c",
			"user.name=Test",
			"-c",
			"user.email=test@example.com",
			"-c",
			"commit.gpgsign=false",
			"-c",
			"core.hooksPath=/dev/null",
		])
		.args(args)
		.current_dir(root)
		.output()
		.unwrap();
	assert!(
		output.status.success(),
		"git {args:?}: {}",
		String::from_utf8_lossy(&output.stderr)
	);
	String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

/// Commits the app's configuration and returns the commit.
fn commit(root: &Path) -> String {
	git(root, &["init", "-q"]);
	git(root, &["add", "leptoskit.toml"]);
	git(root, &["commit", "-q", "-m", "fixture"]);
	git(root, &["rev-parse", "HEAD"])
}

#[tokio::test]
async fn a_deploy_runs_the_release_engine_with_its_flags_and_keeps_a_private_log() {
	let fixture = Fixture::new(CONFIG, "");
	let result = fixture
		.runner()
		.run(
			fixture.project.clone(),
			operation(
				"deploy-one",
				Action::Deploy {
					version: "v2".into(),
					setup: true,
					maintenance: true,
				},
			),
		)
		.await;
	assert!(result.succeeded, "{}", result.message);
	assert_eq!(result.details, json!({"code": "completed"}));
	let calls = fixture.calls();
	assert_eq!(calls.len(), 1);
	assert_eq!(calls[0]["args"], "deploy --version v2 --setup --maintenance");
	assert_eq!(calls[0]["cwd"], fixture.root.display().to_string());
	assert_eq!(calls[0]["origin"], "http://localhost:4700");
	assert_eq!(calls[0]["local"], "1");
	assert_eq!(calls[0]["control_key"], "unset");
	assert_eq!(calls[0]["state"], "unset unset unset unset unset");
	assert_eq!(calls[0]["read"], "unset");
	let output = fixture.data.join("logs/deploy-one.run.out");
	assert_eq!(
		fs::metadata(&output).unwrap().permissions().mode() & 0o777,
		0o600
	);
	assert!(fixture.log("deploy-one.run.out").contains("engine output for deploy"));
	assert!(fixture.log("deploy-one.run.err").contains("private-tool-detail"));
	assert!(!fixture.data.join("workspaces/deploy-one").exists());
}

#[tokio::test]
async fn each_action_runs_one_fixed_engine_command() {
	let fixture = Fixture::new(CONFIG, "");
	let runner = fixture.runner();
	for (id, action) in [
		(
			"rollback",
			Action::Rollback {
				version: "v1".into(),
			},
		),
		("status", Action::Status),
		("backup", Action::Backup),
		(
			"apply",
			Action::Apply {
				plan_id: "reviewed-plan".into(),
			},
		),
	] {
		let result = runner
			.run(fixture.project.clone(), operation(id, action))
			.await;
		assert!(result.succeeded, "{id}: {}", result.message);
	}
	let arguments = fixture
		.calls()
		.into_iter()
		.map(|call| call["args"].clone())
		.collect::<Vec<_>>();
	assert_eq!(
		arguments,
		[
			"deploy --rollback v1",
			"infra status",
			"infra backups verify",
			"infra apply"
		]
	);
}

#[tokio::test]
async fn a_failing_command_is_reported_without_the_tool_output() {
	let fixture = Fixture::new(CONFIG, "exit 3");
	let result = fixture
		.runner()
		.run(fixture.project.clone(), operation("plan-one", Action::Plan))
		.await;
	assert!(!result.succeeded);
	assert_eq!(result.details, json!({"code": "command_failed"}));
	assert!(result.message.contains("private log"));
	assert!(!result.message.contains("private-tool-detail"));
	assert_eq!(fixture.calls()[0]["args"], "infra plan");
	assert!(fixture.log("plan-one.run.err").contains("private-tool-detail"));
}

#[tokio::test]
async fn hosted_state_and_analytics_bindings_reach_the_engine() {
	let fixture = Fixture::new(&format!("{CONFIG}\n[infra.plugins.analytics]\n"), "");
	let mut runner = fixture.runner();
	runner.hosted_state = Some((
		"https://hosted.example.com/api/v1/state/notes-production".into(),
		"hosted-installation-key".into(),
	));
	runner.hosted_bindings = Some(("hosted-read-key".into(), "hosted-ingest-key".into()));
	let result = runner
		.run(fixture.project.clone(), operation("status", Action::Status))
		.await;
	assert!(result.succeeded, "{}", result.message);
	let call = &fixture.calls()[0];
	assert_eq!(
		call["state"],
		"https://hosted.example.com/api/v1/state/notes-production \
		 https://hosted.example.com/api/v1/state/notes-production worker hosted-installation-key \
		 LOCK"
	);
	assert_eq!(call["read"], "hosted-read-key");
	assert_eq!(call["write"], "hosted-ingest-key");
}

#[tokio::test]
async fn without_hosted_bindings_the_service_derives_the_app_keys() {
	let fixture = Fixture::new(&format!("{CONFIG}\n[infra.plugins.analytics]\n"), "");
	let result = fixture
		.runner()
		.run(fixture.project.clone(), operation("status", Action::Status))
		.await;
	assert!(result.succeeded, "{}", result.message);
	let call = &fixture.calls()[0];
	assert_eq!(
		call["read"],
		fixture
			.service
			.managed_app_key("notes-production", false)
			.unwrap()
	);
	assert_eq!(
		call["write"],
		fixture
			.service
			.managed_app_key("notes-production", true)
			.unwrap()
	);
	assert_ne!(call["read"], call["write"]);
}

#[tokio::test]
async fn an_external_metrics_endpoint_keeps_the_ingest_key_from_the_engine() {
	let fixture = Fixture::new(
		&format!(
			"{CONFIG}\n[infra.plugins.analytics]\notlp_endpoint = \
			 \"https://otel.example.com/v1/metrics\"\n"
		),
		"",
	);
	let mut runner = fixture.runner();
	runner.hosted_bindings = Some(("hosted-read-key".into(), "hosted-ingest-key".into()));
	let result = runner
		.run(fixture.project.clone(), operation("status", Action::Status))
		.await;
	assert!(result.succeeded, "{}", result.message);
	let call = &fixture.calls()[0];
	assert_eq!(call["read"], "hosted-read-key");
	assert_eq!(call["write"], "unset");
}

#[tokio::test]
async fn engine_output_beyond_four_mebibytes_is_truncated_in_the_log() {
	let fixture = Fixture::new(CONFIG, "head -c 5000000 /dev/zero");
	let result = fixture
		.runner()
		.run(fixture.project.clone(), operation("noisy", Action::Status))
		.await;
	assert!(result.succeeded, "{}", result.message);
	let log = fs::read(fixture.data.join("logs/noisy.run.out")).unwrap();
	let marker = b"\n[output truncated at 4 MiB]\n";
	assert_eq!(log.len(), 4 * 1024 * 1024 + marker.len());
	assert!(log.ends_with(marker));
}

#[tokio::test]
async fn only_the_newest_logs_are_kept() {
	let fixture = Fixture::new(CONFIG, "");
	let logs = fixture.data.join("logs");
	fs::create_dir_all(&logs).unwrap();
	let start = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
	for index in 0..70_u64 {
		let path = logs.join(format!("old-{index:02}.run.out"));
		fs::write(&path, "old").unwrap();
		fs::File::options()
			.write(true)
			.open(&path)
			.unwrap()
			.set_modified(start + std::time::Duration::from_secs(index))
			.unwrap();
	}
	fs::write(logs.join("notes.txt"), "kept").unwrap();
	let result = fixture
		.runner()
		.run(fixture.project.clone(), operation("latest", Action::Status))
		.await;
	assert!(result.succeeded, "{}", result.message);
	for index in 0..6 {
		assert!(!logs.join(format!("old-{index:02}.run.out")).exists());
	}
	for index in 6..70 {
		assert!(logs.join(format!("old-{index:02}.run.out")).exists());
	}
	assert!(logs.join("notes.txt").exists());
	assert!(logs.join("latest.run.out").exists());
}

#[tokio::test]
async fn an_unreadable_app_fails_the_operation_before_running_the_engine() {
	let fixture = Fixture::new(CONFIG, "");
	fs::remove_file(fixture.root.join("leptoskit.toml")).unwrap();
	let result = fixture
		.runner()
		.run(fixture.project.clone(), operation("status", Action::Status))
		.await;
	assert!(!result.succeeded);
	assert_eq!(result.details, json!({"code": "worker_failed"}));
	assert!(fixture.calls().is_empty());
}

#[tokio::test]
async fn a_commit_is_built_and_then_deployed_from_an_isolated_checkout() {
	let fixture = Fixture::new(CONFIG, "");
	let version = commit(&fixture.root);
	// Connecting the app writes the service URL after the commit; the policy
	// check ignores it.
	fs::write(
		fixture.root.join("leptoskit.toml"),
		format!("{CONFIG}control_url = \"https://control.example.com\"\n"),
	)
	.unwrap();
	let result = fixture
		.runner()
		.run(fixture.project.clone(), operation("source", deploy(&version)))
		.await;
	assert!(result.succeeded, "{}", result.message);
	let calls = fixture.calls();
	assert_eq!(calls.len(), 2);
	assert_eq!(
		calls[0]["args"],
		format!("infra build --version {version} --push --reuse-existing")
	);
	assert_eq!(calls[1]["args"], format!("deploy --version {version}"));
	let workspace = fixture.data.join("workspaces/source");
	for call in &calls {
		assert_eq!(call["cwd"], workspace.display().to_string());
		assert_eq!(
			call["shared"],
			fixture
				.root
				.join("infra/notes-production")
				.display()
				.to_string()
		);
	}
	assert!(fixture.log("source.build.out").contains("infra build"));
	assert!(fixture.log("source.run.out").contains("deploy"));
	assert!(!workspace.exists());
	assert!(fixture.root.join("infra/notes-production").is_dir());
}

#[tokio::test]
async fn a_failed_build_stops_before_the_deploy() {
	let fixture = Fixture::new(CONFIG, r#"if [ "$2" = build ]; then exit 4; fi"#);
	let version = commit(&fixture.root);
	let result = fixture
		.runner()
		.run(fixture.project.clone(), operation("source", deploy(&version)))
		.await;
	assert!(!result.succeeded);
	assert_eq!(result.details, json!({"code": "command_failed"}));
	assert_eq!(fixture.calls().len(), 1);
	assert!(!fixture.data.join("workspaces/source").exists());
}

#[tokio::test]
async fn a_commit_with_different_infrastructure_is_refused() {
	let fixture = Fixture::new(CONFIG, "");
	let version = commit(&fixture.root);
	fs::write(
		fixture.root.join("leptoskit.toml"),
		CONFIG.replace("app.example.com", "other.example.com"),
	)
	.unwrap();
	let result = fixture
		.runner()
		.run(fixture.project.clone(), operation("source", deploy(&version)))
		.await;
	assert!(!result.succeeded);
	assert_eq!(result.details, json!({"code": "worker_failed"}));
	assert!(fixture.calls().is_empty());
	assert!(!fixture.data.join("workspaces/source").exists());
}

#[tokio::test]
async fn an_unavailable_commit_is_refused() {
	let fixture = Fixture::new(CONFIG, "");
	commit(&fixture.root);
	let result = fixture
		.runner()
		.run(
			fixture.project.clone(),
			operation("source", deploy(&"0123456789abcdef".repeat(4)[..40])),
		)
		.await;
	assert!(!result.succeeded);
	assert_eq!(result.details, json!({"code": "worker_failed"}));
	assert!(fixture.calls().is_empty());
}

#[tokio::test]
async fn a_tag_that_is_not_a_commit_is_released_without_a_build() {
	let fixture = Fixture::new(CONFIG, "");
	let result = fixture
		.runner()
		.run(
			fixture.project.clone(),
			operation("tag", deploy(&"abcdef0".repeat(6))),
		)
		.await;
	assert!(result.succeeded, "{}", result.message);
	let calls = fixture.calls();
	assert_eq!(calls.len(), 1);
	assert!(calls[0]["args"].starts_with("deploy --version abcdef0"));
}

/// A temporary directory the management store accepts: it refuses to open
/// databases that other users can read.
fn private_directory() -> tempfile::TempDir {
	let directory = tempfile::tempdir().unwrap();
	fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
	directory
}

#[test]
fn a_backup_copies_the_management_database_without_overwriting() {
	let directory = private_directory();
	let data = directory.path().join("service");
	Store::open(&data.join("console.sqlite3")).unwrap();
	let out = directory.path().join("console.backup");
	run(ConsoleCommand::Backup {
		data: Some(data.clone()),
		out: out.clone(),
	})
	.unwrap();
	assert!(Store::open(&out).unwrap().projects().unwrap().is_empty());
	let error = run(ConsoleCommand::Backup {
		data: Some(data),
		out,
	})
	.unwrap_err();
	assert!(error.to_string().contains("already exists"), "{error}");
}

#[test]
fn restoring_and_rolling_back_require_confirmation() {
	let directory = tempfile::tempdir().unwrap();
	let error = run(ConsoleCommand::Restore {
		data: Some(directory.path().join("service")),
		from: directory.path().join("backup"),
		previous: directory.path().join("previous"),
		yes: false,
	})
	.unwrap_err();
	assert!(error.to_string().contains("pass --yes"), "{error}");
	let error = run(ConsoleCommand::RollbackDatabase {
		data: Some(directory.path().join("service")),
		backup: directory.path().join("backup"),
		yes: false,
	})
	.unwrap_err();
	assert!(error.to_string().contains("pass --yes"), "{error}");
	assert!(!directory.path().join("service").exists());
}

#[test]
fn a_restore_keeps_the_previous_database_and_revokes_restored_tokens() {
	let source = Fixture::new(CONFIG, "");
	let store = Store::open(&source.data.join("console.sqlite3")).unwrap();
	let token = store
		.issue(Scope::Read, Some("notes-production"), 3600, false)
		.unwrap();
	assert_eq!(
		overview(&source.service, &token),
		axum::http::StatusCode::OK
	);
	let directory = private_directory();
	let backup = directory.path().join("console.backup");
	store.backup(&backup).unwrap();
	let data = directory.path().join("service");
	Store::open(&data.join("console.sqlite3")).unwrap();
	let previous = directory.path().join("previous.backup");
	run(ConsoleCommand::Restore {
		data: Some(data.clone()),
		from: backup,
		previous: previous.clone(),
		yes: true,
	})
	.unwrap();
	assert!(Store::open(&previous).unwrap().projects().unwrap().is_empty());
	let restored = Store::open(&data.join("console.sqlite3")).unwrap();
	assert_eq!(restored.projects().unwrap()[0].id, "notes-production");
	let service = Service::new(restored, OWNER, "http://localhost:4700").unwrap();
	assert_eq!(
		overview(&service, &token),
		axum::http::StatusCode::UNAUTHORIZED
	);
}

fn overview(service: &Service, token: &str) -> axum::http::StatusCode {
	use tower::ServiceExt as _;
	tokio::runtime::Builder::new_current_thread()
		.enable_all()
		.build()
		.unwrap()
		.block_on(
			service.router().oneshot(
				axum::http::Request::get("/api/v1/overview")
					.header("authorization", format!("Bearer {token}"))
					.body(axum::body::Body::empty())
					.unwrap(),
			),
		)
		.unwrap()
		.status()
}

#[test]
fn a_schema_rollback_takes_a_backup_first() {
	let directory = private_directory();
	let data = directory.path().join("service");
	Store::open(&data.join("console.sqlite3"))
		.unwrap()
		.register(&Project {
			id: "notes-production".into(),
			environment: "production".into(),
			root: directory.path().to_owned(),
			revision: 0,
		})
		.unwrap();
	let backup = directory.path().join("before-rollback.backup");
	run(ConsoleCommand::RollbackDatabase {
		data: Some(data.clone()),
		backup: backup.clone(),
		yes: true,
	})
	.unwrap();
	assert_eq!(Store::open(&backup).unwrap().projects().unwrap().len(), 1);
	// The reversal dropped the schema and its history; opening the database
	// creates an empty one.
	assert!(
		Store::open(&data.join("console.sqlite3"))
			.unwrap()
			.projects()
			.unwrap()
			.is_empty()
	);
}

#[test]
fn offline_maintenance_waits_for_the_running_service() {
	let directory = tempfile::tempdir().unwrap();
	let data = directory.path().join("service");
	let _running = directory_lock(&data).unwrap();
	let error = run(ConsoleCommand::RollbackDatabase {
		data: Some(data),
		backup: directory.path().join("backup"),
		yes: true,
	})
	.unwrap_err();
	assert!(error.to_string().contains("stop it before"), "{error}");
}

#[test]
fn a_shared_data_directory_is_refused() {
	let directory = tempfile::tempdir().unwrap();
	let data = directory.path().join("service");
	fs::create_dir_all(&data).unwrap();
	fs::set_permissions(&data, fs::Permissions::from_mode(0o750)).unwrap();
	let error = directory_lock(&data).unwrap_err();
	assert!(error.to_string().contains("dedicated private"), "{error}");
	let fresh = directory.path().join("fresh");
	directory_lock(&fresh).unwrap();
	assert_eq!(
		fs::metadata(&fresh).unwrap().permissions().mode() & 0o777,
		0o700
	);
}

#[test]
fn the_data_directory_defaults_to_the_home_directory() {
	let explicit = PathBuf::from("/srv/console");
	assert_eq!(data_directory(Some(explicit.clone())).unwrap(), explicit);
	assert!(
		data_directory(None)
			.unwrap()
			.ends_with(".local/share/leptoskit/console")
	);
}

#[test]
fn the_service_only_registers_existing_apps_with_infrastructure() {
	let directory = tempfile::tempdir().unwrap();
	let serve = |project: PathBuf| {
		run(ConsoleCommand::Serve {
			project: vec![project],
			data: Some(directory.path().join("service")),
			bind: "127.0.0.1:0".parse().unwrap(),
			url: "http://localhost:4700".into(),
		})
		.unwrap_err()
		.to_string()
	};
	assert!(serve(directory.path().join("missing")).contains("does not exist"));
	let app = directory.path().join("app");
	fs::create_dir_all(&app).unwrap();
	fs::write(app.join("leptoskit.toml"), "[app]\nname = \"notes\"\n").unwrap();
	assert!(serve(app).contains("needs [infra]"));
}

/// The hosted service's worker endpoints, recording what the worker sends.
#[derive(Clone, Default)]
struct Hosted {
	requests: Arc<Mutex<Vec<(String, String, Value)>>>,
}

impl Hosted {
	fn router(&self, claim: Value) -> axum::Router {
		let record = |hosted: Self, path: &'static str, reply: Value| {
			move |headers: axum::http::HeaderMap, body: String| {
				let hosted = hosted.clone();
				let reply = reply.clone();
				async move {
					hosted.requests.lock().unwrap().push((
						path.to_owned(),
						headers
							.get("authorization")
							.and_then(|value| value.to_str().ok())
							.unwrap_or_default()
							.to_owned(),
						serde_json::from_str(&body).unwrap_or(Value::Null),
					));
					if reply.is_null() {
						String::new()
					} else {
						reply.to_string()
					}
				}
			}
		};
		axum::Router::new()
			.route(
				"/api/v1/worker/finish",
				axum::routing::post(record(self.clone(), "finish", Value::Null)),
			)
			.route(
				"/api/v1/worker/claim",
				axum::routing::post(record(self.clone(), "claim", claim)),
			)
	}

	fn requests(&self) -> Vec<(String, String, Value)> {
		self.requests.lock().unwrap().clone()
	}
}

/// An app set up for the hosted worker, with a saved worker credential.
fn hosted_app(url: &str, credential: &Value) -> (tempfile::TempDir, PathBuf, PathBuf) {
	let directory = tempfile::tempdir().unwrap();
	let root = directory.path().join("app");
	let data = directory.path().join("worker");
	fs::create_dir_all(&root).unwrap();
	fs::create_dir_all(&data).unwrap();
	fs::set_permissions(&data, fs::Permissions::from_mode(0o700)).unwrap();
	fs::write(
		root.join("leptoskit.toml"),
		CONFIG.replace("[infra]", "[infra]\nhosted_state = true"),
	)
	.unwrap();
	let mut credential = credential.clone();
	if credential["url"].is_null() {
		credential["url"] = json!(url);
	}
	let path = data.join("worker.json");
	fs::write(&path, credential.to_string()).unwrap();
	fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
	(directory, root, data)
}

fn credential() -> Value {
	json!({
		"project": "notes-production",
		"key": "pilot.hosted-worker-key",
		"ownership": "customer",
	})
}

#[test]
fn the_hosted_worker_requires_hosted_state() {
	let (_directory, root, data) = hosted_app("http://127.0.0.1:9", &credential());
	fs::write(root.join("leptoskit.toml"), CONFIG).unwrap();
	let error = hosted_worker::run("http://127.0.0.1:9", &root, &data).unwrap_err();
	assert!(error.to_string().contains("hosted_state=true"), "{error}");
}

#[test]
fn a_credential_for_another_installation_is_refused() {
	let mut other = credential();
	other["project"] = json!("other-app");
	let (_directory, root, data) = hosted_app("http://127.0.0.1:9", &other);
	let error = hosted_worker::run("http://127.0.0.1:9", &root, &data).unwrap_err();
	assert!(error.to_string().contains("another installation"), "{error}");
	let mut moved = credential();
	moved["url"] = json!("http://localhost:9");
	let (_directory, root, data) = hosted_app("http://127.0.0.1:9", &moved);
	let error = hosted_worker::run("http://127.0.0.1:9", &root, &data).unwrap_err();
	assert!(error.to_string().contains("another installation"), "{error}");
}

#[test]
fn a_readable_worker_credential_is_refused() {
	let (_directory, root, data) = hosted_app("http://127.0.0.1:9", &credential());
	fs::set_permissions(data.join("worker.json"), fs::Permissions::from_mode(0o644)).unwrap();
	let error = hosted_worker::run("http://127.0.0.1:9", &root, &data).unwrap_err();
	assert!(error.to_string().contains("must be private"), "{error}");
}

#[test]
fn managed_capacity_requires_digitalocean_and_a_budget() {
	let mut managed = credential();
	managed["ownership"] = json!("leptoskit_digitalocean");
	let (_directory, root, data) = hosted_app("http://127.0.0.1:9", &managed);
	let error = hosted_worker::run("http://127.0.0.1:9", &root, &data).unwrap_err();
	assert!(error.to_string().contains("DigitalOcean"), "{error}");
}

#[test]
fn an_interrupted_execution_must_be_reconciled_before_new_work() {
	let (_directory, root, data) = hosted_app("http://127.0.0.1:9", &credential());
	fs::write(
		data.join("pending.json"),
		json!({"operation": "interrupted"}).to_string(),
	)
	.unwrap();
	let error = hosted_worker::run("http://127.0.0.1:9", &root, &data).unwrap_err();
	assert!(error.to_string().contains("reconcile"), "{error}");
	assert!(data.join("pending.json").exists());
}

#[test]
fn a_saved_result_is_reported_and_work_for_another_app_stops_the_worker() {
	let hosted = Hosted::default();
	let mut foreign = operation("foreign", Action::Status);
	foreign.project = "other-app".into();
	let url = serve(hosted.router(json!({
		"operation": foreign,
		"analytics_read_key": "read",
		"analytics_ingest_key": "ingest",
	})));
	let (_directory, root, data) = hosted_app(&url, &credential());
	let result = json!({"id": "finished", "succeeded": true, "message": "Completed", "details": {}});
	fs::write(
		data.join("pending.json"),
		json!({"operation": "finished", "result": result}).to_string(),
	)
	.unwrap();
	let error = hosted_worker::run(&url, &root, &data).unwrap_err();
	assert!(error.to_string().contains("different project"), "{error}");
	let requests = hosted.requests();
	assert_eq!(
		requests,
		[
			(
				"finish".to_owned(),
				"Bearer pilot.hosted-worker-key".to_owned(),
				result
			),
			(
				"claim".to_owned(),
				"Bearer pilot.hosted-worker-key".to_owned(),
				json!({})
			),
		]
	);
	assert!(!data.join("pending.json").exists());
	assert!(data.join("local.sqlite3").exists());
}
