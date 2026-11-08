//! Experimental Meta Quest / Oculus host scaffolding via
//! `leptoskit_platform_oculus`.

use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use leptoskit_config::LeptoskitConfig;
use leptoskit_platform::PlatformExtension;
use leptoskit_platform::ScaffoldContext;
use leptoskit_platform_oculus::Oculus;

use super::Target;

/// Validates Quest starter requirements.
pub fn validate(config: &LeptoskitConfig) -> Result<()> {
	anyhow::ensure!(
		!config.native.package_id.is_empty(),
		"native.package_id is required for Meta Quest / Oculus starters"
	);
	anyhow::ensure!(
		!config.codepush.enabled,
		"Meta Quest / Oculus does not yet support code push; set codepush.enabled = false"
	);
	Ok(())
}

/// Writes experimental host stubs under `hosts/oculus`.
pub fn scaffold(root: &Path, config: &LeptoskitConfig) -> Result<()> {
	validate(config)?;
	let host = root.join("hosts").join(Oculus::ID);
	let context = ScaffoldContext {
		app_name: &config.app.name,
		package_id: &config.native.package_id,
		version: &config.app.version.to_string(),
	};
	for file in Oculus.scaffold_files(context) {
		write(&host.join(file.relative_path), file.contents)?;
	}
	Ok(())
}

fn write(path: &Path, content: impl AsRef<[u8]>) -> Result<()> {
	let parent = path
		.parent()
		.with_context(|| format!("could not create parent for {}", path.display()))?;
	std::fs::create_dir_all(parent)
		.with_context(|| format!("could not create {}", parent.display()))?;
	std::fs::write(path, content).with_context(|| format!("could not write {}", path.display()))
}

/// Explains that Quest packaging is still experimental.
pub fn build(target: Target) -> Result<()> {
	anyhow::ensure!(
		matches!(target, Target::Oculus),
		"{target:?} is not an Oculus target"
	);
	anyhow::bail!("{}", Oculus::packaging_unavailable_message())
}

#[cfg(test)]
mod tests {
	use leptoskit_config::AppConfig;
	use leptoskit_config::NativeConfig;

	use super::*;

	#[test]
	fn scaffolds_oculus_host_files() {
		let root = std::env::temp_dir().join(format!("leptoskit-oculus-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&root);
		std::fs::create_dir_all(&root).unwrap();
		let config = LeptoskitConfig {
			app: AppConfig {
				name: "Arena".into(),
				..AppConfig::default()
			},
			native: NativeConfig {
				package_id: "com.example.arena".into(),
				..NativeConfig::default()
			},
			..LeptoskitConfig::default()
		};
		scaffold(&root, &config).unwrap();
		assert!(root.join("hosts/oculus/android/QuestActivity.kt").is_file());
		assert!(root.join("hosts/oculus/guest.rs").is_file());
		assert!(
			std::fs::read_to_string(root.join("hosts/oculus/readme.md"))
				.unwrap()
				.contains("Arena")
		);
		let _ = std::fs::remove_dir_all(&root);
	}

	#[test]
	fn build_explains_packaging_is_experimental() {
		let error = build(Target::Oculus).unwrap_err().to_string();
		assert!(error.contains("experimental"));
		assert!(error.contains("OpenXR"));
	}
}
