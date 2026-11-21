//! Self-contained offline web bundle preparation and validation for Meta
//! Horizon Creator Program (MHCP) hackathon submissions.
//!
//! MHCP web-prototype competitions expect a single `.zip` archive with:
//! - `index.html` at the archive root (never nested inside a folder);
//! - every script, stylesheet, Wasm binary, and asset referenced by relative
//!   path and bundled inside the archive;
//! - zero external network/CDN requests (`http://`, `https://`, or
//!   protocol-relative `//`);
//! - total archive size within the 35 MiB cap.

use std::collections::BTreeSet;

use serde::Deserialize;
use serde::Serialize;

/// Default maximum MHCP submission archive size: 35 MiB.
pub const MAX_BUNDLE_BYTES: usize = 35 * 1024 * 1024;

/// Required top-level entry point in an MHCP `.zip` archive.
pub const ENTRY_HTML: &str = "index.html";

/// Severity of one bundle validation finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Severity {
	Error,
	Warning,
}

/// One problem or warning found while checking an MHCP bundle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct BundleIssue {
	pub severity: Severity,
	pub code: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub path: Option<String>,
	pub message: String,
}

/// Summary of checking a staged file list against MHCP submission rules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct BundleReport {
	pub file_count: usize,
	pub total_bytes: usize,
	pub max_bytes: usize,
	pub issues: Vec<BundleIssue>,
}

impl BundleReport {
	/// Whether no error-severity issues were found.
	#[must_use]
	pub fn is_valid(&self) -> bool {
		self.issues
			.iter()
			.all(|issue| issue.severity != Severity::Error)
	}

	/// Only error-severity issues.
	pub fn errors(&self) -> impl Iterator<Item = &BundleIssue> {
		self.issues
			.iter()
			.filter(|issue| issue.severity == Severity::Error)
	}
}

/// Checks staged archive entries `(relative/path, bytes)` against MHCP rules.
///
/// `max_bytes` caps the total uncompressed input bytes checked before zipping
/// (and callers should also check the final compressed `.zip` size against
/// `max_bytes`).
#[must_use]
pub fn validate_files(files: &[(String, Vec<u8>)], max_bytes: usize) -> BundleReport {
	let mut issues = Vec::new();
	let names: BTreeSet<&str> = files.iter().map(|(path, _)| path.as_str()).collect();
	let total_bytes: usize = files.iter().map(|(_, bytes)| bytes.len()).sum();

	if !names.contains(ENTRY_HTML) {
		issues.push(BundleIssue {
			severity: Severity::Error,
			code: "missing-index-html".into(),
			path: Some(ENTRY_HTML.into()),
			message: "MHCP bundles require `index.html` at the top level of the archive".into(),
		});
	}

	if total_bytes > max_bytes {
		issues.push(BundleIssue {
			severity: Severity::Error,
			code: "bundle-too-large".into(),
			path: None,
			message: format!(
				"bundle input size is {total_bytes} bytes, exceeding the {max_bytes}-byte limit"
			),
		});
	}

	for (path, bytes) in files {
		if path.starts_with('/') || path.split('/').any(|part| part == "..") {
			issues.push(BundleIssue {
				severity: Severity::Error,
				code: "unsafe-path".into(),
				path: Some(path.clone()),
				message: format!("archive entry `{path}` must be a clean relative path"),
			});
		}

		if is_html(path)
			&& let Ok(text) = std::str::from_utf8(bytes)
		{
			check_html(path, text, &names, &mut issues);
		} else if is_text_asset(path)
			&& let Ok(text) = std::str::from_utf8(bytes)
		{
			check_external_urls(path, text, &mut issues);
		}
	}

	BundleReport {
		file_count: files.len(),
		total_bytes,
		max_bytes,
		issues,
	}
}

/// Rewrites root-relative asset attributes (`src="/..."`, `href="/..."`,
/// `from '/...'`) inside a leptoskit web `index.html` into `./...` relative
/// paths so the bundle runs cleanly from a local offline `.zip`.
#[must_use]
pub fn rewrite_html_relative(html: &str) -> String {
	let mut out = html.to_owned();
	for (from, to) in [
		("src=\"/", "src=\"./"),
		("src='/", "src='./"),
		("href=\"/", "href=\"./"),
		("href='/", "href='./"),
		("from \"/", "from \"./"),
		("from '/", "from './"),
		("import(\"/", "import(\"./"),
		("import('/", "import('./"),
	] {
		out = replace_non_protocol_relative(&out, from, to);
	}
	out
}

/// Replaces `needle` with `replacement` unless followed immediately by another
/// `/` (which is a protocol-relative external URL like `//cdn.example.com`).
fn replace_non_protocol_relative(input: &str, needle: &str, replacement: &str) -> String {
	let mut result = String::with_capacity(input.len());
	let mut rest = input;
	while let Some(pos) = rest.find(needle) {
		result.push_str(&rest[..pos]);
		let after = &rest[pos + needle.len()..];
		if after.starts_with('/') {
			result.push_str(needle);
		} else {
			result.push_str(replacement);
		}
		rest = after;
	}
	result.push_str(rest);
	result
}

fn has_extension(path: &str, extensions: &[&str]) -> bool {
	std::path::Path::new(path)
		.extension()
		.and_then(std::ffi::OsStr::to_str)
		.is_some_and(|ext| {
			extensions
				.iter()
				.any(|candidate| ext.eq_ignore_ascii_case(candidate))
		})
}

fn is_html(path: &str) -> bool {
	has_extension(path, &["html", "htm"])
}

fn is_text_asset(path: &str) -> bool {
	has_extension(path, &["js", "mjs", "css"])
}

fn check_html(path: &str, text: &str, names: &BTreeSet<&str>, issues: &mut Vec<BundleIssue>) {
	let lower = text.to_ascii_lowercase();
	if path == ENTRY_HTML
		&& !lower.contains("name=\"viewport\"")
		&& !lower.contains("name='viewport'")
	{
		issues.push(BundleIssue {
			severity: Severity::Warning,
			code: "missing-viewport-meta".into(),
			path: Some(path.to_owned()),
			message: "index.html has no `<meta name=\"viewport\">` tag for portrait mobile play"
				.into(),
		});
	}

	for value in html_attr_values(text) {
		let trimmed = value.trim();
		if trimmed.is_empty()
			|| trimmed.starts_with('#')
			|| trimmed.starts_with("data:")
			|| trimmed.starts_with("blob:")
			|| trimmed.starts_with("javascript:")
			|| trimmed.starts_with("mailto:")
		{
			continue;
		}
		if is_external_url(trimmed) {
			issues.push(BundleIssue {
				severity: Severity::Error,
				code: "external-url".into(),
				path: Some(path.to_owned()),
				message: format!(
					"external URL `{trimmed}` is disallowed in offline MHCP bundles; bundle the \
					 dependency under `vendor/` with a relative path"
				),
			});
			continue;
		}
		if trimmed.starts_with('/') {
			issues.push(BundleIssue {
				severity: Severity::Error,
				code: "absolute-path".into(),
				path: Some(path.to_owned()),
				message: format!(
					"root-relative path `{trimmed}` breaks offline MHCP bundles; use a relative \
					 path such as `.{trimmed}`"
				),
			});
			continue;
		}
		let normalized = normalize_relative(path, trimmed);
		if !names.contains(normalized.as_str()) {
			issues.push(BundleIssue {
				severity: Severity::Error,
				code: "missing-referenced-asset".into(),
				path: Some(path.to_owned()),
				message: format!(
					"referenced relative asset `{trimmed}` (`{normalized}`) is missing from the \
					 bundle"
				),
			});
		}
	}

	check_external_urls(path, text, issues);
}

fn check_external_urls(path: &str, text: &str, issues: &mut Vec<BundleIssue>) {
	for marker in [
		"fetch(\"http://",
		"fetch('http://",
		"fetch(\"https://",
		"fetch('https://",
		"url(http://",
		"url(https://",
		"url(\"http://",
		"url(\"https://",
		"url('http://",
		"url('https://",
		"from \"http://",
		"from 'http://",
		"from \"https://",
		"from 'https://",
		"import(\"http://",
		"import('http://",
		"import(\"https://",
		"import('https://",
	] {
		if text.contains(marker) {
			issues.push(BundleIssue {
				severity: Severity::Error,
				code: "external-network-request".into(),
				path: Some(path.to_owned()),
				message: format!(
					"found external network reference (`{marker}...`); offline MHCP bundles must \
					 not load code or assets over the network"
				),
			});
		}
	}
}

fn is_external_url(value: &str) -> bool {
	value.starts_with("http://") || value.starts_with("https://") || value.starts_with("//")
}

/// Extracts `src="..."` and `href="..."` attribute values from HTML markup.
fn html_attr_values(html: &str) -> Vec<&str> {
	let mut values = Vec::new();
	for attr in ["src=", "href="] {
		let mut rest = html;
		while let Some(pos) = rest.find(attr) {
			let after = &rest[pos + attr.len()..];
			let Some(quote) = after.as_bytes().first().copied() else {
				break;
			};
			if quote != b'"' && quote != b'\'' {
				rest = after;
				continue;
			}
			let content = &after[1..];
			let Some(end) = content.find(quote as char) else {
				break;
			};
			values.push(&content[..end]);
			rest = &content[end + 1..];
		}
	}
	values
}

fn normalize_relative(base_file: &str, reference: &str) -> String {
	let clean = reference.split(['?', '#']).next().unwrap_or(reference);
	let mut segments: Vec<&str> = base_file
		.rsplit_once('/')
		.map_or(Vec::new(), |(dir, _)| dir.split('/').collect());
	for part in clean.split('/') {
		match part {
			"" | "." => {}
			".." => {
				segments.pop();
			}
			other => segments.push(other),
		}
	}
	segments.join("/")
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn accepts_a_self_contained_offline_bundle() {
		let files = vec![
			(
				"index.html".into(),
				b"<!doctype html><html><head><meta name=\"viewport\" content=\"width=device-width\"><script type=\"module\" src=\"./vendor/app.js\"></script></head><body></body></html>"
					.to_vec(),
			),
			("vendor/app.js".into(), b"console.log('ready');".to_vec()),
		];
		let report = validate_files(&files, MAX_BUNDLE_BYTES);
		assert!(report.is_valid(), "{report:?}");
		assert!(report.issues.is_empty(), "{report:?}");
	}

	#[test]
	fn rejects_missing_index_cdn_urls_and_absolute_paths() {
		let missing = validate_files(
			&[("nested/index.html".into(), b"<html></html>".to_vec())],
			MAX_BUNDLE_BYTES,
		);
		assert!(!missing.is_valid());
		assert!(
			missing
				.errors()
				.any(|issue| issue.code == "missing-index-html")
		);

		let cdn = validate_files(
			&[(
				"index.html".into(),
				b"<meta name=\"viewport\" content=\"width=device-width\"><script src=\"https://cdn.example.com/three.js\"></script>"
					.to_vec(),
			)],
			MAX_BUNDLE_BYTES,
		);
		assert!(!cdn.is_valid());
		assert!(cdn.errors().any(|issue| issue.code == "external-url"));

		let absolute = validate_files(
			&[(
				"index.html".into(),
				b"<meta name=\"viewport\" content=\"width=device-width\"><script src=\"/pkg/app.js\"></script>"
					.to_vec(),
			)],
			MAX_BUNDLE_BYTES,
		);
		assert!(!absolute.is_valid());
		assert!(absolute.errors().any(|issue| issue.code == "absolute-path"));
	}

	#[test]
	fn rejects_oversized_bundles_and_missing_local_assets() {
		let oversized = validate_files(&[("index.html".into(), vec![b' '; 64])], 32);
		assert!(!oversized.is_valid());
		assert!(
			oversized
				.errors()
				.any(|issue| issue.code == "bundle-too-large")
		);

		let broken = validate_files(
			&[(
				"index.html".into(),
				b"<meta name=\"viewport\" content=\"width=device-width\"><script src=\"./vendor/missing.js\"></script>"
					.to_vec(),
			)],
			MAX_BUNDLE_BYTES,
		);
		assert!(!broken.is_valid());
		assert!(
			broken
				.errors()
				.any(|issue| issue.code == "missing-referenced-asset")
		);
	}

	#[test]
	fn rewrites_root_relative_paths_without_touching_protocol_relative_urls() {
		let input = r#"<link href="/assets/app.css"><script src="/pkg/app.js"></script><script src="//cdn.example.com/x.js"></script>"#;
		let rewritten = rewrite_html_relative(input);
		assert!(
			rewritten.contains(r#"href="./assets/app.css""#),
			"{rewritten}"
		);
		assert!(rewritten.contains(r#"src="./pkg/app.js""#), "{rewritten}");
		assert!(
			rewritten.contains(r#"src="//cdn.example.com/x.js""#),
			"{rewritten}"
		);
	}
}
