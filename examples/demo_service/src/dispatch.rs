//! One listener, three authorities, and the fallback that serves before a
//! generation exists.
//!
//! Dispatch is by exact configured authority, validated the same way on
//! every request: a request names exactly one `Host` header, and that
//! header names exactly one authority — the site origin's host goes to
//! the site, the admin origin's host goes to the admin, and this
//! process's own private loopback address serves only health checks and
//! the token-authenticated analytics service prefix. Anything else — no
//! usable `Host`, two of them, or one spelled any way this service
//! cannot mean — is refused with 421, before the generation exists and
//! after.
//!
//! Before initialization commits, every authority gets the fallback:
//! `/healthz` answers 200, `/readyz` names the failing generation check,
//! and ordinary pages answer 503 with `Retry-After` and a styled "opening
//! today's demo" page that works without scripts, so the edge never routes
//! a visitor into a half-open generation. Once the generation is
//! set — exactly once, into a `OnceLock` — those same authorities dispatch
//! into its routers.

use axum::Router;
use axum::extract::Request;
use axum::http::HeaderValue;
use axum::http::StatusCode;
use axum::http::header;
use axum::response::IntoResponse as _;
use axum::response::Response;
use tower::ServiceExt as _;

use crate::config::Authority;
use crate::generation::Generation;
use crate::marker::GENERATION_FORMAT;
use crate::pages;

/// The path of the generation event stream, served on the public hosts
/// only.
const GENERATION_PATH: &str = "/demo/generation";

/// The router every request enters by.
pub fn router() -> Router {
	Router::new().fallback(handle)
}

async fn handle(request: Request) -> Response {
	let Some(authority) = host_of(&request) else {
		eprintln!("refusing a request with no single usable Host header");
		return misdirected();
	};

	let Some(authorities) = crate::AUTHORITIES.get() else {
		eprintln!("refusing {authority}: this process has no authorities yet");
		return misdirected();
	};
	if !authorities.allows(&authority) {
		eprintln!(
			"refusing {authority}: this process answers for {} {} and {}",
			authorities.site, authorities.admin, authorities.bind
		);
		return misdirected();
	}

	let Some(generation) = crate::GENERATION.get() else {
		return fallback(request.uri().path());
	};

	if authorities.bind.matches(&authority) {
		// The private loopback answers health checks and the
		// token-authenticated analytics service, and nothing else.
		return serve(&generation.loopback, request).await;
	}

	let public = if authorities.site.matches(&authority) {
		Some(&generation.site)
	} else if authorities.admin.matches(&authority) {
		Some(&generation.admin)
	} else {
		None
	};

	if request.uri().path() == GENERATION_PATH {
		return generation_stream(generation);
	}

	match public {
		Some(router) => serve(router, request).await,
		None => misdirected(),
	}
}

/// The request's one authority: exactly one `Host` header, holding one
/// value the grammar accepts. A missing, duplicated, or unusable header
/// names no authority — it is never resolved by silently taking one of
/// several values.
fn host_of(request: &Request) -> Option<Authority> {
	let mut hosts = request.headers().get_all(header::HOST).iter();
	let value = hosts.next()?;
	if hosts.next().is_some() {
		return None;
	}
	value.to_str().ok().and_then(Authority::of_host_header)
}

/// Sends a request into one of the generation's routers.
async fn serve(router: &Router, request: Request) -> Response {
	// A router's service error is `Infallible`, so this cannot panic.
	router.clone().oneshot(request).await.unwrap()
}

/// What every authority serves before the generation exists, and what the
/// process keeps serving if initialization fails: alive, not ready, and
/// honest about when to come back.
fn fallback(path: &str) -> Response {
	match path {
		"/healthz" => (StatusCode::OK, "ok").into_response(),
		"/readyz" => readiness_report(),
		_ => opening_page(),
	}
}

/// What a visitor sees while the generation builds: a styled page that
/// works without scripts, asks for nothing else, and checks again by
/// itself. The status and `Retry-After` stay what a proxy or crawler
/// reads; the page says only that the demo is not ready, never that a
/// reset happened.
fn opening_page() -> Response {
	let mut response = (
		StatusCode::SERVICE_UNAVAILABLE,
		[
			(header::CONTENT_TYPE, "text/html; charset=utf-8"),
			(header::CACHE_CONTROL, "no-store"),
		],
		pages::opening(),
	)
		.into_response();
	response.headers_mut().insert(
		header::RETRY_AFTER,
		HeaderValue::from(u64::from(pages::RETRY_SECONDS)),
	);
	response
}

/// The readiness report the fallback serves: the generation check fails
/// until initialization commits. Once a generation exists, `/readyz` is
/// answered by the routers' own readiness routes, never here.
fn readiness_report() -> Response {
	let body = serde_json::json!({ "ready": false, "failing": ["generation"] });
	let mut response = (StatusCode::SERVICE_UNAVAILABLE, body.to_string()).into_response();
	response.headers_mut().insert(
		header::CONTENT_TYPE,
		HeaderValue::from_static("application/json"),
	);
	response
		.headers_mut()
		.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
	response
}

fn misdirected() -> Response {
	(
		StatusCode::MISDIRECTED_REQUEST,
		"this service does not answer for that host",
	)
		.into_response()
}

/// The generation event stream: one `generation` event with the day and
/// format, then keep-alives, capped at the generation's permit count.
/// Permits live inside the stream, so a disconnect or a reset's drop
/// returns its permit at once.
fn generation_stream(generation: &Generation) -> Response {
	let Ok(permit) = generation.permits().clone().try_acquire_owned() else {
		let mut response = (
			StatusCode::SERVICE_UNAVAILABLE,
			"too many generation streams; reconnect shortly",
		)
			.into_response();
		response
			.headers_mut()
			.insert(header::RETRY_AFTER, HeaderValue::from_static("5"));
		return response;
	};

	let event = axum::response::sse::Event::default()
		.event("generation")
		.data(
			serde_json::json!({
				"day": generation.day,
				"format": GENERATION_FORMAT,
			})
			.to_string(),
		);

	let stream = futures::stream::unfold(
		(
			Some(event),
			permit,
			tokio::time::interval(tokio::time::Duration::from_secs(1)),
		),
		|(first, permit, mut interval)| {
			async move {
				if let Some(event) = first {
					return Some((
						Ok::<_, std::convert::Infallible>(event),
						(None, permit, interval),
					));
				}
				interval.tick().await;
				Some((
					Ok(axum::response::sse::Event::default().comment("keep-alive")),
					(None, permit, interval),
				))
			}
		},
	);

	axum::response::sse::Sse::new(stream).into_response()
}

#[cfg(test)]
mod tests {
	use axum::body::Body;

	use super::*;

	/// Routes one in-memory request carrying exactly the named Host
	/// values, and returns its status.
	async fn status_of(hosts: &[&str]) -> StatusCode {
		let mut builder = Request::builder();
		for host in hosts {
			builder = builder.header(header::HOST, *host);
		}
		let request = builder.body(Body::empty()).expect("the request builds");
		router()
			.oneshot(request)
			.await
			.expect("the router answers")
			.status()
	}

	#[tokio::test]
	async fn a_request_without_a_host_header_names_no_authority() {
		assert_eq!(
			status_of(&[]).await,
			StatusCode::MISDIRECTED_REQUEST,
			"no Host at all is refused before routing"
		);
	}

	/// Two Host values name two authorities; the request must not be
	/// resolved by silently taking one of them.
	#[tokio::test]
	async fn two_host_headers_name_no_authority() {
		assert_eq!(
			status_of(&["site.example", "stranger.example"]).await,
			StatusCode::MISDIRECTED_REQUEST
		);
	}

	/// Every value the authority grammar refuses is refused before
	/// routing, not resolved leniently.
	#[tokio::test]
	async fn hosts_that_cannot_mean_an_authority_are_refused() {
		for host in [
			"[::1]garbage",
			"[::1]:bad",
			"[::1]:0",
			"::1",
			"user@site.example",
			"site.example:",
			"site.example:0",
			"site.example:99999",
			"site.example/a",
			"site.example?q",
			"site.example#f",
			"site.example,a.example",
			"site.example..",
			"site exam ple",
			"   ",
		] {
			assert_eq!(
				status_of(&[host]).await,
				StatusCode::MISDIRECTED_REQUEST,
				"{host:?} names no authority"
			);
		}
	}

	/// While no generation exists, a page is a styled 503 that checks
	/// again by itself and runs no script, and the health routes keep
	/// their meaning.
	#[tokio::test]
	async fn the_fallback_page_is_styled_safe_and_honest() {
		let response = fallback("/");
		assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
		assert_eq!(response.headers()[header::RETRY_AFTER], "5");
		assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
		assert!(
			response.headers()[header::CONTENT_TYPE]
				.to_str()
				.expect("a content type")
				.starts_with("text/html")
		);
		let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
			.await
			.expect("a body");
		let html = String::from_utf8_lossy(&body);
		assert!(
			html.contains("Notes is opening today&#39;s demo.")
				|| html.contains("Notes is opening today's demo.")
		);
		assert!(html.contains("http-equiv=\"refresh\" content=\"5\""));
		assert!(!html.contains("<script"), "the page needs no script");
		assert!(!html.contains("reset happened"), "it never claims a reset");

		assert_eq!(fallback("/healthz").status(), StatusCode::OK);
		let readiness = fallback("/readyz");
		assert_eq!(readiness.status(), StatusCode::SERVICE_UNAVAILABLE);
		assert_eq!(
			readiness.headers()[header::CONTENT_TYPE],
			"application/json"
		);
	}
}
