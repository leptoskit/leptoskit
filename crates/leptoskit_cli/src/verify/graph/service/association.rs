//! Declaration-to-input identity checks, independent of source reproduction.
use std::path::Path;

use anyhow::Context as _;
use anyhow::Result;
use anyhow::ensure;
use leptoskit_config::services::ServiceSource;

use super::ServiceEvidence;

/// Check before portable normalization erases a path declaration. Both capture
/// and consumption use the exact selected manifest and the current project.
pub(crate) fn source(
	declared: &ServiceSource,
	project: &Path,
	selected: &Path,
	package: &str,
) -> Result<()> {
	let project = project.canonicalize()?;
	let selected = selected.canonicalize()?;
	ensure!(
		selected.starts_with(&project),
		"selected service source is outside project"
	);
	ensure!(
		selected.file_name() == Some(std::ffi::OsStr::new("Cargo.toml")),
		"selected service input is not a Cargo manifest"
	);
	match declared {
		ServiceSource::Package { name } => {
			ensure!(
				name == package,
				"selected package differs from source declaration"
			);
		}
		ServiceSource::Path { path } => {
			let expected = project
				.join(path)
				.join("Cargo.toml")
				.canonicalize()
				.context("declared service path has no package manifest")?;
			ensure!(
				expected == selected,
				"captured package manifest differs from declared service path"
			);
		}
		_ => anyhow::bail!("service source association is unsupported"),
	}
	Ok(())
}

pub(super) fn check(
	evidence: &ServiceEvidence,
	project: &Path,
	declared: &ServiceSource,
	package: &toml::Value,
	workspace: &toml::Value,
	lock: &toml::Value,
) -> Result<()> {
	let original = |name: &str| -> Result<&str> {
		let path = evidence
			.sources
			.get(name)
			.context("service input mapping missing")?;
		ensure!(
			crate::verify::assets::safe_path(path),
			"unsafe service source mapping"
		);
		Ok(path)
	};
	let selected = project.join(original(&evidence.package_manifest)?);
	source(declared, project, &selected, &evidence.package)?;
	let root_manifest = project.join(original(&evidence.workspace_manifest)?);
	ensure!(
		root_manifest.file_name() == Some(std::ffi::OsStr::new("Cargo.toml")),
		"workspace input is not a Cargo manifest"
	);
	let root = root_manifest.parent().context("workspace parent missing")?;
	ensure!(
		project.join(original(&evidence.lock)?) == root.join("Cargo.lock"),
		"captured lock is not the selected workspace lock"
	);
	if package.get("workspace").is_some() {
		ensure!(
			selected == root_manifest,
			"package declares its own workspace"
		);
	} else if let Some(explicit) = package.get("package").and_then(|p| p.get("workspace")) {
		let explicit = explicit
			.as_str()
			.context("invalid package workspace path")?;
		let expected = selected
			.parent()
			.context("package parent missing")?
			.join(explicit)
			.join("Cargo.toml");
		ensure!(
			expected.canonicalize()? == root_manifest.canonicalize()?,
			"captured workspace differs from package.workspace"
		);
	} else {
		ensure!(
			selected.starts_with(root),
			"selected package is outside its captured workspace"
		);
	}
	ensure!(
		workspace.get("workspace").is_some() || selected == root_manifest,
		"workspace input does not declare a workspace"
	);
	ensure!(
		lock.get("package")
			.and_then(toml::Value::as_array)
			.is_some_and(|packages| {
				packages.iter().any(|package| {
					package.get("name").and_then(toml::Value::as_str)
						== Some(evidence.package.as_str())
						&& package.get("version").and_then(toml::Value::as_str)
							== Some(evidence.version.as_str())
						&& package.get("source").is_none()
				})
			}),
		"workspace lock does not contain the selected local package/version"
	);
	Ok(())
}

#[cfg(test)]
mod tests;
