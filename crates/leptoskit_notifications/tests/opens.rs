//! Open tracking, a send's report, and a send's label, on SQLite: the
//! first open of a row recorded once and marking it read, only by its
//! owner; the report's counts of what became of a send's rows, and what
//! they say once retention has pruned those rows; a label checked, stored,
//! and part of the idempotent intent; and rows from before the migration.
//! `tests/postgres.rs` runs the same checks on Postgres.

#![cfg(not(target_arch = "wasm32"))]

use leptoskit_db::Db;
use leptoskit_notifications::MIGRATIONS;

#[path = "support/opens.rs"]
mod opens;
#[path = "support/sql.rs"]
#[allow(
	dead_code,
	reason = "the SQL spellings are shared with other test files"
)]
mod sql;

/// A fresh, migrated database, and the service over it.
async fn service() -> (std::net::SocketAddr, Db) {
	let db = leptoskit_db::testing::sqlite_migrated(MIGRATIONS).await;
	let address = opens::serve(&db).await;

	(address, db)
}

#[tokio::test(flavor = "multi_thread")]
async fn the_first_open_is_recorded_and_marks_the_row_read() {
	let (address, db) = service().await;
	opens::the_first_open_is_recorded_and_marks_the_row_read(address, &db).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn only_the_owner_opens_a_row() {
	let (address, db) = service().await;
	opens::only_the_owner_opens_a_row(address, &db).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_data_message_has_no_open() {
	let (address, db) = service().await;
	opens::a_data_message_has_no_open(address, &db).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_report_counts_what_became_of_a_sends_rows() {
	let (address, db) = service().await;
	opens::a_report_counts_what_became_of_a_sends_rows(address, &db).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reports_counts_shrink_when_rows_are_pruned() {
	let (address, db) = service().await;
	opens::a_reports_counts_shrink_when_rows_are_pruned(address, &db).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_label_is_checked_stored_and_part_of_the_intent() {
	let (address, db) = service().await;
	opens::a_label_is_checked_stored_and_part_of_the_intent(address, &db).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn rows_from_before_the_migration_are_unopened() {
	let before = MIGRATIONS
		.iter()
		.position(|migration| migration.name.ends_with("_endpoint_seen"))
		.expect("the registry lists endpoint_seen")
		+ 1;
	let db = Db::connect("sqlite::memory:")
		.await
		.expect("SQLite opens an in-memory database");

	opens::rows_from_before_the_migration_are_unopened(&db, &MIGRATIONS[..before], MIGRATIONS)
		.await;
}
