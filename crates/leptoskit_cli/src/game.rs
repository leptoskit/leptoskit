//! `leptoskit game`: build and run a game crate.
//!
//! Gameplay stays in the crate's Rust. The CLI does not compile an interface
//! language; it builds the crate, which commits leptoskit components through
//! the game host.

use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use anyhow::Context as _;
use anyhow::Result;
use anyhow::bail;

/// `leptoskit game` subcommands.
#[derive(Debug, clap::Subcommand)]
pub enum GameCommand {
	/// Builds the game crate.
	Build {
		/// The game directory. Defaults to `game/lantern` when that crate is
		/// here, otherwise the current directory.
		path: Option<PathBuf>,
		/// Build in release mode.
		#[arg(long)]
		release: bool,
	},
	/// Runs the game and writes the last frame.
	Run {
		/// The game directory. Defaults to `game/lantern` when that crate is
		/// here, otherwise the current directory.
		path: Option<PathBuf>,
		/// Build in release mode.
		#[arg(long)]
		release: bool,
		/// Fixed steps to run. Each step is one frame.
		#[arg(long)]
		frames: Option<u32>,
		/// Where to write the last frame.
		#[arg(long)]
		out: Option<PathBuf>,
		/// An action held for every tick. Repeat for several.
		#[arg(long)]
		hold: Vec<String>,
		/// Press the control with this label halfway through the run.
		#[arg(long)]
		tap: Option<String>,
		/// Frame width in pixels.
		#[arg(long)]
		width: Option<u32>,
		/// Frame height in pixels.
		#[arg(long)]
		height: Option<u32>,
		/// `auto` (default), `gpu`, or `cpu`.
		#[arg(long)]
		painter: Option<String>,
	},
}

/// A cargo invocation that builds or runs a game.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Plan {
	/// `cargo`, or another program in tests.
	pub program: String,
	/// Arguments, including the `--` that separates cargo from the game.
	pub args: Vec<String>,
	/// The directory cargo runs in.
	pub cwd: PathBuf,
}

/// Builds or runs the game named by `command`.
pub fn run(command: GameCommand) -> Result<()> {
	let plan = plan(command, &std::env::current_dir()?)?;
	execute(&plan)
}

pub(crate) fn plan(command: GameCommand, start: &Path) -> Result<Plan> {
	let (path, release, action, forwarded) = match command {
		GameCommand::Build { path, release } => (path, release, "build", Vec::new()),
		GameCommand::Run {
			path,
			release,
			frames,
			out,
			hold,
			tap,
			width,
			height,
			painter,
		} => {
			let mut forwarded = Vec::new();
			if let Some(frames) = frames {
				forwarded.push("--frames".to_owned());
				forwarded.push(frames.to_string());
			}
			if let Some(out) = out {
				forwarded.push("--out".to_owned());
				forwarded.push(out.display().to_string());
			}
			for action in hold {
				forwarded.push("--hold".to_owned());
				forwarded.push(action);
			}
			if let Some(tap) = tap {
				forwarded.push("--tap".to_owned());
				forwarded.push(tap);
			}
			if let Some(width) = width {
				forwarded.push("--width".to_owned());
				forwarded.push(width.to_string());
			}
			if let Some(height) = height {
				forwarded.push("--height".to_owned());
				forwarded.push(height.to_string());
			}
			if let Some(painter) = painter {
				forwarded.push("--painter".to_owned());
				forwarded.push(painter);
			}
			(path, release, "run", forwarded)
		}
	};
	let game = resolve_game_dir(start, path.as_deref())?;
	let manifest = game.join("Cargo.toml");
	let name = package_name(&manifest)?;
	let workspace = workspace_root(&game);
	let mut args = vec![action.to_owned()];
	if release {
		args.push("--release".to_owned());
	}
	let cwd = if let Some(root) = workspace {
		args.push("-p".to_owned());
		args.push(name);
		args.push("--manifest-path".to_owned());
		args.push(root.join("Cargo.toml").display().to_string());
		root
	} else {
		args.push("--manifest-path".to_owned());
		args.push(manifest.display().to_string());
		game
	};
	if !forwarded.is_empty() {
		args.push("--".to_owned());
		args.extend(forwarded);
	}
	Ok(Plan {
		program: "cargo".to_owned(),
		args,
		cwd,
	})
}

pub(crate) fn execute(plan: &Plan) -> Result<()> {
	let status = Command::new(&plan.program)
		.args(&plan.args)
		.current_dir(&plan.cwd)
		.status()
		.with_context(|| format!("failed to run {} {}", plan.program, plan.args.join(" ")))?;
	if status.success() {
		Ok(())
	} else {
		bail!("{status}: {} {}", plan.program, plan.args.join(" "))
	}
}

fn resolve_game_dir(start: &Path, explicit: Option<&Path>) -> Result<PathBuf> {
	let dir = match explicit {
		Some(path) if path.is_absolute() => path.to_path_buf(),
		Some(path) => start.join(path),
		None if start.join("game/lantern/Cargo.toml").is_file() => start.join("game/lantern"),
		None if start.join("Cargo.toml").is_file() => start.to_path_buf(),
		None => bail!("no game crate here; pass its directory, or run from game/lantern"),
	};
	if dir.join("Cargo.toml").is_file() {
		Ok(dir)
	} else {
		bail!("no Cargo.toml in {}", dir.display())
	}
}

fn package_name(manifest: &Path) -> Result<String> {
	let text = std::fs::read_to_string(manifest)
		.with_context(|| format!("reading {}", manifest.display()))?;
	let mut in_package = false;
	for line in text.lines() {
		let line = line.trim();
		if line.starts_with('[') {
			in_package = line == "[package]";
			continue;
		}
		if in_package && let Some(rest) = line.strip_prefix("name") {
			let name = rest.trim().trim_start_matches('=').trim().trim_matches('"');
			if !name.is_empty() && !name.contains(' ') {
				return Ok(name.to_owned());
			}
		}
	}
	bail!("{} has no package name", manifest.display())
}

fn workspace_root(start: &Path) -> Option<PathBuf> {
	let mut dir = start.to_path_buf();
	loop {
		let manifest = dir.join("Cargo.toml");
		if manifest.is_file()
			&& std::fs::read_to_string(&manifest).is_ok_and(|text| text.contains("[workspace]"))
		{
			return Some(dir);
		}
		if !dir.pop() {
			return None;
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn start() -> PathBuf {
		PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
	}

	#[test]
	fn the_sample_plans_a_workspace_run() {
		let plan = plan(
			GameCommand::Run {
				path: None,
				release: false,
				frames: Some(4),
				out: Some(PathBuf::from("artifacts/frame.png")),
				hold: vec!["move".to_owned()],
				tap: None,
				width: None,
				height: None,
				painter: Some("cpu".to_owned()),
			},
			&start(),
		)
		.unwrap();
		assert!(plan.args.iter().any(|arg| arg == "-p"));
		assert!(plan.args.iter().any(|arg| arg == "lantern"));
		assert!(plan.args.windows(2).any(|pair| pair == ["--frames", "4"]));
		assert!(
			plan.args
				.windows(2)
				.any(|pair| pair == ["--painter", "cpu"])
		);
		assert!(plan.cwd.join("Cargo.toml").is_file());
	}

	#[test]
	fn a_standalone_crate_is_built_from_its_own_manifest() {
		let dir = std::env::temp_dir().join(format!("leptoskit-game-plan-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).unwrap();
		std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"side\"\n").unwrap();
		let plan = plan(
			GameCommand::Build {
				path: Some(dir.clone()),
				release: true,
			},
			&dir,
		)
		.unwrap();
		assert!(plan.args.iter().any(|arg| arg == "--release"));
		assert!(!plan.args.iter().any(|arg| arg == "-p"));
		assert_eq!(plan.cwd, dir);
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn a_missing_crate_and_a_nameless_manifest_fail() {
		let dir =
			std::env::temp_dir().join(format!("leptoskit-game-missing-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).unwrap();
		assert!(resolve_game_dir(&dir, None).is_err());
		std::fs::write(dir.join("Cargo.toml"), "[package]\n").unwrap();
		assert!(package_name(&dir.join("Cargo.toml")).is_err());
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn execute_honours_the_program_status() {
		assert!(
			execute(&Plan {
				program: "true".to_owned(),
				args: Vec::new(),
				cwd: std::env::temp_dir(),
			})
			.is_ok()
		);
		assert!(
			execute(&Plan {
				program: "false".to_owned(),
				args: Vec::new(),
				cwd: std::env::temp_dir(),
			})
			.is_err()
		);
	}
}
