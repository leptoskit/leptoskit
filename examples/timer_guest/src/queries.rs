//! Real query operations on the guest executor, without custom timer providers.

use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;
use std::time::Duration;

use leptoskit_query::FocusManager;
use leptoskit_query::GcTime;
use leptoskit_query::NetworkMode;
use leptoskit_query::QueryClient;
use leptoskit_query::QueryDefaults;
use leptoskit_query::QueryObserverOptions;
use leptoskit_query::QueryScope;
use leptoskit_query::RefetchOn;
use leptoskit_query::Retry;
use leptoskit_query::now_ms;
use leptoskit_query::try_sleep;
use leptoskit_query::use_query_in;
use reactive_graph::owner::Owner;

fn client() -> QueryClient {
	QueryClient::new().with_defaults(
		QueryDefaults::new()
			.with_network_mode(NetworkMode::Always)
			.with_refetch_on_window_focus(RefetchOn::Never)
			.with_refetch_on_reconnect(RefetchOn::Never)
			.with_gc_time(GcTime::Never),
	)
}

fn flaky(attempts: &Arc<AtomicU32>) -> QueryScope<(), u32, &'static str> {
	let attempts = Arc::clone(attempts);
	QueryScope::new(move |()| {
		let attempt = attempts.fetch_add(1, Ordering::SeqCst) + 1;
		async move {
			if attempt < 3 {
				Err("flaky")
			} else {
				Ok(attempt)
			}
		}
	})
	.with_name("timer-probe-retries")
	.with_retry(Retry::count(2))
	.with_retry_delay(Duration::from_millis(10))
}

pub async fn without_timer() {
	let client = client();
	let attempts = Arc::new(AtomicU32::new(0));
	let scope = flaky(&attempts).with_gc_time(Duration::from_millis(1));
	assert_eq!(client.fetch_query(&scope, ()).await, Err("flaky"));
	assert_eq!(
		attempts.load(Ordering::SeqCst),
		1,
		"no immediate retry loop"
	);
	// Let queued GC run. Missing timers must not be mistaken for expiry.
	any_spawner::Executor::tick().await;
	assert_eq!(
		client.query_infos().len(),
		1,
		"failed timer preserves cache"
	);
	client.clear();
}

pub async fn with_timer() {
	let client = client();
	let attempts = Arc::new(AtomicU32::new(0));
	let scope = flaky(&attempts).with_gc_time(Duration::from_millis(25));
	let start = now_ms();
	assert_eq!(client.fetch_query(&scope, ()).await, Ok(3));
	assert_eq!(attempts.load(Ordering::SeqCst), 3);
	assert!(now_ms() >= start + 20, "two retries really waited");
	assert_eq!(client.get_query_data(&scope, &()), Some(3));

	try_sleep(Duration::from_millis(50)).await.expect("GC wait");
	assert!(client.query_infos().is_empty(), "unobserved entry expired");
	intervals(&client).await;
	client.clear();
	// Drain cancelled interval/GC futures before the host checks for leaks.
	any_spawner::Executor::tick().await;
}

async fn intervals(client: &QueryClient) {
	let calls = Arc::new(AtomicU32::new(0));
	let counter = Arc::clone(&calls);
	let scope: QueryScope<(), u32, &'static str> = QueryScope::new(move |()| {
		let count = counter.fetch_add(1, Ordering::SeqCst) + 1;
		async move { Ok(count) }
	})
	.with_name("timer-probe-interval")
	.with_refetch_interval(Duration::from_millis(10));
	let owner = Owner::new();
	let query =
		owner.with(|| use_query_in(client.clone(), scope, || (), QueryObserverOptions::new()));
	wait_for_calls(&calls, 3).await;
	assert!(query.data().is_some());

	FocusManager::set_focused(Some(false));
	let background = calls.load(Ordering::SeqCst);
	try_sleep(Duration::from_millis(35))
		.await
		.expect("background wait");
	assert_eq!(
		calls.load(Ordering::SeqCst),
		background,
		"background polling pauses"
	);
	FocusManager::set_focused(Some(true));
	wait_for_calls(&calls, background + 1).await;
	owner.cleanup();
	let unmounted = calls.load(Ordering::SeqCst);
	try_sleep(Duration::from_millis(35))
		.await
		.expect("unmounted wait");
	assert_eq!(
		calls.load(Ordering::SeqCst),
		unmounted,
		"unmounted polling stops"
	);
}

async fn wait_for_calls(calls: &AtomicU32, minimum: u32) {
	for _ in 0..100 {
		if calls.load(Ordering::SeqCst) >= minimum {
			return;
		}

		try_sleep(Duration::from_millis(5))
			.await
			.expect("interval wait");
	}

	panic!("interval made no progress");
}
