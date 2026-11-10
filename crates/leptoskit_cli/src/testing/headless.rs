//! `leptoskit test headless`: runs the built guest `.wasm` without a device
//! or window, for scripted input, tree dumps, and snapshots.
//!
//! Every command loads the guest that `leptoskit native bundle` writes, runs
//! an optional script to reach the screen of interest, then reports. Set
//! `LEPTOSKIT_FRAME_LOG` to `stderr` or a file to log each commit's costs.

use std::fs;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use clap::Args;
use clap::Subcommand;
use leptoskit_testing::headless::HeadlessApp;
use leptoskit_testing::headless::Script;
use leptoskit_testing::headless::SnapshotFormat;

/// `leptoskit test headless` subcommands.
#[derive(Debug, Subcommand)]
pub enum HeadlessCommand {
	/// Runs a JSON script of presses, typing, waits, and assertions, and
	/// reports each step. Fails when a step fails.
	Run {
		/// The script.
		script: PathBuf,
		#[command(flatten)]
		guest: GuestArgs,
		/// Print the report as JSON.
		#[arg(long)]
		json: bool,
	},
	/// Prints the committed widget tree: kinds, names, roles, key props, and
	/// frames.
	Tree {
		#[command(flatten)]
		guest: GuestArgs,
		/// A script to run first, to reach the screen to dump.
		#[arg(long)]
		script: Option<PathBuf>,
		/// Print JSON instead of indented text.
		#[arg(long)]
		json: bool,
	},
	/// Writes an SVG or PNG of the laid-out tree, chosen by the extension.
	Snapshot {
		/// The `.svg` or `.png` file to write.
		out: PathBuf,
		#[command(flatten)]
		guest: GuestArgs,
		/// A script to run first, to reach the screen to draw.
		#[arg(long)]
		script: Option<PathBuf>,
		/// Pixels per layout unit, for PNG.
		#[arg(long, default_value_t = 2.0)]
		scale: f32,
	},
}

/// Which guest to load, at what size.
#[derive(Debug, Args)]
pub struct GuestArgs {
	/// The guest module; `leptoskit native bundle` writes `dist/app.wasm`.
	#[arg(long, default_value = "dist/app.wasm")]
	wasm: PathBuf,
	/// Viewport width, replacing the script's.
	#[arg(long)]
	width: Option<f32>,
	/// Viewport height, replacing the script's.
	#[arg(long)]
	height: Option<f32>,
	/// Resolve `dark:` utilities.
	#[arg(long)]
	dark: bool,
}

/// Runs a headless command.
pub fn run(command: HeadlessCommand) -> anyhow::Result<()> {
	match command {
		HeadlessCommand::Run {
			script,
			guest,
			json,
		} => {
			let script = read_script(&script)?;
			let report = HeadlessApp::run_script(&read_wasm(&guest.wasm)?, &guest.apply(script))?;

			if json {
				println!("{}", serde_json::to_string_pretty(&report)?);
			} else {
				print!("{}", report.summary());
			}

			anyhow::ensure!(report.passed, "the headless script failed");
			Ok(())
		}
		HeadlessCommand::Tree {
			guest,
			script,
			json,
		} => {
			let app = reach(&guest, script.as_deref())?;

			if json {
				println!("{}", serde_json::to_string_pretty(&app.tree_json())?);
			} else {
				print!("{}", app.tree());
			}

			Ok(())
		}
		HeadlessCommand::Snapshot {
			out,
			guest,
			script,
			scale,
		} => {
			let format = SnapshotFormat::for_path(&out, scale).map_err(anyhow::Error::msg)?;
			let app = reach(&guest, script.as_deref())?;
			app.snapshot(format)?
				.save(&out)
				.with_context(|| format!("writing {}", out.display()))?;
			crate::ui::success(format_args!("snapshot: {}", out.display()));
			Ok(())
		}
	}
}

impl GuestArgs {
	/// The script with this command's viewport and appearance applied.
	fn apply(&self, mut script: Script) -> Script {
		let (width, height) = script.start_viewport();
		script.viewport = Some(leptoskit_testing::headless::Viewport {
			width: self.width.unwrap_or(width),
			height: self.height.unwrap_or(height),
		});
		script.dark |= self.dark;
		script
	}
}

/// Loads the guest and runs a script, if one is given, failing when it does.
fn reach(guest: &GuestArgs, script: Option<&Path>) -> anyhow::Result<HeadlessApp> {
	let script = match script {
		Some(path) => read_script(path)?,
		None => Script::default(),
	};
	let script = guest.apply(script);
	let (width, height) = script.start_viewport();
	let mut app = HeadlessApp::start(&read_wasm(&guest.wasm)?, width, height)?;

	if script.dark {
		app.set_dark_mode(true)?;
	}

	let report = app.run_steps(&script.steps);

	if !report.passed {
		eprint!("{}", report.summary());
		anyhow::bail!("the headless script failed");
	}

	Ok(app)
}

fn read_script(path: &Path) -> anyhow::Result<Script> {
	let json = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
	Script::from_json(&json).with_context(|| format!("parsing {}", path.display()))
}

fn read_wasm(path: &Path) -> anyhow::Result<Vec<u8>> {
	fs::read(path).with_context(|| {
		format!(
			"reading {}; build the guest with `leptoskit native bundle`, or pass --wasm",
			path.display()
		)
	})
}

#[cfg(test)]
mod tests {
	use clap::Parser;

	use super::*;

	#[derive(Parser)]
	struct Cli {
		#[command(subcommand)]
		command: HeadlessCommand,
	}

	#[test]
	fn arguments_override_the_script_viewport_and_appearance() {
		let Cli {
			command: HeadlessCommand::Tree { guest, .. },
		} = Cli::parse_from(["headless", "tree", "--width", "800", "--dark"])
		else {
			panic!("parsed a tree command");
		};
		assert_eq!(guest.wasm, Path::new("dist/app.wasm"));
		let script = guest.apply(Script::from_json(r#"{"steps": []}"#).unwrap());
		assert_eq!(script.start_viewport(), (800.0, 844.0));
		assert!(script.dark);
	}

	#[test]
	fn a_missing_guest_says_how_to_build_one() {
		let error = read_wasm(Path::new("/nonexistent/app.wasm")).unwrap_err();
		assert!(
			error.to_string().contains("leptoskit native bundle"),
			"{error}"
		);
	}
}
