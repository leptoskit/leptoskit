//! The bodies of [`crate::operator`]'s operations. Each one takes an
//! operator: a person in the `operators` table, never a token or a worker.

use leptoskit_auth::Account;
use leptoskit_db::Id;
use leptoskit_db::Json;
use leptoskit_rpc as rpc;
use serde::Deserialize;
use serde::Serialize;
use uuid::Uuid;

use super::audit;
use super::audit::Actor;
use super::authorize::person_required;
use super::identity::Caller;
use super::identity::caller;
use super::identity::client;
use super::is_operator;
use super::jobs;
use super::secret;
use super::services;
use crate::operator::ClusterFacts;
use crate::operator::ClusterSummary;
use crate::operator::IssuedPairing;
use crate::operator::JobState;
use crate::operator::JobSummary;
use crate::operator::Reconciliation;
use crate::slug::Slug;
use crate::tables::Cluster;
use crate::tables::Job;
use crate::tables::NewCluster;
use crate::tables::NewPairing;
use crate::tables::Organization;
use crate::tables::Placement;
use crate::tables::Worker;
use crate::workers::JobKind;
use crate::workers::PAIRING_CODE_LEN;
use crate::workers::PAIRING_CODE_PREFIX;
use crate::workers::PAIRING_MINUTES;
use crate::workers::ReleaseOutcome;
use crate::workers::WorkerRole;
use crate::workers::WorkerSummary;

/// The most jobs `jobs` lists.
const MAX_JOBS: u64 = 200;

/// The longest region name.
const MAX_REGION_LEN: usize = 32;

/// The operator calling.
///
/// # Errors
///
/// `Unauthorized` for an anonymous caller; `Forbidden` for a worker, a
/// token (`person_required`), or a person who isn't an operator
/// (`operator_required`).
async fn operator(
	db: &impl leptoskit_db::Read,
	context: &leptoskit_db::Context,
) -> rpc::Result<Id<Account>> {
	let account = match caller(context)? {
		Caller::Person { account, .. } => *account,
		Caller::Token(_) => return Err(person_required("run the cloud")),
	};

	if !is_operator(db, account).await? {
		return Err(rpc::Error::forbidden(
			"operator_required: only the people who run the cloud can do this",
		));
	}

	Ok(account)
}

async fn cluster_summary(
	db: &impl leptoskit_db::Read,
	cluster: &Cluster,
) -> rpc::Result<ClusterSummary> {
	let id = cluster.id;
	let environments = Placement::query()
		.filter(|placement| placement.cluster_id.eq(id))
		.count(db)
		.await?;
	Ok(ClusterSummary {
		id: cluster.id.uuid(),
		name: cluster.name.clone(),
		region: cluster.region.clone(),
		facts: cluster.facts.0.clone(),
		capacity_notes: cluster.capacity_notes.clone(),
		environments,
		created_at: cluster.created_at,
	})
}

pub(crate) async fn clusters(cx: &rpc::QueryCx) -> rpc::Result<Vec<ClusterSummary>> {
	operator(cx.db(), cx.context()).await?;
	let rows = Cluster::query()
		.order_by(|cluster| cluster.created_at.asc())
		.all(cx.db())
		.await?;
	let mut out = Vec::with_capacity(rows.len());

	for row in &rows {
		out.push(cluster_summary(cx.db(), row).await?);
	}

	Ok(out)
}

pub(crate) async fn register_cluster(
	cx: &rpc::MutationCx,
	name: String,
	region: String,
	facts: ClusterFacts,
	capacity_notes: String,
) -> rpc::Result<ClusterSummary> {
	let account = operator(cx.tx(), cx.context()).await?;
	Slug::parse(&name).map_err(|error| rpc::Error::invalid(format!("invalid_slug: {error}")))?;
	let region = crate::size::Region::parse(&region)
		.map_err(|error| rpc::Error::invalid(format!("invalid_region: {error}")))?;

	if region.as_str().len() > MAX_REGION_LEN {
		return Err(rpc::Error::invalid(format!(
			"invalid_region: a region's name is at most {MAX_REGION_LEN} characters"
		)));
	}

	if let Some(class) = &facts.runtime_class
		&& !crate::release::is_dns_label(class)
	{
		return Err(rpc::Error::invalid(
			"invalid_facts: a runtime class is a DNS label, such as `gvisor`",
		));
	}

	let cluster = NewCluster {
		name: name.clone(),
		region: region.as_str().to_owned(),
		facts: Json(facts),
		capacity_notes: capacity_notes.trim().to_owned(),
	}
	.insert(cx.tx())
	.await
	.map_err(|error| {
		if error.is_conflict() {
			rpc::Error::conflict(format!("name_taken: a cluster is called `{name}`"))
		} else {
			error.into()
		}
	})?;
	audit::record(
		cx.tx(),
		&Actor::Person(account),
		"cluster.register",
		None,
		None,
		Some(format!("{name} in {}", region.as_str())),
		&client(cx.context()),
	)
	.await?;
	cluster_summary(cx.tx(), &cluster).await
}

pub(crate) async fn workers(cx: &rpc::QueryCx) -> rpc::Result<Vec<WorkerSummary>> {
	operator(cx.db(), cx.context()).await?;
	let rows = Worker::query()
		.order_by(|worker| worker.created_at.desc())
		.all(cx.db())
		.await?;
	let mut out = Vec::with_capacity(rows.len());

	for row in &rows {
		out.push(super::workers::summary(cx.db(), row).await?);
	}

	Ok(out)
}

pub(crate) async fn jobs(
	cx: &rpc::QueryCx,
	state: Option<JobState>,
) -> rpc::Result<Vec<JobSummary>> {
	operator(cx.db(), cx.context()).await?;
	let rows = Job::query()
		.filter(|job| {
			match state {
				Some(state) => job.state.eq(state),
				None => job.state.ne(JobState::Done) & job.state.ne(JobState::Abandoned),
			}
		})
		.order_by(|job| job.created_at.desc())
		.limit(MAX_JOBS)
		.all(cx.db())
		.await?;
	let mut out = Vec::with_capacity(rows.len());

	for row in &rows {
		out.push(jobs::summary(cx.db(), row).await?);
	}

	Ok(out)
}

pub(crate) async fn issue_pairing(
	cx: &rpc::ActionCx,
	role: WorkerRole,
	organization: Option<String>,
	cluster: Option<String>,
) -> rpc::Result<IssuedPairing> {
	cx.mutate(IssuePairing {
		role,
		organization,
		cluster,
	})
	.await
}

/// `issue_pairing`'s writes. Never registered, so the code is never kept
/// as a receipt.
#[derive(Clone, Serialize, Deserialize)]
struct IssuePairing {
	role: WorkerRole,
	organization: Option<String>,
	cluster: Option<String>,
}

impl rpc::Operation for IssuePairing {
	type Context = rpc::MutationCx;
	type Output = IssuedPairing;

	const KIND: rpc::Kind = rpc::Kind::Mutation;
	const NAME: &'static str = "issue_pairing";

	fn run(self, cx: &rpc::MutationCx) -> impl Future<Output = rpc::Result<IssuedPairing>> + Send {
		issue(cx, self)
	}
}

async fn issue(cx: &rpc::MutationCx, args: IssuePairing) -> rpc::Result<IssuedPairing> {
	let account = operator(cx.tx(), cx.context()).await?;

	match (args.role, &args.organization, &args.cluster) {
		(WorkerRole::Build, _, None) | (WorkerRole::Cluster, None, Some(_)) => {}
		(WorkerRole::Build, _, Some(_)) => {
			return Err(rpc::Error::invalid(
				"invalid_pairing: a build worker serves a pool, not a cluster",
			));
		}
		(WorkerRole::Cluster, _, None) => {
			return Err(rpc::Error::invalid(
				"invalid_pairing: a cluster worker needs its `cluster`",
			));
		}
		(WorkerRole::Cluster, Some(_), Some(_)) => {
			return Err(rpc::Error::invalid(
				"invalid_pairing: a cluster worker serves every organisation on its cluster",
			));
		}
	}

	let organization = match &args.organization {
		Some(slug) => {
			Some(
				Organization::query()
					.filter(|row| row.slug.eq(slug.as_str()))
					.first(cx.tx())
					.await?
					.ok_or_else(|| {
						rpc::Error::not_found(format!(
							"organization_not_found: there is no organisation `{slug}`"
						))
					})?,
			)
		}
		None => None,
	};
	let cluster = match &args.cluster {
		Some(name) => {
			Some(
				Cluster::query()
					.filter(|row| row.name.eq(name.as_str()))
					.first(cx.tx())
					.await?
					.ok_or_else(|| {
						rpc::Error::not_found(format!(
							"cluster_not_found: there is no cluster `{name}`"
						))
					})?,
			)
		}
		None => None,
	};
	let code = format!(
		"{PAIRING_CODE_PREFIX}{}",
		secret::random_hex(PAIRING_CODE_LEN / 2)
			.map_err(|_| rpc::Error::server("no randomness to make a code with"))?
	);
	let expires_at = leptoskit_db::now() + chrono::TimeDelta::minutes(PAIRING_MINUTES);
	NewPairing {
		code_hash: secret::hash(&code),
		role: args.role,
		organization_id: organization.as_ref().map(|row| row.id),
		cluster_id: cluster.as_ref().map(|row| row.id),
		expires_at,
		used_at: None,
		worker_id: None,
		issued_by: account,
	}
	.insert(cx.tx())
	.await?;
	audit::record(
		cx.tx(),
		&Actor::Person(account),
		"worker.pairing",
		organization.as_ref().map(|row| row.id),
		None,
		Some(format!(
			"{} worker{}",
			args.role,
			cluster
				.as_ref()
				.map(|row| format!(" for {}", row.name))
				.unwrap_or_default()
		)),
		&client(cx.context()),
	)
	.await?;
	Ok(IssuedPairing {
		code,
		role: args.role,
		organization: args.organization,
		cluster: args.cluster,
		expires_at,
	})
}

pub(crate) async fn revoke_worker(
	cx: &rpc::MutationCx,
	worker: Uuid,
) -> rpc::Result<WorkerSummary> {
	let account = operator(cx.tx(), cx.context()).await?;
	let id = Id::<Worker>::from_uuid(worker);
	let mut row = Worker::query()
		.filter(|row| row.id.eq(id) & row.revoked_at.is_null())
		.first(cx.tx())
		.await?
		.ok_or_else(|| {
			rpc::Error::not_found(format!(
				"worker_not_found: there is no live worker {worker}"
			))
		})?;
	row.revoked_at = Some(leptoskit_db::now());
	row.save(cx.tx()).await?;
	let held = Job::query()
		.filter(|job| job.worker_id.eq(Some(id)) & job.state.eq(JobState::Leased))
		.all(cx.tx())
		.await?;
	let mut requeued = false;

	for job in held {
		requeued |=
			jobs::lapse(cx.tx(), job, "its worker was revoked").await? == jobs::Lapsed::Requeued;
	}

	audit::record(
		cx.tx(),
		&Actor::Person(account),
		"worker.revoke",
		row.organization_id,
		None,
		Some(format!("{} {}", row.id, row.name)),
		&client(cx.context()),
	)
	.await?;

	if requeued {
		let waker = services(cx.context())?.clone();
		cx.after_commit(async move { jobs::wake(&waker) });
	}

	super::workers::summary(cx.tx(), &row).await
}

pub(crate) async fn reconcile(
	cx: &rpc::MutationCx,
	job: Uuid,
	found: Reconciliation,
) -> rpc::Result<JobSummary> {
	let account = operator(cx.tx(), cx.context()).await?;
	let mut row = Job::query()
		.filter(|row| row.id.eq(Id::<Job>::from_uuid(job)))
		.first(cx.tx())
		.await?
		.ok_or_else(|| rpc::Error::not_found(format!("job_not_found: there is no job {job}")))?;

	if row.state != JobState::Unknown {
		return Err(rpc::Error::conflict(format!(
			"not_unknown: job {job} is {}, so there is nothing to reconcile",
			row.state
		)));
	}

	row.lease_expires_at = None;
	let mut woke = false;

	match (&row.kind, &found) {
		(JobKind::Release, Reconciliation::Applied) => {
			row.state = JobState::Done;
			row.save(cx.tx()).await?;
			super::deployments::release_finished(cx.tx(), &row, &ReleaseOutcome::Applied).await?;
		}
		(JobKind::Release, Reconciliation::NotApplied { detail }) => {
			row.state = JobState::Done;
			row.save(cx.tx()).await?;
			super::deployments::release_interrupted(cx.tx(), &row, detail).await?;
		}
		(JobKind::DestroyEnvironment, Reconciliation::Applied) => {
			row.state = JobState::Done;
			row.save(cx.tx()).await?;
		}
		(JobKind::DestroyEnvironment, Reconciliation::NotApplied { .. }) => {
			// Removing an environment is safe to run again.
			row.state = JobState::Queued;
			row.worker_id = None;
			row.queued_at = leptoskit_db::now();
			row.save(cx.tx()).await?;
			woke = true;
		}
		(JobKind::BuildServer, _) => {
			return Err(rpc::Error::server("a build is never unknown"));
		}
	}

	audit::record(
		cx.tx(),
		&Actor::Person(account),
		"job.reconcile",
		Some(row.organization_id),
		None,
		Some(format!(
			"{} {}: {}",
			row.kind,
			row.id,
			match &found {
				Reconciliation::Applied => "applied".to_owned(),
				Reconciliation::NotApplied { detail } => format!("not applied ({detail})"),
			}
		)),
		&client(cx.context()),
	)
	.await?;

	if woke {
		let waker = services(cx.context())?.clone();
		cx.after_commit(async move { jobs::wake(&waker) });
	}

	jobs::summary(cx.tx(), &row).await
}
