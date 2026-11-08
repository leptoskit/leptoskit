//! Command acceptance uses fake SSH and engine observations. Real Linux file
//! installation and old/new PostgreSQL processes have separate acceptance.

use leptoskit_db::Migration;
use leptoskit_db::MigrationContract;
use leptoskit_db::MigrationWindow;
use leptoskit_db::checksum;
use leptoskit_db_schema::Column;
use leptoskit_db_schema::ColumnType;
use leptoskit_db_schema::Dialect;
use leptoskit_db_schema::RequireHints;
use leptoskit_db_schema::Schema;
use leptoskit_db_schema::Table;
use leptoskit_db_schema::TableName;
use serde_json::Value;
use serde_json::json;

use super::*;

struct Fixture {
	root: tempfile::TempDir,
	path: String,
	pin: String,
	history: String,
	fingerprint: String,
	window: String,
	window_pin: String,
}

impl Fixture {
	fn new() -> Self {
		Self::with_comment(None)
	}

	fn with_comment(comment: Option<String>) -> Self {
		let root = project();
		let (path, _) = verified_descriptor(root.path());
		let mut baseline = Schema::new(
			Dialect::Postgres,
			vec![Table::new(
				TableName::new("notes"),
				vec![Column::new("id", ColumnType::Int64)],
				vec!["id".into()],
			)],
		);
		baseline.tables[0].comment = comment;
		let mut expanded = baseline.clone();
		expanded.tables[0]
			.columns
			.push(Column::new("body", ColumnType::TEXT).nullable());
		let render = |before: &Schema, after: &Schema| {
			let plan = leptoskit_db_schema::diff(before, after, &mut RequireHints).unwrap();
			leptoskit_db_schema::render(&plan, before, after).sql()
		};
		let sql = render(&Schema::empty(Dialect::Postgres), &baseline);
		let migrations = [Migration::new("initial", sql.clone())];
		fs::create_dir(root.path().join("migrations/initial")).unwrap();
		fs::write(root.path().join("migrations/initial/migration.sql"), &sql).unwrap();
		fs::write(
			root.path().join("migrations/initial/schema.json"),
			baseline.to_json(),
		)
		.unwrap();
		fs::write(root.path().join("migrations/registry.txt"), "initial\n").unwrap();
		let history = serde_json::to_string(&vec![("initial", checksum(&sql))]).unwrap();
		let fingerprint = checksum(&history);
		let window = MigrationWindow::new(
			&migrations,
			&baseline.to_json(),
			[(
				Migration::new("expanded", render(&baseline, &expanded)),
				expanded.to_json(),
			)],
		)
		.unwrap()
		.to_json();
		let contract = MigrationContract::new(&migrations, Some(&baseline.to_json()))
			.unwrap()
			.to_json();
		let window_pin = checksum(&window);
		let mut value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
		value["format_version"] = json!(4);
		value["migrations"] = json!(fingerprint);
		value["schema_window"] = json!({"format_version":1, "window":window, "window_sha256":window_pin,
			"contract":contract, "contract_sha256":checksum(&contract)});
		let bytes = serde_json::to_string(&value).unwrap();
		let pin = checksum(&bytes);
		fs::write(&path, bytes).unwrap();
		Self {
			root,
			path,
			pin,
			history,
			fingerprint,
			window,
			window_pin,
		}
	}

	fn command(&self, args: &[&str], extra: &[(&str, &str)]) -> Output {
		let mut env = vec![
			("MOCK_HISTORY", self.history.as_str()),
			("MOCK_SCHEMA_LABEL", self.fingerprint.as_str()),
		];
		env.extend_from_slice(extra);
		cli(self.root.path(), args, &env)
	}

	fn initial(&self) {
		succeeds(&self.command(&["deploy", "--setup", "--version", "v1"], &[]));
	}

	fn deploy(&self, version: &str, extra: &[(&str, &str)]) -> Output {
		self.command(
			&[
				"deploy",
				"--version",
				version,
				"--verified-descriptor",
				&self.path,
				"--descriptor-sha256",
				&self.pin,
			],
			extra,
		)
	}

	fn host_file(&self) -> std::path::PathBuf {
		self.root
			.path()
			.join("remote/state/notes-production/control/schema-windows")
			.join(format!("{}.json", self.window_pin))
	}

	fn boot(&self, version: &str) -> Value {
		serde_json::from_slice(
			&fs::read(
				self.root
					.path()
					.join("infra/notes-production")
					.join(format!("boot-window-{version}.json")),
			)
			.unwrap(),
		)
		.unwrap()
	}
}

#[test]
fn approved_window_requires_an_installed_baseline_before_setup() {
	let fixture = Fixture::new();
	fails_with(
		&fixture.command(
			&[
				"deploy",
				"--setup",
				"--version",
				"v1",
				"--verified-descriptor",
				&fixture.path,
				"--descriptor-sha256",
				&fixture.pin,
			],
			&[],
		),
		"requires an installed baseline",
	);
	assert!(
		!fixture
			.root
			.path()
			.join("infra/notes-production/kamal-calls")
			.exists()
	);
	assert!(!fixture.host_file().exists());
}

#[test]
fn approved_window_is_delivered_and_reused_with_recorded_evidence() {
	let fixture = Fixture::new();
	fixture.initial();
	succeeds(&fixture.deploy("v2", &[]));
	assert_eq!(
		fs::read_to_string(fixture.host_file()).unwrap(),
		fixture.window
	);
	assert_eq!(
		fixture.boot("v2")["env"]["clear"]["LEPTOSKIT_MIGRATION_WINDOW_SHA256"],
		fixture.window_pin
	);
	for flags in [
		vec!["deploy", "--rollback", "v2"],
		vec!["deploy", "--recover", "--version", "v2"],
	] {
		succeeds(&fixture.command(&flags, &[]));
	}
	assert_eq!(release_count(fixture.root.path()), 4);
}

#[test]
fn conflicting_window_file_or_lost_reservation_prevents_boot() {
	for failure in ["file", "install", "reservation"] {
		let fixture = Fixture::new();
		fixture.initial();
		if failure == "file" {
			fs::create_dir_all(fixture.host_file().parent().unwrap()).unwrap();
			fs::write(fixture.host_file(), "conflicting input").unwrap();
		}
		let extra = match failure {
			"install" => vec![("MOCK_WINDOW_INSTALL_FAIL", "1")],
			"reservation" => vec![("MOCK_WINDOW_LOSE_RESERVATION", "1")],
			_ => vec![],
		};
		assert!(!fixture.deploy("v2", &extra).status.success());
		assert_eq!(release_count(fixture.root.path()), 1);
		assert!(
			!fixture
				.root
				.path()
				.join("infra/notes-production/boot-window-v2.json")
				.exists()
		);
		if failure == "file" {
			assert_eq!(
				fs::read_to_string(fixture.host_file()).unwrap(),
				"conflicting input"
			);
		}
	}
}

#[test]
fn failed_window_observation_stops_the_application_without_advancing_history() {
	for mismatch in ["pin", "mount", "file"] {
		let fixture = Fixture::new();
		fixture.initial();
		assert!(
			!fixture
				.deploy("v2", &[("MOCK_WINDOW_OBSERVATION", mismatch)])
				.status
				.success()
		);
		assert_eq!(release_count(fixture.root.path()), 1);
		assert_eq!(
			operation(fixture.root.path())["phase"],
			"verification-failed-app-stopped"
		);
	}
}

#[test]
fn boot_failure_restores_the_previous_window_configuration() {
	let fixture = Fixture::new();
	fixture.initial();
	// A previous unverified release cannot be auto-restored after verified
	// history exists, but the initial failed candidate has not entered history.
	assert!(
		!fixture
			.deploy("v2", &[("MOCK_KAMAL_FAIL_VERSION", "v2")])
			.status
			.success()
	);
	assert_eq!(operation(fixture.root.path())["phase"], "previous-restored");
	assert!(
		fixture.boot("v1")["env"]["clear"]
			.get("LEPTOSKIT_MIGRATION_WINDOW")
			.is_none()
	);
	succeeds(&fixture.deploy("v2", &[]));
	let mut value: Value = serde_json::from_slice(&fs::read(&fixture.path).unwrap()).unwrap();
	let candidate = format!("{}\n", fixture.window);
	value["schema_window"]["window"] = json!(candidate);
	value["schema_window"]["window_sha256"] = json!(checksum(&candidate));
	let bytes = serde_json::to_string(&value).unwrap();
	fs::write(&fixture.path, &bytes).unwrap();
	let fixture = Fixture {
		pin: checksum(&bytes),
		..fixture
	};
	assert!(
		!fixture
			.deploy("v3", &[("MOCK_KAMAL_FAIL_VERSION", "v3")])
			.status
			.success()
	);
	assert_eq!(operation(fixture.root.path())["phase"], "previous-restored");
	assert_eq!(
		fixture.boot("v3")["env"]["clear"]["LEPTOSKIT_MIGRATION_WINDOW_SHA256"],
		checksum(&candidate)
	);
	assert_eq!(
		fixture.boot("v2")["env"]["clear"]["LEPTOSKIT_MIGRATION_WINDOW_SHA256"],
		fixture.window_pin
	);
	assert_eq!(release_count(fixture.root.path()), 2);
}

#[test]
fn oversized_evidence_is_refused_before_changing_the_application() {
	// Both schemas have the same comment, so the generated expansion remains
	// a nullable column addition. The valid window fits its own 8 MiB bound;
	// the complete descriptor would exceed the shared history reader's bound.
	let fixture = Fixture::with_comment(Some("x".repeat(1536 * 1024)));
	fixture.initial();
	let calls = fixture
		.root
		.path()
		.join("infra/notes-production/kamal-calls");
	let before = fs::read(&calls).unwrap();
	fails_with(&fixture.deploy("v2", &[]), "release history would exceed");
	assert_eq!(fs::read(calls).unwrap(), before);
	assert_eq!(release_count(fixture.root.path()), 1);
	assert!(!fixture.host_file().exists());
}

#[test]
fn repeated_evidence_cannot_make_previously_readable_history_unreadable() {
	let fixture = Fixture::with_comment(Some("x".repeat(768 * 1024)));
	fixture.initial();
	succeeds(&fixture.deploy("v2", &[]));
	let history = fixture
		.root
		.path()
		.join("remote/state/notes-production/control/releases.json");
	let before = fs::read(&history).unwrap();
	fails_with(&fixture.deploy("v3", &[]), "release history would exceed");
	assert_eq!(fs::read(history).unwrap(), before);
	assert_eq!(release_count(fixture.root.path()), 2);
	assert!(
		!fixture
			.root
			.path()
			.join("infra/notes-production/boot-window-v3.json")
			.exists()
	);
}

#[test]
fn malformed_registry_digest_is_never_written_into_release_history() {
	for digest in [
		"g".repeat(64),
		"a".repeat(63),
		format!("{}extra", "a".repeat(64)),
	] {
		let fixture = Fixture::new();
		fixture.initial();
		fails_with(
			&fixture.deploy("v2", &[("MOCK_CONTAINER_DIGEST", &digest)]),
			"running container has no verified registry digest",
		);
		assert_eq!(release_count(fixture.root.path()), 1);
		fails_with(
			&fixture.command(
				&["deploy", "--recover", "--version", "v1"],
				&[("MOCK_DIGEST", &digest)],
			),
			"pulled image lacks a registry digest",
		);
		assert_eq!(release_count(fixture.root.path()), 1);
	}
}

#[test]
fn reused_window_is_freshly_checked_without_installation_or_boot() {
	let fixture = Fixture::new();
	fixture.initial();
	succeeds(&fixture.deploy("v2", &[]));
	let history = fixture
		.root
		.path()
		.join("remote/state/notes-production/control/releases.json");
	let calls = fixture
		.root
		.path()
		.join("infra/notes-production/kamal-calls");
	let before = fs::read(&history).unwrap();
	let called = fs::read(&calls).unwrap();
	let args = [
		"deploy",
		"--version",
		"v2",
		"--reuse-current",
		"--verified-descriptor",
		&fixture.path,
		"--descriptor-sha256",
		&fixture.pin,
	];
	let base = [
		("MOCK_REUSE_CONTAINER", "notes-production-web-v2"),
		("MOCK_REUSE_WINDOW_PIN", fixture.window_pin.as_str()),
		("MOCK_WINDOW_INSTALL_FAIL", "1"),
	];
	succeeds(&fixture.command(&args, &base));
	for mismatch in [
		"pin",
		"mount",
		"file",
		"missing-env",
		"missing-mount",
		"source",
	] {
		let mut extra = base.to_vec();
		extra.push(("MOCK_WINDOW_OBSERVATION", mismatch));
		assert!(
			!fixture.command(&args, &extra).status.success(),
			"{mismatch}"
		);
		assert_eq!(fs::read(&history).unwrap(), before);
		assert_eq!(fs::read(&calls).unwrap(), called);
	}
	// A valid replacement document is still not the approved exact bytes.
	fs::set_permissions(fixture.host_file(), fs::Permissions::from_mode(0o644)).unwrap();
	fs::write(fixture.host_file(), format!("{}\n", fixture.window)).unwrap();
	fs::set_permissions(fixture.host_file(), fs::Permissions::from_mode(0o444)).unwrap();
	fails_with(&fixture.command(&args, &base), "migration window changed");
	let original_descriptor = fs::read(&fixture.path).unwrap();
	let mut changed: Value = serde_json::from_slice(&original_descriptor).unwrap();
	let replacement = format!("{}\n", fixture.window);
	changed["schema_window"]["window"] = json!(replacement);
	changed["schema_window"]["window_sha256"] = json!(checksum(&replacement));
	let bytes = serde_json::to_vec(&changed).unwrap();
	fs::write(&fixture.path, &bytes).unwrap();
	let replacement_pin = checksum(std::str::from_utf8(&bytes).unwrap());
	let mut changed_args = args;
	changed_args[7] = &replacement_pin;
	fails_with(
		&fixture.command(&changed_args, &base),
		"descriptor differs from the current release",
	);
	fs::write(&fixture.path, original_descriptor).unwrap();
	fs::remove_file(fixture.host_file()).unwrap();
	assert!(!fixture.command(&args, &base).status.success());
	assert!(!fixture.host_file().exists());
	assert_eq!(fs::read(&history).unwrap(), before);
	assert_eq!(fs::read(&calls).unwrap(), called);
}
