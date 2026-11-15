//! The operation-stream window: the runtime-only fix that lets an admin
//! page's `EventSource` reconnect into the durable terminal state after a
//! worker handoff.
//!
//! The admin crate's SSE endpoint requires a signed-in admin session,
//! which this slice deliberately cannot provide — there is no inbox yet —
//! so this proof points the runtime's wrapper at stand-in streams with
//! the same shape (same path, same 200 event-stream response that only an
//! authenticated handler can produce) and verifies:
//!
//! 1. a 200 event stream on an operation-events path ends within the window, so
//!    the browser reconnects;
//! 2. everything else passes through byte for byte, however long;
//! 3. an auth refusal at the *actual* operation-events path is untouched — the
//!    wrapper never sits in front of the auth handler's refusals.
//!
//! What stays explicitly pending: a real authenticated overlap test — a signed
//! in admin's `EventSource`, in a browser, watching an operation finish in
//! another process and reconnecting into the durable terminal state. That
//! needs the onboarding slice's real admin session; the stand-ins here prove
//! the wrapper's behavior only, and a direct Store read is not an SSE proof.

/// The pending proof, so it is named where it will be written: see the
/// module docs. Runs only with `--ignored`, and fails loudly until the
/// onboarding slice provides a real admin session.
#[test]
#[ignore = "needs the onboarding slice's real admin session; a Store read is not an SSE proof"]
fn an_authenticated_browser_reconnects_across_a_worker_handoff() {
	panic!("pending: drive the real admin session's EventSource across a worker handoff");
}

use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::StatusCode;
use axum::response::IntoResponse as _;
use axum::response::sse::Event;
use axum::response::sse::Sse;
use axum::routing::get;
use demo_service::admin;
use futures::StreamExt as _;
use tower::ServiceExt as _;

/// The window under test, plus margin for scheduling.
const WINDOW: Duration = admin::OPERATION_SSE_WINDOW;
const MARGIN: Duration = Duration::from_secs(5);

/// An endless 200 event stream at the operation-events path, a
/// never-ending plain 200 at another path, and an auth-shaped refusal at
/// the operation-events path itself: the handler answers 401 for the id
/// `denied`, exactly as the admin's auth gate would on a matching path.
fn stand_in() -> Router {
	let events = get(|axum::extract::Path(id): axum::extract::Path<String>| {
		async move {
			// The auth gate's refusal, at the real path shape: the wrapper
			// must pass it through without softening or buffering it.
			if id == "denied" {
				return StatusCode::UNAUTHORIZED.into_response();
			}
			let stream = futures::stream::unfold(0_u32, |tick| {
				async move {
					tokio::time::sleep(Duration::from_millis(50)).await;
					Some((
						Ok::<_, std::convert::Infallible>(
							Event::default()
								.event("progress")
								.data(format!(r#"{{"tick":{tick}}}"#)),
						),
						tick + 1,
					))
				}
			});
			Sse::new(stream).into_response()
		}
	});
	// A 200 whose body never ends, and is not an event stream: the
	// wrapper must leave it alone.
	let endless_plain = get(|| {
		async {
			let stream = futures::stream::unfold(0_u32, |tick| {
				async move {
					tokio::time::sleep(Duration::from_millis(50)).await;
					Some((
						Ok::<_, std::convert::Infallible>(axum::body::Bytes::from("chunk")),
						tick + 1,
					))
				}
			});
			axum::response::Response::new(Body::from_stream(stream))
		}
	});

	let base = leptoskit_admin::BASE;
	Router::new()
		.route(&format!("{base}/infra/operations/{{id}}/events"), events)
		.route(&format!("{base}/infra/operations/nope"), endless_plain)
}

#[tokio::test]
async fn an_operation_stream_ends_within_the_window() {
	let wrapped = admin::wrap_operation_sse(stand_in());
	let path = format!(
		"{}{}/events",
		leptoskit_admin::BASE,
		"/infra/operations/op-1"
	);
	let response = wrapped
		.oneshot(
			axum::http::Request::builder()
				.method("GET")
				.uri(path)
				.body(Body::empty())
				.expect("the request builds"),
		)
		.await
		.expect("the router answers");
	assert_eq!(response.status(), StatusCode::OK);

	let started = std::time::Instant::now();
	let stream = response.into_body().into_data_stream();
	let events = count_events_until_closed(stream).await;
	let elapsed = started.elapsed();

	assert!(
		elapsed >= WINDOW.saturating_sub(Duration::from_millis(200)),
		"the stream lives out most of its window: {elapsed:?}"
	);
	assert!(
		elapsed < WINDOW + MARGIN,
		"the stream ends near the window, not long after: {elapsed:?}"
	);
	assert!(
		events > 1,
		"events flowed before the window closed: {events}"
	);
}

#[tokio::test]
async fn other_bodies_pass_through_untouched() {
	let wrapped = admin::wrap_operation_sse(stand_in());

	// A 200 response that is not an event stream stays open past the
	// window.
	let path = format!("{}{}/nope", leptoskit_admin::BASE, "/infra/operations");
	let response = wrapped
		.clone()
		.oneshot(
			axum::http::Request::builder()
				.method("GET")
				.uri(path)
				.body(Body::empty())
				.expect("the request builds"),
		)
		.await
		.expect("the router answers");
	assert_eq!(response.status(), StatusCode::OK);

	let started = std::time::Instant::now();
	let stream = response.into_body().into_data_stream();
	let bytes = tokio::time::timeout(
		WINDOW + MARGIN + Duration::from_secs(1),
		collect_all(stream),
	)
	.await;
	assert!(
		bytes.is_err(),
		"a plain 200 was still open past the window, as it must be"
	);
	assert!(
		started.elapsed() >= WINDOW + MARGIN,
		"the timeout, not the body, ended the wait"
	);

	// A refusal at the *actual* operation-events path — a path the
	// wrapper's predicate matches — passes through with its status and
	// its body, so the auth handler's answer is never softened.
	let path = format!(
		"{}{}/events",
		leptoskit_admin::BASE,
		"/infra/operations/denied"
	);
	let response = wrapped
		.oneshot(
			axum::http::Request::builder()
				.method("GET")
				.uri(path)
				.body(Body::empty())
				.expect("the request builds"),
		)
		.await
		.expect("the router answers");
	assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
	let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
		.await
		.expect("the refusal's body arrives immediately");
	assert!(
		body.is_empty(),
		"the refusal is passed through, not wrapped in a stream"
	);
}

/// Reads the stream to its end, counting the SSE event frames that
/// arrived.
async fn count_events_until_closed(stream: axum::body::BodyDataStream) -> usize {
	let mut events = 0;
	let mut pinned = std::pin::pin!(stream);
	while let Some(frame) = pinned.next().await {
		let bytes = frame.expect("the stream's frames are readable");
		if String::from_utf8_lossy(&bytes).contains("event: progress") {
			events += 1;
		}
	}
	events
}

/// Collects every byte until the body ends.
async fn collect_all(stream: axum::body::BodyDataStream) -> usize {
	let mut pinned = std::pin::pin!(stream);
	let mut total = 0;
	while let Some(frame) = pinned.next().await {
		total += frame.expect("the stream's frames are readable").len();
	}
	total
}
