//! Release markers and reading a migrations directory from disk: the parts of
//! `leptoskit db`'s bookkeeping that need no database.

#![cfg(feature = "server")]

use std::fs;
use std::path::Path;

use leptoskit_db::Migration;
use leptoskit_db::MigrationError;

const SNAPSHOT: &str = "f0e1d2c3b4a5968778695a4b3c2d1e0f";

fn write(directory: &Path, path: &str, contents: &str) {
	let path = directory.join(path);
	fs::create_dir_all(path.parent().unwrap()).unwrap();
	fs::write(path, contents).unwrap();
}

fn registry() -> String {
	format!(
		"# the first migration\n\n20260101T0900_create\n20260102T0900_consolidate\n# release \
		 1.4.0 (monochange-20261101) snapshot {SNAPSHOT}\n"
	)
}

#[test]
fn release_markers_parse_the_version_source_and_snapshot() {
	let marker = leptoskit_db::parse_release("# release 1.4.0 snapshot abc123").unwrap();
	assert_eq!(marker.version, "1.4.0");
	assert_eq!(marker.source, None);
	assert_eq!(marker.snapshot, "abc123");

	let line = format!("# release 1.4.0 (monochange-20261101) snapshot {SNAPSHOT}");
	let marker = leptoskit_db::parse_release(&line).unwrap();
	assert_eq!(marker.version, "1.4.0");
	assert_eq!(marker.source, Some("monochange-20261101"));
	assert_eq!(marker.snapshot, SNAPSHOT);

	// A source may hold spaces, and the first ` snapshot ` ends it.
	let marker =
		leptoskit_db::parse_release("# release 2.0 (a release id) snapshot a snapshot b").unwrap();
	assert_eq!(marker.version, "2.0");
	assert_eq!(marker.source, Some("a release id"));
	assert_eq!(marker.snapshot, "a snapshot b");

	// An empty source is still a source.
	let marker = leptoskit_db::parse_release("# release 2.0 () snapshot abc").unwrap();
	assert_eq!(marker.source, Some(""));
}

#[test]
fn release_markers_with_a_typo_or_no_parts_dont_parse() {
	for line in [
		"# release",
		"# release 1.4.0",
		"# release 1.4.0 snapshot ",
		"# release  snapshot abc123",
		"# released 1.4.0 snapshot abc123",
		"release 1.4.0 snapshot abc123",
	] {
		assert!(
			leptoskit_db::parse_release(line).is_none(),
			"{line:?} shouldn't parse"
		);
	}

	// An unclosed source is part of the version, which is wrong, but the
	// registry reader below is what decides whether to refuse it.
	let marker = leptoskit_db::parse_release("# release 1.4 (source snapshot abc").unwrap();
	assert_eq!(marker.version, "1.4 (source");
	assert_eq!(marker.source, None);
}

#[test]
fn a_registry_lists_migrations_and_release_markers_separately() {
	let registry = registry();
	assert_eq!(
		leptoskit_db::registry_names(&registry).collect::<Vec<_>>(),
		["20260101T0900_create", "20260102T0900_consolidate"]
	);

	let releases: Vec<leptoskit_db::Release<'_>> =
		leptoskit_db::registry_releases(&registry).collect();
	assert_eq!(releases.len(), 1);
	assert_eq!(releases[0].version, "1.4.0");
	assert_eq!(releases[0].source, Some("monochange-20261101"));
	assert_eq!(releases[0].snapshot, SNAPSHOT);

	// Markers keep their order, and a malformed one is skipped rather than
	// taking the well-formed ones with it.
	let registry = "# release 2.0 snapshot two\n# release nope\n\n# release 1.0 snapshot one\n";
	let versions: Vec<&str> = leptoskit_db::registry_releases(registry)
		.map(|release| release.version)
		.collect();
	assert_eq!(versions, ["2.0", "1.0"]);
	assert_eq!(
		leptoskit_db::registry_releases("# only comments\n").count(),
		0
	);
}

#[test]
fn checksums_are_the_sha256_of_the_sql_in_hex() {
	assert_eq!(
		leptoskit_db::checksum(""),
		"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
	);
	assert_eq!(
		leptoskit_db::checksum("abc"),
		"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
	);

	let migration = Migration::new("20260101T0900_create", "SELECT 1;");
	assert_eq!(migration.checksum, leptoskit_db::checksum("SELECT 1;"));
	assert_eq!(migration.name, "20260101T0900_create");
	assert_eq!(migration.down, None);
	assert_eq!(migration.replaces.len(), 0);
}

#[test]
fn a_migration_says_when_foreign_keys_must_be_off() {
	let plain = Migration::new("plain", "CREATE TABLE t (id INTEGER);");
	assert!(!plain.foreign_keys_off());

	let rebuild = Migration::new(
		"rebuild",
		"PRAGMA foreign_keys = off;\n-- leptoskit: foreign-keys-off\nCREATE TABLE t (id INTEGER);",
	);
	assert!(rebuild.foreign_keys_off());

	let indented = Migration::new("indented", "  -- leptoskit: foreign-keys-off  \nSELECT 1;");
	assert!(indented.foreign_keys_off());

	// Only a whole line asks for it: a trailing comment doesn't.
	let trailing = Migration::new(
		"trailing",
		"CREATE TABLE t (id INTEGER); -- leptoskit: foreign-keys-off",
	);
	assert!(!trailing.foreign_keys_off());
}

#[test]
fn loading_reads_the_registry_down_sql_and_replaces() {
	let directory = tempfile::tempdir().unwrap();
	write(directory.path(), "registry.txt", &registry());
	write(
		directory.path(),
		"20260101T0900_create/migration.sql",
		"CREATE TABLE notes (id INTEGER PRIMARY KEY);",
	);
	write(
		directory.path(),
		"20260101T0900_create/down.sql",
		"DROP TABLE notes;",
	);
	write(
		directory.path(),
		"20260102T0900_consolidate/migration.sql",
		"SELECT 1;",
	);
	write(
		directory.path(),
		"20260102T0900_consolidate/replaces.json",
		r#"["20260101T0900_create"]"#,
	);

	let loaded = leptoskit_db::load_migrations(directory.path()).unwrap();
	assert_eq!(loaded.len(), 2);

	assert_eq!(loaded[0].name, "20260101T0900_create");
	assert_eq!(loaded[0].down.as_deref(), Some("DROP TABLE notes;"));
	assert_eq!(loaded[0].replaces.len(), 0);
	assert_eq!(
		loaded[0].checksum,
		leptoskit_db::checksum("CREATE TABLE notes (id INTEGER PRIMARY KEY);")
	);

	assert_eq!(loaded[1].name, "20260102T0900_consolidate");
	assert_eq!(loaded[1].down, None);
	assert_eq!(loaded[1].replaces, ["20260101T0900_create"]);
}

#[test]
fn loading_refuses_a_registry_that_points_at_nothing() {
	let directory = tempfile::tempdir().unwrap();
	write(directory.path(), "registry.txt", "20260101T0900_missing\n");

	let error = leptoskit_db::load_migrations(directory.path()).unwrap_err();
	assert!(matches!(error, MigrationError::Read { .. }));
	assert!(
		error.to_string().contains("20260101T0900_missing"),
		"{error}"
	);
}

#[test]
fn loading_refuses_replaces_that_are_not_a_list_of_names() {
	let directory = tempfile::tempdir().unwrap();
	write(directory.path(), "registry.txt", "20260101T0900_broken\n");
	write(
		directory.path(),
		"20260101T0900_broken/migration.sql",
		"SELECT 1;",
	);
	write(
		directory.path(),
		"20260101T0900_broken/replaces.json",
		"{\"not\": \"a list\"}",
	);

	let error = leptoskit_db::load_migrations(directory.path()).unwrap_err();
	assert!(matches!(error, MigrationError::Read { .. }));
	assert!(error.to_string().contains("replaces.json"), "{error}");
}

#[test]
fn embedded_migrations_keep_their_checksums_and_reverse_sql() {
	let plain = Migration::embedded("20260101T0900_create", "SELECT 1;", "given");
	assert_eq!(plain.name, "20260101T0900_create");
	assert_eq!(plain.checksum, "given");
	assert_eq!(plain.down, None);

	let reversible =
		Migration::embedded_reversible("20260102T0900_down", "SELECT 1;", "sum", "SELECT 2;");
	assert_eq!(reversible.down.as_deref(), Some("SELECT 2;"));
	assert_eq!(reversible.replaces, Vec::<String>::new());

	let built = Migration::new("built", "SELECT 1;")
		.with_down("SELECT 2;")
		.with_replaces(["older", "oldest"]);
	assert_eq!(built.down.as_deref(), Some("SELECT 2;"));
	assert_eq!(built.replaces, ["older", "oldest"]);
}
