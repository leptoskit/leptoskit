//! `leptoskit-cloud-control`: see the crate's readme.

use std::process::ExitCode;

use clap::Parser as _;
use leptoskit_cloud_app::server::Command;

#[tokio::main]
async fn main() -> ExitCode {
	let command = Command::parse();

	match leptoskit_cloud_app::server::run(command, &|name| std::env::var(name).ok()).await {
		Ok(()) => ExitCode::SUCCESS,
		Err(error) => {
			eprintln!("leptoskit-cloud-control: {error}");
			ExitCode::FAILURE
		}
	}
}
