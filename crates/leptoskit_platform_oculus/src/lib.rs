//! <!-- {=oculusPlatformSummary|trim|linePrefix:"//! ":true} -->
//!
//! Experimental Meta Quest / Oculus platform extension: host stubs and the
//! capability manifest. Packaging and `OpenXR` install are not implemented yet.
//!
//! <!-- {/oculusPlatformSummary} -->

#![forbid(unsafe_code)]

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

/// Embedded Quest host templates.
pub mod templates {
	pub const GUEST_RS: &str = include_str!("../templates/guest.rs");
	pub const QUEST_ACTIVITY: &str = include_str!("../templates/QuestActivity.kt");
	pub const README: &str = include_str!("../templates/readme.md");
}

/// The Meta Quest / Oculus platform extension.
#[derive(Debug, Clone, Copy, Default)]
pub struct Oculus;

impl Oculus {
	/// Extension registry id and `hosts/<id>/` directory name. Shipping headset
	/// installs use first-party `quest` packaging; this id is not a
	/// `leptoskit_config::Platform` variant.
	pub const ID: &'static str = "oculus";

	/// Message returned when packaging is requested before it exists.
	#[must_use]
	pub fn packaging_unavailable_message() -> &'static str {
		"Meta Quest / Oculus platform-extension stubs are experimental. For a headset build that \
		 packages the Android host today, use `--platform quest`. OpenXR foreign-headless \
		 packaging through this extension is not implemented yet."
	}
}

impl PlatformExtension for Oculus {
	fn id(&self) -> &'static str {
		Self::ID
	}

	fn display_name(&self) -> &'static str {
		"Meta Quest"
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
				widgets: [
					"view",
					"text",
					"button",
					"pressable",
					"text_input",
					"password_input",
					"scroll_view",
					"image",
				]
				.into_iter()
				.map(|kind| {
					WidgetCapability {
						kind: kind.into(),
						support: "partial".into(),
						note: Some("planned for a foreign-headless Quest panel".into()),
					}
				})
				.collect(),
				plugins: PluginHostCapabilities {
					sync: false,
					worker: false,
					asynchronous: false,
					service: false,
					native: Some("openxr".into()),
				},
				codepush: CodepushSupport::Unsupported,
				text_measurer: "approx".into(),
				packaging: vec!["apk-quest".into()],
			},
			docs_url: Some("https://leptoskit.dev/docs/platforms/oculus.html".into()),
		}
	}

	fn scaffold_files(&self, context: ScaffoldContext<'_>) -> Vec<ScaffoldFile> {
		let readme = templates::README
			.replace("__PACKAGE_ID__", context.package_id)
			.replace("__APP_NAME__", context.app_name);
		let activity = templates::QUEST_ACTIVITY.replace("__PACKAGE_ID__", context.package_id);

		vec![
			ScaffoldFile {
				relative_path: "readme.md".into(),
				contents: readme,
			},
			ScaffoldFile {
				relative_path: "android/QuestActivity.kt".into(),
				contents: activity,
			},
			ScaffoldFile {
				relative_path: "guest.rs".into(),
				contents: templates::GUEST_RS.into(),
			},
		]
	}
}

#[cfg(test)]
mod tests {
	use leptoskit_platform::PlatformExtension;

	use super::*;

	#[test]
	fn manifest_is_valid_and_experimental() {
		let manifest = Oculus.manifest();
		manifest.validate().unwrap();
		assert!(manifest.experimental);
		assert_eq!(manifest.id, "oculus");
	}

	#[test]
	fn scaffold_writes_quest_activity_and_guest() {
		let files = Oculus.scaffold_files(ScaffoldContext {
			app_name: "Arena",
			package_id: "com.example.arena",
			version: "0.1.0",
		});
		assert!(files.iter().any(|file| {
			file.relative_path == "android/QuestActivity.kt"
				&& file.contents.contains("com.example.arena")
				&& file.contents.contains("experimental-stub")
		}));
		assert!(files.iter().any(|file| file.relative_path == "guest.rs"));
		assert!(
			files.iter().any(|file| {
				file.relative_path == "readme.md" && file.contents.contains("Arena")
			})
		);
	}
}
