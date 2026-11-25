//! `leptoskit-cloud-notifications`'s command line.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use leptoskit_cloud::workers::WorkerRole;
use leptoskit_cloud_worker::Control;
use leptoskit_cloud_worker::Key;
use leptoskit_cloud_worker::State;
use tokio::sync::watch;

use crate::Connect;
use crate::Hub;

/// Serves the hosted notifications of every environment on one cluster.
///
/// Pair it first, once, with `leptoskit-cloud-worker pair` and a code an
/// operator issued for the cluster's `notifications` role.
#[derive(Debug, clap::Parser)]
#[command(name = "leptoskit-cloud-notifications", version, about)]
pub struct Run {
	/// Where the paired worker keeps its key.
	#[arg(long, env = "LEPTOSKIT_CLOUD_NOTIFICATIONS_STATE")]
	pub state: PathBuf,
	/// The address the hub serves on: the cluster's edge and its apps
	/// reach it here.
	#[arg(
		long,
		env = "LEPTOSKIT_CLOUD_NOTIFICATIONS_LISTEN",
		default_value = "0.0.0.0:8080"
	)]
	pub listen: SocketAddr,
	/// How often it asks the control plane what changed and reports, in
	/// seconds.
	#[arg(
		long,
		env = "LEPTOSKIT_CLOUD_NOTIFICATIONS_EVERY_SECONDS",
		default_value_t = 30
	)]
	pub every_seconds: u64,
	/// The most connections it holds to each tenant's database. The
	/// cluster's worker lets each tenant's role hold 20.
	#[arg(
		long,
		env = "LEPTOSKIT_CLOUD_NOTIFICATIONS_CONNECTIONS",
		default_value_t = 4
	)]
	pub connections: usize,
}

/// Why the hub stopped.
#[derive(Debug, thiserror::Error)]
pub enum CommandError {
	#[error("{0}")]
	Usage(String),
	#[error(transparent)]
	State(#[from] leptoskit_cloud_worker::StateError),
	#[error(transparent)]
	Origin(#[from] leptoskit_cloud_worker::OriginError),
	#[error("{0}")]
	Serve(#[from] std::io::Error),
}

/// Runs the hub until `shutdown` turns true.
///
/// # Errors
///
/// [`CommandError`], to print.
pub async fn run(arguments: Run, mut shutdown: watch::Receiver<bool>) -> Result<(), CommandError> {
	if !(1..=86_400).contains(&arguments.every_seconds) {
		return Err(CommandError::Usage(
			"--every-seconds is between 1 and 86400".to_owned(),
		));
	}

	if !(1..=20).contains(&arguments.connections) {
		return Err(CommandError::Usage(
			"--connections is between 1 and 20, the most a tenant's role may hold".to_owned(),
		));
	}

	let state = State::open(&arguments.state)?;
	let identity = state.identity()?;

	if identity.role != WorkerRole::Notifications {
		return Err(CommandError::Usage(format!(
			"this worker is paired as {:?}, not as a notifications hub",
			identity.role
		)));
	}

	let control = Control::new(
		&identity.origin,
		&identity.rpc,
		Some(Key::new(identity.key)),
	)?;
	let hub = Hub::new(Arc::new(Connect {
		connections: arguments.connections,
	}));
	let listener = tokio::net::TcpListener::bind(arguments.listen).await?;
	eprintln!(
		"leptoskit-cloud-notifications: {} serves on {} and asks {} every {}s",
		identity.name, arguments.listen, identity.origin, arguments.every_seconds
	);

	let router = hub.router();
	let mut serving = shutdown.clone();
	let server = tokio::spawn(async move {
		leptoskit_axum::axum::serve(listener, router)
			.with_graceful_shutdown(async move {
				let _ = serving.wait_for(|stop| *stop).await;
			})
			.await
	});

	let every = Duration::from_secs(arguments.every_seconds);

	loop {
		let round = hub.round(&control, leptoskit_db::now()).await;

		for (tenant, reason) in &round.failed {
			eprintln!("leptoskit-cloud-notifications: tenant {tenant} can't be served: {reason}");
		}

		if !round.opened.is_empty() || !round.removed.is_empty() {
			eprintln!(
				"leptoskit-cloud-notifications: opened {}, removed {}, serving {}",
				round.opened.len(),
				round.removed.len(),
				hub.serving().len()
			);
		}

		if let Some(reason) = &round.unreachable {
			eprintln!("leptoskit-cloud-notifications: the control plane: {reason}");
		}

		tokio::select! {
			() = tokio::time::sleep(every) => {}
			_ = shutdown.wait_for(|stop| *stop) => break,
		}
	}

	match server.await {
		Ok(result) => result.map_err(CommandError::Serve),
		Err(error) => Err(CommandError::Usage(format!("the server stopped: {error}"))),
	}
}
