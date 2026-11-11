//! Portable service evidence. No source reproduction or runtime claim.
use std::collections::BTreeMap;

use anyhow::Context as _;
use anyhow::Result;
use anyhow::ensure;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::services::ResolvedService;
use leptoskit_config::services::ServiceManifest;
use leptoskit_config::services::ServiceSource;
use serde::Deserialize;
use serde::Serialize;

use crate::verify::assets::Root;
use crate::verify::assets::Tree;

pub(crate) mod association;
pub(crate) mod environment;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ServiceEvidence {
	pub(crate) settings: ResolvedService,
	pub(crate) package: String,
	pub(crate) binary: String,
	pub(crate) version: String,
	pub(crate) profile: String,
	pub(crate) default_features: bool,
	pub(crate) package_manifest: String,
	pub(crate) workspace_manifest: String,
	pub(crate) lock: String,
	pub(crate) inputs: Tree,
	pub(crate) sources: BTreeMap<String, String>,
}

pub(crate) fn portable(mut settings: ResolvedService) -> Result<ResolvedService> {
	ensure!(
		settings.secrets.is_empty(),
		"service secret delivery is unsupported; secret values must not enter receipts"
	);
	environment::validate(&settings.env)?;
	match &mut settings.source {
		ServiceSource::Package { .. } => {}
		ServiceSource::Path { path } => *path = "source".into(),
		_ => {
			anyhow::bail!(
				"registry/git/remote service evidence is unsupported: resolver identity alone \
				 does not establish exact build lock inputs"
			)
		}
	}
	Ok(settings)
}

// Exact namespace tracking allows declared public settings without admitting
// unrelated env tables or treating an array member as package metadata.
pub(crate) fn manifest(bytes: &[u8]) -> Result<toml::Value> {
	let value: toml::Value = toml::from_str(std::str::from_utf8(bytes)?)?;
	// Copy exact inputs or fail, never silently redact bytes used by Cargo.
	fn safe(value: &toml::Value, path: &mut Vec<String>) -> bool {
		if environment::metadata_path(path) {
			return environment::metadata(value).is_ok();
		}

		match value {
			toml::Value::Table(table) => {
				table.iter().all(|(key, value)| {
					path.push(key.clone());
					let declared_env = environment::metadata_path(path);
					let accepted = (declared_env
						|| !matches!(key.as_str(), "env" | "secrets" | "token" | "password"))
						&& safe(value, path);
					path.pop();
					accepted
				})
			}
			toml::Value::Array(values) => {
				path.push("[]".into());
				let accepted = values.iter().all(|value| safe(value, path));
				path.pop();
				accepted
			}
			toml::Value::String(value) => {
				!value.starts_with('/')
					&& !value.contains(":\\")
					&& !value.split_once("://").is_some_and(|(_, rest)| {
						rest.split('/')
							.next()
							.is_some_and(|host| host.contains('@'))
					})
			}
			_ => true,
		}
	}
	ensure!(
		safe(&value, &mut Vec::new()),
		"input contains undeclared environment, credentials or absolute paths; portable exact \
		 capture unsupported"
	);
	Ok(value)
}

pub(crate) fn version(package: &toml::Value, workspace: &toml::Value) -> Result<String> {
	let value = package
		.get("package")
		.and_then(|p| p.get("version"))
		.context("package version missing")?;
	let version = if value.get("workspace").and_then(toml::Value::as_bool) == Some(true) {
		workspace
			.get("workspace")
			.and_then(|w| w.get("package"))
			.and_then(|p| p.get("version"))
			.and_then(toml::Value::as_str)
	} else {
		value.as_str()
	};
	Ok(version
		.context("resolved package version missing")?
		.to_owned())
}

impl ServiceEvidence {
	// Compare original and copied input membership before parsing any metadata.
	fn input_bytes(&self, root: &Root, project: &Root) -> Result<BTreeMap<String, Vec<u8>>> {
		ensure!(
			self.sources.keys().eq(self.inputs.keys()),
			"service source membership differs"
		);
		ensure!(
			self.sources
				.values()
				.collect::<std::collections::BTreeSet<_>>()
				.len()
				== self.sources.len(),
			"duplicate service source names"
		);
		let mut bytes = BTreeMap::new();
		let mut total = 0usize;

		for path in self.inputs.keys() {
			let data = root.metadata(path)?;
			ensure!(
				project.metadata(&self.sources[path])? == data,
				"original service input differs from receipt"
			);
			total = total
				.checked_add(data.len())
				.context("input size overflow")?;
			ensure!(
				total <= 16 * 1024 * 1024,
				"service input byte budget exceeded"
			);
			bytes.insert(path.clone(), data);
		}

		Ok(bytes)
	}

	pub(super) fn validate(
		&self,
		root: &Root,
		project: &Root,
		config: &LeptoskitConfig,
		name: &str,
		profile: &str,
		project_path: &std::path::Path,
	) -> Result<()> {
		crate::verify::assets::validate(&self.inputs)?;
		ensure!(
			crate::verify::assets::safe_path(&self.binary) && !self.binary.contains('/'),
			"invalid service binary name"
		);
		ensure!(
			self.settings
				.bin
				.as_ref()
				.is_none_or(|bin| bin == &self.binary),
			"selected service binary differs"
		);
		ensure!(
			self.inputs.len() <= 256 && self.profile == profile && self.default_features,
			"invalid service input count or build profile"
		);
		let bytes = self.input_bytes(root, project)?;
		let package = manifest(
			bytes
				.get(&self.package_manifest)
				.context("package manifest missing")?,
		)?;
		let workspace = manifest(
			bytes
				.get(&self.workspace_manifest)
				.context("workspace manifest missing")?,
		)?;
		ensure!(
			package
				.get("package")
				.and_then(|p| p.get("name"))
				.and_then(toml::Value::as_str)
				== Some(self.package.as_str()),
			"service package differs"
		);
		ensure!(
			version(&package, &workspace)? == self.version,
			"resolved package version differs"
		);
		let lock = manifest(bytes.get(&self.lock).context("dependency lock missing")?)?;
		ensure!(
			lock.get("package")
				.and_then(toml::Value::as_array)
				.is_some_and(|p| !p.is_empty()),
			"missing or empty dependency lock"
		);
		ensure!(
			workspace.get("workspace").is_some()
				|| self.workspace_manifest == self.package_manifest,
			"invalid workspace input"
		);
		let declared: Option<ServiceManifest> = package
			.get("package")
			.and_then(|p| p.get("metadata"))
			.and_then(|p| p.get("leptoskit"))
			.and_then(|p| p.get("service"))
			.cloned()
			.map(toml::Value::try_into)
			.transpose()?;

		if let ServiceSource::Package { name } = &self.settings.source {
			ensure!(
				name == &self.package,
				"selected package differs from source declaration"
			);
		}

		let settings = ResolvedService::merge(name, &config.services[name], declared.as_ref())
			.map_err(anyhow::Error::msg)?;
		association::check(
			self,
			project_path,
			&settings.source,
			&package,
			&workspace,
			&lock,
		)?;
		ensure!(
			portable(settings)? == self.settings,
			"resolved service settings differ from captured manifest and declaration"
		);
		Ok(())
	}
}
