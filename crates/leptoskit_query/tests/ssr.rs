//! Server rendering streams a settled query, including an error, and the
//! browser's first read is that settled state.

#![cfg(not(target_arch = "wasm32"))]

mod common;

use std::sync::Arc;

use common::quiet_client;
use hydration_context::PinnedFuture;
use hydration_context::PinnedStream;
use hydration_context::SerializedDataId;
use hydration_context::SharedContext;
use hydration_context::SsrSharedContext;
use leptos::prelude::*;
use leptoskit_query::*;
use throw_error::Error;
use throw_error::ErrorId;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Miss(String);

fn scope(result: Result<u32, Miss>) -> QueryScope<u32, u32, Miss> {
	QueryScope::new(move |_key: u32| {
		let result = result.clone();
		async move { result }
	})
	.with_name("ssr")
	.with_retry(Retry::never())
	.with_ssr()
	.with_ssr_error()
}

/// What the server wrote for the query resource: the JSON of
/// `Option<DehydratedQuery>`, never the loading placeholder `null` once the
/// fetch has settled.
async fn server_payload(scope: QueryScope<u32, u32, Miss>) -> String {
	let client = quiet_client();
	let context = Arc::new(SsrSharedContext::new());
	let owner = Owner::new_root(Some(context.clone()));
	owner.with(|| {
		client.provide();
		let _query = use_query(scope, || 1);
	});
	let buffers = context.consume_buffers().await;
	owner.cleanup();
	assert_eq!(buffers.len(), 1, "one query resource: {buffers:?}");
	buffers.into_iter().next().expect("one resource").1
}

/// A browser shared context that answers `read_data` with the server payload.
struct Hydrating {
	payload: String,
}

impl std::fmt::Debug for Hydrating {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter.write_str("Hydrating")
	}
}

impl SharedContext for Hydrating {
	fn is_browser(&self) -> bool {
		true
	}

	fn next_id(&self) -> SerializedDataId {
		SerializedDataId::new(0)
	}

	fn write_async(&self, _id: SerializedDataId, _fut: PinnedFuture<String>) {}

	fn read_data(&self, _id: &SerializedDataId) -> Option<String> {
		Some(self.payload.clone())
	}

	fn await_data(&self, _id: &SerializedDataId) -> Option<String> {
		None
	}

	fn pending_data(&self) -> Option<PinnedStream<String>> {
		None
	}

	fn during_hydration(&self) -> bool {
		true
	}

	fn hydration_complete(&self) {}

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

	fn register_error(&self, _error_boundary: SerializedDataId, _error_id: ErrorId, _error: Error) {
	}

	fn defer_stream(&self, _wait_for: PinnedFuture<()>) {}

	fn await_deferred(&self) -> Option<PinnedFuture<()>> {
		None
	}

	fn set_incomplete_chunk(&self, _id: SerializedDataId) {}

	fn get_incomplete_chunk(&self, _id: &SerializedDataId) -> bool {
		false
	}
}

fn first_read(scope: QueryScope<u32, u32, Miss>, payload: String) -> QueryHandle<u32, Miss> {
	let client = quiet_client();
	let owner = Owner::new_root(Some(Arc::new(Hydrating { payload })));
	owner.with(|| {
		client.provide();
		use_query(scope, || 1)
	})
}

#[tokio::test]
async fn ssr_streams_a_settled_error_and_hydration_starts_there() {
	let payload = server_payload(scope(Err(Miss("nope".into())))).await;
	assert_ne!(payload, "null", "an error must not dehydrate as null");
	assert!(
		payload.contains("nope"),
		"the payload carries the error: {payload}"
	);

	let query = first_read(scope(Err(Miss("nope".into()))), payload);
	assert!(
		query.is_error(),
		"the first read is the server's error, not loading"
	);
	assert!(!query.is_loading());
	assert_eq!(query.error(), Some(Miss("nope".into())));
}

#[tokio::test]
async fn ssr_streams_settled_data_and_hydration_starts_there() {
	let payload = server_payload(scope(Ok(7))).await;
	assert_ne!(payload, "null");
	assert!(
		payload.contains('7'),
		"the payload carries the data: {payload}"
	);

	let query = first_read(scope(Ok(7)), payload);
	assert_eq!(query.data(), Some(7));
	assert!(!query.is_loading());
}
