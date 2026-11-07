//! Prepare exact expansion evidence without opening a database or changing SQL.

use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use anyhow::ensure;
use leptoskit_db::MigrationWindow;
use leptoskit_db_schema::Dialect;

use super::History;

pub(super) fn prepare(
	directory: &Path,
	dialect: Dialect,
	baseline: &str,
	out: &Path,
) -> Result<String> {
	ensure!(
		dialect == Dialect::Postgres,
		"online expansion windows require PostgreSQL"
	);
	let registry = fs::read_to_string(directory.join("registry.txt"))?;
	// Refuse paths before History::read opens any registry-named snapshot.
	for name in leptoskit_db::registry_names(&registry) {
		ensure!(
			!name.is_empty()
				&& name.len() <= 255
				&& name
					.bytes()
					.all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte)),
			"invalid migration name in registry.txt"
		);
	}
	let history = History::read(directory, dialect)?;
	let end = history
		.names
		.iter()
		.position(|name| name == baseline)
		.context("--baseline is not in registry.txt")?
		+ 1;
	ensure!(
		end < history.names.len(),
		"the window needs at least one migration after --baseline"
	);
	let migrations = leptoskit_db::load_migrations(directory)?;
	let read_snapshot = |name: &str| -> Result<String> {
		let path = directory.join(name).join("schema.json");
		fs::read_to_string(&path).with_context(|| format!("couldn't read {}", path.display()))
	};
	let expansions = migrations[end..]
		.iter()
		.cloned()
		.map(|migration| {
			let snapshot = read_snapshot(&migration.name)?;
			Ok((migration, snapshot))
		})
		.collect::<Result<Vec<_>>>()?;
	let window = MigrationWindow::new(&migrations[..end], &read_snapshot(baseline)?, expansions)?;
	let json = window.to_json();
	let digest = leptoskit_db::checksum(&json);
	// Validate everything before opening the output. Persist without replacing
	// an existing plan, including when another process creates it meanwhile.
	let parent = out
		.parent()
		.filter(|path| !path.as_os_str().is_empty())
		.unwrap_or(Path::new("."));
	let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
	temporary.write_all(json.as_bytes())?;
	temporary.as_file().sync_all()?;
	temporary.persist_noclobber(out).with_context(|| {
		format!(
			"couldn't create {}; choose a new output file",
			out.display()
		)
	})?;
	Ok(digest)
}

#[cfg(test)]
mod tests {
	use leptoskit_db_schema::Column;
	use leptoskit_db_schema::ColumnType;
	use leptoskit_db_schema::RequireHints;
	use leptoskit_db_schema::Schema;
	use leptoskit_db_schema::Table;
	use leptoskit_db_schema::TableName;

	use super::*;

	fn fixture(root: &Path) -> std::path::PathBuf {
		let directory = root.join("migrations");
		fs::create_dir(&directory).unwrap();
		let mut schema = Schema::new(
			Dialect::Postgres,
			vec![Table::new(
				TableName::new("notes"),
				vec![Column::new("id", ColumnType::Int64)],
				vec!["id".into()],
			)],
		);
		let mut before = Schema::empty(Dialect::Postgres);
		for name in ["initial", "expanded"] {
			let folder = directory.join(name);
			fs::create_dir(&folder).unwrap();
			let plan = leptoskit_db_schema::diff(&before, &schema, &mut RequireHints).unwrap();
			fs::write(
				folder.join("migration.sql"),
				leptoskit_db_schema::render(&plan, &before, &schema).sql(),
			)
			.unwrap();
			fs::write(folder.join("schema.json"), schema.to_json()).unwrap();
			before = schema.clone();
			schema.tables[0]
				.columns
				.push(Column::new("extra", ColumnType::TEXT).nullable());
		}
		fs::write(directory.join("registry.txt"), "initial\nexpanded\n").unwrap();
		directory
	}

	#[test]
	fn prepares_pinned_evidence_without_mutating_committed_inputs_or_existing_output() {
		let root = tempfile::tempdir().unwrap();
		let directory = fixture(root.path());
		let out = root.path().join("window.json");
		let migration_path = directory.join("expanded/migration.sql");
		let sql = fs::read(&migration_path).unwrap();
		let pin = prepare(&directory, Dialect::Postgres, "initial", &out).unwrap();
		let json = fs::read_to_string(&out).unwrap();
		MigrationWindow::from_json_pinned(&json, &pin).unwrap();
		assert_eq!(fs::read(migration_path).unwrap(), sql);
		assert!(prepare(&directory, Dialect::Postgres, "initial", &out).is_err());
		assert_eq!(fs::read_to_string(out).unwrap(), json);
	}

	#[test]
	fn invalid_sql_or_baseline_never_creates_output() {
		let root = tempfile::tempdir().unwrap();
		let directory = fixture(root.path());
		let out = root.path().join("window.json");
		for baseline in ["missing", "expanded"] {
			assert!(prepare(&directory, Dialect::Postgres, baseline, &out).is_err());
			assert!(!out.exists());
		}
		fs::write(directory.join("expanded/migration.sql"), "SELECT 1;").unwrap();
		assert!(prepare(&directory, Dialect::Postgres, "initial", &out).is_err());
		assert!(!out.exists());
	}
}
