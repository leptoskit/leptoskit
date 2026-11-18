//! The worker protocol: how build and cluster workers pair with the control
//! plane, claim jobs, and report what happened.
//!
//! A worker is the control plane's fourth kind of caller, after people,
//! the CLI, and API tokens. It calls with a worker key,
//! `lkw_<prefix>_<secret>`, which works on these operations and nowhere
//! else; no other credential works here.
//!
//! ```text
//! operator: issue_pairing { role, pool, cluster? } → lkp_… (one use, ten minutes)
//! worker:   pair { code, name, capabilities }      → lkw_… (shown once)
//!           claim { wait_seconds }                 → a job and its lease, or nothing
//!           renew { job }                          → a later expiry, and whether to stop
//!           append_log { job, attempt, seq, lines }
//!           finish { job, attempt, outcome }       → accepted; the same outcome again is too
//!           report_status { environment, observed, logs }   (cluster workers)
//! ```
//!
//! # Leases
//!
//! A claim holds its job for [`Assignment::lease_seconds`]. `renew` before
//! it runs out; a lease that runs out is the control plane's to decide:
//!
//! - a build is retried by another worker, at most [`BUILD_ATTEMPTS`] times,
//!   and then fails with `builder_unavailable`: a build's only effect is an
//!   image keyed by its digest, so running it again is safe;
//! - a release becomes *unknown*: the cluster may or may not run it, so nothing
//!   retries it or calls it failed. An operator reconciles it, or the worker
//!   that held it sends its saved outcome with `finish`, which is still
//!   accepted.
//!
//! A worker writes a pending marker before it starts a job and the outcome
//! before it calls `finish`; a marker without an outcome means it doesn't
//! know what happened, and it must not run the job again.
//!
//! # Payloads
//!
//! Everything a job needs is in its [`JobInput`], and nothing more: a build
//! gets a download address for one archive that expires with the lease's
//! attempt, never a credential for the store, and a shared-pool build
//! worker holds one organisation's job at a time.

use std::borrow::Cow;
use std::collections::BTreeMap;

use leptoskit_cloud_runtime::EnvironmentRef;
use leptoskit_cloud_runtime::LogLine;
use leptoskit_cloud_runtime::Observed;
use leptoskit_cloud_runtime::ReleaseSpec;
use leptoskit_db::Timestamp;
use leptoskit_rpc as rpc;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use uuid::Uuid;

use crate::archive::Digest;
use crate::deployments::AppFacts;
use crate::deployments::BuildStep;

/// What every worker key starts with.
pub const WORKER_KEY_PREFIX: &str = "lkw_";

/// What every pairing code starts with.
pub const PAIRING_CODE_PREFIX: &str = "lkp_";

/// How many lower-case hex characters a worker key's lookup part has.
pub const WORKER_LOOKUP_LEN: usize = 12;

/// How many lower-case hex characters a worker key's secret part has: 32
/// random bytes.
pub const WORKER_SECRET_LEN: usize = 64;

/// How many lower-case hex characters follow `lkp_`: 16 random bytes.
pub const PAIRING_CODE_LEN: usize = 32;

/// How long a pairing code works for, in minutes.
pub const PAIRING_MINUTES: i64 = 10;

/// The longest a `claim` waits for a job before answering nothing.
pub const MAX_WAIT_SECONDS: u32 = 20;

/// How many times a build runs before its deployment fails with
/// `builder_unavailable`.
pub const BUILD_ATTEMPTS: u32 = 3;

/// The most capabilities a worker may declare.
pub const MAX_CAPABILITIES: usize = 32;

/// The most lines one `append_log` may carry.
pub const MAX_LOG_LINES: usize = 1_000;

/// The most bytes one `append_log` may carry, across its lines.
pub const MAX_LOG_BYTES: usize = 64 * 1024;

/// The longest line a log keeps, in bytes; longer ones are cut, saying so.
pub const MAX_LOG_LINE_BYTES: usize = 4 * 1024;

/// The most runtime log lines one `report_status` may carry.
pub const MAX_STATUS_LOG_LINES: usize = 500;

/// A value from `leptoskit_cloud_runtime` on the wire, unchanged: the
/// runtime's types are plain serde data without schemas, and this describes
/// them to generated clients by name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RuntimeValue<T>(pub T);

/// A runtime type [`RuntimeValue`] can carry, with how the schema names it.
pub trait RuntimeSchema {
	/// The schema's name.
	const NAME: &'static str;
	/// What the value is, and where its fields are documented.
	const DESCRIPTION: &'static str;
}

impl RuntimeSchema for ReleaseSpec {
	const DESCRIPTION: &'static str = "A `leptoskit_cloud_runtime::ReleaseSpec`: the whole \
	                                   desired state of one environment, as the cluster worker \
	                                   applies it.";
	const NAME: &'static str = "ReleaseSpec";
}

impl RuntimeSchema for EnvironmentRef {
	const DESCRIPTION: &'static str = "A `leptoskit_cloud_runtime::EnvironmentRef`: an \
	                                   environment's organisation, project, and environment, by \
	                                   slug and ID.";
	const NAME: &'static str = "EnvironmentRef";
}

impl RuntimeSchema for Observed {
	const DESCRIPTION: &'static str = "A `leptoskit_cloud_runtime::Observed`: what a cluster runs \
	                                   for one environment, process by process.";
	const NAME: &'static str = "Observed";
}

impl RuntimeSchema for LogLine {
	const DESCRIPTION: &'static str =
		"A `leptoskit_cloud_runtime::LogLine`: one line a process wrote, with its pod and time.";
	const NAME: &'static str = "LogLine";
}

impl<T: RuntimeSchema> JsonSchema for RuntimeValue<T> {
	fn schema_name() -> Cow<'static, str> {
		T::NAME.into()
	}

	fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
		schemars::json_schema!({
			"type": "object",
			"description": T::DESCRIPTION,
		})
	}
}

/// What a worker does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, leptoskit_db::Enum)]
pub enum WorkerRole {
	/// Builds server images from source archives.
	#[db(rename = "build")]
	Build,
	/// Applies releases to one cluster and reports what it runs.
	#[db(rename = "cluster")]
	Cluster,
}

/// The kinds of job.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, leptoskit_db::Enum)]
pub enum JobKind {
	/// Build a deployment's server image: for build workers.
	#[db(rename = "build_server")]
	BuildServer,
	/// Apply a release to the cluster: for its cluster worker.
	#[db(rename = "release")]
	Release,
	/// Remove an environment from the cluster: for its cluster worker.
	#[db(rename = "destroy_environment")]
	DestroyEnvironment,
}

impl JobKind {
	/// Which role takes the job.
	#[must_use]
	pub const fn role(self) -> WorkerRole {
		match self {
			Self::BuildServer => WorkerRole::Build,
			Self::Release | Self::DestroyEnvironment => WorkerRole::Cluster,
		}
	}
}

/// A worker key's value, split into the part that finds it and the part
/// that proves it. Its `Debug` never shows the secret.
#[derive(Clone, PartialEq, Eq)]
pub struct WorkerKeyValue<'a> {
	/// The twelve hex characters after `lkw_`.
	pub lookup: &'a str,
	/// The 64 hex characters after the second underscore.
	pub secret: &'a str,
}

impl<'a> WorkerKeyValue<'a> {
	/// Splits `value`, or `None` when it isn't shaped like a worker key.
	#[must_use]
	pub fn parse(value: &'a str) -> Option<Self> {
		let rest = value.strip_prefix(WORKER_KEY_PREFIX)?;
		let (lookup, secret) = rest.split_once('_')?;
		(is_hex(lookup, WORKER_LOOKUP_LEN) && is_hex(secret, WORKER_SECRET_LEN))
			.then_some(Self { lookup, secret })
	}
}

impl std::fmt::Debug for WorkerKeyValue<'_> {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_struct("WorkerKeyValue")
			.field("lookup", &self.lookup)
			.field("secret", &"[redacted]")
			.finish()
	}
}

/// Whether `value` is shaped like a pairing code: `lkp_` and 32 lower-case
/// hex characters.
#[must_use]
pub fn is_pairing_code(value: &str) -> bool {
	value
		.strip_prefix(PAIRING_CODE_PREFIX)
		.is_some_and(|rest| is_hex(rest, PAIRING_CODE_LEN))
}

fn is_hex(part: &str, len: usize) -> bool {
	part.len() == len
		&& part
			.bytes()
			.all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Checks a worker's capabilities: at most [`MAX_CAPABILITIES`], each 1 to
/// 64 lower-case letters, digits, and `.`, `_`, `:`, `-`, starting with a
/// letter or digit. Returns them sorted and without repeats.
///
/// # Errors
///
/// Which rule they break.
pub fn check_capabilities(capabilities: Vec<String>) -> Result<Vec<String>, String> {
	let mut capabilities = capabilities;
	capabilities.sort_unstable();
	capabilities.dedup();

	if capabilities.len() > MAX_CAPABILITIES {
		return Err(format!("at most {MAX_CAPABILITIES} capabilities"));
	}

	for capability in &capabilities {
		let valid = (1..=64).contains(&capability.len())
			&& capability
				.starts_with(|first: char| first.is_ascii_lowercase() || first.is_ascii_digit())
			&& capability.chars().all(|character| {
				character.is_ascii_lowercase()
					|| character.is_ascii_digit()
					|| matches!(character, '.' | '_' | ':' | '-')
			});

		if !valid {
			return Err(format!(
				"`{capability}` isn't a capability: 1 to 64 lower-case letters, digits, `.`, `_`, \
				 `:` and `-`"
			));
		}
	}

	Ok(capabilities)
}

/// A worker, as operators see it. Never its key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WorkerSummary {
	pub id: Uuid,
	pub name: String,
	pub role: WorkerRole,
	/// The non-secret part of its key, `lkw_<prefix>_…`.
	pub prefix: String,
	/// The organisation whose jobs it takes, or `None` for the shared pool.
	pub organization: Option<String>,
	/// The cluster it applies releases to, for a cluster worker.
	pub cluster: Option<String>,
	pub capabilities: Vec<String>,
	pub paired_at: Timestamp,
	/// When it last claimed, renewed, finished, or reported.
	pub last_seen_at: Option<Timestamp>,
	pub revoked_at: Option<Timestamp>,
}

/// A newly paired worker and its key: shown this once, and never stored.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PairedWorker {
	pub worker: WorkerSummary,
	/// The whole key, `lkw_<prefix>_<secret>`. Keep it: the cloud can't show
	/// it again.
	pub key: String,
	/// How long a claim holds a job before it must be renewed.
	pub lease_seconds: u32,
}

impl std::fmt::Debug for PairedWorker {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_struct("PairedWorker")
			.field("worker", &self.worker)
			.field("key", &"[redacted]")
			.finish_non_exhaustive()
	}
}

/// A presigned address to download one object from, until it expires. The
/// address is the credential: its `Debug` never shows it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Download {
	pub url: String,
	pub expires_at: Timestamp,
}

impl std::fmt::Debug for Download {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_struct("Download")
			.field("url", &"[redacted]")
			.field("expires_at", &self.expires_at)
			.finish()
	}
}

/// Which deployment a job is for, by slug and number.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeploymentRef {
	pub organization: String,
	pub project: String,
	pub environment: String,
	pub number: u64,
}

/// `build_server`'s input: one archive to build into one image.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BuildServerInput {
	pub deployment: DeploymentRef,
	/// The source archive, to fetch and unpack with
	/// `leptoskit_cloud::archive`.
	pub archive: Download,
	/// Its digest: check the bytes against it before unpacking.
	pub digest: Digest,
	/// Its size in bytes.
	pub size: u64,
	/// The Rust toolchain to build with, such as `stable`.
	pub toolchain: String,
	/// What `lk cloud launch` found to build, such as `server` and `web`.
	pub targets: Vec<String>,
	/// Where to push the image: the worker returns its digest, never a tag.
	pub image_repository: String,
	/// What the CLI said about the app: its services and database.
	pub app: AppFacts,
}

/// The image a build produced.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BuiltImage {
	pub repository: String,
	/// `sha256:` and 64 lower-case hex digits.
	pub digest: Digest,
}

/// How one process of the built image starts, as the build found it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProcessFacts {
	/// `app`, or a service's name.
	pub name: String,
	/// Its own image, when it isn't the app's.
	#[serde(default)]
	pub image: Option<BuiltImage>,
	#[serde(default)]
	pub command: Vec<String>,
	#[serde(default)]
	pub args: Vec<String>,
	pub port: u16,
	/// The path it answers readiness on, such as `/readyz`.
	pub ready_path: String,
	/// The path it answers liveness on, such as `/healthz`.
	pub live_path: String,
}

/// One migration the built server carries: its name and checksum, as
/// `leptoskit_db` records them.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
pub struct MigrationFingerprint {
	pub name: String,
	pub checksum: String,
}

/// Where a build put the web assets.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WebAssets {
	/// Inside the image, served by the app, under `path`.
	InImage { path: String },
}

/// What a successful build made.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BuildServerOutput {
	pub image: BuiltImage,
	/// The app first, then each service the image runs.
	pub processes: Vec<ProcessFacts>,
	#[serde(default)]
	pub web_assets: Option<WebAssets>,
	/// The app database's migrations, in the order they apply.
	#[serde(default)]
	pub migrations: Vec<MigrationFingerprint>,
	/// What the build did, in order.
	#[serde(default)]
	pub steps: Vec<BuildStep>,
}

/// How a build ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum BuildOutcome {
	/// The image is pushed.
	Succeeded(BuildServerOutput),
	/// The build failed: `reason` is its first error, with file and line.
	Failed {
		reason: String,
		#[serde(default)]
		steps: Vec<BuildStep>,
	},
	/// It stopped because `renew` said the deployment was cancelled.
	Stopped,
}

/// What happens to the database during a release.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MigrationStep {
	/// Nothing: the release changes no schema, or the app has no database.
	None { reason: String },
	/// The new server applies `pending` as it starts, before it answers
	/// `/readyz`: leptoskit servers migrate at startup and refuse a
	/// database with migrations they don't know. `history` is the
	/// fingerprint of every migration once they have applied.
	AtStartup {
		history: String,
		pending: Vec<String>,
	},
}

/// The managed Postgres database a release's processes use.
///
/// Before it applies the spec, the worker makes sure its cluster's Postgres
/// server has a role `role` that logs in with the password in the claim's
/// `DATABASE_URL` secret, and a database `name` that role owns and no other
/// role can connect to. Doing so again changes nothing but the password.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DatabaseStep {
	pub name: String,
	pub role: String,
	pub size: crate::size::DatabaseSize,
	pub backups: crate::size::Backups,
}

/// `release`'s input: the spec to apply, and what it does to the database.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ReleaseInput {
	pub deployment: DeploymentRef,
	pub spec: RuntimeValue<ReleaseSpec>,
	pub migration: MigrationStep,
	/// The deployment serving now, which keeps serving until the new one is
	/// ready.
	#[serde(default)]
	pub previous: Option<u64>,
	/// The environment's managed Postgres database, to make sure of before
	/// the spec is applied.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub database: Option<DatabaseStep>,
}

/// How a release ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum ReleaseOutcome {
	/// Every process is ready on the new spec.
	Applied,
	/// A process never became ready; the previous release kept serving.
	NotReady { process: String, reason: String },
	/// The new server's startup migrations failed; the previous release
	/// kept serving.
	MigrationsFailed { reason: String },
}

/// `destroy_environment`'s input.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DestroyEnvironmentInput {
	pub environment: RuntimeValue<EnvironmentRef>,
	/// The environment's managed Postgres database, by its name and its
	/// role's: both are dropped with the environment.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub database: Option<String>,
}

/// How removing an environment ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum DestroyOutcome {
	Destroyed,
	Failed { reason: String },
}

/// What a job asks of its worker.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum JobInput {
	BuildServer(BuildServerInput),
	Release(Box<ReleaseInput>),
	DestroyEnvironment(DestroyEnvironmentInput),
}

/// How a job ended, as its worker reports it with `finish`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum JobOutcome {
	BuildServer(BuildOutcome),
	Release(ReleaseOutcome),
	DestroyEnvironment(DestroyOutcome),
}

impl JobOutcome {
	/// The kind of job this outcome is for.
	#[must_use]
	pub const fn kind(&self) -> JobKind {
		match self {
			Self::BuildServer(_) => JobKind::BuildServer,
			Self::Release(_) => JobKind::Release,
			Self::DestroyEnvironment(_) => JobKind::DestroyEnvironment,
		}
	}
}

/// The values of the secrets a release's spec names in `secret_env`, by
/// those names: handed to the one cluster worker that claims the release,
/// in the claim's answer, and nowhere else. `Debug` prints names only.
///
/// A worker puts them in the environment's Kubernetes Secret
/// (`leptoskit_cloud_runtime::SecretValues`) and keeps them nowhere: not in
/// its pending marker, its saved outcome, or a log line.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct LeasedSecrets(BTreeMap<String, String>);

impl LeasedSecrets {
	/// Values by the names a spec's `secret_env` uses.
	#[must_use]
	pub const fn new(values: BTreeMap<String, String>) -> Self {
		Self(values)
	}

	/// Whether there are none.
	#[must_use]
	pub fn is_empty(&self) -> bool {
		self.0.is_empty()
	}

	/// The names, in order.
	pub fn names(&self) -> impl Iterator<Item = &str> {
		self.0.keys().map(String::as_str)
	}

	/// The values by name, for the one place a Secret is written.
	#[must_use]
	pub fn into_values(self) -> BTreeMap<String, String> {
		self.0
	}
}

impl std::fmt::Debug for LeasedSecrets {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_map()
			.entries(self.0.keys().map(|name| (name, "[redacted]")))
			.finish()
	}
}

/// A job a worker holds, and its lease. The input may carry presigned
/// addresses, and `secrets` a release's secret values, so `claim` is an
/// action: no receipt keeps it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Assignment {
	pub job: Uuid,
	/// Which run of the job this is, from 1: logs and the outcome name it.
	pub attempt: u32,
	pub lease_expires_at: Timestamp,
	pub lease_seconds: u32,
	pub input: JobInput,
	/// For a release whose spec names secrets: their values. Each is
	/// recorded as leased to this job before it is sent.
	#[serde(default, skip_serializing_if = "LeasedSecrets::is_empty")]
	pub secrets: LeasedSecrets,
}

/// What `renew` says.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Renewed {
	pub lease_expires_at: Timestamp,
	/// Stop: the deployment was cancelled. Finish with `stopped`.
	pub cancelled: bool,
}

/// What `finish` says.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Finished {
	pub job: Uuid,
	/// Whether this outcome was already recorded: a repeat of the same
	/// outcome is accepted and changes nothing.
	pub repeated: bool,
}

/// What `report_status` keeps.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StatusReceived {
	pub environment: Uuid,
	pub reported_at: Timestamp,
	/// How many log lines it kept.
	pub log_lines: u32,
}

/// Pairs a worker with a code an operator issued, and returns its key,
/// once. The code is the credential: anyone holding a live one may pair,
/// and it works once.
///
/// It is an action, so the key is never kept, not even as the answer to a
/// repeated request.
///
/// # Arguments
///
/// - `code`: `lkp_…`, from `issue_pairing`.
/// - `name`: what operators call it, such as `builder-eu-1`.
/// - `capabilities`: what it can do, such as `linux-x86_64`; a job runs only on
///   a worker with every capability it requires.
///
/// # Errors
///
/// `NotFound` (`pairing_invalid`) for a code that is unknown, used, or
/// expired, said the same way for each.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "workers")]
#[rpc::action]
pub async fn pair(
	cx: &rpc::ActionCx,
	code: String,
	name: String,
	#[serde(default)] capabilities: Vec<String>,
) -> rpc::Result<PairedWorker> {
	crate::server::workers::pair(cx, code, name, capabilities).await
}

/// Claims the oldest job this worker may take, with a lease, waiting up to
/// `wait_seconds` (at most 20) for one. `null` when there is none.
///
/// A job matches the worker's role, its pool (the shared pool's, or its
/// organisation's), its cluster for a cluster worker, and its
/// capabilities. A worker holds one job at a time, and a cluster worker
/// takes one job per environment at a time.
///
/// # Errors
///
/// `Conflict` (`job_held`) while the worker holds a job.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "workers")]
#[rpc::action]
pub async fn claim(
	cx: &rpc::ActionCx,
	#[serde(default)] wait_seconds: Option<u32>,
) -> rpc::Result<Option<Assignment>> {
	Box::pin(crate::server::workers::claim(cx, wait_seconds)).await
}

/// Extends the lease on a job this worker holds, and says whether to stop.
///
/// # Errors
///
/// `NotFound` (`job_not_held`) for a job it doesn't hold, including one
/// whose lease ran out.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "workers")]
#[rpc::mutation]
pub async fn renew(cx: &rpc::MutationCx, job: Uuid) -> rpc::Result<Renewed> {
	crate::server::workers::renew(cx, job).await
}

/// Appends lines to a job's log: chunk `seq` of `attempt`, from 0, in
/// order. Sending a chunk again with the same lines is accepted.
///
/// # Arguments
///
/// - `lines`: at most 1,000 lines and 64 KiB; a line over 4 KiB is cut.
///
/// # Errors
///
/// `NotFound` (`job_not_held`) for a job it doesn't hold; `Invalid`
/// (`log_out_of_order`) for a `seq` that isn't the next; `Conflict`
/// (`log_chunk_conflict`) for a chunk sent again with other lines.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "workers")]
#[rpc::action]
pub async fn append_log(
	cx: &rpc::ActionCx,
	job: Uuid,
	attempt: u32,
	seq: u32,
	lines: Vec<String>,
) -> rpc::Result<()> {
	crate::server::workers::append_log(cx, job, attempt, seq, lines).await
}

/// Reports how a job ended. The same outcome again is accepted and changes
/// nothing; a different one is refused. A worker whose lease ran out on a
/// release may still send the outcome it saved: the control plane was
/// waiting to learn it.
///
/// # Errors
///
/// `NotFound` (`job_not_held`) for a job the worker doesn't hold or held;
/// `Conflict` (`outcome_conflict`) for an outcome other than the one
/// recorded; `Invalid` (`outcome_kind`) for an outcome of another kind of
/// job.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "workers")]
#[rpc::mutation]
pub async fn finish(
	cx: &rpc::MutationCx,
	job: Uuid,
	attempt: u32,
	outcome: JobOutcome,
) -> rpc::Result<Finished> {
	crate::server::workers::finish(cx, job, attempt, outcome).await
}

/// A cluster worker's report of what its cluster runs for one environment,
/// with recent log lines (at most 500). `environment_status` shows the
/// newest; `runtime_logs` keeps the lines.
///
/// # Errors
///
/// `Forbidden` (`cluster_worker_required`) for a build worker; `NotFound`
/// (`environment_not_found`) for an environment not placed on its cluster.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "workers")]
#[rpc::action]
pub async fn report_status(
	cx: &rpc::ActionCx,
	environment: Uuid,
	observed: RuntimeValue<Observed>,
	#[serde(default)] logs: Vec<RuntimeValue<LogLine>>,
) -> rpc::Result<StatusReceived> {
	crate::server::workers::report_status(cx, environment, observed, logs).await
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn only_a_well_formed_key_parses() {
		let lookup = "0123456789ab";
		let secret = "e".repeat(WORKER_SECRET_LEN);
		let value = format!("lkw_{lookup}_{secret}");
		let parsed = WorkerKeyValue::parse(&value).expect("a key");
		assert_eq!(parsed.lookup, lookup);
		assert!(!format!("{parsed:?}").contains(&secret));

		for bad in [
			format!("lkc_{lookup}_{secret}"),
			format!("lkw_{lookup}{secret}"),
			format!("lkw_{}_{secret}", "0123456789AB"),
			format!("lkw_{lookup}_{secret}0"),
		] {
			assert!(WorkerKeyValue::parse(&bad).is_none(), "{bad}");
		}

		assert!(is_pairing_code(&format!("lkp_{}", "a".repeat(32))));
		assert!(!is_pairing_code(&format!("lkp_{}", "a".repeat(31))));
		assert!(!is_pairing_code(&format!("lkw_{}", "a".repeat(32))));
	}

	#[test]
	fn capabilities_are_checked_sorted_and_deduplicated() {
		assert_eq!(
			check_capabilities(vec![
				"linux-x86_64".into(),
				"docker".into(),
				"docker".into()
			]),
			Ok(vec!["docker".into(), "linux-x86_64".into()])
		);
		assert!(check_capabilities(vec!["Linux".into()]).is_err());
		assert!(check_capabilities(vec![String::new()]).is_err());
		assert!(check_capabilities((0..40).map(|n| format!("c{n}")).collect()).is_err());
	}

	#[test]
	fn outcomes_name_their_job_kind() {
		let outcome = JobOutcome::Release(ReleaseOutcome::Applied);
		assert_eq!(outcome.kind(), JobKind::Release);
		assert_eq!(
			serde_json::to_value(&outcome).unwrap(),
			serde_json::json!({ "kind": "release", "result": "applied" })
		);
	}
}
