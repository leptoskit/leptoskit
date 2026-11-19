//! The control plane's tables: organisations and who is in them,
//! invitations, API tokens, projects, environments, operators, and the
//! audit log; source archives, deployments and their events, clusters and
//! where environments are placed on them, workers and their pairing codes,
//! jobs, log chunks, and what clusters report; variables, secrets and the
//! keys they are sealed with, and the leases that say where a secret's
//! value went; managed databases and the backups taken of them; what each
//! organisation used each month, and its spend cap.
//!
//! Every key is a UUID v7, so rows order by creation. People are
//! `Id<Account>`s from `leptoskit_auth`, stored without a foreign key: a
//! membership, a token, or an audit entry outlives nothing it shouldn't when
//! an account goes, because every check reads the membership again, and the
//! audit log must keep naming who did what after they've left.
//!
//! None of these tables is `public`: clients never read them directly, only
//! through the operations, which decide who sees which rows.

// Tables are server-only; client types are plain serde types and operations
// never return rows.
use leptoskit_auth::Account;
use leptoskit_db::Id;
use leptoskit_db::Json;
use leptoskit_db::Timestamp;
use leptoskit_db::table;

use crate::access::Role;
use crate::access::TokenScope;
use crate::databases::BackupKind;
use crate::databases::BackupState;
use crate::deployment::DeploymentState;
use crate::deployment::Source;
use crate::deployments::DeployConfig;
use crate::deployments::EventKind;
use crate::deployments::Line;
use crate::deployments::Signer;
use crate::operator::ClusterFacts;
use crate::operator::JobState;
use crate::projects::EnvironmentKind;
use crate::projects::EnvironmentScale;
use crate::release::Plan;
use crate::workers::BuildServerOutput;
use crate::workers::JobKind;
use crate::workers::JobOutcome;
use crate::workers::MigrationFingerprint;
use crate::workers::WorkerRole;

/// An organisation: the tenant everything else belongs to. Each person has
/// one personal organisation, made the first time they use the cloud, and
/// may make or join others.
#[cfg_attr(feature = "server", derive(Clone))]
#[table(name = "organizations")]
pub struct Organization {
	pub id: Id<Self>,
	/// What `/<org>` and `--org` call it, unique across the cloud.
	#[db(unique, min_len = 3, max_len = 40)]
	pub slug: String,
	/// What the console shows.
	#[db(min_len = 1, max_len = 100)]
	pub name: String,
	/// The plan it is on, such as `hobby`: an identifier billing reads,
	/// never a price.
	#[db(min_len = 1, max_len = 40)]
	pub plan: String,
	/// The person whose personal organisation this is. Unique, so a person
	/// has at most one, however many first requests race to make it.
	#[db(unique, no_foreign_key)]
	pub personal_owner: Option<Id<Account>>,
	/// Who made it.
	#[db(no_foreign_key)]
	pub created_by: Id<Account>,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// Someone in an organisation, with their role. One row per person and
/// organisation.
#[table(
	name = "memberships",
	unique(organization_id, account_id),
	index(account_id)
)]
pub struct Membership {
	pub id: Id<Self>,
	#[db(on_delete = cascade)]
	pub organization_id: Id<Organization>,
	#[db(no_foreign_key)]
	pub account_id: Id<Account>,
	pub role: Role,
	#[db(created_at)]
	pub created_at: Timestamp,
	#[db(updated_at)]
	pub updated_at: Timestamp,
}

/// An invitation to join an organisation, sent by email. The link's token
/// works once, until it expires; only its SHA-256 is kept.
#[table(name = "invitations", index(organization_id, email))]
pub struct Invitation {
	pub id: Id<Self>,
	#[db(on_delete = cascade)]
	pub organization_id: Id<Organization>,
	/// The invited address, trimmed and lowercased. Accepting needs a
	/// verified address equal to it.
	#[db(min_len = 3, max_len = 320)]
	pub email: String,
	/// The role accepting gives.
	pub role: Role,
	/// SHA-256 of the link's token.
	#[db(unique, secret)]
	pub token_hash: Vec<u8>,
	pub expires_at: Timestamp,
	#[db(no_foreign_key)]
	pub invited_by: Id<Account>,
	pub accepted_at: Option<Timestamp>,
	#[db(no_foreign_key)]
	pub accepted_by: Option<Id<Account>>,
	pub revoked_at: Option<Timestamp>,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// An API token, `lkc_<prefix>_<secret>`. The prefix finds the row; only
/// the SHA-256 of the secret is kept, and the whole value is shown once,
/// when it is made.
#[table(name = "api_tokens", index(organization_id))]
pub struct ApiToken {
	pub id: Id<Self>,
	#[db(on_delete = cascade)]
	pub organization_id: Id<Organization>,
	/// The one project it may touch, when it is bound to one.
	#[db(on_delete = cascade)]
	pub project_id: Option<Id<Project>>,
	#[db(min_len = 1, max_len = 64)]
	pub name: String,
	pub scopes: Json<Vec<TokenScope>>,
	/// The non-secret part of the value, which finds the row.
	#[db(unique, min_len = 12, max_len = 12)]
	pub prefix: String,
	/// SHA-256 of the secret part.
	#[db(secret)]
	pub secret_hash: Vec<u8>,
	pub expires_at: Timestamp,
	/// When it last authenticated a request, to within a few minutes.
	pub last_used_at: Option<Timestamp>,
	pub revoked_at: Option<Timestamp>,
	/// The person it acts for: it never does more than their current role
	/// allows.
	#[db(no_foreign_key)]
	pub created_by: Id<Account>,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// A project: one app, deployed to one or more environments.
#[table(name = "projects", unique(organization_id, slug))]
pub struct Project {
	pub id: Id<Self>,
	#[db(on_delete = cascade)]
	pub organization_id: Id<Organization>,
	/// Unique within its organisation.
	#[db(min_len = 3, max_len = 40)]
	pub slug: String,
	#[db(min_len = 1, max_len = 100)]
	pub name: String,
	#[db(min_len = 2, max_len = 32)]
	pub region: String,
	/// The targets `lk cloud launch` found, such as `server` or `ios`.
	pub detected_targets: Json<Vec<String>>,
	/// The variable names `lk cloud launch` noticed in `.env`, never their
	/// values.
	pub detected_variables: Json<Vec<String>>,
	#[db(no_foreign_key)]
	pub created_by: Id<Account>,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// A running copy of a project, such as `production`. Its scale is stored
/// state, changed only with the revision it was read at.
#[table(name = "environments", unique(project_id, name))]
pub struct Environment {
	pub id: Id<Self>,
	#[db(on_delete = cascade)]
	pub project_id: Id<Project>,
	/// Unique within its project.
	#[db(min_len = 3, max_len = 40)]
	pub name: String,
	pub kind: EnvironmentKind,
	pub scale: Json<EnvironmentScale>,
	/// Bumped by every write, so a change based on an older read is a
	/// conflict instead of a lost update.
	#[db(version)]
	pub revision: i64,
	/// Whether the scale was changed here since the last deployment's
	/// `leptoskit.toml`, which the next deployment should be told about.
	pub scale_differs: bool,
	#[db(created_at)]
	pub created_at: Timestamp,
	#[db(updated_at)]
	pub updated_at: Timestamp,
}

/// Someone who runs the cloud itself. Granted and revoked from the server's
/// command line, never through an operation.
#[table(name = "operators")]
pub struct Operator {
	pub id: Id<Self>,
	#[db(unique, no_foreign_key)]
	pub account_id: Id<Account>,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// Who acted: a person, an API token, or the server's own command line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, leptoskit_db::Enum)]
pub enum ActorKind {
	#[db(rename = "person")]
	Person,
	#[db(rename = "token")]
	Token,
	#[db(rename = "system")]
	System,
}

/// One thing someone did, kept forever: who, what, in which organisation
/// and project, when, and from which client. Never a secret's value: a
/// token is named by its ID and prefix, an invitation by its ID and
/// address. Nothing updates or deletes these rows.
#[table(name = "audit_log", index(organization_id, created_at))]
pub struct AuditEvent {
	pub id: Id<Self>,
	pub actor: ActorKind,
	/// The person, or the person a token acts for.
	#[db(no_foreign_key)]
	pub account_id: Option<Id<Account>>,
	#[db(no_foreign_key)]
	pub token_id: Option<Id<ApiToken>>,
	/// What happened, such as `project.create`.
	#[db(min_len = 1, max_len = 64)]
	pub action: String,
	#[db(no_foreign_key)]
	pub organization_id: Option<Id<Organization>>,
	#[db(no_foreign_key)]
	pub project_id: Option<Id<Project>>,
	/// What it happened to, such as a member's account or a token's ID.
	#[db(max_len = 400)]
	pub target: Option<String>,
	/// Where it came from: the web, the CLI, or an API token.
	#[db(min_len = 1, max_len = 300)]
	pub client: String,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// A source archive an organisation uploaded, kept under
/// `sources/<organization>/<archive id>/<digest>` in the object store, a key
/// of its own so a deleted row's object is never a later row's. Unique by
/// organisation and digest: bytes are shared within an organisation, never
/// across them. Until `verified_at` is set it is an upload in progress,
/// and `expires_at` is when its upload address stops working; after, it is
/// when the archive is deleted unless a deployment uses it.
#[table(
	name = "source_archives",
	unique(organization_id, digest),
	index(expires_at)
)]
pub struct SourceArchive {
	pub id: Id<Self>,
	#[db(on_delete = cascade)]
	pub organization_id: Id<Organization>,
	/// `sha256:<hex>`.
	#[db(min_len = 71, max_len = 71)]
	pub digest: String,
	/// The size it was promised, then checked, in bytes.
	pub size: i64,
	pub verified_at: Option<Timestamp>,
	pub expires_at: Timestamp,
	#[db(no_foreign_key)]
	pub created_by: Id<Account>,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// A Kubernetes cluster environments run on, registered by an operator.
#[table(name = "clusters")]
pub struct Cluster {
	pub id: Id<Self>,
	/// Unique across the cloud, such as `eu-central-1`.
	#[db(unique, min_len = 3, max_len = 40)]
	pub name: String,
	#[db(min_len = 2, max_len = 32)]
	pub region: String,
	pub facts: Json<ClusterFacts>,
	#[db(max_len = 2_000)]
	pub capacity_notes: String,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// Where an environment runs: its cluster, its hostname, and the migrations
/// its database has, as far as the control plane knows. Made by its first
/// deployment.
#[table(name = "environment_placements")]
pub struct Placement {
	pub id: Id<Self>,
	#[db(unique, on_delete = cascade)]
	pub environment_id: Id<Environment>,
	pub cluster_id: Id<Cluster>,
	/// Unique across the cloud.
	#[db(unique, min_len = 3, max_len = 253)]
	pub hostname: String,
	/// The migrations the last live deployment carried, which its server
	/// applied as it started.
	pub migrations: Json<Vec<MigrationFingerprint>>,
	/// Whether a release that may have applied migrations failed or was
	/// interrupted, so the database may have more than `migrations` says.
	pub schema_uncertain: bool,
	#[db(created_at)]
	pub created_at: Timestamp,
	#[db(updated_at)]
	pub updated_at: Timestamp,
}

/// One deployment of an environment, numbered from 1 in it. Its state
/// changes only in `server::lifecycle`, each change with its event.
#[table(
	name = "deployments",
	live,
	unique(environment_id, number),
	index(organization_id, created_at),
	index(project_id, created_at)
)]
pub struct Deployment {
	pub id: Id<Self>,
	#[db(on_delete = cascade)]
	pub organization_id: Id<Organization>,
	#[db(on_delete = cascade)]
	pub project_id: Id<Project>,
	#[db(on_delete = cascade)]
	pub environment_id: Id<Environment>,
	pub number: i64,
	pub state: Json<DeploymentState>,
	/// The environment, while this deployment is queued, building,
	/// migrating, or rolling out: unique, so an environment has one
	/// deployment in flight, whoever races to make a second.
	#[db(unique, no_foreign_key)]
	pub in_flight_environment: Option<Id<Environment>>,
	/// The environment, while this deployment is live: unique, so an
	/// environment has one.
	#[db(unique, no_foreign_key)]
	pub live_environment: Option<Id<Environment>>,
	pub source: Json<Source>,
	/// The archive an upload source uses, kept while this deployment needs
	/// it.
	#[db(no_foreign_key)]
	pub archive_id: Option<Id<SourceArchive>>,
	/// The number of the deployment a rollback returns to.
	pub rollback_of: Option<i64>,
	pub signer: Json<Signer>,
	pub config: Json<DeployConfig>,
	pub plan: Json<Plan>,
	pub lines: Json<Vec<Line>>,
	pub fine_print: Json<Vec<String>>,
	pub cluster_id: Id<Cluster>,
	#[db(min_len = 3, max_len = 253)]
	pub hostname: String,
	/// What the build made, once it has.
	pub build: Json<Option<BuildServerOutput>>,
	/// The spec its release applies, once built.
	pub spec: Json<Option<leptoskit_cloud_runtime::ReleaseSpec>>,
	/// Whether its release's worker went quiet and what the cluster runs is
	/// unknown.
	pub unknown: bool,
	/// The number of the deployment that replaced it.
	pub superseded_by: Option<i64>,
	#[db(created_at)]
	pub created_at: Timestamp,
	#[db(updated_at)]
	pub updated_at: Timestamp,
}

/// One thing that happened to a deployment, numbered from 1.
#[table(name = "deployment_events", live, unique(deployment_id, seq))]
pub struct EventRecord {
	pub id: Id<Self>,
	#[db(on_delete = cascade)]
	pub deployment_id: Id<Deployment>,
	pub seq: i64,
	pub event: Json<EventKind>,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// A worker: a build worker for the shared pool or one organisation, or a
/// cluster's worker. Its key is `lkw_<prefix>_<secret>`; only the SHA-256
/// of the secret is kept.
#[table(name = "workers")]
pub struct Worker {
	pub id: Id<Self>,
	#[db(min_len = 1, max_len = 64)]
	pub name: String,
	pub role: WorkerRole,
	/// The organisation whose jobs it takes, or `None` for the shared pool.
	#[db(on_delete = cascade)]
	pub organization_id: Option<Id<Organization>>,
	/// The cluster it applies releases to.
	pub cluster_id: Option<Id<Cluster>>,
	#[db(unique, min_len = 12, max_len = 12)]
	pub prefix: String,
	#[db(secret)]
	pub secret_hash: Vec<u8>,
	pub capabilities: Json<Vec<String>>,
	pub last_seen_at: Option<Timestamp>,
	pub revoked_at: Option<Timestamp>,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// A one-use code that pairs one worker, for ten minutes. Only its SHA-256
/// is kept.
#[table(name = "worker_pairings")]
pub struct Pairing {
	pub id: Id<Self>,
	#[db(unique, secret)]
	pub code_hash: Vec<u8>,
	pub role: WorkerRole,
	#[db(on_delete = cascade)]
	pub organization_id: Option<Id<Organization>>,
	pub cluster_id: Option<Id<Cluster>>,
	pub expires_at: Timestamp,
	pub used_at: Option<Timestamp>,
	/// The worker it paired.
	#[db(no_foreign_key)]
	pub worker_id: Option<Id<Worker>>,
	#[db(no_foreign_key)]
	pub issued_by: Id<Account>,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// A job for a worker. Its input is kept without presigned addresses, which
/// `claim` makes fresh for each attempt.
#[table(name = "jobs", index(state, created_at), index(deployment_id))]
pub struct Job {
	pub id: Id<Self>,
	pub kind: JobKind,
	pub state: JobState,
	/// The organisation it is for: a worker bound to one takes only its
	/// jobs, and a shared-pool worker any.
	#[db(no_foreign_key)]
	pub organization_id: Id<Organization>,
	#[db(no_foreign_key)]
	pub deployment_id: Option<Id<Deployment>>,
	#[db(no_foreign_key)]
	pub environment_id: Option<Id<Environment>>,
	/// The cluster whose worker takes it, for a cluster job.
	#[db(no_foreign_key)]
	pub cluster_id: Option<Id<Cluster>>,
	/// The capabilities a worker needs to take it.
	pub requirements: Json<Vec<String>>,
	pub input: Json<crate::server::StoredInput>,
	/// Runs so far.
	pub attempt: i64,
	pub max_attempts: i64,
	/// Its deployment was cancelled: `renew` tells the worker to stop.
	pub cancel_requested: bool,
	/// The worker that holds it, or last held it.
	#[db(no_foreign_key)]
	pub worker_id: Option<Id<Worker>>,
	pub lease_expires_at: Option<Timestamp>,
	/// When the attempt now held was claimed: a build's lease is never
	/// renewed past this and the build deadline.
	pub claimed_at: Option<Timestamp>,
	/// The log bytes its attempts have sent, held to the build log cap.
	pub log_bytes: i64,
	pub outcome: Json<Option<JobOutcome>>,
	/// When it first went unclaimed: a build nobody claims for long enough
	/// fails with `builder_unavailable`.
	pub queued_at: Timestamp,
	#[db(created_at)]
	pub created_at: Timestamp,
	#[db(updated_at)]
	pub updated_at: Timestamp,
}

/// Which log a chunk belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, leptoskit_db::Enum)]
pub enum LogKind {
	/// A deployment's build.
	#[db(rename = "build")]
	Build,
	/// An environment's processes.
	#[db(rename = "runtime")]
	Runtime,
}

/// A chunk of a log, kept in the object store under `object_key`, with its
/// place in the log: its lines are `first_line` to `first_line + lines -
/// 1`. Deleted with its object when it expires.
#[table(
	name = "log_chunks",
	live,
	unique(job_id, attempt, seq),
	index(deployment_id, first_line),
	index(environment_id, first_line),
	index(expires_at)
)]
pub struct LogChunk {
	pub id: Id<Self>,
	pub kind: LogKind,
	#[db(no_foreign_key)]
	pub organization_id: Id<Organization>,
	#[db(no_foreign_key)]
	pub environment_id: Id<Environment>,
	/// The deployment, for a build log.
	#[db(no_foreign_key)]
	pub deployment_id: Option<Id<Deployment>>,
	/// The job and its attempt, for a build log.
	#[db(no_foreign_key)]
	pub job_id: Option<Id<Job>>,
	pub attempt: i64,
	pub seq: i64,
	pub first_line: i64,
	pub lines: i64,
	/// The bytes of its lines as the worker sent them, counted against the
	/// organisation's log cap until the chunk is deleted.
	pub bytes: i64,
	/// SHA-256 of the lines as the worker sent them, so a chunk sent again
	/// is recognised.
	#[db(min_len = 64, max_len = 64)]
	pub digest: String,
	#[db(min_len = 1, max_len = 300)]
	pub object_key: String,
	pub expires_at: Timestamp,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// The build log bytes an organisation keeps: every chunk's bytes from
/// when it is indexed until it is deleted, held to the organisation's log
/// cap. One row per organisation, made by its first chunk.
#[table(name = "log_usage")]
pub struct LogUsage {
	pub id: Id<Self>,
	#[db(unique, no_foreign_key)]
	pub organization_id: Id<Organization>,
	pub bytes: i64,
}

/// What a cluster worker last reported an environment runs.
#[table(name = "environment_statuses", live)]
pub struct StatusReport {
	pub id: Id<Self>,
	#[db(unique, on_delete = cascade)]
	pub environment_id: Id<Environment>,
	#[db(no_foreign_key)]
	pub cluster_id: Id<Cluster>,
	pub observed: Json<leptoskit_cloud_runtime::Observed>,
	pub reported_at: Timestamp,
	/// The time the environment's compute is counted up to: `reported_at`,
	/// less the part of a second the last report left over.
	pub metered_at: Option<Timestamp>,
}

/// A variable an environment's processes start with: a name and a value
/// anyone who can see the project can read. What mustn't be read is a
/// [`Secret`].
#[table(name = "variables", unique(environment_id, name))]
pub struct Variable {
	pub id: Id<Self>,
	#[db(on_delete = cascade)]
	pub environment_id: Id<Environment>,
	#[db(min_len = 1, max_len = 128)]
	pub name: String,
	#[db(max_len = 32_768)]
	pub value: String,
	#[db(no_foreign_key)]
	pub updated_by: Id<Account>,
	#[db(created_at)]
	pub created_at: Timestamp,
	#[db(updated_at)]
	pub updated_at: Timestamp,
}

/// An organisation's data key, wrapped by a master key: what its secrets
/// are sealed with. One row per organisation, made by its first secret.
#[table(name = "vault_data_keys")]
pub struct DataKey {
	pub id: Id<Self>,
	#[db(unique, on_delete = cascade)]
	pub organization_id: Id<Organization>,
	/// The key, sealed to this organisation by the master key its header
	/// names.
	#[db(secret)]
	pub wrapped: Vec<u8>,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// A secret an environment's processes start with. The value is sealed with
/// the organisation's data key, bound to this row's organisation, project,
/// environment, name, and version, and no operation returns it.
#[table(name = "secrets", unique(environment_id, name))]
pub struct Secret {
	pub id: Id<Self>,
	#[db(on_delete = cascade)]
	pub environment_id: Id<Environment>,
	#[db(min_len = 1, max_len = 128)]
	pub name: String,
	/// Counts the values this name has had here, from 1.
	pub version: i64,
	#[db(secret)]
	pub sealed: Vec<u8>,
	#[db(no_foreign_key)]
	pub updated_by: Id<Account>,
	#[db(created_at)]
	pub created_at: Timestamp,
	#[db(updated_at)]
	pub updated_at: Timestamp,
}

/// A custom domain of an environment. It routes nothing until it is
/// verified, and a verified hostname is one environment's in the whole
/// cloud: `claim` holds it, uniquely.
#[table(name = "domains", unique(environment_id, hostname), index(hostname))]
pub struct Domain {
	pub id: Id<Self>,
	/// Not a foreign key: a hostname the cluster may still route outlives
	/// its environment's row, until the cluster's worker has removed it.
	#[db(no_foreign_key)]
	pub environment_id: Id<Environment>,
	#[db(min_len = 3, max_len = 253)]
	pub hostname: String,
	/// What the challenge record must hold. It isn't a secret: it is
	/// published in DNS.
	#[db(min_len = 32, max_len = 64)]
	pub token: String,
	/// The hostname again, from when it is verified until the row goes:
	/// unique, so two environments never hold one hostname, whoever races
	/// to verify it.
	#[db(unique, max_len = 253)]
	pub claim: Option<String>,
	pub verified_at: Option<Timestamp>,
	/// Whether a release that routes it has been handed to the cluster,
	/// applied or not: the cluster may answer on it. Cleared only when a
	/// release without it is applied.
	pub routed: bool,
	/// Detached, or its environment deleted, while the cluster may still
	/// route it. The row, and its claim, go when a release without it is
	/// applied or the environment is removed from the cluster.
	pub removing: bool,
	#[db(no_foreign_key)]
	pub created_by: Id<Account>,
	#[db(created_at)]
	pub created_at: Timestamp,
	#[db(updated_at)]
	pub updated_at: Timestamp,
}

/// An environment's managed Postgres database: a database and a login role
/// of the same name on its cluster's shared server. The cluster's worker
/// makes them; this row is what the control plane decided, and the role's
/// password, sealed with the organisation's data key and bound to this row.
/// No operation returns the password.
#[table(name = "databases")]
pub struct Database {
	pub id: Id<Self>,
	#[db(unique, on_delete = cascade)]
	pub environment_id: Id<Environment>,
	#[db(no_foreign_key)]
	pub organization_id: Id<Organization>,
	#[db(no_foreign_key)]
	pub cluster_id: Id<Cluster>,
	/// The database's name and its role's, from the environment's ID:
	/// never another environment's, and never used again.
	#[db(unique, min_len = 8, max_len = 63)]
	pub name: String,
	/// The size and backups its latest deployment planned.
	pub settings: Json<crate::release::PlannedDatabase>,
	/// Counts the passwords the role has had, from 1.
	pub credential_version: i64,
	#[db(secret)]
	pub sealed_password: Vec<u8>,
	/// Whether a release that uses it has been applied.
	pub ready: bool,
	#[db(created_at)]
	pub created_at: Timestamp,
	#[db(updated_at)]
	pub updated_at: Timestamp,
}

/// How much of one meter an organisation's project used in one calendar
/// month (UTC). Not a foreign key to anything: usage outlives the project
/// it was for.
#[table(
	name = "usage_totals",
	unique(organization_id, project_id, period, meter),
	index(organization_id, period)
)]
pub struct UsageTotal {
	pub id: Id<Self>,
	#[db(no_foreign_key)]
	pub organization_id: Id<Organization>,
	/// The nil ID when the project was gone before the usage was counted.
	#[db(no_foreign_key)]
	pub project_id: Id<Project>,
	/// The month, as `2026-11`.
	#[db(min_len = 7, max_len = 7)]
	pub period: String,
	/// The meter's code, such as `build_minutes_linux`.
	#[db(min_len = 1, max_len = 40)]
	pub meter: String,
	pub quantity: i64,
	#[db(created_at)]
	pub created_at: Timestamp,
	#[db(updated_at)]
	pub updated_at: Timestamp,
}

/// What an organisation has decided about its bill: so far, its own spend
/// cap.
#[table(name = "billing_accounts")]
pub struct BillingAccount {
	pub id: Id<Self>,
	#[db(unique, on_delete = cascade)]
	pub organization_id: Id<Organization>,
	/// The most a month's usage may cost beyond what the plan includes, in
	/// the catalogue's minor units, or `None` for the plan's own cap.
	pub spend_cap: Option<i64>,
	#[db(created_at)]
	pub created_at: Timestamp,
	#[db(updated_at)]
	pub updated_at: Timestamp,
}

/// A backup of an environment's managed database: a record of a dump its
/// cluster's worker took and keeps. The control plane never holds the
/// dump, and `location` is the worker's own name for it, which no
/// operation answers.
#[table(name = "database_backups", index(environment_id, created_at))]
pub struct DatabaseBackup {
	pub id: Id<Self>,
	#[db(on_delete = cascade)]
	pub environment_id: Id<Environment>,
	#[db(no_foreign_key)]
	pub organization_id: Id<Organization>,
	#[db(no_foreign_key)]
	pub database_id: Id<Database>,
	pub kind: BackupKind,
	pub state: BackupState,
	/// Where the worker keeps the dump, once it has taken it.
	#[db(max_len = 512)]
	pub location: Option<String>,
	pub bytes: Option<i64>,
	/// The migrations the database had when the dump was taken, as the
	/// environment's placement recorded them then.
	pub migrations: Json<Vec<MigrationFingerprint>>,
	/// Whether the database may have had more than `migrations` says.
	pub schema_uncertain: bool,
	#[db(max_len = 400)]
	pub failure: Option<String>,
	pub completed_at: Option<Timestamp>,
	/// When its row is deleted, and its worker deletes the dump.
	pub expires_at: Timestamp,
	#[db(created_at)]
	pub created_at: Timestamp,
	#[db(updated_at)]
	pub updated_at: Timestamp,
}

/// That a job was given a secret's value: which secret, and which version,
/// written in the transaction that leases the job, before the value leaves.
/// Kept after the secret and the job are gone, as the record of where a
/// value went.
#[table(name = "secret_leases", index(job_id), index(environment_id))]
pub struct SecretLease {
	pub id: Id<Self>,
	#[db(no_foreign_key)]
	pub job_id: Id<Job>,
	/// Which run of the job, from 1.
	pub attempt: i64,
	#[db(no_foreign_key)]
	pub worker_id: Id<Worker>,
	#[db(no_foreign_key)]
	pub organization_id: Id<Organization>,
	#[db(no_foreign_key)]
	pub environment_id: Id<Environment>,
	#[db(no_foreign_key)]
	pub secret_id: Id<Secret>,
	#[db(max_len = 128)]
	pub name: String,
	pub version: i64,
	#[db(created_at)]
	pub created_at: Timestamp,
}
