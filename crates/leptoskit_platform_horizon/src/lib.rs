//! Experimental Meta Horizon platform extension: Horizon Worlds TypeScript
//! Custom UI (`horizon/ui`) host templates, capability manifest, and MHCP
//! offline web-bundle validation.

#![forbid(unsafe_code)]

pub mod bundle;

use leptoskit_platform::CapabilityMatrix;
use leptoskit_platform::CodepushSupport;
use leptoskit_platform::Manifest;
use leptoskit_platform::PROTOCOL;
use leptoskit_platform::PlatformExtension;
use leptoskit_platform::PluginHostCapabilities;
use leptoskit_platform::RenderStrategy;
use leptoskit_platform::ScaffoldContext;
use leptoskit_platform::ScaffoldFile;
use leptoskit_platform::WidgetCapability;

/// Embedded Meta Horizon host and MHCP submission templates.
pub mod templates {
	pub const GUEST_RS: &str = include_str!("../templates/guest.rs");
	pub const HOST_TS: &str = include_str!("../templates/scripts/LeptosKitHost.ts");
	pub const MHCP_BUILD_LOG: &str = include_str!("../templates/mhcp/BUILD_LOG.md");
	pub const MHCP_INDEX_HTML: &str = include_str!("../templates/mhcp/index.html");
	pub const README: &str = include_str!("../templates/readme.md");
	pub const SCENE_TEST_TS: &str = include_str!("../templates/scripts/scene_test.ts");
	pub const SCENE_TS: &str = include_str!("../templates/scripts/scene.ts");
}

/// The Meta Horizon platform extension.
#[derive(Debug, Clone, Copy, Default)]
pub struct Horizon;

impl Horizon {
	/// Extension registry id and `hosts/<id>/` directory name. Shipping Meta
	/// Quest headset APKs use the first-party `quest` destination; this id is
	/// not a `leptoskit_config::Platform` variant.
	pub const ID: &'static str = "horizon";
}

impl PlatformExtension for Horizon {
	fn id(&self) -> &'static str {
		Self::ID
	}

	fn display_name(&self) -> &'static str {
		"Meta Horizon"
	}

	fn manifest(&self) -> Manifest {
		Manifest {
			protocol: PROTOCOL,
			id: Self::ID.into(),
			display_name: self.display_name().into(),
			version: env!("CARGO_PKG_VERSION").into(),
			experimental: true,
			render: RenderStrategy::ForeignHeadless,
			capabilities: CapabilityMatrix {
				widgets: vec![
					widget("view", "full", None),
					widget("text", "full", None),
					widget("button", "full", None),
					widget("pressable", "full", None),
					widget(
						"scroll_view",
						"partial",
						Some("maps to horizon/ui ScrollView without programmatic scroll_offset"),
					),
					widget(
						"image",
						"partial",
						Some("requires registered TextureAsset keys; remote URLs are rejected"),
					),
					widget(
						"text_input",
						"unsupported",
						Some("horizon/ui Custom UI does not expose a native text input widget"),
					),
					widget(
						"password_input",
						"unsupported",
						Some("horizon/ui Custom UI does not expose a native password input widget"),
					),
					widget(
						"vector",
						"unsupported",
						Some("horizon/ui Custom UI has no arbitrary vector path surface"),
					),
					widget(
						"rive",
						"unsupported",
						Some("horizon/ui Custom UI has no native Rive canvas"),
					),
				],
				plugins: PluginHostCapabilities {
					sync: false,
					worker: false,
					asynchronous: false,
					service: false,
					native: Some("horizon-ts".into()),
				},
				codepush: CodepushSupport::Unsupported,
				text_measurer: "approx".into(),
				packaging: vec!["mhcp-zip".into(), "horizon-ts".into()],
			},
			docs_url: Some("https://leptoskit.dev/docs/platforms/horizon.html".into()),
		}
	}

	fn scaffold_files(&self, context: ScaffoldContext<'_>) -> Vec<ScaffoldFile> {
		let expand = |template: &str| {
			template
				.replace("__APP_NAME__", context.app_name)
				.replace("__PACKAGE_ID__", context.package_id)
				.replace("__VERSION__", context.version)
		};

		vec![
			file("readme.md", expand(templates::README)),
			file("guest.rs", templates::GUEST_RS),
			file("scripts/scene.ts", templates::SCENE_TS),
			file("scripts/LeptosKitHost.ts", expand(templates::HOST_TS)),
			file("scripts/scene_test.ts", templates::SCENE_TEST_TS),
			file("mhcp/index.html", expand(templates::MHCP_INDEX_HTML)),
			file("mhcp/BUILD_LOG.md", expand(templates::MHCP_BUILD_LOG)),
		]
	}
}

fn widget(kind: &str, support: &str, note: Option<&str>) -> WidgetCapability {
	WidgetCapability {
		kind: kind.into(),
		support: support.into(),
		note: note.map(str::to_owned),
	}
}

fn file(relative_path: impl Into<String>, contents: impl Into<String>) -> ScaffoldFile {
	ScaffoldFile {
		relative_path: relative_path.into(),
		contents: contents.into(),
	}
}

#[cfg(test)]
mod tests {
	use leptoskit_platform::PlatformExtension;

	use super::*;

	#[test]
	fn manifest_is_valid_and_experimental() {
		let manifest = Horizon.manifest();
		manifest.validate().unwrap();
		assert!(manifest.experimental);
		assert_eq!(manifest.id, "horizon");
		assert_eq!(manifest.display_name, "Meta Horizon");
		assert!(
			manifest
				.capabilities
				.packaging
				.iter()
				.any(|item| item == "mhcp-zip")
		);
	}

	#[test]
	fn scaffold_writes_horizon_ui_scripts_and_mhcp_templates() {
		let files = Horizon.scaffold_files(ScaffoldContext {
			app_name: "Starbase",
			package_id: "com.example.starbase",
			version: "0.2.0",
		});
		assert!(files.iter().any(|file| {
			file.relative_path == "scripts/LeptosKitHost.ts"
				&& file.contents.contains("horizon/ui")
				&& file.contents.contains("Starbase")
		}));
		assert!(files.iter().any(|file| {
			file.relative_path == "scripts/scene.ts" && file.contents.contains("SUPPORTED_WIDGETS")
		}));
		assert!(files.iter().any(|file| {
			file.relative_path == "mhcp/index.html" && file.contents.contains("Starbase")
		}));
		assert!(files.iter().any(|file| {
			file.relative_path == "mhcp/BUILD_LOG.md"
				&& file.contents.contains("com.example.starbase")
				&& file.contents.contains("0.2.0")
		}));
		assert!(files.iter().any(|file| file.relative_path == "guest.rs"));
	}

	#[test]
	fn scaffolded_mhcp_index_passes_offline_validation() {
		let files = Horizon.scaffold_files(ScaffoldContext {
			app_name: "Starbase",
			package_id: "com.example.starbase",
			version: "0.2.0",
		});
		let index = files
			.iter()
			.find(|file| file.relative_path == "mhcp/index.html")
			.expect("mhcp/index.html");
		let report = bundle::validate_files(
			&[("index.html".into(), index.contents.as_bytes().to_vec())],
			bundle::MAX_BUNDLE_BYTES,
		);
		assert!(report.is_valid(), "{report:?}");
		assert!(report.issues.is_empty(), "{report:?}");
	}
}
