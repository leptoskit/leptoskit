//! Freeze configuration and descriptor identity, without granting authority.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context as _;
use anyhow::Result;
use anyhow::ensure;
use leptoskit_config::LeptoskitConfig;
use serde::Deserialize;
use serde::Serialize;

use super::MAX_DESCRIPTOR;
use super::MAX_DOCUMENT;
use super::hash;
use super::read;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReleaseSet {
	format_version: u32,
	applications: BTreeMap<String, Target>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct Target {
	version: String,
	descriptor: PathBuf,
	descriptor_sha256: String,
	#[serde(default)]
	setup: bool,
	#[serde(default)]
	maintenance: bool,
	#[serde(default)]
	environment: BTreeMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Plan {
	pub format_version: u32,
	pub workspace_sha256: String,
	pub startup_order: Vec<String>,
	pub applications: BTreeMap<String, Application>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Application {
	pub path: PathBuf,
	pub configuration_sha256: String,
	pub version: String,
	pub descriptor: PathBuf,
	pub descriptor_sha256: String,
	pub setup: bool,
	pub maintenance: bool,
	pub environment: BTreeMap<String, String>,
}

pub(super) fn create(manifest: &Path, releases: ReleaseSet) -> Result<Plan> {
	ensure!(
		releases.format_version == 1,
		"unsupported release-set format_version"
	);
	let root = manifest
		.parent()
		.context("workspace manifest needs parent")?;
	let report = super::super::inspect(manifest)?;
	let mut applications = BTreeMap::new();

	for (name, target) in releases.applications {
		let declared = report
			.applications
			.get(&name)
			.with_context(|| format!("unknown application {name}"))?;
		let config = LeptoskitConfig::load_from(&root.join(&declared.path))?;
		applications.insert(
			name,
			Application {
				path: declared.path.clone(),
				configuration_sha256: hash(&serde_json::to_vec(&config)?),
				version: target.version,
				descriptor: target.descriptor,
				descriptor_sha256: target.descriptor_sha256,
				setup: target.setup,
				maintenance: target.maintenance,
				environment: target.environment,
			},
		);
	}

	let plan = Plan {
		format_version: 1,
		workspace_sha256: hash(&read(manifest, MAX_DOCUMENT)?),
		startup_order: report
			.startup_order
			.into_iter()
			.filter(|name| applications.contains_key(name))
			.collect(),
		applications,
	};
	validate(manifest, &plan)?;
	Ok(plan)
}

pub(super) fn validate(manifest: &Path, plan: &Plan) -> Result<()> {
	ensure!(
		plan.format_version == 1 && !plan.applications.is_empty(),
		"unsupported or empty workspace release plan"
	);
	ensure!(
		hash(&read(manifest, MAX_DOCUMENT)?) == plan.workspace_sha256,
		"workspace changed after planning"
	);
	let report = super::super::inspect(manifest)?;
	let expected: Vec<_> = report
		.startup_order
		.iter()
		.filter(|name| plan.applications.contains_key(*name))
		.cloned()
		.collect();
	ensure!(
		plan.startup_order == expected && expected.len() == plan.applications.len(),
		"plan applications or readiness order differ from workspace"
	);

	for (name, application) in &plan.applications {
		let declared = &report.applications[name];
		ensure!(
			application.path == declared.path,
			"application {name} path changed"
		);

		for dependency in &declared.depends_on {
			ensure!(
				plan.applications.contains_key(dependency),
				"application {name} requires {dependency}; include its current verified artifact \
				 so readiness is rechecked"
			);
		}

		let root = manifest.parent().context("workspace needs parent")?;
		let app_root = within(root, &application.path)?;
		let config = configuration(&app_root, &application.configuration_sha256)?;
		variables(application, &config)?;
		let descriptor = within(root, &application.descriptor)?;
		let bytes = read(&descriptor, MAX_DESCRIPTOR)?;
		ensure!(
			hash(&bytes) == application.descriptor_sha256,
			"application {name} descriptor differs from its independent pin"
		);
		crate::infra::preflight_verified_release(
			&app_root,
			&config,
			&application.version,
			&descriptor,
			&application.descriptor_sha256,
		)?;
	}

	ensure!(
		hash(&read(manifest, MAX_DOCUMENT)?) == plan.workspace_sha256,
		"workspace changed during validation"
	);
	Ok(())
}

pub(super) fn unchanged(manifest: &Path, plan: &Plan, name: &str) -> Result<()> {
	ensure!(
		hash(&read(manifest, MAX_DOCUMENT)?) == plan.workspace_sha256,
		"workspace changed after planning"
	);
	let app = &plan.applications[name];
	let root = within(
		manifest.parent().context("workspace needs parent")?,
		&app.path,
	)?;
	configuration(&root, &app.configuration_sha256)?;
	Ok(())
}

fn configuration(root: &Path, pin: &str) -> Result<LeptoskitConfig> {
	let config = LeptoskitConfig::load_from(root)?;
	ensure!(
		hash(&serde_json::to_vec(&config)?) == pin,
		"application configuration changed after planning"
	);
	Ok(config)
}

fn within(root: &Path, path: &Path) -> Result<PathBuf> {
	ensure!(
		!path.as_os_str().is_empty()
			&& path
				.components()
				.all(|part| matches!(part, Component::Normal(_) | Component::CurDir)),
		"release input path must stay relative to the workspace"
	);
	let resolved = root.join(path).canonicalize()?;
	ensure!(
		resolved.starts_with(root),
		"release input resolves outside the workspace"
	);
	Ok(resolved)
}

fn variables(application: &Application, config: &LeptoskitConfig) -> Result<()> {
	let infrastructure = config
		.managed_infra()
		.context("workspace release needs managed infrastructure")?;
	let allowed: BTreeSet<&str> = infrastructure
		.secrets
		.iter()
		.map(String::as_str)
		.chain(config.auth.iter().map(|auth| auth.keys_env.as_str()))
		.chain([
			"HCLOUD_TOKEN",
			"DIGITALOCEAN_TOKEN",
			"CLOUDFLARE_API_TOKEN",
			"AWS_ACCESS_KEY_ID",
			"AWS_SECRET_ACCESS_KEY",
			"KAMAL_REGISTRY_PASSWORD",
			"POSTGRES_PASSWORD",
			"DATABASE_PASSWORD",
			"NATS_TOKEN",
			"VALKEY_PASSWORD",
			"LEPTOSKIT_BACKUP_PASSWORD",
			"LEPTOSKIT_BACKUP_ACCESS_KEY_ID",
			"LEPTOSKIT_BACKUP_SECRET_ACCESS_KEY",
			"LEPTOSKIT_ANALYTICS_READ_KEY",
			"LEPTOSKIT_ANALYTICS_WRITE_KEY",
		])
		.collect();

	for (destination, source) in &application.environment {
		ensure!(
			allowed.contains(destination.as_str())
				&& !reserved(destination)
				&& leptoskit_config::infra::env_name(destination)
				&& leptoskit_config::infra::env_name(source),
			"release environment mappings must reference declared application secrets or managed \
			 provider credentials"
		);
	}

	Ok(())
}

fn reserved(name: &str) -> bool {
	matches!(
		name,
		"PATH" | "HOME" | "SHELL" | "ENV" | "BASH_ENV" | "RUSTUP_HOME" | "CARGO_HOME"
	) || [
		"LD_",
		"DYLD_",
		"PYTHON",
		"GIT_",
		"SSH_",
		"LEPTOSKIT_CONTROL_",
	]
	.iter()
	.any(|prefix| name.starts_with(prefix))
}

pub(super) fn stage(root: &Path, plan: &Plan, destination: &Path) -> Result<()> {
	let mut total = 0_usize;

	for (name, application) in &plan.applications {
		let bytes = read(&within(root, &application.descriptor)?, MAX_DESCRIPTOR)?;
		total = total
			.checked_add(bytes.len())
			.context("descriptor size overflow")?;
		ensure!(
			total <= MAX_DESCRIPTOR,
			"combined workspace descriptors exceed 256 MiB"
		);
		ensure!(
			hash(&bytes) == application.descriptor_sha256,
			"application {name} descriptor changed while staging"
		);
		fs::write(destination.join(format!("{name}.json")), bytes)?;
	}

	Ok(())
}
