//! Mixed-version startup and bounded expansion on a real PostgreSQL database.

#![cfg(all(feature = "postgres", feature = "sqlite"))]

mod support;

use std::time::Duration;
use std::time::Instant;

use leptoskit_db::Connection;
use leptoskit_db::Db;
use leptoskit_db::Migration;
use leptoskit_db::MigrationError;
use leptoskit_db::MigrationWindow;
use leptoskit_db::MigrationWindowError;
use leptoskit_db::PoolOptions;
use leptoskit_db::Read;
use leptoskit_db::Statement;
use leptoskit_db::Value;
use leptoskit_db_schema::Column;
use leptoskit_db_schema::ColumnType;
use leptoskit_db_schema::Dialect;
use leptoskit_db_schema::RequireHints;
use leptoskit_db_schema::Schema;
use leptoskit_db_schema::Table;
use leptoskit_db_schema::TableName;

struct Fixture {
	migrations: Vec<Migration>,
	snapshots: Vec<String>,
	window: MigrationWindow,
}

impl Fixture {
	fn new() -> Self {
		let baseline = Schema::new(
			Dialect::Postgres,
			vec![Table::new(
				TableName::new("notes"),
				vec![
					Column::new("id", ColumnType::Int64),
					Column::new("body", ColumnType::TEXT),
				],
				vec!["id".into()],
			)],
		);
		let mut schemas = vec![baseline];
		for name in ["extra", "later"] {
			let mut next = schemas.last().unwrap().clone();
			next.tables[0]
				.columns
				.push(Column::new(name, ColumnType::TEXT).nullable());
			schemas.push(next);
		}
		let mut before = Schema::empty(Dialect::Postgres);
		let migrations: Vec<_> = schemas
			.iter()
			.enumerate()
			.map(|(index, after)| {
				let plan = leptoskit_db_schema::diff(&before, after, &mut RequireHints).unwrap();
				let migration = Migration::new(
					format!("step_{index}"),
					leptoskit_db_schema::render(&plan, &before, after).sql(),
				);
				before = after.clone();
				migration
			})
			.collect();
		let snapshots: Vec<_> = schemas.iter().map(Schema::to_json).collect();
		let window = MigrationWindow::new(
			&migrations[..1],
			&snapshots[0],
			migrations[1..]
				.iter()
				.cloned()
				.zip(snapshots[1..].iter().cloned()),
		)
		.unwrap();
		Self {
			migrations,
			snapshots,
			window,
		}
	}

	async fn start(&self, db: &Db, length: usize) -> Result<Vec<String>, MigrationError> {
		db.migrate_in_window(
			&self.migrations[..length],
			&self.snapshots[length - 1],
			&self.window,
		)
		.await
	}
}

fn statement(sql: &str) -> Statement {
	Statement {
		sql: sql.into(),
		args: vec![],
	}
}

#[tokio::test(flavor = "multi_thread")]
async fn old_build_restarts_and_writes_after_new_build_expands() {
	let Some(cluster) = support::postgres::PostgresCluster::start() else {
		return;
	};
	let url = cluster.database();
	let db = Db::connect(&url).await.unwrap();
	let fixture = Fixture::new();
	db.migrate(&fixture.migrations[..1]).await.unwrap();
	db.execute(statement("INSERT INTO notes (id, body) VALUES (1, 'old')"))
		.await
		.unwrap();
	assert_eq!(fixture.start(&db, 3).await.unwrap(), ["step_1", "step_2"]);
	let restarted = Db::connect(&url).await.unwrap();
	assert!(restarted.migrate(&fixture.migrations[..1]).await.is_err());
	assert_eq!(
		fixture.start(&restarted, 1).await.unwrap(),
		Vec::<String>::new()
	);
	restarted
		.execute(statement(
			"UPDATE notes SET body = 'old restarted' WHERE id = 1",
		))
		.await
		.unwrap();
	db.execute(statement("UPDATE notes SET extra = 'new' WHERE id = 1"))
		.await
		.unwrap();
	assert_eq!(
		restarted
			.fetch(statement("SELECT body, extra FROM notes WHERE id = 1"))
			.await
			.unwrap(),
		vec![vec![
			Value::Text("old restarted".into()),
			Value::Text("new".into())
		]]
	);
	assert_eq!(restarted.applied_migrations().await.unwrap().len(), 3);
	assert_eq!(fixture.start(&db, 3).await.unwrap(), Vec::<String>::new());
}

#[tokio::test(flavor = "multi_thread")]
async fn later_failure_rolls_back_all_expansions_and_their_history() {
	let Some(cluster) = support::postgres::PostgresCluster::start() else {
		return;
	};
	let db = Db::connect(&cluster.database()).await.unwrap();
	let fixture = Fixture::new();
	db.migrate(&fixture.migrations[..1]).await.unwrap();
	// Unrecorded physical drift causes the second expansion to fail. The
	// first expansion must not survive the failed transaction.
	db.execute(statement("ALTER TABLE notes ADD COLUMN later TEXT"))
		.await
		.unwrap();
	assert!(fixture.start(&db, 3).await.is_err());
	assert_eq!(db.applied_migrations().await.unwrap(), ["step_0"]);
	assert!(
		db.fetch(statement("SELECT extra FROM notes"))
			.await
			.is_err()
	);
	db.execute(statement("ALTER TABLE notes DROP COLUMN later"))
		.await
		.unwrap();
	assert_eq!(fixture.start(&db, 3).await.unwrap(), ["step_1", "step_2"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn pushes_are_refused_even_when_the_old_build_has_no_pending_steps() {
	let Some(cluster) = support::postgres::PostgresCluster::start() else {
		return;
	};
	let db = Db::connect(&cluster.database()).await.unwrap();
	let fixture = Fixture::new();
	db.migrate(&fixture.migrations[..1]).await.unwrap();
	db.push(
		&Migration::new("development", "SELECT 1;"),
		&fixture.snapshots[0],
	)
	.await
	.unwrap();
	assert!(matches!(
		fixture.start(&db, 1).await,
		Err(MigrationError::Window(MigrationWindowError::PushedState))
	));
	assert_eq!(db.applied_migrations().await.unwrap(), ["step_0"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn lock_contention_has_a_deadline_and_does_not_advance_history() {
	let Some(cluster) = support::postgres::PostgresCluster::start() else {
		return;
	};
	let db = Db::connect(&cluster.database()).await.unwrap();
	let fixture = Fixture::new();
	db.migrate(&fixture.migrations[..1]).await.unwrap();
	let writer = db.begin().await.unwrap();
	writer
		.execute(statement("LOCK TABLE notes IN ACCESS EXCLUSIVE MODE"))
		.await
		.unwrap();
	let started = Instant::now();
	assert!(
		tokio::time::timeout(Duration::from_secs(15), fixture.start(&db, 3))
			.await
			.unwrap()
			.is_err()
	);
	assert!(started.elapsed() >= Duration::from_secs(4));
	assert_eq!(db.applied_migrations().await.unwrap(), ["step_0"]);
	writer.rollback().await.unwrap();
	assert_eq!(fixture.start(&db, 3).await.unwrap(), ["step_1", "step_2"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelled_wait_does_not_leave_a_pooled_connection_holding_the_lock() {
	let Some(cluster) = support::postgres::PostgresCluster::start() else {
		return;
	};
	let url = cluster.database();
	let control = Db::connect(&url).await.unwrap();
	let db = Db::connect_with(&url, PoolOptions { max_connections: 1 })
		.await
		.unwrap();
	let fixture = Fixture::new();
	control.migrate(&fixture.migrations[..1]).await.unwrap();
	let holder = control.begin().await.unwrap();
	holder
		.execute(statement(
			"SELECT pg_advisory_xact_lock(7812448352509454194)",
		))
		.await
		.unwrap();
	let applying = db.clone();
	let task = tokio::spawn(async move { Fixture::new().start(&applying, 3).await });
	tokio::time::timeout(Duration::from_secs(3), async {
		loop {
			let rows = control
				.fetch(statement(
					"SELECT EXISTS (SELECT 1 FROM pg_locks WHERE locktype = 'advisory' AND NOT \
					 granted)",
				))
				.await
				.unwrap();
			if rows == vec![vec![Value::Bool(true)]] {
				break;
			}
			tokio::time::sleep(Duration::from_millis(10)).await;
		}
	})
	.await
	.unwrap();
	task.abort();
	assert!(task.await.unwrap_err().is_cancelled());
	holder.rollback().await.unwrap();
	assert_eq!(
		tokio::time::timeout(Duration::from_secs(5), fixture.start(&db, 3))
			.await
			.unwrap()
			.unwrap(),
		["step_1", "step_2"]
	);
	// Another connection must now acquire the same lock and restart the old
	// build. A leaked session lock in the one-connection pool would block it.
	assert_eq!(
		tokio::time::timeout(Duration::from_secs(3), fixture.start(&control, 1))
			.await
			.unwrap()
			.unwrap(),
		Vec::<String>::new()
	);
}

#[tokio::test]
async fn sqlite_refuses_the_protocol_without_changing_any_history() {
	let db = Db::connect("sqlite::memory:").await.unwrap();
	let fixture = Fixture::new();
	assert!(matches!(
		fixture.start(&db, 1).await,
		Err(MigrationError::Window(
			MigrationWindowError::UnsupportedDialect
		))
	));
	assert!(
		db.fetch(statement("SELECT * FROM _leptoskit_migrations"))
			.await
			.is_err()
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_an_ordinary_migration_discards_its_locked_session() {
	let Some(cluster) = support::postgres::PostgresCluster::start() else {
		return;
	};
	let url = cluster.database();
	let control = Db::connect(&url).await.unwrap();
	let db = Db::connect_with(&url, PoolOptions { max_connections: 1 })
		.await
		.unwrap();
	let fixture = Fixture::new();
	control.migrate(&fixture.migrations[..1]).await.unwrap();
	let holder = control.begin().await.unwrap();
	holder
		.execute(statement("LOCK TABLE notes IN ACCESS EXCLUSIVE MODE"))
		.await
		.unwrap();
	let applying = db.clone();
	let task = tokio::spawn(async move { applying.migrate(&Fixture::new().migrations).await });
	tokio::time::timeout(Duration::from_secs(3), async {
		loop {
			let rows = control
				.fetch(statement(
					"SELECT EXISTS (SELECT 1 FROM pg_locks WHERE locktype = 'relation' AND NOT \
					 granted)",
				))
				.await
				.unwrap();
			if rows == vec![vec![Value::Bool(true)]] {
				break;
			}
			tokio::time::sleep(Duration::from_millis(10)).await;
		}
	})
	.await
	.unwrap();
	task.abort();
	assert!(task.await.unwrap_err().is_cancelled());
	// The cancelled migration already held the session advisory lock. An
	// independent connection must be able to acquire it without waiting for
	// the original pool to be destroyed or its connection to be reused.
	assert_eq!(
		tokio::time::timeout(Duration::from_secs(3), fixture.start(&control, 1))
			.await
			.unwrap()
			.unwrap(),
		Vec::<String>::new()
	);
	holder.rollback().await.unwrap();
	assert_eq!(db.applied_migrations().await.unwrap(), ["step_0"]);
	assert_eq!(
		db.migrate(&fixture.migrations).await.unwrap(),
		["step_1", "step_2"]
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn cancellation_after_first_expansion_rolls_back_sql_and_history() {
	let Some(cluster) = support::postgres::PostgresCluster::start() else {
		return;
	};
	let url = cluster.database();
	let control = Db::connect(&url).await.unwrap();
	let db = Db::connect_with(&url, PoolOptions { max_connections: 1 })
		.await
		.unwrap();
	let fixture = Fixture::new();
	control.migrate(&fixture.migrations[..1]).await.unwrap();
	control
		.execute(statement(
			"CREATE FUNCTION pause_later() RETURNS event_trigger LANGUAGE plpgsql AS $$ BEGIN IF \
			 current_query() LIKE '%later%' THEN PERFORM pg_advisory_xact_lock(12345); END IF; \
			 END $$",
		))
		.await
		.unwrap();
	control
		.execute(statement(
			"CREATE EVENT TRIGGER pause_expansion ON ddl_command_start EXECUTE FUNCTION \
			 pause_later()",
		))
		.await
		.unwrap();
	let holder = control.begin().await.unwrap();
	holder
		.execute(statement("SELECT pg_advisory_xact_lock(12345)"))
		.await
		.unwrap();
	let applying = db.clone();
	let task = tokio::spawn(async move { Fixture::new().start(&applying, 3).await });
	tokio::time::timeout(Duration::from_secs(3), async {
		loop {
			let rows = control
				.fetch(statement(
					"SELECT EXISTS (SELECT 1 FROM pg_locks WHERE locktype = 'advisory' AND objid \
					 = 12345 AND NOT granted)",
				))
				.await
				.unwrap();
			if rows == vec![vec![Value::Bool(true)]] {
				break;
			}
			tokio::time::sleep(Duration::from_millis(10)).await;
		}
	})
	.await
	.unwrap();
	task.abort();
	assert!(task.await.unwrap_err().is_cancelled());
	assert_eq!(
		tokio::time::timeout(Duration::from_secs(3), fixture.start(&control, 1))
			.await
			.unwrap()
			.unwrap(),
		Vec::<String>::new()
	);
	assert_eq!(db.applied_migrations().await.unwrap(), ["step_0"]);
	assert!(
		db.fetch(statement("SELECT extra FROM notes"))
			.await
			.is_err()
	);
	assert!(
		db.fetch(statement("SELECT later FROM notes"))
			.await
			.is_err()
	);
	holder.rollback().await.unwrap();
	control
		.execute(statement("DROP EVENT TRIGGER pause_expansion"))
		.await
		.unwrap();
	assert_eq!(fixture.start(&db, 3).await.unwrap(), ["step_1", "step_2"]);
}
