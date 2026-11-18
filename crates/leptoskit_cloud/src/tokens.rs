//! API tokens: what CI and scripts call the cloud with.
//!
//! A token is `lkc_<prefix>_<secret>`: `lkc_`, twelve lower-case hex
//! characters that find it, an underscore, and 64 more that prove it. The
//! cloud keeps only the SHA-256 of the secret, so the value is shown once,
//! by [`create_token`], and never again. A token belongs to one
//! organisation, and optionally one project; it expires (in 90 days unless
//! asked otherwise, and never later than 365), and revoking it stops it at
//! once.
//!
//! A token can do what its scopes allow *and* the person who made it can
//! still do: a token never outlives a demotion or its maker leaving. Only a
//! person lists, makes, or revokes tokens; a token can't, whatever its
//! scopes, so no token outlives the revocation of the one that made it.

use leptoskit_db::Timestamp;
use leptoskit_rpc as rpc;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use uuid::Uuid;

use crate::access::TokenScope;

/// What every token's value starts with.
pub const TOKEN_PREFIX: &str = "lkc_";

/// How many lower-case hex characters the lookup part has.
pub const LOOKUP_LEN: usize = 12;

/// How many lower-case hex characters the secret part has: 32 random bytes.
pub const SECRET_LEN: usize = 64;

/// The lifetimes a token may have, in days. There is no "never".
pub const LIFETIMES_DAYS: [u32; 3] = [30, 90, 365];

/// The lifetime a token gets when none is asked for, in days.
pub const DEFAULT_LIFETIME_DAYS: u32 = 90;

/// A token's value, split into the part that finds it and the part that
/// proves it. Its `Debug` never shows the secret.
#[derive(Clone, PartialEq, Eq)]
pub struct TokenValue<'a> {
	/// The twelve hex characters after `lkc_`.
	pub lookup: &'a str,
	/// The 64 hex characters after the second underscore.
	pub secret: &'a str,
}

impl<'a> TokenValue<'a> {
	/// Splits `value`, or `None` when it isn't shaped like a token, so it
	/// can't be one: nothing is looked up for it.
	#[must_use]
	pub fn parse(value: &'a str) -> Option<Self> {
		let rest = value.strip_prefix(TOKEN_PREFIX)?;
		let (lookup, secret) = rest.split_once('_')?;
		let hex = |part: &str, len: usize| {
			part.len() == len
				&& part
					.bytes()
					.all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
		};

		(hex(lookup, LOOKUP_LEN) && hex(secret, SECRET_LEN)).then_some(Self { lookup, secret })
	}
}

impl std::fmt::Debug for TokenValue<'_> {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_struct("TokenValue")
			.field("lookup", &self.lookup)
			.field("secret", &"[redacted]")
			.finish()
	}
}

/// A token as its organisation's managers see it: never its secret.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TokenSummary {
	pub id: Uuid,
	pub name: String,
	/// The non-secret part of its value, `lkc_<prefix>_…`, to tell tokens
	/// apart.
	pub prefix: String,
	pub scopes: Vec<TokenScope>,
	/// The one project it may touch, when it is bound to one.
	pub project: Option<String>,
	pub expires_at: Timestamp,
	/// When it last authenticated a request, to within a few minutes.
	pub last_used_at: Option<Timestamp>,
	/// The person it acts for.
	pub created_by: Uuid,
	pub created_at: Timestamp,
}

/// A new token, and its value: shown this once, and never stored.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CreatedToken {
	pub token: TokenSummary,
	/// The whole value, `lkc_<prefix>_<secret>`. Keep it now: the cloud
	/// can't show it again.
	pub value: String,
}

impl std::fmt::Debug for CreatedToken {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_struct("CreatedToken")
			.field("token", &self.token)
			.field("value", &"[redacted]")
			.finish()
	}
}

/// The organisation's live tokens: neither revoked nor expired.
///
/// # Errors
///
/// `Forbidden` unless the caller manages tokens.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "tokens")]
#[rpc::query]
pub async fn tokens(cx: &rpc::QueryCx, organization: String) -> rpc::Result<Vec<TokenSummary>> {
	crate::server::tokens::tokens(cx, &organization).await
}

/// Makes a token, and returns its value this once. Only a person may.
///
/// It is an action, not a mutation, so the value is never kept anywhere,
/// not even as the answer to a repeated request: an `Idempotency-Key` is
/// ignored. A live token's name is unique in its organisation instead, so
/// a request sent again after its answer was lost is refused with
/// `token_name_taken` naming the token it made, whose value nobody saw:
/// revoke that one and ask again.
///
/// # Arguments
///
/// - `name`: what to call it, such as `ci`; no other live token in the
///   organisation may have it.
/// - `scopes`: what it may do; at least one, and only scopes the caller's role
///   may give.
/// - `project`: the one project it may touch, when it should be bound to one.
/// - `expires_in_days`: 30, 90, or 365; 90 when left out.
///
/// # Errors
///
/// `Forbidden` (`person_required`) for a token, and (`token_too_strong`) for
/// scopes the caller's role can't give; `NotFound` for a project the
/// organisation doesn't have; `Conflict` (`token_name_taken`) for a name a
/// live token has.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "tokens")]
#[rpc::action]
pub async fn create_token(
	cx: &rpc::ActionCx,
	organization: String,
	name: String,
	scopes: Vec<TokenScope>,
	#[serde(default)] project: Option<String>,
	#[serde(default)] expires_in_days: Option<u32>,
) -> rpc::Result<CreatedToken> {
	crate::server::tokens::create_token(cx, organization, name, scopes, project, expires_in_days)
		.await
}

/// Revokes a token: the next request with it is turned away.
///
/// # Errors
///
/// `NotFound` for a token that isn't this organisation's, or is already
/// revoked.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "tokens")]
#[rpc::mutation]
pub async fn revoke_token(
	cx: &rpc::MutationCx,
	organization: String,
	token: Uuid,
) -> rpc::Result<()> {
	crate::server::tokens::revoke_token(cx, &organization, token).await
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn only_a_well_formed_value_parses() {
		let lookup = "0123456789ab";
		let secret = "f".repeat(SECRET_LEN);
		let value = format!("lkc_{lookup}_{secret}");
		let parsed = TokenValue::parse(&value).expect("a token");
		assert_eq!(parsed.lookup, lookup);
		assert_eq!(parsed.secret, secret);
		assert!(!format!("{parsed:?}").contains(&secret));

		for bad in [
			String::new(),
			format!("lkc_{lookup}{secret}"),
			format!("lkx_{lookup}_{secret}"),
			format!("lkc_{}_{secret}", "0123456789AB"),
			format!("lkc_{lookup}_{}", &secret[1..]),
			format!("lkc_{lookup}_{secret}0"),
			format!("lkc_{lookup}_{}", "g".repeat(SECRET_LEN)),
			format!("lkc_{lookup}_{}_x", &secret[..SECRET_LEN - 2]),
		] {
			assert!(TokenValue::parse(&bad).is_none(), "{bad}");
		}
	}
}
