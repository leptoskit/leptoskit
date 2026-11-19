//! Translated variants and delivery in each person's local time, on
//! SQLite: each person's one inbox row and its push in their own language,
//! and a wall-clock time arriving zone by zone — visible and pushed at
//! once where it has passed, waiting where it is ahead, with one wake per
//! distinct instant and a cancel that stops what is still ahead.
//! `tests/postgres.rs` runs the same checks on Postgres.

#![cfg(not(target_arch = "wasm32"))]

use leptoskit_notifications::MIGRATIONS;

#[path = "support/engine.rs"]
#[allow(
	dead_code,
	reason = "the engine checks are shared with other test files"
)]
mod engine;
#[path = "support/local.rs"]
mod local;
#[path = "support/sql.rs"]
#[allow(
	dead_code,
	reason = "the SQL spellings are shared with other test files"
)]
mod sql;

/// A fresh, migrated database, and the service over it.
async fn service(workers: bool) -> local::Service {
	let db = leptoskit_db::testing::sqlite_migrated(MIGRATIONS).await;

	local::serve(&db, workers).await
}

#[tokio::test(flavor = "multi_thread")]
async fn each_person_reads_their_own_language() {
	local::each_person_reads_their_own_language(&service(true).await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_most_recently_seen_device_decides() {
	local::the_most_recently_seen_device_decides(&service(true).await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn variants_reach_a_topic_and_a_test_send() {
	local::variants_reach_a_topic_and_a_test_send(&service(true).await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn bad_variants_and_local_times_are_refused() {
	local::bad_variants_and_local_times_are_refused(&service(true).await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn each_person_gets_it_at_their_own_nine_o_clock() {
	local::each_person_gets_it_at_their_own_nine_o_clock(&service(true).await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_local_time_send_cancels_like_any_other_at_its_edges() {
	local::a_local_time_send_cancels_like_any_other_at_its_edges(&service(true).await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_fanout_wakes_once_per_batch_and_instant() {
	local::a_fanout_wakes_once_per_batch_and_instant(&service(false).await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_large_local_cancel_stops_only_what_is_ahead() {
	local::a_large_local_cancel_stops_only_what_is_ahead(&service(false).await).await;
}
