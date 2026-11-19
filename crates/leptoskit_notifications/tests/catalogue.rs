//! The audience catalogue on SQLite: the overview's bounded counts, the
//! topics, attribute keys and values, and categories an audience can be
//! built from, the audience estimate — exact below its cap, and saying so
//! above it — a person's endpoints for the app, and that no answer
//! carries a credential. `tests/postgres.rs` runs the same checks on
//! Postgres.

#![cfg(not(target_arch = "wasm32"))]

use leptoskit_notifications::MIGRATIONS;

#[path = "support/composer.rs"]
#[allow(
	dead_code,
	reason = "the history's checks are in the same file and run from tests/history.rs"
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
async fn the_overview_counts_what_the_service_knows() {
	composer::the_overview_counts_what_the_service_knows(&service().await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn topics_are_listed_with_their_subscribers() {
	composer::topics_are_listed_with_their_subscribers(&service().await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn attributes_are_listed_with_their_values() {
	composer::attributes_are_listed_with_their_values(&service().await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn categories_are_listed_with_opt_outs_and_last_send() {
	composer::categories_are_listed_with_opt_outs_and_last_send(&service().await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_estimate_is_exact_below_its_cap_and_says_so_above_it() {
	composer::an_estimate_is_exact_below_its_cap_and_says_so_above_it(&service().await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_persons_endpoints_are_listed_without_credentials() {
	composer::a_persons_endpoints_are_listed_without_credentials(&service().await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn only_the_app_reads_the_catalogue() {
	composer::only_the_app_reads_the_catalogue(&service().await).await;
}
