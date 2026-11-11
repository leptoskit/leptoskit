//! `leptoskit lint`: the leptoskit project rules and clippy, run side by side
//! and reported as one.
//!
//! The project rules read source and finish in milliseconds, while clippy
//! compiles; clippy starts first and the rules run while it builds. Both
//! print in rustc's diagnostic layout, followed by one summary.
//!
//! When `leptoskit.toml` lists platforms, clippy still uses `--workspace` but
//! excludes host packages for destinations that are not enabled, so a
//! web-only `platforms = ["web"]` project does not need GTK just to pass
//! lint. The enabled hosts that compile here include
//! `.leptoskit/<platform>/registrant.rs`, which native builds write, and call
//! into features native builds turn on; lint writes the registrants that are
//! missing and passes those features, so a fresh checkout passes without a
//! native build.

use std::io::BufRead as _;
use std::io::BufReader;
use std::io::Read as _;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;
use std::time::Instant;

use anyhow::Context as _;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::LintConfig;
use leptoskit_config::Platform;

use crate::ui;
use crate::ui::Outcome;

/// Host packages the scaffold puts in `[workspace].members` that clippy should
/// skip when their platform is not in `app.platforms`.
const HOST_PLATFORMS: [Platform; 6] = [
	Platform::Ios,
	Platform::Android,
	Platform::Macos,
	Platform::Windows,
	Platform::Linux,
	Platform::Tui,
];

/// Checks the project with the leptoskit rules and clippy.
#[derive(Debug, clap::Args)]
pub struct LintArgs {
	/// Files or directories for the leptoskit rules; the whole project by
	/// default. clippy checks the workspace, excluding host crates whose
	/// platforms are not listed in `app.platforms`.
	pub paths: Vec<PathBuf>,
	/// Run only the leptoskit rules.
	#[arg(long)]
	pub no_clippy: bool,
	/// Fail on warnings as well as errors.
	#[arg(long)]
	pub deny_warnings: bool,
	/// Arguments passed on to clippy, such as `-- -W clippy::nursery`.
	#[arg(last = true, value_name = "CLIPPY_ARGS")]
	pub clippy_args: Vec<String>,
}

/// What clippy reported.
#[derive(Debug, Default)]
struct ClippyReport {
	/// Each diagnostic, rendered by rustc.
	rendered: Vec<String>,
	warnings: usize,
	errors: usize,
	/// cargo's own output when it failed without a compiler diagnostic, such
	/// as an unreadable manifest.
	failure: Option<String>,
	elapsed: std::time::Duration,
}

/// Runs `leptoskit lint`.
///
/// # Errors
///
/// Returns an error when the project cannot be read, clippy cannot start,
/// or either check fails.
pub fn run(args: &LintArgs) -> anyhow::Result<()> {
	let cwd = std::env::current_dir().context("read the current directory")?;
	let root = LeptoskitConfig::find_root().unwrap_or_else(|_| cwd.clone());
	let config_path = root.join(leptoskit_config::CONFIG_FILE);
	let config = if config_path.is_file() {
		LintConfig::load_from_file(&config_path)?
	} else {
		LintConfig::default()
	};
	let paths = if args.paths.is_empty() {
		vec![root.clone()]
	} else {
		args.paths.clone()
	};

	let name = root.file_name().map_or_else(
		|| root.display().to_string(),
		|name| name.to_string_lossy().into_owned(),
	);
	ui::heading("lint", name);

	let clippy_wanted = config.clippy && !args.no_clippy;

	let clippy = clippy_wanted.then(|| {
		let root = root.clone();
		let extra = args.clippy_args.clone();
		let deny_warnings = args.deny_warnings;
		let excludes = clippy_host_excludes(&root);
		let features = prepare_hosts(&root);
		std::thread::spawn(move || run_clippy(&root, deny_warnings, &extra, &excludes, &features))
	});

	let started = Instant::now();
	let report = leptoskit_lint::lint_paths(&paths)?;
	let rules_elapsed = started.elapsed();

	let clippy = match clippy {
		Some(handle) => {
			Some(
				handle
					.join()
					.map_err(|_| anyhow::anyhow!("the clippy thread panicked"))??,
			)
		}
		None => None,
	};

	// Everything prints once clippy is done, so the progress line it shows
	// while it builds never splits a diagnostic.
	let mut out = anstream::stdout().lock();

	for diagnostic in &report.diagnostics {
		let _ = writeln!(
			out,
			"{}",
			leptoskit_lint::render_diagnostic(diagnostic, &cwd)
		);
	}

	if let Some(clippy) = &clippy {
		for rendered in &clippy.rendered {
			let _ = writeln!(out, "{rendered}\n");
		}

		if let Some(failure) = &clippy.failure {
			let _ = writeln!(out, "{failure}\n");
		}
	}

	drop(out);

	let mut summary = ui::Summary::default();
	let mut rules_detail = vec![ui::count(report.files, "file")];
	rules_detail.push(
		problems(report.errors(), report.warnings()).unwrap_or_else(|| "no problems".to_owned()),
	);

	if !report.unparsed.is_empty() {
		rules_detail.push(format!(
			"{} not parsed",
			ui::count(report.unparsed.len(), "file")
		));
	}

	summary.row(
		outcome(report.errors(), report.warnings(), args.deny_warnings),
		"leptoskit",
		rules_detail.join(" · "),
		Some(rules_elapsed),
	);

	match &clippy {
		Some(clippy) if clippy.failure.is_some() => {
			summary.row(
				Outcome::Failed,
				"clippy",
				"cargo failed before clippy could check",
				Some(clippy.elapsed),
			);
		}

		Some(clippy) => {
			summary.row(
				outcome(clippy.errors, clippy.warnings, args.deny_warnings),
				"clippy",
				problems(clippy.errors, clippy.warnings)
					.unwrap_or_else(|| "no problems".to_owned()),
				Some(clippy.elapsed),
			);
		}

		None if args.no_clippy => {
			summary.row(Outcome::Skipped, "clippy", "skipped with --no-clippy", None);
		}

		None => {
			summary.row(
				Outcome::Skipped,
				"clippy",
				"skipped by `[lint] clippy = false`",
				None,
			);
		}
	}

	summary.print();

	anyhow::ensure!(!summary.failed(), "lint found problems to fix");
	Ok(())
}

/// The outcome of a check that found `errors` and `warnings`.
fn outcome(errors: usize, warnings: usize, deny_warnings: bool) -> Outcome {
	if errors > 0 || (deny_warnings && warnings > 0) {
		Outcome::Failed
	} else if warnings > 0 {
		Outcome::Warned
	} else {
		Outcome::Passed
	}
}

/// `1 error · 2 warnings`, or `None` when there are neither.
fn problems(errors: usize, warnings: usize) -> Option<String> {
	let parts: Vec<String> = [(errors, "error"), (warnings, "warning")]
		.into_iter()
		.filter(|&(count, _)| count > 0)
		.map(|(count, noun)| ui::count(count, noun))
		.collect();
	(!parts.is_empty()).then(|| parts.join(" · "))
}

/// Prepares the enabled hosts that compile here for clippy: writes the
/// registrants they include, which only native builds write, so a fresh
/// checkout compiles, and returns the features native builds turn on for
/// them. A host that can't be prepared is a warning, and clippy reports
/// what it then can't compile.
fn prepare_hosts(root: &Path) -> Vec<String> {
	let Ok(config) = LeptoskitConfig::load_from(root) else {
		return Vec::new();
	};

	match crate::native::workspace_hosts(root, &config) {
		Ok(hosts) => hosts.features,
		Err(error) => {
			ui::warning(format_args!(
				"could not prepare the platform hosts for clippy: {error:#}"
			));
			Vec::new()
		}
	}
}

/// Host package names to `--exclude` from clippy for platforms that are not
/// enabled in `app.platforms`. Missing or unreadable config yields no excludes.
fn clippy_host_excludes(root: &Path) -> Vec<String> {
	let Ok(config) = LeptoskitConfig::load_from(root) else {
		return Vec::new();
	};
	let crate_name = config.app.name.replace('-', "_");
	HOST_PLATFORMS
		.into_iter()
		.filter(|platform| !config.app.platforms.contains(platform))
		.map(|platform| format!("{crate_name}_{platform}"))
		.collect()
}

/// Runs clippy over the workspace at `root`, collecting its diagnostics
/// from cargo's JSON messages and showing which crate it is on.
///
/// Denying warnings goes through cargo's `build.warnings` rather than
/// `-D warnings`, so cargo's own lints, such as an unused dependency, fail
/// the run too, and the build cache survives the switch.
fn run_clippy(
	root: &Path,
	deny_warnings: bool,
	extra: &[String],
	excludes: &[String],
	features: &[String],
) -> anyhow::Result<ClippyReport> {
	let started = Instant::now();
	let cargo = std::env::var_os("CARGO")
		.filter(|cargo| !cargo.is_empty())
		.unwrap_or_else(|| "cargo".into());
	let mut command = Command::new(cargo);
	command.current_dir(root).args([
		"clippy",
		"--workspace",
		"--all-targets",
		"--message-format=json-diagnostic-rendered-ansi",
	]);

	for package in excludes {
		command.args(["--exclude", package]);
	}

	if !features.is_empty() {
		command.args(["--features", &features.join(",")]);
	}

	let mut child = command
		.args(
			deny_warnings
				.then_some(["--config", "build.warnings=\"deny\""])
				.into_iter()
				.flatten(),
		)
		.arg("--")
		.args(extra)
		.stdin(Stdio::null())
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn()
		.context("start `cargo clippy`; is clippy installed? (`rustup component add clippy`)")?;

	// cargo's progress and errors go to stderr; read it alongside stdout so
	// neither pipe fills and stalls the build.
	let mut stderr = child.stderr.take().context("clippy's stderr")?;
	let stderr = std::thread::spawn(move || {
		let mut text = String::new();
		let _ = stderr.read_to_string(&mut text);
		text
	});

	let progress = ui::Progress::default();
	let mut report = ClippyReport::default();
	let mut seen = std::collections::HashSet::new();
	let stdout = child.stdout.take().context("clippy's stdout")?;

	for line in BufReader::new(stdout).lines() {
		let line = line.context("read clippy's output")?;
		let Ok(message) = serde_json::from_str::<serde_json::Value>(&line) else {
			continue;
		};

		match message["reason"].as_str() {
			Some("compiler-artifact") => {
				if let Some(name) = message["target"]["name"].as_str() {
					progress.update(format_args!("clippy · checking {name}"));
				}
			}
			Some("compiler-message") => {
				let diagnostic = &message["message"];
				let counted = match diagnostic["level"].as_str() {
					Some("warning") => &mut report.warnings,
					Some("error") => &mut report.errors,
					_ => continue,
				};

				// A diagnostic in code shared by several targets, such as a
				// library and its tests, arrives once per target.
				let rendered = diagnostic["rendered"]
					.as_str()
					.unwrap_or_default()
					.trim_end();
				if seen.insert(rendered.to_owned()) {
					*counted += 1;
					report.rendered.push(rendered.to_owned());
				}
			}
			_ => {}
		}
	}

	progress.finish();

	let status = child.wait().context("wait for clippy")?;
	let stderr = stderr.join().unwrap_or_default();
	// A failure without a compiler diagnostic is cargo's own: an unreadable
	// manifest, or its lints with warnings denied.
	if !status.success() && report.errors == 0 && report.warnings == 0 {
		report.failure = Some(stderr.trim_end().to_owned());
	}

	report.elapsed = started.elapsed();
	Ok(report)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn outcomes_follow_counts_and_the_warning_policy() {
		assert_eq!(outcome(0, 0, true), Outcome::Passed);
		assert_eq!(outcome(0, 2, false), Outcome::Warned);
		assert_eq!(outcome(0, 2, true), Outcome::Failed);
		assert_eq!(outcome(1, 0, false), Outcome::Failed);
	}

	#[test]
	fn clippy_host_excludes_uses_app_name_and_platforms() {
		let dir = tempfile::tempdir().expect("scratch");
		std::fs::write(
			dir.path().join("leptoskit.toml"),
			"[app]\nname = \"my-notes\"\nplatforms = [\"web\", \"ios\"]\n",
		)
		.expect("write config");
		let excludes = clippy_host_excludes(dir.path());
		assert!(excludes.contains(&"my_notes_android".to_owned()));
		assert!(excludes.contains(&"my_notes_linux".to_owned()));
		assert!(excludes.contains(&"my_notes_tui".to_owned()));
		assert!(!excludes.iter().any(|name| name == "my_notes_ios"));
		let missing = dir.path().join("missing");

		assert_eq!(clippy_host_excludes(&missing), Vec::<String>::new());
	}

	#[test]
	fn hosts_that_cannot_be_prepared_are_left_to_clippy() {
		let dir = tempfile::tempdir().expect("scratch");
		// No configuration: nothing to prepare.
		assert_eq!(prepare_hosts(dir.path()), Vec::<String>::new());

		std::fs::write(
			dir.path().join("leptoskit.toml"),
			"[app]\nname = \"notes\"\nplatforms = [\"web\", \"tui\"]\n",
		)
		.expect("write config");
		std::fs::create_dir(dir.path().join("tui")).expect("terminal host");
		std::fs::write(dir.path().join("tui/Cargo.toml"), "").expect("host manifest");
		// The host can't be resolved, so lint warns and clippy reports what
		// it can't compile.
		assert_eq!(prepare_hosts(dir.path()), Vec::<String>::new());
		assert!(!dir.path().join(".leptoskit/tui/registrant.rs").exists());
		std::fs::write(
			dir.path().join("leptoskit.toml"),
			"[app]\nname = \"notes\"\nplatforms = [\"web\"]\n",
		)
		.expect("web-only config");
		assert_eq!(prepare_hosts(dir.path()), Vec::<String>::new());
	}

	fn lint_fixture(source: &str) -> tempfile::TempDir {
		let dir = tempfile::tempdir().expect("scratch");
		std::fs::write(
			dir.path().join("Cargo.toml"),
			"[package]\nname = \"lint-probe\"\nversion = \"0.1.0\"\nedition = \
			 \"2024\"\n[workspace]\n[features]\nnative-host = []\n",
		)
		.expect("manifest");
		std::fs::create_dir(dir.path().join("src")).expect("source directory");
		std::fs::write(dir.path().join("src/lib.rs"), source).expect("source");
		dir
	}

	#[test]
	fn clippy_builds_with_the_native_host_features() {
		let dir = lint_fixture(
			"#[cfg(not(feature = \"native-host\"))]\ncompile_error!(\"the host feature must reach \
			 clippy\");\n",
		);
		let report = run_clippy(dir.path(), true, &[], &[], &["native-host".to_owned()])
			.expect("clippy starts");
		assert_eq!((report.errors, report.warnings), (0, 0), "{report:?}");
		assert!(report.failure.is_none(), "{report:?}");
	}

	#[test]
	fn lint_runs_clippy_in_a_fresh_workspace() {
		const CHILD_ROOT: &str = "LEPTOSKIT_TEST_LINT_WORKSPACE";
		if let Some(root) = std::env::var_os(CHILD_ROOT) {
			// Change the working directory only in this isolated child.
			std::env::set_current_dir(root).expect("fixture directory");
			run(&LintArgs {
				paths: Vec::new(),
				no_clippy: false,
				deny_warnings: true,
				clippy_args: Vec::new(),
			})
			.expect("fresh workspace passes both lint stages");
			return;
		}
		let dir = lint_fixture("pub fn ready() {}\n");
		std::fs::write(
			dir.path().join("leptoskit.toml"),
			"[app]\nname = \"lint-probe\"\nplatforms = [\"web\", \"macos\", \"linux\", \
			 \"windows\", \"ios\", \"android\", \"tui\"]\n",
		)
		.expect("config");
		let result = Command::new(std::env::current_exe().expect("test executable"))
			.args([
				"--exact",
				"lint::tests::lint_runs_clippy_in_a_fresh_workspace",
				"--nocapture",
			])
			.env(CHILD_ROOT, dir.path())
			.output()
			.expect("isolated lint process");
		assert!(
			result.status.success(),
			"{}\n{}",
			String::from_utf8_lossy(&result.stdout),
			String::from_utf8_lossy(&result.stderr)
		);
	}

	#[test]
	fn problems_name_only_what_was_found() {
		assert_eq!(problems(0, 0), None);
		assert_eq!(problems(1, 0).as_deref(), Some("1 error"));
		assert_eq!(problems(2, 1).as_deref(), Some("2 errors · 1 warning"));
	}
}
