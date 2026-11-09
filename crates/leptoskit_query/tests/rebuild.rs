#![cfg(not(target_arch = "wasm32"))]

//! Screens that rebuild from their own data: every rebuild mounts its
//! queries again, and a mount right after a fetch must not fetch again.
mod common;

use std::time::Duration;

use common::Boom;
use common::Counter;
use common::owned;
use common::quiet_client;
use common::settle;
use common::wait_for;
use leptos::prelude::*;
use leptoskit_query::*;

/// A scope whose fetcher doubles the key and counts calls, stale at once.
fn doubler(calls: Counter) -> QueryScope<u32, u32, Boom> {
	QueryScope::new(move |key: u32| {
		let calls = calls.clone();
		async move {
			calls.hit();
			tokio::time::sleep(Duration::from_millis(2)).await;
			Ok(key * 2)
		}
	})
	.with_name("rebuild-doubler")
}

/// A scope whose fetcher always fails, without retrying.
fn failing(calls: Counter) -> QueryScope<u32, u32, Boom> {
	QueryScope::new(move |_key: u32| {
		let calls = calls.clone();
		async move {
			calls.hit();
			Err(Boom("down"))
		}
	})
	.with_name("rebuild-failing")
	.with_retry(Retry::never())
}

/// A screen the way leptoskit runs one: an effect that builds the whole view
/// again whenever a signal it read changes, cleaning up the last build's
/// observers first. It reads the query's data and error, as a screen that
/// shows either would.
fn screen(
	client: QueryClient,
	scope: QueryScope<u32, u32, Boom>,
	options: impl Fn() -> QueryObserverOptions<u32, Boom> + Send + Sync + 'static,
	builds: Counter,
) -> Owner {
	let (owner, ()) = owned(move || {
		Effect::new_isomorphic(move |_| {
			builds.hit();
			let query = use_query_in(client.clone(), scope.clone(), || 21, options());
			let _ = (query.data(), query.error());
		});
	});
	owner
}

#[tokio::test]
async fn a_screen_rebuilt_by_its_own_data_fetches_once() {
	let client = quiet_client();
	let calls = Counter::new();
	let builds = Counter::new();
	let owner = screen(
		client.clone(),
		doubler(calls.clone()),
		QueryObserverOptions::new,
		builds.clone(),
	);

	wait_for("the data", || {
		client.get_query_data(&doubler(Counter::new()), &21) == Some(42)
	})
	.await;
	for _ in 0..5 {
		settle().await;
	}

	assert_eq!(
		calls.get(),
		1,
		"the rebuild reuses the fetch that caused it"
	);
	assert_eq!(builds.get(), 2, "one build to load, one to show the data");
	owner.cleanup();
}

#[tokio::test]
async fn without_the_dedupe_window_the_same_screen_refetches_in_a_loop() {
	let client = quiet_client();
	let calls = Counter::new();
	let owner = screen(
		client,
		doubler(calls.clone()),
		|| QueryObserverOptions::new().with_mount_dedupe(Duration::ZERO),
		Counter::new(),
	);

	wait_for("a refetch loop", || calls.get() >= 4).await;
	owner.cleanup();
}

#[tokio::test]
async fn a_failed_query_is_not_retried_by_the_rebuild_its_error_caused() {
	let client = quiet_client();
	let calls = Counter::new();
	let builds = Counter::new();
	let owner = screen(
		client,
		failing(calls.clone()),
		QueryObserverOptions::new,
		builds.clone(),
	);

	wait_for("the error", || builds.get() >= 2).await;
	for _ in 0..5 {
		settle().await;
	}

	assert_eq!(calls.get(), 1, "retry on mount waits for the window too");
	assert_eq!(builds.get(), 2);
	owner.cleanup();
}

#[tokio::test]
async fn a_mount_after_the_window_refetches_stale_data() {
	let client = QueryClient::new().with_defaults(
		QueryDefaults::new()
			.with_refetch_on_window_focus(RefetchOn::Never)
			.with_refetch_on_reconnect(RefetchOn::Never)
			.with_network_mode(NetworkMode::Always)
			.with_mount_dedupe(Duration::from_millis(40)),
	);
	let calls = Counter::new();
	let scope = doubler(calls.clone());
	assert_eq!(client.fetch_query(&scope, 21).await, Ok(42));

	let (inside, _) = owned(|| {
		use_query_in(
			client.clone(),
			scope.clone(),
			|| 21,
			QueryObserverOptions::new(),
		)
	});
	settle().await;
	assert_eq!(calls.get(), 1, "a mount inside the window reuses the fetch");

	tokio::time::sleep(Duration::from_millis(60)).await;
	let (after, _) = owned(|| {
		use_query_in(
			client.clone(),
			scope.clone(),
			|| 21,
			QueryObserverOptions::new(),
		)
	});
	wait_for("the refetch", || calls.get() == 2).await;

	inside.cleanup();
	after.cleanup();
}

#[tokio::test]
async fn always_refetching_on_mount_still_waits_for_the_window() {
	let client = quiet_client();
	let calls = Counter::new();
	let scope = doubler(calls.clone());
	assert_eq!(client.fetch_query(&scope, 21).await, Ok(42));

	let options = || QueryObserverOptions::new().with_refetch_on_mount(RefetchOn::Always);
	let (inside, _) = owned(|| use_query_in(client.clone(), scope.clone(), || 21, options()));
	settle().await;
	assert_eq!(calls.get(), 1);

	let (outside, _) = owned(|| {
		use_query_in(
			client.clone(),
			scope.clone(),
			|| 21,
			options().with_mount_dedupe(Duration::ZERO),
		)
	});
	wait_for("the refetch", || calls.get() == 2).await;

	inside.cleanup();
	outside.cleanup();
}

#[tokio::test]
async fn an_invalidated_query_refetches_on_mount_inside_the_window() {
	let client = quiet_client();
	let calls = Counter::new();
	let scope = doubler(calls.clone());
	assert_eq!(client.fetch_query(&scope, 21).await, Ok(42));
	drop(client.invalidate_queries(QueryFilter::all()));

	let (owner, _) = owned(|| {
		use_query_in(
			client.clone(),
			scope.clone(),
			|| 21,
			QueryObserverOptions::new(),
		)
	});
	wait_for("the refetch", || calls.get() == 2).await;
	owner.cleanup();
}

#[tokio::test]
async fn data_written_by_hand_is_not_a_fetch_that_just_settled() {
	let client = quiet_client();
	let calls = Counter::new();
	let scope = doubler(calls.clone());
	client.set_query_data(&scope, 21, 7);

	let (owner, query) = owned(|| {
		use_query_in(
			client.clone(),
			scope.clone(),
			|| 21,
			QueryObserverOptions::new(),
		)
	});
	assert_eq!(query.data(), Some(7));
	wait_for("the fetch", || calls.get() == 1).await;
	owner.cleanup();
}

#[tokio::test]
async fn a_reset_query_forgets_its_last_fetch() {
	let client = quiet_client();
	let calls = Counter::new();
	let scope = doubler(calls.clone());
	assert_eq!(client.fetch_query(&scope, 21).await, Ok(42));
	drop(client.reset_queries(QueryFilter::all()));

	let (owner, _) = owned(|| {
		use_query_in(
			client.clone(),
			scope.clone(),
			|| 21,
			QueryObserverOptions::new(),
		)
	});
	wait_for("the fetch", || calls.get() == 2).await;
	owner.cleanup();
}
