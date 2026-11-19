//! The admin: the deploy console's control plane and its screens, signed
//! in through the site's identity provider, reading analytics over the
//! private loopback.
//!
//! Two runtime-only adjustments live here, and neither touches the admin
//! or console crates:
//!
//! - **In-process provider HTTP.** The admin reaches the main site's identity
//!   provider through the site's router, directly — never public DNS for a
//!   self-call.
//! - **Bounded operation streams.** An operation's progress is a process-local
//!   broadcast. In an overlap, an operation may finish in the old process, and
//!   a candidate's SSE stream for it would then wait forever for a `done` event
//!   that will never arrive locally. Successful authenticated operation streams
//!   therefore end after a bounded window; the browser's `EventSource`
//!   reconnects, and the fresh stream opens with the durable terminal snapshot
//!   the shared store holds. Authorization is untouched: only a 200
//!   event-stream response that already passed the admin's own gate is wrapped,
//!   and the wrapper buffers nothing — it only stops the stream when its window
//!   passes.

use std::time::Duration;

use anyhow::Result;
use axum::Router;
use axum::body::Body;
use axum::http::Request;
use axum::http::header::CONTENT_TYPE;
use axum::middleware::Next;
use axum::response::Response;
use futures::StreamExt as _;
use leptoskit_admin::Admin;
use leptoskit_admin::AnalyticsService;
use leptoskit_admin::MainSite;
use leptoskit_auth::Keys;
use leptoskit_auth::ProviderFuture;
use leptoskit_auth::ProviderHttp;
use leptoskit_auth::ProviderRequest;
use leptoskit_auth::ProviderResponse;
use tower::ServiceExt as _;

/// How long one successful operation stream may stay open before it ends
/// and the browser reconnects into the durable terminal state.
pub const OPERATION_SSE_WINDOW: Duration = Duration::from_secs(15);

/// The site's real routes, answering the admin's provider requests in
/// process.
#[derive(Clone)]
pub struct InProcess(Router);

impl ProviderHttp for InProcess {
	fn send(&self, request: ProviderRequest) -> ProviderFuture<'_> {
		let router = self.0.clone();
		Box::pin(async move {
			let url = url::Url::parse(&request.url).map_err(|error| error.to_string())?;
			let path_with_query = match url.query() {
				Some(query) => format!("{}?{query}", url.path()),
				None => url.path().to_owned(),
			};
			let mut builder = if request.method == "POST" {
				Request::post(path_with_query)
			} else {
				Request::get(path_with_query)
			};
			for (name, value) in &request.headers {
				builder = builder.header(name.as_str(), value.as_str());
			}
			// A router's service error is `Infallible`, so this cannot
			// panic.
			let response = router
				.oneshot(
					builder
						.body(Body::from(request.body.unwrap_or_default()))
						.map_err(|error| error.to_string())?,
				)
				.await
				.unwrap();
			let status = response.status().as_u16();
			let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
				.await
				.map_err(|error| error.to_string())?
				.to_vec();
			Ok(ProviderResponse { status, body })
		})
	}
}

/// Builds the admin's router for a generation.
///
/// # Errors
///
/// When the admin can't start (configuration, database, migrations).
#[allow(clippy::too_many_arguments, reason = "one call site, all real inputs")]
pub async fn build(
	admin_origin: &str,
	site_origin: &str,
	keys: Keys,
	client_id: &str,
	client_secret: &str,
	data_dir: std::path::PathBuf,
	control: leptoskit_deploy_console::control::Service,
	analytics_url: String,
	analytics_token: &str,
	site_router: Router,
) -> Result<Router> {
	let admin = Admin::builder("Notes")
		.origin(admin_origin)
		.main_site(MainSite::new(site_origin, client_id, client_secret).name("Notes"))
		.keys(keys)
		.data_dir(data_dir)
		.control(control)
		.analytics(AnalyticsService::new(analytics_url, analytics_token))
		// The executor really is simulated, and every screen says so.
		.simulated_infrastructure(true)
		.provider_http(InProcess(site_router))
		.build()
		.await
		.map_err(|error| anyhow::Error::new(error).context("building the admin"))?;

	Ok(wrap_operation_sse(admin.router()))
}

/// Ends successful authenticated operation event streams after
/// [`OPERATION_SSE_WINDOW`]. Everything else — every page, every refused
/// request, every other stream — passes through unchanged. Public so the
/// runtime's own tests can point the wrapper at a stand-in stream and
/// prove the window, the path scoping, and the auth-respecting passthrough
/// without an admin session.
pub fn wrap_operation_sse(router: Router) -> Router {
	router.layer(axum::middleware::from_fn(operation_stream_window))
}

async fn operation_stream_window(request: axum::extract::Request, next: Next) -> Response {
	let is_operation_stream = is_operation_events_path(request.uri().path());
	let response = next.run(request).await;

	if !is_operation_stream || !response.status().is_success() || !is_event_stream(&response) {
		return response;
	}

	let (parts, body) = response.into_parts();
	let bounded = time_limited(body.into_data_stream(), OPERATION_SSE_WINDOW);
	Response::from_parts(parts, Body::from_stream(bounded))
}

fn is_operation_events_path(path: &str) -> bool {
	let base = leptoskit_admin::BASE;
	path.starts_with(base)
		&& path[base.len()..].starts_with("/infra/operations/")
		&& path.ends_with("/events")
}

fn is_event_stream(response: &Response) -> bool {
	response
		.headers()
		.get(CONTENT_TYPE)
		.and_then(|value| value.to_str().ok())
		.is_some_and(|value| value.starts_with("text/event-stream"))
}

/// A stream that ends, rather than waits, once its window has passed.
fn time_limited<S>(stream: S, window: Duration) -> impl futures::Stream<Item = S::Item>
where
	S: futures::Stream + Send + 'static,
{
	let deadline = tokio::time::Instant::now() + window;
	// Box-pinned so the unfold state can move it in and out of every
	// poll: the deadline governs the whole stream, not one item.
	futures::stream::unfold(Box::pin(stream), move |mut stream| {
		async move {
			let remaining = deadline.checked_duration_since(tokio::time::Instant::now())?;
			match tokio::time::timeout(remaining, stream.as_mut().next()).await {
				// The window passed, or the stream finished on its own: end it.
				Err(_) | Ok(None) => None,
				Ok(Some(item)) => Some((item, stream)),
			}
		}
	})
}
