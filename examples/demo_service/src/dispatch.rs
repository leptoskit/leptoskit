//! One listener, three authorities, and the fallback that serves before a
//! generation exists.
//!
//! Dispatch is by exact configured authority, validated the same way on
//! every request: the site origin's host goes to the site, the admin
//! origin's host goes to the admin, and this process's own private
//! loopback address serves only health checks and the token-authenticated
//! analytics service prefix. Anything else — any other `Host`, spelled
//! any way — is refused with 421, before the generation exists and after.
//!
//! Before initialization commits, every authority gets the fallback:
//! `/healthz` answers 200, `/readyz` names the failing generation check,
//! and ordinary pages answer 503 with `Retry-After`, so the edge never
//! routes a visitor into a half-open generation. Once the generation is
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

/// The path of the generation event stream, served on the public hosts
/// only.
const GENERATION_PATH: &str = "/demo/generation";

/// The router every request enters by.
pub fn router() -> Router {
	Router::new().fallback(handle)
}

async fn handle(request: Request) -> Response {
	let authority = request
		.headers()
		.get(header::HOST)
		.and_then(|value| value.to_str().ok())
		.and_then(Authority::of_host_header);
	let Some(authority) = authority else {
		eprintln!("refusing a request with no usable Host header");
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
		_ => {
			let mut response = (
				StatusCode::SERVICE_UNAVAILABLE,
				"The demo is resetting. Back in a few seconds.",
			)
				.into_response();
			response
				.headers_mut()
				.insert(header::RETRY_AFTER, HeaderValue::from_static("5"));
			response
		}
	}
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
