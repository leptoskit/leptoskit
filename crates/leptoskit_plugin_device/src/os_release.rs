//! A strict-enough parser for `os-release(5)` and `lsb-release` files.
//!
//! Lines are `KEY=value`; values may be single- or double-quoted, and double
//! quotes allow `\"`, `\\`, `\$` and `` \` `` escapes. Comments, blank lines
//! and malformed lines are ignored, as the format requires.

use std::collections::HashMap;

use crate::Linux;
use crate::MAX_SHORT_ITEM_BYTES;
use crate::MAX_SHORT_LIST;
use crate::MAX_TEXT_BYTES;

/// Parses assignments. Later assignments win.
pub(crate) fn parse(text: &str) -> HashMap<&str, String> {
	let mut values = HashMap::new();

	for line in text.lines() {
		let line = line.trim();

		if line.is_empty() || line.starts_with('#') {
			continue;
		}

		let Some((key, raw)) = line.split_once('=') else {
			continue;
		};

		if key.is_empty()
			|| !key
				.bytes()
				.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
		{
			continue;
		}

		if let Some(value) = unquote(raw) {
			values.insert(key, value);
		}
	}

	values
}

/// Removes one level of shell-style quoting. `None` for an unterminated
/// quote.
fn unquote(raw: &str) -> Option<String> {
	let raw = raw.trim();

	if let Some(inner) = raw.strip_prefix('\'') {
		return inner.strip_suffix('\'').map(str::to_owned);
	}

	let Some(inner) = raw.strip_prefix('"') else {
		return Some(raw.to_owned());
	};
	let inner = inner.strip_suffix('"')?;
	let mut value = String::with_capacity(inner.len());
	let mut characters = inner.chars();

	while let Some(character) = characters.next() {
		if character == '\\' {
			match characters.next() {
				Some(escaped @ ('"' | '\\' | '$' | '`')) => value.push(escaped),
				Some(other) => {
					value.push('\\');
					value.push(other);
				}
				None => value.push('\\'),
			}
		} else {
			value.push(character);
		}
	}

	Some(value)
}

/// Builds the Linux section from `os-release` and, for missing fields,
/// `lsb-release`. With an `os-release` file, the format's documented defaults
/// (`Linux`, `linux`) fill `NAME`, `ID` and `PRETTY_NAME`.
pub(crate) fn linux(os_release: Option<&str>, lsb_release: Option<&str>) -> Linux {
	let os = os_release.map(parse).unwrap_or_default();
	let lsb = lsb_release.map(parse).unwrap_or_default();
	let defaults = os_release.is_some();
	let field = |key: &str| bounded(os.get(key).map(String::as_str));
	let either = |key: &str, fallback: &str| {
		field(key).or_else(|| bounded(lsb.get(fallback).map(String::as_str)))
	};
	let with_default = |value: Option<String>, default: &str| {
		value.or_else(|| defaults.then(|| default.to_owned()))
	};

	Linux {
		name: with_default(field("NAME"), "Linux"),
		version: field("VERSION"),
		id: with_default(either("ID", "DISTRIB_ID"), "linux"),
		version_codename: either("VERSION_CODENAME", "DISTRIB_CODENAME"),
		version_id: either("VERSION_ID", "DISTRIB_RELEASE"),
		pretty_name: with_default(either("PRETTY_NAME", "DISTRIB_DESCRIPTION"), "Linux"),
		build_id: field("BUILD_ID"),
		variant: field("VARIANT"),
		variant_id: field("VARIANT_ID"),
		id_like: os
			.get("ID_LIKE")
			.map(|value| {
				value
					.split_ascii_whitespace()
					.filter(|item| item.len() <= MAX_SHORT_ITEM_BYTES)
					.take(MAX_SHORT_LIST)
					.map(str::to_owned)
					.collect()
			})
			.unwrap_or_default(),
	}
}

/// A non-empty value that fits the wire.
fn bounded(value: Option<&str>) -> Option<String> {
	value
		.filter(|value| !value.is_empty() && value.len() <= MAX_TEXT_BYTES)
		.map(str::to_owned)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn parses_quotes_escapes_comments_and_malformed_lines() {
		let values = parse(concat!(
			"# comment\n",
			"\n",
			"NAME=\"Ubuntu\"\n",
			"VERSION='24.04.1 LTS (Noble Numbat)'\n",
			"ID=ubuntu\n",
			"PRETTY_NAME=\"Say \\\"hi\\\" \\\\ \\$HOME \\`x\\` \\n\"\n",
			"BROKEN\n",
			"bad-key=1\n",
			"UNTERMINATED=\"open\n",
			"  SPACED = value  \n",
			"ID=override\n",
		));
		assert_eq!(values["NAME"], "Ubuntu");
		assert_eq!(values["VERSION"], "24.04.1 LTS (Noble Numbat)");
		assert_eq!(values["ID"], "override");
		assert_eq!(values["PRETTY_NAME"], "Say \"hi\" \\ $HOME `x` \\n");
		assert!(!values.contains_key("BROKEN"));
		assert!(!values.contains_key("bad-key"));
		assert!(!values.contains_key("UNTERMINATED"));
		assert!(!values.contains_key("SPACED "));
	}

	#[test]
	fn builds_linux_from_os_release_with_lsb_fallbacks() {
		let linux = linux(
			Some(concat!(
				"NAME=\"Fedora Linux\"\n",
				"VERSION=\"41 (Workstation Edition)\"\n",
				"ID=fedora\n",
				"ID_LIKE=\"rhel  centos\tdebian\"\n",
				"VARIANT=\"Workstation Edition\"\n",
				"VARIANT_ID=workstation\n",
				"BUILD_ID=\"\"\n",
			)),
			Some("DISTRIB_CODENAME=noble\nDISTRIB_RELEASE=24.04\n"),
		);
		assert_eq!(linux.name.as_deref(), Some("Fedora Linux"));
		assert_eq!(linux.version.as_deref(), Some("41 (Workstation Edition)"));
		assert_eq!(linux.id.as_deref(), Some("fedora"));
		assert_eq!(linux.id_like, ["rhel", "centos", "debian"]);
		assert_eq!(linux.version_codename.as_deref(), Some("noble"));
		assert_eq!(linux.version_id.as_deref(), Some("24.04"));
		assert_eq!(linux.pretty_name.as_deref(), Some("Linux"));
		assert_eq!(linux.build_id, None, "an empty value is unknown");
		assert_eq!(linux.variant.as_deref(), Some("Workstation Edition"));
		assert_eq!(linux.variant_id.as_deref(), Some("workstation"));
	}

	#[test]
	fn defaults_apply_only_with_an_os_release_file() {
		assert_eq!(linux(None, None), Linux::default());
		let lsb = linux(
			None,
			Some("DISTRIB_ID=Ubuntu\nDISTRIB_DESCRIPTION=\"Ubuntu 24.04\"\n"),
		);
		assert_eq!(lsb.id.as_deref(), Some("Ubuntu"));
		assert_eq!(lsb.pretty_name.as_deref(), Some("Ubuntu 24.04"));
		assert_eq!(lsb.name, None);
		let empty = linux(Some(""), None);
		assert_eq!(empty.name.as_deref(), Some("Linux"));
		assert_eq!(empty.id.as_deref(), Some("linux"));
	}

	#[test]
	fn bounds_values_and_id_like_to_the_wire_limits() {
		let long = "x".repeat(MAX_TEXT_BYTES + 1);
		let many = (0..40).map(|index| format!("d{index}")).collect::<Vec<_>>();
		let source = format!(
			"VERSION={long}\nID_LIKE=\"{} {}\"\n",
			"y".repeat(MAX_SHORT_ITEM_BYTES + 1),
			many.join(" ")
		);
		let linux = linux(Some(&source), None);
		assert_eq!(linux.version, None);
		assert_eq!(linux.id_like.len(), MAX_SHORT_LIST);
		assert_eq!(linux.id_like[0], "d0");
		let snapshot = crate::Snapshot {
			platform: crate::Platform::Linux(linux),
			..crate::Snapshot::default()
		};
		assert!(snapshot.encode().is_ok());
	}
}
