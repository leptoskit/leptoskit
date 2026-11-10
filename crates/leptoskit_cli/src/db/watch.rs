//! The database half of `leptoskit dev`: the development database the app
//! server connects to, and the watch that keeps it in step with the tables.
//!
//! When the tables change, the watch compares them with the latest
//! migration, as `leptoskit db migration create` would. A change that keeps
//! every row becomes a migration, applied to the development database before
//! the app rebuilds when `dev` knows its URL, and by the server as it
//! restarts otherwise. A change that would delete data, or that needs an
//! answer such as whether a column was renamed, writes nothing and says which
//! command to run.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Child;
use std::process::Command;

use anyhow::Context as _;
use anyhow::Result;
use leptoskit_config::DbWatch;
use leptoskit_config::LeptoskitConfig;
use leptoskit_db_schema::RequireHints;
use leptoskit_db_schema::Risk;
use leptoskit_db_schema::Schema;

use super::CreateOptions;
use super::Created;
use super::History;
use super::Project;
use super::StudioOptions;
use super::dev_server;
use super::dev_server::DevServer;
use crate::cli::invoked_name;

/// The database `leptoskit dev` runs the app against.
pub(crate) struct DevDatabase {
	project: Project,
	/// The URL, when `leptoskit dev` knows it: from `db.url_env`, or the
	/// development server's.
	url: Option<String>,
	/// The studio over the database, when `leptoskit dev --studio` asked
	/// for it: declared before the server, so it stops first.
	studio: Option<StudioProcess>,
	/// The development server, when `leptoskit dev` provides one. Held so
	/// that a server it started stops when `dev` does.
	server: Option<DevServer>,
}

impl DevDatabase {
	/// The database for `config`'s project, or `None` when it has no `[db]`.
	/// Starts the development database when `db.url_env` isn't set and the
	/// project uses Postgres.
	pub(crate) fn start(config: &LeptoskitConfig, root: &Path) -> Result<Option<Self>> {
		let Some(project) = Project::of(config, root.to_path_buf()) else {
			return Ok(None);
		};
		let from_env = std::env::var(&project.config.url_env).ok();
		let binaries = dev_server::binaries_from_env();
		Self::open(project, from_env, binaries.as_deref()).map(Some)
	}

	/// The database for `project`: the URL `db.url_env` holds, `from_env`,
	/// when set, and otherwise the development server, if it has one.
	fn open(project: Project, from_env: Option<String>, binaries: Option<&Path>) -> Result<Self> {
		if let Some(url) = from_env {
			println!("[db] using the database {} names", project.config.url_env);
			return Ok(Self {
				project,
				url: Some(url),
				studio: None,
				server: None,
			});
		}

		let Some(source) = dev_server::source(&project) else {
			return Ok(Self {
				project,
				url: None,
				studio: None,
				server: None,
			});
		};
		let server = dev_server::start(&project, source, binaries)?;
		let how = match (source, server.started_here()) {
			(dev_server::Source::Devenv, _) => "devenv's Postgres",
			(dev_server::Source::Embedded, true) => "started Postgres",
			(dev_server::Source::Embedded, false) => "Postgres, already running",
		};
		println!("[db] {how} at {}", server.display_url());

		Ok(Self {
			project,
			url: Some(server.url.clone()),
			studio: None,
			server: Some(server),
		})
	}

	/// Moves to the database of the branch the project is on now, when
	/// `leptoskit dev` runs the development Postgres and the branch changed:
	/// copied from the main branch's database the first time. Returns the app
	/// server's new variables when the database changed, and brings the
	/// branch's database in step with the tables. The studio, when it runs,
	/// starts again over the branch's database.
	pub(crate) fn switch_branch(&mut self) -> Result<Option<Vec<(String, String)>>> {
		let Some(server) = &self.server else {
			return Ok(None);
		};
		let base = server.url.clone();
		let target =
			dev_server::with_database(&base, &super::branches::current_database(&self.project))?;
		let moving = self.url.as_deref() != Some(target.as_str());

		// Postgres copies a database only while nothing is connected to it,
		// and the studio is connected to the one being left.
		if moving && let Some(studio) = &mut self.studio {
			studio.stop();
		}

		let switched = dev_server::with_database(&base, "postgres").and_then(|postgres| {
			let database = super::branches::ensure(&self.project, &postgres)?;
			Ok((dev_server::with_database(&base, &database)?, database))
		});
		let (url, database) = match switched {
			Ok(switched) => switched,
			Err(error) => {
				self.restart_studio();
				return Err(error);
			}
		};

		if self.url.as_deref() == Some(url.as_str()) {
			self.restart_studio();
			return Ok(None);
		}

		println!("[db] switched to the database `{database}`");
		self.url = Some(url);
		self.tables_changed();

		if self.studio.is_some() {
			println!(
				"[db] restarting the studio over `{database}`; open the new address it prints"
			);
			self.restart_studio();
		}

		Ok(Some(self.env()))
	}

	/// Starts the studio over this database, in the background, until `dev`
	/// stops. A branch switch moves it to the branch's database.
	pub(crate) fn start_studio(&mut self, options: StudioOptions) -> Result<()> {
		self.start_studio_with(options, |project, url, options| {
			project.studio(url, 0, options)
		})
	}

	/// [`Self::start_studio`], with `command` serving it over a URL.
	fn start_studio_with(
		&mut self,
		options: StudioOptions,
		command: fn(&Project, &str, StudioOptions) -> Result<Command>,
	) -> Result<()> {
		let Some(url) = &self.url else {
			anyhow::bail!(
				"the studio needs the database's URL: set {}",
				self.project.config.url_env
			);
		};
		let mut studio = StudioProcess {
			options,
			command,
			running: None,
		};
		studio.start(&self.project, url)?;
		self.studio = Some(studio);
		Ok(())
	}

	/// Starts the studio again over the current database, when it runs and
	/// a branch switch stopped it. Problems are printed: the dev loop
	/// carries on.
	fn restart_studio(&mut self) {
		let (Some(studio), Some(url)) = (&mut self.studio, &self.url) else {
			return;
		};

		if studio.running.is_none()
			&& let Err(error) = studio.start(&self.project, url)
		{
			eprintln!("[db] couldn't restart the studio: {error:#}");
		}
	}

	/// The variables the app server needs to find the database: the URL,
	/// when `leptoskit dev` provides it.
	pub(crate) fn env(&self) -> Vec<(String, String)> {
		match (&self.server, &self.url) {
			(Some(_), Some(url)) => vec![(self.project.config.url_env.clone(), url.clone())],
			_ => Vec::new(),
		}
	}

	/// The directory of the package that declares the tables, whose changes
	/// [`Self::tables_changed`] follows.
	pub(crate) fn package_dir(&self, metadata: &crate::wasm::CargoMetadata) -> Option<PathBuf> {
		metadata
			.packages
			.iter()
			.find(|package| package.name == self.project.package)
			.and_then(|package| package.manifest_path.parent())
			.map(Path::to_path_buf)
	}

	/// Brings the development database in step with the tables, as
	/// `db.watch` says. Problems are printed: the dev loop carries on.
	pub(crate) fn tables_changed(&self) {
		if self.project.config.watch == DbWatch::Off {
			return;
		}

		if let Err(error) = self.sync() {
			eprintln!("[db] {error:#}");
		}
	}

	fn sync(&self) -> Result<()> {
		self.sync_to(&self.project.export()?)
	}

	/// Brings the development database in step with `target`.
	fn sync_to(&self, target: &Schema) -> Result<()> {
		if self.project.config.watch == DbWatch::Push {
			let Some(url) = &self.url else {
				anyhow::bail!(
					"push mode changes the development database, but `{} dev` doesn't know its \
					 URL; set {}",
					invoked_name(),
					self.project.config.url_env
				);
			};

			if let Some(message) = super::push::push(&self.project, url, target, false)?.message() {
				print!("{message}");
			}

			return Ok(());
		}

		let history = History::read(&self.project.migrations_dir(), self.project.dialect())?;
		let change = classify(&history, target);

		if let Some(message) = change.message(self.project.config.watch) {
			print!("{message}");
		}

		if let (Change::Keeps(_), DbWatch::Migrate) = (&change, self.project.config.watch) {
			let created = create_dev_migration(&self.project, target)?;
			println!(
				"[db] wrote migration {}; review it, then commit it with schema.json and \
				 registry.txt",
				created.name
			);

			if let Some(url) = &self.url
				&& let Err(error) = apply(&self.project, url)
			{
				discard(&self.project, &created)?;
				anyhow::bail!(
					"the migration didn't apply, so it was removed again: {error:#}\nfix the \
					 tables, or the rows that break the new rule"
				);
			}
		}

		Ok(())
	}
}

/// How the tables differ from the latest migration.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Change {
	/// Not at all.
	Unchanged,
	/// In ways that keep every row: these steps.
	Keeps(Vec<String>),
	/// In ways that delete data: every step, and why the risky ones are.
	Deletes(Vec<String>),
	/// In a way that needs an answer first, such as whether a column was
	/// renamed.
	Unresolved(String),
}

impl Change {
	/// What to tell the developer, under `watch`.
	pub(crate) fn message(&self, watch: DbWatch) -> Option<String> {
		let command = invoked_name();
		let mut message = String::new();

		match self {
			Self::Unchanged => return None,
			Self::Keeps(steps) => {
				let _ = writeln!(
					message,
					"[db] the tables changed since the latest migration:"
				);
				write_steps(&mut message, steps);

				if watch == DbWatch::Report {
					let _ = writeln!(
						message,
						"[db] run `{command} db migration create <name>` to write the migration"
					);
				}
			}
			Self::Deletes(steps) => {
				let _ = writeln!(
					message,
					"[db] the tables changed in a way that deletes data, so nothing was written \
					 or applied:"
				);
				write_steps(&mut message, steps);
				let _ = writeln!(
					message,
					"[db] if that's intended, run `{command} db migration create <name> \
					 --allow-destructive`; if a column or table was renamed, mark it with \
					 #[db(renamed_from = \"...\")]"
				);
			}
			Self::Unresolved(error) => {
				let _ = writeln!(
					message,
					"[db] the tables changed, but nothing was written or applied: {error}"
				);
			}
		}

		Some(message)
	}
}

fn write_steps(message: &mut String, steps: &[String]) {
	for step in steps {
		let _ = writeln!(message, "  {step}");
	}
}

/// How `target` differs from the latest migration's snapshot. Renames count
/// only when `#[db(renamed_from = …)]` marks them: the dev loop never asks.
pub(crate) fn classify(history: &History, target: &Schema) -> Change {
	if target.without_rename_hints() == history.latest {
		return Change::Unchanged;
	}

	let plan = match leptoskit_db_schema::diff(&history.latest, target, &mut RequireHints) {
		Ok(plan) if plan.is_empty() => return Change::Unchanged,
		Ok(plan) => plan,
		Err(error) => return Change::Unresolved(error.to_string()),
	};
	let deletes = plan.destructive_steps().next().is_some();
	let steps = plan
		.steps
		.iter()
		.map(|step| {
			match step.risk() {
				Risk::LosesData(reason) if deletes => format!("{step}: {reason}"),
				_ => step.to_string(),
			}
		})
		.collect();

	if deletes {
		Change::Deletes(steps)
	} else {
		Change::Keeps(steps)
	}
}

/// Writes the migration for changes that keep every row, named after its
/// first step.
fn create_dev_migration(project: &Project, target: &Schema) -> Result<Created> {
	let history = History::read(&project.migrations_dir(), project.dialect())?;
	let plan = leptoskit_db_schema::diff(&history.latest, target, &mut RequireHints)?;
	let mut name = super::slug(&plan.steps[0].to_string())?;
	name.truncate(40);

	if plan.steps.len() > 1 {
		name.push_str("_and_more");
	}

	let options = CreateOptions {
		allow_destructive: false,
		empty: false,
	};
	super::create(
		project,
		name.trim_end_matches('_'),
		target,
		&options,
		&mut RequireHints,
	)
}

/// Applies every pending migration to the database at `url`.
fn apply(project: &Project, url: &str) -> Result<()> {
	let migrations = leptoskit_db::load_migrations(&project.migrations_dir())?;
	let applied = super::runtime()?.block_on(async {
		let db = leptoskit_db::Db::connect(url).await?;
		anyhow::Ok(db.migrate(&migrations).await?)
	})?;

	for name in applied {
		println!("[db] applied {name} to the development database");
	}

	Ok(())
}

/// Removes a migration this watch wrote and couldn't apply: its directory
/// and its line in `registry.txt`.
pub(crate) fn discard(project: &Project, created: &Created) -> Result<()> {
	fs::remove_dir_all(&created.directory)?;
	let registry = project.migrations_dir().join("registry.txt");
	let text = fs::read_to_string(&registry)?;
	let kept: String = text
		.lines()
		.filter(|line| line.trim() != created.name)
		.flat_map(|line| [line, "\n"])
		.collect();
	fs::write(&registry, kept)?;
	Ok(())
}

#[cfg(test)]
mod tests {
	use leptoskit_config::DbConfig;
	use leptoskit_config::DbDialect;
	use leptoskit_db_schema::Column;
	use leptoskit_db_schema::ColumnType;
	use leptoskit_db_schema::Dialect;
	use leptoskit_db_schema::Index;
	use leptoskit_db_schema::Table;
	use leptoskit_db_schema::TableName;

	use super::*;

	fn project(root: &Path, watch: DbWatch) -> Project {
		Project {
			root: root.to_path_buf(),
			config: DbConfig {
				dialect: DbDialect::Sqlite,
				watch,
				..DbConfig::default()
			},
			package: "app_db".into(),
			name: "app".into(),
		}
	}

	fn notes(columns: Vec<Column>) -> Schema {
		let name = TableName::new("notes");
		let mut all = vec![Column::new("id", ColumnType::Uuid)];
		all.extend(columns);
		let mut table = Table::new(name.clone(), all, vec!["id".into()]);
		table.indexes.push(Index::new(&name, &["id"]));
		Schema::new(Dialect::Sqlite, vec![table])
	}

	fn history(project: &Project) -> History {
		History::read(&project.migrations_dir(), Dialect::Sqlite).unwrap()
	}

	#[test]
	fn changes_are_sorted_by_what_they_do_to_rows() {
		let root = tempfile::tempdir().unwrap();
		let project = project(root.path(), DbWatch::Migrate);
		let first = notes(vec![Column::new("body", ColumnType::TEXT)]);
		assert_eq!(
			classify(&history(&project), &first),
			Change::Keeps(vec!["+ table notes (2 columns)".into()])
		);

		create_dev_migration(&project, &first).unwrap();
		assert_eq!(classify(&history(&project), &first), Change::Unchanged);

		let pinned = notes(vec![
			Column::new("body", ColumnType::TEXT),
			Column::new("pinned", ColumnType::Bool).nullable(),
		]);
		assert_eq!(
			classify(&history(&project), &pinned),
			Change::Keeps(vec!["+ column notes.pinned (bool)".into()])
		);

		let without_body = notes(Vec::new());
		let Change::Deletes(steps) = classify(&history(&project), &without_body) else {
			panic!("dropping a column deletes data");
		};
		assert_eq!(
			steps,
			["- column notes.body: every value in `notes.body` is deleted"]
		);

		let renamed = notes(vec![Column::new("text", ColumnType::TEXT)]);
		let Change::Unresolved(error) = classify(&history(&project), &renamed) else {
			panic!("a possible rename needs an answer");
		};
		assert!(error.contains("renamed_from"), "{error}");
	}

	#[test]
	fn messages_say_what_happens_next() {
		let keeps = Change::Keeps(vec!["+ column notes.pinned (bool)".into()]);
		let reported = keeps.message(DbWatch::Report).unwrap();
		assert!(reported.contains("+ column notes.pinned"), "{reported}");
		assert!(
			reported.contains("db migration create <name>"),
			"{reported}"
		);
		assert!(
			!keeps
				.message(DbWatch::Migrate)
				.unwrap()
				.contains("migration create")
		);

		let deletes = Change::Deletes(vec!["- column notes.body: gone".into()]);
		let message = deletes.message(DbWatch::Migrate).unwrap();
		assert!(
			message.contains("nothing was written or applied"),
			"{message}"
		);
		assert!(message.contains("--allow-destructive"), "{message}");
		assert!(
			Change::Unresolved("which?".into())
				.message(DbWatch::Migrate)
				.unwrap()
				.ends_with("nothing was written or applied: which?\n")
		);
		assert_eq!(Change::Unchanged.message(DbWatch::Report), None);
	}

	#[test]
	fn dev_migrations_apply_and_are_removed_when_they_cannot() {
		let root = tempfile::tempdir().unwrap();
		let project = project(root.path(), DbWatch::Migrate);
		let database = root.path().join("dev.sqlite3");
		let url = format!("sqlite://{}", database.display());

		let first = notes(vec![Column::new("body", ColumnType::TEXT)]);
		let created = create_dev_migration(&project, &first).unwrap();
		assert!(
			created.name.ends_with("_table_notes_2_columns"),
			"{}",
			created.name
		);
		apply(&project, &url).unwrap();

		// A step the database refuses: a second table with the same name.
		std::thread::sleep(std::time::Duration::from_secs(1));
		let mut grown = notes(vec![
			Column::new("body", ColumnType::TEXT),
			Column::new("pinned", ColumnType::Bool).nullable(),
			Column::new("rank", ColumnType::Int32).nullable(),
		]);
		let created = create_dev_migration(&project, &grown).unwrap();
		assert!(created.name.ends_with("_and_more"), "{}", created.name);
		fs::write(
			created.directory.join("migration.sql"),
			"CREATE TABLE \"notes\" (\"id\" BLOB);\n",
		)
		.unwrap();
		assert!(apply(&project, &url).is_err());
		discard(&project, &created).unwrap();
		assert!(!created.directory.exists());
		assert_eq!(history(&project).names.len(), 1);

		// Without the bad SQL it applies.
		grown.tables[0].columns.pop();
		std::thread::sleep(std::time::Duration::from_secs(1));
		create_dev_migration(&project, &grown).unwrap();
		apply(&project, &url).unwrap();
		assert_eq!(classify(&history(&project), &grown), Change::Unchanged);
	}

	#[test]
	fn the_dev_loop_finds_its_database() {
		let root = tempfile::tempdir().unwrap();
		let none = DevDatabase::start(&LeptoskitConfig::default(), root.path()).unwrap();
		assert!(none.is_none(), "no [db], no database");

		let sqlite = DevDatabase::open(project(root.path(), DbWatch::Migrate), None, None).unwrap();
		assert!(sqlite.url.is_none() && sqlite.env().is_empty());

		let mut postgres = project(root.path(), DbWatch::Migrate);
		postgres.config.dialect = DbDialect::Postgres;
		let named =
			DevDatabase::open(postgres, Some("postgres://elsewhere/app".into()), None).unwrap();
		assert_eq!(named.url.as_deref(), Some("postgres://elsewhere/app"));
		assert!(
			named.env().is_empty(),
			"the server reads the variable itself"
		);

		let metadata: crate::wasm::CargoMetadata = serde_json::from_value(serde_json::json!({
			"packages": [{
				"name": "app_db",
				"manifest_path": root.path().join("app_db/Cargo.toml"),
				"targets": [],
			}],
			"target_directory": root.path().join("target"),
			"workspace_root": root.path(),
		}))
		.unwrap();
		assert_eq!(
			named.package_dir(&metadata),
			Some(root.path().join("app_db"))
		);

		let Some(binaries) = dev_server::test_binaries() else {
			return;
		};
		let mut embedded = project(root.path(), DbWatch::Migrate);
		embedded.config.dialect = DbDialect::Postgres;
		embedded.config.dev_server = leptoskit_config::DbDevServer::Embedded;
		let started = DevDatabase::open(embedded, None, Some(&binaries)).unwrap();
		let env = started.env();
		assert_eq!(env.len(), 1);
		assert_eq!(env[0].0, "DATABASE_URL");
		assert!(env[0].1.contains("/app?"), "{}", env[0].1);
	}

	#[test]
	fn the_studio_needs_the_databases_url() {
		let root = tempfile::tempdir().unwrap();
		let mut sqlite =
			DevDatabase::open(project(root.path(), DbWatch::Migrate), None, None).unwrap();
		let error = sqlite.start_studio(StudioOptions::default()).err().unwrap();
		assert!(
			error
				.to_string()
				.contains("the studio needs the database's URL: set DATABASE_URL"),
			"{error}"
		);

		// With a URL, it starts cargo in the project, and stops it when
		// dropped; this project has no workspace, so cargo metadata fails
		// before anything runs.
		let mut named = DevDatabase::open(
			project(root.path(), DbWatch::Migrate),
			Some("sqlite::memory:".into()),
			None,
		)
		.unwrap();
		assert!(named.start_studio(StudioOptions::default()).is_err());
		assert!(named.studio.is_none());
	}

	#[test]
	fn the_watch_writes_applies_and_refuses() {
		let root = tempfile::tempdir().unwrap();
		let url = format!("sqlite://{}", root.path().join("dev.sqlite3").display());
		let mut database = DevDatabase {
			project: project(root.path(), DbWatch::Migrate),
			url: Some(url.clone()),
			studio: None,
			server: None,
		};
		let migrations = |database: &DevDatabase| history(&database.project).names.len();

		let first = notes(vec![Column::new("body", ColumnType::TEXT)]);
		database.sync_to(&first).unwrap();
		assert_eq!(migrations(&database), 1);
		database.sync_to(&first).unwrap();
		assert_eq!(migrations(&database), 1, "nothing changed");

		// Deleting data, or a question, writes nothing.
		database.sync_to(&notes(Vec::new())).unwrap();
		database
			.sync_to(&notes(vec![Column::new("text", ColumnType::TEXT)]))
			.unwrap();
		assert_eq!(migrations(&database), 1);

		// A migration the database refuses is removed again.
		let with_tags = |name: &str| {
			let mut schema = first.clone();
			let table_name = TableName::new(name);
			let mut table = Table::new(
				table_name.clone(),
				vec![Column::new("id", ColumnType::Uuid)],
				vec!["id".into()],
			);
			table.indexes.push(Index::new(&table_name, &["id"]));
			schema.tables.push(table);
			schema
		};
		super::super::runtime()
			.unwrap()
			.block_on(async {
				let db = leptoskit_db::Db::connect(&url).await?;
				db.run_script("CREATE TABLE \"tags\" (\"id\" BLOB)").await
			})
			.unwrap();
		std::thread::sleep(std::time::Duration::from_secs(1));
		let error = database.sync_to(&with_tags("tags")).unwrap_err();
		assert!(error.to_string().contains("removed again"), "{error}");
		assert_eq!(migrations(&database), 1);

		// Reporting only prints; off ignores the tables altogether.
		database.project.config.watch = DbWatch::Report;
		database.sync_to(&with_tags("labels")).unwrap();
		assert_eq!(migrations(&database), 1);
		database.project.config.watch = DbWatch::Off;
		database.tables_changed();

		// Without its db crate, the export fails, and the loop carries on.
		database.project.config.watch = DbWatch::Migrate;
		database.tables_changed();
		assert_eq!(migrations(&database), 1);

		// Without a URL, the migration waits for the server to apply it.
		database.url = None;
		std::thread::sleep(std::time::Duration::from_secs(1));
		database.sync_to(&with_tags("labels")).unwrap();
		assert_eq!(migrations(&database), 2);
	}
}

#[cfg(test)]
mod push_tests {
	use leptoskit_config::DbConfig;
	use leptoskit_config::DbDialect;
	use leptoskit_db_schema::Column;
	use leptoskit_db_schema::ColumnType;
	use leptoskit_db_schema::Dialect;
	use leptoskit_db_schema::Table;
	use leptoskit_db_schema::TableName;

	use super::*;

	#[test]
	fn the_dev_loop_moves_to_each_branch_s_database() {
		let git = |root: &Path, args: &[&str]| {
			let status = crate::git::command(root)
				.args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
				.args(args)
				.env("GIT_CONFIG_GLOBAL", root.join("no-global-config"))
				.env("GIT_CONFIG_NOSYSTEM", "1")
				.stdout(std::process::Stdio::null())
				.stderr(std::process::Stdio::null())
				.status()
				.unwrap();
			assert!(status.success(), "git {args:?}");
		};
		let root = tempfile::tempdir().unwrap();
		let project = Project {
			root: root.path().to_path_buf(),
			config: DbConfig {
				dialect: DbDialect::Sqlite,
				watch: DbWatch::Off,
				..DbConfig::default()
			},
			package: "app_db".into(),
			name: "app".into(),
		};
		let mut without_server = DevDatabase::open(project, None, None).unwrap();
		assert_eq!(without_server.switch_branch().unwrap(), None);

		let Some(binaries) = dev_server::test_binaries() else {
			return;
		};
		git(root.path(), &["init", "--quiet", "--initial-branch=main"]);
		git(
			root.path(),
			&["commit", "--quiet", "--allow-empty", "-m", "first"],
		);
		let project = Project {
			root: root.path().to_path_buf(),
			config: DbConfig {
				dialect: DbDialect::Postgres,
				dev_server: leptoskit_config::DbDevServer::Embedded,
				watch: DbWatch::Off,
				..DbConfig::default()
			},
			package: "app_db".into(),
			name: "app".into(),
		};
		let mut database = DevDatabase::open(project, None, Some(&binaries)).unwrap();
		assert!(database.env()[0].1.contains("/app?"));
		assert_eq!(database.switch_branch().unwrap(), None, "still on main");

		git(root.path(), &["checkout", "--quiet", "-b", "feature"]);
		let env = database.switch_branch().unwrap().unwrap();
		assert!(env[0].1.contains("/app__feature?"), "{env:?}");
		assert_eq!(database.switch_branch().unwrap(), None);

		git(root.path(), &["checkout", "--quiet", "main"]);
		let env = database.switch_branch().unwrap().unwrap();
		assert!(env[0].1.contains("/app?"), "{env:?}");
	}

	/// Stands in for the studio: `psql`, connected to the database the
	/// studio would serve and idle until it's stopped, as the studio's own
	/// connections are.
	fn stand_in(_: &Project, url: &str, _: StudioOptions) -> Result<Command> {
		let binaries = dev_server::test_binaries().context("no Postgres binaries")?;
		let mut command =
			Command::new(binaries.join(format!("psql{}", std::env::consts::EXE_SUFFIX)));
		command
			.args(["--no-psqlrc", "--quiet", url])
			.stdin(std::process::Stdio::piped())
			.stdout(std::process::Stdio::null());
		Ok(command)
	}

	/// Waits until something holds a connection to `database` on the
	/// server at `url`.
	fn wait_for_connection(url: &str, database: &str) {
		let postgres = dev_server::with_database(url, "postgres").unwrap();

		for _ in 0..100 {
			let connections = super::super::runtime()
				.unwrap()
				.block_on(async {
					let db = leptoskit_db::Db::connect(&postgres).await?;
					leptoskit_db::Read::fetch(
						&db,
						leptoskit_db::Statement {
							sql: "SELECT 1 FROM pg_catalog.pg_stat_activity WHERE datname = $1 \
							      AND application_name = 'psql'"
								.to_owned(),
							args: vec![leptoskit_db::Value::Text(database.to_owned())],
						},
					)
					.await
				})
				.unwrap();

			if !connections.is_empty() {
				return;
			}

			std::thread::sleep(std::time::Duration::from_millis(100));
		}

		panic!("nothing connected to {database}");
	}

	#[test]
	fn the_studio_follows_each_branch_s_database() {
		let git = |root: &Path, args: &[&str]| {
			let status = crate::git::command(root)
				.args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
				.args(args)
				.env("GIT_CONFIG_GLOBAL", root.join("no-global-config"))
				.env("GIT_CONFIG_NOSYSTEM", "1")
				.stdout(std::process::Stdio::null())
				.stderr(std::process::Stdio::null())
				.status()
				.unwrap();
			assert!(status.success(), "git {args:?}");
		};
		let Some(binaries) = dev_server::test_binaries() else {
			return;
		};
		let root = tempfile::tempdir().unwrap();
		git(root.path(), &["init", "--quiet", "--initial-branch=main"]);
		git(
			root.path(),
			&["commit", "--quiet", "--allow-empty", "-m", "first"],
		);
		let project = Project {
			root: root.path().to_path_buf(),
			config: DbConfig {
				dialect: DbDialect::Postgres,
				dev_server: leptoskit_config::DbDevServer::Embedded,
				watch: DbWatch::Off,
				..DbConfig::default()
			},
			package: "app_db".into(),
			name: "app".into(),
		};
		let mut database = DevDatabase::open(project, None, Some(&binaries)).unwrap();
		database
			.start_studio_with(StudioOptions::default(), stand_in)
			.unwrap();
		let serving = |database: &DevDatabase| {
			let studio = database.studio.as_ref().unwrap();
			let (url, child) = studio.running.as_ref().unwrap();
			(url.clone(), child.id())
		};
		let (url, first) = serving(&database);
		assert!(url.contains("/app?"), "{url}");
		wait_for_connection(&url, "app");

		// Staying on the branch leaves the studio running as it was.
		assert_eq!(database.switch_branch().unwrap(), None);
		assert_eq!(serving(&database), (url.clone(), first));

		// Postgres copies the main branch's database for a new branch only
		// while nothing is connected to it, so the studio stops first, then
		// starts again over the copy.
		git(root.path(), &["checkout", "--quiet", "-b", "feature"]);
		let env = database.switch_branch().unwrap().unwrap();
		assert!(env[0].1.contains("/app__feature?"), "{env:?}");
		let (url, second) = serving(&database);
		assert_eq!(url, env[0].1);
		assert_ne!(second, first, "a new process");
		wait_for_connection(&url, "app__feature");

		git(root.path(), &["checkout", "--quiet", "main"]);
		database.switch_branch().unwrap().unwrap();
		let (url, third) = serving(&database);
		assert!(url.contains("/app?"), "{url}");
		assert_ne!(third, second);
	}

	#[test]
	fn push_mode_changes_the_database_and_writes_no_migration() {
		let root = tempfile::tempdir().unwrap();
		let project = Project {
			root: root.path().to_path_buf(),
			config: DbConfig {
				dialect: DbDialect::Sqlite,
				watch: DbWatch::Push,
				..DbConfig::default()
			},
			package: "app_db".into(),
			name: "app".into(),
		};
		let url = format!("sqlite://{}", root.path().join("dev.sqlite3").display());
		let mut database = DevDatabase {
			project,
			url: None,
			studio: None,
			server: None,
		};
		let notes = Schema::new(
			Dialect::Sqlite,
			vec![Table::new(
				TableName::new("notes"),
				vec![Column::new("id", ColumnType::Int64)],
				vec!["id".into()],
			)],
		);

		let error = database.sync_to(&notes).unwrap_err();
		assert!(
			error.to_string().contains("doesn't know its URL"),
			"{error}"
		);

		database.url = Some(url.clone());
		database.sync_to(&notes).unwrap();
		database.sync_to(&notes).unwrap();
		let history = History::read(&database.project.migrations_dir(), Dialect::Sqlite).unwrap();
		assert!(history.names.is_empty(), "pushes write no migration");
		let pushed = super::super::runtime()
			.unwrap()
			.block_on(async {
				let db = leptoskit_db::Db::connect(&url).await?;
				db.pushed_schema().await
			})
			.unwrap()
			.unwrap();
		assert_eq!(Schema::from_json(&pushed).unwrap(), notes);
	}
}

/// The studio `leptoskit dev --studio` runs beside the app, stopped when
/// `dev` stops, and before a branch switch moves its database.
pub(crate) struct StudioProcess {
	options: StudioOptions,
	/// Builds the command that serves it over a database's URL:
	/// [`Project::studio`], or a stand-in in tests.
	command: fn(&Project, &str, StudioOptions) -> Result<Command>,
	/// The database it serves, and the process serving it, while it runs.
	running: Option<(String, Child)>,
}

impl StudioProcess {
	fn start(&mut self, project: &Project, url: &str) -> Result<()> {
		println!("[db] building the studio into {}", project.package);
		let child = (self.command)(project, url, self.options)?
			.spawn()
			.with_context(|| format!("couldn't run cargo for {}", project.package))?;
		self.running = Some((url.to_owned(), child));
		Ok(())
	}

	fn stop(&mut self) {
		if let Some((_, mut child)) = self.running.take() {
			let _ = child.kill();
			let _ = child.wait();
		}
	}
}

impl Drop for StudioProcess {
	fn drop(&mut self) {
		self.stop();
	}
}
