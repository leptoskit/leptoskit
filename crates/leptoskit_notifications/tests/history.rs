//! Send history on SQLite: the list of sends, a page at a time and newest
//! first; every kind of send listed with its content; a send's report
//! with its spec, its audience, its push deliveries by state, and its
//! reads by the hour; who may read any of it; what a recipients send's
//! retry answers now that its spec is stored; and sends from before the
//! migration. `tests/postgres.rs` runs the same checks on Postgres.

#![cfg(not(target_arch = "wasm32"))]

use leptoskit_db::Db;
use leptoskit_notifications::MIGRATIONS;

#[path = "support/composer.rs"]
#[allow(
	dead_code,
	reason = "the other checks in the same file run from their own test files"
)]
mod composer;
#[path = "support/sql.rs"]
#[allow(
	dead_code,
	reason = "the SQL spellings are shared with other test files"
)]
mod sql;

/// A fresh, migrated database, and the service over it.
async fn service(workers: bool) -> composer::Service {
	let db = leptoskit_db::testing::sqlite_migrated(MIGRATIONS).await;

	composer::serve(&db, workers).await
}

#[tokio::test(flavor = "multi_thread")]
async fn the_list_pages_newest_first() {
	composer::the_list_pages_newest_first(&service(false).await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn each_kind_of_send_is_listed_with_its_content() {
	composer::each_kind_of_send_is_listed_with_its_content(&service(false).await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_report_describes_the_send_and_counts_its_pushes_and_reads() {
	composer::a_report_describes_the_send_and_counts_its_pushes_and_reads(&service(true).await)
		.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn only_the_app_reads_the_history() {
	composer::only_the_app_reads_the_history(&service(false).await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_retried_recipients_send_still_answers_correctly() {
	composer::a_retried_recipients_send_still_answers_correctly(&service(false).await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pruned_send_leaves_the_list() {
	composer::a_pruned_send_leaves_the_list(&service(false).await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn sends_from_before_the_history_are_not_listed() {
	let before = MIGRATIONS
		.iter()
		.position(|migration| migration.name.ends_with("_open_tracking"))
		.expect("the registry lists open_tracking")
		+ 1;
	let db = Db::connect("sqlite::memory:")
		.await
		.expect("SQLite opens an in-memory database");

	composer::sends_from_before_the_history_are_not_listed(&db, &MIGRATIONS[..before], MIGRATIONS)
		.await;
}
