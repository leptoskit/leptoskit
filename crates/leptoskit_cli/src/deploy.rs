//! `leptoskit generate deploy`: writes the files that run the app's server
//! somewhere else, for one target at a time.
//!
//! The files are the app's to own. A file that already matches is left
//! alone, a file that differs is kept unless `--force` replaces it, and
//! everything the files need comes from `leptoskit.toml` and the server's
//! `Cargo.toml`.

use std::fmt;
use std::fmt::Write as _;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use leptoskit_config::DbDialect;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::Platform;

/// Where the server is deployed. Each target writes the container recipe
/// and, except `docker`, the platform's own file beside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Target {
	/// A `Dockerfile` and `.dockerignore`, for any container platform.
	Docker,
	/// `compose.yaml` beside the `Dockerfile`, with the database it needs.
	Compose,
	/// `fly.toml` beside the `Dockerfile`, for Fly.io.
	Fly,
	/// `kubernetes.yaml` beside the `Dockerfile`: a Deployment, Service,
	/// and `PodDisruptionBudget` for any Kubernetes cluster.
	Kubernetes,
	/// A systemd unit beside the `Dockerfile`, for a Linux host.
	Systemd,
}

impl Target {
	/// The name `--target` takes.
	#[must_use]
	pub fn as_str(self) -> &'static str {
		match self {
			Self::Docker => "docker",
			Self::Compose => "compose",
			Self::Fly => "fly",
			Self::Kubernetes => "kubernetes",
			Self::Systemd => "systemd",
		}
	}

	/// The files this target writes, relative to the project root.
	fn files(self, project: &Project) -> Vec<(String, String)> {
		let mut files = vec![
			("Dockerfile".to_owned(), dockerfile(project)),
			(project.ignore_file.clone(), project.ignore_contents.clone()),
		];

		match self {
			Self::Docker => {}
			Self::Compose => files.push(("compose.yaml".to_owned(), compose(project))),
			Self::Fly => files.push(("fly.toml".to_owned(), fly(project))),
			Self::Kubernetes => files.push(("kubernetes.yaml".to_owned(), kubernetes(project))),
			Self::Systemd => files.push((format!("{}.service", project.app), systemd(project))),
		}

		files
	}

	/// The commands that take the files from here to a running server.
	fn next(self, project: &Project) -> Vec<String> {
		match self {
			Self::Docker => docker_next(project),
			Self::Compose => compose_next(project),
			Self::Fly => fly_next(project),
			Self::Kubernetes => kubernetes_next(project),
			Self::Systemd => systemd_next(project),
		}
	}
}

impl fmt::Display for Target {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(self.as_str())
	}
}

/// How long the platform waits after `SIGTERM` before it kills the server.
/// The server's own [`GracefulShutdown`](leptoskit_axum::GracefulShutdown)
/// defaults stop it within 25 seconds, so this leaves it a margin.
const STOP_TIMEOUT_SECONDS: u64 = 30;

/// Where the container keeps a SQLite database: the one writable path.
const DATA_DIR: &str = "/data";

/// Where the site goes: `leptoskit build`'s default output directory.
const DIST_DIR: &str = "dist";

/// The binaryen release whose `wasm-opt` shrinks the release client.
const BINARYEN_VERSION: &str = "132";

/// Everything the files need to know about the project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
	/// `app.name`: the server package.
	pub app: String,
	/// The server package's binary.
	pub binary: String,
	/// The port the server listens on, `serve.port`.
	pub port: u16,
	/// `LEPTOS_OUTPUT_NAME`: the client library's name, which names the
	/// files in `dist/pkg`.
	pub output_name: String,
	/// The platform `leptoskit build` builds: `web` when the app has a web
	/// client, otherwise `server`.
	pub platform: Platform,
	/// The Cargo workspace root: the Docker build context.
	pub workspace_root: PathBuf,
	/// The app's root, where `leptoskit.toml` lives.
	pub app_root: PathBuf,
	/// The app's root relative to the workspace root, with `/` separators.
	/// Empty when the app is the workspace root.
	pub app_dir: String,
	/// The path to the workspace root from the app root, for a Compose file
	/// written beside the app's `leptoskit.toml`.
	pub context: String,
	/// The `Dockerfile` path in the build context, with `/` separators.
	pub dockerfile: String,
	/// Where Cargo writes binaries inside the build container.
	pub target_dir: String,
	/// The Rust toolchain file, relative to the workspace root, if there is
	/// one between the app root and the workspace root.
	pub toolchain_file: Option<String>,
	/// The Rust image's version: the toolchain file's channel when it names
	/// a release, otherwise `1`, the latest stable.
	pub rust_version: String,
	/// The database, from `[db]`.
	pub database: Option<Database>,
	/// The environment variables that hold secrets, from `[auth]`.
	pub secrets: Vec<String>,
	/// Whether `serve.probes` declares health and readiness routes.
	pub has_probes: bool,
	/// The environment variables holding other URLs with credentials: the
	/// live relay's, and the read replicas'.
	pub urls: Vec<String>,
	/// Whether to run each role as its own Deployment (`--split-roles`).
	pub split_roles: bool,
	/// The ignore file the container build reads, beside the `Dockerfile`.
	ignore_file: String,
	/// Its contents, including the project's own exclusions.
	ignore_contents: String,
}

/// The database a project uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Database {
	/// The dialect.
	pub dialect: DbDialect,
	/// The environment variable holding its URL, `db.url_env`.
	pub url_env: String,
}

impl Project {
	/// Reads the project at `root`.
	///
	/// # Errors
	///
	/// Returns an error when the app has no configured backend, or the
	/// server package or its binary can't be found.
	pub fn read(root: &Path, config: &LeptoskitConfig) -> Result<Self> {
		anyhow::ensure!(
			config.backend_enabled(true)?,
			"build.backend = false selects a static app; upload {DIST_DIR}/ to a CDN, or set \
			 build.backend = true and name the backend in app.name"
		);
		let app_root = std::fs::canonicalize(root)
			.with_context(|| format!("could not resolve the app root {}", root.display()))?;
		// A local manifest is parsed here so a malformed one names itself,
		// before cargo reads it on the way to the workspace.
		if app_root.join("Cargo.toml").is_file() {
			read_manifest(&app_root.join("Cargo.toml"))?;
		}
		if !has_manifest_in_ancestors(&app_root) {
			anyhow::bail!("there's no Cargo.toml at {}", root.display());
		}

		// Cargo owns package discovery: it matches workspace members, globs,
		// and inherited manifests, including an app directory with no
		// manifest of its own.
		let metadata = crate::wasm::cargo_metadata(&app_root)
			.with_context(|| format!("could not read the Cargo workspace for {app_root:?}"))?;
		let workspace_root = std::fs::canonicalize(&metadata.workspace_root)
			.unwrap_or_else(|_| metadata.workspace_root.clone());
		let app_dir = app_relative_dir(&workspace_root, &app_root)?;
		let app_dir = path_to_slashes(&app_dir);
		let app = config.app.name.clone();
		let package = metadata.package(&app).with_context(|| {
			format!(
				"app.name `{app}` in {} must name the server package, but there's no package \
				 called `{app}` in {}",
				leptoskit_config::CONFIG_FILE,
				workspace_root.join("Cargo.toml").display()
			)
		})?;
		let server = package_directory(&workspace_root, &app, &package.manifest_path)?;
		let (binary, _) = binary(&server, &app)?;

		let output_name = match &config.build.client_package {
			Some(client) => {
				let package = metadata.package(client).with_context(|| {
					format!(
						"build.client_package `{client}` in {} isn't a package in the workspace",
						leptoskit_config::CONFIG_FILE
					)
				})?;
				let directory = package_directory(&workspace_root, client, &package.manifest_path)?;
				lib_name(&directory.join("Cargo.toml"))?.unwrap_or_else(|| client.replace('-', "_"))
			}
			None => app.replace('-', "_"),
		};
		let platform = if config.app.platforms.contains(&Platform::Web) {
			Platform::Web
		} else {
			Platform::Server
		};

		let toolchain = find_toolchain(&app_root, &workspace_root);
		let toolchain_file = toolchain
			.as_deref()
			.and_then(|file| file.strip_prefix(&workspace_root).ok())
			.map(path_to_slashes);
		let rust_version = toolchain
			.as_deref()
			.and_then(toolchain_channel)
			.unwrap_or_else(|| "1".to_owned());
		let context = context_from_app(&app_dir);
		let dockerfile = if app_dir.is_empty() {
			"Dockerfile".to_owned()
		} else {
			format!("{app_dir}/Dockerfile")
		};
		let ignore_file = if app_dir.is_empty() {
			".dockerignore".to_owned()
		} else {
			"Dockerfile.dockerignore".to_owned()
		};
		let ignore_contents = if app_dir.is_empty() {
			DOCKERIGNORE.to_owned()
		} else {
			workspace_dockerignore(&workspace_root, &app_root, &app_dir)
		};
		let target_dir = container_target(&workspace_root, &metadata.target_directory);
		let database = config.db.as_ref().map(|db| {
			Database {
				dialect: db.dialect,
				url_env: db.url_env.clone(),
			}
		});
		let mut secrets = Vec::new();

		if let Some(auth) = &config.auth {
			secrets.push(auth.keys_env.clone());
			secrets.extend(
				[
					&auth.mail.smtp_url_env,
					&auth.mail.webhook_url_env,
					&auth.mail.webhook_secret_env,
				]
				.into_iter()
				.flatten()
				.cloned(),
			);
		}

		let has_probes = config.serve.probes;
		let urls = config
			.rpc
			.live
			.relay_url_env
			.iter()
			.chain(config.db.iter().flat_map(|db| &db.replica_url_envs))
			.cloned()
			.collect();
		Ok(Self {
			app,
			binary,
			port: config.serve.port,
			output_name,
			platform,
			workspace_root,
			app_root,
			app_dir,
			context,
			dockerfile,
			target_dir,
			toolchain_file,
			rust_version,
			database,
			secrets,
			has_probes,
			urls,
			split_roles: false,
			ignore_file,
			ignore_contents,
		})
	}

	/// The Docker build context.
	#[must_use]
	pub fn build_context(&self) -> &Path {
		&self.workspace_root
	}

	/// The `Dockerfile` path inside the build context.
	#[must_use]
	pub fn dockerfile_path(&self) -> &str {
		&self.dockerfile
	}

	/// The ignore file Docker reads beside the `Dockerfile`.
	#[must_use]
	pub fn ignore_path(&self) -> PathBuf {
		self.app_root.join(self.ignore_file.as_str())
	}

	/// The managed infrastructure's state patterns, relative to the build
	/// context, to add beside the generated ignore policy.
	#[must_use]
	pub fn managed_state_patterns(&self) -> [String; 2] {
		let root = if self.app_dir.is_empty() {
			String::new()
		} else {
			format!("/{}", self.app_dir)
		};
		[format!("{root}/infra"), format!("{root}/.kamal")]
	}

	/// The command that builds the app's image, run from the build context.
	#[must_use]
	pub fn docker_build_command(&self) -> String {
		let image = self.image();

		if self.app_dir.is_empty() {
			format!("docker build -t {image} .")
		} else {
			format!(
				"docker build -f {} -t {image} .",
				shell_quote(&self.dockerfile)
			)
		}
	}

	/// A file beside `leptoskit.toml`, relative to the build context.
	#[must_use]
	fn in_context(&self, file: &str) -> String {
		if self.app_dir.is_empty() {
			file.to_owned()
		} else {
			format!("{}/{file}", self.app_dir)
		}
	}

	/// The image name: the app's name in lowercase.
	fn image(&self) -> String {
		self.app.to_ascii_lowercase()
	}

	/// The secrets the server reads at run time: the database URL on
	/// Postgres, which holds a password, then `[auth]`'s.
	fn runtime_secrets(&self) -> Vec<String> {
		let mut secrets = Vec::new();

		if let Some(database) = &self.database
			&& database.dialect == DbDialect::Postgres
		{
			secrets.push(database.url_env.clone());
		}
		secrets.extend(self.urls.iter().cloned());
		secrets.extend(self.secrets.iter().cloned());
		secrets
	}

	fn sqlite(&self) -> Option<&Database> {
		self.database
			.as_ref()
			.filter(|database| database.dialect == DbDialect::Sqlite)
	}

	fn postgres(&self) -> Option<&Database> {
		self.database
			.as_ref()
			.filter(|database| database.dialect == DbDialect::Postgres)
	}

	/// The SQLite URL inside the container.
	fn sqlite_url(&self) -> String {
		format!("sqlite://{DATA_DIR}/{}.sqlite3", self.app)
	}
}

/// Runs `leptoskit generate deploy` in the project around the current
/// directory, and prints what it did and what's left to do.
///
/// # Errors
///
/// Returns an error when there's no project, it has no server to deploy,
/// or a file can't be read or written.
pub fn run(args: &crate::cli::DeployArgs) -> Result<()> {
	let (config, root) = LeptoskitConfig::load()?;
	let report = generate_with(&root, &config, args.target, args.force, args.split_roles)?;
	print!("{report}");
	Ok(())
}

/// What one run did, and what's left for the developer.
#[derive(Debug, Default)]
pub struct Report {
	/// The target the files are for.
	pub target: Option<Target>,
	/// One line per file created or replaced.
	pub changed: Vec<String>,
	/// One line per file left as it was because it differs.
	pub kept: Vec<String>,
	/// The steps the generator can't take itself.
	pub next: Vec<String>,
}

impl fmt::Display for Report {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		for line in self.changed.iter().chain(&self.kept) {
			writeln!(f, "{line}")?;
		}

		if self.changed.is_empty() {
			let target = self.target.map_or("deployment", Target::as_str);
			writeln!(
				f,
				"nothing changed: the {target} files are already in place"
			)?;
		}

		if !self.next.is_empty() {
			writeln!(f, "\nnext:")?;

			for (number, step) in self.next.iter().enumerate() {
				writeln!(f, "  {}. {step}", number + 1)?;
			}
		}

		Ok(())
	}
}

/// Writes `target`'s files into the project at `root`, and reports what
/// changed.
///
/// # Errors
///
/// As [`run`].
pub fn generate(
	root: &Path,
	config: &LeptoskitConfig,
	target: Target,
	force: bool,
) -> Result<Report> {
	generate_with(root, config, target, force, false)
}

/// [`generate`], with each role as its own Deployment when `split_roles`.
///
/// # Errors
///
/// As [`run`], and when `split_roles` is asked of a target other than
/// Kubernetes, or of an app without Postgres, whose roles can't share one
/// database across machines.
pub fn generate_with(
	root: &Path,
	config: &LeptoskitConfig,
	target: Target,
	force: bool,
	split_roles: bool,
) -> Result<Report> {
	let mut project = Project::read(root, config)?;

	if split_roles {
		anyhow::ensure!(
			target == Target::Kubernetes,
			"--split-roles runs each role as its own Kubernetes Deployment, so it goes with \
			 --target kubernetes; on {target}, start the same binary with --role instead"
		);
		anyhow::ensure!(
			project.postgres().is_some(),
			"--split-roles needs Postgres: the roles run on separate machines that share one \
			 database, which a SQLite file can't be; set [db] dialect = \"postgres\""
		);
		project.split_roles = true;
	}

	let mut report = Report {
		target: Some(target),
		..Report::default()
	};

	for (file, contents) in target.files(&project) {
		write_owned(root, &file, &contents, force, &mut report)?;
	}

	if !project.has_probes {
		report.next.push(
			"Backend probes are not declared (serve.probes = false), and deployment health checks \
			 require them. In its main, make a `leptoskit::server::Readiness`, merge \
			 `readiness.routes()` into the router, and serve with \
			 `leptoskit::server::serve_listener_with(listener, app, \
			 GracefulShutdown::new().readiness(&readiness))`; the deployment guide shows the \
			 whole of it. Then declare serve.probes = true."
				.to_owned(),
		);
	}

	report.next.extend(target.next(&project));
	Ok(report)
}

/// Writes a file the app owns once it exists: creates it when missing, and
/// leaves it alone when it matches or, without `force`, when it differs.
fn write_owned(
	root: &Path,
	file: &str,
	contents: &str,
	force: bool,
	report: &mut Report,
) -> Result<()> {
	let path = root.join(file);

	match std::fs::symlink_metadata(&path) {
		Ok(metadata) => {
			anyhow::ensure!(metadata.is_file(), "{file} isn't an ordinary file");
			let existing =
				std::fs::read_to_string(&path).with_context(|| format!("could not read {file}"))?;
			if existing == contents {
				return Ok(());
			}
			if !force {
				report.kept.push(format!(
					"kept {file}: it differs from the template; --force replaces it"
				));
				return Ok(());
			}

			std::fs::write(&path, contents).with_context(|| format!("could not write {file}"))?;
			report.changed.push(format!("replaced {file}"));
		}

		Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
			std::fs::write(&path, contents).with_context(|| format!("could not write {file}"))?;
			report.changed.push(format!("created {file}"));
		}
		Err(error) => return Err(error).with_context(|| format!("could not inspect {file}")),
	}

	Ok(())
}

/// The app directory relative to the Cargo workspace that owns it.
fn app_relative_dir(workspace_root: &Path, app_root: &Path) -> Result<PathBuf> {
	app_root
		.strip_prefix(workspace_root)
		.map(Path::to_path_buf)
		.with_context(|| {
			format!(
				"the app at {} is outside its Cargo workspace at {}; the container build context \
				 can only include files inside the workspace",
				app_root.display(),
				workspace_root.display()
			)
		})
}

/// The directory of a Cargo package that the build context can include.
fn package_directory(workspace_root: &Path, package: &str, manifest: &Path) -> Result<PathBuf> {
	let directory = manifest
		.parent()
		.with_context(|| format!("the package `{package}` has no directory"))?;

	directory.strip_prefix(workspace_root).with_context(|| {
		format!(
			"the package `{package}` at {} is outside the Cargo workspace at {}; the container \
			 build context can only include files inside the workspace",
			directory.display(),
			workspace_root.display()
		)
	})?;
	Ok(directory.to_path_buf())
}

/// Whether `root` or one of its ancestors has a `Cargo.toml`, so an app
/// directory inside a workspace needs no manifest of its own.
fn has_manifest_in_ancestors(root: &Path) -> bool {
	root.ancestors().any(|dir| dir.join("Cargo.toml").is_file())
}

/// A path with `/` separators, for Docker and Compose files.
fn path_to_slashes(path: &Path) -> String {
	path.to_string_lossy().replace('\\', "/")
}

/// The path from the app root back to the workspace root.
fn context_from_app(app_dir: &str) -> String {
	let depth = app_dir
		.split('/')
		.filter(|component| !component.is_empty())
		.count();

	if depth == 0 {
		".".to_owned()
	} else {
		vec![".."; depth].join("/")
	}
}

/// The toolchain file Cargo and rustup select from the app root: the
/// nearest one between the app root and the workspace root.
fn find_toolchain(app_root: &Path, workspace_root: &Path) -> Option<PathBuf> {
	let mut current = Some(app_root);

	while let Some(dir) = current {
		for name in ["rust-toolchain.toml", "rust-toolchain"] {
			let file = dir.join(name);
			if file.is_file() {
				return Some(file);
			}
		}

		if dir == workspace_root {
			break;
		}

		current = dir.parent();
	}

	None
}

/// Where Cargo writes binaries in the build container: the workspace's
/// target directory, mounted at `/src`.
fn container_target(workspace_root: &Path, target_directory: &Path) -> String {
	target_directory.strip_prefix(workspace_root).map_or_else(
		|_| "/src/target".to_owned(),
		|relative| {
			let relative = path_to_slashes(relative);
			if relative.is_empty() {
				"/src".to_owned()
			} else {
				format!("/src/{relative}")
			}
		},
	)
}

/// The workspace build context's ignore policy, plus the exclusions the
/// project already keeps in `.dockerignore` files. A Dockerfile-specific
/// ignore file replaces the context's own, so those exclusions are copied
/// in rather than silently dropped.
fn workspace_dockerignore(workspace_root: &Path, app_root: &Path, app_dir: &str) -> String {
	let mut out = WORKSPACE_DOCKERIGNORE.to_owned();

	if let Ok(text) = std::fs::read_to_string(workspace_root.join(".dockerignore")) {
		append_ignore(&mut out, "the workspace's", &text, "");
	}

	if app_root != workspace_root
		&& let Ok(text) = std::fs::read_to_string(app_root.join(".dockerignore"))
	{
		append_ignore(&mut out, "the app's", &text, app_dir);
	}

	out
}

/// Adds a project's own ignore lines, rebased into the workspace context.
fn append_ignore(out: &mut String, owner: &str, text: &str, prefix: &str) {
	let lines: Vec<String> = text
		.lines()
		.filter_map(|line| {
			let line = line.trim();
			if line.is_empty() || line.starts_with('#') {
				return None;
			}
			Some(rebase_ignore(line, prefix))
		})
		.collect();

	if lines.is_empty() {
		return;
	}

	let _ = write!(
		out,
		"\n# {owner} .dockerignore, which a Dockerfile-specific ignore file\n# would otherwise \
		 replace.\n"
	);

	for line in lines {
		out.push_str(&line);
		out.push('\n');
	}
}

/// Rebase one ignore pattern under the app directory. Negations keep their
/// `!`, so an exception still applies after the prefix.
fn rebase_ignore(pattern: &str, prefix: &str) -> String {
	if prefix.is_empty() {
		return pattern.to_owned();
	}

	let (negation, rest) = match pattern.strip_prefix('!') {
		Some(rest) => ("!", rest),
		None => ("", pattern),
	};
	let rest = rest.trim_start_matches('/');
	format!("{negation}{prefix}/{rest}")
}

/// A shell word, quoted when it needs to be.
fn shell_quote(word: &str) -> String {
	if !word.is_empty()
		&& word
			.bytes()
			.all(|byte| byte.is_ascii_alphanumeric() || b"-_./:@%+".contains(&byte))
	{
		return word.to_owned();
	}

	format!("'{}'", word.replace('\'', "'\\''"))
}

/// A Compose scalar, quoted so spaces and `#` stay part of the path.
fn yaml_string(text: &str) -> String {
	serde_json::to_string(text).unwrap_or_default()
}

/// A `COPY` instruction, with JSON syntax when a path has whitespace.
fn docker_copy(source: &str, destination: &str) -> String {
	if source.bytes().any(is_space) || destination.bytes().any(is_space) {
		format!(
			"COPY [{}]",
			[source, destination]
				.iter()
				.map(|path| serde_json::to_string(path).unwrap_or_default())
				.collect::<Vec<_>>()
				.join(", ")
		)
	} else {
		format!("COPY {source} {destination}")
	}
}

/// A `COPY --from` instruction, with JSON syntax when a path has whitespace.
fn docker_copy_from(stage: &str, source: &str, destination: &str) -> String {
	if source.bytes().any(is_space) || destination.bytes().any(is_space) {
		format!(
			"COPY --from={stage} [{}]",
			[source, destination]
				.iter()
				.map(|path| serde_json::to_string(path).unwrap_or_default())
				.collect::<Vec<_>>()
				.join(", ")
		)
	} else {
		format!("COPY --from={stage} {source} {destination}")
	}
}

fn is_space(byte: u8) -> bool {
	byte.is_ascii_whitespace()
}

/// A cache mount, unless its path has whitespace: the mount syntax treats
/// whitespace as syntax, and the build stays correct without the cache.
fn cache_mount(path: &str) -> Option<String> {
	(!path.bytes().any(is_space)).then(|| format!("--mount=type=cache,target={path}"))
}

/// The server's binary target, and its source when it can be found:
/// the one named after the package, or its only one.
fn binary(dir: &Path, package: &str) -> Result<(String, Option<PathBuf>)> {
	let manifest = dir.join("Cargo.toml");
	let document = read_manifest(&manifest)?.unwrap_or_default();
	let mut binaries: Vec<(String, PathBuf)> = document
		.get("bin")
		.and_then(toml::Value::as_array)
		.into_iter()
		.flatten()
		.filter_map(|bin| {
			let name = bin.get("name")?.as_str()?.to_owned();
			let path = bin.get("path").and_then(toml::Value::as_str).map_or_else(
				|| dir.join("src/bin").join(format!("{name}.rs")),
				|path| dir.join(path),
			);
			Some((name, path))
		})
		.collect();
	let main = dir.join("src/main.rs");

	if main.is_file() && !binaries.iter().any(|(_, path)| *path == main) {
		binaries.push((package.to_owned(), main));
	}

	if let Some(index) = binaries.iter().position(|(name, _)| name == package) {
		let (name, path) = binaries.swap_remove(index);

		return Ok((name, Some(path).filter(|path| path.is_file())));
	}

	match binaries.as_slice() {
		[(name, path)] => {
			Ok((
				name.clone(),
				Some(path.clone()).filter(|path| path.is_file()),
			))
		}
		[] => {
			anyhow::bail!(
				"the server package `{package}` has no binary: add src/main.rs, or a [[bin]] in {}",
				manifest.display()
			)
		}
		_ => {
			anyhow::bail!(
				"the server package `{package}` has several binaries and none is called \
				 `{package}`; name the server's [[bin]] `{package}` in {}",
				manifest.display()
			)
		}
	}
}

/// A library's crate name: `[lib] name`, or the package name with `-` as
/// `_`.
fn lib_name(manifest: &Path) -> Result<Option<String>> {
	let Some(document) = read_manifest(manifest)? else {
		return Ok(None);
	};
	let explicit = document
		.get("lib")
		.and_then(|lib| lib.get("name"))
		.and_then(toml::Value::as_str);
	Ok(explicit
		.or_else(|| package_name(&document))
		.map(|name| name.replace('-', "_")))
}

/// The channel a toolchain file pins, when it's a release such as `1.96`
/// or `1.96.0` that the `rust` image has a tag for.
fn toolchain_channel(file: &Path) -> Option<String> {
	let contents = std::fs::read_to_string(file).ok()?;

	let channel = match contents.parse::<toml::Table>() {
		Ok(table) => table.get("toolchain")?.get("channel")?.as_str()?.to_owned(),
		// The legacy `rust-toolchain` file is the bare channel.
		Err(_) => contents.trim().to_owned(),
	};

	let release = !channel.is_empty()
		&& channel
			.split('.')
			.all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()));
	release.then_some(channel)
}

fn read_manifest(manifest: &Path) -> Result<Option<toml::Table>> {
	let Ok(contents) = std::fs::read_to_string(manifest) else {
		return Ok(None);
	};
	contents
		.parse()
		.map(Some)
		.with_context(|| format!("could not parse {}", manifest.display()))
}

fn package_name(document: &toml::Table) -> Option<&str> {
	document
		.get("package")
		.and_then(|package| package.get("name"))
		.and_then(toml::Value::as_str)
}

/// Keeps local builds, databases, and secrets out of the build context.
const DOCKERIGNORE: &str = "\
# Written by `leptoskit generate deploy`. Keeps the build context small, and
# local builds, databases, and secrets out of the image.
.git
**/target
/dist
/.leptoskit
/infra
/.kamal
/android/.gradle
/android/**/build
*.sqlite3*
*.dump
.env
.env.*
";

/// The same policy for a build context that is the whole Cargo workspace:
/// every nested app's state, databases, caches, and environment files stay
/// out, and a Dockerfile-specific ignore file applies from the workspace
/// root.
const WORKSPACE_DOCKERIGNORE: &str = "\
# Written by `leptoskit generate deploy`. Keeps the build context small, and
# local builds, databases, state, and secrets out of the image. Docker reads
# this beside the Dockerfile and applies it from the workspace root.
.git
**/.git
**/target
**/node_modules
**/.leptoskit
**/dist
**/.gradle
**/android/**/build
**/*.sqlite3
**/*.sqlite3-*
**/*.dump
**/.env
**/.env.*
**/.cargo/credentials
**/.cargo/credentials.toml
**/.kamal
**/infra/.terraform
**/infra/*.tfstate*
**/infra/*.plan
**/infra/plan.sha256
**/infra/operation.lock
**/infra/releases.json
**/infra/release.yml
**/infra/.kamal
";

/// The multi-stage `Dockerfile`: build with `leptoskit build --release`,
/// run as an unprivileged user on a read-only filesystem.
#[allow(clippy::too_many_lines)]
fn dockerfile(project: &Project) -> String {
	let Project {
		app,
		binary,
		port,
		output_name,
		platform,
		rust_version,
		..
	} = project;
	let image = project.image();
	let web = *platform == Platform::Web;
	let version = env!("CARGO_PKG_VERSION");
	let build = if project.app_dir.is_empty() {
		format!("Build it from the project root:\n#\n#   docker build -t {image} .\n")
	} else {
		format!(
			"Build it from the workspace root:\n#\n#   {}\n",
			project.docker_build_command()
		)
	};
	let mut out = format!(
		"# syntax=docker/dockerfile:1
#
# The container for {app}, written by `leptoskit generate deploy`. It's yours
# to edit: running the generator again keeps your changes unless you pass
# --force. {build}#
# The first stage runs `leptoskit build --release`, as you would on your
# machine. The second runs the server as an unprivileged user that writes
# nothing to the image's filesystem, so it works with a read-only root
# filesystem.
"
	);

	if project.sqlite().is_some() {
		let _ = writeln!(
			out,
			"#\n# The SQLite database is the one thing it writes, on a volume at {DATA_DIR}."
		);
	}

	let _ = write!(
		out,
		"
# The Rust release that builds the app. `1` is the latest stable; pin one,
# such as 1.96, to build the same way every time.
ARG RUST_VERSION={rust_version}
# The leptoskit CLI that builds the app. Keep it in step with the `leptoskit`
# version in Cargo.toml.
ARG LEPTOSKIT_VERSION={version}
"
	);

	if web {
		let _ = writeln!(
			out,
			"# binaryen's wasm-opt shrinks the release client.\nARG \
			 BINARYEN_VERSION={BINARYEN_VERSION}"
		);
	}

	out.push_str(
		"
FROM rust:${RUST_VERSION}-bookworm AS build
ARG LEPTOSKIT_VERSION
",
	);

	if web {
		out.push_str(
			"ARG BINARYEN_VERSION
RUN curl --fail --silent --show-error --location \\
      \"https://github.com/WebAssembly/binaryen/releases/download/version_${BINARYEN_VERSION}/binaryen-version_${BINARYEN_VERSION}-$(uname -m)-linux.tar.gz\" \\
    | tar -xz -C /opt \\
 && ln -s \"/opt/binaryen-version_${BINARYEN_VERSION}/bin/wasm-opt\" /usr/local/bin/wasm-opt
",
		);
	}

	out.push_str(
		"RUN cargo install leptoskit_cli --version \"${LEPTOSKIT_VERSION}\" --locked
WORKDIR /src
",
	);

	if let Some(file) = &project.toolchain_file {
		let destination = Path::new(file.as_str()).parent().map_or_else(
			|| "./".to_owned(),
			|parent| {
				if parent.as_os_str().is_empty() {
					"./".to_owned()
				} else {
					format!("{}/", path_to_slashes(parent))
				}
			},
		);
		let _ = writeln!(
			out,
			"# The toolchain file picks the toolchain everything below uses.\n{}",
			docker_copy(file.as_str(), &destination)
		);
	}

	if web {
		if project.app_dir.is_empty() {
			out.push_str("RUN rustup target add wasm32-unknown-unknown\n");
		} else {
			let _ = writeln!(
				out,
				"RUN cd {} && rustup target add wasm32-unknown-unknown",
				shell_quote(&project.app_dir)
			);
		}
	}

	if project.app_dir.is_empty() {
		let _ = write!(
			out,
			"COPY . .
# The cache mounts keep downloaded crates and compiled dependencies between
# builds. The server is copied out, since the target directory is a cache.
RUN --mount=type=cache,target=/usr/local/cargo/registry \\
    --mount=type=cache,target=/src/target \\
    --mount=type=cache,target=/src/.leptoskit \\
    leptoskit build --release --platform {platform} \\
 && cp target/release/{binary} /usr/local/bin/{binary}
"
		);
	} else {
		let app = shell_quote(&project.app_dir);
		let target = shell_quote(&project.target_dir);
		let mut mounts = vec!["--mount=type=cache,target=/usr/local/cargo/registry".to_owned()];

		if let Some(mount) = cache_mount(&project.target_dir) {
			mounts.push(mount);
		}

		if let Some(mount) = cache_mount(&format!("/src/{}/.leptoskit", project.app_dir)) {
			mounts.push(mount);
		}

		out.push_str(
			"COPY . .
# The cache mounts keep downloaded crates and compiled dependencies between
# builds. The server is copied out, since the target directory is a cache.
RUN ",
		);

		for mount in mounts {
			let _ = writeln!(out, "{mount} \\");
		}

		let _ = write!(
			out,
			"    cd {app} && CARGO_TARGET_DIR={target} leptoskit build --release --platform \
			 {platform} \\
 && cp {target}/release/{binary} /usr/local/bin/{binary}
"
		);
	}

	out.push_str(
		"
FROM debian:bookworm-slim
# curl answers the health check, and `app` is the unprivileged user the
# server runs as.
RUN apt-get update \\
 && apt-get install --yes --no-install-recommends curl \\
 && rm -rf /var/lib/apt/lists/* \\
 && useradd --system --user-group --uid 10001 --no-create-home app",
	);

	if project.sqlite().is_some() {
		let _ = write!(
			out,
			" \\\n && mkdir {DATA_DIR} \\\n && chown app:app {DATA_DIR}"
		);
	}

	let build_dir = if project.app_dir.is_empty() {
		"/src".to_owned()
	} else {
		format!("/src/{}", project.app_dir)
	};
	let server_copy = docker_copy_from("build", &format!("/usr/local/bin/{binary}"), "/app/server");
	let dist_copy = docker_copy_from("build", &format!("{build_dir}/{DIST_DIR}"), "/app/dist");
	let _ = write!(
		out,
		"
WORKDIR /app
{server_copy}
{dist_copy}
# What `leptoskit serve` sets from leptoskit.toml, for a container: listen on
# every interface, and serve the site from /app/{DIST_DIR}.
ENV LEPTOS_ENV=PROD \\
    LEPTOS_OUTPUT_NAME={output_name} \\
    LEPTOS_SITE_ROOT=/app/{DIST_DIR} \\
    LEPTOS_SITE_PKG_DIR=pkg \\
    LEPTOS_SITE_ADDR=0.0.0.0:{port}
"
	);

	if let Some(database) = project.sqlite() {
		let _ = write!(
			out,
			"# The SQLite database lives on a volume, the one place the server writes.
ENV {}={}
VOLUME {DATA_DIR}
",
			database.url_env,
			project.sqlite_url()
		);
	}

	let secrets = project.runtime_secrets();

	if !secrets.is_empty() {
		out.push_str(
			"# These secrets come from the environment when the container starts, never\n# from \
			 this file:\n",
		);

		for secret in secrets {
			let _ = writeln!(out, "#   {secret}");
		}
	}

	let _ = write!(
		out,
		"USER app
EXPOSE {port}
HEALTHCHECK --interval=10s --timeout=3s --start-period=30s --retries=3 \\
  CMD [\"curl\", \"--fail\", \"--silent\", \"--output\", \"/dev/null\", \"http://127.0.0.1:{port}/healthz\"]
CMD [\"/app/server\"]
"
	);

	out
}

/// `compose.yaml`: the app, read-only, with Postgres or a SQLite volume.
#[allow(clippy::too_many_lines)]
fn compose(project: &Project) -> String {
	let image = project.image();
	let port = project.port;
	let user = project.app.replace('-', "_");
	let mut required = Vec::new();

	if project.postgres().is_some() {
		required.push("POSTGRES_PASSWORD".to_owned());
	}

	required.extend(project.secrets.iter().cloned());
	let command = if project.app_dir.is_empty() {
		"docker compose up --build".to_owned()
	} else {
		format!(
			"docker compose -f {} up --build",
			shell_quote(&project.in_context("compose.yaml"))
		)
	};
	let ignore = if project.app_dir.is_empty() {
		".dockerignore"
	} else {
		"Dockerfile.dockerignore"
	};
	let mut out = format!(
		"# {app} with Docker Compose, written by `leptoskit generate deploy`. It's
# yours to edit: running the generator again keeps your changes unless you
# pass --force.
#
#   {command}
#
",
		app = project.app
	);

	if required.is_empty() {
		let _ = write!(
			out,
			"# Secrets, once the app has some, come from the environment or an `.env`\n# file \
			 beside this one, which {ignore} keeps out of the image. They never\n# go in this \
			 file.\n"
		);
	} else {
		let _ = write!(
			out,
			"# It reads these secrets from the environment, or from an `.env` file\n# beside this \
			 one, which {ignore} keeps out of the image. They never go\n# in this file.\n#\n"
		);

		for secret in &required {
			let _ = writeln!(out, "#   {secret}");
		}
	}

	let build = if project.app_dir.is_empty() {
		"    build: .\n".to_owned()
	} else {
		format!(
			"    build:\n      context: {}\n      dockerfile: {}\n",
			yaml_string(&project.context),
			yaml_string(&project.dockerfile)
		)
	};
	let _ = write!(
		out,
		"
name: {image}

services:
  app:
{build}    image: {image}
    # Only this machine reaches the server: put a proxy that terminates TLS
    # in front of it.
    ports:
      - \"127.0.0.1:{port}:{port}\"
"
	);
	let mut environment = Vec::new();

	if let Some(database) = project.postgres() {
		environment.push(format!(
			"{}: postgres://{user}:${{POSTGRES_PASSWORD}}@db:5432/{user}?sslmode=disable",
			database.url_env
		));
	}

	for secret in &project.secrets {
		environment.push(format!("{secret}: ${{{secret}:?set {secret}}}"));
	}

	if !environment.is_empty() {
		out.push_str("    environment:\n");

		for line in environment {
			let _ = writeln!(out, "      {line}");
		}
	}

	let _ = write!(
		out,
		"    # Nothing in the image changes while it runs{}.
    read_only: true
    tmpfs:
      - /tmp
",
		if project.sqlite().is_some() {
			"; the database lives on a volume"
		} else {
			""
		}
	);

	if project.sqlite().is_some() {
		let _ = write!(out, "    volumes:\n      - data:{DATA_DIR}\n");
	}

	if project.postgres().is_some() {
		out.push_str(
			"    depends_on:
      db:
        condition: service_healthy
",
		);
	}

	let _ = write!(
		out,
		"    # On `docker compose down` the server fails /readyz, waits 5 seconds, and
    # gives the requests in flight 20 more, so this leaves it time to finish.
    stop_grace_period: {STOP_TIMEOUT_SECONDS}s
    restart: unless-stopped
"
	);

	if project.postgres().is_some() {
		let _ = write!(
			out,
			"
  db:
    image: postgres:17
    environment:
      POSTGRES_USER: {user}
      POSTGRES_DB: {user}
      POSTGRES_PASSWORD: ${{POSTGRES_PASSWORD:?set POSTGRES_PASSWORD}}
    volumes:
      - postgres:/var/lib/postgresql/data
    healthcheck:
      test: [\"CMD\", \"pg_isready\", \"--username={user}\", \"--dbname={user}\"]
      interval: 5s
      timeout: 3s
      retries: 10
    restart: unless-stopped

volumes:
  postgres:
"
		);
	} else if project.sqlite().is_some() {
		out.push_str("\nvolumes:\n  data:\n");
	}

	out
}

/// `fly.toml`: checks on `/readyz`, a kill timeout past the grace period,
/// and machines that stay up for long-lived connections.
fn fly(project: &Project) -> String {
	let config = project.in_context("fly.toml");
	let create = if project.app_dir.is_empty() {
		"Create the app from it with `fly launch --copy-config --no-deploy`.".to_owned()
	} else {
		format!(
			"Create it from the workspace root with `fly launch --config {} --copy-config \
			 --no-deploy`.",
			shell_quote(&config)
		)
	};
	let dockerfile = toml::Value::String(project.dockerfile.clone()).to_string();
	let mut out = format!(
		"# {app} on Fly.io, written by `leptoskit generate deploy`. It's yours to
# edit: running the generator again keeps your changes unless you pass
# --force. {create}

app = \"{fly_app}\"

# On SIGTERM the server fails /readyz, waits 5 seconds for the proxy to
# notice, and gives the requests in flight 20 more. Fly kills it after this.
kill_signal = \"SIGTERM\"
kill_timeout = \"{STOP_TIMEOUT_SECONDS}s\"

[build]
  dockerfile = {dockerfile}

[http_service]
  internal_port = {port}
  force_https = true
  # Machines keep running. Fly stops a machine it thinks is idle, and that
  # drops every WebSocket and streaming connection on it, so clients of an
  # app that holds them would disconnect without warning. An app that only
  # answers short requests can scale to zero instead, with
  # auto_stop_machines = \"stop\" and min_machines_running = 0.
  auto_stop_machines = \"off\"
  auto_start_machines = true
  min_machines_running = 1

  # The proxy sends traffic only to machines whose /readyz answers 200.
  [[http_service.checks]]
    grace_period = \"30s\"
    interval = \"10s\"
    timeout = \"2s\"
    method = \"GET\"
    path = \"/readyz\"
"
		app = project.app,
		fly_app = project.app.to_ascii_lowercase().replace('_', "-"),
		port = project.port,
	);

	if project.sqlite().is_some() {
		let _ = write!(
			out,
			"
# The SQLite database lives on this volume. A volume belongs to one machine,
# so run one machine: `fly scale count 1`.
[mounts]
  source = \"data\"
  destination = \"{DATA_DIR}\"
"
		);
	}

	if let Some(database) = project.postgres() {
		let _ = write!(
			out,
			"
# {} comes from `fly postgres attach`, which stores it as a secret.
",
			database.url_env
		);
	}

	out
}

fn docker_next(project: &Project) -> Vec<String> {
	let image = project.image();
	let port = project.port;
	let mut run = format!("docker run --rm --read-only --tmpfs /tmp -p 127.0.0.1:{port}:{port}");

	if project.sqlite().is_some() {
		let _ = write!(run, " -v {image}-data:{DATA_DIR}");
	}

	for secret in project.runtime_secrets() {
		let _ = write!(run, " -e {secret}");
	}

	let _ = write!(run, " {image}");

	let mut steps = if project.app_dir.is_empty() {
		vec![format!(
			"Build the image: `{}`",
			project.docker_build_command()
		)]
	} else {
		vec![format!(
			"Build the image from the workspace root: `{}`",
			project.docker_build_command()
		)]
	};
	let secrets = project.runtime_secrets();

	if !secrets.is_empty() {
		steps.push(format!(
			"Set {} in your shell; `-e NAME` passes each one into the container.",
			secrets.join(", ")
		));
	}

	steps.push(format!("Run it: `{run}`"));
	steps
}

fn compose_next(project: &Project) -> Vec<String> {
	let mut steps = Vec::new();

	if project.postgres().is_some() {
		steps.push(
			"Choose the database password: `export POSTGRES_PASSWORD=\"$(openssl rand -hex \
			 24)\"`, or put it in .env."
				.to_owned(),
		);
	}

	for secret in &project.secrets {
		steps.push(format!("Set {secret}, in your shell or in .env."));
	}

	if project.app_dir.is_empty() {
		steps.push("Start it: `docker compose up --build`".to_owned());
	} else {
		steps.push(format!(
			"Start it from the workspace root: `docker compose -f {} up --build`",
			shell_quote(&project.in_context("compose.yaml"))
		));
	}
	steps
}

fn fly_next(project: &Project) -> Vec<String> {
	let config = project.in_context("fly.toml");
	let dockerfile = project.dockerfile.clone();
	let workspace = !project.app_dir.is_empty();
	let config_arg = if workspace {
		format!(" --config {}", shell_quote(&config))
	} else {
		String::new()
	};
	let mut steps = vec![if workspace {
		format!(
			"Create the app from fly.toml: `fly launch --config {} --copy-config --no-deploy` \
			 (from the workspace root)",
			shell_quote(&config)
		)
	} else {
		"Create the app from fly.toml: `fly launch --copy-config --no-deploy`".to_owned()
	}];

	if project.sqlite().is_some() {
		steps.push(format!(
			"Create the database's volume: `fly volumes create data --size 1{config_arg}`"
		));
	}

	if let Some(database) = project.postgres() {
		let variable = if database.url_env == "DATABASE_URL" {
			String::new()
		} else {
			format!(" --variable-name {}", database.url_env)
		};

		steps.push(format!(
			"Give it a database: `fly postgres create`, then `fly postgres attach \
			 <name>{variable}` sets {}.",
			database.url_env
		));
	}

	if !project.secrets.is_empty() {
		let pairs: Vec<String> = project
			.secrets
			.iter()
			.map(|secret| format!("{secret}=..."))
			.collect();
		steps.push(format!(
			"Set the secrets: `fly secrets set {}{config_arg}`",
			pairs.join(" ")
		));
	}

	steps.push(if workspace {
		format!(
			"Deploy: `fly deploy --config {} --dockerfile {}`",
			shell_quote(&config),
			shell_quote(&dockerfile)
		)
	} else {
		"Deploy: `fly deploy`".to_owned()
	});
	steps
}

/// A Kubernetes Deployment, Service, and `PodDisruptionBudget`.
#[expect(
	clippy::too_many_lines,
	reason = "one manifest per resource reads best beside the others"
)]
fn kubernetes(project: &Project) -> String {
	if project.split_roles {
		return kubernetes_split(project);
	}

	let image = project.image();
	let port = project.port;
	let replicas = if project.sqlite().is_some() { 1 } else { 2 };
	let mut out = format!(
		"# {app} on Kubernetes, written by `leptoskit generate deploy`. It's
# yours to edit: running the generator again keeps your changes unless you
# pass --force.
#
#   kubectl apply -f kubernetes.yaml
#
# The Deployment runs {replicas} replica{plural}. On SIGTERM the server fails
# /readyz, waits 5 seconds for the load balancer to stop sending it traffic,
# and gives requests in flight 20 more; the pod's grace period is {STOP_TIMEOUT_SECONDS} seconds.
",
		app = project.app,
		plural = if replicas == 1 { "" } else { "s" },
	);

	if project.sqlite().is_some() {
		out.push_str(
			"#\n# The SQLite database lives on a PersistentVolumeClaim, so the\n# Deployment runs \
			 one replica and replaces it on deploy.\n",
		);
	}

	if !project.runtime_secrets().is_empty() {
		let _ = write!(
			out,
			"#\n# The container reads these from the Secret `{image}`, which you create\n# with \
			 `kubectl create secret generic {image} --from-literal=...`:\n",
		);

		for secret in project.runtime_secrets() {
			let _ = writeln!(out, "#   {secret}");
		}
	}

	let _ = write!(
		out,
		"
apiVersion: apps/v1
kind: Deployment
metadata:
  name: {image}
spec:
  replicas: {replicas}
"
	);

	if project.sqlite().is_some() {
		out.push_str(
			"  strategy:
    type: Recreate
",
		);
	}

	let _ = write!(
		out,
		"  selector:
    matchLabels:
      app: {image}
  template:
    metadata:
      labels:
        app: {image}
    spec:
      terminationGracePeriodSeconds: {STOP_TIMEOUT_SECONDS}
      containers:
        - name: {image}
          image: {image}
          ports:
            - containerPort: {port}
          env:
            - name: LEPTOS_SITE_ADDR
              value: \"0.0.0.0:{port}\"
"
	);

	if !project.runtime_secrets().is_empty() {
		let _ = write!(
			out,
			"          envFrom:
            - secretRef:
                name: {image}
"
		);
	}

	let _ = write!(
		out,
		"          readinessProbe:
            httpGet:
              path: /readyz
              port: {port}
            initialDelaySeconds: 5
            periodSeconds: 5
          livenessProbe:
            httpGet:
              path: /healthz
              port: {port}
            initialDelaySeconds: 10
            periodSeconds: 10
          lifecycle:
            preStop:
              exec:
                command: [\"sh\", \"-c\", \"sleep 5\"]
          securityContext:
            runAsNonRoot: true
            readOnlyRootFilesystem: true
            allowPrivilegeEscalation: false
"
	);

	if project.sqlite().is_some() {
		let _ = write!(
			out,
			"          volumeMounts:
            - name: data
              mountPath: {DATA_DIR}
      volumes:
        - name: data
          persistentVolumeClaim:
            claimName: {image}-data
---
apiVersion: v1
kind: PersistentVolumeClaim
metadata:
  name: {image}-data
spec:
  accessModes: [ReadWriteOnce]
  resources:
    requests:
      storage: 1Gi
"
		);
	}

	let _ = write!(
		out,
		"---
apiVersion: v1
kind: Service
metadata:
  name: {image}
spec:
  selector:
    app: {image}
  ports:
    - port: 80
      targetPort: {port}
---
apiVersion: policy/v1
kind: PodDisruptionBudget
metadata:
  name: {image}
spec:
  minAvailable: 1
  selector:
    matchLabels:
      app: {image}
"
	);
	out
}

/// The roles `--split-roles` runs, each as its own Deployment: its name,
/// replicas, and what it does.
const SPLIT_ROLES: [(&str, u32, &str); 4] = [
	(
		"web",
		2,
		"pages, server rendering, the API and sign-in, behind the Service",
	),
	("live", 2, "the live sockets, behind the Service"),
	(
		"worker",
		1,
		"background jobs, cron, and the change log's upkeep",
	),
	(
		"capture",
		2,
		"replication capture and the relay's publisher: one works at a time, the other takes over",
	),
];

/// One Deployment per role, the same image in each with `LEPTOSKIT_ROLE`
/// set, with Services and disruption budgets for the two that serve
/// traffic.
fn kubernetes_split(project: &Project) -> String {
	let image = project.image();
	let port = project.port;
	let mut out = format!(
		"# {app} on Kubernetes, one Deployment per role, written by `leptoskit generate
# deploy --split-roles`. It's yours to edit: running the generator again keeps
# your changes unless you pass --force.
#
#   kubectl apply -f kubernetes.yaml
#
# Every Deployment runs the same image, and LEPTOSKIT_ROLE says which part of
# the app it is:
",
		app = project.app,
	);

	for (role, replicas, what) in SPLIT_ROLES {
		let service = match role {
			"web" => format!(" `{image}`"),
			"live" => format!(" `{image}-live`"),
			_ => String::new(),
		};
		let _ = writeln!(
			out,
			"#   {image}-{role}: {replicas} replica{plural}, {what}{service}",
			plural = if replicas == 1 { "" } else { "s" },
		);
	}

	let _ = write!(
		out,
		"#
# Send /live, /live/events and /live/frames to the Service `{image}-live` in
# your ingress, and everything else to `{image}`. With more than one live
# replica, give them the live relay ([rpc.live] relay_url_env, such as
# NATS_URL, naming a NATS server with JetStream), so they take each commit
# from it rather than each reading the database; the capture role publishes
# to it. On SIGTERM a server fails /readyz, waits 5 seconds for traffic to
# move, and drains; the pods' grace period is {STOP_TIMEOUT_SECONDS} seconds.
"
	);

	if !project.runtime_secrets().is_empty() {
		let _ = write!(
			out,
			"#
# The containers read these from the Secret `{image}`, which you create
# with `kubectl create secret generic {image} --from-literal=...`:
",
		);

		for secret in project.runtime_secrets() {
			let _ = writeln!(out, "#   {secret}");
		}
	}

	for (role, replicas, _) in SPLIT_ROLES {
		let name = format!("{image}-{role}");
		let _ = write!(
			out,
			"---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: {name}
spec:
  replicas: {replicas}
  selector:
    matchLabels:
      app: {name}
  template:
    metadata:
      labels:
        app: {name}
    spec:
      terminationGracePeriodSeconds: {STOP_TIMEOUT_SECONDS}
      containers:
        - name: {image}
          image: {image}
          ports:
            - containerPort: {port}
          env:
            - name: LEPTOS_SITE_ADDR
              value: \"0.0.0.0:{port}\"
            - name: LEPTOSKIT_ROLE
              value: {role}
"
		);

		if !project.runtime_secrets().is_empty() {
			let _ = write!(
				out,
				"          envFrom:
            - secretRef:
                name: {image}
"
			);
		}

		let _ = write!(
			out,
			"          readinessProbe:
            httpGet:
              path: /readyz
              port: {port}
            initialDelaySeconds: 5
            periodSeconds: 5
          livenessProbe:
            httpGet:
              path: /healthz
              port: {port}
            initialDelaySeconds: 10
            periodSeconds: 10
          lifecycle:
            preStop:
              exec:
                command: [\"sh\", \"-c\", \"sleep 5\"]
          securityContext:
            runAsNonRoot: true
            readOnlyRootFilesystem: true
            allowPrivilegeEscalation: false
"
		);
	}

	for (service, role) in [(image.clone(), "web"), (format!("{image}-live"), "live")] {
		let _ = write!(
			out,
			"---
apiVersion: v1
kind: Service
metadata:
  name: {service}
spec:
  selector:
    app: {image}-{role}
  ports:
    - port: 80
      targetPort: {port}
---
apiVersion: policy/v1
kind: PodDisruptionBudget
metadata:
  name: {image}-{role}
spec:
  minAvailable: 1
  selector:
    matchLabels:
      app: {image}-{role}
"
		);
	}

	out
}

/// A systemd service that runs the server as a hardened unit.
fn systemd(project: &Project) -> String {
	let mut out = format!(
		"# {app} under systemd, written by `leptoskit generate deploy`. It's
# yours to edit: running the generator again keeps your changes unless you
# pass --force.
#
#   sudo cp {app}.service /etc/systemd/system/
#   sudo systemctl daemon-reload
#   sudo systemctl enable --now {app}
#
# It reads the environment from /etc/{app}/{app}.env, which is where the
# secrets go. The binary and the built site are installed by your deploy;
# the paths below match /usr/local/bin and /srv/{app}.
",
		app = project.app,
	);
	let _ = write!(
		out,
		"
[Unit]
Description={app}
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User={app}
Group={app}
WorkingDirectory=/srv/{app}
EnvironmentFile=/etc/{app}/{app}.env
Environment=LEPTOS_SITE_ADDR=0.0.0.0:{port}
Environment=LEPTOS_SITE_ROOT=/srv/{app}/{DIST_DIR}
",
		app = project.app,
		port = project.port,
	);

	if project.sqlite().is_some() {
		let _ = writeln!(
			out,
			"Environment={url_env}=sqlite:///var/lib/{app}/{app}.sqlite3",
			url_env = project.sqlite().unwrap().url_env,
			app = project.app
		);
		out.push_str("StateDirectory=");
		out.push_str(&project.app);
		out.push('\n');
	}

	let _ = write!(
		out,
		"ExecStart=/usr/local/bin/{binary}
Restart=on-failure
RestartSec=5
KillSignal=SIGTERM
TimeoutStopSec={STOP_TIMEOUT_SECONDS}

# A read-only filesystem, except for the paths systemd gives the service.
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true
PrivateDevices=true
ProtectKernelTunables=true
ProtectKernelModules=true
ProtectControlGroups=true
RestrictSUIDSGID=true
",
		binary = project.binary,
	);

	if project.sqlite().is_some() {
		let _ = writeln!(out, "ReadWritePaths=/var/lib/{}", project.app);
	}

	out.push_str(
		"
[Install]
WantedBy=multi-user.target
",
	);
	out
}

fn kubernetes_next(project: &Project) -> Vec<String> {
	let image = project.image();
	let mut steps = vec![
		if project.app_dir.is_empty() {
			format!(
				"Build and push the image: `{}` then push it to your registry.",
				project.docker_build_command()
			)
		} else {
			format!(
				"Build and push the image from the workspace root: `{}` then push it to your \
				 registry.",
				project.docker_build_command()
			)
		},
		"Set image to the pushed reference in kubernetes.yaml.".to_owned(),
	];

	if !project.runtime_secrets().is_empty() {
		steps.push(format!(
			"Create the Secret: `kubectl create secret generic {image} --from-literal=NAME=...`."
		));
	}

	if project.split_roles {
		steps.push(format!(
			"Route /live, /live/events and /live/frames to the Service `{image}-live` in your \
			 ingress, and everything else to `{image}`."
		));
	}

	steps.push("Apply it: `kubectl apply -f kubernetes.yaml`".to_owned());
	steps
}

fn systemd_next(project: &Project) -> Vec<String> {
	let mut steps = Vec::new();

	if !project.app_dir.is_empty() {
		steps.push(format!(
			"Build and install from the workspace root: run `leptoskit build --release --platform \
			 {}` in {}.",
			project.platform, project.app_dir
		));
	}

	steps.push("Install the binary at /usr/local/bin and the built site at /srv/<app>.".to_owned());
	steps.push("Put the environment, including secrets, in /etc/<app>/<app>.env.".to_owned());
	steps.push(format!(
		"Install the unit and start it: `sudo cp {}.service /etc/systemd/system/ && sudo \
		 systemctl daemon-reload && sudo systemctl enable --now {}`.",
		project.app, project.app
	));
	steps.push("Create the app's system user, or change User/Group in the unit.".to_owned());
	steps
}

#[cfg(test)]
mod tests;
