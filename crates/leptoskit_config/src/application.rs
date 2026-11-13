//! Multiple build and deployment targets within one application root.

use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;

use crate::BuildConfig;
use crate::ConfigError;
use crate::DbConfig;
use crate::LeptoskitConfig;
use crate::NativeConfig;
use crate::Platform;
use crate::PluginSpec;
use crate::RenderMode;
use crate::RouterConfig;
use crate::ServeConfig;
use crate::infra::InfraConfig;

/// Files copied for one frontend. Inputs never change the Cargo project root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields, default)]
pub struct FrontendInputs {
	/// HTML and other web entry files copied to the output root.
	pub web: PathBuf,
	/// Public files copied to the output root.
	pub public: PathBuf,
	/// Assets copied beneath the output's `assets/` directory.
	pub assets: PathBuf,
}

impl Default for FrontendInputs {
	fn default() -> Self {
		Self {
			web: "web".into(),
			public: "public".into(),
			assets: "assets".into(),
		}
	}
}

impl FrontendInputs {
	/// Whether inputs use the traditional root directories.
	pub fn is_default(&self) -> bool {
		self == &Self::default()
	}
}

/// A UI package and its own routes, server, and public inputs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct FrontendConfig {
	/// Workspace package producing the client library.
	pub package: String,
	/// Optional server package for this frontend.
	pub server_package: Option<String>,
	/// Browser rendering mode.
	#[serde(default)]
	pub render: RenderMode,
	/// Frontend-specific asset roots.
	#[serde(default)]
	pub inputs: FrontendInputs,
	/// Frontend-specific file routes.
	pub router: Option<RouterConfig>,
	/// Tooling plugins, in addition to application-wide plugins.
	#[serde(default)]
	pub plugins: Vec<PluginSpec>,
	/// Frontend-specific markup rules.
	pub lint: Option<crate::LintConfig>,
}

/// One named destination. Omitted sidecars mean no sidecars, including for
/// a public frontend in an application that also declares private services.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields, default)]
pub struct TargetConfig {
	/// Destination platform.
	pub platform: Platform,
	/// Frontend key, or no frontend for a server-only target.
	pub frontend: Option<String>,
	/// Explicit build overrides; replaced as a unit when supplied.
	pub build: Option<BuildConfig>,
	/// Native configuration for this destination.
	pub native: Option<NativeConfig>,
	/// Listener settings for this destination.
	pub serve: Option<ServeConfig>,
	/// Database used by this target, independent of other targets.
	pub db: Option<DbConfig>,
	/// Infrastructure for this target's independent deployment.
	pub infra: Option<InfraConfig>,
	/// Keys in the application's existing `[services]` registry.
	pub services: Vec<String>,
	/// A declared destination that is not available yet. Selecting it fails
	/// before generating files or compiling anything, with this explanation.
	pub unavailable: Option<String>,
}

/// Native startup behavior. Neither mode needs a URL or web server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "kebab-case")]
pub enum NativeMode {
	/// Open the main window at launch.
	#[default]
	Window,
	/// Keep a macOS status item alive; create UI only when requested.
	MenuBar,
}

impl NativeMode {
	/// Whether the app opens a window at launch.
	pub fn is_default(&self) -> bool {
		*self == Self::Window
	}
}

fn invalid(message: impl Into<String>) -> ConfigError {
	ConfigError::InvalidApplication(message.into())
}

/// Parse only lint and target-selection fields so unrelated, unfinished
/// application settings do not prevent linting source during editing.
pub(crate) fn lint_config(
	raw: &str,
	path: &Path,
	requested: Option<&str>,
) -> Result<crate::LintConfig, ConfigError> {
	#[derive(Default, Deserialize)]
	#[serde(default)]
	struct App {
		default_target: Option<String>,
	}
	#[derive(Default, Deserialize)]
	#[serde(default)]
	struct Frontend {
		lint: Option<crate::LintConfig>,
	}
	#[derive(Default, Deserialize)]
	#[serde(default)]
	struct Target {
		frontend: Option<String>,
	}
	#[derive(Default, Deserialize)]
	#[serde(default)]
	struct Document {
		lint: crate::LintConfig,
		app: App,
		frontends: BTreeMap<String, Frontend>,
		targets: BTreeMap<String, Target>,
	}
	let document: Document = toml::from_str(raw).map_err(|source| {
		ConfigError::Parse {
			path: path.to_owned(),
			source,
		}
	})?;
	let Some(name) = requested.or(document.app.default_target.as_deref()) else {
		return Ok(document.lint);
	};
	let target = document
		.targets
		.get(name)
		.ok_or_else(|| invalid(format!("unknown target `{name}`")))?;
	let Some(name) = &target.frontend else {
		return Ok(document.lint);
	};
	let frontend = document
		.frontends
		.get(name)
		.ok_or_else(|| invalid(format!("unknown frontend `{name}`")))?;
	Ok(frontend.lint.clone().unwrap_or(document.lint))
}

fn name_valid(name: &str) -> bool {
	!name.is_empty()
		&& name
			.bytes()
			.all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}

impl LeptoskitConfig {
	/// Package that supplies this target's server binary.
	pub fn server_package(&self) -> &str {
		self.build
			.server_package
			.as_deref()
			.unwrap_or(&self.app.name)
	}

	/// Default output, isolated by target. An explicit `--out` still wins.
	pub fn output_directory(&self, requested: &str) -> PathBuf {
		match (&self.selected_target, requested) {
			(Some(target), "dist") => PathBuf::from("dist").join(target),
			_ => requested.into(),
		}
	}

	/// Build records use the target's identity while Cargo keeps one root.
	pub fn artifact_root(&self, root: &Path) -> PathBuf {
		match &self.selected_target {
			Some(target) => root.join(".leptoskit/targets").join(target),
			None => root.to_path_buf(),
		}
	}

	/// Resolves a named destination without moving the application root.
	///
	/// # Errors
	/// Rejects unknown, unavailable, or inconsistent targets.
	pub fn resolve_target(&self, requested: Option<&str>) -> Result<Self, ConfigError> {
		let Some(name) = requested.or(self.app.default_target.as_deref()) else {
			return Ok(self.clone());
		};
		let target = self
			.targets
			.get(name)
			.ok_or_else(|| invalid(format!("unknown target `{name}`")))?;
		if let Some(reason) = &target.unavailable {
			return Err(invalid(format!("target `{name}` is unavailable: {reason}")));
		}
		let config = self.project_target(name, target)?;
		config.validate()?;
		Ok(config)
	}

	fn project_target(&self, name: &str, target: &TargetConfig) -> Result<Self, ConfigError> {
		let mut config = self.clone();
		config.frontends.clear();
		config.targets.clear();
		config.app.default_target = None;
		config.selected_target = Some(name.into());
		config.app.default_platform = target.platform;
		config.app.platforms = vec![target.platform];
		config.db.clone_from(&target.db);
		config.infra.clone_from(&target.infra);
		if let Some(frontend) = &target.frontend {
			let frontend = self.frontends.get(frontend).ok_or_else(|| {
				invalid(format!(
					"targets.{name}.frontend names unknown frontend `{frontend}`"
				))
			})?;
			config.build.client_package = Some(frontend.package.clone());
			config
				.build
				.server_package
				.clone_from(&frontend.server_package);
			config.build.backend = Some(frontend.server_package.is_some());
			config.build.inputs.clone_from(&frontend.inputs);
			config.app.render = frontend.render;
			config.router.clone_from(&frontend.router);
			if let Some(lint) = &frontend.lint {
				config.lint.clone_from(lint);
			}
			config.plugins.extend(frontend.plugins.iter().cloned());
		} else {
			config.build.client_package = None;
			config.router = None;
		}
		if let Some(build) = &target.build {
			config.build.clone_from(build);
		}
		if let Some(native) = &target.native {
			config.native.clone_from(native);
		}
		if config.native.package_id.is_empty() {
			config.native.package_id = crate::default_package_id(&config.app.name);
		}
		if let Some(serve) = &target.serve {
			config.serve.clone_from(serve);
		}
		config.services = target
			.services
			.iter()
			.map(|service| {
				self.services
					.get(service)
					.cloned()
					.map(|value| (service.clone(), value))
					.ok_or_else(|| {
						invalid(format!(
							"targets.{name}.services names unknown service `{service}`"
						))
					})
			})
			.collect::<Result<BTreeMap<_, _>, _>>()?;
		Ok(config)
	}

	pub(crate) fn validate_application(&self) -> Result<(), ConfigError> {
		for (name, frontend) in &self.frontends {
			if !name_valid(name) || frontend.package.trim().is_empty() {
				return Err(invalid(format!(
					"frontends.{name} needs a valid name and package"
				)));
			}
			validate_inputs(&frontend.inputs)?;
		}
		validate_inputs(&self.build.inputs)?;
		if let Some(name) = &self.app.default_target
			&& !self.targets.contains_key(name)
		{
			return Err(invalid(format!(
				"app.default_target names unknown target `{name}`"
			)));
		}
		for (name, target) in &self.targets {
			if !name_valid(name) {
				return Err(invalid(format!("invalid target name `{name}`")));
			}
			if !self.app.platforms.contains(&target.platform) {
				return Err(invalid(format!(
					"targets.{name}.platform must be in app.platforms"
				)));
			}
			if target
				.services
				.iter()
				.collect::<std::collections::BTreeSet<_>>()
				.len()
				!= target.services.len()
			{
				return Err(invalid(format!(
					"targets.{name}.services contains duplicates"
				)));
			}
			self.project_target(name, target)?.validate()?;
		}
		if self.native.mode == NativeMode::MenuBar
			&& self.selected_target.is_some()
			&& self.app.default_platform != Platform::Macos
		{
			return Err(invalid(
				"native.mode = menu-bar currently supports macos only",
			));
		}
		Ok(())
	}
}

fn validate_inputs(inputs: &FrontendInputs) -> Result<(), ConfigError> {
	for path in [&inputs.web, &inputs.public, &inputs.assets] {
		if !crate::is_inside_project(path) {
			return Err(invalid(format!(
				"frontend input `{}` must stay inside the project",
				path.display()
			)));
		}
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn lint_selection_keeps_frontend_rules_and_exclusions_together() {
		let raw = r#"
            [app]
            default_target = "desktop"
            editing_an_unrelated_setting = true
            [lint]
            markup = "builder"
            exclude = ["crates/site/**"]
            [frontends.site.lint]
            markup = "view"
            clippy = false
            exclude = ["crates/product/**"]
            [frontends.site.lint.rules]
            component_attribute = "deny"
            [targets.landing]
            frontend = "site"
            [targets.desktop]
            [targets.relay]
        "#;
		let path = Path::new("leptoskit.toml");
		let landing = lint_config(raw, path, Some("landing")).unwrap();
		assert_eq!(landing.markup, crate::MarkupStyle::View);
		assert_eq!(landing.exclude, ["crates/product/**"]);
		assert!(!landing.clippy);
		assert_eq!(landing.rules.component_attribute, crate::LintLevel::Deny);
		let default_landing = raw.replace(
			"default_target = \"desktop\"",
			"default_target = \"landing\"",
		);
		assert_eq!(lint_config(&default_landing, path, None).unwrap(), landing);
		for target in [None, Some("desktop"), Some("relay")] {
			let lint = lint_config(raw, path, target).unwrap();
			assert_eq!(lint.markup, crate::MarkupStyle::Builder);
			assert_eq!(lint.exclude, ["crates/site/**"]);
		}
		assert!(lint_config(raw, path, Some("missing")).is_err());
		assert!(
			lint_config(
				&raw.replace("frontend = \"site\"", "frontend = \"missing\""),
				path,
				Some("landing")
			)
			.is_err()
		);
	}

	const CONFIG: &str = r#"
        [app]
        name = "quant"
        default_target = "landing"
        platforms = ["web", "server", "macos"]
        [frontends.landing]
        package = "landing_app"
        server_package = "landing_server"
        render = "ssr"
        [frontends.landing.inputs]
        assets = "assets/landing"
        [targets.landing]
        frontend = "landing"
        platform = "web"
        [targets.relay]
        platform = "server"
        [targets.relay.build]
        backend = true
        server_package = "relay"
    "#;

	#[test]
	fn target_selection_keeps_identity_and_isolates_frontend_and_server() {
		let root = Path::new("/project");
		let config = LeptoskitConfig::from_toml(CONFIG, root).unwrap();
		let landing = config.resolve_target(None).unwrap();
		let relay = config.resolve_target(Some("relay")).unwrap();
		assert_eq!(landing.app.name, "quant");
		assert_eq!(landing.server_package(), "landing_server");
		assert_eq!(landing.build.inputs.assets, Path::new("assets/landing"));
		assert_eq!(landing.output_directory("dist"), Path::new("dist/landing"));
		assert_eq!(landing.output_directory("custom"), Path::new("custom"));
		assert_eq!(relay.server_package(), "relay");
		assert!(relay.build.client_package.is_none());
		assert_ne!(landing.artifact_root(root), relay.artifact_root(root));
		assert!(landing.services.is_empty());
	}

	#[test]
	fn every_target_is_validated_even_when_not_selected() {
		for suffix in [
			"\n[targets.bad]\nfrontend='missing'",
			"\n[targets.bad]\nservices=['private']",
			"\n[targets.bad]\nservices=['x','x']",
		] {
			assert!(
				LeptoskitConfig::from_toml(&format!("{CONFIG}{suffix}"), Path::new("/project"))
					.is_err()
			);
		}
		assert!(
			LeptoskitConfig::from_toml(
				&CONFIG.replace("assets/landing", "../private"),
				Path::new("/project")
			)
			.is_err()
		);
	}

	#[test]
	fn unavailable_target_fails_before_build() {
		let config = LeptoskitConfig::from_toml(
			&format!("{CONFIG}\n[targets.future]\nunavailable='host not implemented'"),
			Path::new("/project"),
		)
		.unwrap();
		assert!(
			config
				.resolve_target(Some("future"))
				.unwrap_err()
				.to_string()
				.contains("host not implemented")
		);
		assert!(config.resolve_target(Some("missing")).is_err());
	}
}
