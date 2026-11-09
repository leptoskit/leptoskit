//! Reusing a current release requires fresh operator and runtime evidence.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Context as _;
use anyhow::Result;
use anyhow::ensure;
use leptoskit_config::LeptoskitConfig;

use super::Release;
use super::ReleaseArgs;

#[derive(Clone, Copy)]
pub(super) struct Current<'a> {
	pub root: &'a Path,
	pub config: &'a LeptoskitConfig,
	pub dir: &'a Path,
	pub hosts: &'a [String],
	pub history: &'a [Release],
	pub local_hash: &'a str,
	pub private: &'a str,
}

pub(super) fn current(args: &ReleaseArgs, current: Current<'_>) -> Result<bool> {
	let Current {
		root,
		config,
		dir,
		hosts,
		history,
		local_hash,
		private,
	} = current;
	let version = args
		.version
		.as_deref()
		.context("reuse requires --version")?;
	ensure!(
		args.rollback.is_none() && !args.recover && !args.rollback_schema,
		"reuse cannot select rollback or recovery"
	);
	let path = args
		.verified_descriptor
		.as_ref()
		.context("reuse requires a verified descriptor")?;
	let pin = args
		.descriptor_sha256
		.as_deref()
		.context("reuse requires an independent descriptor pin")?;
	let descriptor = super::super::verified::read(path, pin)?;
	descriptor.application(root, config)?;
	let Some(release) = selected(history, version, local_hash)? else {
		return Ok(false);
	};
	let recorded = release
		.evidence
		.as_ref()
		.context("current release has no verified evidence")?;
	let configuration = super::super::digest(&serde_json::to_vec(config)?);
	ensure!(
		release.configuration_sha256.as_deref() == Some(configuration.as_str()),
		"current release configuration changed or predates configuration recording; deploy a new \
		 version"
	);
	ensure!(
		serde_json::to_value(&descriptor)? == serde_json::to_value(recorded)?,
		"requested descriptor differs from the current release"
	);
	let c = config
		.managed_infra()
		.context("missing managed infrastructure")?;
	let phase: serde_json::Value =
		serde_json::from_slice(&super::remote::read(&hosts[0], &c.name, "operation.json")?)?;
	ensure!(
		phase["phase"] == "complete" && phase["version"] == version,
		"current release has unfinished operation state; inspect status and recover explicitly"
	);
	let authorization =
		super::authorize_release(dir, c, args, Some(&descriptor), Some(release), history)?;
	let project = crate::deploy::Project::read(root, config)?;
	ensure!(
		project.has_probes,
		"release reuse requires declared readiness probes"
	);
	ensure!(
		super::super::service_schemas(&project.services)? == release.services,
		"current release's service schemas differ from the local migrations; deploy a new version"
	);
	let window = descriptor.migration_window()?;
	let environment = environment(config, &project, hosts, private, version, window.as_ref())?;
	let image = super::verify_release(
		hosts,
		c,
		version,
		local_hash,
		&release.services,
		Some(release),
	)?;

	for host in hosts {
		descriptor.check_remote(host, &image, local_hash)?;
		ready(host, &c.name, version, project.port, &environment)?;
	}

	if let Some(window) = &window {
		window.verify_running(hosts, &c.name, version)?;
	}

	// Approval expiry and policy changes during observations are not ignored.
	super::super::approvals::authorize(dir, &c.domain, Some(&descriptor), authorization.as_ref())?;
	Ok(true)
}

fn selected<'a>(
	history: &'a [Release],
	version: &str,
	local_hash: &str,
) -> Result<Option<&'a Release>> {
	if !history.iter().any(|release| release.version == version) {
		return Ok(None);
	}

	let current = history.last().context("missing current release")?;
	ensure!(
		current.version == version,
		"reuse only accepts the latest recorded release; select rollback explicitly"
	);
	ensure!(
		current.migrations == local_hash,
		"current release schema differs from the local migrations"
	);
	ensure!(
		current.completed_at.is_some() && current.image_digest.is_some(),
		"current release lacks completed image evidence"
	);
	Ok(Some(current))
}

fn environment(
	config: &LeptoskitConfig,
	project: &crate::deploy::Project,
	hosts: &[String],
	private: &str,
	version: &str,
	window: Option<&super::super::schema_window::Window>,
) -> Result<BTreeMap<String, String>> {
	let c = config.managed_infra().context("missing infrastructure")?;
	let (raw, _) = super::runtime::kamal(c, project, hosts, private)?;
	let raw = super::super::schema_window::configure(&raw, &c.name, window)?;
	let runtime: serde_json::Value = serde_json::from_str(&raw)?;
	let mut environment: BTreeMap<String, String> =
		serde_json::from_value(runtime["env"]["clear"].clone())?;
	let mut resolved = super::secrets(config, private)?;
	resolved.insert("LEPTOSKIT_RELEASE".into(), version.into());

	for name in runtime["env"]["secret"]
		.as_array()
		.context("invalid generated runtime environment")?
	{
		let name = name.as_str().context("invalid generated secret name")?;
		let value = resolved
			.get(name)
			.cloned()
			.or_else(|| std::env::var(name).ok())
			.filter(|value| !value.is_empty())
			.with_context(|| format!("set required application secret {name}"))?;
		environment.insert(name.to_owned(), value);
	}
	Ok(environment)
}

fn ready(
	host: &str,
	service: &str,
	version: &str,
	port: u16,
	environment: &BTreeMap<String, String>,
) -> Result<()> {
	let script = format!(
		"python3 -c {}",
		super::remote::quote(include_str!("readiness.py"))
	);
	let input = serde_json::to_vec(
		&serde_json::json!({"container":format!("{service}-web-{version}"),"service":service,"port":port,"environment":environment}),
	)?;
	let output = super::remote::execute(host, &script, Some(&input))?;
	let result: serde_json::Value = serde_json::from_slice(&output)?;
	ensure!(
		result == serde_json::json!({"ready":true}),
		"current container failed readiness verification"
	);
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	fn release(version: &str) -> Release {
		Release {
			configuration_sha256: None,
			authorization: None,
			evidence: None,
			version: version.into(),
			migrations: "schema".into(),
			schema: Vec::new(),
			image_digest: Some("image".into()),
			completed_at: Some("completed".into()),
			services: BTreeMap::new(),
		}
	}

	#[test]
	fn reuse_selects_only_completed_latest_matching_schema() {
		let mut history = vec![release("old"), release("current")];
		assert!(selected(&history, "new", "schema").unwrap().is_none());
		assert!(selected(&history, "old", "schema").is_err());
		assert!(selected(&history, "current", "different").is_err());
		assert!(selected(&history, "current", "schema").unwrap().is_some());
		history[1].completed_at = None;
		assert!(selected(&history, "current", "schema").is_err());
		history[1].completed_at = Some("completed".into());

		history[1].image_digest = None;
		assert!(selected(&history, "current", "schema").is_err());
	}

	#[test]
	fn actual_readiness_probe_checks_runtime_and_environment() {
		let status = std::process::Command::new("python3")
			.arg(concat!(
				env!("CARGO_MANIFEST_DIR"),
				"/src/infra/releases/readiness_test.py"
			))
			.status()
			.expect("Python 3 is required for managed release readiness tests");
		assert!(status.success());
	}
}
