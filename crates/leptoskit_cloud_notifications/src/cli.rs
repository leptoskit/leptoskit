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
/// operator issued for the cluster's `notifications` role, or give it the
/// code with `--url` and `--name`, and it pairs itself when its state isn't
/// paired yet.
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
	/// The control plane's address, such as `https://cloud.example.com`.
	/// Used only to pair.
	#[arg(long, env = "LEPTOSKIT_CLOUD_NOTIFICATIONS_URL")]
	pub url: Option<String>,
	/// What operators call this hub, such as `eu-one-notifications`. Used
	/// only to pair.
	#[arg(long, env = "LEPTOSKIT_CLOUD_NOTIFICATIONS_NAME")]
	pub name: Option<String>,
	/// A pairing code, `lkp_…`, for the cluster's `notifications` role. It
	/// is used only while the state isn't paired, so it can stay set after
	/// it is spent. Give it in the environment: an argument is seen by
	/// anyone who lists the machine's processes.
	#[arg(
		long,
		env = "LEPTOSKIT_CLOUD_NOTIFICATIONS_PAIRING_CODE",
		hide_env_values = true
	)]
	pub pairing_code: Option<String>,
}

/// Why the hub stopped.
#[derive(Debug, thiserror::Error)]
pub enum CommandError {
	#[error("{0}")]
	Usage(String),
	#[error(transparent)]
	State(#[from] leptoskit_cloud_worker::StateError),
	#[error(transparent)]
	Pair(#[from] leptoskit_cloud_worker::cli::CommandError),
	#[error(transparent)]
	Origin(#[from] leptoskit_cloud_worker::OriginError),
	#[error("{0}")]
	Serve(#[from] std::io::Error),
}

/// Pairs the state with the pairing code when one is given and the state
/// isn't paired yet. A paired state leaves the code unused.
async fn pair_unless_paired(arguments: &Run) -> Result<(), CommandError> {
	let Some(code) = arguments
		.pairing_code
		.as_deref()
		.filter(|code| !code.is_empty())
	else {
		return Ok(());
	};

	// The state is locked while it is open, and `pair` opens it again.
	let paired = match State::open(&arguments.state)?.identity() {
		Ok(_) => true,
		Err(leptoskit_cloud_worker::StateError::NotPaired(_)) => false,
		Err(error) => return Err(error.into()),
	};

	if paired {
		return Ok(());
	}

	let (Some(url), Some(name)) = (&arguments.url, &arguments.name) else {
		return Err(CommandError::Usage(
			"a pairing code needs --url (the control plane's address) and --name".to_owned(),
		));
	};

	leptoskit_cloud_worker::cli::pair(leptoskit_cloud_worker::cli::Pair {
		url: url.clone(),
		rpc: "/rpc".to_owned(),
		code: code.to_owned(),
		name: name.clone(),
		capabilities: Vec::new(),
		state: arguments.state.clone(),
		role: Some(WorkerRole::Notifications),
	})
	.await?;
	Ok(())
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

	pair_unless_paired(&arguments).await?;
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

#[cfg(test)]
mod tests {
	use clap::Parser as _;
	use leptoskit_cloud_worker::Identity;

	use super::*;

	fn arguments(state: &std::path::Path, extra: &[&str]) -> Run {
		let mut line = vec![
			"leptoskit-cloud-notifications",
			"--state",
			state.to_str().unwrap(),
		];
		line.extend_from_slice(extra);
		Run::try_parse_from(line).unwrap()
	}

	#[tokio::test]
	async fn a_pairing_code_is_used_only_while_the_state_is_unpaired() {
		let directory = tempfile::tempdir().unwrap();

		// Without a code, nothing is paired, and the hub then says so.
		pair_unless_paired(&arguments(directory.path(), &[]))
			.await
			.unwrap();
		assert!(matches!(
			State::open(directory.path()).unwrap().identity(),
			Err(leptoskit_cloud_worker::StateError::NotPaired(_))
		));

		// A code needs to know where, and as whom, before it is spent.
		let error = pair_unless_paired(&arguments(
			directory.path(),
			&["--pairing-code", "lkp_unused"],
		))
		.await
		.unwrap_err();
		assert!(
			matches!(&error, CommandError::Usage(message) if message.contains("--url")),
			"{error}"
		);

		// Once paired, the code is left unused, so it may stay set: no
		// control plane is asked, though this one doesn't exist.
		State::open(directory.path())
			.unwrap()
			.pair(&Identity {
				origin: "https://cloud.example.com".to_owned(),
				rpc: "/rpc".to_owned(),
				key: "lkw_example".to_owned(),
				worker: uuid::Uuid::nil(),
				name: "eu-one-notifications".to_owned(),
				role: WorkerRole::Notifications,
				capabilities: Vec::new(),
			})
			.unwrap();
		pair_unless_paired(&arguments(
			directory.path(),
			&[
				"--pairing-code",
				"lkp_spent",
				"--url",
				"http://127.0.0.1:9",
				"--name",
				"eu-one-notifications",
			],
		))
		.await
		.unwrap();
	}
}
