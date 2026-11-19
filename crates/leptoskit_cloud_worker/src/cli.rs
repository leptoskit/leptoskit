//! `leptoskit-cloud-worker`'s commands.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use leptoskit_cloud::workers::WorkerRole;
use leptoskit_cloud_runtime::DatabaseAccess;
use tokio::sync::watch;

use crate::client::Control;
use crate::client::Key;
use crate::cluster::Bounds;
use crate::cluster::ClusterRole;
use crate::cluster::Kubernetes;
use crate::command::CommandBuilder;
use crate::postgres::Postgres;
use crate::publish::CommandPublisher;
use crate::run::Settings;
use crate::run::Worker;
use crate::state::Identity;
use crate::state::State;

/// Pairs with a control plane, claims its jobs, and builds deployments.
#[derive(Debug, clap::Parser)]
#[command(name = "leptoskit-cloud-worker", version, about)]
pub enum Command {
	/// Pairs this machine with a control plane, once, with a code an
	/// operator issued, and keeps the key it is given in the state
	/// directory.
	Pair(Pair),
	/// Claims jobs and runs them, until it is told to stop. It finishes
	/// the job in hand first; a second signal stops it at once.
	Run(Box<Run>),
	/// Forgets a job that was started and never reported, after an
	/// operator has reconciled it.
	Forget(Forget),
	/// Built-in OCI image builder used as the default `--build-command` when
	/// no custom build program is specified.
	BuildApp(crate::builder::BuildAppArgs),
}

#[derive(Debug, clap::Args)]
pub struct Pair {
	/// The control plane's address, such as `https://cloud.example.com`.
	#[arg(long, env = "LEPTOSKIT_CLOUD_WORKER_URL")]
	pub url: String,
	/// The path the control plane serves its operations under.
	#[arg(long, default_value = "/rpc")]
	pub rpc: String,
	/// The pairing code, `lkp_…`. It works once, for ten minutes. Give it
	/// in the environment: an argument is seen by anyone who lists the
	/// machine's processes.
	#[arg(
		long,
		env = "LEPTOSKIT_CLOUD_WORKER_PAIRING_CODE",
		hide_env_values = true
	)]
	pub code: String,
	/// What operators call this worker, such as `builder-eu-1`.
	#[arg(long, env = "LEPTOSKIT_CLOUD_WORKER_NAME")]
	pub name: String,
	/// What this worker can do, such as `linux` and `x86_64`: a job runs
	/// only on a worker with every capability it needs.
	#[arg(long = "capability")]
	pub capabilities: Vec<String>,
	/// Where the worker keeps its key and its job's state.
	#[arg(long, env = "LEPTOSKIT_CLOUD_WORKER_STATE")]
	pub state: PathBuf,
}

#[derive(Debug, clap::Args)]
pub struct Run {
	/// Where the worker keeps its key and its job's state.
	#[arg(long, env = "LEPTOSKIT_CLOUD_WORKER_STATE")]
	pub state: PathBuf,
	/// Where each build's directory is made, and deleted afterwards.
	/// Defaults to `workspace` in the state directory.
	#[arg(long, env = "LEPTOSKIT_CLOUD_WORKER_WORKSPACE")]
	pub workspace: Option<PathBuf>,
	/// The program that builds: run once for each build, in the unpacked
	/// source. When omitted, defaults to this binary's built-in `build-app`
	/// OCI builder subcommand.
	#[arg(long, env = "LEPTOSKIT_CLOUD_WORKER_BUILD_COMMAND")]
	pub build_command: Option<PathBuf>,
	/// An argument for the build command. Repeat it for more.
	#[arg(long = "build-arg")]
	pub build_arguments: Vec<String>,
	/// The name of a variable of the worker's own to give the build
	/// command, which otherwise gets `PATH` and nothing else of them.
	#[arg(long = "build-env")]
	pub build_environment: Vec<String>,
	/// Persistent cache directory shared across builds (`CARGO_HOME`,
	/// `SCCACHE_DIR`, and `BuildKit` local cache). Not deleted between jobs.
	#[arg(long, env = "LEPTOSKIT_CLOUD_WORKER_BUILD_CACHE_DIR")]
	pub build_cache_dir: Option<PathBuf>,
	/// Optional sandbox wrapper command prefix (repeatable, e.g.
	/// `--build-sandbox bwrap --build-sandbox --unshare-pid`).
	#[arg(long = "build-sandbox")]
	pub build_sandbox: Vec<String>,
	/// How long one build may run, in minutes.
	#[arg(long, default_value_t = 60)]
	pub build_timeout_minutes: u64,
	/// The program that publishes to app and extension stores: run once for
	/// each publish, in the unpacked source, with the job's store
	/// credentials. The worker doesn't isolate it; see the readme. Without
	/// it, a build worker takes no publish jobs: pair one with
	/// `--capability publish` to be offered them.
	#[arg(long, env = "LEPTOSKIT_CLOUD_WORKER_PUBLISH_COMMAND")]
	pub publish_command: Option<PathBuf>,
	/// An argument for the publish command. Repeat it for more.
	#[arg(long = "publish-arg")]
	pub publish_arguments: Vec<String>,
	/// The name of a variable of the worker's own to give the publish
	/// command, which otherwise gets `PATH`, the job's credentials, and
	/// nothing else of them.
	#[arg(long = "publish-env")]
	pub publish_environment: Vec<String>,
	/// How long one publish may run, in minutes.
	#[arg(long, default_value_t = 120)]
	pub publish_timeout_minutes: u64,
	/// For a cluster's worker: the kubeconfig file to reach the cluster
	/// with, while developing. Without it the worker uses the service
	/// account of the pod it runs in.
	#[arg(long, env = "LEPTOSKIT_CLOUD_WORKER_KUBECONFIG")]
	pub kubeconfig: Option<PathBuf>,
	/// For a cluster's worker: path to a JSON file containing a serialized
	/// `leptoskit_cloud_runtime::ClusterProfile` (for example mounted from
	/// the platform's `cluster-profile` `ConfigMap`). Defaults to
	/// `ClusterProfile::platform_defaults()` when omitted.
	#[arg(long, env = "LEPTOSKIT_CLOUD_WORKER_PROFILE_JSON")]
	pub profile_json: Option<PathBuf>,
	/// Overrides `ClusterProfile.image_pull_secret` (for external registries).
	#[arg(long, env = "LEPTOSKIT_CLOUD_WORKER_IMAGE_PULL_SECRET")]
	pub image_pull_secret: Option<String>,
	/// Overrides `ClusterProfile.cluster_issuer` (e.g. `apps-http01`).
	#[arg(long, env = "LEPTOSKIT_CLOUD_WORKER_CLUSTER_ISSUER")]
	pub cluster_issuer: Option<String>,
	/// Overrides `ClusterProfile.edge_terminated_custom_domains` (true under
	/// Cloudflare for `SaaS` / Tunnel edge TLS).
	#[arg(long, env = "LEPTOSKIT_CLOUD_WORKER_EDGE_TERMINATED_CUSTOM_DOMAINS")]
	pub edge_terminated_custom_domains: Option<bool>,
	/// Additional egress CIDRs to block in tenant `NetworkPolicy` manifests.
	#[arg(long = "blocked-egress-cidr")]
	pub blocked_egress_cidrs: Vec<String>,
	/// Overrides `ClusterProfile.tmp_size_limit_mib` (bounded `/tmp`
	/// `emptyDir`).
	#[arg(long, env = "LEPTOSKIT_CLOUD_WORKER_TMP_SIZE_LIMIT_MIB")]
	pub tmp_size_limit_mib: Option<u32>,
	/// For a cluster's worker: the shared Postgres server's administrator,
	/// as `postgres://…`. Without it, a release that needs a managed
	/// database fails. Give it in the environment: an argument is seen by
	/// anyone who lists the machine's processes.
	#[arg(
		long,
		env = "LEPTOSKIT_CLOUD_WORKER_POSTGRES_URL",
		hide_env_values = true
	)]
	pub postgres_url: Option<Hidden>,
	/// For a cluster's worker: a PEM file of certificates to trust for the
	/// Postgres server's, besides the webpki roots, when a private
	/// authority signed it.
	#[arg(long, env = "LEPTOSKIT_CLOUD_WORKER_POSTGRES_CA")]
	pub postgres_ca: Option<PathBuf>,
	/// For a cluster's worker: the namespace the Postgres server runs in,
	/// the only one a release's processes may reach a database in. Needed
	/// with `--postgres-url`.
	#[arg(long, env = "LEPTOSKIT_CLOUD_WORKER_POSTGRES_NAMESPACE")]
	pub postgres_namespace: Option<String>,
	/// For a cluster's worker: the Postgres server's port.
	#[arg(
		long,
		env = "LEPTOSKIT_CLOUD_WORKER_POSTGRES_PORT",
		default_value_t = 5432
	)]
	pub postgres_port: u16,
	/// Per-environment role `CONNECTION LIMIT` on the shared Postgres server.
	#[arg(
		long,
		env = "LEPTOSKIT_CLOUD_WORKER_POSTGRES_CONNECTION_LIMIT",
		default_value_t = crate::postgres::CONNECTION_LIMIT
	)]
	pub postgres_connection_limit: u32,
	/// Warn instead of failing when the Postgres admin role does not own
	/// system databases (`postgres`, `template1`) on managed Postgres.
	#[arg(
		long,
		env = "LEPTOSKIT_CLOUD_WORKER_POSTGRES_ALLOW_UNOWNED_SYSTEM_DBS",
		default_value_t = false
	)]
	pub postgres_allow_unowned_system_dbs: bool,
	/// For a cluster's worker: the `RuntimeClass` every release must run
	/// under, such as a sandboxed one. Without it a release may keep the
	/// cluster's default.
	#[arg(long, env = "LEPTOSKIT_CLOUD_WORKER_RUNTIME_CLASS")]
	pub runtime_class: Option<String>,
	/// For a cluster's worker: how long a release may take to become
	/// ready, in minutes, a day at most.
	#[arg(long, default_value_t = 10)]
	pub rollout_timeout_minutes: u64,
	/// Clears an unfinished cluster-job marker (`job.json` with no saved
	/// outcome) on startup with a warning instead of exiting with
	/// `Unreconciled`. Recommended in Kubernetes pods to avoid
	/// `CrashLoopBackOff` after pod preemption or restarts.
	#[arg(
		long,
		env = "LEPTOSKIT_CLOUD_WORKER_RECONCILE_ON_START",
		default_value_t = false
	)]
	pub reconcile_on_start: bool,
	/// Seconds between periodic `report_status` scrapes on a cluster worker.
	/// Set to `0` to disable periodic status reporting.
	#[arg(
		long,
		env = "LEPTOSKIT_CLOUD_WORKER_STATUS_INTERVAL_SECONDS",
		default_value_t = 30
	)]
	pub status_interval_seconds: u64,
}

/// A value that is never printed, such as an address with a password in
/// it.
#[derive(Clone)]
pub struct Hidden(String);

impl Hidden {
	/// The value itself.
	#[must_use]
	pub fn expose(&self) -> &str {
		&self.0
	}
}

impl std::fmt::Debug for Hidden {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter.write_str("[hidden]")
	}
}

impl std::str::FromStr for Hidden {
	type Err = std::convert::Infallible;

	fn from_str(value: &str) -> Result<Self, Self::Err> {
		Ok(Self(value.to_owned()))
	}
}

/// The longest a rollout may be given, in minutes: a day.
const MOST_ROLLOUT_MINUTES: u64 = 24 * 60;

#[derive(Debug, clap::Args)]
pub struct Forget {
	/// Where the worker keeps its key and its job's state.
	#[arg(long, env = "LEPTOSKIT_CLOUD_WORKER_STATE")]
	pub state: PathBuf,
}

/// Why a command failed, as it is printed.
#[derive(Debug, thiserror::Error)]
pub enum CommandError {
	#[error(transparent)]
	State(#[from] crate::state::StateError),
	#[error(transparent)]
	Origin(#[from] crate::client::OriginError),
	#[error("pairing failed: {0}")]
	Pair(crate::client::CallError),
	#[error(transparent)]
	Run(#[from] crate::run::RunError),
	#[error("{0}")]
	Usage(String),
}

/// Loads `ClusterProfile` from `--profile-json` when provided (falling back to
/// `ClusterProfile::platform_defaults()`) and applies explicit CLI/env
/// overrides on top.
///
/// # Errors
///
/// [`CommandError::Usage`] if the JSON file cannot be read or parsed.
pub fn load_cluster_profile(
	arguments: &Run,
) -> Result<leptoskit_cloud_runtime::ClusterProfile, CommandError> {
	let mut profile = match &arguments.profile_json {
		Some(path) => {
			let bytes = std::fs::read(path)
				.map_err(|error| CommandError::Usage(format!("{}: {error}", path.display())))?;
			serde_json::from_slice(&bytes).map_err(|error| {
				CommandError::Usage(format!(
					"invalid ClusterProfile JSON in {}: {error}",
					path.display()
				))
			})?
		}
		None => leptoskit_cloud_runtime::ClusterProfile::platform_defaults(),
	};

	if let Some(secret) = &arguments.image_pull_secret {
		profile.image_pull_secret = if secret.is_empty() {
			None
		} else {
			Some(secret.clone())
		};
	}
	if let Some(issuer) = &arguments.cluster_issuer {
		profile.cluster_issuer = if issuer.is_empty() {
			None
		} else {
			Some(issuer.clone())
		};
	}
	if let Some(edge_terminated) = arguments.edge_terminated_custom_domains {
		profile.edge_terminated_custom_domains = edge_terminated;
	}
	for cidr in &arguments.blocked_egress_cidrs {
		if !profile.blocked_egress_cidrs.contains(cidr) {
			profile.blocked_egress_cidrs.push(cidr.clone());
		}
	}
	if let Some(limit) = arguments.tmp_size_limit_mib {
		profile.tmp_size_limit_mib = limit.max(1);
	}

	Ok(profile)
}

async fn pair(arguments: Pair) -> Result<(), CommandError> {
	let state = State::open(&arguments.state)?;
	// Before the code is spent: it works once.
	state.ensure_unpaired()?;
	let control = Control::new(&arguments.url, &arguments.rpc, None)?;
	let paired = control
		.pair(
			&arguments.code,
			&arguments.name,
			arguments.capabilities.clone(),
		)
		.await
		.map_err(CommandError::Pair)?;
	state.pair(&Identity {
		origin: crate::client::origin(&arguments.url)?,
		rpc: arguments.rpc,
		key: paired.key.clone(),
		worker: paired.worker.id,
		name: paired.worker.name.clone(),
		role: paired.worker.role,
		capabilities: arguments.capabilities,
	})?;
	println!(
		"Paired as {} ({:?}). The key is kept in {}.",
		paired.worker.name,
		paired.worker.role,
		arguments.state.display()
	);
	Ok(())
}

async fn run(arguments: Run, shutdown: watch::Receiver<bool>) -> Result<(), CommandError> {
	if arguments.build_timeout_minutes == 0
		|| arguments.publish_timeout_minutes == 0
		|| arguments.rollout_timeout_minutes == 0
	{
		return Err(CommandError::Usage(
			"a timeout is a minute or more".to_owned(),
		));
	}

	if arguments.rollout_timeout_minutes > MOST_ROLLOUT_MINUTES {
		return Err(CommandError::Usage(format!(
			"a rollout may take {MOST_ROLLOUT_MINUTES} minutes at most"
		)));
	}

	let state = State::open(&arguments.state)?;
	let identity = state.identity()?;
	let control = Control::new(
		&identity.origin,
		&identity.rpc,
		Some(Key::new(identity.key)),
	)?;
	let workspace = arguments
		.workspace
		.clone()
		.unwrap_or_else(|| arguments.state.join("workspace"));
	let mut settings = Settings::new(workspace);
	settings.reconcile_on_start = arguments.reconcile_on_start;
	settings.status_every = (arguments.status_interval_seconds > 0)
		.then(|| Duration::from_secs(arguments.status_interval_seconds));

	let worker = match identity.role {
		WorkerRole::Build => {
			let (program, args) = if let Some(program) = arguments.build_command {
				(program, arguments.build_arguments)
			} else {
				let self_bin = std::env::current_exe().map_err(|error| {
					CommandError::Usage(format!(
						"couldn't locate current executable for built-in build-app: {error}"
					))
				})?;
				let mut args = vec!["build-app".to_owned()];
				args.extend(arguments.build_arguments);
				(self_bin, args)
			};
			let mut builder = CommandBuilder::new(program, args)
				.pass(arguments.build_environment)
				.sandbox(arguments.build_sandbox)
				.timeout(Duration::from_secs(
					arguments.build_timeout_minutes.saturating_mul(60),
				));
			if let Some(cache_dir) = arguments.build_cache_dir {
				builder = builder.cache_dir(cache_dir);
			}
			// Offered publishes, it must be able to run them: one it can't
			// would hold its organisation's credentials and then fail.
			if arguments.publish_command.is_none()
				&& identity
					.capabilities
					.iter()
					.any(|capability| capability == leptoskit_cloud::workers::PUBLISH_CAPABILITY)
			{
				return Err(CommandError::Usage(
					"this worker paired with the `publish` capability, so it needs \
					 --publish-command"
						.to_owned(),
				));
			}

			let worker = Worker::new(control, state, builder, settings)?;

			match arguments.publish_command {
				Some(program) => {
					worker.with_publisher(
						CommandPublisher::new(program, arguments.publish_arguments)
							.pass(arguments.publish_environment)
							.timeout(Duration::from_secs(
								arguments.publish_timeout_minutes.saturating_mul(60),
							)),
					)
				}
				None => worker,
			}
		}
		WorkerRole::Cluster => {
			let deadline =
				Duration::from_secs(arguments.rollout_timeout_minutes.saturating_mul(60));
			let profile = load_cluster_profile(&arguments)?;
			let place = match arguments.kubeconfig {
				Some(path) => Kubernetes::from_kubeconfig(path, profile, deadline),
				None => Kubernetes::in_cluster(profile, deadline),
			};
			let mut bounds = Bounds {
				runtime_class: arguments.runtime_class,
				database: None,
			};
			let mut databases = None;

			if let Some(address) = arguments.postgres_url {
				let Some(namespace) = arguments.postgres_namespace else {
					return Err(CommandError::Usage(
						"a database server needs --postgres-namespace".to_owned(),
					));
				};
				let mut postgres = Postgres::new(address.expose())
					.map_err(|error| CommandError::Usage(error.to_string()))?
					.connection_limit(arguments.postgres_connection_limit)
					.allow_unowned_system_databases(arguments.postgres_allow_unowned_system_dbs);

				if let Some(path) = arguments.postgres_ca {
					let pem = std::fs::read(&path).map_err(|error| {
						CommandError::Usage(format!("{}: {error}", path.display()))
					})?;
					postgres = postgres
						.trust(&pem)
						.map_err(|error| CommandError::Usage(error.to_string()))?;
				}

				bounds.database = Some(DatabaseAccess {
					namespace,
					port: arguments.postgres_port,
				});
				databases = Some(Arc::new(postgres));
			}

			let mut cluster = ClusterRole::new(Arc::new(place)).bounds(bounds);

			if let Some(postgres) = databases {
				cluster = cluster.databases(postgres);
			}

			// A cluster's worker is never given a build: this builder is
			// never run.
			let builder = CommandBuilder::new("false", Vec::new());
			Worker::new(control, state, builder, settings)?.with_cluster(cluster)
		}
	};
	eprintln!(
		"leptoskit-cloud-worker: {} is asking {} for {} jobs",
		identity.name,
		identity.origin,
		match identity.role {
			WorkerRole::Build => "build",
			WorkerRole::Cluster => "cluster",
		}
	);
	Ok(worker.run(shutdown).await?)
}

fn forget(arguments: &Forget) -> Result<(), CommandError> {
	let state = State::open(&arguments.state)?;

	match state.marker()? {
		Some(marker) => {
			state.clear()?;
			println!("Forgot job {} ({:?}).", marker.job, marker.kind);
		}
		None => println!("There is no job to forget."),
	}

	Ok(())
}

/// Runs `command`. `shutdown` turning true stops `run` after the job in
/// hand.
///
/// # Errors
///
/// [`CommandError`], to print.
pub async fn execute(
	command: Command,
	shutdown: watch::Receiver<bool>,
) -> Result<(), CommandError> {
	match command {
		Command::Pair(arguments) => pair(arguments).await,
		Command::Run(arguments) => run(*arguments, shutdown).await,
		Command::Forget(arguments) => forget(&arguments),
		Command::BuildApp(arguments) => {
			crate::builder::run(arguments)
				.await
				.map_err(CommandError::Usage)
		}
	}
}

#[cfg(test)]
mod tests {
	use clap::Parser as _;

	use super::*;

	#[test]
	fn loads_cluster_profile_from_json_and_applies_overrides() {
		let dir = tempfile::tempdir().expect("tempdir");
		let path = dir.path().join("profile.json");
		let mut base = leptoskit_cloud_runtime::ClusterProfile::platform_defaults();
		base.image_pull_secret = Some("ghcr-pull".to_owned());
		base.edge_terminated_custom_domains = true;
		std::fs::write(&path, serde_json::to_vec(&base).expect("json")).expect("write");

		let Command::Run(args) = Command::parse_from([
			"leptoskit-cloud-worker",
			"run",
			"--state",
			"/tmp/state",
			"--profile-json",
			path.to_str().expect("utf-8"),
			"--blocked-egress-cidr",
			"10.42.0.0/16",
			"--tmp-size-limit-mib",
			"256",
		]) else {
			panic!("expected Run");
		};
		let loaded = load_cluster_profile(&args).expect("loads");
		assert_eq!(loaded.image_pull_secret.as_deref(), Some("ghcr-pull"));
		assert!(loaded.edge_terminated_custom_domains);
		assert!(
			loaded
				.blocked_egress_cidrs
				.contains(&"10.42.0.0/16".to_owned())
		);
		assert_eq!(loaded.tmp_size_limit_mib, 256);
	}
}
