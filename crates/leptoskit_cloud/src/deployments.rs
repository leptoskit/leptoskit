//! Uploads, deployments, their events, and their logs: the operations a
//! client ships with, and what it watches while a release goes out.
//!
//! # Shipping
//!
//! ```text
//! begin_upload { digest, size }   → stored already, or a presigned PUT bound to both
//! PUT the archive                 (to the object store, never through the control plane's API)
//! complete_upload { digest }      → the control plane checks size and SHA-256 of what was stored
//! deploy { environment, source, config } → deployment #n, queued
//! ```
//!
//! A deployment then moves through its states as workers claim and finish
//! its jobs ([`crate::deployment`] has the diagram):
//!
//! | Transition                          | Who drives it                                               |
//! | ----------------------------------- | ----------------------------------------------------------- |
//! | `queued → building`                 | a build worker's `claim`                                    |
//! | `building → migrating → rolling_out` | the build's `finish`, after the schema check; a release job is queued |
//! | `building → failed(building, build_failed)` | the build's `finish`                                  |
//! | `queued`/`building` → `cancelled`   | `cancel_deployment`; a claimed build learns it on `renew`   |
//! | `rolling_out → live`, previous `live → superseded` | the release's `finish`, in one transaction    |
//! | `rolling_out → failed(rolling_out, …)` | the release's `finish` (`not_ready`, `migration_failed`) or `reconcile` (`interrupted`) |
//! | a build's lease runs out            | the control plane: retried, then `failed(building, builder_unavailable)` |
//! | a release's lease runs out          | nobody: the release is *unknown* until `reconcile` or the worker's saved `finish` |
//! | `queued → migrating` (a rollback)   | `rollback` itself, then `rolling_out` in the same transaction |
//!
//! One deployment per environment is in flight (`queued` to `rolling_out`)
//! at a time: `deploy` and `rollback` refuse a second with
//! `deployment_in_flight`, naming the first.
//!
//! # The sheet
//!
//! Each deployment has lines, one per thing it ships: the server, each
//! service, the database's migrations, and the web assets. A line carries a
//! stamp, such as `built`, `migrated`, `live`, `failed`, or `unknown`, and
//! the time it was stamped; the server's line carries the build's steps.
//! The sheet's fine print says what the release won't do.

use std::collections::BTreeMap;

use leptoskit_cloud_runtime::ReleaseSpec;
use leptoskit_config::cloud::CloudConfig;
use leptoskit_db::Timestamp;
use leptoskit_rpc as rpc;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use uuid::Uuid;

use crate::account::Via;
use crate::archive::Digest;
use crate::archive::Limits;
use crate::deployment::DeploymentState;
use crate::deployment::Source;
use crate::workers::BuiltImage;
use crate::workers::JobKind;
use crate::workers::RuntimeValue;

/// The largest archive `begin_upload` accepts: the archive's own bound on
/// its uncompressed size, which its compressed bytes never exceed.
pub const MAX_ARCHIVE_BYTES: u64 = Limits::DEFAULT.max_bytes;

/// The longest host name a CLI may report as where it deployed from.
pub const MAX_HOST_LEN: usize = 64;

/// The most lines one page of a log holds.
pub const MAX_LOG_PAGE: u32 = 1_000;

/// The most deployments one page of `deployments` holds.
pub const MAX_DEPLOYMENTS_PAGE: u32 = 100;

/// A source archive the organisation has, checked.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ArchiveSummary {
	pub digest: Digest,
	/// Its size in bytes.
	pub size: u64,
	pub verified_at: Timestamp,
	/// When it is deleted unless a deployment uses it again first.
	pub expires_at: Timestamp,
}

/// Where to put an archive's bytes: a presigned `PUT` bound to its size and
/// SHA-256, so the store refuses anything else. The address is the
/// credential: its `Debug` never shows it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UploadTicket {
	pub url: String,
	/// Always `PUT`.
	pub method: String,
	/// Send exactly these headers with the bytes.
	pub headers: BTreeMap<String, String>,
	pub expires_at: Timestamp,
}

impl std::fmt::Debug for UploadTicket {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_struct("UploadTicket")
			.field("url", &"[redacted]")
			.field("expires_at", &self.expires_at)
			.finish_non_exhaustive()
	}
}

/// What `begin_upload` says.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum BeganUpload {
	/// The organisation already has these bytes: deploy them.
	Stored { archive: ArchiveSummary },
	/// Put the bytes where the ticket says, then call `complete_upload`.
	Upload { ticket: UploadTicket },
}

/// The app's database, as the CLI read `[db]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DatabaseDialect {
	Sqlite,
	Postgres,
}

/// One `[services.<name>]`, as the CLI read it: only what deploying needs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[expect(
	clippy::struct_excessive_bools,
	reason = "each flag is one `[services.<name>]` setting the CLI reports, as leptoskit.toml has \
	          it"
)]
pub struct ServiceFacts {
	/// It runs elsewhere (`url` is set), so the cloud doesn't run it.
	pub remote: bool,
	/// It answers browsers, under `/_leptoskit/services/<name>/`.
	pub public: bool,
	/// It is `trust = "isolated"`: third-party code that never sees the
	/// app's cookies.
	pub isolated: bool,
	/// It has `database = true`: a database of its own, on the app's
	/// engine.
	pub database: bool,
}

/// What the CLI reports about the app beside `[cloud]`: what
/// `leptoskit.toml` says about its database, live queries, and services.
/// The control plane resolves `[cloud]` against these facts, and trusts
/// them for nothing else: who may deploy, and how big, it decides itself.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct AppFacts {
	/// `[db] dialect`, when the app has a database.
	pub database: Option<DatabaseDialect>,
	/// Whether the app serves live queries.
	pub live: bool,
	/// `[services]`, by name.
	pub services: BTreeMap<String, ServiceFacts>,
}

/// What a deployment is configured from: the `[cloud]` table of
/// `leptoskit.toml`, and the facts about the app it resolves against.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DeployConfig {
	#[serde(default)]
	pub cloud: CloudConfig,
	#[serde(default)]
	pub app: AppFacts,
}

/// Who signed a deployment off: recorded by the control plane from the
/// caller's identity, never from an argument.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SignedBy {
	/// A person, by account and the address they had then.
	Person {
		account: Uuid,
		email: Option<String>,
	},
	/// An API token, by ID, name, and prefix, and the person it acts for.
	Token {
		token: Uuid,
		name: String,
		prefix: String,
		created_by: Uuid,
	},
}

/// The signature on a deployment: who, how, and, for the CLI and tokens,
/// the host the client said it ran on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Signer {
	pub by: SignedBy,
	pub via: Via,
	/// What the client reported as its host, such as `ana-mbp`. Shown, never
	/// trusted.
	pub host: Option<String>,
}

/// A line of a deployment's sheet: one thing it ships.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum LineKind {
	/// The app's server image.
	Server,
	/// One of the app's services.
	Service { name: String },
	/// The app database's migrations.
	Migrations,
	/// The web assets.
	WebAssets,
}

/// What a line's stamp says.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Stamp {
	/// Not started.
	Pending,
	/// In progress.
	Working,
	/// The image is built.
	Built,
	/// The migrations applied.
	Migrated,
	/// Serving.
	Live,
	/// Nothing to do, or never reached.
	Skipped,
	/// It failed: the line's detail says why.
	Failed,
	/// The worker went quiet: nobody knows yet whether it happened.
	Unknown,
}

/// One step a build took.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BuildStep {
	/// Such as `cargo build --release -p notes_server`.
	pub name: String,
	pub millis: u64,
	/// How much of it came from the cache, as a percentage.
	#[serde(default)]
	pub cached_percent: Option<u8>,
}

/// One line of the sheet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Line {
	pub kind: LineKind,
	pub title: String,
	/// One sentence about it: what it is, or why it failed.
	pub detail: String,
	pub stamp: Stamp,
	pub stamped_at: Option<Timestamp>,
	/// The build's steps, on the server's line.
	#[serde(default)]
	pub steps: Vec<BuildStep>,
}

/// A deployment, as lists show it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeploymentSummary {
	pub id: Uuid,
	pub project: String,
	pub environment: String,
	/// Counted from 1 in each environment.
	pub number: u64,
	pub state: DeploymentState,
	pub source: Source,
	/// The deployment a rollback returns to.
	pub rollback_of: Option<u64>,
	pub signer: Signer,
	/// Its release's worker went quiet, and the control plane is waiting to
	/// learn what the cluster runs.
	pub unknown: bool,
	pub created_at: Timestamp,
	pub updated_at: Timestamp,
}

/// A deployment's whole sheet.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DeploymentSheet {
	pub deployment: DeploymentSummary,
	pub lines: Vec<Line>,
	/// What this release won't do, as sentences.
	pub fine_print: Vec<String>,
	/// Where it is served.
	pub hostname: Option<String>,
	/// The image it runs, once built.
	pub image: Option<BuiltImage>,
	/// Everything the cluster gets, once built.
	pub spec: Option<RuntimeValue<ReleaseSpec>>,
	/// The deployment that replaced it.
	pub superseded_by: Option<u64>,
}

/// One thing that happened to a deployment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum EventKind {
	/// It was signed off.
	Created { signer: Signer },
	/// It changed state.
	Transition {
		from: DeploymentState,
		to: DeploymentState,
	},
	/// A line got a stamp.
	Stamped {
		line: LineKind,
		stamp: Stamp,
		detail: String,
	},
	/// The build finished a step.
	Step { line: LineKind, step: BuildStep },
	/// A worker took one of its jobs.
	Claimed {
		job: Uuid,
		kind: JobKind,
		attempt: u32,
	},
	/// A build's worker went quiet, and the build runs again.
	Retrying {
		job: Uuid,
		attempt: u32,
		reason: String,
	},
	/// Someone asked it to stop.
	CancelRequested,
	/// A release's worker went quiet: the control plane is finding out what
	/// the cluster runs, and calls it neither live nor failed until it
	/// knows.
	Reconciling { job: Uuid, reason: String },
	/// What the cluster runs is known again.
	Reconciled { job: Uuid, outcome: String },
}

/// An event, in order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeploymentEvent {
	pub id: Uuid,
	/// From 1, in the order they happened.
	pub seq: u32,
	pub at: Timestamp,
	pub event: EventKind,
}

/// One meter's line of a cost estimate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CostLine {
	pub meter: String,
	pub quantity: u64,
	pub allowance: u64,
	pub billable: u64,
	/// In minor units.
	pub amount: u64,
}

/// A month's expected cost of the environment on the organisation's plan,
/// in minor units.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CostEstimate {
	pub currency: String,
	pub plan: String,
	pub plan_price: Option<u64>,
	pub lines: Vec<CostLine>,
	pub usage: u64,
	pub included: u64,
	pub total: u64,
	pub assumptions: Vec<String>,
}

/// What a preview can say about cost.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PreviewCost {
	Estimate(CostEstimate),
	/// The control plane can't price it, and says why: it has no price
	/// catalogue, or the catalogue doesn't have the plan.
	Unpriced {
		reason: String,
	},
}

/// A release that hasn't been shipped: its lines, cost, and fine print.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReleasePreview {
	pub environment: String,
	/// The number it would get.
	pub number: u64,
	pub lines: Vec<Line>,
	pub cost: PreviewCost,
	pub fine_print: Vec<String>,
	pub hostname: String,
}

/// A deployment on its way out, for the organisation's shipping chip.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InFlightDeployment {
	pub id: Uuid,
	pub project: String,
	pub environment: String,
	pub number: u64,
	pub state: DeploymentState,
	pub unknown: bool,
	pub created_at: Timestamp,
}

/// One log line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LogEntry {
	/// Its position in the log, from 0: pass the last one back as `after`.
	pub n: u64,
	pub at: Timestamp,
	/// The build's attempt, for a build log.
	pub attempt: Option<u32>,
	/// The process, for a runtime log.
	pub process: Option<String>,
	/// The pod, for a runtime log.
	pub pod: Option<String>,
	/// The text, with secrets the control plane knows the shape of
	/// redacted. Untrusted: show it as plain text.
	pub text: String,
}

/// A page of a log.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LogPage {
	pub lines: Vec<LogEntry>,
	/// Pass as `after` for the lines after these.
	pub next: Option<u64>,
	/// How long these logs are kept, in days.
	pub retention_days: u32,
}

/// One process, as its cluster worker last reported it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProcessStatus {
	pub name: String,
	pub ready: u32,
	pub desired: u32,
	pub image: String,
	pub restarts: u64,
	/// When it was last killed for using too much memory, in milliseconds
	/// since the epoch.
	pub last_oom_kill_at: Option<i64>,
	pub state: ProcessCondition,
}

/// What a process is doing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ProcessCondition {
	Sleeping,
	Pending,
	Ready,
	Failed { reason: String },
}

/// What an environment runs, as its cluster worker last said.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EnvironmentStatus {
	pub environment: String,
	/// The cluster it is placed on, once it has been deployed.
	pub cluster: Option<String>,
	pub hostname: Option<String>,
	/// When the cluster worker last reported; `None` when it never has.
	pub reported_at: Option<Timestamp>,
	pub processes: Vec<ProcessStatus>,
}

/// Starts uploading a source archive: says whether the organisation has
/// it already, or where to put it. Bytes are kept by organisation, so
/// another organisation's copy never counts, and every caller gets the same
/// answer for an archive their organisation doesn't have.
///
/// It is an action, so the upload address is never kept.
///
/// # Arguments
///
/// - `digest`: the archive's `sha256:…`.
/// - `size`: its size in bytes, at most 512 MiB.
///
/// # Errors
///
/// `Invalid` (`archive_too_large`, `archive_empty`) for a size out of
/// bounds; `Conflict` (`storage_full`) when the organisation's archives
/// would pass its storage cap.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "deployments")]
#[rpc::action]
pub async fn begin_upload(
	cx: &rpc::ActionCx,
	organization: String,
	project: String,
	digest: Digest,
	size: u64,
) -> rpc::Result<BeganUpload> {
	crate::server::uploads::begin_upload(cx, organization, project, digest, size).await
}

/// Finishes an upload: checks the stored bytes' size and SHA-256 against
/// what `begin_upload` was told, and keeps the archive for deploying.
/// Calling it again for a checked archive answers the same.
///
/// # Errors
///
/// `NotFound` (`upload_not_found`) when nothing was begun or stored;
/// `Conflict` (`upload_mismatch`) when the bytes aren't the ones promised,
/// which are then deleted.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "deployments")]
#[rpc::action]
pub async fn complete_upload(
	cx: &rpc::ActionCx,
	organization: String,
	project: String,
	digest: Digest,
) -> rpc::Result<ArchiveSummary> {
	crate::server::uploads::complete_upload(cx, organization, project, digest).await
}

/// Ships an environment: makes deployment number *n*, queued.
///
/// # Arguments
///
/// - `source`: `upload` with a checked archive. `git` and `template` sources
///   aren't available yet, and a rollback is `rollback`.
/// - `config`: the `[cloud]` table and what the CLI found about the app.
/// - `host`: the CLI's host name, shown on the signature.
///
/// # Errors
///
/// `Conflict` (`deployment_in_flight`) while another deployment of the
/// environment is on its way; `Invalid` for a configuration `[cloud]`
/// refuses, a source that isn't available, a Postgres database
/// (`postgres_unavailable`); `NotFound` (`archive_not_found`) for an archive
/// the organisation doesn't have; `Retry` (`cluster_unavailable`) when no
/// cluster runs the project's region.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "deployments")]
#[rpc::mutation]
pub async fn deploy(
	cx: &rpc::MutationCx,
	organization: String,
	project: String,
	environment: String,
	source: Source,
	#[serde(default)] config: DeployConfig,
	#[serde(default)] host: Option<String>,
) -> rpc::Result<DeploymentSummary> {
	Box::pin(crate::server::deployments::deploy(
		cx,
		&organization,
		&project,
		&environment,
		source,
		config,
		host,
	))
	.await
}

/// What shipping would do, without shipping: the lines, the cost, and the
/// fine print. Give `config` to preview a deploy, or `rollback_to` to
/// preview returning to an earlier deployment.
///
/// # Errors
///
/// `Invalid` for a configuration `[cloud]` refuses, or neither or both of
/// `config` and `rollback_to`.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "deployments")]
#[rpc::query]
pub async fn preview_release(
	cx: &rpc::QueryCx,
	organization: String,
	project: String,
	environment: String,
	#[serde(default)] config: Option<DeployConfig>,
	#[serde(default)] rollback_to: Option<u64>,
) -> rpc::Result<ReleasePreview> {
	crate::server::deployments::preview_release(
		cx,
		&organization,
		&project,
		&environment,
		config,
		rollback_to,
	)
	.await
}

/// Stops a deployment that is queued or building. A build worker that has
/// it stops at its next `renew`.
///
/// # Errors
///
/// `Conflict` (`not_cancellable`) once it is migrating or later.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "deployments")]
#[rpc::mutation]
pub async fn cancel_deployment(
	cx: &rpc::MutationCx,
	organization: String,
	project: String,
	environment: String,
	number: u64,
) -> rpc::Result<DeploymentSummary> {
	crate::server::deployments::cancel_deployment(cx, &organization, &project, &environment, number)
		.await
}

/// Returns an environment to an earlier deployment that was live: a new
/// deployment that reuses its image, skipping the build.
///
/// # Errors
///
/// `Conflict` (`rollback_unavailable`) when that deployment never went
/// live, its image is gone, or the database has migrations it doesn't know;
/// `Conflict` (`deployment_in_flight`) as for `deploy`.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "deployments")]
#[rpc::mutation]
pub async fn rollback(
	cx: &rpc::MutationCx,
	organization: String,
	project: String,
	environment: String,
	to: u64,
) -> rpc::Result<DeploymentSummary> {
	Box::pin(crate::server::deployments::rollback(
		cx,
		&organization,
		&project,
		&environment,
		to,
	))
	.await
}

/// One deployment's sheet.
///
/// # Errors
///
/// `NotFound` (`deployment_not_found`).
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "deployments")]
#[rpc::query]
pub async fn deployment(
	cx: &rpc::QueryCx,
	organization: String,
	project: String,
	environment: String,
	number: u64,
) -> rpc::Result<DeploymentSheet> {
	crate::server::deployments::deployment(cx, &organization, &project, &environment, number).await
}

/// A project's deployments, newest first: of one environment, or all of
/// them. Pass the last one's `id` as `before` for the page after.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "deployments")]
#[rpc::query]
pub async fn deployments(
	cx: &rpc::QueryCx,
	organization: String,
	project: String,
	#[serde(default)] environment: Option<String>,
	#[serde(default)] before: Option<Uuid>,
	#[serde(default)] limit: Option<u32>,
) -> rpc::Result<Vec<DeploymentSummary>> {
	crate::server::deployments::deployments(cx, &organization, &project, environment, before, limit)
		.await
}

/// What happened to a deployment, in order. Subscribe to it to watch a
/// release go out.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "deployments")]
#[rpc::query]
pub async fn deployment_events(
	cx: &rpc::QueryCx,
	organization: String,
	project: String,
	environment: String,
	number: u64,
) -> rpc::Result<Vec<DeploymentEvent>> {
	crate::server::deployments::deployment_events(cx, &organization, &project, &environment, number)
		.await
}

/// The organisation's deployments on their way out, oldest first: those a
/// project-bound token can see, for a token.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "deployments")]
#[rpc::query]
pub async fn in_flight(
	cx: &rpc::QueryCx,
	organization: String,
) -> rpc::Result<Vec<InFlightDeployment>> {
	crate::server::deployments::in_flight(cx, &organization).await
}

/// A deployment's build log, after line `after`. Subscribe to it to follow
/// the build.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "deployments")]
#[rpc::query]
pub async fn build_logs(
	cx: &rpc::QueryCx,
	organization: String,
	project: String,
	environment: String,
	number: u64,
	#[serde(default)] after: Option<u64>,
	#[serde(default)] limit: Option<u32>,
) -> rpc::Result<LogPage> {
	crate::server::logs::build_logs(
		cx,
		&organization,
		&project,
		&environment,
		number,
		after,
		limit,
	)
	.await
}

/// What an environment's processes wrote, as its cluster worker reported
/// it: the newest `limit` lines, or the lines after `after`.
///
/// # Arguments
///
/// - `process`: only this process's lines.
/// - `since_minutes`: only lines this recent.
/// - `contains`: only lines with this text.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "deployments")]
#[rpc::query]
#[allow(
	clippy::too_many_arguments,
	reason = "each filter is its own named argument on the wire"
)]
pub async fn runtime_logs(
	cx: &rpc::QueryCx,
	organization: String,
	project: String,
	environment: String,
	#[serde(default)] process: Option<String>,
	#[serde(default)] since_minutes: Option<u32>,
	#[serde(default)] contains: Option<String>,
	#[serde(default)] after: Option<u64>,
	#[serde(default)] limit: Option<u32>,
) -> rpc::Result<LogPage> {
	crate::server::logs::runtime_logs(
		cx,
		crate::server::logs::RuntimeLogQuery {
			organization,
			project,
			environment,
			process,
			since_minutes,
			contains,
			after,
			limit,
		},
	)
	.await
}

/// Checks a host name a client reported: at most [`MAX_HOST_LEN`] letters,
/// digits, `.`, `-`, and `_`.
///
/// # Errors
///
/// Why it isn't one.
pub fn check_host(host: &str) -> Result<(), String> {
	let valid = !host.is_empty()
		&& host.len() <= MAX_HOST_LEN
		&& host.chars().all(|character| {
			character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_')
		});

	if valid {
		Ok(())
	} else {
		Err(format!(
			"a host is 1 to {MAX_HOST_LEN} letters, digits, `.`, `-`, and `_`"
		))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn hosts_are_short_and_plain() {
		assert!(check_host("ana-mbp.local").is_ok());
		assert!(check_host("").is_err());
		assert!(check_host("a b").is_err());
		assert!(check_host(&"a".repeat(65)).is_err());
	}

	#[test]
	fn an_upload_ticket_never_shows_its_address() {
		let ticket = UploadTicket {
			url: "https://store.example.com/x?X-Amz-Signature=secret".into(),
			method: "PUT".into(),
			headers: BTreeMap::new(),
			expires_at: chrono::DateTime::UNIX_EPOCH,
		};
		assert!(!format!("{ticket:?}").contains("secret"));
	}
}
