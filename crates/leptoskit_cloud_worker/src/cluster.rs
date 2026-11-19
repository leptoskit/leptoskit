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
//! the runtime class pods run under, and the one database server they may
//! reach.
//!
//! Backups and restores aren't taken yet: a worker given one answers that
//! it failed, which leaves the database as it was.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

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
}

/// The name of `environment`'s managed database, and of its role, as the
/// control plane makes it: `env_` and the environment's ID in hex.
#[must_use]
pub fn database_name(environment: &EnvironmentRef) -> String {
	format!("env_{}", environment.environment.id.simple())
}

/// What a cluster worker does with each kind of job.
#[derive(Clone)]
pub struct ClusterRole {
	place: Arc<dyn Place>,
	databases: Option<Arc<dyn Databases>>,
	bounds: Bounds,
}

impl std::fmt::Debug for ClusterRole {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_struct("ClusterRole")
			.field("databases", &self.databases.is_some())
			.field("bounds", &self.bounds)
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
		secrets: BTreeMap<String, String>,
		log: &LogSink,
	) -> ReleaseOutcome {
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

		if let Err(reason) = self.bounds.check(&spec.policy) {
			log.line(format!("worker: the release is refused: {reason}"));
			return not_ready(format!("the release is refused: {reason}"));
		}

		if let Some(step) = &input.database {
			log.line("worker: making sure of the environment's database");

			if let Err(reason) = self.database(step, &spec.environment, &secrets).await {
				log.line(format!("worker: the database: {reason}"));
				return not_ready(format!("its database: {reason}"));
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

	/// Makes sure of `step`'s database, which must be `environment`'s, with
	/// the password in the leased `DATABASE_URL`.
	async fn database(
		&self,
		step: &DatabaseStep,
		environment: &EnvironmentRef,
		secrets: &BTreeMap<String, String>,
	) -> Result<(), String> {
		let name = database_name(environment);

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
		let password = secrets
			.get(DATABASE_URL)
			.and_then(|address| url::Url::parse(address).ok())
			.and_then(|address| address.password().map(str::to_owned))
			.ok_or_else(|| "its address wasn't leased with the release".to_owned())?;

		databases.ensure(step, &password).await
	}

	async fn destroy(&self, input: &DestroyEnvironmentInput, log: &LogSink) -> DestroyOutcome {
		let environment = &input.environment.0;

		// Checked before anything is removed, so a destroy that can't
		// finish doesn't start.
		let databases = match &input.database {
			None => None,
			Some(name) if *name != database_name(environment) => {
				log.line("worker: the database named isn't this environment's");
				return DestroyOutcome::Failed {
					reason: "its database isn't this environment's".to_owned(),
				};
			}
			Some(name) => {
				let Some(databases) = &self.databases else {
					log.line("worker: this worker has no database server");
					return DestroyOutcome::Failed {
						reason: "this cluster's worker has no database server".to_owned(),
					};
				};
				Some((databases, name))
			}
		};

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

		if let Some((databases, name)) = databases {
			log.line("worker: dropping the environment's database");

			if let Err(reason) = databases.remove(name).await {
				log.line(format!("worker: the database: {reason}"));
				return DestroyOutcome::Failed {
					reason: format!("its database: {reason}"),
				};
			}
		}

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
	}
}
