//! Custom domains: the hostnames an environment answers on besides its
//! default one.

use std::fmt;
use std::str::FromStr;

use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde::Serializer;

/// The domain the cloud's default hostnames live under, such as
/// `notes-production.leptoskit.app`. It is a configuration default, not a
/// promise that the domain serves anything yet.
pub const DEFAULT_APPS_DOMAIN: &str = "leptoskit.app";

/// Domains the cloud keeps for itself: no app may claim a hostname at or
/// under them.
pub const RESERVED_DOMAINS: [&str; 2] = [DEFAULT_APPS_DOMAIN, "leptoskit.dev"];

/// A custom domain: a fully qualified hostname in lower case, with no
/// wildcard, no trailing dot, and nothing at or under [`RESERVED_DOMAINS`].
///
/// ```
/// use leptoskit_config::cloud::Domain;
///
/// assert!(Domain::parse("notes.example.com").is_ok());
/// assert!(Domain::parse("*.example.com").is_err());
/// assert!(Domain::parse("notes.leptoskit.app").is_err());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Domain(String);

/// Why a string isn't a custom [`Domain`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DomainError {
	/// The string is empty.
	#[error("a domain can't be empty")]
	Empty,
	/// A `*` wildcard.
	#[error("`{0}` is a wildcard; list each hostname on its own")]
	Wildcard(String),
	/// Upper-case letters.
	#[error("`{domain}` must be lower case: `{lower}`")]
	NotLowerCase {
		/// The domain as written.
		domain: String,
		/// The domain in lower case.
		lower: String,
	},
	/// A trailing dot.
	#[error("`{0}` ends with a dot; leave it off")]
	TrailingDot(String),
	/// Longer than 253 characters.
	#[error("`{0}` is longer than the 253 characters a hostname may have")]
	TooLong(String),
	/// A single label, such as `localhost`.
	#[error("`{0}` isn't a fully qualified hostname, such as `notes.example.com`")]
	NotQualified(String),
	/// A label that is empty, longer than 63 characters, has a character
	/// other than `a-z`, `0-9`, and `-`, or starts or ends with `-`.
	#[error(
		"`{domain}` has the label `{label}`; each label is 1 to 63 letters, digits, and hyphens, \
		 not starting or ending with a hyphen"
	)]
	Label {
		/// The domain as written.
		domain: String,
		/// The label that breaks the rule.
		label: String,
	},
	/// An IP address rather than a name.
	#[error("`{0}` is an IP address; a custom domain is a hostname")]
	IpAddress(String),
	/// At or under one of the cloud's own domains.
	#[error("`{domain}` is under `{reserved}`, which the cloud keeps for its default hostnames")]
	Reserved {
		/// The domain as written.
		domain: String,
		/// The reserved domain it is at or under.
		reserved: &'static str,
	},
}

impl Domain {
	/// Checks `value` against the hostname rules.
	///
	/// # Errors
	///
	/// The first rule broken, as a [`DomainError`].
	pub fn parse(value: &str) -> Result<Self, DomainError> {
		check(value)?;
		Ok(Self(value.to_owned()))
	}

	/// The hostname.
	#[must_use]
	pub fn as_str(&self) -> &str {
		&self.0
	}
}

fn check(value: &str) -> Result<(), DomainError> {
	if value.is_empty() {
		return Err(DomainError::Empty);
	}

	if value.contains('*') {
		return Err(DomainError::Wildcard(value.to_owned()));
	}

	if value.chars().any(char::is_uppercase) {
		return Err(DomainError::NotLowerCase {
			domain: value.to_owned(),
			lower: value.to_lowercase(),
		});
	}

	if value.ends_with('.') {
		return Err(DomainError::TrailingDot(value.to_owned()));
	}

	if value.len() > 253 {
		return Err(DomainError::TooLong(value.to_owned()));
	}

	let labels: Vec<&str> = value.split('.').collect();

	if labels.len() < 2 {
		return Err(DomainError::NotQualified(value.to_owned()));
	}

	if let Some(label) = labels.iter().find(|label| !valid_label(label)) {
		return Err(DomainError::Label {
			domain: value.to_owned(),
			label: (*label).to_owned(),
		});
	}

	if labels
		.last()
		.is_some_and(|top| top.chars().all(|character| character.is_ascii_digit()))
	{
		return Err(DomainError::IpAddress(value.to_owned()));
	}

	if let Some(reserved) = RESERVED_DOMAINS.into_iter().find(|reserved| {
		value == *reserved
			|| value
				.strip_suffix(reserved)
				.is_some_and(|prefix| prefix.ends_with('.'))
	}) {
		return Err(DomainError::Reserved {
			domain: value.to_owned(),
			reserved,
		});
	}

	Ok(())
}

fn valid_label(label: &str) -> bool {
	(1..=63).contains(&label.len())
		&& !label.starts_with('-')
		&& !label.ends_with('-')
		&& label
			.chars()
			.all(|character| matches!(character, 'a'..='z' | '0'..='9' | '-'))
}

impl FromStr for Domain {
	type Err = DomainError;

	fn from_str(value: &str) -> Result<Self, Self::Err> {
		Self::parse(value)
	}
}

impl fmt::Display for Domain {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.write_str(&self.0)
	}
}

impl Serialize for Domain {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		serializer.serialize_str(&self.0)
	}
}

impl<'de> Deserialize<'de> for Domain {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		let value = String::deserialize(deserializer)?;
		check(&value).map_err(serde::de::Error::custom)?;
		Ok(Self(value))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn hostnames_parse() {
		for domain in [
			"notes.example.com",
			"example.com",
			"a.b.c.d.example.co.uk",
			"xn--bcher-kva.example",
			"my-app.example.com",
			"leptoskit.app.example.com",
			"notleptoskit.app",
		] {
			assert_eq!(Domain::parse(domain).unwrap().as_str(), domain);
		}
	}

	/// The rule an error names, for comparing without its fields.
	fn rule(error: &DomainError) -> &'static str {
		match error {
			DomainError::Empty => "empty",
			DomainError::Wildcard(_) => "wildcard",
			DomainError::NotLowerCase { .. } => "lower case",
			DomainError::TrailingDot(_) => "trailing dot",
			DomainError::TooLong(_) => "too long",
			DomainError::NotQualified(_) => "not qualified",
			DomainError::Label { .. } => "label",
			DomainError::IpAddress(_) => "ip address",
			DomainError::Reserved { reserved, .. } => reserved,
		}
	}

	#[test]
	fn each_rule_has_its_own_error() {
		let long = format!("{}.com", "a.".repeat(126));
		let wide_label = format!("{}.com", "a".repeat(64));

		for (domain, expected) in [
			("", "empty"),
			("*.example.com", "wildcard"),
			("Notes.example.com", "lower case"),
			("example.com.", "trailing dot"),
			(&long, "too long"),
			("localhost", "not qualified"),
			("-a.example.com", "label"),
			("a..example.com", "label"),
			("under_score.example.com", "label"),
			(&wide_label, "label"),
			("10.0.0.1", "ip address"),
			("notes.leptoskit.app", "leptoskit.app"),
			("leptoskit.dev", "leptoskit.dev"),
		] {
			let error = Domain::parse(domain).unwrap_err();
			assert_eq!(rule(&error), expected, "{domain}: {error}");
		}

		assert_eq!(
			Domain::parse("Notes.example.com").unwrap_err().to_string(),
			"`Notes.example.com` must be lower case: `notes.example.com`"
		);
	}
}
