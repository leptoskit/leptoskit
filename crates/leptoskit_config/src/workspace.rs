//! Composition of independently configured applications in one repository.
//!
//! This manifest sits above each application's `leptoskit.toml`. Startup
//! dependencies form a graph without cycles; call bindings are separate and
//! may point in both directions. Neither kind of edge grants authority or
//! supplies a secret value.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Component;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;

/// Default workspace composition file.
pub const WORKSPACE_FILE: &str = "leptoskit.workspace.toml";
/// The independently versioned workspace manifest format.
pub const WORKSPACE_VERSION: u32 = 1;

/// Applications with separate configuration, artifacts, and release state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Workspace {
	/// Must be [`WORKSPACE_VERSION`]. Omission never silently selects a format.
	pub format_version: u32,
	/// Stable deployment identities, independent of Cargo package names.
	pub applications: BTreeMap<String, Application>,
}

/// Role of an application in a multi-frontend workspace.
///
/// Scaffold and docs use this to choose platforms and ports. Omission keeps
/// older workspace manifests valid; names like `applications.landing` still
/// work without a `kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum ApplicationKind {
	/// Public marketing site: web only.
	Landing,
	/// Signed-in product: web plus native hosts.
	Product,
	/// Operator console: web only.
	Admin,
	/// API or accounts backend without a browser UI.
	Server,
}

impl ApplicationKind {
	/// The kebab/lowercase spelling written in `leptoskit.workspace.toml`.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Landing => "landing",
			Self::Product => "product",
			Self::Admin => "admin",
			Self::Server => "server",
		}
	}

	/// Whether this role is a browser frontend (not a server-only app).
	#[must_use]
	pub const fn is_web_frontend(self) -> bool {
		matches!(self, Self::Landing | Self::Product | Self::Admin)
	}

	/// Whether scaffold should include native host crates.
	#[must_use]
	pub const fn includes_native(self) -> bool {
		matches!(self, Self::Product)
	}

	/// Parse a frontend list entry (`landing`, `app`/`product`, `admin`).
	///
	/// # Errors
	/// Unknown names.
	pub fn parse_frontend(name: &str) -> Result<Self, String> {
		match name {
			"landing" => Ok(Self::Landing),
			"app" | "product" => Ok(Self::Product),
			"admin" => Ok(Self::Admin),
			other => {
				Err(format!(
					"unknown frontend `{other}`; use landing, app, or admin"
				))
			}
		}
	}

	/// Default workspace application id for this kind.
	#[must_use]
	pub const fn application_id(self) -> &'static str {
		match self {
			Self::Landing => "landing",
			Self::Product => "app",
			Self::Admin => "admin",
			Self::Server => "api",
		}
	}
}

/// One app directory containing its own `leptoskit.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Application {
	/// Directory relative to the workspace manifest. Paths cannot escape it.
	pub path: PathBuf,
	/// Optional role hint for tooling and scaffolds (`landing`, `product`,
	/// `admin`, `server`). Does not change runtime behaviour by itself.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub kind: Option<ApplicationKind>,
	/// Applications that must become ready before this application starts.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub depends_on: Vec<String>,
	/// Calls this app can make. These do not imply startup ordering or trust.
	#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
	pub bindings: BTreeMap<String, Binding>,
}

/// A private endpoint and credential reference owned by one caller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Binding {
	/// The application providing the endpoint.
	pub application: String,
	/// Caller environment variable that will hold the private base URL.
	pub url_env: String,
	/// Caller environment variable that will hold the credential for this
	/// binding. This names a variable; it never contains a credential.
	pub token_env: String,
}

impl Workspace {
	/// Checks identities, references, paths, bindings, and startup ordering.
	///
	/// # Errors
	/// Returns a field-specific diagnostic for an invalid declaration.
	pub fn validate(&self) -> Result<(), String> {
		if self.format_version != WORKSPACE_VERSION {
			return Err(format!(
				"format_version must be {WORKSPACE_VERSION}, found {}",
				self.format_version
			));
		}

		if self.applications.is_empty() || self.applications.len() > 256 {
			return Err("applications must contain between 1 and 256 entries".into());
		}

		for (name, app) in &self.applications {
			self.validate_application(name, app)?;
		}

		self.order().map(|_| ())
	}

	/// Deterministic readiness order. Call bindings never add startup edges.
	///
	/// # Errors
	/// Rejects invalid manifests and startup cycles.
	pub fn startup_order(&self) -> Result<Vec<String>, String> {
		self.validate()?;
		self.order()
	}

	fn validate_application(&self, name: &str, app: &Application) -> Result<(), String> {
		let field = format!("applications.{name}");

		if !identifier(name) {
			return Err(format!(
				"{field}: use a lowercase name with letters, digits, or dashes"
			));
		}

		if app.path.as_os_str().is_empty()
			|| app
				.path
				.components()
				.any(|part| !matches!(part, Component::Normal(_) | Component::CurDir))
		{
			return Err(format!(
				"{field}.path must stay inside the workspace without `..`"
			));
		}
		let mut dependencies = BTreeSet::new();

		for dependency in &app.depends_on {
			self.reference(&format!("{field}.depends-on"), name, dependency)?;

			if !dependencies.insert(dependency) {
				return Err(format!("{field}.depends-on repeats `{dependency}`"));
			}
		}

		let mut variables = BTreeSet::new();

		for (binding_name, binding) in &app.bindings {
			let field = format!("{field}.bindings.{binding_name}");

			if !identifier(binding_name) {
				return Err(format!(
					"{field}: binding names use lowercase letters, digits, or dashes"
				));
			}

			self.reference(&format!("{field}.application"), name, &binding.application)?;
			for (key, variable) in [
				("url-env", &binding.url_env),
				("token-env", &binding.token_env),
			] {
				if !environment_name(variable) {
					return Err(format!(
						"{field}.{key} must name an uppercase environment variable"
					));
				}

				if !variables.insert(variable) {
					return Err(format!(
						"{field}.{key}: `{variable}` is already used by another binding field"
					));
				}
			}
		}

		Ok(())
	}

	fn reference(&self, field: &str, caller: &str, target: &str) -> Result<(), String> {
		if target == caller {
			return Err(format!(
				"{field} cannot refer to its own application `{caller}`"
			));
		}

		if !self.applications.contains_key(target) {
			return Err(format!("{field} names unknown application `{target}`"));
		}

		Ok(())
	}

	fn order(&self) -> Result<Vec<String>, String> {
		let mut pending: BTreeSet<_> = self.applications.keys().cloned().collect();
		let mut ready = BTreeSet::new();
		let mut order = Vec::with_capacity(pending.len());

		while !pending.is_empty() {
			let next = pending.iter().find(|name| {
				self.applications[*name]
					.depends_on
					.iter()
					.all(|dependency| ready.contains(dependency))
			});
			let Some(next) = next.cloned() else {
				return Err(format!(
					"startup dependencies contain a cycle; blocked applications: {}. Use bindings \
					 for calls that do not require startup ordering",
					pending.into_iter().collect::<Vec<_>>().join(", ")
				));
			};
			pending.remove(&next);

			ready.insert(next.clone());
			order.push(next);
		}

		Ok(order)
	}
}

fn identifier(value: &str) -> bool {
	!value.is_empty()
		&& value.len() <= 63
		&& value.as_bytes()[0].is_ascii_lowercase()
		&& value
			.bytes()
			.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn environment_name(value: &str) -> bool {
	let mut bytes = value.bytes();
	bytes
		.next()
		.is_some_and(|byte| byte.is_ascii_uppercase() || byte == b'_')
		&& bytes.all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}
