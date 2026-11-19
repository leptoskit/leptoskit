//! `leptoskit-cloud-worker`'s commands.

use std::path::PathBuf;
use std::time::Duration;

use leptoskit_cloud::workers::WorkerRole;
use tokio::sync::watch;

use crate::client::Control;
use crate::client::Key;
use crate::command::CommandBuilder;
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
	/// source. The worker doesn't isolate it; see the readme.
	#[arg(long, env = "LEPTOSKIT_CLOUD_WORKER_BUILD_COMMAND")]
	pub build_command: PathBuf,
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
}

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
	#[error("this worker is paired as a cluster worker, and the cluster role isn't built yet")]
	ClusterRole,
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
	if arguments.build_timeout_minutes == 0 {
		return Err(CommandError::Usage(
			"--build-timeout-minutes is a minute or more".to_owned(),
		));
	}

	let state = State::open(&arguments.state)?;
	let identity = state.identity()?;

	if identity.role != WorkerRole::Build {
		return Err(CommandError::ClusterRole);
	}

	let control = Control::new(
		&identity.origin,
		&identity.rpc,
		Some(Key::new(identity.key)),
	)?;
	let builder = CommandBuilder::new(arguments.build_command, arguments.build_arguments)
		.pass(arguments.build_environment)
		.timeout(Duration::from_secs(
			arguments.build_timeout_minutes.saturating_mul(60),
		));
	let workspace = arguments
		.workspace
		.unwrap_or_else(|| arguments.state.join("workspace"));
	let worker = Worker::new(control, state, builder, Settings::new(workspace))?;
	eprintln!(
		"leptoskit-cloud-worker: {} is asking {} for builds",
		identity.name, identity.origin
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
