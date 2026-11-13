//! Literal, public service settings shared by capture and release consumers.
use std::collections::BTreeMap;

use anyhow::Result;
use anyhow::ensure;

const MAX_ENTRIES: usize = 128;
const MAX_NAME: usize = 128;
const MAX_VALUE: usize = 8192;
const MAX_BYTES: usize = 32768;

pub(crate) fn validate(values: &BTreeMap<String, String>) -> Result<()> {
	ensure!(
		values.len() <= MAX_ENTRIES,
		"service environment exceeds 128 entries"
	);
	let mut total = 0;

	for (name, value) in values {
		ensure!(
			name.len() <= MAX_NAME && leptoskit_config::infra::env_name(name),
			"invalid service environment variable name"
		);
		ensure!(
			value.len() <= MAX_VALUE && value.chars().all(printable),
			"service environment values must be printable Unicode, at most 8192 UTF-8 bytes"
		);
		total += name.len() + value.len() + 2;
	}

	ensure!(
		total <= MAX_BYTES,
		"service environment exceeds 32768 bytes"
	);
	Ok(())
}

// Match the worker contract. Control characters and Unicode noncharacters
// cannot safely survive the systemd Environment directive.
fn printable(value: char) -> bool {
	let code = u32::from(value);
	!value.is_control()
		&& code != 0xfeff
		&& !(0xfdd0..=0xfdef).contains(&code)
		&& code & 0xffff < 0xfffe
}

/// Only the declared service env table may contain literal runtime values.
/// These bytes are public artifact inputs, not an operator secret channel.
pub(super) fn metadata(value: &toml::Value) -> Result<()> {
	let values: BTreeMap<String, String> = value.clone().try_into()?;
	validate(&values)
}

pub(super) fn metadata_path(path: &[String]) -> bool {
	path.iter()
		.map(String::as_str)
		.eq(["package", "metadata", "leptoskit", "service", "env"])
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn literal_settings_preserve_empty_unicode_paths_and_systemd_metacharacters() {
		let values = BTreeMap::from([
			("EMPTY".into(), String::new()),
			("LABEL".into(), "Hello 世界 \\\" %n $HOME = /srv/app".into()),
		]);
		validate(&values).unwrap();
	}

	#[test]
	fn invalid_or_unbounded_values_fail_without_printing_values() {
		for value in ["x\nUser=root", "\0", "\u{85}", "\u{feff}", "\u{ffff}"] {
			let error = validate(&BTreeMap::from([("VALUE".into(), value.into())])).unwrap_err();
			assert!(!error.to_string().contains(value));
		}
		assert!(validate(&BTreeMap::from([("BAD-NAME".into(), "x".into())])).is_err());
		assert!(validate(&BTreeMap::from([("VALUE".into(), "x".repeat(8193))])).is_err());
		let many = (0..129).map(|i| (format!("V{i}"), String::new())).collect();
		assert!(validate(&many).is_err());
		let large = (0..5)
			.map(|i| (format!("V{i}"), "x".repeat(8192)))
			.collect();
		assert!(validate(&large).is_err());
	}

	#[test]
	fn only_declared_service_metadata_env_can_enter_exact_manifest_capture() {
		let source = "[package.metadata.leptoskit.service.env]\nMODE = 'production'\nROOT = \
		              '/srv/public files'\nEMPTY = ''\nLABEL = '世界 %n $HOME'\n";
		let parsed = super::super::manifest(source.as_bytes()).unwrap();
		assert_eq!(
			parsed["package"]["metadata"]["leptoskit"]["service"]["env"]["ROOT"].as_str(),
			Some("/srv/public files")
		);
		for source in [
			"[package.metadata.other.env]\nMODE = 'production'",
			"[[package.metadata.leptoskit.service]]\n[package.metadata.leptoskit.service.env]\\
			 nMODE = 'production'",
			"[package.metadata.leptoskit.service.env]\nCOUNT = 1",
			"[package.metadata.leptoskit.service.env]\nTEXT = \"bad\\nvalue\"",
			"[package.metadata.leptoskit.service]\nsecrets = ['TOKEN']",
			"[dependencies.local]\npath = '/private/source'",
			"[package.metadata.other]\npassword = 'not-for-capture'",
		] {
			assert!(
				super::super::manifest(source.as_bytes()).is_err(),
				"{source}"
			);
		}
	}
}
