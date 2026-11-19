//! The migrations themselves, on both engines: every `down.sql` reverses
//! its migration, a rollback leaves nothing behind that blocks applying
//! again, each migration reverses and reapplies on its own — a failure
//! names the migration — and the data backfill inside
//! `persons_and_segments` registers exactly the people the older tables
//! already knew.
//!
//! The migrations are discovered from each directory's `registry.txt`, so
//! a migration added later is covered without editing this file. Rollback
//! runs through the database crate's own `Db::rollback`, which executes
//! the embedded `down.sql`; the test first checks that what is embedded is
//! what the registry and the files on disk say.
//!
//! The Postgres halves start a private Postgres from local binaries and
//! skip without them, unless `LEPTOSKIT_REQUIRE_POSTGRES` is set, as
//! `tests/postgres.rs` does.

#![cfg(not(target_arch = "wasm32"))]

use std::collections::BTreeSet;
use std::path::Path;

use leptoskit_db::Db;
use leptoskit_db::Dialect;
use leptoskit_db::Migration;
use leptoskit_db::Read as _;
use leptoskit_db::Statement;
use leptoskit_db::Value;
use uuid::Uuid;

#[path = "support/sql.rs"]
mod sql;
use sql::Sql;

#[cfg(feature = "postgres")]
#[path = "support/postgres.rs"]
mod postgres;

/// One engine's migrations: where they live and what the crate ships.
struct Engine {
	dialect: Dialect,
	directory: &'static str,
	shipped: &'static [Migration],
}

fn sqlite() -> Engine {
	Engine {
		dialect: Dialect::Sqlite,
		directory: "migrations",
		shipped: leptoskit_notifications::MIGRATIONS,
	}
}

#[cfg(feature = "postgres")]
fn postgres_engine() -> Engine {
	Engine {
		dialect: Dialect::Postgres,
		directory: "migrations-postgres",
		shipped: leptoskit_notifications::POSTGRES_MIGRATIONS,
	}
}

impl Engine {
	/// Every migration the registry lists, in registry order, read from
	/// the files on disk. It checks first that the registry and the
	/// directory agree, that the crate embeds exactly these migrations,
	/// and that every one of them has a reviewed `down.sql`, so a rollback
	/// can't quietly stop short.
	fn discover(&self) -> Vec<Migration> {
		let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join(self.directory);
		let found = leptoskit_db::load_migrations(&directory).expect("the registry loads");

		assert!(
			!found.is_empty(),
			"{} lists no migrations",
			directory.display()
		);

		let on_disk: BTreeSet<String> = std::fs::read_dir(&directory)
			.expect("the migrations directory reads")
			.filter_map(Result::ok)
			.filter(|entry| entry.path().is_dir())
			.map(|entry| entry.file_name().to_string_lossy().into_owned())
			.collect();
		let listed: BTreeSet<String> = found.iter().map(|m| m.name.to_string()).collect();
		assert_eq!(
			on_disk, listed,
			"every migration directory is in registry.txt, and every entry has a directory"
		);

		for migration in &found {
			assert!(
				migration
					.down
					.as_deref()
					.is_some_and(|down| !down.trim().is_empty()),
				"`{}` has no down.sql",
				migration.name
			);
		}

		assert_eq!(
			found, self.shipped,
			"the embedded migrations are the registry's, down.sql included"
		);

		found
	}

	async fn database(&self) -> Db {
		assert_eq!(self.dialect, Dialect::Sqlite, "Postgres brings its own");

		Db::connect("sqlite::memory:")
			.await
			.expect("SQLite opens an in-memory database")
	}
}

fn names(migrations: &[Migration]) -> Vec<String> {
	migrations.iter().map(|m| m.name.to_string()).collect()
}

/// What the service's migrations leave in a database, as sorted lines:
/// every table, index, trigger, sequence, function, and type, with its
/// definition — columns, defaults, constraints, comments — so that "the
/// same schema" is compared by what the schema says, not only by name. The
/// database crate's own bookkeeping (`_leptoskit_*`) is left out. Postgres
/// schemas themselves are left out too: `init` creates `live` and `rpc`
/// with `IF NOT EXISTS`, and its rollback drops each only when it is left
/// empty, which has its own check below; what lives inside them is
/// compared.
async fn snapshot(db: &Db) -> Vec<String> {
	let (queries, separator): (&[&str], &str) = match db.dialect() {
		Dialect::Sqlite => (SQLITE_CATALOG, ""),
		Dialect::Postgres => (POSTGRES_CATALOG, " "),
	};
	let mut lines = Vec::new();

	for query in queries {
		let rows = db
			.fetch(Statement {
				sql: (*query).into(),
				args: Vec::new(),
			})
			.await
			.unwrap_or_else(|error| panic!("the catalog read failed: {error}\n{query}"));

		for row in rows {
			let Some(Value::Text(line)) = row.first() else {
				panic!("a catalog row is one text column: {row:?}");
			};

			// SQLite keeps a table's SQL as it was written, and the
			// reverse migrations are formatted differently from the
			// forward ones, so whitespace is not a difference.
			lines.push(line.split_whitespace().collect::<Vec<_>>().join(separator));
		}
	}

	lines.sort();
	lines
}

/// SQLite: one line per schema object, with the SQL it was made from.
const SQLITE_CATALOG: &[&str] = &[
	"SELECT type || ' ' || name || ' ON ' || tbl_name || ': ' || coalesce(sql, '') FROM \
	 sqlite_master WHERE name NOT LIKE 'sqlite!_%' ESCAPE '!' AND name NOT LIKE '!_leptoskit!_%' \
	 ESCAPE '!'",
];

/// Postgres: the same, from the catalogs, outside the system schemas and
/// the database crate's own tables.
const POSTGRES_CATALOG: &[&str] = &[
	// Relations: tables, indexes, sequences, views, and their comments.
	"SELECT c.relkind::text || ' ' || n.nspname || '.' || c.relname || ' // ' || \
	 coalesce(obj_description(c.oid, 'pg_class'), '') FROM pg_class c JOIN pg_namespace n ON \
	 n.oid = c.relnamespace WHERE n.nspname NOT IN ('pg_catalog', 'information_schema') AND \
	 n.nspname NOT LIKE 'pg\\_toast%' AND n.nspname NOT LIKE 'pg\\_temp%' AND c.relname NOT LIKE \
	 '\\_leptoskit\\_%'",
	// Columns, in order, with type, nullability, default, and comment.
	"SELECT n.nspname || '.' || c.relname || ' #' || lpad((row_number() OVER (PARTITION BY \
	 a.attrelid ORDER BY a.attnum))::text, 3, '0') || ' ' || a.attname || ' ' || \
	 format_type(a.atttypid, a.atttypmod) || ' notnull=' || a.attnotnull::text || ' default=' || \
	 coalesce(pg_get_expr(d.adbin, d.adrelid), '') || ' // ' || coalesce(col_description(c.oid, \
	 a.attnum), '') FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid JOIN pg_namespace n \
	 ON n.oid = c.relnamespace LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = \
	 a.attnum WHERE a.attnum > 0 AND NOT a.attisdropped AND c.relkind IN ('r', 'p') AND n.nspname \
	 NOT IN ('pg_catalog', 'information_schema') AND n.nspname NOT LIKE 'pg\\_toast%' AND \
	 n.nspname NOT LIKE 'pg\\_temp%' AND c.relname NOT LIKE '\\_leptoskit\\_%'",
	// Constraints.
	"SELECT n.nspname || '.' || c.relname || ' ' || k.conname || ' ' || \
	 pg_get_constraintdef(k.oid) FROM pg_constraint k JOIN pg_class c ON c.oid = k.conrelid JOIN \
	 pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname NOT IN ('pg_catalog', \
	 'information_schema') AND n.nspname NOT LIKE 'pg\\_toast%' AND n.nspname NOT LIKE \
	 'pg\\_temp%' AND c.relname NOT LIKE '\\_leptoskit\\_%'",
	// Indexes, as Postgres would recreate them.
	"SELECT schemaname || '.' || indexname || ' ' || indexdef FROM pg_indexes WHERE schemaname \
	 NOT IN ('pg_catalog', 'information_schema') AND tablename NOT LIKE '\\_leptoskit\\_%'",
	// Triggers.
	"SELECT n.nspname || '.' || c.relname || ' ' || t.tgname || ' ' || pg_get_triggerdef(t.oid) \
	 FROM pg_trigger t JOIN pg_class c ON c.oid = t.tgrelid JOIN pg_namespace n ON n.oid = \
	 c.relnamespace WHERE NOT t.tgisinternal AND n.nspname NOT IN ('pg_catalog', \
	 'information_schema') AND c.relname NOT LIKE '\\_leptoskit\\_%'",
	// Functions, with a digest of their bodies.
	"SELECT 'function ' || n.nspname || '.' || p.proname || '(' || \
	 pg_get_function_identity_arguments(p.oid) || ') ' || md5(pg_get_functiondef(p.oid)) FROM \
	 pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace WHERE n.nspname NOT IN \
	 ('pg_catalog', 'information_schema') AND n.nspname NOT LIKE 'pg\\_toast%'",
	// Enums and domains.
	"SELECT 'type ' || n.nspname || '.' || t.typname FROM pg_type t JOIN pg_namespace n ON n.oid \
	 = t.typnamespace WHERE t.typtype IN ('e', 'd') AND n.nspname NOT IN ('pg_catalog', \
	 'information_schema') AND n.nspname NOT LIKE 'pg\\_toast%'",
];

/// Panics, naming `context` and the lines that differ, unless `actual`
/// is `expected`.
fn assert_same(expected: &[String], actual: &[String], context: &str) {
	let missing: Vec<&String> = expected.iter().filter(|l| !actual.contains(l)).collect();
	let extra: Vec<&String> = actual.iter().filter(|l| !expected.contains(l)).collect();

	assert!(
		missing.is_empty() && extra.is_empty(),
		"{context}\nmissing from the schema:\n{missing:#?}\nleft behind:\n{extra:#?}"
	);
}

/// Every migration applies in registry order, every `down.sql` applies in
/// reverse order, and the database holds none of the service's tables or
/// indexes afterwards.
async fn full_rollback_leaves_nothing(engine: &Engine, db: &Db) {
	let migrations = engine.discover();
	let names = names(&migrations);

	let applied = db
		.migrate(&migrations)
		.await
		.expect("every migration applies to an empty database");
	assert_eq!(applied, names, "applied in registry order");
	assert!(
		!snapshot(db).await.is_empty(),
		"the migrations build something to roll back"
	);

	let reversed = db
		.rollback(&migrations, migrations.len())
		.await
		.expect("every down.sql applies, newest first");
	assert_eq!(
		reversed,
		names.iter().rev().cloned().collect::<Vec<_>>(),
		"reversed in the opposite order"
	);

	let left = snapshot(db).await;
	assert!(
		left.is_empty(),
		"a full rollback leaves no table, index, trigger, sequence, or function: {left:#?}"
	);
}

/// A rollback leaves nothing that blocks applying again, and what applies
/// again is the same schema, down to defaults, constraints, and comments —
/// twice over.
async fn reapply_after_full_rollback(engine: &Engine, db: &Db) {
	let migrations = engine.discover();
	let names = names(&migrations);

	db.migrate(&migrations).await.expect("the first apply");
	let built = snapshot(db).await;

	for round in 1..=2 {
		db.rollback(&migrations, migrations.len())
			.await
			.unwrap_or_else(|error| panic!("rollback {round} failed: {error}"));
		assert_eq!(snapshot(db).await, Vec::<String>::new(), "rollback {round}");

		let applied = db
			.migrate(&migrations)
			.await
			.unwrap_or_else(|error| panic!("reapply {round} failed: {error}"));
		assert_eq!(applied, names, "reapply {round} runs every migration again");
		assert_same(
			&built,
			&snapshot(db).await,
			&format!("reapply {round} builds a different schema than the first apply"),
		);
	}
}

/// One migration at a time: up N, down N, up N, for every N. Each step
/// compares the schema against what the migrations up to that point built
/// the first time, so a `down.sql` that restores the wrong column, drops
/// the wrong index, or leaves a trigger behind fails here, named.
async fn each_migration_reverses_and_reapplies(engine: &Engine, db: &Db) {
	let migrations = engine.discover();
	let mut expected: Vec<Vec<String>> = vec![snapshot(db).await];
	assert!(expected[0].is_empty(), "the database starts empty");

	for count in 1..=migrations.len() {
		let name = migrations[count - 1].name.to_string();

		db.migrate(&migrations[..count])
			.await
			.unwrap_or_else(|error| panic!("applying `{name}` failed: {error}"));
		let after_up = snapshot(db).await;
		assert_ne!(
			after_up,
			expected[count - 1],
			"`{name}` changes the schema, or this test proves nothing about it"
		);
		expected.push(after_up);

		let reversed = db
			.rollback(&migrations, 1)
			.await
			.unwrap_or_else(|error| panic!("rolling back `{name}` failed: {error}"));
		assert_eq!(
			reversed,
			std::slice::from_ref(&name),
			"one step reverses `{name}` only"
		);
		assert_same(
			&expected[count - 1],
			&snapshot(db).await,
			&format!("after rolling back `{name}`, the schema is not what it was before it"),
		);

		db.migrate(&migrations[..count])
			.await
			.unwrap_or_else(|error| panic!("reapplying `{name}` failed: {error}"));
		assert_same(
			&expected[count],
			&snapshot(db).await,
			&format!("after reapplying `{name}`, the schema is not what it was the first time"),
		);
	}
}

fn int(value: &Value) -> i64 {
	match value {
		Value::Int(int) => *int,
		other => panic!("expected an integer, found {other:?}"),
	}
}

async fn run(db: &Db, sql: impl Into<String>) -> Vec<Vec<Value>> {
	let sql = sql.into();

	db.fetch(Statement {
		sql: sql.clone(),
		args: Vec::new(),
	})
	.await
	.unwrap_or_else(|error| panic!("{error}\n{sql}"))
}

fn person(n: u128) -> Uuid {
	Uuid::from_u128(0xBAC0_0000_0000_0000_0000_0000_0000_0000 | n)
}

/// The day of March 2026 an act happened on, as the timestamp it is
/// stored under.
fn day(day: u32) -> String {
	format!("2026-03-{day:02}T12:00:00+00:00")
}

/// Rows the backfill reads, inserted by SQL the way an older service's own
/// writes would have left them: who the backfill must register, with the
/// day of their earliest act, and a person it must not.
async fn seed_backfill_sources(dialect: Dialect, db: &Db) -> ([(Uuid, u32); 9], Uuid) {
	let sql = Sql(dialect);

	let boolean = |value: bool| {
		match dialect {
			Dialect::Sqlite => i32::from(value).to_string(),
			Dialect::Postgres => value.to_string(),
		}
	};
	let subscribe = |who: Uuid, topic: &str, on: u32| {
		format!(
			"INSERT INTO topic_subscriptions (id, person, topic, created_at) VALUES ({}, {}, \
			 '{topic}', {})",
			sql.new_id(),
			sql.uuid(who),
			sql.timestamp(&day(on))
		)
	};
	let prefer = |who: Uuid, category: &str, opted_out: bool, on: u32| {
		format!(
			"INSERT INTO category_preferences (id, person, category, opted_out, created_at) \
			 VALUES ({}, {}, '{category}', {}, {})",
			sql.new_id(),
			sql.uuid(who),
			boolean(opted_out),
			sql.timestamp(&day(on))
		)
	};
	let enroll = |who: Uuid, fingerprint: u128, invalidated: Option<u32>, on: u32| {
		format!(
			"INSERT INTO push_endpoints (id, person, platform, credential, generation, \
			 fingerprint, invalidated_at, created_at) VALUES ({}, {}, 'apns', 'token', 1, \
			 '{fingerprint:064x}', {}, {})",
			sql.new_id(),
			sql.uuid(who),
			invalidated.map_or("NULL".to_owned(), |on| sql.timestamp(&day(on))),
			sql.timestamp(&day(on))
		)
	};

	// Who the backfill must register, and the day of their earliest act.
	let expected = [
		// One source each.
		(person(1), 10),
		(person(2), 11),
		(person(3), 12),
		// All three sources: the endpoint is the earliest evidence.
		(person(4), 3),
		// Two sources: the preference is earlier than the subscription.
		(person(5), 15),
		// One source, twice: the second subscription is the earlier.
		(person(6), 14),
		// An endpoint the provider has since invalidated is still an act.
		(person(7), 13),
		// A preference that opts in is still an act.
		(person(8), 16),
		// Two endpoints: the earlier one counts.
		(person(10), 17),
	];
	let named_only = person(9);

	let statements = [
		subscribe(person(1), "news.old", 10),
		prefer(person(2), "news", true, 11),
		enroll(person(3), 3, None, 12),
		enroll(person(4), 4, None, 3),
		subscribe(person(4), "news.old", 7),
		prefer(person(4), "news", true, 9),
		subscribe(person(5), "news.old", 20),
		prefer(person(5), "news", true, 15),
		subscribe(person(6), "news.old", 25),
		subscribe(person(6), "sports.old", 14),
		enroll(person(7), 7, Some(20), 13),
		prefer(person(8), "news", false, 16),
		enroll(person(10), 10, None, 18),
		enroll(person(10), 11, None, 17),
		// A person a send only named: a notification and a wake row, and
		// none of the three source tables.
		format!(
			"INSERT INTO notifications (id, send_key, person, category, title, body, data, \
			 visible_at, created_at) VALUES ({}, 'old-send', {}, 'news', 'Hello', '', '{{}}', {}, \
			 {})",
			sql.new_id(),
			sql.uuid(named_only),
			sql.timestamp(&day(1)),
			sql.timestamp(&day(1))
		),
		format!(
			"INSERT INTO wakes (id, person, woken_at, created_at) VALUES ({}, {}, {}, {})",
			sql.new_id(),
			sql.uuid(named_only),
			sql.timestamp(&day(1)),
			sql.timestamp(&day(1))
		),
	];

	for statement in statements {
		run(db, statement).await;
	}

	(expected, named_only)
}

/// The migration before `persons_and_segments` has run; rows the backfill
/// reads are inserted by SQL, the way an older service's own writes would
/// have left them; then `persons_and_segments` runs, and the registry
/// holds exactly one row per distinct person the three source tables
/// know — with the earliest evidence as `first_seen` — and nothing else.
async fn backfill_registers_exactly_the_known_persons(engine: &Engine, db: &Db) {
	let sql = Sql(engine.dialect);
	let migrations = engine.discover();
	let index = migrations
		.iter()
		.position(|migration| migration.name.ends_with("_persons_and_segments"))
		.expect("the registry lists persons_and_segments");

	db.migrate(&migrations[..index])
		.await
		.expect("the migrations before it apply");

	let (expected, named_only) = seed_backfill_sources(engine.dialect, db).await;

	let applied = db
		.migrate(&migrations[..=index])
		.await
		.expect("persons_and_segments applies over existing rows");
	assert_eq!(applied, [migrations[index].name.to_string()]);

	let counts = run(
		db,
		"SELECT count(*), count(DISTINCT person), count(DISTINCT id) FROM audience_members",
	)
	.await;
	let wanted = i64::try_from(expected.len()).expect("small");
	assert_eq!(
		(int(&counts[0][0]), int(&counts[0][1]), int(&counts[0][2])),
		(wanted, wanted, wanted),
		"one row per distinct person, each with an id of its own, and no one else"
	);

	// The day, in both engines' own spelling of a timestamp.
	let (first_seen, updated_at) = match engine.dialect {
		Dialect::Sqlite => ("substr(first_seen, 1, 10)", "substr(updated_at, 1, 10)"),
		Dialect::Postgres => {
			(
				"to_char(first_seen AT TIME ZONE 'UTC', 'YYYY-MM-DD')",
				"to_char(updated_at AT TIME ZONE 'UTC', 'YYYY-MM-DD')",
			)
		}
	};

	for (who, earliest) in expected {
		let rows = run(
			db,
			format!(
				"SELECT {first_seen}, {updated_at} FROM audience_members WHERE person = {}",
				sql.uuid(who)
			),
		)
		.await;
		let want = format!("2026-03-{earliest:02}");

		assert_eq!(rows.len(), 1, "{who} is registered once");
		assert_eq!(
			rows[0],
			[Value::Text(want.clone()), Value::Text(want.clone())],
			"{who}'s first_seen and updated_at are their earliest act, {want}"
		);
	}

	let rows = run(
		db,
		format!(
			"SELECT count(*) FROM audience_members WHERE person = {}",
			sql.uuid(named_only)
		),
	)
	.await;
	assert_eq!(
		int(&rows[0][0]),
		0,
		"a person a send only named stays unregistered"
	);

	// The migration registers persons and invents nothing about them, and
	// reads the other tables without writing them.
	for (table, rows) in [
		("person_attributes", 0),
		("topic_subscriptions", 5),
		("category_preferences", 4),
		("push_endpoints", 5),
		("notifications", 1),
		("wakes", 1),
	] {
		let found = run(db, format!("SELECT count(*) FROM {table}")).await;
		assert_eq!(int(&found[0][0]), rows, "`{table}` after the backfill");
	}
}

/// Rolling `message_options` back takes away the column that says which
/// rows are silent data messages. Those rows must not turn into ordinary
/// notifications in somebody's inbox: the rollback cancels each one, and
/// leaves every other row as it was.
async fn rolling_message_options_back_hides_data_messages(engine: &Engine, db: &Db) {
	let sql = Sql(engine.dialect);
	let migrations = engine.discover();
	let index = migrations
		.iter()
		.position(|migration| migration.name.ends_with("_message_options"))
		.expect("the registry lists message_options");
	let through = &migrations[..=index];

	db.migrate(through).await.expect("the migrations apply");

	let at = sql.timestamp(&day(3));
	let earlier = sql.timestamp(&day(2));

	for (key, kind, cancelled) in [
		("silent", "data", "NULL"),
		("silent-cancelled", "data", earlier.as_str()),
		("shown", "notification", "NULL"),
		("shown-cancelled", "notification", earlier.as_str()),
	] {
		run(
			db,
			format!(
				"INSERT INTO notifications (id, send_key, person, category, title, body, data, \
				 visible_at, cancelled_at, created_at, kind) VALUES ({}, '{key}', {}, 'news', \
				 't', '', '{{}}', {at}, {cancelled}, {at}, '{kind}')",
				sql.new_id(),
				sql.uuid(person(1)),
			),
		)
		.await;
	}

	let reversed = db
		.rollback(through, 1)
		.await
		.expect("message_options rolls back over rows");
	assert_eq!(reversed, [migrations[index].name.to_string()]);

	let visible: Vec<String> = run(
		db,
		"SELECT send_key FROM notifications WHERE cancelled_at IS NULL ORDER BY send_key",
	)
	.await
	.into_iter()
	.map(|row| {
		match &row[0] {
			Value::Text(key) => key.clone(),
			other => panic!("expected a key, found {other:?}"),
		}
	})
	.collect();
	assert_eq!(
		visible,
		["shown"],
		"only the ordinary, uncancelled notification is still visible"
	);

	// A row cancelled before the rollback keeps the time it was cancelled.
	let kept = run(
		db,
		format!(
			"SELECT count(*) FROM notifications WHERE cancelled_at = {earlier} AND send_key IN \
			 ('silent-cancelled', 'shown-cancelled')"
		),
	)
	.await;
	assert_eq!(int(&kept[0][0]), 2, "earlier cancellations are untouched");
}

#[tokio::test(flavor = "multi_thread")]
async fn sqlite_rolling_message_options_back_hides_data_messages() {
	let engine = sqlite();
	rolling_message_options_back_hides_data_messages(&engine, &engine.database().await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn sqlite_a_full_rollback_leaves_nothing() {
	let engine = sqlite();
	full_rollback_leaves_nothing(&engine, &engine.database().await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn sqlite_a_rollback_leaves_nothing_that_blocks_a_reapply() {
	let engine = sqlite();
	reapply_after_full_rollback(&engine, &engine.database().await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn sqlite_each_migration_reverses_and_reapplies_on_its_own() {
	let engine = sqlite();
	each_migration_reverses_and_reapplies(&engine, &engine.database().await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn sqlite_the_backfill_registers_exactly_the_known_persons() {
	let engine = sqlite();
	backfill_registers_exactly_the_known_persons(&engine, &engine.database().await).await;
}

#[cfg(feature = "postgres")]
mod on_postgres {
	use super::*;
	use crate::postgres::Postgres;

	async fn connect(postgres: &Postgres) -> Db {
		Db::connect(&postgres.database())
			.await
			.expect("the database opens")
	}

	async fn schemas(db: &Db) -> Vec<String> {
		run(
			db,
			"SELECT nspname::text FROM pg_namespace WHERE nspname IN ('live', 'rpc') ORDER BY \
			 nspname",
		)
		.await
		.into_iter()
		.map(|row| {
			match &row[0] {
				Value::Text(name) => name.clone(),
				other => panic!("expected a schema name, found {other:?}"),
			}
		})
		.collect()
	}

	/// `init` creates the `live` and `rpc` schemas, so its rollback drops
	/// them — but only the ones it leaves empty. A schema that holds
	/// something of another owner's stays, with what it holds, and the
	/// rollback still succeeds and can be applied again.
	#[tokio::test(flavor = "multi_thread")]
	async fn postgres_rolling_init_back_drops_only_the_schemas_it_emptied() {
		let Some(postgres) = Postgres::start() else {
			return;
		};
		let engine = postgres_engine();
		let db = connect(&postgres).await;
		let migrations = engine.discover();

		// On its own in a database: both schemas go.
		db.migrate(&migrations).await.expect("the migrations apply");
		assert_eq!(schemas(&db).await, ["live", "rpc"]);
		db.rollback(&migrations, migrations.len())
			.await
			.expect("the migrations roll back");
		assert_eq!(schemas(&db).await, Vec::<String>::new());

		// Sharing `live` with something that was there first: `live` and
		// the stranger's table stay, `rpc` goes.
		run(&db, "CREATE SCHEMA live").await;
		run(&db, "CREATE TABLE live.somebody_elses (id integer)").await;
		run(&db, "INSERT INTO live.somebody_elses VALUES (7)").await;
		db.migrate(&migrations)
			.await
			.expect("the migrations apply beside it");
		db.rollback(&migrations, migrations.len())
			.await
			.expect("the migrations roll back beside it");
		assert_eq!(schemas(&db).await, ["live"]);
		assert_eq!(
			int(&run(&db, "SELECT id FROM live.somebody_elses").await[0][0]),
			7,
			"what the rollback did not make is untouched"
		);

		db.migrate(&migrations)
			.await
			.expect("and they apply again afterwards");
	}

	#[tokio::test(flavor = "multi_thread")]
	async fn postgres_rolling_message_options_back_hides_data_messages() {
		let Some(postgres) = Postgres::start() else {
			return;
		};

		rolling_message_options_back_hides_data_messages(
			&postgres_engine(),
			&connect(&postgres).await,
		)
		.await;
	}

	#[tokio::test(flavor = "multi_thread")]
	async fn postgres_a_full_rollback_leaves_nothing() {
		let Some(postgres) = Postgres::start() else {
			return;
		};

		full_rollback_leaves_nothing(&postgres_engine(), &connect(&postgres).await).await;
	}

	#[tokio::test(flavor = "multi_thread")]
	async fn postgres_a_rollback_leaves_nothing_that_blocks_a_reapply() {
		let Some(postgres) = Postgres::start() else {
			return;
		};

		reapply_after_full_rollback(&postgres_engine(), &connect(&postgres).await).await;
	}

	#[tokio::test(flavor = "multi_thread")]
	async fn postgres_each_migration_reverses_and_reapplies_on_its_own() {
		let Some(postgres) = Postgres::start() else {
			return;
		};

		each_migration_reverses_and_reapplies(&postgres_engine(), &connect(&postgres).await).await;
	}

	#[tokio::test(flavor = "multi_thread")]
	async fn postgres_the_backfill_registers_exactly_the_known_persons() {
		let Some(postgres) = Postgres::start() else {
			return;
		};

		backfill_registers_exactly_the_known_persons(&postgres_engine(), &connect(&postgres).await)
			.await;
	}
}
