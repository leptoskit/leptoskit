//! Return targets: where a sign-in may send a visitor back to.
//!
//! A `return_to` is never trusted. The only honest answers are the local
//! paths this site actually serves, so the validator is an allowlist of
//! exact paths — never a prefix filter, which `//evil.example` and
//! `/\evil.example` both walk past. Everything else falls back to
//! [`FALLBACK`], so a caller can carry the value around and follow it
//! after the sign-in completes.

/// The fallback for every refused target: the app's own page.
pub const FALLBACK: &str = "/notes";

/// The longest accepted target, in bytes.
const MAX_LEN: usize = 2048;

/// The paths a target may name before its query, exactly.
const PATHS: [&str; 3] = ["/", "/notes", "/oauth/authorize"];

/// The `path` part of a target: everything before its query.
fn path_of(target: &str) -> &str {
	target.split_once('?').map_or(target, |(path, _)| path)
}

/// Validates an untrusted return target as one of this site's local
/// paths. A refused target becomes [`FALLBACK`]; the answer is always a
/// local path a caller may follow or render.
///
/// The checks, in order:
///
/// 1. At most [`MAX_LEN`] printable ASCII bytes, with no space, no control, and
///    no backslash — a backslash is a path delimiter on Windows and an escape
///    in JSON and HTML, so it never travels.
/// 2. One leading slash, and never a second one: `//host` is a
///    protocol-relative URL for another site.
/// 3. The path before the query is exactly one of [`PATHS`]. A percent encoding
///    cannot make another path pass, because comparison is by the raw bytes.
///
/// The query rides along untouched, so a legitimate
/// `/oauth/authorize?client_id=…` survives intact.
#[must_use]
pub fn local_return_to(raw: &str) -> String {
	let usable = raw.len() <= MAX_LEN
		&& raw.starts_with('/')
		&& !raw[1..].starts_with('/')
		&& !raw[1..].starts_with('\\')
		&& raw
			.bytes()
			.all(|byte| (0x21..=0x7E).contains(&byte) && byte != b'\\')
		&& PATHS.contains(&path_of(raw));

	if usable {
		raw.to_owned()
	} else {
		FALLBACK.to_owned()
	}
}

/// Serializes a validated target into a `Location` query value,
/// percent-encoded with the standard form encoding. Never interpolate a
/// raw target into a header, HTML, or JavaScript.
#[must_use]
pub fn encoded_query_value(target: &str) -> String {
	url::form_urlencoded::Serializer::new(String::new())
		.append_pair("return_to", target)
		.finish()
		.trim_start_matches("return_to=")
		.to_owned()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn local_paths_pass_with_their_queries() {
		for target in [
			"/notes",
			"/",
			"/oauth/authorize",
			"/oauth/authorize?client_id=abc&scope=openid%20roles&state=st",
			"/notes?a=1&b=2",
		] {
			assert_eq!(local_return_to(target), target, "{target} is accepted");
		}
	}

	#[test]
	fn foreign_and_dangerous_targets_fall_back() {
		for target in [
			"",
			"notes",
			"//evil.example",
			"/\\evil.example",
			"\\evil.example",
			"/%2F%5Cevil",
			"%5C%5Cevil",
			"/notes%2F..%2Fadmin",
			"https://evil.example",
			"http://evil.example/notes",
			"javascript:alert(1)",
			"//",
			"/notes\n",
			"/notes\r",
			"/notes x",
			"/admin",
			"/signin",
			"/oauth/authorize/extra",
		] {
			assert_eq!(
				local_return_to(target),
				FALLBACK,
				"{target:?} is refused to the fallback"
			);
		}

		// The root with a query is still the root: its path is allowed,
		// and its query rides along.
		assert_eq!(local_return_to("/?q=1"), "/?q=1");
	}

	#[test]
	fn overlength_falls_back_and_the_boundary_passes() {
		let at_the_limit = format!("/notes?{}", "a".repeat(MAX_LEN - "/notes?".len()));
		assert_eq!(local_return_to(&at_the_limit), at_the_limit);

		let over = format!("/notes?{}", "a".repeat(MAX_LEN - "/notes?".len() + 1));
		assert_eq!(local_return_to(&over), FALLBACK);
	}

	#[test]
	fn controls_and_non_ascii_fall_back() {
		assert_eq!(local_return_to("/notesé"), FALLBACK);
		assert_eq!(local_return_to("/notes\t"), FALLBACK);
		assert_eq!(local_return_to("/notes\0"), FALLBACK);
	}

	#[test]
	fn encoding_is_the_standard_form_encoding() {
		assert_eq!(encoded_query_value("/notes"), "%2Fnotes");
		assert_eq!(
			encoded_query_value("/oauth/authorize?client_id=abc"),
			"%2Foauth%2Fauthorize%3Fclient_id%3Dabc"
		);
	}
}
