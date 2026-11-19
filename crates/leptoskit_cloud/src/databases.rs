//! An environment's managed Postgres database.
//!
//! An app whose `[db]` is Postgres gets one database per environment, made
//! the first time the environment is deployed: a database and a login role
//! of its own on its cluster's shared Postgres server. Its processes start
//! with `DATABASE_URL`, which holds the role's password, so it travels as a
//! secret does: named in the release's spec, and given by value only to the
//! cluster worker that claims the release.
//!
//! Nothing here answers the password or the address. The database stays
//! with the environment, whatever later deployments do. When the
//! environment is deleted, the control plane forgets it and tells the
//! cluster's worker to drop it.
//!
//! # Backups
//!
//! [`create_backup`] asks the cluster's worker for a dump of the database,
//! which the worker keeps for [`BACKUP_KEEP_DAYS`] days. The control plane
//! keeps a record of it and never its bytes, and nothing answers where a
//! dump is. [`restore_backup`] puts one back over the database it was taken
//! from, after the worker has taken a dump of what it replaces. Nothing
//! takes backups on a schedule yet.

use leptoskit_db::Timestamp;
use leptoskit_rpc as rpc;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use uuid::Uuid;

use crate::size::Backups;
use crate::size::DatabaseSize;

/// An environment's database.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DatabaseSummary {
	/// The database's name on its server, which is its role's name too.
	pub name: String,
	pub size: DatabaseSize,
	/// What `[cloud.database] backups` asks for. Nothing takes backups on
	/// a schedule yet: [`create_backup`] takes one when asked.
	pub backups: Backups,
	/// Whether a release that uses it has been applied: until one has, the
	/// database may not exist on the server yet.
	pub ready: bool,
	pub created_at: Timestamp,
}

/// An environment's managed Postgres database, or `None` when it has none:
/// its app has no `[db]`, keeps SQLite, or hasn't been deployed.
///
/// # Errors
///
/// `NotFound` for an environment the caller can't see.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "databases")]
#[rpc::query]
pub async fn database(
	cx: &rpc::QueryCx,
	organization: String,
	project: String,
	environment: String,
) -> rpc::Result<Option<DatabaseSummary>> {
	crate::server::databases::database(cx, &organization, &project, &environment).await
}

/// How many days a backup is kept.
pub const BACKUP_KEEP_DAYS: u32 = 30;

/// The most backups taken on request an environment keeps at once. One
/// that failed doesn't count, and neither does the one a restore takes.
pub const MAX_BACKUPS: u64 = 10;

/// Why a backup was taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, leptoskit_db::Enum)]
pub enum BackupKind {
	/// Someone asked for it with [`create_backup`].
	#[db(rename = "manual")]
	Manual,
	/// A restore took it of the data it replaced.
	#[db(rename = "before_restore")]
	BeforeRestore,
}

/// Where a backup is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, leptoskit_db::Enum)]
pub enum BackupState {
	/// Its cluster's worker hasn't taken it yet.
	#[db(rename = "pending")]
	Pending,
	/// The worker keeps it, and it can be restored.
	#[db(rename = "complete")]
	Complete,
	/// The worker couldn't take it, or went quiet while it did.
	#[db(rename = "failed")]
	Failed,
}

/// One backup of an environment's database. Never where it is kept.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BackupSummary {
	pub id: Uuid,
	pub kind: BackupKind,
	pub state: BackupState,
	/// How large the dump is, once it is complete.
	pub bytes: Option<u64>,
	/// Why it failed.
	pub failure: Option<String>,
	pub created_at: Timestamp,
	pub completed_at: Option<Timestamp>,
	/// When it is deleted.
	pub expires_at: Timestamp,
}

/// Where a restore is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RestoreState {
	/// Its cluster's worker hasn't finished it.
	Pending,
	/// The environment runs on the backup's data.
	Restored,
	/// The database is as it was before the restore.
	Failed,
	/// The worker went quiet during it, and an operator is finding out
	/// which database the environment runs on.
	Unknown,
}

/// A restore of a backup over the environment's database.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RestoreSummary {
	/// The backup it puts back.
	pub backup: Uuid,
	pub state: RestoreState,
	/// Why it failed.
	pub failure: Option<String>,
	pub requested_at: Timestamp,
	pub updated_at: Timestamp,
}

/// An environment's backups, and its latest restore.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BackupList {
	/// Newest first.
	pub backups: Vec<BackupSummary>,
	/// The latest restore asked for, whatever became of it.
	pub restore: Option<RestoreSummary>,
}

/// An environment's backups that haven't expired, newest first, and its
/// latest restore.
///
/// # Errors
///
/// `NotFound` for an environment the caller can't see.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "databases")]
#[rpc::query]
pub async fn database_backups(
	cx: &rpc::QueryCx,
	organization: String,
	project: String,
	environment: String,
) -> rpc::Result<BackupList> {
	crate::server::backups::database_backups(cx, &organization, &project, &environment).await
}

/// Asks the environment's cluster for a backup of its database. It is
/// `pending` until the cluster's worker has taken it, and kept for
/// [`BACKUP_KEEP_DAYS`] days.
///
/// # Errors
///
/// `Conflict`: `no_database` when the environment has none;
/// `database_not_ready` until a release that uses it has been applied;
/// `backup_in_progress` while another backup or a restore is under way;
/// `too_many_backups` at [`MAX_BACKUPS`].
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "databases")]
#[rpc::mutation]
pub async fn create_backup(
	cx: &rpc::MutationCx,
	organization: String,
	project: String,
	environment: String,
) -> rpc::Result<BackupSummary> {
	crate::server::backups::create_backup(cx, &organization, &project, &environment).await
}

/// Puts `backup` back over the environment's database: everything written
/// since it was taken is replaced. Type `<project>/<environment>` back as
/// `confirm`. The cluster's worker first takes a backup of what it
/// replaces, which shows among the environment's backups once the restore
/// is done. Owners and admins only.
///
/// # Errors
///
/// `Invalid` (`confirmation_mismatch`) when `confirm` isn't
/// `<project>/<environment>`. `NotFound` (`backup_not_found`) for a backup
/// that isn't this environment's. `Conflict`: `backup_unavailable` for one
/// that isn't complete, or has expired; `deployment_in_flight` while a
/// deployment is under way; `backup_in_progress` while a backup or another
/// restore is; `restore_unavailable` when the backup holds migrations the
/// live deployment doesn't know, or the database's migrations are
/// uncertain.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "databases")]
#[rpc::mutation]
pub async fn restore_backup(
	cx: &rpc::MutationCx,
	organization: String,
	project: String,
	environment: String,
	backup: Uuid,
	confirm: String,
) -> rpc::Result<RestoreSummary> {
	crate::server::backups::restore_backup(
		cx,
		&organization,
		&project,
		&environment,
		backup,
		&confirm,
	)
	.await
}
