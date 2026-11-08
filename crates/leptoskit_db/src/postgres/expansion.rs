//! Bounded, atomic execution of an independently checked schema window.

use std::time::Duration;

use super::MIGRATION_LOCK;
use super::Pool;
use super::error;
use super::has_pushes;
use crate::Migration;
use crate::MigrationError;
use crate::MigrationWindowError;
use crate::migration_window::CheckedWindow;

impl Pool {
	pub(crate) async fn migrate_in_window(
		&self,
		migrations: &[Migration],
		window: &CheckedWindow,
	) -> Result<Vec<String>, MigrationError> {
		tokio::time::timeout(
			Duration::from_secs(60),
			self.apply_window(migrations, window),
		)
		.await
		.map_err(|_| MigrationWindowError::Timeout)?
	}

	async fn apply_window(
		&self,
		migrations: &[Migration],
		window: &CheckedWindow,
	) -> Result<Vec<String>, MigrationError> {
		let mut session = self.migration_session().await?;
		let client = session.client_mut();
		// The transaction guard queues ROLLBACK. The session guard also closes
		// the connection on cancellation: dropping a query future alone does
		// not stop a long batch already executing on PostgreSQL.
		let transaction = client.transaction().await.map_err(error)?;
		transaction
			.batch_execute("SET LOCAL lock_timeout = '5s'; SET LOCAL statement_timeout = '30s'")
			.await
			.map_err(error)?;
		transaction
			.execute("SELECT pg_advisory_xact_lock($1)", &[&MIGRATION_LOCK])
			.await
			.map_err(error)?;
		// Do not create bookkeeping tables or bootstrap a baseline here. A
		// missing baseline is not an online expansion of an existing database.
		let rows = transaction
			.query("SELECT name, checksum FROM _leptoskit_migrations", &[])
			.await
			.map_err(error)?;
		let applied = rows
			.iter()
			.map(|row| (row.get(0), row.get(1)))
			.collect::<Vec<_>>();
		let pending = window.pending(&applied, migrations)?;

		if has_pushes(&transaction).await? {
			return Err(MigrationWindowError::PushedState.into());
		}

		transaction
			.query_one(
				"SELECT set_config($1, 'on', true)",
				&[&leptoskit_db_schema::BYPASS_SETTING],
			)
			.await
			.map_err(error)?;
		let mut names = Vec::with_capacity(pending.len());

		for migration in pending {
			apply(&transaction, migration).await?;
			names.push(migration.name.to_string());
		}

		transaction.commit().await.map_err(error)?;
		session.release();
		Ok(names)
	}
}

async fn apply(
	transaction: &deadpool_postgres::Transaction<'_>,
	migration: &Migration,
) -> Result<(), MigrationError> {
	let failed = |source| {
		MigrationError::Failed {
			name: migration.name.to_string(),
			source: error(source),
		}
	};
	transaction
		.batch_execute(&migration.sql)
		.await
		.map_err(failed)?;
	let now = crate::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
	transaction
		.execute(
			"INSERT INTO _leptoskit_migrations (name, checksum, applied_at) VALUES ($1, $2, $3)",
			&[&migration.name.as_ref(), &migration.checksum.as_ref(), &now],
		)
		.await
		.map_err(failed)?;
	Ok(())
}
