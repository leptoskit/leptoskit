//! An environment's custom domains: the hostnames it answers on besides the
//! one the cloud gives it.
//!
//! A domain is **attached** to an environment, then **verified**: the cloud
//! reads a TXT record only whoever runs the domain's DNS can write, and
//! until it finds it the domain routes nothing. A verified hostname belongs
//! to one environment in the whole cloud. Like variables, domains apply at
//! the environment's next deployment: attaching, verifying, or detaching
//! one changes nothing that is running.
//!
//! A hostname is a fully qualified name in lower case, with no wildcard,
//! port, or trailing dot, and nothing at or under a domain the cloud keeps
//! for itself.

use leptoskit_db::Timestamp;
use leptoskit_rpc as rpc;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

/// The most domains an environment has.
pub const MAX_DOMAINS: usize = 20;

/// The label the challenge record is published under, before the hostname:
/// `_leptoskit-challenge.notes.example.com`.
pub const CHALLENGE_LABEL: &str = "_leptoskit-challenge";

/// What the challenge record's value starts with, before the token.
pub const CHALLENGE_PREFIX: &str = "leptoskit-verify=";

/// The name of the TXT record that proves `hostname`.
#[must_use]
pub fn challenge_name(hostname: &str) -> String {
	format!("{CHALLENGE_LABEL}.{hostname}")
}

/// The value that record must hold for `token`.
#[must_use]
pub fn challenge_value(token: &str) -> String {
	format!("{CHALLENGE_PREFIX}{token}")
}

/// Where a domain is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DomainStatus {
	/// Attached, and waiting for its challenge record to be found.
	Pending,
	/// Proved. The environment's next deployment serves it.
	Verified,
	/// Detached while a deployment still serves it. It is gone when a
	/// deployment without it is live, and until then nobody else can
	/// verify the hostname.
	Removing,
}

/// A DNS record to publish for a domain.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DnsRecord {
	/// `TXT` or `CNAME`.
	pub kind: String,
	pub name: String,
	pub value: String,
}

/// A domain of an environment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DomainSummary {
	pub hostname: String,
	pub status: DomainStatus,
	/// What to publish: the challenge record that proves the domain, and,
	/// once the environment has been deployed, where to point it. An apex
	/// can't hold a `CNAME`; its DNS host's alias record does the same.
	pub records: Vec<DnsRecord>,
	pub verified_at: Option<Timestamp>,
	pub created_at: Timestamp,
}

/// An environment's domains, by hostname.
///
/// # Errors
///
/// `NotFound` for an environment the caller can't see.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "domains")]
#[rpc::query]
pub async fn domains(
	cx: &rpc::QueryCx,
	organization: String,
	project: String,
	environment: String,
) -> rpc::Result<Vec<DomainSummary>> {
	crate::server::domains::domains(cx, &organization, &project, &environment).await
}

/// Attaches a domain to an environment, pending until it is verified. It
/// answers with the records to publish. Attaching one the environment
/// already has changes nothing, except that one being removed is kept.
///
/// # Errors
///
/// `Invalid` (`invalid_domain`) for a hostname that breaks the rules or is
/// the cloud's own; `Conflict` (`too_many_domains`) at [`MAX_DOMAINS`].
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "domains")]
#[rpc::mutation]
pub async fn attach_domain(
	cx: &rpc::MutationCx,
	organization: String,
	project: String,
	environment: String,
	hostname: String,
) -> rpc::Result<DomainSummary> {
	crate::server::domains::attach_domain(cx, &organization, &project, &environment, &hostname)
		.await
}

/// Looks for a domain's challenge record and, finding it, marks the domain
/// verified. The environment's next deployment serves it.
///
/// It is an action: the DNS lookup happens outside any transaction.
///
/// # Errors
///
/// `NotFound` (`domain_not_found`) when the environment has no such
/// domain; `Conflict` (`domain_not_verified`) when the record isn't there
/// yet, (`domain_taken`) when another environment holds the hostname, and
/// (`dns_unavailable`) when the lookup itself failed.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "domains")]
#[rpc::action]
pub async fn verify_domain(
	cx: &rpc::ActionCx,
	organization: String,
	project: String,
	environment: String,
	hostname: String,
) -> rpc::Result<DomainSummary> {
	crate::server::domains::verify_domain(cx, organization, project, environment, hostname).await
}

/// Detaches a domain. What is running keeps serving it until the
/// environment's next deployment, and until then the hostname stays this
/// environment's.
///
/// # Errors
///
/// `NotFound` (`domain_not_found`) when the environment has no such
/// domain.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "domains")]
#[rpc::mutation]
pub async fn detach_domain(
	cx: &rpc::MutationCx,
	organization: String,
	project: String,
	environment: String,
	hostname: String,
) -> rpc::Result<()> {
	crate::server::domains::detach_domain(cx, &organization, &project, &environment, &hostname)
		.await
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_challenge_is_a_record_under_the_hostname() {
		assert_eq!(
			challenge_name("notes.example.com"),
			"_leptoskit-challenge.notes.example.com"
		);
		assert_eq!(challenge_value("abc123"), "leptoskit-verify=abc123");
	}
}
