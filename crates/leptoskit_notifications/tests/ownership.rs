//! Who owns an endpoint, and what an erasure leaves, on SQLite: a device
//! passing to its next person, a server-side revoke that a launch never
//! undoes, and `forget_person` erasing everything about a person — a long
//! history included, in batches — and the bounds on what a caller can
//! make the service read: a pinned page, and an attribute call's keys.
//! `tests/postgres.rs` runs the same checks on Postgres.

#![cfg(not(target_arch = "wasm32"))]

use leptoskit_db::Db;
use leptoskit_notifications::MIGRATIONS;

#[path = "support/engine.rs"]
#[allow(
	dead_code,
	reason = "the engine checks are shared with other test files"
)]
mod engine;
#[path = "support/ownership.rs"]
mod ownership;
#[path = "support/sql.rs"]
#[allow(
	dead_code,
	reason = "the SQL spellings are shared with other test files"
)]
mod sql;

/// A fresh, migrated database, and the service over it.
async fn service() -> (std::net::SocketAddr, Db) {
	let db = leptoskit_db::testing::sqlite_migrated(MIGRATIONS).await;
	let address = ownership::serve(&db).await;

	(address, db)
}

#[tokio::test(flavor = "multi_thread")]
async fn the_next_person_takes_a_device_over() {
	let (address, db) = service().await;
	ownership::the_next_person_takes_a_device_over(address, &db).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_side_revoke_sticks() {
	let (address, db) = service().await;
	ownership::a_server_side_revoke_sticks(address, &db).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn forgetting_erases_everything() {
	let (address, db) = service().await;
	ownership::forgetting_erases_everything(address, &db).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn forgetting_a_long_history_is_batched() {
	let (address, db) = service().await;
	ownership::forgetting_a_long_history_is_batched(address, &db).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pinned_page_is_bounded() {
	let (address, db) = service().await;
	ownership::a_pinned_page_is_bounded(address, &db).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_attribute_call_names_a_bounded_number_of_keys() {
	let (address, db) = service().await;
	ownership::an_attribute_call_names_a_bounded_number_of_keys(address, &db).await;
}

/// A fresh, migrated database, and a service over it that also speaks
/// FCM.
async fn service_with_fcm() -> (std::net::SocketAddr, Db) {
	let db = leptoskit_db::testing::sqlite_migrated(MIGRATIONS).await;
	let address = ownership::serve_with_fcm(&db).await;

	(address, db)
}

#[tokio::test(flavor = "multi_thread")]
async fn imported_endpoints_work_until_the_app_enrolls() {
	let (address, db) = service_with_fcm().await;
	ownership::imported_endpoints_work_until_the_app_enrolls(address, &db).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_import_is_bounded_and_ends_like_any_endpoint() {
	let (address, db) = service_with_fcm().await;
	ownership::an_import_is_bounded_and_ends_like_any_endpoint(address, &db).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_import_needs_the_platforms_transport() {
	let (address, db) = service().await;
	ownership::an_import_needs_the_platforms_transport(address, &db).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_endpoint_keeps_the_apns_environment_it_reported() {
	let (address, db) = service_with_fcm().await;
	ownership::an_endpoint_keeps_the_apns_environment_it_reported(address, &db).await;
}
