//! `leptoskit-cloud-notifications`.

use std::process::ExitCode;

#[cfg(unix)]
use clap::Parser as _;
#[cfg(unix)]
use leptoskit_cloud_notifications::cli::Run;

/// Turns the first interrupt or termination signal into a request to stop,
/// and ends the process at the second.
#[cfg(unix)]
async fn signals(shutdown: tokio::sync::watch::Sender<bool>) {
	let mut asked = false;

	loop {
		let Ok(mut terminate) =
			tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
		else {
			return;
		};
		tokio::select! {
			_ = tokio::signal::ctrl_c() => {}
			_ = terminate.recv() => {}
		}

		if asked {
			eprintln!("leptoskit-cloud-notifications: stopping now");
			std::process::exit(130);
		}

		asked = true;
		eprintln!("leptoskit-cloud-notifications: stopping; signal again to stop now");
		let _ = shutdown.send(true);
	}
}

#[cfg(unix)]
#[tokio::main]
async fn main() -> ExitCode {
	let arguments = Run::parse();
	let (sender, shutdown) = tokio::sync::watch::channel(false);
	tokio::spawn(signals(sender));

	match leptoskit_cloud_notifications::cli::run(arguments, shutdown).await {
		Ok(()) => ExitCode::SUCCESS,
		Err(error) => {
			eprintln!("leptoskit-cloud-notifications: {error}");
			ExitCode::FAILURE
		}
	}
}

/// The hub shares the worker's paired state, which it locks as the worker
/// does, on Linux and macOS only.
#[cfg(not(unix))]
fn main() -> ExitCode {
	eprintln!("leptoskit-cloud-notifications: the hub runs on Linux and macOS");
	ExitCode::FAILURE
}
