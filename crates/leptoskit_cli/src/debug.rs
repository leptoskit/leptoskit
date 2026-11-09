//! `leptoskit debug`: the running tree, its errors, and its commits.
//!
//! The same text the MCP tools return, printed for a person at a terminal.
//! The hub page is the inspector; `debug open` prints its address.

use std::path::Path;

use anyhow::Result;
use clap::Subcommand;

use crate::mcp::devtools::Report;
use crate::mcp::devtools::report;

/// What to print. Absent means the element tree.
#[derive(Debug, Subcommand)]
pub enum DebugCommand {
	/// Prints the element tree: kind, text, classes, frame, and source line.
	Tree {
		/// A session id, a platform, or the app name.
		#[arg(long)]
		session: Option<String>,
		/// How many levels to print.
		#[arg(long)]
		depth: Option<u64>,
	},
	/// Prints failed plugin calls and server attempts.
	Errors {
		/// A session id, a platform, or the app name.
		#[arg(long)]
		session: Option<String>,
	},
	/// Prints render commits and, when the app was profiling, layout and draw
	/// time.
	Performance {
		/// A session id, a platform, or the app name.
		#[arg(long)]
		session: Option<String>,
	},
	/// Prints the hub address. Open it to use the inspector.
	Open,
}

/// Prints one report from the hub in the working directory.
pub fn run(command: Option<DebugCommand>) -> Result<()> {
	emit(&report_here(command)?);
	Ok(())
}

fn report_here(command: Option<DebugCommand>) -> Result<String> {
	let start = std::env::current_dir()?;
	report_at(&start, command)
}

/// The report `command` would print, reading the hub from `start`.
pub fn report_at(start: &Path, command: Option<DebugCommand>) -> Result<String> {
	let kind = match command {
		None => {
			Report::Tree {
				session: None,
				depth: None,
			}
		}
		Some(DebugCommand::Tree { session, depth }) => Report::Tree { session, depth },
		Some(DebugCommand::Errors { session }) => Report::Errors { session },
		Some(DebugCommand::Performance { session }) => Report::Performance { session },
		Some(DebugCommand::Open) => Report::Open,
	};
	report(start, kind).map_err(anyhow::Error::msg)
}

fn emit(text: &str) {
	println!("{text}");
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_directory_without_a_hub_is_an_error_and_a_report_can_be_printed() {
		let directory =
			std::env::temp_dir().join(format!("leptoskit-debug-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&directory);
		std::fs::create_dir_all(&directory).unwrap();
		let error = report_at(&directory, None).unwrap_err();
		assert!(error.to_string().contains("no hub is running"));
		let error =
			report_at(&directory, Some(DebugCommand::Errors { session: None })).unwrap_err();
		assert!(error.to_string().contains("no hub is running"));
		let error = report_at(
			&directory,
			Some(DebugCommand::Performance { session: None }),
		)
		.unwrap_err();
		assert!(error.to_string().contains("no hub is running"));
		let error = report_at(&directory, Some(DebugCommand::Open)).unwrap_err();
		assert!(error.to_string().contains("no hub is running"));
		let error = report_at(
			&directory,
			Some(DebugCommand::Tree {
				session: Some("web".to_owned()),
				depth: Some(2),
			}),
		)
		.unwrap_err();
		assert!(error.to_string().contains("no hub is running"));
		let _ = run(None);
		emit("tree");
		let _ = std::fs::remove_dir_all(&directory);
	}
}
