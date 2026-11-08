//! Pack ownership through a real generated app and its exported schema.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use leptoskit_db_schema::Schema;
use serde_json::Value;

use super::fails;
use super::leptoskit;
use super::succeeds;

fn migration_files(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
	let directory = root.join("migrations");
	walkdir::WalkDir::new(&directory)
		.into_iter()
		.map(Result::unwrap)
		.map(|entry| {
			assert!(entry.file_type().is_file() || entry.file_type().is_dir());
			(
				entry.path().strip_prefix(&directory).unwrap().to_owned(),
				entry
					.file_type()
					.is_file()
					.then(|| fs::read(entry.path()).unwrap()),
			)
		})
		.collect()
}

fn latest_schema(root: &Path) -> Schema {
	let directory = root.join("migrations");
	let migrations = leptoskit_db::load_migrations(&directory).unwrap();
	let latest = migrations.last().unwrap();
	Schema::from_json(
		&fs::read_to_string(directory.join(latest.name.as_ref()).join("schema.json")).unwrap(),
	)
	.unwrap()
}

fn database_state(root: &Path) -> Value {
	let output = Command::new("python3")
		.arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/schema_pack_state.py"))
		.arg(root.join("notes.sqlite3"))
		.output()
		.expect("Python 3 with SQLite is required for generated pack acceptance");
	serde_json::from_str(&succeeds(&output)).unwrap()
}

pub(super) fn generated_app_requires_explicit_removal(root: &Path) {
	let baseline = latest_schema(root);
	let rpc = baseline
		.pack_membership()
		.expect("new scaffolds must record pack membership")
		.iter()
		.find(|pack| pack.name == "rpc")
		.expect("the generated packs! declaration must retain RPC ownership");
	assert!(!rpc.builtin);
	assert!(!rpc.tables.is_empty());
	let before = database_state(root);
	assert!(!before["rows"]["notes"].as_array().unwrap().is_empty());
	for table in &rpc.tables {
		assert!(before["rows"].get(table.flat()).is_some());
	}

	// Freeze the baseline before changing the dependency, including when a
	// generated project uses amend mode for unpublished migrations.
	succeeds(&leptoskit(root, &["db", "release", "record", "0.1.0"]));
	let files = migration_files(root);
	let source_path = root.join("notes_db/src/lib.rs");
	let source = fs::read_to_string(&source_path).unwrap();
	let declaration = "#[cfg(feature = \"server\")]\nleptoskit_db::packs!(leptoskit_rpc);";
	assert_eq!(source.matches(declaration).count(), 1);
	fs::write(&source_path, source.replace(declaration, "")).unwrap();
	let url = format!("sqlite://{}", root.join("notes.sqlite3").display());

	let commands: &[&[&str]] = &[
		&["db", "check"],
		&[
			"db",
			"migration",
			"create",
			"accidental",
			"--allow-destructive",
			"--no-renames",
		],
		&["db", "migration", "consolidate", "--no-renames"],
		&["db", "push", "--url", &url, "--allow-destructive"],
	];
	for command in commands {
		let error = fails(&leptoskit(root, command));
		assert!(error.contains("--drop-pack rpc"), "{command:?}: {error}");
		assert_eq!(
			migration_files(root),
			files,
			"{command:?} changed migration files"
		);
		assert_eq!(
			database_state(root),
			before,
			"{command:?} changed data or schema"
		);
	}
	let wrong = fails(&leptoskit(
		root,
		&[
			"db",
			"migration",
			"create",
			"wrong_pack",
			"--allow-destructive",
			"--no-renames",
			"--drop-pack",
			"auth",
		],
	));
	assert!(wrong.contains("doesn't lose it"), "{wrong}");
	assert_eq!(migration_files(root), files);
	assert_eq!(database_state(root), before);

	let removed = succeeds(&leptoskit(
		root,
		&[
			"db",
			"migration",
			"create",
			"remove_rpc",
			"--allow-destructive",
			"--no-renames",
			"--drop-pack",
			"rpc",
		],
	));
	assert!(removed.contains("- table rpc.receipts"), "{removed}");
	let after_schema = latest_schema(root);
	assert!(
		after_schema
			.pack_membership()
			.unwrap()
			.iter()
			.all(|pack| pack.name != "rpc")
	);
	assert!(
		after_schema
			.pack_membership()
			.unwrap()
			.iter()
			.any(|pack| pack.name == "live" && pack.builtin)
	);
	for (path, bytes) in &files {
		if let Some(bytes) = bytes
			&& path != Path::new("registry.txt")
		{
			assert_eq!(
				&fs::read(root.join("migrations").join(path)).unwrap(),
				bytes
			);
		}
	}
	succeeds(&leptoskit(root, &["db", "check"]));
	succeeds(&leptoskit(root, &["db", "migrate", "--url", &url]));
	let after = database_state(root);
	assert_eq!(after["rows"]["notes"], before["rows"]["notes"]);
	for table in &rpc.tables {
		assert!(after["rows"].get(table.flat()).is_none());
	}
}
