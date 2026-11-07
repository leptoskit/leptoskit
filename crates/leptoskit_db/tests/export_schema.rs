//! `export_schema!`'s work, driven the way `leptoskit db` drives it: through
//! the environment, in a child process so each setting starts clean.
//!
//! The variables are process-wide, so the parent runs this test again with
//! them set and checks what the child wrote or why it stopped.

#![cfg(feature = "server")]

use std::ffi::OsStr;
use std::process::Command;
use std::process::Output;

use leptoskit_db::DIALECT_ENV;
use leptoskit_db::GEOPACKAGE_ENV;
use leptoskit_db::Id;
use leptoskit_db::LIVE_CAPTURE_ENV;
use leptoskit_db::POLICIES_ENV;
use leptoskit_db::SCHEMA_OUT_ENV;
use leptoskit_db::STUDIO_ENV;
use leptoskit_db::table;

const SCENARIO: &str = "LEPTOSKIT_DB_EXPORT_SCENARIO";

/// A table whose schema the export should hold.
#[table(name = "exported_notes", live)]
pub struct ExportedNote {
	pub id: Id<Self>,
	pub body: String,
}

#[test]
fn export_schema_follows_the_environment() {
	if std::env::var_os(SCENARIO).is_some() {
		leptoskit_db::__private::export_schema_if_requested();

		return;
	}

	// Nothing asks for an export, so it does nothing.
	assert!(run("nothing", &[], &[]).status.success());

	// Each dialect, both policy renderings, and GeoPackage's metadata.
	for (name, dialect) in [("sqlite", "sqlite"), ("postgres", "postgres")] {
		let directory = tempfile::tempdir().unwrap();
		let out = directory.path().join("schema.json");

		let output = run(
			"export",
			&[
				(DIALECT_ENV, dialect),
				(POLICIES_ENV, "orm"),
				(GEOPACKAGE_ENV, "0"),
			],
			&[(SCHEMA_OUT_ENV, out.as_os_str())],
		);
		assert!(output.status.success(), "{output:?}");

		let schema: leptoskit_db_schema::Schema =
			serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
		assert_eq!(schema.dialect.to_string(), dialect);
		assert!(
			schema
				.tables
				.iter()
				.any(|table| table.name.name == "exported_notes"),
			"{name} exports the app's tables"
		);
	}

	let directory = tempfile::tempdir().unwrap();
	let out = directory.path().join("rls.json");
	let output = run(
		"rls",
		&[(DIALECT_ENV, "postgres"), (POLICIES_ENV, "rls")],
		&[(SCHEMA_OUT_ENV, out.as_os_str())],
	);
	assert!(output.status.success(), "{output:?}");

	let out = directory.path().join("geopackage.json");
	let output = run(
		"geopackage",
		&[
			(DIALECT_ENV, "sqlite"),
			(GEOPACKAGE_ENV, "1"),
			(LIVE_CAPTURE_ENV, "triggers"),
		],
		&[(SCHEMA_OUT_ENV, out.as_os_str())],
	);
	assert!(output.status.success(), "{output:?}");

	// A setting the exporter doesn't know stops with a message naming it.
	let never = directory.path().join("bad.json");

	for (variable, value) in [
		(DIALECT_ENV, "mysql"),
		(POLICIES_ENV, "both"),
		(GEOPACKAGE_ENV, "yes"),
		(LIVE_CAPTURE_ENV, "listen"),
	] {
		let output = run(
			"bad",
			&[(variable, value)],
			&[(SCHEMA_OUT_ENV, never.as_os_str())],
		);
		assert!(
			!output.status.success(),
			"{variable}={value} should stop: {:?}, stdout {}, stderr {}",
			output.status.code(),
			String::from_utf8_lossy(&output.stdout),
			stderr(&output)
		);
		assert!(
			stderr(&output).contains(variable),
			"{variable}={value}: {}",
			stderr(&output)
		);
	}

	// Replication capture needs Postgres.
	let never = directory.path().join("never.json");
	let output = run(
		"replication",
		&[(DIALECT_ENV, "sqlite"), (LIVE_CAPTURE_ENV, "replication")],
		&[(SCHEMA_OUT_ENV, never.as_os_str())],
	);
	assert!(!output.status.success());
	assert!(
		stderr(&output).contains("replication capture needs Postgres"),
		"{}",
		stderr(&output)
	);
	assert!(!never.exists());

	// A file that can't be written is reported with its path.
	let output = run(
		"unwritable",
		&[(DIALECT_ENV, "sqlite")],
		&[(SCHEMA_OUT_ENV, directory.path().as_os_str())],
	);
	assert!(!output.status.success());
	assert!(
		stderr(&output).contains("couldn't write the schema"),
		"{}",
		stderr(&output)
	);

	// Asked to serve the studio, it tries, and here the address is bad.
	let output = run("studio", &[(STUDIO_ENV, "not-an-address")], &[]);
	assert!(!output.status.success());
	assert!(stderr(&output).contains(STUDIO_ENV), "{}", stderr(&output));
}

/// Runs this test binary again with `scenario` and `variables` set.
fn run(scenario: &str, variables: &[(&str, &str)], paths: &[(&str, &OsStr)]) -> Output {
	let mut command = Command::new(std::env::current_exe().unwrap());
	command
		.args([
			"--exact",
			"export_schema_follows_the_environment",
			"--nocapture",
		])
		.env(SCENARIO, scenario);

	for name in [
		DIALECT_ENV,
		POLICIES_ENV,
		GEOPACKAGE_ENV,
		LIVE_CAPTURE_ENV,
		SCHEMA_OUT_ENV,
		STUDIO_ENV,
	] {
		command.env_remove(name);
	}

	command.envs(variables.iter().copied());

	for (name, path) in paths {
		command.env(name, path);
	}

	command.output().unwrap()
}

fn stderr(output: &Output) -> String {
	String::from_utf8_lossy(&output.stderr).into_owned()
}
