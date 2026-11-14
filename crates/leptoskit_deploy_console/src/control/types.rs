use std::path::PathBuf;
use std::pin::Pin;

use serde::Deserialize;
use serde::Serialize;
use tokio_stream::Stream;

/// A repository explicitly registered by the service owner. API callers
/// cannot supply a path, shell command, remote URL or environment variable.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
	pub id: String,
	pub environment: String,
	#[serde(skip)]
	pub root: PathBuf,
	pub revision: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
	Read,
	Deploy,
	Ingest,
	Owner,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, tag = "kind", rename_all = "snake_case")]
pub enum Action {
	Plan,
	Apply {
		plan_id: String,
	},
	Deploy {
		version: String,
		#[serde(default)]
		setup: bool,
		#[serde(default)]
		maintenance: bool,
	},
	Rollback {
		version: String,
	},
	Status,
	Backup,
}

impl Action {
	#[must_use]
	pub fn valid(&self) -> bool {
		let tag = |s: &str| {
			!s.is_empty()
				&& s.len() <= 128
				&& s.bytes()
					.all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
		};

		match self {
			Self::Apply { plan_id } => super::identifier(plan_id),
			Self::Deploy { version, .. } | Self::Rollback { version } => {
				tag(version) && !version.starts_with('-')
			}
			Self::Plan | Self::Status | Self::Backup => true,
		}
	}
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Intent {
	pub expected_revision: u64,
	pub idempotency_key: String,
	pub action: Action,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
	Queued,
	Running,
	Succeeded,
	Failed,
	Interrupted,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Operation {
	#[serde(default)]
	pub events: Vec<Event>,
	pub id: String,
	pub project: String,
	/// Who requested the operation, when the embedder recorded it.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub actor: Option<String>,
	pub revision: u64,
	pub intent: Intent,
	pub phase: Phase,
	pub created: u64,
	pub updated: u64,
	pub message: String,
	pub result: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
	pub sequence: u64,
	pub phase: Phase,
	pub message: String,
	pub created: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FleetSeries {
	pub release: String,
	pub key: leptoskit::apm::SeriesKey,
	pub windows: Vec<leptoskit::apm::Window>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Fleet {
	pub hosts: Vec<HostReport>,
	pub retained_from: Option<u64>,
	pub from: u64,
	pub to: u64,
	pub instances: usize,
	pub stale_instances: usize,
	pub series: Vec<FleetSeries>,
	pub limitations: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HostReport {
	pub instance: String,
	pub release: String,
	pub seen: u64,
	pub host: Option<leptoskit::apm::HostSample>,
	pub export: Option<leptoskit::apm::ExportHealth>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Overview {
	pub format_version: u32,
	pub projects: Vec<Project>,
	pub operations: Vec<Operation>,
}

/// One authenticated caller of the deployment service. The HTTP layer
/// resolves it from the owner key, a stored bearer token or a browser
/// session; embedders that authenticate people themselves construct it
/// directly with [`Principal::owner`] or [`Principal::scoped`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Principal {
	pub scope: Scope,
	pub project: Option<String>,
	pub session: bool,
	/// Who performed the action, e.g. the admin's account ID or email.
	pub actor: Option<String>,
}

impl Principal {
	/// The service owner's principal, for embedders acting with the owner's
	/// authority.
	#[must_use]
	pub fn owner() -> Self {
		Self {
			scope: Scope::Owner,
			project: None,
			session: false,
			actor: None,
		}
	}

	/// A principal for one registered project. Project IDs validate against
	/// the registry, so an unknown or malformed ID simply grants nothing.
	/// Owner authority is available only through [`Principal::owner`].
	#[must_use]
	pub fn scoped(scope: Scope, project: impl Into<String>) -> Self {
		Self {
			scope,
			project: Some(project.into()),
			session: false,
			actor: None,
		}
	}

	/// Records who performed the action, shown in operation history.
	#[must_use]
	pub fn with_actor(mut self, actor: impl Into<String>) -> Self {
		self.actor = Some(actor.into());
		self
	}

	pub fn allows(&self, project: &str, scope: Scope) -> bool {
		self.project
			.as_ref()
			.is_none_or(|allowed| allowed == project)
			&& (self.scope == Scope::Owner
				|| self.scope == scope
				|| (self.scope == Scope::Deploy && scope == Scope::Read))
	}
}

/// One journaled change to an operation, published to subscribers of
/// [`Service::subscribe`](crate::control::Service::subscribe).
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Update {
	/// The operation's phase or message changed.
	Progress { operation_id: String, event: Event },
	/// The operation reached a terminal phase.
	Finished { operation: Operation },
}

/// A live stream of journal updates for one operation, created by
/// [`Service::subscribe`](crate::control::Service::subscribe). It yields
/// progress events and ends after the terminal operation. A slow consumer
/// that falls behind the bounded broadcast skips missed updates; the
/// operation endpoint always returns the authoritative state.
pub struct Subscription {
	id: String,
	inner: Pin<Box<dyn Stream<Item = Update> + Send>>,
}

impl Subscription {
	pub(crate) fn new(id: String, receiver: tokio::sync::broadcast::Receiver<Update>) -> Self {
		use tokio_stream::StreamExt as _;
		let id_for_filter = id.clone();
		let stream =
			tokio_stream::wrappers::BroadcastStream::new(receiver).filter_map(move |update| {
				match update {
					Ok(update) if update.operation_id() == id_for_filter => Some(update),
					Ok(_) | Err(_) => None,
				}
			});
		Self {
			id,
			inner: Box::pin(stream),
		}
	}

	/// The operation this subscription follows.
	#[must_use]
	pub fn operation_id(&self) -> &str {
		&self.id
	}
}

impl Stream for Subscription {
	type Item = Update;

	fn poll_next(
		mut self: Pin<&mut Self>,
		cx: &mut std::task::Context<'_>,
	) -> std::task::Poll<Option<Update>> {
		self.inner.as_mut().poll_next(cx)
	}
}

impl Update {
	/// The operation the update belongs to.
	#[must_use]
	pub fn operation_id(&self) -> &str {
		match self {
			Self::Progress { operation_id, .. } => operation_id,
			Self::Finished { operation } => &operation.id,
		}
	}
}

/// Why the durable queue refused an intent. [`Rejection::code`] is the
/// stable wire vocabulary the HTTP API returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rejection {
	InvalidIntent,
	InvalidActor,
	RevisionConflict,
	IdempotencyConflict,
	OperationInProgress,
	PlanIsNotCurrent,
	PlanAlreadyConsumed,
	UnknownProject,
}

impl Rejection {
	/// The stable wire code an HTTP response maps this rejection to. An
	/// unknown project reports the generic rejection code so the API does
	/// not reveal which projects exist.
	#[must_use]
	pub fn code(self) -> &'static str {
		match self {
			Self::InvalidIntent => "invalid_intent",
			Self::InvalidActor => "invalid_actor",
			Self::RevisionConflict => "revision_conflict",
			Self::IdempotencyConflict => "idempotency_conflict",
			Self::OperationInProgress => "operation_in_progress",
			Self::PlanIsNotCurrent => "plan_is_not_current",
			Self::PlanAlreadyConsumed => "plan_already_consumed",
			Self::UnknownProject => "operation_rejected",
		}
	}
}

impl std::fmt::Display for Rejection {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str(self.code())
	}
}

/// Typed outcome of [`Service::enqueue`](crate::control::Service::enqueue).
#[derive(Debug)]
#[allow(
	variant_size_differences,
	reason = "one is made per request and reported immediately"
)]
pub enum EnqueueError {
	/// The principal's scope does not cover this action on this project.
	Forbidden,
	/// The durable queue refused the intent.
	Rejected(Rejection),
	/// The management store is temporarily unavailable.
	Storage(anyhow::Error),
}

impl From<anyhow::Error> for EnqueueError {
	fn from(error: anyhow::Error) -> Self {
		Self::Storage(error)
	}
}

impl From<rusqlite::Error> for EnqueueError {
	fn from(error: rusqlite::Error) -> Self {
		Self::Storage(error.into())
	}
}

impl From<serde_json::Error> for EnqueueError {
	fn from(error: serde_json::Error) -> Self {
		Self::Storage(error.into())
	}
}

impl std::fmt::Display for EnqueueError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Forbidden => f.write_str("scope denied"),
			Self::Rejected(rejection) => write!(f, "operation rejected: {rejection}"),
			Self::Storage(error) => write!(f, "management store unavailable: {error}"),
		}
	}
}

impl std::error::Error for EnqueueError {
	fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
		match self {
			Self::Storage(error) => Some(error.as_ref()),
			_ => None,
		}
	}
}

/// Typed outcome of the read and token calls on
/// [`Service`](crate::control::Service).
#[derive(Debug)]
pub enum FacadeError {
	/// The principal's scope does not cover this request.
	Forbidden,
	/// The requested operation does not exist or is not visible.
	NotFound,
	/// The request was rejected, e.g. an invalid analytics range.
	Invalid(anyhow::Error),
	/// The management store is temporarily unavailable.
	Storage(anyhow::Error),
}

impl std::fmt::Display for FacadeError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Forbidden => f.write_str("scope denied"),
			Self::NotFound => f.write_str("not found"),
			Self::Invalid(error) => write!(f, "request rejected: {error}"),
			Self::Storage(error) => write!(f, "management store unavailable: {error}"),
		}
	}
}

impl std::error::Error for FacadeError {
	fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
		match self {
			Self::Invalid(error) | Self::Storage(error) => Some(error.as_ref()),
			_ => None,
		}
	}
}

/// A freshly issued management credential, shaped like the token endpoint's
/// JSON response.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssuedToken {
	pub key: String,
	pub expires_in: u64,
}
