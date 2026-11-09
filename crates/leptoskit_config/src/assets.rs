//! Icon and splash asset tables: Flutter-shaped declarations in
//! `leptoskit.toml`, with free-form `extensions` maps for tools that need
//! extra metadata without opening unknown top-level keys.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;

/// Free-form metadata keyed by extension name. Built-in splash and icon
/// generators ignore these tables; process plugins and other tools may read
/// them.
pub type AssetExtensions = BTreeMap<String, toml::Table>;

/// Rich icon declaration, matching [icons_launcher](https://pub.dev/packages/icons_launcher)'s
/// shape. When present, it overrides `app.icon` as the default source image.
///
/// ```toml
/// [app.icons]
/// image = "assets/icon.svg"
/// notification = "assets/notification.png"
///
/// [app.icons.android]
/// adaptive_background = "#FFFFFF"
///
/// [app.icons.extensions.my_tool]
/// flavor = "beta"
/// ```
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields, default)]
pub struct IconsConfig {
	/// Default square PNG or SVG for every platform that does not override it.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub image: Option<PathBuf>,
	/// Android notification / status-bar small icon (white silhouette on
	/// transparent). Also used when `[app.icons.android].notification` is
	/// omitted.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub notification: Option<PathBuf>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub android: Option<AndroidIcons>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub ios: Option<AppleIcons>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub macos: Option<PlatformIcons>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub windows: Option<PlatformIcons>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub linux: Option<PlatformIcons>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub web: Option<WebIcons>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub extension: Option<PlatformIcons>,
	/// Named free-form tables for asset tooling extensions.
	#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
	#[cfg_attr(
		feature = "schema",
		schemars(with = "serde_json::Map<String, serde_json::Value>")
	)]
	pub extensions: AssetExtensions,
}

impl IconsConfig {
	/// A human-readable problem with the icons table, if any.
	#[must_use]
	pub fn problem(&self) -> Option<String> {
		if let Some(android) = &self.android
			&& android.adaptive_background.is_some()
			&& android.adaptive_background_image.is_some()
		{
			return Some(
				"android.adaptive_background and android.adaptive_background_image can't both be \
				 set"
				.into(),
			);
		}

		if let Some(android) = &self.android
			&& let Some(colour) = &android.adaptive_background
			&& crate::launch_colour(colour).is_none()
		{
			return Some(format!(
				"android.adaptive_background `{colour}` must be an opaque hex colour such as \
				 \"#FFFFFF\""
			));
		}

		None
	}

	/// Whether icons should be generated for `platform`.
	#[must_use]
	pub fn enabled(&self, platform: IconPlatform) -> bool {
		match platform {
			IconPlatform::Android => self.android.as_ref().is_none_or(|p| p.enable),
			IconPlatform::Ios => self.ios.as_ref().is_none_or(|p| p.enable),
			IconPlatform::Macos => self.macos.as_ref().is_none_or(|p| p.enable),
			IconPlatform::Windows => self.windows.as_ref().is_none_or(|p| p.enable),
			IconPlatform::Linux => self.linux.as_ref().is_none_or(|p| p.enable),
			IconPlatform::Web => self.web.as_ref().is_none_or(|p| p.enable),
			IconPlatform::Extension => self.extension.as_ref().is_none_or(|p| p.enable),
		}
	}

	/// The image path for `platform`, falling back to `image`.
	#[must_use]
	pub fn image_for(&self, platform: IconPlatform) -> Option<&PathBuf> {
		let override_path = match platform {
			IconPlatform::Android => self.android.as_ref().and_then(|p| p.image.as_ref()),
			IconPlatform::Ios => self.ios.as_ref().and_then(|p| p.image.as_ref()),
			IconPlatform::Macos => self.macos.as_ref().and_then(|p| p.image.as_ref()),
			IconPlatform::Windows => self.windows.as_ref().and_then(|p| p.image.as_ref()),
			IconPlatform::Linux => self.linux.as_ref().and_then(|p| p.image.as_ref()),
			IconPlatform::Web => self.web.as_ref().and_then(|p| p.image.as_ref()),
			IconPlatform::Extension => self.extension.as_ref().and_then(|p| p.image.as_ref()),
		};
		override_path.or(self.image.as_ref())
	}

	/// The Android notification icon path.
	#[must_use]
	pub fn notification_image(&self) -> Option<&PathBuf> {
		self.android
			.as_ref()
			.and_then(|p| p.notification.as_ref())
			.or(self.notification.as_ref())
	}
}

/// Platforms the icon generator writes files for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconPlatform {
	Android,
	Ios,
	Macos,
	Windows,
	Linux,
	Web,
	Extension,
}

/// Shared per-platform enable + image override.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields, default)]
pub struct PlatformIcons {
	/// When false, skip generating this platform's icons.
	#[serde(default = "default_true")]
	pub enable: bool,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub image: Option<PathBuf>,
}

/// Android launcher and notification icons.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields, default)]
pub struct AndroidIcons {
	#[serde(default = "default_true")]
	pub enable: bool,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub image: Option<PathBuf>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub notification: Option<PathBuf>,
	/// Opaque hex fill behind the adaptive foreground.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub adaptive_background: Option<String>,
	/// Image behind the adaptive foreground (mutually exclusive with
	/// `adaptive_background`).
	#[serde(skip_serializing_if = "Option::is_none")]
	pub adaptive_background_image: Option<PathBuf>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub adaptive_foreground: Option<PathBuf>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub adaptive_monochrome: Option<PathBuf>,
}

/// iOS app icons, including iOS 18 dark and tinted appearances.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields, default)]
pub struct AppleIcons {
	#[serde(default = "default_true")]
	pub enable: bool,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub image: Option<PathBuf>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub dark: Option<PathBuf>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub tinted: Option<PathBuf>,
}

/// Web favicon and PWA maskable icon.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields, default)]
pub struct WebIcons {
	#[serde(default = "default_true")]
	pub enable: bool,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub image: Option<PathBuf>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub favicon: Option<PathBuf>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub maskable: Option<PathBuf>,
}

const fn default_true() -> bool {
	true
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn notification_falls_back_from_android_to_root() {
		let mut icons = IconsConfig {
			notification: Some(PathBuf::from("root.png")),
			..IconsConfig::default()
		};
		assert_eq!(icons.notification_image(), Some(&PathBuf::from("root.png")));
		icons.android = Some(AndroidIcons {
			notification: Some(PathBuf::from("android.png")),
			..AndroidIcons::default()
		});
		assert_eq!(
			icons.notification_image(),
			Some(&PathBuf::from("android.png"))
		);
	}

	#[test]
	fn adaptive_background_colour_and_image_conflict() {
		let icons = IconsConfig {
			android: Some(AndroidIcons {
				adaptive_background: Some("#fff".into()),
				adaptive_background_image: Some(PathBuf::from("bg.png")),
				..AndroidIcons::default()
			}),
			..IconsConfig::default()
		};
		assert!(icons.problem().unwrap().contains("can't both"));
	}

	#[test]
	fn enable_defaults_to_true_and_false_disables() {
		let icons = IconsConfig::default();
		assert!(icons.enabled(IconPlatform::Web));
		let icons = IconsConfig {
			web: Some(WebIcons {
				enable: false,
				..WebIcons::default()
			}),
			..IconsConfig::default()
		};
		assert!(!icons.enabled(IconPlatform::Web));
	}
}
