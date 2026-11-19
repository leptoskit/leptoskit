//! The adapter boundary between the control plane and a place environments
//! run.
//!
//! A [`Runtime`] applies a [`ReleaseSpec`] to one place — a Kubernetes
//! cluster, a machine's local processes, or a recording fake in a test —
//! and answers what is running there. The methods return boxed futures so
//! `dyn Runtime` works with no extra machinery: the control plane holds one
//! runtime per cluster behind the same trait object.
//!
//! This module carries no cluster access of its own. The `kube` feature
//! provides the cluster adapter; [`crate::testing::Fake`] records test calls.

use std::future::Future;
use std::pin::Pin;

use serde::Deserialize;
use serde::Serialize;

use crate::spec::EnvironmentRef;
use crate::spec::ReleaseSpec;
use crate::spec::SpecError;

/// A boxed future, so `Runtime` is usable as `dyn Runtime`.
pub type RuntimeFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// What an apply put into one environment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Applied {
	pub environment: EnvironmentRef,
	/// The deployment that now serves the environment.
	pub deployment: u64,
	/// The process names the release brought, in spec order.
	pub processes: Vec<String>,
}

/// What a process looks like from the outside, as a rollout runs or rests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservedProcess {
	pub name: String,
	/// How many replicas answer right now.
	pub ready_replicas: u32,
	/// How many replicas the runtime wants up: the release's minimum, so a
	/// sleeping process shows zero here.
	pub desired_replicas: u32,
	/// The image the process's pods were started with.
	pub image: String,
	/// Sum of restart counts on existing pods owned by the current
	/// `ReplicaSet`. Replaced pods take their counts with them; kubelet can
	/// reset a count.
	#[serde(default)]
	pub observed_restarts: u64,
	/// Latest OOM termination retained in those pods' current or last container
	/// states, milliseconds since Unix epoch. This is evidence, not an OOM
	/// total.
	#[serde(default)]
	pub last_oom_kill: Option<i64>,
	pub state: ProcessState,
}

/// Where a process's rollout stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum ProcessState {
	/// No replicas are requested: the process sleeps until traffic wakes it.
	Sleeping,
	/// The rollout has not finished; some replicas are not ready yet.
	Pending,
	/// Every replica the release wants is ready and serving.
	Ready,
	/// The rollout cannot finish. The reason is what the runtime observed,
	/// such as an image that fails to start.
	Failed { reason: String },
}

/// What one environment looks like from the outside.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observed {
	pub environment: EnvironmentRef,
	pub processes: Vec<ObservedProcess>,
}

/// Which process's logs to read, and how much of them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogQuery {
	/// One process's logs, or every process's when absent.
	pub process: Option<String>,
	/// Only lines newer than this many seconds.
	pub since_seconds: Option<u64>,
	/// At most this many lines, the most recent ones.
	pub limit: Option<u32>,
}

/// One line a process wrote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogLine {
	/// Pod that wrote the line; absent for adapters without pods.
	#[serde(default)]
	pub pod: Option<String>,
	/// The process that wrote the line.
	pub process: String,
	/// When the line was written, in milliseconds since the Unix epoch. A
	/// plain number keeps this type a wire type with no clock dependency.
	pub at_millis: i64,
	pub message: String,
}

/// Why a runtime call failed.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
	/// The spec the call carried is not applicable anywhere.
	#[error("the release spec is invalid: {0}")]
	Invalid(#[from] SpecError),
	/// A rollout never became ready; the release before it keeps serving.
	#[error("{process} never became ready: {reason}")]
	NotReady { process: String, reason: String },
	/// The place the environment runs cannot be reached now.
	#[error("the runtime is unreachable right now; try again later")]
	Unavailable,
	/// The credentials the call carried do not allow it.
	#[error("the runtime refused the call: the credentials do not allow it")]
	Forbidden,
	/// The environment already holds a different, newer release.
	#[error("the environment already holds a different release; reconcile it before applying")]
	Conflict,
	#[error("{0}")]
	Other(String),
}

/// A place environments run. Every method takes the environment it acts on,
/// so one adapter can hold many.
///
/// # Applying
/// `apply` replaces the environment's processes with the spec's, waiting
/// for each rollout to become ready before returning. A release that never
/// becomes ready answers [`RuntimeError::NotReady`] and leaves the
/// previous release serving — the rollouts replace pods with
/// `maxUnavailable: 0`, so nothing serving stops before its replacement is
/// ready.
pub trait Runtime: Send + Sync {
	fn apply(&self, spec: &ReleaseSpec) -> RuntimeFuture<'_, Result<Applied, RuntimeError>>;
	fn status(
		&self,
		environment: &EnvironmentRef,
	) -> RuntimeFuture<'_, Result<Observed, RuntimeError>>;
	fn logs(
		&self,
		environment: &EnvironmentRef,
		query: &LogQuery,
	) -> RuntimeFuture<'_, Result<Vec<LogLine>, RuntimeError>>;
	fn destroy(&self, environment: &EnvironmentRef) -> RuntimeFuture<'_, Result<(), RuntimeError>>;
}
