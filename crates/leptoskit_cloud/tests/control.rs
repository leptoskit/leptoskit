//! The control plane on a real port over SQLite: sign-up and personal
//! organisations, API tokens, invitations, membership rules, tenant
//! isolation, projects and environments, the audit log, the CLI's device
//! flow, and operators. The scenarios are in `support/scenarios.rs`, which
//! `postgres.rs` runs on Postgres too; the authorisation matrix is in
//! `authorization.rs`.

#![cfg(not(target_arch = "wasm32"))]

mod support;

use support::scenarios;

#[tokio::test]
async fn a_personal_organization_is_made_once() {
	scenarios::a_personal_organization_is_made_once(&support::app().await).await;
}

#[tokio::test]
async fn anonymous_callers_reach_nothing() {
	scenarios::anonymous_callers_reach_nothing(&support::app().await).await;
}

#[tokio::test]
async fn tokens_are_shown_once_and_stop_when_revoked() {
	scenarios::tokens_are_shown_once_and_stop_when_revoked(&support::app().await).await;
}

#[tokio::test]
async fn tokens_are_never_stronger_than_their_maker() {
	scenarios::tokens_are_never_stronger_than_their_maker(&support::app().await).await;
}

#[tokio::test]
async fn invitations_work_once_for_their_address() {
	scenarios::invitations_work_once_for_their_address(&support::app().await).await;
}

#[tokio::test]
async fn an_organization_keeps_an_owner() {
	scenarios::an_organization_keeps_an_owner(&support::app().await).await;
}

#[tokio::test]
async fn tenants_are_isolated() {
	scenarios::tenants_are_isolated(&support::app().await).await;
}

#[tokio::test]
async fn projects_and_environments() {
	scenarios::projects_and_environments(&support::app().await).await;
}

#[tokio::test]
async fn the_audit_log_names_the_actor_and_no_secrets() {
	scenarios::the_audit_log_names_the_actor_and_no_secrets(&support::app().await).await;
}

#[tokio::test]
async fn the_cli_signs_in_with_the_device_flow() {
	scenarios::the_cli_signs_in_with_the_device_flow(&support::app().await).await;
}

#[tokio::test]
async fn operators_are_granted_on_the_command_line() {
	scenarios::operators_are_granted_on_the_command_line(&support::app().await).await;
}

/// The CLI's client is registered once, however often the server starts.
#[tokio::test]
async fn the_cli_client_is_registered_once() {
	let db = leptoskit_db::testing::sqlite_migrated(leptoskit_cloud::MIGRATIONS).await;
	let first = support::app_on(db.clone()).await;
	let second = support::app_on(db).await;
	assert_eq!(
		first.control.cli_client_id(),
		second.control.cli_client_id()
	);
	let clients = second.control.idp().clients().await.expect("clients");
	assert_eq!(clients.len(), 1);
	assert_eq!(clients[0].name, leptoskit_cloud::server::CLI_CLIENT_NAME);
	assert!(clients[0].first_party);
}
