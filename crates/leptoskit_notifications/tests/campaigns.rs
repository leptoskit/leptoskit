//! Campaigns on SQLite: a draft saved incomplete and revised under its
//! revision; a launch that makes one send however often it is pressed and
//! checks what a send checks; a past schedule refused; a cancel before
//! the send is due and a refusal after; copies without the schedule; a
//! test that writes no inbox row and records an event; archiving; who may
//! call any of it; what is bounded; and launches and saves that race.
//! `tests/postgres.rs` runs the same checks on Postgres.

#![cfg(not(target_arch = "wasm32"))]

use leptoskit_notifications::MIGRATIONS;

#[path = "support/composer.rs"]
#[allow(
	dead_code,
	reason = "the history's and the catalogue's checks are in the same file"
)]
mod composer;
#[path = "support/sql.rs"]
#[allow(
	dead_code,
	reason = "the SQL spellings are shared with other test files"
)]
mod sql;

/// A fresh, migrated database, and the service over it, with no workers:
/// nothing here waits for a job.
async fn service() -> composer::Service {
	let db = leptoskit_db::testing::sqlite_migrated(MIGRATIONS).await;

	composer::serve(&db, false).await
}

#[tokio::test(flavor = "multi_thread")]
async fn a_draft_is_saved_incomplete_and_revised() {
	composer::a_draft_is_saved_incomplete_and_revised(&service().await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_launch_makes_one_send_however_often_it_is_pressed() {
	composer::a_launch_makes_one_send_however_often_it_is_pressed(&service().await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_launch_checks_what_a_send_checks() {
	composer::a_launch_checks_what_a_send_checks(&service().await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_past_schedule_is_refused_and_a_future_one_is_kept() {
	composer::a_past_schedule_is_refused_and_a_future_one_is_kept(&service().await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancel_stops_a_scheduled_campaign_and_not_a_seen_one() {
	composer::a_cancel_stops_a_scheduled_campaign_and_not_a_seen_one(&service().await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_duplicate_is_a_new_draft_without_the_schedule() {
	composer::a_duplicate_is_a_new_draft_without_the_schedule(&service().await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_test_writes_no_inbox_row_and_records_an_event() {
	composer::a_test_writes_no_inbox_row_and_records_an_event(&service().await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_archived_campaign_is_put_away() {
	composer::an_archived_campaign_is_put_away(&service().await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn only_the_app_touches_campaigns() {
	composer::only_the_app_touches_campaigns(&service().await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn campaigns_are_bounded() {
	composer::campaigns_are_bounded(&service().await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn racing_launches_make_one_send() {
	composer::racing_launches_make_one_send(&service().await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn racing_saves_keep_one() {
	composer::racing_saves_keep_one(&service().await).await;
}
