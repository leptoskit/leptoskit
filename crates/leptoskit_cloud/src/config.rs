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

/// The most an environment's variables take together, names and values, in
/// bytes. A release carries them all, to every process.
pub const MAX_VARIABLES_TOTAL_LEN: usize = 256 * 1024;

/// The most an environment's secrets take together, names and sealed
/// values, in bytes. A cluster keeps them in one object of at most 1 MiB.
pub const MAX_SECRETS_TOTAL_LEN: usize = 512 * 1024;

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

/// A secret's value on its way in. It is a string on the wire.
///
/// Its `Debug` never shows it, and reading one that isn't a string says so
/// without repeating what was sent: a secret sent as a number by mistake
/// must not come back in the error.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(transparent)]
pub struct SecretValue(String);

impl SecretValue {
	#[must_use]
	pub fn new(value: impl Into<String>) -> Self {
		Self(value.into())
	}

	/// The value itself.
	#[must_use]
	pub fn expose(&self) -> &str {
		&self.0
	}

	#[must_use]
	pub fn into_inner(self) -> String {
		self.0
	}
}

impl std::fmt::Debug for SecretValue {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter.write_str("SecretValue([redacted])")
	}
}

impl From<String> for SecretValue {
	fn from(value: String) -> Self {
		Self(value)
	}
}

impl From<&str> for SecretValue {
	fn from(value: &str) -> Self {
		Self(value.to_owned())
	}
}

impl<'de> Deserialize<'de> for SecretValue {
	fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		use serde::de::IgnoredAny;

		/// What was sent, without keeping anything that isn't a string.
		struct Sent;

		impl<'de> serde::de::Visitor<'de> for Sent {
			type Value = Option<String>;

			fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
				formatter.write_str("a string")
			}

			fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
				Ok(Some(value.to_owned()))
			}

			fn visit_string<E: serde::de::Error>(self, value: String) -> Result<Self::Value, E> {
				Ok(Some(value))
			}

			fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<Self::Value, E> {
				Ok(None)
			}

			fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<Self::Value, E> {
				Ok(None)
			}

			fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<Self::Value, E> {
				Ok(None)
			}

			fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<Self::Value, E> {
				Ok(None)
			}

			fn visit_bytes<E: serde::de::Error>(self, _: &[u8]) -> Result<Self::Value, E> {
				Ok(None)
			}

			fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
				Ok(None)
			}

			fn visit_none<E: serde::de::Error>(self) -> Result<Self::Value, E> {
				Ok(None)
			}

			fn visit_some<D: serde::Deserializer<'de>>(
				self,
				deserializer: D,
			) -> Result<Self::Value, D::Error> {
				IgnoredAny::deserialize(deserializer).map(|_| None)
			}

			fn visit_seq<A: serde::de::SeqAccess<'de>>(
				self,
				mut seq: A,
			) -> Result<Self::Value, A::Error> {
				while seq.next_element::<IgnoredAny>()?.is_some() {}

				Ok(None)
			}

			fn visit_map<A: serde::de::MapAccess<'de>>(
				self,
				mut map: A,
			) -> Result<Self::Value, A::Error> {
				while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}

				Ok(None)
			}
		}

		deserializer
			.deserialize_any(Sent)?
			.map(Self)
			.ok_or_else(|| {
				serde::de::Error::custom(
					"a secret's value must be a string; it isn't repeated here",
				)
			})
	}
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
	value: SecretValue,
) -> rpc::Result<SecretSummary> {
	crate::server::config::set_secret(
		cx,
		organization,
		project,
		environment,
		name,
		value.into_inner(),
	)
	.await
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

	#[test]
	fn a_secret_that_is_not_a_string_is_never_repeated() {
		let value: SecretValue = serde_json::from_str("\"hunter2\"").expect("a string");
		assert_eq!(value.expose(), "hunter2");
		assert_eq!(format!("{value:?}"), "SecretValue([redacted])");
		assert_eq!(serde_json::to_string(&value).expect("json"), "\"hunter2\"");

		for sent in [
			"4815162342",
			"-4815162342",
			"48151.62342",
			"true",
			"null",
			"[\"hunter2\"]",
			"{\"hunter2\": \"hunter2\"}",
		] {
			let error = serde_json::from_str::<SecretValue>(sent)
				.expect_err(sent)
				.to_string();
			assert!(error.contains("must be a string"), "{error}");
			assert!(!error.contains("4815"), "{error}");
			assert!(!error.contains("hunter2"), "{error}");

			// And through a value, as the server reads arguments.
			let value: serde_json::Value = serde_json::from_str(sent).expect("json");
			let error = serde_json::from_value::<SecretValue>(value)
				.expect_err(sent)
				.to_string();
			assert!(!error.contains("4815"), "{error}");
			assert!(!error.contains("hunter2"), "{error}");
		}
	}
}
