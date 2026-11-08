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
	use leptoskit_platform::CapabilityMatrix;
	use leptoskit_platform::CodepushSupport;
	use leptoskit_platform::Manifest;
	use leptoskit_platform::PROTOCOL;
	use leptoskit_platform::PluginHostCapabilities;
	use leptoskit_platform::RenderStrategy;
	use leptoskit_platform::ScaffoldFile;
	use leptoskit_platform::WidgetCapability;

	use super::*;

	struct Probe;

	impl PlatformExtension for Probe {
		fn id(&self) -> &'static str {
			"probe"
		}

		fn display_name(&self) -> &'static str {
			"Probe"
		}

		fn manifest(&self) -> Manifest {
			Manifest {
				protocol: PROTOCOL,
				id: "probe".into(),
				display_name: "Probe".into(),
				version: "0.3.0".into(),
				experimental: true,
				render: RenderStrategy::ForeignHeadless,
				capabilities: CapabilityMatrix {
					widgets: vec![WidgetCapability {
						kind: "view".into(),
						support: "full".into(),
						note: None,
					}],
					plugins: PluginHostCapabilities {
						sync: false,
						worker: false,
						asynchronous: false,
						service: false,
						native: None,
					},
					codepush: CodepushSupport::Unsupported,
					text_measurer: "none".into(),
					packaging: Vec::new(),
				},
				docs_url: None,
			}
		}

		fn scaffold_files(&self, _: ScaffoldContext<'_>) -> Vec<ScaffoldFile> {
			vec![
				ScaffoldFile {
					relative_path: "readme.md".into(),
					contents: "probe\n".into(),
				},
				ScaffoldFile {
					relative_path: "nested/note.txt".into(),
					contents: "nested\n".into(),
				},
			]
		}
	}

	struct StableProbe;

	impl PlatformExtension for StableProbe {
		fn id(&self) -> &'static str {
			"stable-probe"
		}

		fn display_name(&self) -> &'static str {
			"Stable Probe"
		}

		fn manifest(&self) -> Manifest {
			let mut manifest = Probe.manifest();
			manifest.id = "stable-probe".into();
			manifest.display_name = "Stable Probe".into();
			manifest.experimental = false;
			manifest
		}

		fn scaffold_files(&self, context: ScaffoldContext<'_>) -> Vec<ScaffoldFile> {
			Probe.scaffold_files(context)
		}
	}

	#[test]
	fn builtins_start_empty() {
		assert!(builtin().is_empty());
		assert!(find("harmonyos").is_none());
		assert!(find("oculus").is_none());
		assert!(find("quest").is_none());
	}

	#[test]
	fn find_in_returns_a_matching_extension() {
		let extensions: [&dyn PlatformExtension; 1] = [&Probe];
		assert_eq!(
			find_in(&extensions, "probe").unwrap().display_name(),
			"Probe"
		);
		assert!(find_in(&extensions, "missing").is_none());
	}

	#[test]
	fn require_names_the_missing_id_when_empty() {
		let error = match require_in(&[], "missing") {
			Ok(_) => panic!("expected missing extension"),
			Err(error) => error.to_string(),
		};
		assert!(error.contains("missing"));
		assert!(error.contains("no in-tree extensions"));
	}

	#[test]
	fn require_lists_known_ids() {
		let extensions: [&dyn PlatformExtension; 1] = [&Probe];
		let error = match require_in(&extensions, "missing") {
			Ok(_) => panic!("expected missing extension"),
			Err(error) => error.to_string(),
		};
		assert!(error.contains("missing"));
		assert!(error.contains("probe"));
	}

	#[test]
	fn list_reports_when_empty() {
		run_with(&[], PlatformCommand::List).unwrap();
	}

	#[test]
	fn list_prints_registered_extensions() {
		let extensions: [&dyn PlatformExtension; 2] = [&Probe, &StableProbe];
		run_with(&extensions, PlatformCommand::List).unwrap();
	}

	#[test]
	fn capabilities_prints_json_for_a_known_id() {
		let extensions: [&dyn PlatformExtension; 1] = [&Probe];
		run_with(
			&extensions,
			PlatformCommand::Capabilities { id: "probe".into() },
		)
		.unwrap();
	}

	#[test]
	fn capabilities_rejects_unknown_ids() {
		let error = run_with(
			&[],
			PlatformCommand::Capabilities {
				id: "missing".into(),
			},
		)
		.unwrap_err()
		.to_string();
		assert!(error.contains("missing"));
	}

	fn with_project(toml: &str, body: impl FnOnce(&std::path::Path)) {
		let root = std::env::temp_dir().join(format!(
			"leptoskit-platform-scaffold-{}-{}",
			std::process::id(),
			std::time::SystemTime::now()
				.duration_since(std::time::UNIX_EPOCH)
				.unwrap()
				.as_nanos()
		));
		let _ = std::fs::remove_dir_all(&root);
		std::fs::create_dir_all(&root).unwrap();
		std::fs::write(root.join("leptoskit.toml"), toml).unwrap();
		let previous = std::env::current_dir().unwrap();
		std::env::set_current_dir(&root).unwrap();
		let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| body(&root)));
		let _ = std::env::set_current_dir(previous);
		let _ = std::fs::remove_dir_all(&root);
		if let Err(payload) = result {
			std::panic::resume_unwind(payload);
		}
	}

	#[test]
	fn scaffold_writes_new_files_and_skips_existing() {
		with_project(
			r#"
[app]
name = "probe_app"
"#,
			|root| {
				let extensions: [&dyn PlatformExtension; 1] = [&Probe];
				run_with(
					&extensions,
					PlatformCommand::Scaffold { id: "probe".into() },
				)
				.unwrap();
				assert!(root.join("hosts/probe/readme.md").is_file());
				assert!(root.join("hosts/probe/nested/note.txt").is_file());
				run_with(
					&extensions,
					PlatformCommand::Scaffold { id: "probe".into() },
				)
				.unwrap();
			},
		);
	}

	#[test]
	fn scaffold_uses_an_explicit_package_id() {
		with_project(
			r#"
[app]
name = "probe_app"

[native]
package_id = "com.example.probe"
"#,
			|root| {
				let extensions: [&dyn PlatformExtension; 1] = [&Probe];
				run_with(
					&extensions,
					PlatformCommand::Scaffold { id: "probe".into() },
				)
				.unwrap();
				assert!(root.join("hosts/probe/readme.md").is_file());
			},
		);
	}

	#[test]
	fn native_run_dispatches_platform_list() {
		crate::native::run(crate::native::NativeCommand::Platform {
			command: PlatformCommand::List,
		})
		.unwrap();
	}
}
