//! Projects and their environments.
//!
//! A project is one app; [`create_project`] makes it with a `production`
//! environment, which always exists and can't be deleted. An environment's
//! scale is stored state that [`update_environment`] changes only from the
//! revision it was read at, so two people changing it at once can't
//! silently undo each other.

use std::collections::BTreeMap;

use leptoskit_db::Timestamp;
use leptoskit_rpc as rpc;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use uuid::Uuid;

use crate::size::Scale;
use crate::size::Size;
use crate::size::Sleep;

/// The environment every project has, and nobody may delete.
pub const PRODUCTION: &str = "production";

/// The most targets `create_project` stores.
pub const MAX_TARGETS: usize = 16;

/// The most variable names `create_project` stores.
pub const MAX_VARIABLES: usize = 200;

/// The longest variable name, in bytes.
pub const MAX_VARIABLE_LEN: usize = 128;

/// The most services an environment's scale names.
pub const MAX_SERVICES: usize = 32;

/// What an environment is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, leptoskit_db::Enum)]
pub enum EnvironmentKind {
	/// What people use. Every project has exactly one, named `production`.
	#[db(rename = "production")]
	Production,
	/// A long-lived copy for trying things before production.
	#[db(rename = "staging")]
	Staging,
	/// A short-lived copy, such as one per pull request.
	#[db(rename = "preview")]
	Preview,
}

/// One process's scale: its size, its replicas, and whether it sleeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProcessScale {
	pub size: Size,
	pub min: u32,
	pub max: u32,
	pub sleep: Sleep,
}

impl ProcessScale {
	/// What `[cloud]` gives a process it says nothing about: `xs`, no
	/// replicas while idle, at most one, sleeping when idle.
	#[must_use]
	pub const fn sleeping() -> Self {
		Self {
			size: Size::Xs,
			min: 0,
			max: 1,
			sleep: Sleep::WhenIdle,
		}
	}

	/// Checks the `[cloud]` rules: `min <= max`, `1 <= max <= 20`, and
	/// sleeping exactly when `min = 0`.
	///
	/// # Errors
	///
	/// Which rule it breaks, as `[cloud]` would say it.
	pub fn check(&self) -> Result<(), String> {
		let scale = Scale::new(self.min, self.max).map_err(|error| error.to_string())?;
		crate::size::check_sleep(self.sleep, scale).map_err(|error| error.to_string())
	}
}

/// An environment's scale: the app's, and each service's by name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EnvironmentScale {
	pub app: ProcessScale,
	#[serde(default)]
	pub services: BTreeMap<String, ProcessScale>,
}

impl EnvironmentScale {
	/// What a new environment starts with: the app sleeping when idle, and
	/// no service named.
	#[must_use]
	pub fn sleeping() -> Self {
		Self {
			app: ProcessScale::sleeping(),
			services: BTreeMap::new(),
		}
	}

	/// Checks every process's rules, and the services' names.
	///
	/// # Errors
	///
	/// The first process that breaks one, named.
	pub fn check(&self) -> Result<(), String> {
		self.app.check().map_err(|error| format!("app: {error}"))?;

		if self.services.len() > MAX_SERVICES {
			return Err(format!(
				"an environment scales at most {MAX_SERVICES} services"
			));
		}

		for (name, scale) in &self.services {
			if !is_service_name(name) {
				return Err(format!(
					"`{name}` isn't a service name: lower-case letters, digits, and hyphens, \
					 starting with a letter"
				));
			}

			scale.check().map_err(|error| format!("{name}: {error}"))?;
		}

		Ok(())
	}
}

/// Whether `name` can name a service, as `[services.<name>]` does.
fn is_service_name(name: &str) -> bool {
	let mut characters = name.chars();
	name.len() <= 64
		&& characters
			.next()
			.is_some_and(|first| first.is_ascii_lowercase())
		&& characters.all(|character| {
			character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
		})
}

/// What `lk cloud launch` found in a project: its targets and the names of
/// the variables in its `.env`. Never a value: a name must look like an
/// environment variable's, which no `KEY=value` line does.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Detected {
	/// Such as `server`, `web`, `ios`, or `android`.
	#[serde(default)]
	pub targets: Vec<String>,
	/// Such as `DATABASE_URL`.
	#[serde(default)]
	pub variables: Vec<String>,
}

impl Detected {
	/// Checks the lists and returns them sorted and without repeats.
	///
	/// # Errors
	///
	/// A target or variable name that isn't one, or too many of either.
	pub fn checked(self) -> Result<Self, String> {
		let mut targets = self.targets;
		targets.sort_unstable();
		targets.dedup();
		let mut variables = self.variables;
		variables.sort_unstable();
		variables.dedup();

		if targets.len() > MAX_TARGETS {
			return Err(format!("at most {MAX_TARGETS} targets"));
		}

		if variables.len() > MAX_VARIABLES {
			return Err(format!("at most {MAX_VARIABLES} variable names"));
		}

		if let Some(target) = targets.iter().find(|target| !is_target(target)) {
			return Err(format!(
				"`{target}` isn't a target name: 2 to 32 lower-case letters, digits, and hyphens"
			));
		}

		// A name that isn't one may be a value sent by mistake, so the
		// error never repeats it.
		if !variables.iter().all(|variable| is_variable_name(variable)) {
			return Err(format!(
				"a variable name must be upper-case letters, digits, and underscores, starting \
				 with a letter or underscore, at most {MAX_VARIABLE_LEN} characters: send names, \
				 never values"
			));
		}

		Ok(Self { targets, variables })
	}
}

/// Whether `name` can name a target.
fn is_target(name: &str) -> bool {
	(2..=32).contains(&name.len())
		&& name.starts_with(|first: char| first.is_ascii_lowercase())
		&& name.chars().all(|character| {
			character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
		})
}

/// Whether `name` looks like an environment variable's name.
fn is_variable_name(name: &str) -> bool {
	(1..=MAX_VARIABLE_LEN).contains(&name.len())
		&& name.starts_with(|first: char| first.is_ascii_uppercase() || first == '_')
		&& name.chars().all(|character| {
			character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
		})
}

/// A project, without its environments.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProjectSummary {
	pub id: Uuid,
	pub slug: String,
	pub name: String,
	pub region: String,
	pub created_at: Timestamp,
}

/// An environment and its scale.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EnvironmentSummary {
	pub id: Uuid,
	pub name: String,
	pub kind: EnvironmentKind,
	pub scale: EnvironmentScale,
	/// Pass this back to [`update_environment`]; it changes with every
	/// write.
	pub revision: i64,
	/// Whether the scale was changed here since the last deployment's
	/// `leptoskit.toml`.
	pub scale_differs: bool,
	pub created_at: Timestamp,
	pub updated_at: Timestamp,
}

/// A project with its environments, and what `lk cloud launch` found.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProjectDetail {
	pub project: ProjectSummary,
	pub environments: Vec<EnvironmentSummary>,
	pub detected: Detected,
}

/// A new project, its `production` environment, and what was stored of
/// what `lk cloud launch` found.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CreatedProject {
	pub project: ProjectSummary,
	pub environment: EnvironmentSummary,
	pub detected: Detected,
}

/// The organisation's projects that the caller can see: all of them, or a
/// project-bound token's one.
///
/// # Errors
///
/// `NotFound` for an organisation the caller can't see.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "projects")]
#[rpc::query]
pub async fn projects(cx: &rpc::QueryCx, organization: String) -> rpc::Result<Vec<ProjectSummary>> {
	crate::server::projects::projects(cx, &organization).await
}

/// One project, its environments, and what `lk cloud launch` found.
///
/// # Errors
///
/// `NotFound` for a project the caller can't see.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "projects")]
#[rpc::query]
pub async fn project(
	cx: &rpc::QueryCx,
	organization: String,
	// Named `project` on the wire; the macro's locals would shadow the
	// function with that name here.
	#[serde(rename = "project")] slug: String,
) -> rpc::Result<ProjectDetail> {
	crate::server::projects::project(cx, &organization, &slug).await
}

/// Makes a project and its `production` environment.
///
/// # Arguments
///
/// - `slug`: 3 to 40 lower-case letters, digits, and hyphens, starting with a
///   letter, unique in the organisation.
/// - `name`: what the console shows.
/// - `region`: where it runs; the cloud's default when left out.
/// - `detected`: the targets and `.env` variable names `lk cloud launch` found,
///   to show what's still to do. Names only, never values.
///
/// # Errors
///
/// `Invalid` for a slug, name, region, or detected name that breaks the
/// rules; `Conflict` (`slug_taken`) when the organisation has the slug.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "projects")]
#[rpc::mutation]
pub async fn create_project(
	cx: &rpc::MutationCx,
	organization: String,
	slug: String,
	name: String,
	#[serde(default)] region: Option<String>,
	#[serde(default)] detected: Detected,
) -> rpc::Result<CreatedProject> {
	crate::server::projects::create_project(cx, &organization, slug, name, region, detected).await
}

/// Deletes a project and everything in it. Type its slug back as
/// `confirm`.
///
/// # Errors
///
/// `Invalid` (`confirmation_mismatch`) when `confirm` isn't the slug.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "projects")]
#[rpc::mutation]
pub async fn delete_project(
	cx: &rpc::MutationCx,
	organization: String,
	project: String,
	confirm: String,
) -> rpc::Result<()> {
	crate::server::projects::delete_project(cx, &organization, &project, &confirm).await
}

/// A project's environments, `production` first.
///
/// # Errors
///
/// `NotFound` for a project the caller can't see.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "projects")]
#[rpc::query]
pub async fn environments(
	cx: &rpc::QueryCx,
	organization: String,
	project: String,
) -> rpc::Result<Vec<EnvironmentSummary>> {
	crate::server::projects::environments(cx, &organization, &project).await
}

/// Makes an environment, sleeping when idle until its scale is changed.
///
/// # Arguments
///
/// - `name`: a slug, unique in the project.
/// - `kind`: `staging` or `preview`; a project has one production.
///
/// # Errors
///
/// `Invalid` for a name that breaks the rules or a second production;
/// `Conflict` (`name_taken`) when the project has the name.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "projects")]
#[rpc::mutation]
pub async fn create_environment(
	cx: &rpc::MutationCx,
	organization: String,
	project: String,
	name: String,
	kind: EnvironmentKind,
) -> rpc::Result<EnvironmentSummary> {
	crate::server::projects::create_environment(cx, &organization, &project, name, kind).await
}

/// Changes an environment's scale, from the `revision` it was read at.
///
/// # Errors
///
/// `Conflict` (`stale_revision`) when someone changed it since; `Invalid`
/// for a scale that breaks the `[cloud]` rules.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "projects")]
#[rpc::mutation]
pub async fn update_environment(
	cx: &rpc::MutationCx,
	organization: String,
	project: String,
	environment: String,
	revision: i64,
	scale: EnvironmentScale,
) -> rpc::Result<EnvironmentSummary> {
	crate::server::projects::update_environment(
		cx,
		&organization,
		&project,
		&environment,
		revision,
		scale,
	)
	.await
}

/// Deletes an environment other than `production`. Type its name back as
/// `confirm`.
///
/// # Errors
///
/// `Invalid` (`confirmation_mismatch`) when `confirm` isn't the name;
/// `Forbidden` (`production`) for `production`.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "projects")]
#[rpc::mutation]
pub async fn delete_environment(
	cx: &rpc::MutationCx,
	organization: String,
	project: String,
	environment: String,
	confirm: String,
) -> rpc::Result<()> {
	crate::server::projects::delete_environment(cx, &organization, &project, &environment, &confirm)
		.await
}

/// What an environment runs, as its cluster worker last reported it: each
/// process's ready and desired replicas, whether it sleeps, restarts, and
/// out-of-memory kills, and when the worker was last heard from. Subscribe
/// to it to watch.
///
/// # Errors
///
/// `NotFound` for an environment the caller can't see.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "projects")]
#[rpc::query]
pub async fn environment_status(
	cx: &rpc::QueryCx,
	organization: String,
	project: String,
	environment: String,
) -> rpc::Result<crate::deployments::EnvironmentStatus> {
	crate::server::status::environment_status(cx, &organization, &project, &environment).await
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn detected_names_are_names_never_values() {
		let checked = Detected {
			targets: vec!["web".into(), "server".into(), "web".into()],
			variables: vec!["DATABASE_URL".into(), "_TOKEN".into()],
		}
		.checked()
		.expect("names");
		assert_eq!(checked.targets, ["server", "web"]);
		assert_eq!(checked.variables, ["DATABASE_URL", "_TOKEN"]);

		for bad in ["API_KEY=hunter2", "api_key", "1KEY", "KEY VALUE", ""] {
			let error = Detected {
				targets: Vec::new(),
				variables: vec![bad.into()],
			}
			.checked()
			.expect_err(bad);
			// What might be a value is never repeated back.
			assert!(!error.contains("hunter2"), "{error}");
			assert!(bad.is_empty() || !error.contains(bad), "{error}");
		}

		assert!(
			Detected {
				targets: vec!["Web".into()],
				variables: Vec::new(),
			}
			.checked()
			.is_err()
		);
	}

	#[test]
	fn scales_follow_the_cloud_table_rules() {
		assert_eq!(EnvironmentScale::sleeping().check(), Ok(()));

		let mut scale = EnvironmentScale::sleeping();
		scale.app = ProcessScale {
			size: Size::S,
			min: 1,
			max: 3,
			sleep: Sleep::Never,
		};
		assert_eq!(scale.check(), Ok(()));

		scale.app.min = 4;
		assert!(scale.check().is_err());

		let mut services = EnvironmentScale::sleeping();
		services.app.sleep = Sleep::Never;
		assert!(services.check().unwrap_err().starts_with("app: "));

		let mut named = EnvironmentScale::sleeping();
		named
			.services
			.insert("Search".into(), ProcessScale::sleeping());
		assert!(named.check().is_err());
	}
}
