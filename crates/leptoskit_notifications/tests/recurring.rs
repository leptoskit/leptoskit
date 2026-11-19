//! Recurring sends on SQLite: one send per occurrence when its time
//! comes, and exactly one however many workers find it due; the latest
//! missed occurrence and no backlog after an outage; rules that end by
//! count and by date; pause, resume, and end; a refused occurrence
//! pausing its recurrence; who may call any of it;
//! what is bounded; delivery at each person's own time; erasure;
//! retention; and the service's own jobs finding what is due. No check
//! waits for the clock: each tells the service what time it is.
//! `tests/postgres.rs` runs the same checks on Postgres.

#![cfg(not(target_arch = "wasm32"))]

use leptoskit_notifications::MIGRATIONS;

#[path = "support/composer.rs"]
#[allow(
	dead_code,
	reason = "the history's, the catalogue's, and the campaigns' checks are in the same file"
)]
mod composer;
#[path = "support/sql.rs"]
#[allow(
	dead_code,
	reason = "the SQL spellings are shared with other test files"
)]
mod sql;

/// A fresh, migrated database, and the service over it, with no workers:
/// every check here says what time it is, or runs the jobs itself.
async fn service() -> composer::Service {
	let db = leptoskit_db::testing::sqlite_migrated(MIGRATIONS).await;

	composer::serve(&db, false).await
}

#[tokio::test(flavor = "multi_thread")]
async fn an_occurrence_is_one_send_when_its_time_comes() {
	composer::an_occurrence_is_one_send_when_its_time_comes(&service().await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn racing_runs_make_one_send_per_occurrence() {
	composer::racing_runs_make_one_send_per_occurrence(&service().await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_recurrence_catches_up_with_its_latest_occurrence_only() {
	composer::a_recurrence_catches_up_with_its_latest_occurrence_only(&service().await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_recurrence_ends_by_count_and_by_date() {
	composer::a_recurrence_ends_by_count_and_by_date(&service().await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_recurrence_is_paused_resumed_and_ended() {
	composer::a_recurrence_is_paused_resumed_and_ended(&service().await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refused_occurrence_pauses_the_recurrence() {
	composer::a_refused_occurrence_pauses_the_recurrence(&service().await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn only_the_app_touches_recurrences() {
	composer::only_the_app_touches_recurrences(&service().await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn recurrences_are_bounded_and_refused_by_name() {
	composer::recurrences_are_bounded_and_refused_by_name(&service().await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_local_time_recurrence_arrives_at_each_persons_own_time() {
	composer::a_local_time_recurrence_arrives_at_each_persons_own_time(&service().await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn forgetting_a_person_takes_them_out_of_a_recurrence() {
	composer::forgetting_a_person_takes_them_out_of_a_recurrence(&service().await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn ended_recurrences_are_swept_with_send_records() {
	composer::ended_recurrences_are_swept_with_send_records(&service().await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_job_makes_what_is_due_and_schedules_the_next() {
	composer::the_job_makes_what_is_due_and_schedules_the_next(&service().await).await;
}
