//! The cluster role: applying an environment's releases to the one
//! cluster the worker was paired for, and removing environments from it.
//!
//! A release goes through a [`Place`], which gives a
//! [`Runtime`](leptoskit_cloud_runtime::Runtime) for each job holding that
//! job's leased secrets, and the environment's managed Postgres database is
//! made sure of first, through [`Databases`]. The control plane never talks
//! to the cluster or to its Postgres server: this worker does, with
//! credentials that are its own.
//!
//! The worker trusts the control plane only so far. A database it is told
//! to make or drop must be the one named for the job's own environment,
//! and a release's policy must keep within the [`Bounds`] its operator set:
//! the runtime class pods run under, the one database server they may
//! reach, and the platform services, such as the cluster's notifications
//! hub, they may be bridged to.
//!
//! An environment whose notifications the cluster's hub serves has a second
//! database on the same server, named [`notifications_database_name`]. The
//! worker makes sure of it with each release, from the password leased as
//! [`DATABASE_URL_LEASE`](leptoskit_cloud::notifications::DATABASE_URL_LEASE),
//! and drops it with the environment. That lease is for the worker alone:
//! it is taken out of what the release's runtime is given, so no process
//! and no Kubernetes object ever holds it.
//!
//! Backups and restores aren't taken yet: a worker given one answers that
//! it failed, which leaves the database as it was.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use leptoskit_cloud::notifications::DATABASE_URL_LEASE;
use leptoskit_cloud::release::DATABASE_URL;
use leptoskit_cloud::workers::Assignment;
use leptoskit_cloud::workers::BackupOutcome;
use leptoskit_cloud::workers::DatabaseStep;
use leptoskit_cloud::workers::DestroyEnvironmentInput;
use leptoskit_cloud::workers::DestroyOutcome;
use leptoskit_cloud::workers::JobInput;
use leptoskit_cloud::workers::JobOutcome;
use leptoskit_cloud::workers::ReleaseInput;
use leptoskit_cloud::workers::ReleaseOutcome;
use leptoskit_cloud::workers::RestoreOutcome;
use leptoskit_cloud_runtime::ClusterProfile;
use leptoskit_cloud_runtime::DatabaseAccess;
use leptoskit_cloud_runtime::EnvironmentRef;
use leptoskit_cloud_runtime::PlatformService;
use leptoskit_cloud_runtime::Policy;
use leptoskit_cloud_runtime::Runtime;
use leptoskit_cloud_runtime::RuntimeError;
use leptoskit_cloud_runtime::SecretValues;

use crate::logs::LogSink;

/// A boxed future, so the traits here work as trait objects.
pub type Pending<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The cluster a worker applies releases to.
pub trait Place: Send + Sync {
	/// A runtime for one job, holding `secrets`: the values behind the
	/// spec's `secret_env` that the claim leased to it.
	///
	/// # Errors
	///
	/// When the cluster can't be reached or the worker's credentials for
	/// it can't be read.
	fn runtime(&self, secrets: SecretValues)
	-> Pending<'_, Result<Box<dyn Runtime>, RuntimeError>>;
}

/// The Postgres server environments' managed databases are on.
pub trait Databases: Send + Sync {
	/// Makes sure the role and the database `step` names exist, as
	/// [`DatabaseStep`] sets out, with the role logging in with `password`.
	/// Doing so again changes nothing but the password.
	///
	/// The error is said in words for the deployment's sheet, and never
	/// holds the password.
	fn ensure<'a>(
		&'a self,
		step: &'a DatabaseStep,
		password: &'a str,
	) -> Pending<'a, Result<(), String>>;

	/// Drops the database `name` and its role, both called `name`, if they
	/// are there.
	fn remove<'a>(&'a self, name: &'a str) -> Pending<'a, Result<(), String>>;
}

/// A [`Place`] that is a Kubernetes cluster, reached with the worker's own
/// service account, or with a kubeconfig file while developing.
#[derive(Clone, Debug)]
pub struct Kubernetes {
	profile: ClusterProfile,
	kubeconfig: Option<std::path::PathBuf>,
	deadline: Duration,
}

impl Kubernetes {
	/// The cluster the worker runs in, as `profile` describes it.
	#[must_use]
	pub fn in_cluster(profile: ClusterProfile, deadline: Duration) -> Self {
		Self {
			profile,
			kubeconfig: None,
			deadline,
		}
	}

	/// The cluster `kubeconfig` names, for development.
	#[must_use]
	pub fn from_kubeconfig(
		kubeconfig: std::path::PathBuf,
		profile: ClusterProfile,
		deadline: Duration,
	) -> Self {
		Self {
			profile,
			kubeconfig: Some(kubeconfig),
			deadline,
		}
	}
}

impl Place for Kubernetes {
	fn runtime(
		&self,
		secrets: SecretValues,
	) -> Pending<'_, Result<Box<dyn Runtime>, RuntimeError>> {
		Box::pin(async move {
			let cluster = match &self.kubeconfig {
				Some(path) => {
					leptoskit_cloud_runtime::Cluster::from_kubeconfig(
						path,
						self.profile.clone(),
						secrets,
						self.deadline,
					)
					.await?
				}
				None => {
					leptoskit_cloud_runtime::Cluster::in_cluster(
						self.profile.clone(),
						secrets,
						self.deadline,
					)?
				}
			};
			Ok(Box::new(cluster) as Box<dyn Runtime>)
		})
	}
}

/// What the operator lets a release's policy be, whatever the control
/// plane sends: a release outside them is refused before anything is
/// applied.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bounds {
	/// The `RuntimeClass` every release's pods must run under. None lets a
	/// release keep the cluster's default.
	pub runtime_class: Option<String>,
	/// The one database server a release may open its network to. None
	/// refuses every release that asks for one.
	pub database: Option<DatabaseAccess>,
	/// The cluster's notifications hub, the one platform service a release
	/// may be bridged to, exactly as it is here. None refuses every release
	/// that names a platform service.
	pub notifications: Option<PlatformService>,
}

impl Bounds {
	/// Whether `policy` keeps within these bounds, and if not, why.
	///
	/// # Errors
	///
	/// Why, in words for the deployment's sheet.
	pub fn check(&self, policy: &Policy) -> Result<(), String> {
		if let Some(class) = &self.runtime_class
			&& policy.runtime_class.as_ref() != Some(class)
		{
			return Err(format!(
				"its processes don't run under the `{class}` runtime class this cluster requires"
			));
		}

		match &policy.database {
			Some(door) if self.database.as_ref() != Some(door) => {
				Err(
					"it opens its network to a database server that isn't this cluster's"
						.to_owned(),
				)
			}
			_ => Ok(()),
		}
	}

	/// Whether every service in `platform` is one of these bounds', and if
	/// not, why.
	///
	/// # Errors
	///
	/// Why, in words for the deployment's sheet.
	pub fn reaches(&self, platform: &[PlatformService]) -> Result<(), String> {
		match platform
			.iter()
			.find(|service| self.notifications.as_ref() != Some(*service))
		{
			Some(service) => {
				Err(format!(
					"it reaches a platform service, `{}`, that isn't this cluster's",
					service.name
				))
			}
			None => Ok(()),
		}
	}
}

/// The name of `environment`'s managed database, and of its role, as the
/// control plane makes it: `env_` and the environment's ID in hex.
#[must_use]
pub fn database_name(environment: &EnvironmentRef) -> String {
	format!("env_{}", environment.environment.id.simple())
}

/// The name of `environment`'s hosted notifications database, and of its
/// role, as the control plane makes it: `ntf_` and the environment's ID in
/// hex.
#[must_use]
pub fn notifications_database_name(environment: &EnvironmentRef) -> String {
	format!("ntf_{}", environment.environment.id.simple())
}

/// Whether `address`, the notifications database's leased URL, logs in as
/// `name`, and its password is in none of `secrets`, the values the
/// release's processes may be given: a control plane that leased it to the
/// app too would hand the tenant the hub's own database.
fn private_lease(
	address: Option<&str>,
	name: &str,
	secrets: &BTreeMap<String, String>,
) -> Result<(), String> {
	let address = address
		.and_then(|address| url::Url::parse(address).ok())
		.ok_or_else(|| "its address wasn't leased with the release".to_owned())?;
	let password = address.password().unwrap_or_default();

	if address.username() != name || password.is_empty() {
		return Err("its address isn't its role's".to_owned());
	}

	if secrets.values().any(|value| value.contains(password)) {
		return Err("its password is among what the app is given".to_owned());
	}

	Ok(())
}

/// What a cluster worker does with each kind of job.
#[derive(Clone)]
pub struct ClusterRole {
	place: Arc<dyn Place>,
	databases: Option<Arc<dyn Databases>>,
	bounds: Bounds,
	/// Environments observed on this cluster since the worker started, keyed
	/// by environment ID, so periodic status and runtime-log reports keep
	/// `environment_status`, `runtime_logs`, and compute metering current.
	tracked: Arc<std::sync::Mutex<BTreeMap<uuid::Uuid, EnvironmentRef>>>,
	/// Per-environment log cursor tracking each pod's last accepted timestamp
	/// and exact messages seen at that millisecond so multi-pod scrapes and
	/// same-millisecond bursts are neither dropped nor duplicated.
	log_cursors: Arc<std::sync::Mutex<BTreeMap<uuid::Uuid, EnvLogCursor>>>,
}

#[derive(Clone, Debug, Default)]
struct EnvLogCursor {
	last_scrape: Option<std::time::Instant>,
	by_pod: BTreeMap<String, PodCursor>,
}

#[derive(Clone, Debug)]
struct PodCursor {
	at_millis: i64,
	seen_messages: std::collections::BTreeSet<String>,
}

impl EnvLogCursor {
	fn since_seconds(&self) -> u64 {
		match self.last_scrape {
			Some(previous) => {
				previous
					.elapsed()
					.as_secs()
					.saturating_add(15)
					.clamp(30, 3600)
			}
			None => 300,
		}
	}

	fn is_unseen(&self, line: &leptoskit_cloud_runtime::LogLine) -> bool {
		let pod_key = pod_cursor_key(line);
		match self.by_pod.get(pod_key) {
			None => true,
			Some(cursor) if line.at_millis > cursor.at_millis => true,
			Some(cursor) if line.at_millis == cursor.at_millis => {
				!cursor.seen_messages.contains(&line.message)
			}
			Some(_) => false,
		}
	}

	fn advance<'a>(
		&mut self,
		accepted: impl IntoIterator<Item = &'a leptoskit_cloud_runtime::LogLine>,
	) {
		self.last_scrape = Some(std::time::Instant::now());
		for line in accepted {
			let pod_key = pod_cursor_key(line).to_owned();
			match self.by_pod.get_mut(&pod_key) {
				Some(cursor) if line.at_millis > cursor.at_millis => {
					cursor.at_millis = line.at_millis;
					cursor.seen_messages.clear();
					cursor.seen_messages.insert(line.message.clone());
				}
				Some(cursor) if line.at_millis == cursor.at_millis => {
					cursor.seen_messages.insert(line.message.clone());
				}
				Some(_) => {}
				None => {
					let mut seen_messages = std::collections::BTreeSet::new();
					seen_messages.insert(line.message.clone());
					self.by_pod.insert(
						pod_key,
						PodCursor {
							at_millis: line.at_millis,
							seen_messages,
						},
					);
				}
			}
		}
	}
}

fn pod_cursor_key(line: &leptoskit_cloud_runtime::LogLine) -> &str {
	line.pod.as_deref().unwrap_or(line.process.as_str())
}

impl std::fmt::Debug for ClusterRole {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		let tracked = self.tracked.lock().map_or(0, |guard| guard.len());
		formatter
			.debug_struct("ClusterRole")
			.field("databases", &self.databases.is_some())
			.field("bounds", &self.bounds)
			.field("tracked_environments", &tracked)
			.finish_non_exhaustive()
	}
}

impl ClusterRole {
	/// Applies releases to `place`. Without [`databases`](Self::databases),
	/// a release that needs a managed database fails.
	#[must_use]
	pub fn new(place: Arc<dyn Place>) -> Self {
		Self {
			place,
			databases: None,
			bounds: Bounds::default(),
			tracked: Arc::default(),
			log_cursors: Arc::default(),
		}
	}

	/// Refuses releases whose policy isn't within `bounds`.
	#[must_use]
	pub fn bounds(mut self, bounds: Bounds) -> Self {
		self.bounds = bounds;
		self
	}

	/// Makes environments' managed databases on `databases`.
	#[must_use]
	pub fn databases(mut self, databases: Arc<dyn Databases>) -> Self {
		self.databases = Some(databases);
		self
	}

	/// Records `environment` as active on this cluster for periodic status and
	/// log reporting.
	pub fn track(&self, environment: EnvironmentRef) {
		if let Ok(mut guard) = self.tracked.lock() {
			guard.insert(environment.environment.id, environment);
		}
	}

	/// Stops tracking `environment_id` for periodic status and log reporting.
	pub fn untrack(&self, environment_id: uuid::Uuid) {
		if let Ok(mut guard) = self.tracked.lock() {
			guard.remove(&environment_id);
		}
		if let Ok(mut guard) = self.log_cursors.lock() {
			guard.remove(&environment_id);
		}
	}

	/// Re-hydrates tracked environments from the control plane on startup so
	/// periodic status reporting, runtime logs, and compute metering resume
	/// immediately after a cluster worker pod restarts.
	///
	/// # Errors
	///
	/// Returns [`crate::client::CallError`] if the control plane cannot be
	/// reached or rejects the worker's key.
	pub async fn rehydrate(
		&self,
		control: &crate::client::Control,
	) -> Result<usize, crate::client::CallError> {
		let placed = control.cluster_environments().await?;
		let count = placed.len();
		for entry in placed {
			self.track(entry.environment.0);
		}
		Ok(count)
	}

	/// Snapshot of environments currently tracked for status reporting.
	#[must_use]
	pub fn tracked_environments(&self) -> Vec<EnvironmentRef> {
		self.tracked
			.lock()
			.map(|guard| guard.values().cloned().collect())
			.unwrap_or_default()
	}

	/// Scrapes `Runtime::status` and recent `Runtime::logs` for every tracked
	/// environment and reports them to `control` via `report_status`.
	///
	/// Environments that the control plane reports as `environment_not_found`
	/// are automatically untracked. Returns `(environment_id, result)` for
	/// every report attempted.
	pub async fn report_all(
		&self,
		control: &crate::client::Control,
	) -> Vec<(
		uuid::Uuid,
		Result<leptoskit_cloud::workers::StatusReceived, crate::client::CallError>,
	)> {
		let environments = self.tracked_environments();
		if environments.is_empty() {
			return Vec::new();
		}

		let runtime = match self.place.runtime(SecretValues::default()).await {
			Ok(runtime) => runtime,
			Err(error) => {
				eprintln!("leptoskit-cloud-worker: status scrape couldn't reach cluster: {error}");
				return Vec::new();
			}
		};

		let mut results = Vec::with_capacity(environments.len());
		let limit = u32::try_from(leptoskit_cloud::workers::MAX_STATUS_LOG_LINES).unwrap_or(500);

		for environment in environments {
			let env_id = environment.environment.id;
			let observed = match runtime.status(&environment).await {
				Ok(observed) => observed,
				Err(error) => {
					eprintln!(
						"leptoskit-cloud-worker: status scrape for environment {env_id} failed: \
						 {error}"
					);
					continue;
				}
			};

			let cursor = self
				.log_cursors
				.lock()
				.ok()
				.and_then(|guard| guard.get(&env_id).cloned())
				.unwrap_or_default();
			let query = leptoskit_cloud_runtime::LogQuery {
				process: None,
				since_seconds: Some(cursor.since_seconds()),
				limit: Some(limit),
			};
			let mut log_lines = match runtime.logs(&environment, &query).await {
				Ok(lines) => lines,
				Err(error) => {
					eprintln!(
						"leptoskit-cloud-worker: runtime log scrape for environment {env_id} \
						 failed: {error}"
					);
					Vec::new()
				}
			};

			// Sort chronologically across pods before deduplicating and
			// capping so oldest lines are dropped first and newer lines stay
			// in order.
			log_lines.sort_by(|left, right| {
				left.at_millis
					.cmp(&right.at_millis)
					.then_with(|| pod_cursor_key(left).cmp(pod_cursor_key(right)))
					.then_with(|| left.process.cmp(&right.process))
			});
			log_lines.retain(|line| cursor.is_unseen(line));
			if log_lines.len() > leptoskit_cloud::workers::MAX_STATUS_LOG_LINES {
				let drop_count = log_lines.len() - leptoskit_cloud::workers::MAX_STATUS_LOG_LINES;
				log_lines.drain(..drop_count);
			}

			let sent = log_lines.clone();
			let outcome = control.report_status(env_id, observed, log_lines).await;
			match &outcome {
				Ok(received) => {
					let accepted = usize::try_from(received.log_lines)
						.unwrap_or(sent.len())
						.min(sent.len());
					if let Ok(mut guard) = self.log_cursors.lock() {
						guard.entry(env_id).or_default().advance(&sent[..accepted]);
					}
				}
				Err(error) if error.is("environment_not_found") => {
					self.untrack(env_id);
				}
				Err(error) => {
					eprintln!(
						"leptoskit-cloud-worker: report_status for environment {env_id} failed: \
						 {error}"
					);
				}
			}
			results.push((env_id, outcome));
		}

		results
	}

	/// Runs `assignment`, which isn't a build, to its outcome.
	pub(crate) async fn run(&self, assignment: &Assignment, log: &LogSink) -> JobOutcome {
		match &assignment.input {
			JobInput::Release(input) => {
				let secrets = assignment.secrets.clone().into_values();
				JobOutcome::Release(self.release(input, secrets, log).await)
			}
			JobInput::DestroyEnvironment(input) => {
				JobOutcome::DestroyEnvironment(self.destroy(input, log).await)
			}
			JobInput::BackupDatabase(_) => {
				log.line("worker: this worker doesn't take backups yet");
				JobOutcome::BackupDatabase(BackupOutcome::Failed {
					reason: "this cluster's worker doesn't take backups yet".to_owned(),
				})
			}
			JobInput::RestoreDatabase(_) => {
				log.line("worker: this worker doesn't restore backups yet");
				JobOutcome::RestoreDatabase(RestoreOutcome::Failed {
					reason: "this cluster's worker doesn't restore backups yet".to_owned(),
				})
			}
			// The control plane gives builds to build workers only.
			JobInput::BuildServer(_) => {
				JobOutcome::BuildServer(leptoskit_cloud::workers::BuildOutcome::Failed {
					reason: "a cluster's worker doesn't build".to_owned(),
					steps: Vec::new(),
				})
			}
			JobInput::Publish(_) => {
				JobOutcome::Publish(leptoskit_cloud::workers::PublishOutcome::Failed {
					reason: "a cluster's worker doesn't publish".to_owned(),
				})
			}
		}
	}

	async fn release(
		&self,
		input: &ReleaseInput,
		mut secrets: BTreeMap<String, String>,
		log: &LogSink,
	) -> ReleaseOutcome {
		// The notifications database's password is the worker's alone:
		// whatever the spec says, no process or object is given it.
		let notifications_url = secrets.remove(DATABASE_URL_LEASE);
		let spec = &input.spec.0;
		// A failure before any process is touched is the app's to show.
		let first = spec
			.processes
			.first()
			.map_or_else(|| "app".to_owned(), |process| process.name.clone());
		let not_ready = |reason: String| {
			ReleaseOutcome::NotReady {
				process: first.clone(),
				reason,
			}
		};

		if let Err(reason) = self
			.bounds
			.check(&spec.policy)
			.and_then(|()| self.bounds.reaches(&spec.platform))
		{
			log.line(format!("worker: the release is refused: {reason}"));
			return not_ready(format!("the release is refused: {reason}"));
		}

		// Checked before either database is touched: the hub's password must
		// be its role's, and in nothing the release's processes are given.
		if input.notifications.is_some()
			&& let Err(reason) = private_lease(
				notifications_url.as_deref(),
				&notifications_database_name(&spec.environment),
				&secrets,
			) {
			log.line(format!("worker: the notifications database: {reason}"));
			return not_ready(format!("its notifications database: {reason}"));
		}

		if let Some(step) = &input.database {
			log.line("worker: making sure of the environment's database");

			let name = database_name(&spec.environment);
			let password = secrets.get(DATABASE_URL).map(String::as_str);

			if let Err(reason) = self.database(step, &name, password).await {
				log.line(format!("worker: the database: {reason}"));
				return not_ready(format!("its database: {reason}"));
			}
		}

		if let Some(step) = &input.notifications {
			log.line("worker: making sure of the environment's notifications database");
			let name = notifications_database_name(&spec.environment);

			if let Err(reason) = self
				.database(step, &name, notifications_url.as_deref())
				.await
			{
				log.line(format!("worker: the notifications database: {reason}"));
				return not_ready(format!("its notifications database: {reason}"));
			}
		}

		log.line(format!("worker: applying deployment #{}", spec.deployment));
		let runtime = match self.place.runtime(SecretValues::new(secrets)).await {
			Ok(runtime) => runtime,
			Err(error) => {
				log.line(format!("worker: the cluster: {error}"));
				return not_ready(format!("the cluster: {error}"));
			}
		};

		match runtime.apply(spec).await {
			Ok(applied) => {
				self.track(spec.environment.clone());
				log.line(format!(
					"worker: deployment #{} is ready: {}",
					applied.deployment,
					applied.processes.join(", ")
				));
				ReleaseOutcome::Applied
			}
			Err(RuntimeError::NotReady { process, reason }) => {
				log.line(format!("worker: {process} never became ready: {reason}"));
				ReleaseOutcome::NotReady { process, reason }
			}
			Err(error) => {
				log.line(format!("worker: the cluster: {error}"));
				not_ready(format!("the cluster: {error}"))
			}
		}
	}

	/// Makes sure of `step`'s database, which must be the one called `name`,
	/// with the password in `address`, the URL leased for it.
	async fn database(
		&self,
		step: &DatabaseStep,
		name: &str,
		address: Option<&str>,
	) -> Result<(), String> {
		if step.name != name || step.role != name {
			return Err("it isn't this environment's".to_owned());
		}

		if step.size.instance().is_some() {
			return Err(format!(
				"this cluster's worker doesn't run dedicated `{}` databases yet",
				step.size.name()
			));
		}

		let Some(databases) = &self.databases else {
			return Err("this cluster's worker has no database server".to_owned());
		};
		let password = address
			.and_then(|address| url::Url::parse(address).ok())
			.and_then(|address| address.password().map(str::to_owned))
			.ok_or_else(|| "its address wasn't leased with the release".to_owned())?;

		databases.ensure(step, &password).await
	}

	async fn destroy(&self, input: &DestroyEnvironmentInput, log: &LogSink) -> DestroyOutcome {
		let environment = &input.environment.0;

		// Checked before anything is removed, so a destroy that can't
		// finish doesn't start.
		let named = [
			(
				input.database.as_deref(),
				database_name(environment),
				"database",
			),
			(
				input.notifications.as_deref(),
				notifications_database_name(environment),
				"notifications database",
			),
		];
		let mut dropping = Vec::new();

		for (name, expected, what) in &named {
			let Some(name) = name else {
				continue;
			};

			if name != expected {
				log.line(format!("worker: the {what} named isn't this environment's"));
				return DestroyOutcome::Failed {
					reason: format!("its {what} isn't this environment's"),
				};
			}

			let Some(databases) = &self.databases else {
				log.line("worker: this worker has no database server");
				return DestroyOutcome::Failed {
					reason: "this cluster's worker has no database server".to_owned(),
				};
			};
			dropping.push((databases, *name, *what));
		}

		log.line("worker: removing the environment's processes");
		let removed = match self.place.runtime(SecretValues::default()).await {
			Ok(runtime) => runtime.destroy(environment).await,
			Err(error) => Err(error),
		};

		if let Err(error) = removed {
			log.line(format!("worker: the cluster: {error}"));
			return DestroyOutcome::Failed {
				reason: format!("the cluster: {error}"),
			};
		}

		// Each is dropped even when another couldn't be, so one failure
		// leaves no other behind; dropping again is harmless.
		let mut failed = Vec::new();

		for (databases, name, what) in dropping {
			log.line(format!("worker: dropping the environment's {what}"));

			if let Err(reason) = databases.remove(name).await {
				log.line(format!("worker: the {what}: {reason}"));
				failed.push(format!("its {what}: {reason}"));
			}
		}

		if !failed.is_empty() {
			return DestroyOutcome::Failed {
				reason: failed.join("; "),
			};
		}

		self.untrack(environment.environment.id);
		log.line("worker: the environment is removed");
		DestroyOutcome::Destroyed
	}
}

#[cfg(test)]
mod tests {
	use leptoskit_cloud_runtime::Egress;
	use leptoskit_cloud_runtime::NamedRef;
	use leptoskit_cloud_runtime::Quota;
	use uuid::Uuid;

	use super::*;

	fn policy(runtime_class: Option<&str>, database: Option<(&str, u16)>) -> Policy {
		Policy {
			runtime_class: runtime_class.map(str::to_owned),
			egress: Egress::Internet,
			quota: Quota {
				cpu_millis: 1000,
				memory_mib: 256,
				pods: 2,
				storage_gib: 1,
			},
			database: database.map(|(namespace, port)| {
				DatabaseAccess {
					namespace: namespace.to_owned(),
					port,
				}
			}),
		}
	}

	#[test]
	fn a_policy_must_keep_within_the_bounds() {
		let bounds = Bounds {
			runtime_class: Some("gvisor".to_owned()),
			database: Some(DatabaseAccess {
				namespace: "postgres".to_owned(),
				port: 5432,
			}),
			notifications: None,
		};
		assert_eq!(bounds.check(&policy(Some("gvisor"), None)), Ok(()));
		assert_eq!(
			bounds.check(&policy(Some("gvisor"), Some(("postgres", 5432)))),
			Ok(())
		);
		assert!(bounds.check(&policy(None, None)).is_err());
		assert!(bounds.check(&policy(Some("runc"), None)).is_err());
		assert!(
			bounds
				.check(&policy(Some("gvisor"), Some(("leptoskit-cloud", 5432))))
				.is_err()
		);
		assert!(
			bounds
				.check(&policy(Some("gvisor"), Some(("postgres", 6432))))
				.is_err()
		);

		// Without a database server, no release may reach one.
		let none = Bounds::default();
		assert_eq!(none.check(&policy(None, None)), Ok(()));
		assert!(none.check(&policy(None, Some(("postgres", 5432)))).is_err());
	}

	fn hub() -> PlatformService {
		PlatformService {
			name: "notifications".to_owned(),
			namespace: "leptoskit-notifications".to_owned(),
			host: "hub.leptoskit-notifications.svc.cluster.local".to_owned(),
			port: 8080,
		}
	}

	#[test]
	fn a_release_reaches_only_the_clusters_hub() {
		let bounds = Bounds {
			notifications: Some(hub()),
			..Bounds::default()
		};
		assert_eq!(bounds.reaches(&[]), Ok(()));
		assert_eq!(bounds.reaches(&[hub()]), Ok(()));

		let elsewhere = PlatformService {
			namespace: "kube-system".to_owned(),
			..hub()
		};
		let other_port = PlatformService {
			port: 6443,
			..hub()
		};
		let other_name = PlatformService {
			name: "mailer".to_owned(),
			..hub()
		};

		for service in [elsewhere, other_port, other_name] {
			assert!(
				bounds.reaches(&[hub(), service.clone()]).is_err(),
				"{service:?}"
			);
		}

		// Without a hub, no release may reach a platform service.
		assert!(Bounds::default().reaches(&[hub()]).is_err());
	}

	#[test]
	fn a_database_is_named_for_its_environment() {
		let id = Uuid::parse_str("0193a5b0-7c1d-7e2f-8a9b-0c1d2e3f4a5b").expect("an ID");
		let named = |id| {
			NamedRef {
				slug: "x".to_owned(),
				id,
			}
		};
		let environment = EnvironmentRef {
			organization: named(Uuid::nil()),
			project: named(Uuid::nil()),
			environment: named(id),
		};
		assert_eq!(
			database_name(&environment),
			"env_0193a5b07c1d7e2f8a9b0c1d2e3f4a5b"
		);
		assert_eq!(
			notifications_database_name(&environment),
			"ntf_0193a5b07c1d7e2f8a9b0c1d2e3f4a5b"
		);
	}

	/// What the fakes below were asked to do.
	#[derive(Clone, Debug, PartialEq, Eq)]
	enum Asked {
		Ensure {
			name: String,
			password: String,
		},
		Remove {
			name: String,
		},
		/// A runtime, and whether it held the app's and the notifications
		/// database's addresses.
		Runtime {
			database_url: bool,
			notifications: bool,
		},
	}

	#[derive(Clone, Default)]
	struct Recording {
		asked: Arc<std::sync::Mutex<Vec<Asked>>>,
		fake: Arc<leptoskit_cloud_runtime::testing::Fake>,
	}

	impl Recording {
		fn asked(&self) -> Vec<Asked> {
			self.asked.lock().expect("not poisoned").clone()
		}

		fn push(&self, asked: Asked) {
			self.asked.lock().expect("not poisoned").push(asked);
		}
	}

	impl Databases for Recording {
		fn ensure<'a>(
			&'a self,
			step: &'a DatabaseStep,
			password: &'a str,
		) -> Pending<'a, Result<(), String>> {
			self.push(Asked::Ensure {
				name: step.name.clone(),
				password: password.to_owned(),
			});
			Box::pin(async { Ok(()) })
		}

		fn remove<'a>(&'a self, name: &'a str) -> Pending<'a, Result<(), String>> {
			self.push(Asked::Remove {
				name: name.to_owned(),
			});
			Box::pin(async { Ok(()) })
		}
	}

	impl Place for Recording {
		fn runtime(
			&self,
			secrets: SecretValues,
		) -> Pending<'_, Result<Box<dyn Runtime>, RuntimeError>> {
			self.push(Asked::Runtime {
				database_url: secrets.get(DATABASE_URL).is_some(),
				notifications: secrets.get(DATABASE_URL_LEASE).is_some(),
			});
			let fake = Arc::clone(&self.fake);
			Box::pin(async move { Ok(Box::new(ArcRuntime(fake)) as Box<dyn Runtime>) })
		}
	}

	/// A shared fake, as a boxed runtime.
	struct ArcRuntime(Arc<leptoskit_cloud_runtime::testing::Fake>);

	impl Runtime for ArcRuntime {
		fn apply(
			&self,
			spec: &leptoskit_cloud_runtime::ReleaseSpec,
		) -> leptoskit_cloud_runtime::RuntimeFuture<
			'_,
			Result<leptoskit_cloud_runtime::Applied, RuntimeError>,
		> {
			self.0.apply(spec)
		}

		fn status(
			&self,
			environment: &EnvironmentRef,
		) -> leptoskit_cloud_runtime::RuntimeFuture<
			'_,
			Result<leptoskit_cloud_runtime::Observed, RuntimeError>,
		> {
			self.0.status(environment)
		}

		fn logs(
			&self,
			environment: &EnvironmentRef,
			query: &leptoskit_cloud_runtime::LogQuery,
		) -> leptoskit_cloud_runtime::RuntimeFuture<
			'_,
			Result<Vec<leptoskit_cloud_runtime::LogLine>, RuntimeError>,
		> {
			self.0.logs(environment, query)
		}

		fn destroy(
			&self,
			environment: &EnvironmentRef,
		) -> leptoskit_cloud_runtime::RuntimeFuture<'_, Result<(), RuntimeError>> {
			self.0.destroy(environment)
		}
	}

	fn environment() -> EnvironmentRef {
		let named = |slug: &str, id| {
			NamedRef {
				slug: slug.to_owned(),
				id: Uuid::from_u128(id),
			}
		};
		EnvironmentRef {
			organization: named("acme", 1),
			project: named("notes", 2),
			environment: named("production", 3),
		}
	}

	fn step(name: String) -> DatabaseStep {
		DatabaseStep {
			role: name.clone(),
			name,
			size: leptoskit_cloud::size::DatabaseSize::Shared,
			backups: leptoskit_cloud::size::Backups::Daily,
		}
	}

	/// A release of one process, reaching `platform`, with the app's and
	/// the notifications hub's databases.
	fn release(platform: Vec<PlatformService>) -> ReleaseInput {
		let environment = environment();
		let spec = leptoskit_cloud_runtime::ReleaseSpec {
			platform,
			deployment: 7,
			processes: vec![leptoskit_cloud_runtime::Process {
				name: "app".to_owned(),
				kind: leptoskit_cloud_runtime::ProcessKind::App,
				image: leptoskit_cloud_runtime::Image {
					repository: "registry.example/acme/notes".to_owned(),
					digest: format!("sha256:{}", "a".repeat(64)),
				},
				command: Vec::new(),
				args: Vec::new(),
				port: 8080,
				probes: leptoskit_cloud_runtime::Probes {
					ready: "/readyz".to_owned(),
					live: "/healthz".to_owned(),
				},
				env: BTreeMap::new(),
				secret_env: BTreeMap::from([(DATABASE_URL.to_owned(), DATABASE_URL.to_owned())]),
				resources: leptoskit_cloud_runtime::Resources {
					cpu_request_millis: 100,
					cpu_limit_millis: 500,
					memory_mib: 128,
				},
				scale: leptoskit_cloud_runtime::Scale {
					min: 1,
					max: 1,
					sleep: false,
				},
				volumes: Vec::new(),
			}],
			routes: Vec::new(),
			policy: policy(None, Some(("postgres", 5432))),
			environment: environment.clone(),
		};
		ReleaseInput {
			deployment: leptoskit_cloud::workers::DeploymentRef {
				organization: "acme".to_owned(),
				project: "notes".to_owned(),
				environment: "production".to_owned(),
				number: 7,
			},
			spec: leptoskit_cloud::workers::RuntimeValue(spec),
			migration: leptoskit_cloud::workers::MigrationStep::None {
				reason: "none".to_owned(),
			},
			previous: None,
			database: Some(step(database_name(&environment))),
			notifications: Some(step(notifications_database_name(&environment))),
		}
	}

	fn leased() -> BTreeMap<String, String> {
		let hex = environment().environment.id.simple().to_string();
		BTreeMap::from([
			(
				DATABASE_URL.to_owned(),
				format!("postgres://env_{hex}:app-password@db/env_{hex}"),
			),
			(
				DATABASE_URL_LEASE.to_owned(),
				format!("postgres://ntf_{hex}:hub-password@db/ntf_{hex}"),
			),
		])
	}

	fn role(recording: &Recording, notifications: Option<PlatformService>) -> ClusterRole {
		ClusterRole::new(Arc::new(recording.clone()))
			.bounds(Bounds {
				runtime_class: None,
				database: Some(DatabaseAccess {
					namespace: "postgres".to_owned(),
					port: 5432,
				}),
				notifications,
			})
			.databases(Arc::new(recording.clone()))
	}

	#[tokio::test]
	async fn a_release_makes_its_notifications_database_and_keeps_its_password() {
		let recording = Recording::default();
		let (log, _lines) = LogSink::collecting();
		let outcome = role(&recording, Some(hub()))
			.release(&release(vec![hub()]), leased(), &log)
			.await;

		assert_eq!(outcome, ReleaseOutcome::Applied);
		let hex = environment().environment.id.simple().to_string();
		assert_eq!(
			recording.asked(),
			vec![
				Asked::Ensure {
					name: format!("env_{hex}"),
					password: "app-password".to_owned(),
				},
				Asked::Ensure {
					name: format!("ntf_{hex}"),
					password: "hub-password".to_owned(),
				},
				// The app's address goes to the runtime, the hub's never.
				Asked::Runtime {
					database_url: true,
					notifications: false,
				},
			]
		);
	}

	#[tokio::test]
	async fn a_release_names_only_its_own_notifications_database() {
		let recording = Recording::default();
		let (log, _lines) = LogSink::collecting();
		let mut input = release(vec![hub()]);
		input.notifications = Some(step(format!("ntf_{}", Uuid::from_u128(9).simple())));
		let outcome = role(&recording, Some(hub()))
			.release(&input, leased(), &log)
			.await;

		assert_eq!(
			outcome,
			ReleaseOutcome::NotReady {
				process: "app".to_owned(),
				reason: "its notifications database: it isn't this environment's".to_owned(),
			}
		);
		assert!(
			!recording
				.asked()
				.iter()
				.any(|asked| matches!(asked, Asked::Runtime { .. }))
		);

		// Nor when its password is the app's too, or its role another's.
		for secrets in [
			{
				let mut secrets = leased();
				let hub = secrets[DATABASE_URL_LEASE].clone();
				secrets.insert("SHARED".to_owned(), hub);
				secrets
			},
			{
				let mut secrets = leased();
				secrets.insert(
					DATABASE_URL_LEASE.to_owned(),
					"postgres://admin:hub-password@db/postgres".to_owned(),
				);
				secrets
			},
		] {
			let recording = Recording::default();
			let outcome = role(&recording, Some(hub()))
				.release(&release(vec![hub()]), secrets, &log)
				.await;
			assert!(
				matches!(&outcome, ReleaseOutcome::NotReady { reason, .. } if reason.starts_with("its notifications database: its")),
				"{outcome:?}"
			);
			assert_eq!(recording.asked(), Vec::new());
		}

		// Nor is it made without its password.
		let recording = Recording::default();
		let mut secrets = leased();
		secrets.remove(DATABASE_URL_LEASE);
		let outcome = role(&recording, Some(hub()))
			.release(&release(vec![hub()]), secrets, &log)
			.await;
		assert_eq!(
			outcome,
			ReleaseOutcome::NotReady {
				process: "app".to_owned(),
				reason: "its notifications database: its address wasn't leased with the release"
					.to_owned(),
			}
		);
	}

	#[tokio::test]
	async fn a_release_bridged_to_another_hub_is_refused() {
		let recording = Recording::default();
		let (log, _lines) = LogSink::collecting();
		let elsewhere = PlatformService {
			host: "kubernetes.default.svc".to_owned(),
			..hub()
		};
		let outcome = role(&recording, Some(hub()))
			.release(&release(vec![elsewhere]), leased(), &log)
			.await;

		assert_eq!(
			outcome,
			ReleaseOutcome::NotReady {
				process: "app".to_owned(),
				reason: "the release is refused: it reaches a platform service, `notifications`, \
				         that isn't this cluster's"
					.to_owned(),
			}
		);
		assert_eq!(recording.asked(), Vec::new());
	}

	#[tokio::test]
	async fn a_destroy_drops_both_databases() {
		let recording = Recording::default();
		let (log, _lines) = LogSink::collecting();
		let environment = environment();
		let input = DestroyEnvironmentInput {
			environment: leptoskit_cloud::workers::RuntimeValue(environment.clone()),
			database: Some(database_name(&environment)),
			notifications: Some(notifications_database_name(&environment)),
		};
		let outcome = role(&recording, None).destroy(&input, &log).await;

		assert_eq!(outcome, DestroyOutcome::Destroyed);
		let removed: Vec<Asked> = recording
			.asked()
			.into_iter()
			.filter(|asked| matches!(asked, Asked::Remove { .. }))
			.collect();
		assert_eq!(
			removed,
			vec![
				Asked::Remove {
					name: database_name(&environment),
				},
				Asked::Remove {
					name: notifications_database_name(&environment),
				},
			]
		);

		// Another environment's is never dropped, and nothing is removed.
		let recording = Recording::default();
		let input = DestroyEnvironmentInput {
			notifications: Some(format!("ntf_{}", Uuid::from_u128(9).simple())),
			..input
		};
		let outcome = role(&recording, None).destroy(&input, &log).await;
		assert_eq!(
			outcome,
			DestroyOutcome::Failed {
				reason: "its notifications database isn't this environment's".to_owned(),
			}
		);
		assert_eq!(recording.asked(), Vec::new());
	}

	#[test]
	fn env_log_cursor_deduplicates_per_pod_and_preserves_same_ms_bursts() {
		let line = |at_millis: i64, pod: &str, message: &str| {
			leptoskit_cloud_runtime::LogLine {
				at_millis,
				process: "app".to_owned(),
				pod: Some(pod.to_owned()),
				message: message.to_owned(),
			}
		};
		let mut cursor = EnvLogCursor::default();
		assert_eq!(cursor.since_seconds(), 300);

		let first_a = line(1_000, "app-0", "listening");
		let second_a = line(1_000, "app-0", "ready");
		let first_b = line(950, "app-1", "booting");

		assert!(cursor.is_unseen(&first_a));
		assert!(cursor.is_unseen(&second_a));
		assert!(cursor.is_unseen(&first_b));

		// Simulate only the first line from app-0 being accepted in a batch.
		cursor.advance([&first_a]);

		assert!(!cursor.is_unseen(&first_a));
		// Same-millisecond burst from the same pod is still unseen.
		assert!(cursor.is_unseen(&second_a));
		// Earlier timestamp from a second pod is NOT masked by app-0's cursor.
		assert!(cursor.is_unseen(&first_b));

		cursor.advance([&second_a, &first_b]);
		assert!(!cursor.is_unseen(&second_a));
		assert!(!cursor.is_unseen(&first_b));
		assert!(cursor.is_unseen(&line(1_001, "app-0", "request")));
		assert!((30..=3600).contains(&cursor.since_seconds()));
	}
}
