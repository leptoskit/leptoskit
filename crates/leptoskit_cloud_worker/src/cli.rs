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
	Run(Run),
	/// Forgets a job that was started and never reported, after an
	/// operator has reconciled it.
	Forget(Forget),
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
	/// source. The worker doesn't isolate it; see the readme. A build
	/// worker needs one.
	#[arg(long, env = "LEPTOSKIT_CLOUD_WORKER_BUILD_COMMAND")]
	pub build_command: Option<PathBuf>,
	/// An argument for the build command. Repeat it for more.
	#[arg(long = "build-arg")]
	pub build_arguments: Vec<String>,
	/// The name of a variable of the worker's own to give the build
	/// command, which otherwise gets `PATH` and nothing else of them.
	#[arg(long = "build-env")]
	pub build_environment: Vec<String>,
	/// How long one build may run, in minutes.
	#[arg(long, default_value_t = 60)]
	pub build_timeout_minutes: u64,
	/// For a cluster's worker: the kubeconfig file to reach the cluster
	/// with, while developing. Without it the worker uses the service
	/// account of the pod it runs in.
	#[arg(long, env = "LEPTOSKIT_CLOUD_WORKER_KUBECONFIG")]
	pub kubeconfig: Option<PathBuf>,
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
	/// For a cluster's worker: the `RuntimeClass` every release must run
	/// under, such as a sandboxed one. Without it a release may keep the
	/// cluster's default.
	#[arg(long, env = "LEPTOSKIT_CLOUD_WORKER_RUNTIME_CLASS")]
	pub runtime_class: Option<String>,
	/// For a cluster's worker: how long a release may take to become
	/// ready, in minutes, a day at most.
	#[arg(long, default_value_t = 10)]
	pub rollout_timeout_minutes: u64,
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

async fn pair(arguments: Pair) -> Result<(), CommandError> {
	let state = State::open(&arguments.state)?;
	// Before the code is spent: it works once.
	state.ensure_unpaired()?;
	let control = Control::new(&arguments.url, &arguments.rpc, None)?;
	let paired = control
		.pair(&arguments.code, &arguments.name, arguments.capabilities)
		.await
		.map_err(CommandError::Pair)?;
	state.pair(&Identity {
		origin: crate::client::origin(&arguments.url)?,
		rpc: arguments.rpc,
		key: paired.key.clone(),
		worker: paired.worker.id,
		name: paired.worker.name.clone(),
		role: paired.worker.role,
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
	if arguments.build_timeout_minutes == 0 || arguments.rollout_timeout_minutes == 0 {
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
		.unwrap_or_else(|| arguments.state.join("workspace"));

	let worker = match identity.role {
		WorkerRole::Build => {
			let Some(program) = arguments.build_command else {
				return Err(CommandError::Usage(
					"a build worker needs --build-command".to_owned(),
				));
			};
			let builder = CommandBuilder::new(program, arguments.build_arguments)
				.pass(arguments.build_environment)
				.timeout(Duration::from_secs(
					arguments.build_timeout_minutes.saturating_mul(60),
				));
			Worker::new(control, state, builder, Settings::new(workspace))?
		}
		WorkerRole::Cluster => {
			let deadline =
				Duration::from_secs(arguments.rollout_timeout_minutes.saturating_mul(60));
			let profile = leptoskit_cloud_runtime::ClusterProfile::platform_defaults();
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
					.map_err(|error| CommandError::Usage(error.to_string()))?;

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
			Worker::new(control, state, builder, Settings::new(workspace))?.with_cluster(cluster)
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
		Command::Run(arguments) => run(arguments, shutdown).await,
		Command::Forget(arguments) => forget(&arguments),
	}
}
