//! In-tree platform extension crates registered with the CLI.
//!
//! Every `native platform` subcommand goes through [`builtin`] / [`find`] so
//! the registry stays live. HarmonyOS and the experimental Oculus stubs
//! register here; first-party `quest` packaging stays outside this table.

use anyhow::Context;
use anyhow::Result;
use clap::Subcommand;
use leptoskit_config::LeptoskitConfig;
use leptoskit_platform::PlatformExtension;
use leptoskit_platform::ScaffoldContext;
use leptoskit_platform_harmonyos::HarmonyOs;
use leptoskit_platform_oculus::Oculus;

/// Builtin platform extensions shipped in this workspace.
#[must_use]
pub fn builtin() -> [&'static dyn PlatformExtension; 2] {
	[&HarmonyOs, &Oculus]
}

/// Looks up a builtin extension by destination id.
#[must_use]
pub fn find(id: &str) -> Option<&'static dyn PlatformExtension> {
	find_in(&builtin(), id)
}

fn find_in<'a>(
	extensions: &[&'a dyn PlatformExtension],
	id: &str,
) -> Option<&'a dyn PlatformExtension> {
	extensions
		.iter()
		.copied()
		.find(|extension| extension.id() == id)
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
	run_with(&builtin(), command)
}

fn run_with(extensions: &[&dyn PlatformExtension], command: PlatformCommand) -> Result<()> {
	match command {
		PlatformCommand::List => {
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
			let extension = require_in(extensions, &id)?;
			let manifest = extension.manifest();
			manifest.validate().map_err(anyhow::Error::msg)?;
			println!("{}", serde_json::to_string_pretty(&manifest)?);
			Ok(())
		}
		PlatformCommand::Scaffold { id } => {
			let extension = require_in(extensions, &id)?;
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

fn require_in<'a>(
	extensions: &[&'a dyn PlatformExtension],
	id: &str,
) -> Result<&'a dyn PlatformExtension> {
	find_in(extensions, id).with_context(|| {
		let known: Vec<_> = extensions.iter().map(|ext| ext.id()).collect();
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
	fn registers_harmonyos_and_oculus() {
		let extensions = builtin();
		assert_eq!(extensions.len(), 2);
		assert_eq!(find("harmonyos").unwrap().display_name(), "HarmonyOS");
		assert_eq!(find("oculus").unwrap().display_name(), "Meta Quest");
		assert!(find("quest").is_none());
		assert!(find("web").is_none());
		for extension in extensions {
			extension.manifest().validate().unwrap();
		}
	}

	#[test]
	fn require_lists_known_ids() {
		let error = match require_in(&builtin(), "missing") {
			Ok(_) => panic!("expected missing extension"),
			Err(error) => error.to_string(),
		};
		assert!(error.contains("missing"));
		assert!(error.contains("harmonyos"));
		assert!(error.contains("oculus"));
	}

	#[test]
	fn list_and_capabilities_include_oculus() {
		run(PlatformCommand::List).unwrap();
		run(PlatformCommand::Capabilities {
			id: "oculus".into(),
		})
		.unwrap();
	}

	#[test]
	fn scaffold_oculus_writes_host_stubs() {
		let root = std::env::temp_dir().join(format!(
			"leptoskit-oculus-platform-scaffold-{}-{}",
			std::process::id(),
			std::time::SystemTime::now()
				.duration_since(std::time::UNIX_EPOCH)
				.unwrap()
				.as_nanos()
		));
		let _ = std::fs::remove_dir_all(&root);
		std::fs::create_dir_all(&root).unwrap();
		std::fs::write(
			root.join("leptoskit.toml"),
			r#"
[app]
name = "oculus_probe"
"#,
		)
		.unwrap();
		let previous = std::env::current_dir().unwrap();
		std::env::set_current_dir(&root).unwrap();
		let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
			run(PlatformCommand::Scaffold {
				id: "oculus".into(),
			})
			.unwrap();
			assert!(root.join("hosts/oculus/android/QuestActivity.kt").is_file());
			assert!(root.join("hosts/oculus/guest.rs").is_file());
		}));
		let _ = std::env::set_current_dir(previous);
		let _ = std::fs::remove_dir_all(&root);
		if let Err(payload) = result {
			std::panic::resume_unwind(payload);
		}
	}
}
