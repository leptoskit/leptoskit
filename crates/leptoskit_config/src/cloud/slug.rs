//! Slugs: the short names organisations, projects, and environments go by in
//! addresses and commands.

use std::borrow::Borrow;
use std::fmt;
use std::str::FromStr;

use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde::Serializer;

/// Words that are never a slug, because the cloud's own hostnames and paths
/// use them.
pub const RESERVED_SLUGS: [&str; 8] = [
	"api", "www", "admin", "cloud", "console", "app", "status", "docs",
];

/// The fewest characters a slug has.
pub const SLUG_MIN_LEN: usize = 3;

/// The most characters a slug has.
pub const SLUG_MAX_LEN: usize = 40;

/// The slug grammar as a regular expression, without the reserved words or
/// the ban on `--`, which the editor schema states separately.
pub const SLUG_PATTERN: &str = "^[a-z][a-z0-9-]{1,38}[a-z0-9]$";

/// A validated slug: 3 to 40 characters of lower-case letters, digits, and
/// hyphens, starting with a letter, ending with a letter or digit, with no
/// `--` and none of [`RESERVED_SLUGS`].
///
/// ```
/// use leptoskit_config::cloud::Slug;
///
/// let slug: Slug = "notes".parse().unwrap();
/// assert_eq!(slug.as_str(), "notes");
/// assert!("Notes".parse::<Slug>().is_err());
/// assert_eq!(
/// 	Slug::suggest("My Notes App").unwrap().as_str(),
/// 	"my-notes-app"
/// );
/// ```
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Slug(String);

/// Why a string isn't a slug, or why a name can't become one.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SlugError {
	/// Shorter than 3 or longer than 40 characters.
	#[error("`{slug}` has {length} characters; a slug has 3 to 40")]
	Length {
		/// The string that was checked.
		slug: String,
		/// Its length in characters.
		length: usize,
	},
	/// Doesn't start with a lower-case letter.
	#[error("`{0}` must start with a lower-case letter")]
	Start(String),
	/// Ends with a hyphen.
	#[error("`{0}` must end with a lower-case letter or digit")]
	End(String),
	/// Has a character other than `a-z`, `0-9`, or `-`.
	#[error("`{slug}` has `{character}`; a slug is lower-case letters, digits, and hyphens")]
	Character {
		/// The string that was checked.
		slug: String,
		/// The first character that isn't allowed.
		character: char,
	},
	/// Has two hyphens in a row.
	#[error("`{0}` has `--`; use one hyphen between words")]
	DoubleHyphen(String),
	/// Is one of [`RESERVED_SLUGS`].
	#[error("`{0}` is reserved for the cloud's own addresses; choose another name")]
	Reserved(String),
	/// A name given to [`Slug::suggest`] has no ASCII letter to start a slug
	/// with.
	#[error("`{0}` has no letters a-z to make a slug from; choose a slug yourself")]
	NoLetters(String),
}

impl Slug {
	/// Checks `value` against the slug rules.
	///
	/// # Errors
	///
	/// The first rule `value` breaks, as a [`SlugError`].
	pub fn parse(value: &str) -> Result<Self, SlugError> {
		check(value)?;
		Ok(Self(value.to_owned()))
	}

	/// Turns a display name, such as an app's name, into a slug: letters and
	/// digits are kept in lower case, every run of anything else becomes one
	/// hyphen, anything before the first letter is dropped, and the result is
	/// cut to 40 characters.
	///
	/// # Errors
	///
	/// [`SlugError::NoLetters`] when the name has no ASCII letter, and
	/// otherwise the rule the result still breaks, such as being too short
	/// (`"Go"`) or reserved (`"App"`).
	pub fn suggest(name: &str) -> Result<Self, SlugError> {
		let mut slug = String::with_capacity(name.len().min(SLUG_MAX_LEN));
		let mut pending_hyphen = false;

		for character in name.chars() {
			if character.is_ascii_alphanumeric() {
				if slug.is_empty() && !character.is_ascii_alphabetic() {
					continue;
				}

				if pending_hyphen && !slug.is_empty() {
					slug.push('-');
				}

				pending_hyphen = false;
				slug.push(character.to_ascii_lowercase());
			} else {
				pending_hyphen = true;
			}
		}

		if slug.is_empty() {
			return Err(SlugError::NoLetters(name.to_owned()));
		}

		slug.truncate(SLUG_MAX_LEN);
		let slug = slug.trim_end_matches('-');
		Self::parse(slug)
	}

	/// The slug.
	#[must_use]
	pub fn as_str(&self) -> &str {
		&self.0
	}

	/// The slug, owned.
	#[must_use]
	pub fn into_string(self) -> String {
		self.0
	}
}

fn check(value: &str) -> Result<(), SlugError> {
	let length = value.chars().count();

	if !(SLUG_MIN_LEN..=SLUG_MAX_LEN).contains(&length) {
		return Err(SlugError::Length {
			slug: value.to_owned(),
			length,
		});
	}

	if let Some(character) = value
		.chars()
		.find(|character| !matches!(character, 'a'..='z' | '0'..='9' | '-'))
	{
		return Err(SlugError::Character {
			slug: value.to_owned(),
			character,
		});
	}

	if !value.starts_with(|character: char| character.is_ascii_lowercase()) {
		return Err(SlugError::Start(value.to_owned()));
	}

	if value.ends_with('-') {
		return Err(SlugError::End(value.to_owned()));
	}

	if value.contains("--") {
		return Err(SlugError::DoubleHyphen(value.to_owned()));
	}

	if RESERVED_SLUGS.contains(&value) {
		return Err(SlugError::Reserved(value.to_owned()));
	}

	Ok(())
}

impl FromStr for Slug {
	type Err = SlugError;

	fn from_str(value: &str) -> Result<Self, Self::Err> {
		Self::parse(value)
	}
}

impl fmt::Display for Slug {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.write_str(&self.0)
	}
}

impl AsRef<str> for Slug {
	fn as_ref(&self) -> &str {
		&self.0
	}
}

impl Borrow<str> for Slug {
	fn borrow(&self) -> &str {
		&self.0
	}
}

impl Serialize for Slug {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		serializer.serialize_str(&self.0)
	}
}

impl<'de> Deserialize<'de> for Slug {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		let value = String::deserialize(deserializer)?;
		check(&value).map_err(serde::de::Error::custom)?;
		Ok(Self(value))
	}
}

#[cfg(feature = "schema")]
impl schemars::JsonSchema for Slug {
	fn schema_name() -> std::borrow::Cow<'static, str> {
		"Slug".into()
	}

	fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
		schemars::json_schema!({
			"type": "string",
			"description": "3 to 40 lower-case letters, digits, and hyphens, starting with a letter \
				and ending with a letter or digit, with no `--`.",
			"pattern": SLUG_PATTERN,
			"not": {
				"anyOf": [
					{ "enum": RESERVED_SLUGS },
					{ "pattern": "--" }
				]
			}
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn valid_slugs_parse() {
		for slug in [
			"abc",
			"notes",
			"my-notes-app",
			"a1b",
			"staging-2",
			&"a".repeat(SLUG_MAX_LEN),
		] {
			assert_eq!(Slug::parse(slug).unwrap().as_str(), slug);
		}
	}

	#[test]
	fn each_rule_has_its_own_error() {
		assert!(matches!(
			Slug::parse("ab"),
			Err(SlugError::Length { length: 2, .. })
		));
		assert!(matches!(
			Slug::parse(&"a".repeat(SLUG_MAX_LEN + 1)),
			Err(SlugError::Length { length: 41, .. })
		));
		assert!(matches!(Slug::parse("1abc"), Err(SlugError::Start(_))));
		assert!(matches!(Slug::parse("-abc"), Err(SlugError::Start(_))));
		assert!(matches!(Slug::parse("abc-"), Err(SlugError::End(_))));
		assert!(matches!(
			Slug::parse("Abc"),
			Err(SlugError::Character { character: 'A', .. })
		));
		assert!(matches!(
			Slug::parse("ab_c"),
			Err(SlugError::Character { character: '_', .. })
		));
		assert!(matches!(
			Slug::parse("a--b"),
			Err(SlugError::DoubleHyphen(_))
		));

		for reserved in RESERVED_SLUGS {
			assert!(
				matches!(Slug::parse(reserved), Err(SlugError::Reserved(_))),
				"{reserved}"
			);
		}
	}

	#[test]
	fn deserializing_validates() {
		use serde::de::value::Error;
		use serde::de::value::StrDeserializer;

		let slug = Slug::deserialize(StrDeserializer::<Error>::new("notes")).unwrap();
		assert_eq!(slug.to_string(), "notes");
		let error = Slug::deserialize(StrDeserializer::<Error>::new("Notes")).unwrap_err();
		assert!(error.to_string().contains("lower-case"), "{error}");
		assert_eq!(
			toml::Value::try_from(&slug).unwrap(),
			toml::Value::String("notes".into())
		);
	}

	#[test]
	fn names_become_slugs() {
		for (name, slug) in [
			("My Notes App", "my-notes-app"),
			("notes", "notes"),
			("  Notes!!  App  ", "notes-app"),
			("2048 Game", "game"),
			("Café Olé", "caf-ol"),
			("snake_case_name", "snake-case-name"),
			("Rock 'n' Roll", "rock-n-roll"),
		] {
			assert_eq!(Slug::suggest(name).unwrap().as_str(), slug, "{name}");
		}

		let long = Slug::suggest(&"word ".repeat(20)).unwrap();
		assert!(long.as_str().len() <= SLUG_MAX_LEN, "{long}");
		assert!(!long.as_str().ends_with('-'), "{long}");
	}

	#[test]
	fn names_that_cannot_become_slugs_say_why() {
		assert!(matches!(
			Slug::suggest("1234"),
			Err(SlugError::NoLetters(_))
		));
		assert!(matches!(Slug::suggest("!!!"), Err(SlugError::NoLetters(_))));
		assert!(matches!(
			Slug::suggest("Go"),
			Err(SlugError::Length { length: 2, .. })
		));
		assert!(matches!(Slug::suggest("App"), Err(SlugError::Reserved(_))));
	}
}
