//! An environment's variables and secrets: what its processes start with.
//!
//! A **variable** is a name and a value anyone who can see the project can
//! read. A **secret** is a name and a value nobody reads back: it is kept
//! encrypted, [`secrets`] lists names and versions only, and the value
//! leaves the cloud only for the cluster worker applying a release of that
//! environment.
//!
//! Both apply to every process of the environment, and take effect at its
//! next deployment: setting one changes nothing that is running.
//!
//! A name is an environment variable's: upper-case letters, digits, and
//! underscores, not starting with a digit. An environment has a name once,
//! as a variable or as a secret, and the names the cloud sets itself
//! ([`is_reserved`]) can't be set.

use leptoskit_db::Timestamp;
use leptoskit_rpc as rpc;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

/// The most variables an environment has.
pub const MAX_VARIABLES: usize = 200;

/// The most secrets an environment has.
pub const MAX_SECRETS: usize = 200;

/// The longest variable value, in bytes.
pub const MAX_VARIABLE_VALUE_LEN: usize = 32 * 1024;

/// The longest secret value, in bytes.
pub const MAX_SECRET_VALUE_LEN: usize = 64 * 1024;

/// The prefix of the names the cloud sets on every process.
pub const RESERVED_PREFIX: &str = "LEPTOSKIT_";

/// The names the cloud sets or may set itself, besides everything starting
/// with [`RESERVED_PREFIX`].
pub const RESERVED_NAMES: [&str; 2] = ["DATABASE_URL", "PORT"];

/// Whether the cloud keeps `name` for itself.
#[must_use]
pub fn is_reserved(name: &str) -> bool {
	name.starts_with(RESERVED_PREFIX) || RESERVED_NAMES.contains(&name)
}

/// Checks that `name` can name a variable or a secret.
///
/// # Errors
///
/// What is wrong with it. A name that isn't one may be a value sent by
/// mistake, so that error never repeats it.
pub fn check_name(name: &str) -> Result<(), String> {
	if !crate::projects::is_variable_name(name) {
		return Err(format!(
			"a name must be upper-case letters, digits, and underscores, starting with a letter \
			 or underscore, at most {} characters",
			crate::projects::MAX_VARIABLE_LEN
		));
	}

	if is_reserved(name) {
		return Err(format!(
			"`{name}` is set by the cloud: names starting with `{RESERVED_PREFIX}`, and {}, can't \
			 be set",
			RESERVED_NAMES.map(|name| format!("`{name}`")).join(" and ")
		));
	}

	Ok(())
}

/// A variable, with its value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Variable {
	pub name: String,
	pub value: String,
	pub updated_at: Timestamp,
}

/// A secret, without its value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SecretSummary {
	pub name: String,
	/// How many values this name has had here, from 1.
	pub version: u64,
	pub updated_at: Timestamp,
}

/// An environment's variables, by name.
///
/// # Errors
///
/// `NotFound` for an environment the caller can't see.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "config")]
#[rpc::query]
pub async fn variables(
	cx: &rpc::QueryCx,
	organization: String,
	project: String,
	environment: String,
) -> rpc::Result<Vec<Variable>> {
	crate::server::config::variables(cx, &organization, &project, &environment).await
}

/// Sets a variable, replacing its value when it has one. It applies at the
/// environment's next deployment.
///
/// # Errors
///
/// `Invalid` (`invalid_name`, `value_too_long`) for a name or value that
/// breaks the rules; `Conflict` (`name_is_secret`) when a secret has the
/// name, and (`too_many_variables`) at [`MAX_VARIABLES`].
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "config")]
#[rpc::mutation]
pub async fn set_variable(
	cx: &rpc::MutationCx,
	organization: String,
	project: String,
	environment: String,
	name: String,
	value: String,
) -> rpc::Result<Variable> {
	crate::server::config::set_variable(cx, &organization, &project, &environment, name, value)
		.await
}

/// Removes a variable. It stays in what is running until the environment's
/// next deployment.
///
/// # Errors
///
/// `NotFound` (`variable_not_found`) when the environment has none by that
/// name.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "config")]
#[rpc::mutation]
pub async fn unset_variable(
	cx: &rpc::MutationCx,
	organization: String,
	project: String,
	environment: String,
	name: String,
) -> rpc::Result<()> {
	crate::server::config::unset_variable(cx, &organization, &project, &environment, &name).await
}

/// An environment's secrets, by name: names and versions, never values.
///
/// # Errors
///
/// `NotFound` for an environment the caller can't see.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "config")]
#[rpc::query]
pub async fn secrets(
	cx: &rpc::QueryCx,
	organization: String,
	project: String,
	environment: String,
) -> rpc::Result<Vec<SecretSummary>> {
	crate::server::config::secrets(cx, &organization, &project, &environment).await
}

/// Stores a secret, replacing its value when it has one. It applies at the
/// environment's next deployment, and no operation returns it.
///
/// It is an action, not a mutation: a mutation called with an
/// `Idempotency-Key` leaves a receipt that keeps a hash of its arguments,
/// and these hold the value. Sending the same value again is safe; it
/// stores another version.
///
/// # Errors
///
/// `Invalid` (`invalid_name`, `value_empty`, `value_too_long`,
/// `value_not_text`) for a name or value that breaks the rules; `Conflict`
/// (`name_is_variable`) when a variable has the name, and
/// (`too_many_secrets`) at [`MAX_SECRETS`].
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "config")]
#[rpc::action]
pub async fn set_secret(
	cx: &rpc::ActionCx,
	organization: String,
	project: String,
	environment: String,
	name: String,
	value: String,
) -> rpc::Result<SecretSummary> {
	crate::server::config::set_secret(cx, organization, project, environment, name, value).await
}

/// Removes a secret. What is running keeps it until the environment's next
/// deployment; a rollback to a deployment that used it is refused until it
/// is set again.
///
/// # Errors
///
/// `NotFound` (`secret_not_found`) when the environment has none by that
/// name.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "config")]
#[rpc::mutation]
pub async fn unset_secret(
	cx: &rpc::MutationCx,
	organization: String,
	project: String,
	environment: String,
	name: String,
) -> rpc::Result<()> {
	crate::server::config::unset_secret(cx, &organization, &project, &environment, &name).await
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn names_are_environment_variable_names_the_cloud_does_not_set() {
		for good in ["API_KEY", "_PRIVATE", "S3_BUCKET_2", "A"] {
			assert_eq!(check_name(good), Ok(()), "{good}");
		}

		for bad in ["api_key", "1KEY", "KEY VALUE", "", "API_KEY=hunter2"] {
			let error = check_name(bad).expect_err(bad);
			// What might be a value is never repeated back.
			assert!(!error.contains("hunter2"), "{error}");
		}

		for reserved in [
			"DATABASE_URL",
			"PORT",
			"LEPTOSKIT_ENVIRONMENT",
			"LEPTOSKIT_ANYTHING",
		] {
			let error = check_name(reserved).expect_err(reserved);
			assert!(error.contains("set by the cloud"), "{error}");
		}
	}
}
