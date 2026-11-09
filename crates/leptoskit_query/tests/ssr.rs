//! Server rendering and hydration: what the server's render streams for a
//! query, and that the browser's first render from it matches the server's.
//!
//! The server side runs under leptos's own `SsrSharedContext`. The browser
//! side runs under a stand-in context that serves the server's payloads the
//! way `__RESOLVED_RESOURCES` does, so hydration is tested without a
//! browser.

#![cfg(not(target_arch = "wasm32"))]

mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::task::Waker;
use std::time::Duration;

use common::*;
use futures::StreamExt;
use hydration_context::PinnedFuture;
use hydration_context::PinnedStream;
use hydration_context::SerializedDataId;
use hydration_context::SharedContext;
use hydration_context::SsrSharedContext;
use leptos::error::Error;
use leptos::error::ErrorId;
use leptos::prelude::*;
use leptos::reactive::computed::suspense::SuspenseContext;
use leptoskit_query::*;
use serde::Deserialize;
use serde::Serialize;

/// A serializable error, as a server function's would be.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Failure(String);

/// The browser's side of hydration: hands out ids in render order and
/// serves what the server streamed for each.
#[derive(Debug, Default)]
struct Browser {
	id: AtomicUsize,
	resolved: HashMap<String, String>,
	hydrated: AtomicBool,
}

impl Browser {
	fn new(resolved: Vec<(SerializedDataId, String)>) -> Arc<Self> {
		Arc::new(Self {
			resolved: resolved
				.into_iter()
				.map(|(id, data)| (id.into_inner(), data))
				.collect(),
			..Self::default()
		})
	}
}

impl SharedContext for Browser {
	fn is_browser(&self) -> bool {
		true
	}

	fn next_id(&self) -> SerializedDataId {
		SerializedDataId::new(self.id.fetch_add(1, Ordering::Relaxed))
	}

	fn write_async(&self, _id: SerializedDataId, _fut: PinnedFuture<String>) {}

	fn read_data(&self, id: &SerializedDataId) -> Option<String> {
		if !self.during_hydration() {
			return None;
		}

		self.resolved.get(id.as_str()).cloned()
	}

	fn await_data(&self, _id: &SerializedDataId) -> Option<String> {
		None
	}

	fn pending_data(&self) -> Option<PinnedStream<String>> {
		None
	}

	fn during_hydration(&self) -> bool {
		!self.hydrated.load(Ordering::SeqCst)
	}

	fn hydration_complete(&self) {
		self.hydrated.store(true, Ordering::SeqCst);
	}

	fn get_is_hydrating(&self) -> bool {
		true
	}

	fn set_is_hydrating(&self, _is_hydrating: bool) {}

	fn take_errors(&self) -> Vec<(SerializedDataId, ErrorId, Error)> {
		Vec::new()
	}

	fn errors(&self, _boundary_id: &SerializedDataId) -> Vec<(ErrorId, Error)> {
		Vec::new()
	}

	fn seal_errors(&self, _boundary_id: &SerializedDataId) {}

	fn register_error(&self, _boundary: SerializedDataId, _error_id: ErrorId, _error: Error) {}

	fn defer_stream(&self, _wait_for: PinnedFuture<()>) {}

	fn await_deferred(&self) -> Option<PinnedFuture<()>> {
		None
	}

	fn set_incomplete_chunk(&self, _id: SerializedDataId) {}

	fn get_incomplete_chunk(&self, _id: &SerializedDataId) -> bool {
		false
	}
}

/// The context a `<Suspense>` boundary gives its children.
// The task set's type comes from a crate this one does not depend on.
#[allow(clippy::default_trait_access)]
fn suspense_boundary() -> SuspenseContext {
	SuspenseContext::new(ArcRwSignal::new(Default::default()))
}

/// Whether something is holding the boundary.
fn is_held(suspense: &SuspenseContext) -> bool {
	!suspense.poll_empty(Waker::noop())
}

/// What a screen renders from a query's state, as the HTML it would emit.
/// Every field a view might branch on is in it, so any difference between
/// the server's render and the browser's first render shows.
fn render(query: &QueryHandle<u32, Failure>) -> String {
	format!(
		"status={:?} fetch={:?} loading={} data={:?} error={:?} failures={}",
		query.status(),
		query.fetch_status(),
		query.is_loading(),
		query.data(),
		query.error(),
		query.failure_count(),
	)
}

/// A scope whose fetcher answers `answer` after a short wait, counting calls.
fn scope(calls: Counter, answer: Result<u32, Failure>) -> QueryScope<u32, u32, Failure> {
	QueryScope::new(move |_key: u32| {
		calls.hit();
		let answer = answer.clone();
		async move {
			tokio::time::sleep(Duration::from_millis(5)).await;
			answer
		}
	})
	.with_name("ssr")
	.with_retry(Retry::never())
	.with_ssr()
}

/// Renders the query on the server once it has settled, as a page inside
/// `<Suspense>` does, returning the HTML and what the server streamed.
async fn render_on_server(
	scope: QueryScope<u32, u32, Failure>,
) -> (String, Vec<(SerializedDataId, String)>) {
	init();
	let context = Arc::new(SsrSharedContext::new());
	let owner = Owner::new_root(Some(context.clone()));
	let query =
		owner.with(|| use_query_in(QueryClient::new(), scope, || 1, QueryObserverOptions::new()));
	wait_for("the server's fetch settles", || {
		owner.with(|| query.fetch_status()) == FetchStatus::Idle
	})
	.await;
	let html = owner.with(|| render(&query));
	let streamed = context.consume_buffers().await;
	owner.cleanup();
	(html, streamed)
}

/// Hydrates the query in the browser from what the server streamed,
/// returning the browser's context, its owner, and the handle.
fn hydrate_in_browser(
	scope: QueryScope<u32, u32, Failure>,
	streamed: Vec<(SerializedDataId, String)>,
) -> (Arc<Browser>, Owner, QueryHandle<u32, Failure>) {
	// Made outside the owner: `quiet_client` resets the current owner.
	let client = quiet_client();
	let browser = Browser::new(streamed);
	let owner = Owner::new_root(Some(browser.clone()));
	let query = owner.with(|| use_query_in(client, scope, || 1, QueryObserverOptions::new()));
	(browser, owner, query)
}

#[tokio::test]
async fn a_server_error_hydrates_as_the_same_error() {
	let server_calls = Counter::new();
	let failure = Err(Failure("not found".to_owned()));
	let (server_html, streamed) =
		render_on_server(scope(server_calls.clone(), failure.clone()).with_ssr_errors()).await;
	assert!(server_html.contains("status=Error"), "{server_html}");
	assert_eq!(streamed.len(), 1);
	assert!(streamed[0].1.contains("not found"), "{:?}", streamed[0].1);

	let calls = Counter::new();
	let (browser, owner, query) =
		hydrate_in_browser(scope(calls.clone(), failure).with_ssr_errors(), streamed);
	let browser_html = owner.with(|| render(&query));
	assert_eq!(
		browser_html, server_html,
		"the browser's first render matches the server's"
	);

	browser.hydration_complete();
	wait_for("the error is retried once hydrated", || calls.get() == 1).await;
	wait_for("the retry settles", || {
		owner.with(|| query.fetch_status()) == FetchStatus::Idle
	})
	.await;
	assert_eq!(
		owner.with(|| query.error()),
		Some(Failure("not found".to_owned()))
	);

	owner.cleanup();
}

#[tokio::test]
async fn an_error_is_streamed_only_when_the_scope_opts_in() {
	let failure = Err(Failure("not found".to_owned()));
	let (server_html, streamed) = render_on_server(scope(Counter::new(), failure.clone())).await;
	assert!(server_html.contains("status=Error"), "{server_html}");
	assert_eq!(streamed.len(), 1);
	assert_eq!(streamed[0].1, "null", "the error stays on the server");

	let calls = Counter::new();
	let (_browser, owner, query) = hydrate_in_browser(scope(calls.clone(), failure), streamed);
	assert!(
		owner.with(|| query.is_loading()),
		"with nothing to hydrate, the browser loads at once"
	);
	wait_for("the browser fetches", || calls.get() == 1).await;

	owner.cleanup();
}

#[tokio::test]
async fn fresh_server_data_hydrates_without_a_refetch() {
	let fresh = |calls| scope(calls, Ok(7)).with_stale_time(Duration::from_secs(60));
	let (server_html, streamed) = render_on_server(fresh(Counter::new())).await;
	assert!(server_html.contains("data=Some(7)"), "{server_html}");

	let calls = Counter::new();
	let (browser, owner, query) = hydrate_in_browser(fresh(calls.clone()), streamed);
	assert_eq!(owner.with(|| render(&query)), server_html);

	browser.hydration_complete();
	settle().await;
	assert_eq!(calls.get(), 0, "fresh hydrated data is not fetched again");
	assert_eq!(owner.with(|| query.data()), Some(7));

	owner.cleanup();
}

#[tokio::test]
async fn stale_server_data_refetches_only_after_hydration() {
	let (server_html, streamed) = render_on_server(scope(Counter::new(), Ok(7))).await;

	let calls = Counter::new();
	let (browser, owner, query) = hydrate_in_browser(scope(calls.clone(), Ok(8)), streamed);
	assert_eq!(
		owner.with(|| render(&query)),
		server_html,
		"a background refetch does not start during hydration"
	);

	browser.hydration_complete();
	wait_for("the stale data is refetched", || {
		owner.with(|| query.data()) == Some(8)
	})
	.await;
	assert_eq!(calls.get(), 1);

	owner.cleanup();
}

#[tokio::test]
async fn an_unmounted_query_skips_its_deferred_fetch() {
	let (_, streamed) = render_on_server(scope(Counter::new(), Ok(7))).await;

	let calls = Counter::new();
	let (browser, owner, _query) = hydrate_in_browser(scope(calls.clone(), Ok(8)), streamed);
	owner.cleanup();
	browser.hydration_complete();
	settle().await;
	assert_eq!(calls.get(), 0, "nobody is left to show the refetch");
}

#[tokio::test]
async fn streamed_errors_cannot_close_the_script_they_are_embedded_in() {
	let message = "</script><script>alert(1)</script> & \u{2028}\u{2029}".to_owned();
	let failure = Err(Failure(message.clone()));
	let (_, streamed) =
		render_on_server(scope(Counter::new(), failure.clone()).with_ssr_errors()).await;
	let payload = &streamed[0].1;

	for unsafe_character in ['<', '>', '&', '\u{2028}', '\u{2029}'] {
		assert!(
			!payload.contains(unsafe_character),
			"{unsafe_character:?} is escaped in {payload}"
		);
	}

	let (_browser, owner, query) =
		hydrate_in_browser(scope(Counter::new(), failure).with_ssr_errors(), streamed);
	assert_eq!(
		owner.with(|| query.error()),
		Some(Failure(message)),
		"the escapes decode to the original message"
	);

	owner.cleanup();
}

#[tokio::test]
async fn the_streamed_document_never_contains_the_payload_raw() {
	init();
	let context = Arc::new(SsrSharedContext::new());
	let owner = Owner::new_root(Some(context.clone()));
	let failure = Failure("</script><img src=x onerror=alert(1)>".to_owned());
	let query = owner.with(|| {
		use_query_in(
			QueryClient::new(),
			scope(Counter::new(), Err(failure)).with_ssr_errors(),
			|| 1,
			QueryObserverOptions::new(),
		)
	});
	let mut chunks = context.pending_data().expect("the server streams data");
	let mut document = String::new();

	while let Some(chunk) = chunks.next().await {
		document.push_str(&chunk);
	}

	assert!(document.contains("__RESOLVED_RESOURCES"), "{document}");
	assert!(document.contains("onerror"), "the error is in {document}");
	assert!(
		!document.contains('<'),
		"nothing can open a tag: {document}"
	);
	assert_eq!(owner.with(|| query.status()), QueryStatus::Error);

	owner.cleanup();
}

#[tokio::test]
async fn reads_on_the_server_hold_suspense_until_the_fetch_settles() {
	init();
	let context = Arc::new(SsrSharedContext::new());
	let owner = Owner::new_root(Some(context.clone()));
	let gates = Gates::new();
	let suspense = suspense_boundary();
	let query = owner.with(|| {
		provide_context(suspense.clone());
		use_query_in(
			QueryClient::new(),
			gates.scope("gated").with_ssr(),
			|| 1,
			QueryObserverOptions::new(),
		)
	});
	assert!(owner.with(|| query.is_loading()));
	assert!(
		is_held(&suspense),
		"reading a loading query holds the boundary"
	);

	wait_for("the server fetches", || gates.calls() == 1).await;
	gates.answer(0, Ok(3));
	wait_for("the boundary is released", || !is_held(&suspense)).await;
	assert_eq!(owner.with(|| query.data()), Some(3));
	owner.with(|| query.data());
	assert!(!is_held(&suspense), "a settled query holds nothing");

	owner.cleanup();
}

#[tokio::test]
async fn reads_in_the_browser_never_hold_suspense() {
	let gates = Gates::new();
	let client = quiet_client();
	let browser = Browser::new(Vec::new());
	let owner = Owner::new_root(Some(browser));
	let suspense = suspense_boundary();
	let query = owner.with(|| {
		provide_context(suspense.clone());
		use_query_in(
			client,
			gates.scope("gated").with_ssr(),
			|| 1,
			QueryObserverOptions::new(),
		)
	});
	assert!(owner.with(|| query.is_loading()));
	assert!(
		!is_held(&suspense),
		"the browser shows the query's own loading state"
	);

	owner.cleanup();
}
