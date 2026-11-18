use clap::Parser as _;
use leptoskit_cloud_platform::cli::Cli;

fn main() -> std::process::ExitCode {
	match Cli::parse().run() {
		Ok(output) => {
			print!("{output}");
			std::process::ExitCode::SUCCESS
		}
		Err(error) => {
			eprintln!("{error}");
			std::process::ExitCode::FAILURE
		}
	}
}
