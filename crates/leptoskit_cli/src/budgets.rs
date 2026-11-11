//! Performance budgets, and the six blocking checks.
//!
//! Exact (`rules/RULES.md`) blocks on six checks that finish in about 65
//! seconds: `build`, `test`, `lint`, `caps`, `boot`, and `workspace`. A
//! seventh requires deleting one. `caps` holds the budgets. `boot` counts
//! the modules that run before the first pixel, because a count cannot flake
//! the way a timer does.
//!
//! leptoskit keeps the same six names. `caps` and `boot` run in this process
//! and in `setup/scripts/budgets.py`, and fail when the dev profile grows a
//! link-time optimization or the scaffold's boot graph grows. `build`,
//! `test`, `lint`, and `workspace` are the workspace clippy, test, and format
//! commands CI already runs; they do not fit in 65 seconds, so this command
//! names them and does not start them.

use std::fmt::Write as _;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;

use anyhow::Result;

/// Cold start to an interactive first frame. Same target as Exact.
pub const COLD_START: Duration = Duration::from_millis(100);

/// Dev restart, edit to present. Rust compilation is why this is not 100 ms.
pub const EDIT_TO_PRESENT: Duration = Duration::from_millis(500);

/// Touch one line, rebuild that crate. Exact's target, recorded as a metric.
/// The gate does not fail on the clock.
pub const CRATE_REBUILD: Duration = Duration::from_secs(30);

/// The six blocking checks, in Exact's order. A seventh does not type-check.
pub const CHECKS: [&str; 6] = ["build", "test", "lint", "caps", "boot", "workspace"];

/// What `leptoskit dev` rebuilds after an edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rebuild {
	/// The server and the wasm client.
	Full,
	/// The wasm client only.
	Client,
	/// The server only.
	Server,
	/// Files the browser loads without a compile.
	Assets,
}

impl Rebuild {
	/// Whether this edit compiles the server package.
	#[must_use]
	pub const fn compiles_server(self) -> bool {
		matches!(self, Self::Full | Self::Server)
	}

	/// Whether this edit compiles the wasm client.
	#[must_use]
	pub const fn compiles_client(self) -> bool {
		matches!(self, Self::Full | Self::Client)
	}

	/// How a rebuild line names this edit.
	#[must_use]
	pub const fn label(self) -> &'static str {
		match self {
			Self::Full => "the client and the server",
			Self::Client => "the client",
			Self::Server => "the server",
			Self::Assets => "assets",
		}
	}
}

/// One workspace package, reduced to the fields classification needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
	/// Package name.
	pub name: String,
	/// Directory that contains `Cargo.toml`.
	pub dir: PathBuf,
	/// Workspace packages this one depends on, by package name.
	pub deps: Vec<String>,
}

/// Names of `root` and the workspace packages it depends on.
#[must_use]
pub fn dependency_names(packages: &[Package], root: &str) -> std::collections::BTreeSet<String> {
	let mut names = std::collections::BTreeSet::new();
	let mut stack = vec![root.to_owned()];

	while let Some(name) = stack.pop() {
		if !names.insert(name.clone()) {
			continue;
		}

		let Some(package) = packages.iter().find(|package| package.name == name) else {
			continue;
		};

		stack.extend(package.deps.iter().cloned());
	}

	names
}

/// `public/`, `web/`, and `assets/` under a project.
#[must_use]
pub fn asset_dirs(root: &Path) -> Vec<PathBuf> {
	["web", "public", "assets"]
		.into_iter()
		.map(|dir| root.join(dir))
		.collect()
}

/// Which sides an edit needs. A manifest, an empty path list, or a file in a
/// package neither side links rebuilds both sides. The package that owns a
/// file is the one with the longest directory prefix, so the workspace root
/// does not claim every file under it.
#[must_use]
pub fn classify_edit(
	paths: &[PathBuf],
	packages: &[Package],
	client: &std::collections::BTreeSet<String>,
	server: &std::collections::BTreeSet<String>,
	asset_dirs: &[PathBuf],
) -> Rebuild {
	if paths.is_empty() || paths.iter().any(|path| forces_full(path)) {
		return Rebuild::Full;
	}

	let mut saw_asset = false;
	let mut saw_client = false;
	let mut saw_server = false;

	for path in paths {
		if is_asset(path, asset_dirs) {
			saw_asset = true;
			continue;
		}

		let Some(owner) = owner(path, packages) else {
			return Rebuild::Full;
		};

		let on_client = client.contains(owner);
		let on_server = server.contains(owner);

		if !on_client && !on_server {
			return Rebuild::Full;
		}

		saw_client |= on_client;
		saw_server |= on_server;
	}

	match (saw_asset, saw_client, saw_server) {
		(true, false, false) => Rebuild::Assets,
		(false, true, false) => Rebuild::Client,
		(false, false, true) => Rebuild::Server,
		_ => Rebuild::Full,
	}
}

fn owner<'a>(path: &Path, packages: &'a [Package]) -> Option<&'a str> {
	packages
		.iter()
		.filter(|package| path.starts_with(&package.dir))
		.max_by_key(|package| package.dir.components().count())
		.map(|package| package.name.as_str())
}

fn forces_full(path: &Path) -> bool {
	let name = path.file_name().and_then(|name| name.to_str());
	matches!(name, Some("Cargo.toml" | "Cargo.lock" | "leptoskit.toml"))
		|| path
			.components()
			.any(|component| component.as_os_str() == ".cargo")
}

fn is_asset(path: &Path, asset_dirs: &[PathBuf]) -> bool {
	if matches!(
		path.extension().and_then(|extension| extension.to_str()),
		Some("rs" | "toml")
	) {
		return false;
	}

	asset_dirs.iter().any(|dir| path.starts_with(dir))
}

/// Problems in the dev profile and the budget constants. Empty means the
/// caps check passed.
#[must_use]
pub fn caps() -> Vec<String> {
	let mut problems = Vec::new();

	if COLD_START != Duration::from_millis(100) {
		problems.push(format!(
			"cold start budget is {} ms; the target is 100",
			COLD_START.as_millis()
		));
	}

	if EDIT_TO_PRESENT != Duration::from_millis(500) {
		problems.push(format!(
			"edit-to-present budget is {} ms; the target is 500",
			EDIT_TO_PRESENT.as_millis()
		));
	}

	if CHECKS != ["build", "test", "lint", "caps", "boot", "workspace"] {
		problems.push(format!(
			"the blocking checks are {}; they stay build, test, lint, caps, boot, workspace",
			CHECKS.join(", ")
		));
	}

	for entry in crate::wasm::dev_profile_config() {
		if entry.contains("lto") || entry.contains("codegen-units") {
			problems.push(format!(
				"the dev profile enables a setting that slows an incremental rebuild: {entry}"
			));
		}
	}

	let profile = crate::wasm::dev_profile_config().join("\n");

	for required in [
		"inherits = \"dev\"",
		"debug = \"line-tables-only\"",
		"incremental = true",
	] {
		if !profile.contains(required) {
			problems.push(format!("the dev profile is missing `{required}`"));
		}
	}

	problems
		.into_iter()
		.map(|problem| format!("caps: {problem}"))
		.collect()
}

/// Problems in the scaffold's boot graph. Empty means the boot check passed.
#[must_use]
pub fn boot() -> Vec<String> {
	let mut problems = Vec::new();
	problems.extend(boot_document(
		"web/index.html",
		&crate::new_project::index_html("demo"),
	));

	for (name, source) in SHELLS {
		problems.extend(boot_shell(name, source));
	}

	problems
		.into_iter()
		.map(|problem| format!("boot: {problem}"))
		.collect()
}

const SHELLS: &[(&str, &str)] = &[
	(
		"new_project/templates/app.rs",
		include_str!("new_project/templates/app.rs"),
	),
	(
		"new_project/templates/app_auth.rs",
		include_str!("new_project/templates/app_auth.rs"),
	),
	(
		"new_project/templates/app_db.rs",
		include_str!("new_project/templates/app_db.rs"),
	),
];

/// The CSR document may run one inline module, which imports one package
/// glue script and names one wasm module.
pub(crate) fn boot_document(name: &str, html: &str) -> Vec<String> {
	let mut problems = Vec::new();
	let tags = match script_tags(html) {
		Ok(tags) => tags,
		Err(error) => {
			problems.push(format!("{name}: {error}"));
			return problems;
		}
	};

	if tags.len() != 1 {
		problems.push(format!(
			"{name}: {} script tags; the boot budget is 1",
			tags.len()
		));
	}

	for tag in &tags {
		if tag.src.is_some() {
			problems.push(format!(
				"{name}: a script has a src attribute; the document's one script is inline"
			));
		}

		if tag.kind.as_deref() != Some("module") {
			problems.push(format!(
				"{name}: the script type is {:?}; the boot budget is one module",
				tag.kind
			));
		}

		let imports = import_specifiers(&tag.body);

		if imports.len() != 1
			|| !imports.iter().any(|import| {
				import.contains("/pkg/")
					&& Path::new(import)
						.extension()
						.is_some_and(|extension| extension.eq_ignore_ascii_case("js"))
			}) {
			problems.push(format!(
				"{name}: {} package imports ({}); the boot budget is one /pkg/*.js import",
				imports.len(),
				imports.join(", ")
			));
		}

		let wasm = tag.body.matches(".wasm").count();

		if wasm != 1 {
			problems.push(format!(
				"{name}: {wasm} wasm references; the boot budget is 1"
			));
		}
	}

	problems
}

fn boot_shell(name: &str, source: &str) -> Vec<String> {
	let mut problems = Vec::new();
	let scripts = source.matches("<script").count();
	let hydration = source.matches("HydrationScripts").count();

	if scripts != 0 {
		problems.push(format!(
			"{name}: {scripts} <script tags; the shell's boot budget is 0 (hydration is a \
			 component)"
		));
	}

	if hydration != 1 {
		problems.push(format!(
			"{name}: {hydration} HydrationScripts; the boot budget is 1"
		));
	}

	problems
}

struct ScriptTag {
	src: Option<String>,
	kind: Option<String>,
	body: String,
}

fn script_tags(html: &str) -> Result<Vec<ScriptTag>, String> {
	let lower = html.to_ascii_lowercase();
	let mut tags = Vec::new();
	let mut from = 0;

	while let Some(relative) = lower[from..].find("<script") {
		let start = from + relative;
		let after = start + "<script".len();
		let boundary = html[after..].chars().next();

		if !boundary.is_some_and(|character| {
			character.is_whitespace() || character == '>' || character == '/'
		}) {
			from = after;
			continue;
		}

		let end = html[after..]
			.find('>')
			.ok_or_else(|| "unterminated script tag".to_owned())?;
		let open_end = after + end;
		let open = &html[after..open_end];
		let close = lower[open_end..]
			.find("</script")
			.ok_or_else(|| "script has no closing tag".to_owned())?;
		tags.push(ScriptTag {
			src: attribute(open, "src"),
			kind: attribute(open, "type"),
			body: html[open_end + 1..open_end + close].to_owned(),
		});
		from = open_end + close;
	}

	Ok(tags)
}

fn attribute(open: &str, name: &str) -> Option<String> {
	let lower = open.to_ascii_lowercase();
	let needle = format!("{name}=");
	let start = lower.find(&needle)? + needle.len();
	let rest = open[start..].trim_start();
	let quote = rest.chars().next()?;

	if quote == '"' || quote == '\'' {
		let value = rest[1..].split(quote).next()?;
		return Some(value.to_owned());
	}

	Some(
		rest.split_whitespace()
			.next()
			.unwrap_or_default()
			.to_owned(),
	)
}

fn import_specifiers(source: &str) -> Vec<String> {
	let mut imports = Vec::new();
	let mut rest = source;

	while let Some(start) = rest.find("from ") {
		rest = &rest[start + "from ".len()..];
		let trimmed = rest.trim_start();
		let Some(quote) = trimmed.chars().next() else {
			break;
		};

		if quote != '"' && quote != '\'' {
			continue;
		}

		let Some(end) = trimmed[1..].find(quote) else {
			break;
		};

		imports.push(trimmed[1..=end].to_owned());
		rest = &trimmed[1 + end..];
	}

	imports
}

/// The commands for the four checks CI already runs, plus caps and boot.
#[must_use]
pub fn commands() -> &'static [(&'static str, &'static str)] {
	&[
		(
			"build",
			"cargo clippy --locked --keep-going --config 'build.warnings=\"deny\"' --workspace \
			 --all-features --all-targets",
		),
		(
			"test",
			"cargo test --locked --workspace --all-features --no-fail-fast",
		),
		("lint", "devenv shell lint:all"),
		("caps", "python3 setup/scripts/budgets.py"),
		("boot", "python3 setup/scripts/budgets.py"),
		(
			"workspace",
			"cargo clippy --locked --keep-going --config 'build.warnings=\"deny\"' --workspace \
			 --all-features --all-targets",
		),
	]
}

struct Gate {
	report: String,
	problems: Vec<String>,
}

fn gate() -> Gate {
	let started = Instant::now();
	let mut problems = caps();
	problems.extend(boot());
	let elapsed = started.elapsed();
	Gate {
		report: render(&problems, elapsed),
		problems,
	}
}

fn render(problems: &[String], elapsed: Duration) -> String {
	let mut report = String::new();
	let caps_failed = problems.iter().any(|problem| problem.starts_with("caps:"));
	let boot_failed = problems.iter().any(|problem| problem.starts_with("boot:"));

	for (name, command) in commands() {
		let state = match *name {
			"caps" if caps_failed => "failed",
			"boot" if boot_failed => "failed",
			"caps" | "boot" => "ok",
			_ => "ci",
		};
		let _ = writeln!(report, "{name}\t{state}\t{command}");
	}

	let _ = writeln!(
		report,
		"cold start\t{} ms p50\tnot timed by this command",
		COLD_START.as_millis()
	);
	let _ = writeln!(
		report,
		"edit to present\t{} ms\tleptoskit dev prints each rebuild against this",
		EDIT_TO_PRESENT.as_millis()
	);
	let _ = writeln!(
		report,
		"touch one line\t{} s\ta metric; this command does not time a crate rebuild",
		CRATE_REBUILD.as_secs()
	);
	let _ = writeln!(report, "caps+boot\t{} ms", elapsed.as_millis());

	for problem in problems {
		let _ = writeln!(report, "problem\t{problem}");
	}

	report
}

/// Prints the six checks. Exits with an error when caps or boot find a
/// regression.
///
/// # Errors
///
/// Returns an error when caps or boot reported a problem.
pub fn run() -> Result<()> {
	let outcome = gate();
	anstream::print!("{}", outcome.report);

	if outcome.problems.is_empty() {
		Ok(())
	} else {
		anyhow::bail!("{} blocking check(s) failed", outcome.problems.len());
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn package(name: &str, dir: &str, deps: &[&str]) -> Package {
		Package {
			name: name.to_owned(),
			dir: PathBuf::from(dir),
			deps: deps.iter().map(|dep| (*dep).to_owned()).collect(),
		}
	}

	#[test]
	fn six_checks_and_the_time_budgets_stay_put() {
		assert_eq!(
			CHECKS,
			["build", "test", "lint", "caps", "boot", "workspace"]
		);
		assert_eq!(COLD_START, Duration::from_millis(100));
		assert_eq!(EDIT_TO_PRESENT, Duration::from_millis(500));
		assert_eq!(CRATE_REBUILD, Duration::from_secs(30));
		assert!(caps().is_empty(), "{:?}", caps());
		assert!(boot().is_empty(), "{:?}", boot());
	}

	fn sides(
		packages: &[Package],
	) -> (
		std::collections::BTreeSet<String>,
		std::collections::BTreeSet<String>,
		Vec<PathBuf>,
	) {
		(
			dependency_names(packages, "demo_app"),
			dependency_names(packages, "demo"),
			asset_dirs(Path::new("/app")),
		)
	}

	#[test]
	fn a_screen_shared_with_the_server_rebuilds_both_sides() {
		let packages = vec![
			package("demo", "/app", &["demo_app"]),
			package("demo_app", "/app/demo_app", &[]),
		];
		let (client, server, assets) = sides(&packages);
		let screen = PathBuf::from("/app/demo_app/src/routes/index.rs");

		assert_eq!(
			classify_edit(&[screen], &packages, &client, &server, &assets),
			Rebuild::Full
		);
	}

	#[test]
	fn an_asset_does_not_compile_and_a_server_only_file_skips_the_client() {
		let packages = vec![
			package("demo", "/app", &["demo_app", "demo_backend"]),
			package("demo_app", "/app/demo_app", &[]),
			package("demo_backend", "/app/demo_backend", &[]),
		];
		let (client, server, assets) = sides(&packages);

		assert_eq!(
			classify_edit(
				&[PathBuf::from("/app/assets/app.css")],
				&packages,
				&client,
				&server,
				&assets
			),
			Rebuild::Assets
		);
		assert_eq!(
			classify_edit(
				&[PathBuf::from("/app/demo_backend/src/lib.rs")],
				&packages,
				&client,
				&server,
				&assets
			),
			Rebuild::Server
		);
		assert_eq!(
			classify_edit(
				&[
					PathBuf::from("/app/demo_app/src/lib.rs"),
					PathBuf::from("/app/Cargo.toml"),
				],
				&packages,
				&client,
				&server,
				&assets
			),
			Rebuild::Full
		);
		assert_eq!(
			classify_edit(&[], &packages, &client, &server, &assets),
			Rebuild::Full
		);
	}

	#[test]
	fn a_client_package_the_server_does_not_link_skips_the_server() {
		let packages = vec![
			package("demo", "/app/server", &[]),
			package("demo_app", "/app/web_only", &[]),
		];
		let (client, server, assets) = sides(&packages);
		assert_eq!(
			classify_edit(
				&[PathBuf::from("/app/web_only/src/lib.rs")],
				&packages,
				&client,
				&server,
				&assets
			),
			Rebuild::Client
		);
	}

	#[test]
	fn the_scaffold_document_is_one_module_and_a_second_script_fails() {
		let html = crate::new_project::index_html("demo");
		assert_eq!(boot_document("web/index.html", &html), Vec::<String>::new());
		let extra = html.replace("</body>", "<script src=\"/extra.js\"></script></body>");
		let problems = boot_document("web/index.html", &extra);
		assert!(
			problems
				.iter()
				.any(|problem| problem.contains("script tags")),
			"{problems:?}"
		);
	}

	#[test]
	fn the_repo_script_agrees() {
		let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../setup/scripts/budgets.py");
		let output = std::process::Command::new("python3")
			.arg(&script)
			.output()
			.expect("python3");
		assert!(
			output.status.success(),
			"{}{}",
			String::from_utf8_lossy(&output.stdout),
			String::from_utf8_lossy(&output.stderr)
		);
	}
}
