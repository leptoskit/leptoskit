//! Backups of managed databases, and restoring one.
//!
//! The control plane never holds a backup's bytes and never talks to a
//! Postgres server. A backup is a row here and a job for the cluster's
//! worker, which dumps the database with its own connection to the server
//! and keeps the dump where only it reads. What comes back is a `location`
//! in the worker's own words, which is kept on the row and given to no
//! caller: it returns to that cluster's worker, in a restore's input, and
//! goes nowhere else.
//!
//! - A backup and a restore are jobs of their environment, so a cluster's
//!   worker runs them one at a time, in the order they were asked for, with its
//!   releases.
//! - A backup whose worker goes quiet has failed: nothing was promised, and
//!   another can be asked for. A restore whose worker goes quiet is *unknown*,
//!   as a release is, until an operator says which database the environment
//!   runs on.
//! - A restore is refused unless the live deployment knows every migration the
//!   backup holds: its server would refuse to start on anything else.

use leptoskit_db::Id;
use leptoskit_db::Json;
use leptoskit_db::Tx;
use leptoskit_rpc as rpc;
use uuid::Uuid;

use super::audit;
use super::audit::Actor;
use super::authorize;
use super::databases;
use super::delta;
use super::deployments;
use super::identity::client;
use super::jobs;
use super::jobs::Queue;
use super::jobs::StoredInput;
use super::projects::environment;
use super::services;
use crate::access::Permission;
use crate::databases::BACKUP_KEEP_DAYS;
use crate::databases::BackupKind;
use crate::databases::BackupList;
use crate::databases::BackupState;
use crate::databases::BackupSummary;
use crate::databases::MAX_BACKUPS;
use crate::databases::RestoreState;
use crate::databases::RestoreSummary;
use crate::operator::JobState;
use crate::tables::Database;
use crate::tables::DatabaseBackup;
use crate::tables::Environment;
use crate::tables::Job;
use crate::tables::NewDatabaseBackup;
use crate::tables::Placement;
use crate::workers::BackupDatabaseInput;
use crate::workers::BackupOutcome;
use crate::workers::BackupTaken;
use crate::workers::JobKind;
use crate::workers::JobOutcome;
use crate::workers::MAX_BACKUP_LOCATION;
use crate::workers::RestoreDatabaseInput;
use crate::workers::RestoreOutcome;
use crate::workers::RuntimeValue;

/// The longest a failure's reason is kept, in bytes.
const MAX_REASON_LEN: usize = 300;

/// The most backups one answer lists. An environment keeps fewer: at most
/// [`MAX_BACKUPS`] taken on request, the ones restores took, and the ones
/// that failed, none for longer than [`BACKUP_KEEP_DAYS`] days.
const LISTED: u64 = 200;

fn keep() -> chrono::TimeDelta {
	delta(std::time::Duration::from_secs(
		u64::from(BACKUP_KEEP_DAYS) * 24 * 60 * 60,
	))
}

/// A worker's reason as it is kept: with what a log line would have
/// redacted replaced, control characters written out, and cut short.
fn reason(text: &str) -> String {
	let text = super::logs::plain(&super::logs::scrub(text.trim()));

	if text.len() <= MAX_REASON_LEN {
		return text;
	}

	let mut end = MAX_REASON_LEN;

	while !text.is_char_boundary(end) {
		end -= 1;
	}

	format!("{}…", &text[..end])
}

fn summary(row: &DatabaseBackup) -> BackupSummary {
	BackupSummary {
		id: row.id.uuid(),
		kind: row.kind,
		state: row.state,
		bytes: row.bytes.and_then(|bytes| u64::try_from(bytes).ok()),
		failure: row.failure.clone(),
		created_at: row.created_at,
		completed_at: row.completed_at,
		expires_at: row.expires_at,
	}
}

/// A restore as its job tells it.
fn restore_summary(job: &Job) -> Option<RestoreSummary> {
	let StoredInput::RestoreDatabase(input) = &job.input.0 else {
		return None;
	};
	let failed = match &job.outcome.0 {
		Some(JobOutcome::RestoreDatabase(RestoreOutcome::Failed { reason })) => {
			Some(reason.clone())
		}
		_ => None,
	};
	let (state, failure) = match job.state {
		JobState::Queued | JobState::Leased => (RestoreState::Pending, None),
		JobState::Unknown => (RestoreState::Unknown, None),
		JobState::Done => {
			match failed {
				Some(reason) => (RestoreState::Failed, Some(reason)),
				None => (RestoreState::Restored, None),
			}
		}
		JobState::Abandoned => {
			(
				RestoreState::Failed,
				Some("it was given up on before its cluster's worker took it".to_owned()),
			)
		}
	};
	Some(RestoreSummary {
		backup: input.backup,
		state,
		failure,
		requested_at: job.created_at,
		updated_at: job.updated_at,
	})
}

/// The backup or restore of `environment` that hasn't settled, when there
/// is one: queued, with its worker, or unknown.
async fn under_way(
	db: &impl leptoskit_db::Read,
	environment: Id<Environment>,
) -> leptoskit_db::Result<Option<Job>> {
	Job::query()
		.filter(|job| {
			job.environment_id.eq(Some(environment))
				& (job.kind.eq(JobKind::BackupDatabase) | job.kind.eq(JobKind::RestoreDatabase))
				& (job.state.eq(JobState::Queued)
					| job.state.eq(JobState::Leased)
					| job.state.eq(JobState::Unknown))
		})
		.order_by(|job| job.created_at.asc())
		.first(db)
		.await
}

/// `Conflict` (`backup_in_progress`) while a backup or a restore of
/// `environment` hasn't settled.
async fn refuse_under_way(
	db: &impl leptoskit_db::Read,
	environment: &Environment,
) -> rpc::Result<()> {
	let Some(job) = under_way(db, environment.id).await? else {
		return Ok(());
	};
	let what = if job.kind == JobKind::RestoreDatabase {
		"a restore"
	} else {
		"a backup"
	};
	let unknown = if job.state == JobState::Unknown {
		", and what its cluster did is unknown until an operator reconciles it"
	} else {
		""
	};
	Err(rpc::Error::conflict(format!(
		"backup_in_progress: {what} of `{}`'s database is under way{unknown}; wait for it",
		environment.name
	)))
}

/// `Conflict` (`database_restoring`) while a restore of `environment`'s
/// database hasn't settled: nothing is deployed over a database that is
/// being replaced.
///
/// # Errors
///
/// That, or when the database fails.
pub(crate) async fn refuse_restoring(
	db: &impl leptoskit_db::Read,
	environment: &Environment,
) -> rpc::Result<()> {
	match under_way(db, environment.id).await? {
		Some(job) if job.kind == JobKind::RestoreDatabase => {
			Err(rpc::Error::conflict(format!(
				"database_restoring: a backup is being restored over `{}`'s database; wait for it \
				 to finish before deploying",
				environment.name
			)))
		}
		_ => Ok(()),
	}
}

/// `environment`'s database, once a release has made it.
async fn ready_database(
	db: &impl leptoskit_db::Read,
	environment: &Environment,
) -> rpc::Result<Database> {
	let Some(database) = databases::for_environment(db, environment.id).await? else {
		return Err(rpc::Error::conflict(format!(
			"no_database: `{}` has no managed database",
			environment.name
		)));
	};

	if !database.ready {
		return Err(rpc::Error::conflict(format!(
			"database_not_ready: `{}`'s database isn't made until a release that uses it has been \
			 applied",
			environment.name
		)));
	}

	Ok(database)
}

pub(crate) async fn database_backups(
	cx: &rpc::QueryCx,
	organization: &str,
	project: &str,
	environment_name: &str,
) -> rpc::Result<BackupList> {
	let (_, project) = authorize::project(
		cx.db(),
		cx.context(),
		organization,
		project,
		Permission::ViewProjects,
	)
	.await?;
	let environment_id = environment(cx.db(), &project, environment_name).await?.id;
	let now = leptoskit_db::now();
	let rows = DatabaseBackup::query()
		.filter(|row| row.environment_id.eq(environment_id) & row.expires_at.gt(now))
		.order_by(|row| row.created_at.desc())
		.order_by(|row| row.id.desc())
		.limit(LISTED)
		.all(cx.db())
		.await?;
	let restore = Job::query()
		.filter(|job| {
			job.environment_id.eq(Some(environment_id)) & job.kind.eq(JobKind::RestoreDatabase)
		})
		.order_by(|job| job.created_at.desc())
		.order_by(|job| job.id.desc())
		.first(cx.db())
		.await?;
	Ok(BackupList {
		backups: rows.iter().map(summary).collect(),
		restore: restore.as_ref().and_then(restore_summary),
	})
}

pub(crate) async fn create_backup(
	cx: &rpc::MutationCx,
	organization: &str,
	project: &str,
	environment_name: &str,
) -> rpc::Result<BackupSummary> {
	let (access, project) = authorize::project(
		cx.tx(),
		cx.context(),
		organization,
		project,
		Permission::Deploy,
	)
	.await?;
	let environment = environment(cx.tx(), &project, environment_name).await?;
	let database = ready_database(cx.tx(), &environment).await?;
	refuse_under_way(cx.tx(), &environment).await?;
	let environment_id = environment.id;
	let now = leptoskit_db::now();
	let kept = DatabaseBackup::query()
		.filter(|row| {
			row.environment_id.eq(environment_id)
				& row.kind.eq(BackupKind::Manual)
				& row.state.ne(BackupState::Failed)
				& row.expires_at.gt(now)
		})
		.count(cx.tx())
		.await?;

	if kept >= MAX_BACKUPS {
		return Err(rpc::Error::conflict(format!(
			"too_many_backups: `{}` keeps {MAX_BACKUPS} backups already, each for \
			 {BACKUP_KEEP_DAYS} days; wait for the oldest to expire",
			environment.name
		)));
	}

	let expires_at = now + keep();
	let row = NewDatabaseBackup {
		environment_id,
		organization_id: access.organization.id,
		database_id: database.id,
		kind: BackupKind::Manual,
		state: BackupState::Pending,
		location: None,
		bytes: None,
		migrations: Json(Vec::new()),
		schema_uncertain: false,
		failure: None,
		completed_at: None,
		expires_at,
	}
	.insert(cx.tx())
	.await?;
	jobs::queue(
		cx.tx(),
		Queue {
			organization: access.organization.id,
			deployment: None,
			environment: Some(environment_id),
			cluster: Some(database.cluster_id),
			requirements: Vec::new(),
			max_attempts: 1,
			input: StoredInput::BackupDatabase(BackupDatabaseInput {
				environment: RuntimeValue(deployments::environment_ref(
					&access.organization,
					&project,
					&environment,
				)),
				database: database.name.clone(),
				backup: row.id.uuid(),
				expires_at: row.expires_at,
			}),
		},
	)
	.await?;
	audit::record(
		cx.tx(),
		&Actor::from(&access.caller),
		"database.backup",
		Some(access.organization.id),
		Some(project.id),
		Some(format!(
			"{} {}: backup {}",
			environment.id, environment.name, row.id
		)),
		&client(cx.context()),
	)
	.await?;
	let waker = services(cx.context())?.clone();
	cx.after_commit(async move { jobs::wake(&waker) });
	Ok(summary(&row))
}

/// Why `backup` can't go back over a database whose placement is
/// `placement`, when it can't.
fn unrestorable(backup: &DatabaseBackup, placement: &Placement) -> Option<String> {
	if backup.schema_uncertain {
		return Some(
			"it was taken while the database's migrations were uncertain, so nothing can say \
			 which it holds"
				.to_owned(),
		);
	}

	if placement.schema_uncertain {
		return Some(
			"the database's migrations are uncertain: a release that failed may have applied \
			 some; deploy a fix forward, which settles them"
				.to_owned(),
		);
	}

	let known = &placement.migrations.0;
	let held = &backup.migrations.0;

	// The live server applies what the backup is missing as it starts, and
	// would refuse to start on a migration it doesn't know.
	if !known.starts_with(held) {
		let unknown: Vec<&str> = held
			.iter()
			.filter(|migration| !known.contains(migration))
			.map(|migration| migration.name.as_str())
			.collect();
		return Some(if unknown.is_empty() {
			"it holds its migrations in another order than the live deployment applies them"
				.to_owned()
		} else {
			format!(
				"it holds migrations the live deployment doesn't know ({}), so its server would \
				 refuse to start; deploy the version that has them first",
				unknown.join(", ")
			)
		});
	}

	None
}

pub(crate) async fn restore_backup(
	cx: &rpc::MutationCx,
	organization: &str,
	project: &str,
	environment_name: &str,
	backup: Uuid,
	confirm: &str,
) -> rpc::Result<RestoreSummary> {
	// As destructive as deleting the environment: owners and admins.
	let (access, project) = authorize::project(
		cx.tx(),
		cx.context(),
		organization,
		project,
		Permission::DeleteEnvironment,
	)
	.await?;
	let environment = environment(cx.tx(), &project, environment_name).await?;
	let expected = format!("{}/{}", project.slug, environment.name);

	if confirm != expected {
		return Err(rpc::Error::invalid(format!(
			"confirmation_mismatch: type `{expected}` as `confirm` to replace its database with \
			 the backup"
		)));
	}

	let environment_id = environment.id;
	let not_found = || {
		rpc::Error::not_found(format!(
			"backup_not_found: `{environment_name}` has no backup {backup}"
		))
	};
	// Only ever this environment's: an ID from anywhere else isn't found.
	let row = DatabaseBackup::query()
		.filter(|row| {
			row.id.eq(Id::<DatabaseBackup>::from_uuid(backup))
				& row.environment_id.eq(environment_id)
		})
		.first(cx.tx())
		.await?
		.ok_or_else(not_found)?;
	let database = ready_database(cx.tx(), &environment).await?;
	let now = leptoskit_db::now();
	let location = match (&row.state, &row.location) {
		(BackupState::Complete, Some(location))
			if row.expires_at > now
				&& row.database_id == database.id
				&& row.organization_id == access.organization.id =>
		{
			location.clone()
		}
		(BackupState::Pending, _) => {
			return Err(rpc::Error::conflict(format!(
				"backup_unavailable: backup {backup} hasn't been taken yet"
			)));
		}
		(BackupState::Failed, _) => {
			return Err(rpc::Error::conflict(format!(
				"backup_unavailable: backup {backup} failed, so there is nothing to restore"
			)));
		}
		_ => {
			return Err(rpc::Error::conflict(format!(
				"backup_unavailable: backup {backup} has expired"
			)));
		}
	};
	deployments::refuse_in_flight(cx.tx(), &environment).await?;
	refuse_under_way(cx.tx(), &environment).await?;
	let placement = Placement::query()
		.filter(|placement| placement.environment_id.eq(environment_id))
		.first(cx.tx())
		.await?;
	let unavailable =
		|why: String| rpc::Error::conflict(format!("restore_unavailable: backup {backup}: {why}"));
	let Some(placement) = placement else {
		return Err(unavailable("the environment isn't on a cluster".to_owned()));
	};

	if placement.cluster_id != database.cluster_id {
		return Err(unavailable(
			"the database isn't on the environment's cluster".to_owned(),
		));
	}

	if let Some(why) = unrestorable(&row, &placement) {
		return Err(unavailable(why));
	}

	let job = jobs::queue(
		cx.tx(),
		Queue {
			organization: access.organization.id,
			deployment: None,
			environment: Some(environment_id),
			cluster: Some(database.cluster_id),
			requirements: Vec::new(),
			max_attempts: 1,
			input: StoredInput::RestoreDatabase(RestoreDatabaseInput {
				environment: RuntimeValue(deployments::environment_ref(
					&access.organization,
					&project,
					&environment,
				)),
				database: database.name.clone(),
				backup,
				location,
				safety_expires_at: now + keep(),
			}),
		},
	)
	.await?;
	audit::record(
		cx.tx(),
		&Actor::from(&access.caller),
		"database.restore",
		Some(access.organization.id),
		Some(project.id),
		Some(format!(
			"{} {}: backup {}",
			environment.id, environment.name, row.id
		)),
		&client(cx.context()),
	)
	.await?;
	let waker = services(cx.context())?.clone();
	cx.after_commit(async move { jobs::wake(&waker) });
	restore_summary(&job).ok_or_else(|| rpc::Error::server("a restore's job isn't one"))
}

/// The backup `job` is for, while its environment exists.
async fn row_for(tx: &Tx, job: &Job) -> rpc::Result<Option<DatabaseBackup>> {
	let StoredInput::BackupDatabase(input) = &job.input.0 else {
		return Ok(None);
	};
	let id = Id::<DatabaseBackup>::from_uuid(input.backup);
	Ok(DatabaseBackup::query()
		.filter(|row| row.id.eq(id) & row.state.eq(BackupState::Pending))
		.first(tx)
		.await?)
}

/// Whether `taken` can be kept: a location is the worker's own words, but
/// it is short and printable.
fn keepable(taken: &BackupTaken) -> bool {
	!taken.location.is_empty()
		&& taken.location.chars().count() <= MAX_BACKUP_LOCATION
		&& !taken.location.chars().any(char::is_control)
		&& i64::try_from(taken.bytes).is_ok()
}

/// What a backup's outcome does to its row.
///
/// # Errors
///
/// When the database fails.
pub(crate) async fn backup_finished(
	tx: &Tx,
	job: &Job,
	outcome: &BackupOutcome,
) -> rpc::Result<()> {
	let Some(mut row) = row_for(tx, job).await? else {
		return Ok(());
	};

	match outcome {
		BackupOutcome::Taken(taken) if keepable(taken) => {
			// The worker runs one job of an environment at a time, so the
			// placement says now what the database held when it was dumped.
			let environment_id = row.environment_id;
			let placement = Placement::query()
				.filter(|placement| placement.environment_id.eq(environment_id))
				.first(tx)
				.await?;
			row.state = BackupState::Complete;
			row.location = Some(taken.location.clone());
			row.bytes = i64::try_from(taken.bytes).ok();
			row.schema_uncertain = placement
				.as_ref()
				.is_none_or(|placement| placement.schema_uncertain);
			row.migrations = Json(
				placement
					.map(|placement| placement.migrations.0)
					.unwrap_or_default(),
			);
			row.completed_at = Some(leptoskit_db::now());
		}
		BackupOutcome::Taken(_) => {
			row.state = BackupState::Failed;
			row.failure = Some(
				"its cluster's worker said where it is in a way that can't be kept".to_owned(),
			);
		}
		BackupOutcome::Failed { reason: why } => {
			row.state = BackupState::Failed;
			row.failure = Some(reason(why));
		}
	}

	row.save(tx).await?;
	Ok(())
}

/// A backup nobody will finish: its worker went quiet, or was revoked.
///
/// # Errors
///
/// When the database fails.
pub(crate) async fn backup_abandoned(tx: &Tx, job: &Job, why: &str) -> rpc::Result<()> {
	if let Some(mut row) = row_for(tx, job).await? {
		row.state = BackupState::Failed;
		row.failure = Some(reason(why));
		row.save(tx).await?;
	}

	Ok(())
}

/// What a restore's outcome leaves behind: the backup its worker took of
/// the data it replaced.
///
/// # Errors
///
/// When the database fails.
pub(crate) async fn restore_finished(
	tx: &Tx,
	job: &Job,
	outcome: &RestoreOutcome,
) -> rpc::Result<()> {
	let (StoredInput::RestoreDatabase(input), Some(environment_id)) =
		(&job.input.0, job.environment_id)
	else {
		return Ok(());
	};
	let RestoreOutcome::Restored {
		before_restore: Some(taken),
	} = outcome
	else {
		return Ok(());
	};

	if !keepable(taken) {
		return Ok(());
	}

	// While the environment and its database are still there.
	let Some(database) = databases::for_environment(tx, environment_id).await? else {
		return Ok(());
	};
	let placement = Placement::query()
		.filter(|placement| placement.environment_id.eq(environment_id))
		.first(tx)
		.await?;
	NewDatabaseBackup {
		environment_id,
		organization_id: job.organization_id,
		database_id: database.id,
		kind: BackupKind::BeforeRestore,
		state: BackupState::Complete,
		location: Some(taken.location.clone()),
		bytes: i64::try_from(taken.bytes).ok(),
		// What the restore replaced had everything the live deployment
		// applied.
		schema_uncertain: placement
			.as_ref()
			.is_none_or(|placement| placement.schema_uncertain),
		migrations: Json(
			placement
				.map(|placement| placement.migrations.0)
				.unwrap_or_default(),
		),
		failure: None,
		completed_at: Some(leptoskit_db::now()),
		expires_at: input.safety_expires_at,
	}
	.insert(tx)
	.await?;
	Ok(())
}

/// Gives up on `environment`'s backups and restores that no worker has
/// taken, before the environment is removed. Their rows go with it.
///
/// # Errors
///
/// When the database fails.
pub(crate) async fn abandon_queued(tx: &Tx, environment: Id<Environment>) -> rpc::Result<()> {
	let queued = Job::query()
		.filter(|job| {
			job.environment_id.eq(Some(environment))
				& (job.kind.eq(JobKind::BackupDatabase) | job.kind.eq(JobKind::RestoreDatabase))
				& job.state.eq(JobState::Queued)
		})
		.all(tx)
		.await?;

	for mut job in queued {
		job.state = JobState::Abandoned;
		job.save(tx).await?;
	}

	Ok(())
}

/// Deletes the rows of backups past their expiry: up to `batch` of them.
/// Each one's worker deletes its dump on the same date.
///
/// # Errors
///
/// When the database fails.
pub(crate) async fn sweep(db: &leptoskit_db::Db, batch: u64) -> rpc::Result<u32> {
	let now = leptoskit_db::now();
	let expired = DatabaseBackup::query()
		.filter(|row| row.expires_at.lt(now))
		.limit(batch)
		.all(db)
		.await?;
	let mut deleted = 0;

	for row in expired {
		row.destroy(db).await?;
		deleted += 1;
	}

	Ok(deleted)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::workers::MigrationFingerprint;

	fn migration(name: &str) -> MigrationFingerprint {
		MigrationFingerprint {
			name: name.to_owned(),
			checksum: format!("sum-{name}"),
		}
	}

	fn backup(migrations: &[&str]) -> DatabaseBackup {
		let now = leptoskit_db::now();
		DatabaseBackup {
			id: Id::new(),
			environment_id: Id::new(),
			organization_id: Id::new(),
			database_id: Id::new(),
			kind: BackupKind::Manual,
			state: BackupState::Complete,
			location: Some("dumps/one".to_owned()),
			bytes: Some(1),
			migrations: Json(migrations.iter().map(|name| migration(name)).collect()),
			schema_uncertain: false,
			failure: None,
			completed_at: Some(now),
			expires_at: now,
			created_at: now,
			updated_at: now,
		}
	}

	fn placement(migrations: &[&str]) -> Placement {
		let now = leptoskit_db::now();
		Placement {
			id: Id::new(),
			environment_id: Id::new(),
			cluster_id: Id::new(),
			hostname: "notes.example".to_owned(),
			migrations: Json(migrations.iter().map(|name| migration(name)).collect()),
			schema_uncertain: false,
			created_at: now,
			updated_at: now,
		}
	}

	#[test]
	fn a_backup_goes_back_only_under_a_server_that_knows_its_migrations() {
		assert_eq!(unrestorable(&backup(&[]), &placement(&[])), None);
		assert_eq!(unrestorable(&backup(&["a"]), &placement(&["a", "b"])), None);
		assert_eq!(
			unrestorable(&backup(&["a", "b"]), &placement(&["a", "b"])),
			None
		);

		let newer = unrestorable(&backup(&["a", "b"]), &placement(&["a"])).expect("refused");
		assert!(newer.contains("doesn't know (b)"), "{newer}");
		let reordered =
			unrestorable(&backup(&["b", "a"]), &placement(&["a", "b"])).expect("refused");
		assert!(reordered.contains("another order"), "{reordered}");

		let mut uncertain = backup(&["a"]);
		uncertain.schema_uncertain = true;
		assert!(unrestorable(&uncertain, &placement(&["a"])).is_some());
		let mut unsettled = placement(&["a"]);
		unsettled.schema_uncertain = true;
		assert!(unrestorable(&backup(&["a"]), &unsettled).is_some());
	}

	#[test]
	fn a_location_is_short_and_printable() {
		let taken = |location: &str| {
			BackupTaken {
				location: location.to_owned(),
				bytes: 1,
			}
		};
		assert!(keepable(&taken("s3://dumps/env_ab/0192.dump")));
		assert!(!keepable(&taken("")));
		assert!(!keepable(&taken("dumps/\u{1b}[2J")));
		assert!(!keepable(&taken(&"a".repeat(MAX_BACKUP_LOCATION + 1))));
		assert!(!keepable(&BackupTaken {
			location: "dumps/one".to_owned(),
			bytes: u64::MAX,
		}));
	}

	#[test]
	fn a_reason_is_scrubbed_and_cut() {
		let kept = reason("pg_dump: postgres://env_ab:c0ffee@host:5432/env_ab refused\u{7}");
		assert!(!kept.contains("c0ffee"), "{kept}");
		assert!(!kept.contains('\u{7}'), "{kept}");
		assert!(reason(&"é".repeat(400)).len() <= MAX_REASON_LEN + '…'.len_utf8());
	}
}
