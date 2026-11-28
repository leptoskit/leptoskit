//! The headers every page the server answers with carries: a browser reads
//! a page as the type it says, and draws it only where `frame-ancestors`
//! allows.
//!
//! A tenant's app is served on the console's own site — the example
//! configurations put both on `example.com` — so a browser sends the
//! console's session cookie to a page the app frames. A framed console page
//! is one click inside someone else's page: `/device` approves a terminal,
//! `/account` signs a device out. Every page is layered with
//! [`refuse_framing`], whichever layer of the server produced it.

use leptoskit_axum::axum::http::HeaderValue;
use leptoskit_axum::axum::http::header;
use leptoskit_axum::axum::response::Response;

/// Whether `response` is a page: the thing a browser may frame. A redirect
/// has no body to draw, and a font or the client's build is not a page.
pub(super) fn is_html(response: &Response) -> bool {
	response
		.headers()
		.get(header::CONTENT_TYPE)
		.and_then(|value| value.to_str().ok())
		.is_some_and(|value| value.starts_with("text/html"))
}

/// The layer every answer passes, the console's own pages and the control
/// plane's alike: [`harden`] as the response middleware takes it.
pub(crate) async fn refuse_framing(response: Response) -> Response {
	harden(response)
}

/// Marks `response`, when it is a page, as one no site may frame: a browser
/// refuses to draw it inside another page's frame, and reads it as the type
/// it says rather than guessing from its bytes.
///
/// A policy the page set itself is kept whole, gaining `frame-ancestors
/// 'none'` only when it names no `frame-ancestors`: the control plane's
/// pages keep the `script-src` and `form-action` they need.
pub(crate) fn harden(mut response: Response) -> Response {
	if !is_html(&response) {
		return response;
	}

	let headers = response.headers_mut();
	headers
		.entry(header::X_CONTENT_TYPE_OPTIONS)
		.or_insert(HeaderValue::from_static("nosniff"));

	let policy = headers
		.get(header::CONTENT_SECURITY_POLICY)
		.and_then(|value| value.to_str().ok())
		.map(str::to_owned);

	match policy {
		Some(policy) if names_frame_ancestors(&policy) => {}
		Some(policy) => {
			let mut hardened = policy.trim_end().to_owned();

			if !hardened.ends_with(';') {
				hardened.push(';');
			}

			hardened.push_str(" frame-ancestors 'none'");
			headers.insert(
				header::CONTENT_SECURITY_POLICY,
				HeaderValue::from_str(&hardened).expect("a policy with ASCII added is ASCII"),
			);
		}
		None => {
			headers.insert(
				header::CONTENT_SECURITY_POLICY,
				HeaderValue::from_static("frame-ancestors 'none'"),
			);
		}
	}

	response
}

/// Whether `policy` names a `frame-ancestors` directive: directive names
/// are case-insensitive, and a directive may carry its value beside it.
fn names_frame_ancestors(policy: &str) -> bool {
	policy.split(';').any(|directive| {
		directive
			.split_whitespace()
			.next()
			.is_some_and(|name| name.eq_ignore_ascii_case("frame-ancestors"))
	})
}

#[cfg(test)]
mod tests {
	use leptoskit_axum::axum::response::IntoResponse;

	use super::*;

	fn page(content_type: &'static str, policy: Option<&'static str>) -> Response {
		let mut response = ([(header::CONTENT_TYPE, content_type)], "a body").into_response();

		if let Some(policy) = policy {
			response.headers_mut().insert(
				header::CONTENT_SECURITY_POLICY,
				HeaderValue::from_static(policy),
			);
		}

		let hardened = harden(response);
		assert_eq!(
			hardened.headers()[header::X_CONTENT_TYPE_OPTIONS],
			"nosniff",
			"{content_type}"
		);
		hardened
	}

	#[test]
	fn a_page_with_no_policy_is_given_one() {
		let response = page("text/html; charset=utf-8", None);
		assert_eq!(
			response.headers()[header::CONTENT_SECURITY_POLICY],
			"frame-ancestors 'none'"
		);
	}

	#[test]
	fn a_page_with_a_policy_keeps_it_and_gains_ancestors() {
		let response = page(
			"text/html",
			Some("default-src 'none'; script-src 'sha256-abc'; form-action 'self'"),
		);
		assert_eq!(
			response.headers()[header::CONTENT_SECURITY_POLICY],
			"default-src 'none'; script-src 'sha256-abc'; form-action 'self'; frame-ancestors \
			 'none'"
		);
	}

	#[test]
	fn a_policy_that_already_names_ancestors_is_kept_as_it_is() {
		let response = page(
			"text/html",
			Some("frame-ancestors 'self'; form-action 'self'"),
		);
		assert_eq!(
			response.headers()[header::CONTENT_SECURITY_POLICY],
			"frame-ancestors 'self'; form-action 'self'"
		);
	}

	#[test]
	fn a_response_that_is_not_a_page_is_left_alone() {
		let mut response = ([(header::CONTENT_TYPE, "font/woff2")], "a font").into_response();
		response.headers_mut().insert(
			header::CACHE_CONTROL,
			HeaderValue::from_static("public, max-age=31536000, immutable"),
		);

		let response = harden(response);
		assert!(
			!response
				.headers()
				.contains_key(header::CONTENT_SECURITY_POLICY)
		);
		assert!(
			!response
				.headers()
				.contains_key(header::X_CONTENT_TYPE_OPTIONS)
		);
		assert_eq!(
			response.headers()[header::CACHE_CONTROL],
			"public, max-age=31536000, immutable"
		);
	}
}
