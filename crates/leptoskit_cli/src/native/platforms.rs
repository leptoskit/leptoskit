//! In-tree platform extension crates registered with the CLI.
//!
//! Builtins start empty in the primitives layer. Concrete shells (HarmonyOS,
//! Oculus, …) register here in follow-up crates. Every `native platform`
//! subcommand goes through [`builtin`] / [`find`] so the registry stays live.

use anyhow::Context;
use anyhow::Result;
use clap::Subcommand;
use leptoskit_config::LeptoskitConfig;
use leptoskit_platform::PlatformExtension;
use leptoskit_platform::ScaffoldContext;

/// Builtin platform extensions shipped in this workspace.
#[must_use]
pub fn builtin() -> [&'static dyn PlatformExtension; 0] {
	[]
}

/// Looks up a builtin extension by destination id.
#[must_use]
pub fn find(id: &str) -> Option<&'static dyn PlatformExtension> {
	for extension in builtin() {
		if extension.id() == id {
			return Some(extension);
		}
	}
	None
}

/// `leptoskit native platform` subcommands.
#[derive(Debug, Clone, Subcommand)]
pub enum PlatformCommand {
	/// Lists in-tree platform extensions and their destination ids.
	List,
	/// Prints one extension's versioned capability manifest as JSON.
	Capabilities {
		/// Extension id (`harmonyos`, `oculus`, …).
		id: String,
	},
	/// Writes an extension's host stubs under `hosts/<id>/` (existing files
	/// stay).
	Scaffold {
		/// Extension id.
		id: String,
	},
}

/// Dispatches `native platform` through the builtin registry.
pub fn run(command: PlatformCommand) -> Result<()> {
	match command {
		PlatformCommand::List => {
			let extensions = builtin();
			if extensions.is_empty() {
				println!(
					"no in-tree platform extensions registered yet; add a `leptoskit_platform_*` \
					 crate and list it in native/platforms.rs"
				);
				return Ok(());
			}
			for extension in extensions {
				let manifest = extension.manifest();
				manifest.validate().map_err(anyhow::Error::msg)?;
				let experimental = if manifest.experimental {
					"experimental"
				} else {
					"stable"
				};
				println!(
					"{}\t{}\t{}\t{experimental}",
					manifest.id, manifest.display_name, manifest.version
				);
			}
			Ok(())
		}
		PlatformCommand::Capabilities { id } => {
			let extension = require(&id)?;
			let manifest = extension.manifest();
			manifest.validate().map_err(anyhow::Error::msg)?;
			println!("{}", serde_json::to_string_pretty(&manifest)?);
			Ok(())
		}
		PlatformCommand::Scaffold { id } => {
			let extension = require(&id)?;
			let (config, root) = LeptoskitConfig::load()
				.context("platform scaffold needs a leptoskit project (run from the app root)")?;
			let package = if config.native.package_id.is_empty() {
				leptoskit_config::default_package_id(&config.app.name)
			} else {
				config.native.package_id.clone()
			};
			let version = config.app.version.to_string();
			let context = ScaffoldContext {
				app_name: &config.app.name,
				package_id: &package,
				version: &version,
			};
			let host = root.join("hosts").join(extension.id());
			std::fs::create_dir_all(&host)
				.with_context(|| format!("could not create {}", host.display()))?;
			let mut written = 0usize;
			for file in extension.scaffold_files(context) {
				let path = host.join(&file.relative_path);
				if path.exists() {
					continue;
				}
				if let Some(parent) = path.parent() {
					std::fs::create_dir_all(parent)
						.with_context(|| format!("could not create {}", parent.display()))?;
				}
				std::fs::write(&path, file.contents)
					.with_context(|| format!("could not write {}", path.display()))?;
				written += 1;
			}
			println!(
				"scaffolded {} new file(s) for {} under {}",
				written,
				extension.display_name(),
				host.display()
			);
			Ok(())
		}
	}
}

fn require(id: &str) -> Result<&'static dyn PlatformExtension> {
	find(id).with_context(|| {
		let known: Vec<_> = builtin().iter().map(|ext| ext.id()).collect();
		if known.is_empty() {
			format!("unknown platform extension `{id}`; no in-tree extensions are registered yet")
		} else {
			format!(
				"unknown platform extension `{id}`; known: {}",
				known.join(", ")
			)
		}
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn builtins_start_empty() {
		assert!(builtin().is_empty());
		assert!(find("harmonyos").is_none());
		assert!(find("oculus").is_none());
		assert!(find("quest").is_none());
	}

	#[test]
	fn require_names_the_missing_id() {
		let error = match require("missing") {
			Ok(_) => panic!("expected missing extension"),
			Err(error) => error.to_string(),
		};
		assert!(error.contains("missing"));
		assert!(error.contains("no in-tree extensions"));
	}
}
