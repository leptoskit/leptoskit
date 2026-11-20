//! Cargo-style external subcommand dispatch for `lk cloud <name>`.
//!
//! Any executable named `leptoskit-cloud-<name>` beside the `lk`/`leptoskit`
//! binary or on `PATH` can be invoked as `lk cloud <name> [ARGS]...` — for
//! example `lk cloud platform up ...`, `lk cloud worker run ...`, or
//! `lk cloud control serve ...`.

use std::ffi::OsStr;
use std::ffi::OsString;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use anyhow::Context as _;
use anyhow::Result;
use anyhow::bail;
use anyhow::ensure;

use super::CloudArgs;

/// Binary name prefix searched for external `lk cloud` subcommands.
const BINARY_PREFIX: &str = "leptoskit-cloud-";

/// Validates an external subcommand name before turning it into an executable
/// lookup. Only lowercase ASCII identifiers with hyphens are accepted so path
/// traversal (`../`, `/`, `\`) or flag-like names can never resolve.
pub(super) fn validate_name(name: &str) -> Result<()> {
	ensure!(
		!name.is_empty() && name.len() <= 64,
		"Invalid cloud subcommand name: must be 1..=64 ASCII characters."
	);
	let mut chars = name.chars();
	let first_ok = chars.next().is_some_and(|c| c.is_ascii_lowercase());
	let rest_ok = chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
	ensure!(
		first_ok && rest_ok && !name.ends_with('-'),
		"Invalid cloud subcommand `{name}`: external subcommands must match `[a-z][a-z0-9-]*`."
	);
	Ok(())
}

/// Returns the platform-appropriate executable filename for `<name>`.
#[must_use]
pub(super) fn binary_name(name: &str) -> String {
	format!("{BINARY_PREFIX}{name}{}", std::env::consts::EXE_SUFFIX)
}

/// Resolves `leptoskit-cloud-<name>` first beside the current `lk`/`leptoskit`
/// executable (matching Cargo's bindir-first lookup), then along `PATH`.
#[must_use]
pub(super) fn resolve_in(
	name: &str,
	sibling_dir: Option<&Path>,
	path_var: Option<&OsStr>,
) -> Option<PathBuf> {
	let filename = binary_name(name);
	if let Some(dir) = sibling_dir {
		let candidate = dir.join(&filename);
		if is_executable_file(&candidate) {
			return Some(candidate);
		}
	}
	let path_var = path_var?;
	for dir in std::env::split_paths(path_var) {
		if dir.as_os_str().is_empty() {
			continue;
		}
		let candidate = dir.join(&filename);
		if is_executable_file(&candidate) {
			return Some(candidate);
		}
	}
	None
}

fn is_executable_file(path: &Path) -> bool {
	let Ok(metadata) = std::fs::metadata(path) else {
		return false;
	};
	if !metadata.is_file() {
		return false;
	}
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt as _;
		metadata.permissions().mode() & 0o111 != 0
	}
	#[cfg(not(unix))]
	{
		true
	}
}

/// Builds a helpful error message when `leptoskit-cloud-<name>` is not found.
#[must_use]
pub(super) fn missing_command_hint(name: &str) -> String {
	let bin = binary_name(name);
	match name {
		"platform" => {
			format!(
				"`{bin}` was not found beside `lk` or on PATH.\nInstall the operator platform \
				 CLI:\n  cargo binstall leptoskit_cloud_platform\n  cargo install \
				 leptoskit_cloud_platform --locked\nOr run from a leptoskit checkout:\n  cargo \
				 run -p leptoskit_cloud_platform -- <command>"
			)
		}
		"worker" => {
			format!(
				"`{bin}` was not found beside `lk` or on PATH.\nInstall the cloud worker \
				 binary:\n  cargo binstall leptoskit_cloud_worker\n  cargo install \
				 leptoskit_cloud_worker --locked\nOr run from a leptoskit checkout:\n  cargo run \
				 -p leptoskit_cloud_worker -- <command>"
			)
		}
		"control" => {
			format!(
				"`{bin}` was not found beside `lk` or on PATH.\nInstall the cloud control-plane \
				 binary:\n  cargo binstall leptoskit_cloud_app\n  cargo install \
				 leptoskit_cloud_app --features production --locked\nOr run from a leptoskit \
				 checkout:\n  cargo run -p leptoskit_cloud_app --features server -- <command>"
			)
		}
		_ => {
			format!(
				"No built-in or external command `lk cloud {name}` (looked for `{bin}` beside \
				 `lk` and on PATH). Run `lk cloud --help` to list built-in commands."
			)
		}
	}
}

/// Collects arguments to forward to the external executable, re-attaching any
/// global `CloudArgs` flags that clap intercepted (such as `--url` on
/// `lk cloud worker pair --url https://…`).
#[must_use]
pub(super) fn forwarded_args(args: &CloudArgs, raw_tail: &[OsString]) -> Vec<OsString> {
	let mut forwarded = raw_tail.to_vec();
	if let Some(url) = &args.url {
		forwarded.push(OsString::from("--url"));
		forwarded.push(OsString::from(url));
	}
	if args.json {
		forwarded.push(OsString::from("--json"));
	}
	if let Some(org) = &args.org {
		forwarded.push(OsString::from("--org"));
		forwarded.push(OsString::from(org));
	}
	if let Some(project) = &args.project {
		forwarded.push(OsString::from("--project"));
		forwarded.push(OsString::from(project));
	}
	if let Some(environment) = &args.environment {
		forwarded.push(OsString::from("--environment"));
		forwarded.push(OsString::from(environment));
	}
	forwarded
}

/// Dispatches `lk cloud <name> [ARGS]...` to `leptoskit-cloud-<name>`.
///
/// # Errors
///
/// Invalid subcommand name, missing executable, spawn failure, or termination
/// by signal. Non-zero exit codes from the child process are propagated
/// directly via `std::process::exit`.
pub(super) fn run_external(args: &CloudArgs, external_args: &[OsString]) -> Result<()> {
	let sibling_dir = std::env::current_exe()
		.ok()
		.and_then(|exe| exe.parent().map(Path::to_path_buf));
	let path_var = std::env::var_os("PATH");
	let code = run_with_search(
		args,
		external_args,
		sibling_dir.as_deref(),
		path_var.as_deref(),
	)?;
	if code != 0 {
		std::process::exit(code);
	}
	Ok(())
}

fn run_with_search(
	args: &CloudArgs,
	external_args: &[OsString],
	sibling_dir: Option<&Path>,
	path_var: Option<&OsStr>,
) -> Result<i32> {
	let raw_name = external_args
		.first()
		.context("Missing external cloud subcommand name.")?;
	let name = raw_name
		.to_str()
		.context("External cloud subcommand name must be valid UTF-8.")?;
	validate_name(name)?;

	let Some(executable) = resolve_in(name, sibling_dir, path_var) else {
		bail!("{}", missing_command_hint(name));
	};

	let child_args = forwarded_args(args, &external_args[1..]);
	let status = Command::new(&executable)
		.args(&child_args)
		.status()
		.with_context(|| format!("Failed to execute `{}`.", executable.display()))?;

	status
		.code()
		.context("External cloud command was terminated by a signal.")
}

#[cfg(test)]
mod tests {
	use clap::Parser;

	use super::*;
	use crate::cloud::CloudCommand;

	#[derive(Debug, Parser)]
	struct Harness {
		#[command(flatten)]
		cloud: CloudArgs,
	}

	#[test]
	fn validate_name_allows_slugs_and_blocks_paths() {
		for good in ["platform", "worker", "control", "custom-tool-2"] {
			assert!(validate_name(good).is_ok(), "should accept {good}");
		}
		for bad in [
			"",
			"../sh",
			"/bin/sh",
			"a/b",
			r"a\b",
			"Platform",
			"-flag",
			"trailing-",
			"with space",
			"1leading-digit",
		] {
			assert!(validate_name(bad).is_err(), "should reject {bad:?}");
		}
	}

	#[test]
	fn resolve_prefers_sibling_dir_over_path() {
		let root = tempfile::tempdir().unwrap();
		let sibling = root.path().join("bin-sibling");
		let path_dir = root.path().join("bin-path");
		std::fs::create_dir_all(&sibling).unwrap();
		std::fs::create_dir_all(&path_dir).unwrap();

		let name = binary_name("platform");
		let sibling_bin = sibling.join(&name);
		let path_bin = path_dir.join(&name);
		std::fs::write(&sibling_bin, "#!/bin/sh\nexit 0\n").unwrap();
		std::fs::write(&path_bin, "#!/bin/sh\nexit 0\n").unwrap();

		#[cfg(unix)]
		{
			use std::os::unix::fs::PermissionsExt as _;
			std::fs::set_permissions(&sibling_bin, std::fs::Permissions::from_mode(0o755)).unwrap();
			std::fs::set_permissions(&path_bin, std::fs::Permissions::from_mode(0o755)).unwrap();
		}

		assert_eq!(
			resolve_in("platform", Some(&sibling), Some(path_dir.as_os_str())),
			Some(sibling_bin)
		);
		assert_eq!(
			resolve_in("platform", None, Some(path_dir.as_os_str())),
			Some(path_bin)
		);
	}

	#[test]
	fn clap_parses_external_subcommands_and_forwards_intercepted_global_flags() {
		let parsed = Harness::try_parse_from([
			"cloud",
			"worker",
			"pair",
			"--url",
			"https://cloud.example.com",
			"--state",
			"/var/lib/worker",
		])
		.unwrap();

		let CloudCommand::External(argv) = &parsed.cloud.command else {
			panic!("expected CloudCommand::External");
		};
		assert_eq!(argv[0], OsStr::new("worker"));
		let forwarded = forwarded_args(&parsed.cloud, &argv[1..]);
		assert!(forwarded.contains(&OsString::from("pair")));
		assert!(forwarded.contains(&OsString::from("--state")));
		assert!(forwarded.contains(&OsString::from("/var/lib/worker")));
		assert!(forwarded.contains(&OsString::from("--url")));
		assert!(forwarded.contains(&OsString::from("https://cloud.example.com")));
	}

	#[test]
	fn missing_command_hints_name_operator_crates() {
		assert!(missing_command_hint("platform").contains("leptoskit_cloud_platform"));
		assert!(missing_command_hint("worker").contains("leptoskit_cloud_worker"));
		assert!(missing_command_hint("control").contains("leptoskit_cloud_app"));
		assert!(missing_command_hint("unknown").contains("lk cloud --help"));
	}

	#[cfg(unix)]
	#[test]
	fn run_with_search_executes_matching_external_binary_and_returns_exit_code() {
		use std::os::unix::fs::PermissionsExt as _;

		let root = tempfile::tempdir().unwrap();
		let marker = root.path().join("args.txt");
		let script = root.path().join(binary_name("demo"));
		std::fs::write(
			&script,
			format!(
				"#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\nexit 0\n",
				marker.display()
			),
		)
		.unwrap();
		std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

		let parsed =
			Harness::try_parse_from(["cloud", "demo", "up", "--config", "cloud.toml", "--dry-run"])
				.unwrap();
		let CloudCommand::External(argv) = &parsed.cloud.command else {
			panic!("expected external subcommand");
		};

		let code =
			run_with_search(&parsed.cloud, argv, None, Some(root.path().as_os_str())).unwrap();
		assert_eq!(code, 0);
		let recorded = std::fs::read_to_string(&marker).unwrap();
		assert_eq!(recorded, "up\n--config\ncloud.toml\n--dry-run\n");
	}
}
