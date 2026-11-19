//! Jobs: what a job keeps, how one is queued, which worker may take which,
//! and the sweeper that decides what a lapsed lease means.
//!
//! A job keeps its input without presigned addresses ([`StoredInput`]):
//! `claim` makes them fresh for each attempt, so nothing kept in the
//! database grants access to the store.
//!
//! # The sweeper
//!
//! Every [`ShippingSettings::sweep`](super::ShippingSettings::sweep), and
//! whenever [`Control::sweep`](super::Control::sweep) is called, it:
//!
//! - requeues a build whose lease ran out, until it has run
//!   [`BUILD_ATTEMPTS`](crate::workers::BUILD_ATTEMPTS) times, and then fails
//!   its deployment with `builder_unavailable`;
//! - marks a release, a removal, or a restore whose lease ran out *unknown*,
//!   and never retries it or calls it failed;
//! - fails a backup whose lease ran out: nothing was promised, and another can
//!   be asked for;
//! - gives up on a backup or a restore no worker has claimed for an hour, so
//!   neither runs long after it was asked for;
//! - fails a build nobody claimed for
//!   [`ShippingSettings::unclaimed_build`](super::ShippingSettings::unclaimed_build)
//!   with `builder_unavailable`. A release waits for its cluster's worker as
//!   long as it takes: the cluster may be down, and the deployment stays
//!   `rolling_out`;
//! - deletes archives past their expiry that no deployment in flight uses, log
//!   chunks past their retention, and the rows of backups past their expiry.

use std::time::Duration;

use futures::future::BoxFuture;
use leptoskit_db::Db;
use leptoskit_db::Id;
use leptoskit_db::Json;
use leptoskit_db::Tx;
use leptoskit_rpc as rpc;
use serde::Deserialize;
use serde::Serialize;

use super::Services;
use super::backups;
use super::delta;
use super::deployments;
use super::storage::Bucket;
use crate::archive::Digest;
use crate::deployments::AppFacts;
use crate::deployments::EventKind;
use crate::operator::JobState;
use crate::operator::JobSummary;
use crate::tables::Cluster;
use crate::tables::Deployment;
use crate::tables::Environment;
use crate::tables::Job;
use crate::tables::LogChunk;
use crate::tables::LogKind;
use crate::tables::LogUsage;
use crate::tables::NewJob;
use crate::tables::Organization;
use crate::tables::Project;
use crate::tables::SourceArchive;
use crate::workers::BackupDatabaseInput;
use crate::workers::BackupOutcome;
use crate::workers::BuildServerInput;
use crate::workers::DeploymentRef;
use crate::workers::DestroyEnvironmentInput;
use crate::workers::DestroyOutcome;
use crate::workers::Download;
use crate::workers::JobInput;
use crate::workers::JobKind;
use crate::workers::JobOutcome;
use crate::workers::ReleaseInput;
use crate::workers::RestoreDatabaseInput;
use crate::workers::RestoreOutcome;
use crate::workers::WorkerRole;

/// How many times a transaction the sweeper runs is tried when another
/// gets in its way.
const RETRIES: u32 = 5;

/// The most rows of one kind one sweep handles.
const SWEEP_BATCH: u64 = 200;

/// A job's input as the database keeps it: a build names its archive by
/// key, and `claim` turns that into a download address.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StoredInput {
	/// A build of a deployment's server.
	BuildServer {
		deployment: DeploymentRef,
		/// The archive's key in the sources bucket.
		archive_key: String,
		digest: Digest,
		size: u64,
		toolchain: String,
		targets: Vec<String>,
		image_repository: String,
		app: AppFacts,
	},
	/// A release, as the worker gets it.
	Release(Box<ReleaseInput>),
	/// A removal, as the worker gets it.
	DestroyEnvironment(DestroyEnvironmentInput),
	/// A backup, as the worker gets it.
	BackupDatabase(BackupDatabaseInput),
	/// A restore, as the worker gets it. Its `location` is the cluster
	/// worker's own name for a dump, and no credential.
	RestoreDatabase(RestoreDatabaseInput),
}

impl StoredInput {
	/// The kind of job it is the input of.
	#[must_use]
	pub const fn kind(&self) -> JobKind {
		match self {
			Self::BuildServer { .. } => JobKind::BuildServer,
			Self::Release(_) => JobKind::Release,
			Self::DestroyEnvironment(_) => JobKind::DestroyEnvironment,
			Self::BackupDatabase(_) => JobKind::BackupDatabase,
			Self::RestoreDatabase(_) => JobKind::RestoreDatabase,
		}
	}
}

/// A job to queue.
pub(crate) struct Queue {
	pub(crate) organization: Id<Organization>,
	pub(crate) deployment: Option<Id<Deployment>>,
	pub(crate) environment: Option<Id<Environment>>,
	pub(crate) cluster: Option<Id<Cluster>>,
	pub(crate) requirements: Vec<String>,
	pub(crate) max_attempts: u32,
	pub(crate) input: StoredInput,
}

/// Queues a job. Wake the workers waiting to claim once the transaction
/// commits: [`wake`].
///
/// # Errors
///
/// When the database fails.
pub(crate) async fn queue(tx: &Tx, job: Queue) -> rpc::Result<Job> {
	let now = leptoskit_db::now();
	Ok(NewJob {
		kind: job.input.kind(),
		state: JobState::Queued,
		organization_id: job.organization,
		deployment_id: job.deployment,
		environment_id: job.environment,
		cluster_id: job.cluster,
		requirements: Json(job.requirements),
		input: Json(job.input),
		attempt: 0,
		max_attempts: i64::from(job.max_attempts),
		cancel_requested: false,
		worker_id: None,
		lease_expires_at: None,
		claimed_at: None,
		log_bytes: 0,
		outcome: Json(None),
		queued_at: now,
	}
	.insert(tx)
	.await?)
}

/// Wakes every worker waiting in `claim`.
pub(crate) fn wake(services: &Services) {
	services.shipping().jobs.notify_waiters();
}

/// What a worker can take, as far as its row says.
pub(crate) struct Taker<'a> {
	pub(crate) role: WorkerRole,
	pub(crate) organization: Option<Id<Organization>>,
	pub(crate) cluster: Option<Id<Cluster>>,
	pub(crate) capabilities: &'a [String],
}

/// Whether `taker` may take `job`: its role, its pool (a worker bound to an
/// organisation takes only its jobs; the shared pool takes any), its
/// cluster for a cluster job, and every capability the job requires.
pub(crate) fn takes(taker: &Taker<'_>, job: &Job) -> bool {
	if job.kind.role() != taker.role {
		return false;
	}

	if taker
		.organization
		.is_some_and(|organization| organization != job.organization_id)
	{
		return false;
	}

	if taker.role == WorkerRole::Cluster
		&& (taker.cluster.is_none() || job.cluster_id != taker.cluster)
	{
		return false;
	}

	job.requirements
		.0
		.iter()
		.all(|requirement| taker.capabilities.contains(requirement))
}

/// The input a worker gets for `job`: a build's download address is made
/// now, for `ttl`.
///
/// # Errors
///
/// When the store can't sign an address.
pub(crate) fn assignment_input(
	services: &Services,
	stored: &StoredInput,
	ttl: Duration,
) -> rpc::Result<JobInput> {
	Ok(match stored {
		StoredInput::BuildServer {
			deployment,
			archive_key,
			digest,
			size,
			toolchain,
			targets,
			image_repository,
			app,
		} => {
			let presigned =
				services
					.shipping()
					.store
					.presign_get(Bucket::Sources, archive_key, ttl)?;
			JobInput::BuildServer(BuildServerInput {
				deployment: deployment.clone(),
				archive: Download {
					url: presigned.url,
					expires_at: presigned.expires_at,
				},
				digest: digest.clone(),
				size: *size,
				toolchain: toolchain.clone(),
				targets: targets.clone(),
				image_repository: image_repository.clone(),
				app: app.clone(),
			})
		}
		StoredInput::Release(input) => JobInput::Release(input.clone()),
		StoredInput::DestroyEnvironment(input) => JobInput::DestroyEnvironment(input.clone()),
		StoredInput::BackupDatabase(input) => JobInput::BackupDatabase(input.clone()),
		StoredInput::RestoreDatabase(input) => JobInput::RestoreDatabase(input.clone()),
	})
}

/// `job` as operators see it.
///
/// # Errors
///
/// When the database fails.
pub(crate) async fn summary(db: &impl leptoskit_db::Read, job: &Job) -> rpc::Result<JobSummary> {
	let deployment = match job.deployment_id {
		Some(id) => deployment_ref(db, id).await?,
		None => None,
	};
	Ok(JobSummary {
		id: job.id.uuid(),
		kind: job.kind,
		state: job.state,
		attempt: u32::try_from(job.attempt).unwrap_or(u32::MAX),
		deployment,
		worker: job.worker_id.map(|worker| worker.uuid()),
		lease_expires_at: job.lease_expires_at,
		failure: match &job.outcome.0 {
			Some(
				JobOutcome::DestroyEnvironment(DestroyOutcome::Failed { reason })
				| JobOutcome::BackupDatabase(BackupOutcome::Failed { reason })
				| JobOutcome::RestoreDatabase(RestoreOutcome::Failed { reason }),
			) => Some(reason.clone()),
			_ => None,
		},
		created_at: job.created_at,
		updated_at: job.updated_at,
	})
}

/// Names a deployment by its organisation's, project's, and environment's
/// slugs, while it exists.
///
/// # Errors
///
/// When the database fails.
pub(crate) async fn deployment_ref(
	db: &impl leptoskit_db::Read,
	id: Id<Deployment>,
) -> rpc::Result<Option<DeploymentRef>> {
	let Some(deployment) = Deployment::query()
		.filter(|row| row.id.eq(id))
		.first(db)
		.await?
	else {
		return Ok(None);
	};
	let organization = Organization::query()
		.filter(|row| row.id.eq(deployment.organization_id))
		.one(db)
		.await?;
	let project = Project::query()
		.filter(|row| row.id.eq(deployment.project_id))
		.one(db)
		.await?;
	let environment = Environment::query()
		.filter(|row| row.id.eq(deployment.environment_id))
		.one(db)
		.await?;
	Ok(Some(DeploymentRef {
		organization: organization.slug,
		project: project.slug,
		environment: environment.name,
		number: u64::try_from(deployment.number).unwrap_or_default(),
	}))
}

/// Takes a held job back: a build is queued again for another worker while
/// it has attempts left, and fails its deployment when it hasn't; a backup
/// fails; a release, a removal, or a restore becomes unknown. `reason`
/// says why, for the deployment's history.
///
/// # Errors
///
/// When the database fails.
pub(crate) async fn lapse(tx: &Tx, mut job: Job, reason: &str) -> rpc::Result<Lapsed> {
	job.lease_expires_at = None;

	if job.kind == JobKind::BackupDatabase {
		job.state = JobState::Abandoned;
		job.save(tx).await?;
		backups::backup_abandoned(tx, &job, reason).await?;
		return Ok(Lapsed::Abandoned);
	}

	if job.kind != JobKind::BuildServer {
		job.state = JobState::Unknown;
		job.save(tx).await?;

		if job.kind == JobKind::Release {
			deployments::release_unknown(tx, &job, reason).await?;
		}

		return Ok(Lapsed::Unknown);
	}

	if job.cancel_requested || job.attempt >= job.max_attempts {
		job.state = JobState::Abandoned;
		job.save(tx).await?;
		deployments::build_abandoned(
			tx,
			&job,
			&format!("{reason}, and the build has run {} times", job.attempt),
		)
		.await?;
		return Ok(Lapsed::Abandoned);
	}

	job.state = JobState::Queued;
	job.worker_id = None;
	job.claimed_at = None;
	job.queued_at = leptoskit_db::now();
	let attempt = u32::try_from(job.attempt).unwrap_or(u32::MAX);
	job.save(tx).await?;
	deployments::record(
		tx,
		&job,
		EventKind::Retrying {
			job: job.id.uuid(),
			attempt,
			reason: reason.to_owned(),
		},
	)
	.await?;
	Ok(Lapsed::Requeued)
}

/// What [`lapse`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lapsed {
	Requeued,
	Abandoned,
	Unknown,
}

/// What one sweep did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Swept {
	/// Builds queued again after their lease ran out.
	pub retried: u32,
	/// Builds given up on: out of attempts, or never claimed. Backups
	/// whose worker went quiet, and backups and restores never claimed.
	pub abandoned: u32,
	/// Releases, removals, and restores whose outcome is now unknown.
	pub unknown: u32,
	/// Archives deleted.
	pub archives: u32,
	/// Log chunks deleted.
	pub log_chunks: u32,
	/// Expired backups forgotten.
	pub backups: u32,
}

/// Runs `work` in a serializable transaction, again when another
/// transaction gets in its way.
///
/// # Errors
///
/// What `work` fails with, or the database's error.
pub(crate) async fn serializable<T, F>(db: &Db, mut work: F) -> rpc::Result<T>
where
	F: for<'a> FnMut(&'a Tx) -> BoxFuture<'a, rpc::Result<T>>,
{
	let mut attempt = 0;

	loop {
		attempt += 1;
		let tx = db.begin_serializable().await?;
		let result = match work(&tx).await {
			Ok(value) => tx.commit().await.map(|()| value).map_err(rpc::Error::from),
			Err(error) => Err(error),
		};

		match result {
			Err(rpc::Error::Contended) if attempt < RETRIES => {}
			result => return result,
		}
	}
}

/// One sweep: see the [module docs](self).
///
/// # Errors
///
/// When the database or the store fails; what it did before stays done.
pub(crate) async fn sweep(services: &Services) -> rpc::Result<Swept> {
	let db = services.db();
	let now = leptoskit_db::now();
	let mut swept = Swept::default();
	let mut woke = false;

	let lapsed = Job::query()
		.filter(|job| job.state.eq(JobState::Leased) & job.lease_expires_at.lt(now))
		.order_by(|job| job.lease_expires_at.asc())
		.limit(SWEEP_BATCH)
		.all(db)
		.await?;

	for candidate in lapsed {
		let id = candidate.id;
		let outcome = serializable(db, move |tx| {
			Box::pin(async move {
				let now = leptoskit_db::now();
				let Some(job) = Job::query()
					.filter(|job| {
						job.id.eq(id)
							& job.state.eq(JobState::Leased)
							& job.lease_expires_at.lt(now)
					})
					.first(tx)
					.await?
				else {
					return Ok(None);
				};
				lapse(tx, job, "its worker's lease ran out").await.map(Some)
			})
		})
		.await?;

		match outcome {
			Some(Lapsed::Requeued) => {
				swept.retried += 1;
				woke = true;
			}
			Some(Lapsed::Abandoned) => swept.abandoned += 1,
			Some(Lapsed::Unknown) => swept.unknown += 1,
			None => {}
		}
	}

	let settings = services.shipping().settings;
	let stale = now - delta(settings.unclaimed_build);
	let unclaimed = Job::query()
		.filter(|job| {
			job.state.eq(JobState::Queued)
				& job.kind.eq(JobKind::BuildServer)
				& job.queued_at.lt(stale)
		})
		.limit(SWEEP_BATCH)
		.all(db)
		.await?;

	for candidate in unclaimed {
		let id = candidate.id;
		let abandoned = serializable(db, move |tx| {
			Box::pin(async move {
				let Some(mut job) = Job::query()
					.filter(|job| job.id.eq(id) & job.state.eq(JobState::Queued))
					.first(tx)
					.await?
				else {
					return Ok(false);
				};
				job.state = JobState::Abandoned;
				job.save(tx).await?;
				deployments::build_abandoned(tx, &job, "no build worker claimed it in time")
					.await?;
				Ok(true)
			})
		})
		.await?;
		swept.abandoned += u32::from(abandoned);
	}

	if woke {
		wake(services);
	}

	swept.archives = sweep_archives(services).await?;
	swept.log_chunks = sweep_logs(services).await?;
	swept.abandoned += backups::sweep_unclaimed(db, SWEEP_BATCH).await?;
	swept.backups = backups::sweep(db, SWEEP_BATCH).await?;
	Ok(swept)
}

/// Deletes archives past their expiry that no deployment in flight uses,
/// and keeps the rest a while longer.
async fn sweep_archives(services: &Services) -> rpc::Result<u32> {
	let db = services.db();
	let now = leptoskit_db::now();
	let keep = delta(services.shipping().settings.archive_keep);
	let expired = SourceArchive::query()
		.filter(|archive| archive.expires_at.lt(now))
		.limit(SWEEP_BATCH)
		.all(db)
		.await?;
	let mut deleted = 0;

	for archive in expired {
		let id = archive.id;
		let gone = serializable(db, move |tx| {
			Box::pin(async move {
				// Read again, and expired again: a `begin_upload` of the
				// same bytes since the candidates were listed renews it.
				let Some(mut archive) = SourceArchive::query()
					.filter(|row| row.id.eq(id) & row.expires_at.lt(leptoskit_db::now()))
					.first(tx)
					.await?
				else {
					return Ok(None);
				};
				let in_use = Deployment::query()
					.filter(|row| {
						row.archive_id.eq(Some(id)) & row.in_flight_environment.is_not_null()
					})
					.exists(tx)
					.await?;

				if in_use {
					archive.expires_at = leptoskit_db::now() + keep;
					archive.save(tx).await?;
					return Ok(None);
				}

				let key = super::storage::archive_key(
					archive.organization_id.uuid(),
					archive.id.uuid(),
					&Digest::parse(&archive.digest)
						.map_err(|_| rpc::Error::server("an archive's digest is malformed"))?,
				);
				archive.destroy(tx).await?;
				Ok(Some(key))
			})
		})
		.await?;

		if let Some(key) = gone {
			// The row is gone first: an object without one is unreachable.
			// The key is this row's own, so a later upload of the same bytes,
			// which makes a new row, writes a key this can't delete.
			services
				.shipping()
				.store
				.delete(Bucket::Sources, &key)
				.await?;
			deleted += 1;
		}
	}

	Ok(deleted)
}

/// Deletes log chunks past their retention, and their objects.
async fn sweep_logs(services: &Services) -> rpc::Result<u32> {
	let db = services.db();
	let now = leptoskit_db::now();
	let expired = LogChunk::query()
		.filter(|chunk| chunk.expires_at.lt(now))
		.limit(SWEEP_BATCH)
		.all(db)
		.await?;
	let mut deleted = 0;

	for chunk in expired {
		let id = chunk.id;
		let gone = serializable(db, move |tx| {
			Box::pin(async move {
				let Some(chunk) = LogChunk::query()
					.filter(|row| row.id.eq(id))
					.first(tx)
					.await?
				else {
					return Ok(None);
				};

				// A build's chunk stops counting against its organisation's
				// log cap as it goes.
				if chunk.kind == LogKind::Build {
					let organization = chunk.organization_id;

					if let Some(mut usage) = LogUsage::query()
						.filter(|usage| usage.organization_id.eq(organization))
						.first(tx)
						.await?
					{
						usage.bytes = (usage.bytes - chunk.bytes).max(0);
						usage.save(tx).await?;
					}
				}

				let key = chunk.object_key.clone();
				chunk.destroy(tx).await?;
				Ok(Some(key))
			})
		})
		.await?;

		if let Some(key) = gone {
			services.shipping().store.delete(Bucket::Logs, &key).await?;
			deleted += 1;
		}
	}

	Ok(deleted)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn job(kind: JobKind, organization: Id<Organization>, requirements: &[&str]) -> Job {
		let now = leptoskit_db::now();
		Job {
			id: Id::new(),
			kind,
			state: JobState::Queued,
			organization_id: organization,
			deployment_id: None,
			environment_id: None,
			cluster_id: None,
			requirements: Json(requirements.iter().map(|&value| value.to_owned()).collect()),
			input: Json(StoredInput::DestroyEnvironment(DestroyEnvironmentInput {
				environment: crate::workers::RuntimeValue(
					leptoskit_cloud_runtime::EnvironmentRef {
						organization: named("acme"),
						project: named("notes"),
						environment: named("production"),
					},
				),
				database: None,
			})),
			attempt: 0,
			max_attempts: 1,
			cancel_requested: false,
			worker_id: None,
			lease_expires_at: None,
			claimed_at: None,
			log_bytes: 0,
			outcome: Json(None),
			queued_at: now,
			created_at: now,
			updated_at: now,
		}
	}

	fn named(slug: &str) -> leptoskit_cloud_runtime::NamedRef {
		leptoskit_cloud_runtime::NamedRef {
			slug: slug.to_owned(),
			id: uuid::Uuid::now_v7(),
		}
	}

	#[test]
	fn a_worker_takes_its_role_pool_cluster_and_capabilities() {
		let acme = Id::new();
		let other = Id::new();
		let capabilities = ["linux-x86_64".to_owned()];
		let shared = Taker {
			role: WorkerRole::Build,
			organization: None,
			cluster: None,
			capabilities: &capabilities,
		};
		let dedicated = Taker {
			organization: Some(acme),
			..shared
		};

		assert!(takes(&shared, &job(JobKind::BuildServer, acme, &[])));
		assert!(takes(
			&shared,
			&job(JobKind::BuildServer, other, &["linux-x86_64"])
		));
		assert!(!takes(
			&shared,
			&job(JobKind::BuildServer, other, &["macos"])
		));
		assert!(takes(&dedicated, &job(JobKind::BuildServer, acme, &[])));
		assert!(!takes(&dedicated, &job(JobKind::BuildServer, other, &[])));
		assert!(!takes(&shared, &job(JobKind::Release, acme, &[])));

		let cluster = Id::new();
		let cluster_worker = Taker {
			role: WorkerRole::Cluster,
			organization: None,
			cluster: Some(cluster),
			capabilities: &[],
		};
		let mut release = job(JobKind::Release, acme, &[]);
		assert!(!takes(&cluster_worker, &release));
		release.cluster_id = Some(cluster);
		assert!(takes(&cluster_worker, &release));
		release.cluster_id = Some(Id::new());
		assert!(!takes(&cluster_worker, &release));
		assert!(!takes(
			&cluster_worker,
			&job(JobKind::BuildServer, acme, &[])
		));
	}
}
