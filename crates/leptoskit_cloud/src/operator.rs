//! What the people who run the cloud do through the API: register the
//! clusters environments run on, pair and revoke workers, see the jobs,
//! and settle a release whose worker went quiet.
//!
//! Only an operator may: a person in the `operators` table, granted from the
//! server's command line. A token never may, whatever its scopes, and every
//! change is in the audit log.
//!
//! # Placement
//!
//! An environment is placed on a cluster the first time it is deployed:
//! the cluster in the project's region with the fewest environments, ties
//! going to the oldest. It stays there. With no cluster in the region,
//! `deploy` fails with `cluster_unavailable` and makes no deployment.

use leptoskit_db::Timestamp;
use leptoskit_rpc as rpc;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use uuid::Uuid;

use crate::workers::DeploymentRef;
use crate::workers::JobKind;
use crate::workers::WorkerRole;
use crate::workers::WorkerSummary;

/// What the control plane knows about a cluster's setup, as its operator
/// registers it: enough to write a release spec it can apply.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ClusterFacts {
	/// The `RuntimeClass` every process runs under, such as `gvisor`.
	pub runtime_class: Option<String>,
	/// Whether it can wake a sleeping process on its first request (the
	/// KEDA HTTP add-on). Without it, nothing sleeps: a process that would
	/// keeps one replica.
	pub wakes_sleeping: bool,
	/// Its shared Postgres server, when it has one: environments whose app
	/// has a Postgres `[db]` get a database and a role of their own on it.
	/// Without it, such an app can't be deployed here.
	pub postgres: Option<ClusterPostgres>,
	/// Its shared notifications hub, when it runs one: environments whose
	/// app declares `[services.notifications]` are served by it. Without it,
	/// such an app can't be deployed here.
	pub notifications: Option<ClusterNotifications>,
}

/// A cluster's shared notifications hub, as the edge and processes reach
/// it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ClusterNotifications {
	/// The namespace it runs in: a release that uses it opens its
	/// processes' network to that namespace on `port`, and to nothing else
	/// there.
	pub namespace: String,
	/// The name its Service answers on, such as
	/// `hub.leptoskit-notifications.svc.cluster.local`.
	pub host: String,
	pub port: u16,
}

/// A cluster's shared Postgres server, as its processes reach it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ClusterPostgres {
	/// The namespace it runs in: a release opens its processes' network
	/// to that namespace on `port`, and to nothing else in the cluster.
	pub namespace: String,
	/// The name processes connect to, such as
	/// `postgres-rw.leptoskit-postgres.svc`.
	pub host: String,
	pub port: u16,
	/// Whether it takes TLS connections: every `DATABASE_URL` ends
	/// `?sslmode=require` or `?sslmode=disable`. It must be said. `require`
	/// encrypts and doesn't check the server's certificate.
	pub tls: bool,
}

/// A cluster environments run on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ClusterSummary {
	pub id: Uuid,
	pub name: String,
	pub region: String,
	pub facts: ClusterFacts,
	/// Whatever the operator wrote about its capacity.
	pub capacity_notes: String,
	/// How many environments are placed on it.
	pub environments: u64,
	pub created_at: Timestamp,
}

/// Where a job is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, leptoskit_db::Enum)]
pub enum JobState {
	/// Waiting for a worker.
	#[db(rename = "queued")]
	Queued,
	/// A worker holds it.
	#[db(rename = "leased")]
	Leased,
	/// Its worker went quiet, and what it did is unknown until it is
	/// reconciled.
	#[db(rename = "unknown")]
	Unknown,
	/// It finished, and its outcome is recorded.
	#[db(rename = "done")]
	Done,
	/// It was given up on: cancelled, retried too often, or its environment
	/// was deleted.
	#[db(rename = "abandoned")]
	Abandoned,
}

/// A job, as operators see it. Never its payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct JobSummary {
	pub id: Uuid,
	pub kind: JobKind,
	pub state: JobState,
	pub attempt: u32,
	/// The deployment it is for.
	pub deployment: Option<DeploymentRef>,
	/// The worker that holds or last held it.
	pub worker: Option<Uuid>,
	pub lease_expires_at: Option<Timestamp>,
	/// Why its worker said it failed, for a removal that failed: it stays
	/// unknown until an operator reconciles it.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub failure: Option<String>,
	pub created_at: Timestamp,
	pub updated_at: Timestamp,
}

/// A pairing code, shown this once. Its `Debug` never shows the code.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct IssuedPairing {
	/// `lkp_…`: give it to the worker, which pairs with it once.
	pub code: String,
	pub role: WorkerRole,
	/// The organisation whose jobs the worker will take, or `None` for the
	/// shared pool.
	pub organization: Option<String>,
	pub cluster: Option<String>,
	pub expires_at: Timestamp,
}

impl std::fmt::Debug for IssuedPairing {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_struct("IssuedPairing")
			.field("code", &"[redacted]")
			.field("role", &self.role)
			.field("expires_at", &self.expires_at)
			.finish_non_exhaustive()
	}
}

/// What an operator found running, for a job whose worker went quiet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "found", rename_all = "snake_case")]
pub enum Reconciliation {
	/// The cluster runs what the job asked for: a release is live, an
	/// environment is gone.
	Applied,
	/// It doesn't: the previous release serves, or the environment is still
	/// there. A release fails with `interrupted`; a removal is queued again.
	NotApplied { detail: String },
}

/// Every cluster, oldest first.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "operator")]
#[rpc::query]
pub async fn clusters(cx: &rpc::QueryCx) -> rpc::Result<Vec<ClusterSummary>> {
	crate::server::operator::clusters(cx).await
}

/// Registers a cluster in a region, for environments there to be placed
/// on. Its cluster worker pairs with a code issued for it.
///
/// # Arguments
///
/// - `name`: a slug, unique across the cloud, such as `eu-central-1`.
/// - `region`: such as `eu-central`.
///
/// # Errors
///
/// `Conflict` (`name_taken`) for a name a cluster has.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "operator")]
#[rpc::mutation]
pub async fn register_cluster(
	cx: &rpc::MutationCx,
	name: String,
	region: String,
	#[serde(default)] facts: ClusterFacts,
	#[serde(default)] capacity_notes: String,
) -> rpc::Result<ClusterSummary> {
	crate::server::operator::register_cluster(cx, name, region, facts, capacity_notes).await
}

/// Replaces what the control plane knows about a cluster's setup. It reaches
/// the releases planned from then on: nothing running changes.
///
/// # Errors
///
/// `NotFound` (`cluster_not_found`); `Invalid` (`invalid_facts`); `Conflict`
/// (`cluster_has_databases`) for facts without the shared Postgres server
/// while environments keep a database on it.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "operator")]
#[rpc::mutation]
pub async fn set_cluster_facts(
	cx: &rpc::MutationCx,
	name: String,
	facts: ClusterFacts,
) -> rpc::Result<ClusterSummary> {
	crate::server::operator::set_cluster_facts(cx, name, facts).await
}

/// Every worker, newest first, revoked ones included.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "operator")]
#[rpc::query]
pub async fn workers(cx: &rpc::QueryCx) -> rpc::Result<Vec<WorkerSummary>> {
	crate::server::operator::workers(cx).await
}

/// Jobs, newest first: those in `state`, or those not done.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "operator")]
#[rpc::query]
pub async fn jobs(
	cx: &rpc::QueryCx,
	#[serde(default)] state: Option<JobState>,
) -> rpc::Result<Vec<JobSummary>> {
	crate::server::operator::jobs(cx, state).await
}

/// Issues a pairing code for one worker: a build worker for the shared pool
/// or one organisation, or the cluster worker of one cluster. It works once,
/// for ten minutes.
///
/// It is an action, so the code is never kept, not even as the answer to a
/// repeated request.
///
/// # Errors
///
/// `Invalid` (`invalid_pairing`) for a cluster worker without a cluster, or
/// a build worker with one; `NotFound` for an organisation or cluster that
/// doesn't exist.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "operator")]
#[rpc::action]
pub async fn issue_pairing(
	cx: &rpc::ActionCx,
	role: WorkerRole,
	#[serde(default)] organization: Option<String>,
	#[serde(default)] cluster: Option<String>,
) -> rpc::Result<IssuedPairing> {
	crate::server::operator::issue_pairing(cx, role, organization, cluster).await
}

/// Revokes a worker: its key stops working at once, a build it held runs
/// again elsewhere, and a release it held becomes unknown.
///
/// # Errors
///
/// `NotFound` (`worker_not_found`) for a worker that doesn't exist or is
/// already revoked.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "operator")]
#[rpc::mutation]
pub async fn revoke_worker(cx: &rpc::MutationCx, worker: Uuid) -> rpc::Result<WorkerSummary> {
	crate::server::operator::revoke_worker(cx, worker).await
}

/// Settles a job whose worker went quiet, from what the operator found the
/// cluster running.
///
/// # Errors
///
/// `NotFound` (`job_not_found`); `Conflict` (`not_unknown`) for a job whose
/// outcome is known.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "operator")]
#[rpc::mutation]
pub async fn reconcile(
	cx: &rpc::MutationCx,
	job: Uuid,
	found: Reconciliation,
) -> rpc::Result<JobSummary> {
	crate::server::operator::reconcile(cx, job, found).await
}
