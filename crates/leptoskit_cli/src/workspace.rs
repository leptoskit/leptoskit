//! Workspace composition, pre-build diagnostics, and independent releases.
//!
//! This layer references existing app configurations. It does not turn the
//! processes inside one app into independent releases. Inspection does not
//! inspect credentials or infer deployed evidence from declarations. Release
//! execution consumes separately pinned artifacts and per-app deployment
//! checks.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::io::Read as _;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context as _;
use anyhow::Result;
use anyhow::ensure;
use clap::Args;
use clap::Subcommand;
use leptoskit_config::DbDialect;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::Platform;
use leptoskit_config::RenderMode;
use leptoskit_config::workspace::Binding;
use leptoskit_config::workspace::WORKSPACE_FILE;
use leptoskit_config::workspace::Workspace;
use serde::Serialize;

mod releases;

/// Workspace inspection and ordered releases of independently built images.
#[derive(Debug, Subcommand)]
pub enum WorkspaceCommand {
	/// Shows app boundaries, readiness order, call bindings and evidence gaps.
	Explain(WorkspaceArgs),
	/// Rejects invalid graphs, escaping paths and conflicting managed
	/// identities.
	Check(WorkspaceArgs),
	/// Freeze application configurations and independently pinned artifacts.
	Plan(releases::PlanArgs),
	/// Apply an independently pinned plan, revalidating unchanged releases.
	Deploy(releases::DeployArgs),
}

/// Inputs shared by workspace inspection commands.
#[derive(Debug, Args)]
pub struct WorkspaceArgs {
	/// Workspace manifest; otherwise find leptoskit.workspace.toml in this
	/// directory or a parent. Application paths are relative to that file.
	#[arg(long)]
	pub manifest: Option<PathBuf>,
	/// Emit a versioned JSON report without secret values.
	#[arg(long)]
	pub json: bool,
}

/// A declaration report, not a successful release or a deployment plan.
#[derive(Debug, Serialize)]
pub struct Report {
	/// Report format, independent of application configuration versions.
	pub format_version: u32,
	/// Dependency-first order for startup readiness, not call direction.
	pub startup_order: Vec<String>,
	/// Independently configured application boundaries.
	pub applications: BTreeMap<String, ApplicationReport>,
	/// Explicitly distinguishes configuration checks from runtime evidence.
	pub checks: Checks,
}

/// Which checks this command actually performed.
#[derive(Debug, Serialize)]
pub struct Checks {
	/// Graph references, paths, app configurations and managed identity
	/// collisions.
	pub configuration: &'static str,
	/// Exported API and caller compatibility have not been checked here.
	pub api_compatibility: &'static str,
	/// No build or image-content verification is performed.
	pub artifacts: &'static str,
	/// No connection is made to establish physical database ownership.
	pub database_ownership: &'static str,
	/// Environment names do not establish credentials or transport trust.
	pub service_identity: &'static str,
	/// No provider or running process is inspected.
	pub deployment: &'static str,
}

/// Safe, selected fields from one validated app configuration.
#[derive(Debug, Serialize)]
pub struct ApplicationReport {
	/// App directory relative to the workspace manifest.
	pub path: PathBuf,
	/// Optional role from `leptoskit.workspace.toml` (`landing`, `product`, …).
	#[serde(skip_serializing_if = "Option::is_none")]
	pub kind: Option<leptoskit_config::workspace::ApplicationKind>,
	/// The app's Cargo package name.
	pub package: String,
	/// Enabled platform declarations; availability is not inferred here.
	pub platforms: Vec<Platform>,
	/// Browser rendering choice, separate from backend presence.
	pub render: RenderMode,
	/// Explicit backend choice; null still requires Cargo target resolution.
	pub backend: Option<bool>,
	/// Managed image repository, when one is declared.
	pub image: Option<String>,
	/// Declared readiness dependencies.
	pub depends_on: Vec<String>,
	/// Private call bindings, including variable names only.
	pub bindings: BTreeMap<String, Binding>,
	/// This app's database declaration, without a connection string.
	pub database: Option<DatabaseReport>,
}

/// Database intent; the physical database and credentials remain unresolved.
#[derive(Debug, Serialize)]
pub struct DatabaseReport {
	/// SQL dialect of the app's migration set.
	pub dialect: DbDialect,
	/// Migration directory relative to the app, as declared.
	pub migrations: PathBuf,
	/// Environment variable name only, never its value.
	pub url_env: String,
}

/// Loads and checks an explicit workspace without starting tools or services.
///
/// # Errors
/// Rejects invalid declarations, missing app configs, path escapes, repeated
/// app directories, and managed identities that would overwrite one another.
pub fn inspect(manifest: &Path) -> Result<Report> {
	let manifest = manifest
		.canonicalize()
		.context("locate workspace manifest")?;
	let root = manifest
		.parent()
		.context("workspace manifest needs a parent")?;
	let workspace = read_manifest(&manifest)?;
	let startup_order = workspace.startup_order().map_err(anyhow::Error::msg)?;
	let mut applications = BTreeMap::new();
	let mut configs = BTreeSet::new();
	let mut identities = BTreeMap::new();

	for (name, application) in workspace.applications {
		let app_root = root
			.join(&application.path)
			.canonicalize()
			.with_context(|| format!("applications.{name}.path: locate app directory"))?;
		let config_path = app_root
			.join(leptoskit_config::CONFIG_FILE)
			.canonicalize()
			.with_context(|| format!("applications.{name}.path: expected leptoskit.toml"))?;
		ensure!(
			app_root.starts_with(root) && config_path.starts_with(root),
			"applications.{name}.path resolves outside the workspace"
		);
		ensure!(
			configs.insert(config_path),
			"applications.{name}.path repeats another app configuration"
		);
		let config = LeptoskitConfig::load_from(&app_root)
			.with_context(|| format!("applications.{name}: invalid application configuration"))?;
		check_identities(&mut identities, &name, &config)?;
		let image = config
			.infra
			.as_ref()
			.map(|infra| infra.image.clone())
			.filter(|image| !image.is_empty());
		applications.insert(
			name,
			ApplicationReport {
				path: application.path,
				kind: application.kind,
				package: config.app.name,
				platforms: config.app.platforms,
				render: config.app.render,
				backend: config.build.backend,
				image,
				depends_on: application.depends_on,
				bindings: application.bindings,
				database: config.db.map(|db| {
					DatabaseReport {
						dialect: db.dialect,
						migrations: db.migrations,
						url_env: db.url_env,
					}
				}),
			},
		);
	}

	Ok(Report {
		format_version: 1,
		startup_order,
		applications,
		checks: Checks {
			configuration: "passed",
			api_compatibility: "not_checked",
			artifacts: "not_checked",
			database_ownership: "not_checked",
			service_identity: "not_checked",
			deployment: "not_checked",
		},
	})
}

fn read_manifest(path: &Path) -> Result<Workspace> {
	const LIMIT: u64 = 1024 * 1024;
	let mut raw = String::new();
	std::fs::File::open(path)?
		.take(LIMIT + 1)
		.read_to_string(&mut raw)?;
	ensure!(
		raw.len() <= LIMIT as usize,
		"workspace manifest exceeds 1 MiB"
	);
	toml::from_str(&raw).context("parse leptoskit.workspace.toml")
}

fn check_identities(
	seen: &mut BTreeMap<(&'static str, String), String>,
	name: &str,
	config: &LeptoskitConfig,
) -> Result<()> {
	let Some(infra) = &config.infra else {
		return Ok(());
	};
	for (namespace, field, value) in [
		("infra.name", "infra.name", infra.name.clone()),
		("infra.image", "infra.image", infra.image.clone()),
		(
			"hostname",
			"infra.domain",
			infra.domain.to_ascii_lowercase(),
		),
		(
			"hostname",
			"infra.updates_domain",
			infra.updates_domain.to_ascii_lowercase(),
		),
	] {
		if value.is_empty() {
			continue;
		}

		if let Some(previous) =
			seen.insert((namespace, value), format!("applications.{name}.{field}"))
		{
			anyhow::bail!(
				"applications.{name}.{field} is also owned by {previous}; independent releases \
				 need distinct managed identities"
			);
		}
	}
	Ok(())
}

/// Prints a workspace report; success means configuration checks passed only.
///
/// # Errors
/// Returns the same pre-build diagnostics as [`inspect`].
pub fn run(command: WorkspaceCommand) -> Result<()> {
	let command = match command {
		WorkspaceCommand::Plan(args) => return releases::plan(&args),
		WorkspaceCommand::Deploy(args) => return releases::deploy(&args),
		command => command,
	};

	let (options, explain) = match command {
		WorkspaceCommand::Explain(options) => (options, true),
		WorkspaceCommand::Check(options) => (options, false),
		WorkspaceCommand::Plan(_) | WorkspaceCommand::Deploy(_) => unreachable!("handled above"),
	};

	let manifest = match options.manifest {
		Some(path) => path,
		None => find_manifest(&std::env::current_dir()?)?,
	};

	let report = inspect(&manifest)?;

	if options.json {
		println!("{}", serde_json::to_string_pretty(&report)?);
	} else {
		println!(
			"Workspace configuration passed: {} applications",
			report.applications.len()
		);

		println!("Startup order: {}", report.startup_order.join(" -> "));

		if explain {
			for (name, app) in &report.applications {
				println!("  {name}: {} ({})", app.path.display(), app.package);

				for (binding, target) in &app.bindings {
					println!(
						"    {binding} -> {}: {}, {}",
						target.application, target.url_env, target.token_env
					);
				}
			}
		}

		println!(
			"Not checked: artifacts, API compatibility, database ownership, service identity, \
			 deployment."
		);
	}

	Ok(())
}

fn find_manifest(directory: &Path) -> Result<PathBuf> {
	for parent in directory.ancestors() {
		let candidate = parent.join(WORKSPACE_FILE);

		if candidate.is_file() {
			return Ok(candidate);
		}
	}

	anyhow::bail!(
		"no {WORKSPACE_FILE} in this directory or its parents; use --manifest to name one"
	)
}
