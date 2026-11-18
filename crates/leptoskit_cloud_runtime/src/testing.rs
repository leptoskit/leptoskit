//! A [`Runtime`] that records every call, for tests of the control plane
//! and of code that sits above a runtime.
//!
//! The fake validates the specs it is handed like a real runtime would,
//! applies them into memory, and lets a test script how a rollout looks
//! afterwards: ready, never ready, or unreachable. Nothing here touches a
//! cluster or a network, and no dependency beyond the crate's base turns
//! the feature on.

use std::future;
use std::sync::Mutex;
use std::sync::PoisonError;

use crate::runtime::Applied;
use crate::runtime::LogLine;
use crate::runtime::LogQuery;
use crate::runtime::Observed;
use crate::runtime::ObservedProcess;
use crate::runtime::ProcessState;
use crate::runtime::Runtime;
use crate::runtime::RuntimeError;
use crate::runtime::RuntimeFuture;
use crate::spec::EnvironmentRef;
use crate::spec::Process;
use crate::spec::ReleaseSpec;

/// How the fake answers status calls, until a test changes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Script {
	/// Every process of the last release is ready. The default.
	#[default]
	Ready,
	/// The rollout never finishes: every process stays pending with no
	/// ready replica, forever.
	NeverReady,
	/// Every call answers [`RuntimeError::Unavailable`].
	Unavailable,
}

/// One call a test can read back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Call {
	Apply {
		environment: EnvironmentRef,
		deployment: u64,
	},
	Status {
		environment: EnvironmentRef,
	},
	Logs {
		environment: EnvironmentRef,
		query: LogQuery,
	},
	Destroy {
		environment: EnvironmentRef,
	},
}

#[derive(Debug, Default)]
struct Inner {
	script: Script,
	calls: Vec<Call>,
	applied: Vec<ReleaseSpec>,
	logs: Vec<LogLine>,
	now_millis: i64,
}

/// The recording fake. Cheap to clone-free share: every method takes
/// `&self`, so a test passes one reference to everything under test.
#[derive(Debug, Default)]
pub struct Fake {
	inner: Mutex<Inner>,
}

impl Fake {
	/// An empty fake that answers [`Script::Ready`].
	#[must_use]
	pub fn new() -> Self {
		Self::default()
	}

	/// Scripts how the fake answers from now on.
	pub fn script(&self, script: Script) {
		self.lock().script = script;
	}

	/// The calls the fake has answered, in order.
	#[must_use]
	pub fn calls(&self) -> Vec<Call> {
		self.lock().calls.clone()
	}

	/// Every spec the fake has applied, oldest first. A destroy removes its
	/// environment's specs, as a real runtime would stop serving them.
	#[must_use]
	pub fn applied(&self) -> Vec<ReleaseSpec> {
		self.lock().applied.clone()
	}

	/// Gives the fake a log line a real runtime would have read from a
	/// process. Lines answer `logs` calls in the order pushed.
	pub fn push_log(&self, line: LogLine) {
		self.lock().logs.push(line);
	}

	/// Sets what "now" is, in milliseconds since the Unix epoch, for
	/// `since_seconds` filters. Zero by default, which passes every line.
	pub fn set_now_millis(&self, millis: i64) {
		self.lock().now_millis = millis;
	}

	/// A poisoned lock means a test panicked while holding it; the fake's
	/// recorded state is still whole, so the next call recovers it.
	fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
		self.inner.lock().unwrap_or_else(PoisonError::into_inner)
	}

	fn observed_process(process: &Process, script: Script) -> ObservedProcess {
		let desired = if process.scale.sleep {
			0
		} else {
			process.scale.min
		};
		// An outage answers pending; the error comes from `status_sync`.
		let (ready, state) = if script == Script::Ready {
			(desired, ProcessState::Ready)
		} else {
			(0, ProcessState::Pending)
		};
		ObservedProcess {
			name: process.name.clone(),
			ready_replicas: ready,
			desired_replicas: desired,
			image: process.image.reference(),
			state,
		}
	}
}

impl Runtime for Fake {
	fn apply(&self, spec: &ReleaseSpec) -> RuntimeFuture<'_, Result<Applied, RuntimeError>> {
		// The call's whole work happens now; the box only carries its answer.
		Box::pin(future::ready(self.apply_sync(spec)))
	}

	fn status(
		&self,
		environment: &EnvironmentRef,
	) -> RuntimeFuture<'_, Result<Observed, RuntimeError>> {
		Box::pin(future::ready(self.status_sync(environment)))
	}

	fn logs(
		&self,
		environment: &EnvironmentRef,
		query: &LogQuery,
	) -> RuntimeFuture<'_, Result<Vec<LogLine>, RuntimeError>> {
		Box::pin(future::ready(self.logs_sync(environment, query)))
	}

	fn destroy(&self, environment: &EnvironmentRef) -> RuntimeFuture<'_, Result<(), RuntimeError>> {
		Box::pin(future::ready(self.destroy_sync(environment)))
	}
}

/// The sync halves of the four calls, so the tests need no executor.
impl Fake {
	fn apply_sync(&self, spec: &ReleaseSpec) -> Result<Applied, RuntimeError> {
		spec.validate()?;
		let mut inner = self.lock();
		inner.calls.push(Call::Apply {
			environment: spec.environment.clone(),
			deployment: spec.deployment,
		});
		inner.applied.push(spec.clone());
		Ok(Applied {
			environment: spec.environment.clone(),
			deployment: spec.deployment,
			processes: spec.processes.iter().map(|p| p.name.clone()).collect(),
		})
	}

	fn status_sync(&self, environment: &EnvironmentRef) -> Result<Observed, RuntimeError> {
		let mut inner = self.lock();
		inner.calls.push(Call::Status {
			environment: environment.clone(),
		});
		let script = inner.script;
		if script == Script::Unavailable {
			return Err(RuntimeError::Unavailable);
		}
		let last = inner
			.applied
			.iter()
			.rev()
			.find(|spec| &spec.environment == environment);
		let processes = match last {
			None => Vec::new(),
			Some(spec) => {
				spec.processes
					.iter()
					.map(|process| Self::observed_process(process, script))
					.collect()
			}
		};
		Ok(Observed {
			environment: environment.clone(),
			processes,
		})
	}

	fn logs_sync(
		&self,
		environment: &EnvironmentRef,
		query: &LogQuery,
	) -> Result<Vec<LogLine>, RuntimeError> {
		let mut inner = self.lock();
		inner.calls.push(Call::Logs {
			environment: environment.clone(),
			query: query.clone(),
		});
		if inner.script == Script::Unavailable {
			return Err(RuntimeError::Unavailable);
		}
		// Seconds fit an i64 comfortably; an absurd value clamps, it never
		// wraps into a negative cutoff.
		let since_millis = query
			.since_seconds
			.unwrap_or(0)
			.saturating_mul(1000)
			.min(i64::MAX as u64) as i64;
		let cutoff = inner.now_millis.saturating_sub(since_millis);
		let mut lines: Vec<LogLine> = inner
			.logs
			.iter()
			.filter(|line| query.process.as_ref().is_none_or(|p| &line.process == p))
			.filter(|line| line.at_millis >= cutoff)
			.cloned()
			.collect();
		if let Some(limit) = query.limit {
			// The most recent lines: drop from the front, keep the tail.
			let skip = lines.len().saturating_sub(limit as usize);
			lines.drain(..skip);
		}
		Ok(lines)
	}

	/// The trait's every call answers a `Result`, so destroy keeps the shape
	/// even though removing an environment cannot fail.
	#[expect(
		clippy::unnecessary_wraps,
		reason = "the trait's every call answers a Result; the four calls stay parallel"
	)]
	fn destroy_sync(&self, environment: &EnvironmentRef) -> Result<(), RuntimeError> {
		let mut inner = self.lock();
		inner.calls.push(Call::Destroy {
			environment: environment.clone(),
		});
		inner
			.applied
			.retain(|spec| &spec.environment != environment);
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use std::collections::BTreeMap;

	use futures::executor::block_on;

	use super::*;
	use crate::spec::Image;
	use crate::spec::ProcessKind;
	use crate::spec::Scale;

	fn spec(environment_slug: &str) -> ReleaseSpec {
		let environment = EnvironmentRef {
			organization: crate::spec::NamedRef {
				slug: "acme".to_owned(),
				id: uuid::Uuid::from_u128(1),
			},
			project: crate::spec::NamedRef {
				slug: "notes".to_owned(),
				id: uuid::Uuid::from_u128(2),
			},
			environment: crate::spec::NamedRef {
				slug: environment_slug.to_owned(),
				// Distinct ids per name, so the tests' environments differ.
				id: uuid::Uuid::from_u128(match environment_slug {
					"production" => 3,
					_ => 4,
				}),
			},
		};
		ReleaseSpec {
			environment,
			deployment: 7,
			processes: vec![Process {
				name: "app".to_owned(),
				kind: ProcessKind::App,
				image: Image {
					repository: "ghcr.io/acme/notes".to_owned(),
					digest: format!("sha256:{}", "a".repeat(64)),
				},
				command: vec![],
				args: vec![],
				port: 8080,
				probes: crate::spec::Probes {
					ready: "/readyz".to_owned(),
					live: "/healthz".to_owned(),
				},
				env: BTreeMap::new(),
				secret_env: BTreeMap::new(),
				resources: crate::spec::Resources {
					cpu_request_millis: 100,
					cpu_limit_millis: 500,
					memory_mib: 128,
				},
				scale: Scale {
					min: 1,
					max: 2,
					sleep: false,
				},
				volumes: vec![],
			}],
			routes: vec![],
			policy: crate::spec::Policy {
				runtime_class: None,
				egress: crate::spec::Egress::None,
				quota: crate::spec::Quota {
					cpu_millis: 1000,
					memory_mib: 256,
					pods: 2,
					storage_gib: 1,
				},
				database: None,
			},
		}
	}

	fn environment(slug: &str) -> EnvironmentRef {
		spec(slug).environment
	}

	#[test]
	fn apply_validates_the_spec() {
		let fake = Fake::new();
		let mut invalid = spec("production");
		invalid.processes[0].port = 0;
		let error = block_on(fake.apply(&invalid)).expect_err("an invalid spec");
		assert!(matches!(error, RuntimeError::Invalid(_)));
		assert!(fake.calls().is_empty(), "a refused spec leaves no call");
	}

	#[test]
	fn apply_records_and_answers() {
		let fake = Fake::new();
		let spec = spec("production");
		let applied = block_on(fake.apply(&spec)).expect("a valid spec applies");
		assert_eq!(applied.deployment, 7);
		assert_eq!(applied.processes, vec!["app".to_owned()]);
		assert_eq!(fake.applied(), vec![spec.clone()]);
		assert_eq!(
			fake.calls(),
			vec![Call::Apply {
				environment: spec.environment.clone(),
				deployment: 7,
			}]
		);
	}

	#[test]
	fn status_reports_ready_then_never_ready() {
		let fake = Fake::new();
		let spec = spec("production");
		block_on(fake.apply(&spec)).expect("applies");

		let observed = block_on(fake.status(&spec.environment)).expect("ready");
		assert_eq!(observed.processes.len(), 1);
		assert_eq!(observed.processes[0].state, ProcessState::Ready);
		assert_eq!(observed.processes[0].desired_replicas, 1);
		assert_eq!(observed.processes[0].ready_replicas, 1);
		assert_eq!(
			observed.processes[0].image,
			spec.processes[0].image.reference()
		);

		fake.script(Script::NeverReady);
		let observed = block_on(fake.status(&spec.environment)).expect("never ready");
		assert_eq!(observed.processes[0].state, ProcessState::Pending);
		assert_eq!(observed.processes[0].ready_replicas, 0);
	}

	#[test]
	fn status_of_an_unapplied_environment_is_empty() {
		let fake = Fake::new();
		let observed = block_on(fake.status(&environment("production"))).expect("empty");
		assert_eq!(observed.processes, []);
	}

	#[test]
	fn the_scripted_outage_covers_every_call() {
		let fake = Fake::new();
		let spec = spec("production");
		block_on(fake.apply(&spec)).expect("applies before the outage");
		fake.script(Script::Unavailable);

		let error = block_on(fake.status(&spec.environment)).expect_err("unavailable");
		assert!(matches!(error, RuntimeError::Unavailable));
		let error =
			block_on(fake.logs(&spec.environment, &LogQuery::default())).expect_err("unavailable");
		assert!(matches!(error, RuntimeError::Unavailable));
	}

	#[test]
	fn destroy_removes_the_environment() {
		let fake = Fake::new();
		let spec = spec("production");
		block_on(fake.apply(&spec)).expect("applies");
		block_on(fake.destroy(&spec.environment)).expect("destroys");
		assert_eq!(fake.applied(), []);
		assert_eq!(
			fake.calls().last(),
			Some(&Call::Destroy {
				environment: spec.environment.clone()
			})
		);
		let observed = block_on(fake.status(&spec.environment)).expect("empty after destroy");
		assert_eq!(observed.processes, []);
	}

	#[test]
	fn logs_follow_the_query() {
		let fake = Fake::new();
		fake.push_log(LogLine {
			process: "app".to_owned(),
			at_millis: 1_000,
			message: "first".to_owned(),
		});
		fake.push_log(LogLine {
			process: "search".to_owned(),
			at_millis: 2_000,
			message: "second".to_owned(),
		});
		fake.push_log(LogLine {
			process: "app".to_owned(),
			at_millis: 3_000,
			message: "third".to_owned(),
		});

		let query = LogQuery {
			process: Some("app".to_owned()),
			since_seconds: None,
			limit: None,
		};
		let lines = block_on(fake.logs(&environment("production"), &query)).expect("logs");
		assert_eq!(
			lines
				.iter()
				.map(|line| line.message.as_str())
				.collect::<Vec<_>>(),
			vec!["first", "third"]
		);

		let query = LogQuery {
			process: None,
			since_seconds: None,
			limit: Some(2),
		};
		let lines = block_on(fake.logs(&environment("production"), &query)).expect("logs");
		assert_eq!(
			lines
				.iter()
				.map(|line| line.message.as_str())
				.collect::<Vec<_>>(),
			vec!["second", "third"],
			"the limit keeps the most recent lines"
		);

		fake.set_now_millis(3_000);
		let query = LogQuery {
			process: None,
			since_seconds: Some(1),
			limit: None,
		};
		let lines = block_on(fake.logs(&environment("production"), &query)).expect("logs");
		assert_eq!(
			lines
				.iter()
				.map(|line| line.message.as_str())
				.collect::<Vec<_>>(),
			vec!["second", "third"],
			"only lines from the last second"
		);
	}
}
