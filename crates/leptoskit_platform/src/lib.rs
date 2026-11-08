//! <!-- {=platformSummary|trim|linePrefix:"//! ":true} -->
//!
//! Contracts for platform extension crates: a versioned manifest, a capability
//! matrix, and scaffold file lists. HarmonyOS and Oculus implement these so the
//! CLI can grow destinations without baking every OS into `leptoskit_config`.
//!
//! <!-- {/platformSummary} -->
//!
//! # What this is for
//!
//! A **platform extension** owns a destination's host templates, doctor hints,
//! and declared widget/plugin support. It is not a runtime capability plugin
//! (`leptoskit_bridge::Plugin`) and not a build-time `LeptoskitPlugin`.
//!
//! In-tree crates (`leptoskit_platform_harmonyos`, `leptoskit_platform_oculus`)
//! implement [`PlatformExtension`]. Out-of-tree providers will reuse the same
//! [`Manifest`] shape over a CLI sidecar protocol later.

#![forbid(unsafe_code)]

use serde::Deserialize;
use serde::Serialize;

/// Protocol version for manifests and (later) sidecar envelopes.
pub const PROTOCOL: u32 = 1;

/// How the extension draws the guest tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RenderStrategy {
	/// A Rust [`leptoskit_native`] backend drives real widgets.
	RustBackend,
	/// `HeadlessBackend` records mutations; a foreign UI applies transactions.
	ForeignHeadless,
	/// A distinct surface API (watch-like), not a full phone `NativeKind` tree.
	SpecializedSurface,
}

/// Whether code push is available on this shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CodepushSupport {
	Supported,
	Unsupported,
	Experimental,
}

/// Declared support for one widget kind (builtin or custom).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct WidgetCapability {
	/// Widget kind id, lowercase (`view`, `text`, or a custom name).
	pub kind: String,
	/// `full`, `partial`, or `unsupported`.
	pub support: String,
	/// Optional note for gap tables.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub note: Option<String>,
}

/// What the generated host can run for runtime plugins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[allow(clippy::struct_excessive_bools)]
pub struct PluginHostCapabilities {
	pub sync: bool,
	pub worker: bool,
	#[serde(rename = "async")]
	pub asynchronous: bool,
	pub service: bool,
	/// Native plugin language tag (`arkts`, `kotlin`, `openxr`, …), if any.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub native: Option<String>,
}

/// Machine-readable gap matrix for docs and `lk platform capabilities`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct CapabilityMatrix {
	pub widgets: Vec<WidgetCapability>,
	pub plugins: PluginHostCapabilities,
	pub codepush: CodepushSupport,
	/// Text measurement: `native`, `approx`, or `none`.
	pub text_measurer: String,
	/// Packaging artifact labels (`hap`, `apk-quest`, …).
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub packaging: Vec<String>,
}

/// Versioned description of one platform extension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Manifest {
	/// Must equal [`PROTOCOL`] for this crate revision.
	pub protocol: u32,
	/// Destination id (`harmonyos`, `oculus`, …).
	pub id: String,
	pub display_name: String,
	/// Semver of the extension crate.
	pub version: String,
	pub experimental: bool,
	pub render: RenderStrategy,
	pub capabilities: CapabilityMatrix,
	/// Public docs URL when the extension owns its gap page.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub docs_url: Option<String>,
}

impl Manifest {
	/// Validates protocol and non-empty identity fields.
	pub fn validate(&self) -> Result<(), ManifestError> {
		if self.protocol != PROTOCOL {
			return Err(ManifestError::Protocol {
				got: self.protocol,
				expected: PROTOCOL,
			});
		}
		if self.id.is_empty() {
			return Err(ManifestError::EmptyId);
		}
		if self.id.chars().any(|c| !c.is_ascii_lowercase() && c != '-') {
			return Err(ManifestError::InvalidId(self.id.clone()));
		}
		if self.display_name.is_empty() {
			return Err(ManifestError::EmptyDisplayName);
		}
		if self.version.is_empty() {
			return Err(ManifestError::EmptyVersion);
		}
		Ok(())
	}
}

/// Why a manifest was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestError {
	Protocol { got: u32, expected: u32 },
	EmptyId,
	InvalidId(String),
	EmptyDisplayName,
	EmptyVersion,
}

impl std::fmt::Display for ManifestError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Protocol { got, expected } => {
				write!(f, "platform manifest protocol {got} is not {expected}")
			}
			Self::EmptyId => f.write_str("platform manifest id is empty"),
			Self::InvalidId(id) => {
				write!(
					f,
					"platform manifest id `{id}` must be lowercase ASCII with optional hyphens"
				)
			}
			Self::EmptyDisplayName => f.write_str("platform manifest display_name is empty"),
			Self::EmptyVersion => f.write_str("platform manifest version is empty"),
		}
	}
}

impl std::error::Error for ManifestError {}

/// Inputs shared by every scaffold writer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScaffoldContext<'a> {
	pub app_name: &'a str,
	pub package_id: &'a str,
	pub version: &'a str,
}

/// One file the extension wants written under `hosts/<id>/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScaffoldFile {
	/// Path relative to the host directory.
	pub relative_path: String,
	pub contents: String,
}

/// Contract every platform extension crate implements.
pub trait PlatformExtension: Send + Sync {
	/// Stable destination id (`harmonyos`, `oculus`).
	fn id(&self) -> &'static str;

	/// Human-readable name.
	fn display_name(&self) -> &'static str;

	/// Versioned capability and render description.
	fn manifest(&self) -> Manifest;

	/// Host files to create on first scaffold (existing paths are left alone).
	fn scaffold_files(&self, context: ScaffoldContext<'_>) -> Vec<ScaffoldFile>;
}

/// Looks up a builtin extension by id among `extensions`.
pub fn find<'a>(
	extensions: &'a [&'a dyn PlatformExtension],
	id: &str,
) -> Option<&'a dyn PlatformExtension> {
	extensions
		.iter()
		.copied()
		.find(|extension| extension.id() == id)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn sample_manifest() -> Manifest {
		Manifest {
			protocol: PROTOCOL,
			id: "oculus".into(),
			display_name: "Meta Quest".into(),
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
					sync: true,
					worker: false,
					asynchronous: false,
					service: false,
					native: Some("openxr".into()),
				},
				codepush: CodepushSupport::Unsupported,
				text_measurer: "approx".into(),
				packaging: vec!["apk-quest".into()],
			},
			docs_url: None,
		}
	}

	#[test]
	fn accepts_a_well_formed_manifest() {
		sample_manifest().validate().unwrap();
	}

	#[test]
	fn rejects_the_wrong_protocol() {
		let mut manifest = sample_manifest();
		manifest.protocol = 99;
		assert_eq!(
			manifest.validate(),
			Err(ManifestError::Protocol {
				got: 99,
				expected: PROTOCOL
			})
		);
	}

	#[test]
	fn rejects_uppercase_ids() {
		let mut manifest = sample_manifest();
		manifest.id = "MetaQuest".into();
		assert!(matches!(
			manifest.validate(),
			Err(ManifestError::InvalidId(_))
		));
	}

	#[test]
	fn find_returns_the_matching_extension() {
		struct Stub;

		impl PlatformExtension for Stub {
			fn id(&self) -> &'static str {
				"oculus"
			}

			fn display_name(&self) -> &'static str {
				"Meta Quest"
			}

			fn manifest(&self) -> Manifest {
				sample_manifest()
			}

			fn scaffold_files(&self, _: ScaffoldContext<'_>) -> Vec<ScaffoldFile> {
				Vec::new()
			}
		}

		let stub = Stub;
		let extensions: [&dyn PlatformExtension; 1] = [&stub];
		assert!(find(&extensions, "oculus").is_some());
		assert!(find(&extensions, "harmonyos").is_none());
	}

	#[test]
	fn manifest_round_trips_through_json() {
		let manifest = sample_manifest();
		let json = serde_json::to_string(&manifest).unwrap();
		let back: Manifest = serde_json::from_str(&json).unwrap();
		assert_eq!(back, manifest);
	}
}
