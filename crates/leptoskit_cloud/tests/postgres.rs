//! The core of `control.rs` on a private, throwaway Postgres: started from
//! local binaries, no credentials, no network. Postgres aborts a
//! transaction on a unique violation, so the races (personal organisations
//! made at once) and the conflicts (taken slugs, stale revisions) take
//! different paths than on SQLite.
//!
//! Without Postgres binaries the tests are skipped, unless
//! `LEPTOSKIT_REQUIRE_POSTGRES` is set, as the database crate's own tests
//! are.

#![cfg(not(target_arch = "wasm32"))]

mod support;

#[path = "support/postgres.rs"]
mod postgres;

use support::App;
use support::scenarios;

/// A control plane over a fresh, migrated database on its own Postgres,
/// which stops when the returned guard drops, or `None` when there is no
/// Postgres to run on.
async fn app() -> Option<(postgres::Postgres, App)> {
	let server = postgres::Postgres::start()?;
	let url = server.database();
	let db = leptoskit_db::Db::connect(&url).await.expect("connect");
	db.migrate(leptoskit_cloud::POSTGRES_MIGRATIONS)
		.await
		.expect("the Postgres migrations apply");
	Some((server, support::app_on(db).await))
}

#[tokio::test]
async fn a_personal_organization_is_made_once() {
	if let Some((_postgres, app)) = app().await {
		scenarios::a_personal_organization_is_made_once(&app).await;
	}
}

#[tokio::test]
async fn tokens_are_shown_once_and_stop_when_revoked() {
	if let Some((_postgres, app)) = app().await {
		scenarios::tokens_are_shown_once_and_stop_when_revoked(&app).await;
	}
}

#[tokio::test]
async fn tokens_are_never_stronger_than_their_maker() {
	if let Some((_postgres, app)) = app().await {
		scenarios::tokens_are_never_stronger_than_their_maker(&app).await;
	}
}

#[tokio::test]
async fn invitations_work_once_for_their_address() {
	if let Some((_postgres, app)) = app().await {
		scenarios::invitations_work_once_for_their_address(&app).await;
	}
}

#[tokio::test]
async fn an_organization_keeps_an_owner() {
	if let Some((_postgres, app)) = app().await {
		scenarios::an_organization_keeps_an_owner(&app).await;
	}
}

#[tokio::test]
async fn tenants_are_isolated() {
	if let Some((_postgres, app)) = app().await {
		scenarios::tenants_are_isolated(&app).await;
	}
}

#[tokio::test]
async fn projects_and_environments() {
	if let Some((_postgres, app)) = app().await {
		scenarios::projects_and_environments(&app).await;
	}
}

#[tokio::test]
async fn the_cli_signs_in_with_the_device_flow() {
	if let Some((_postgres, app)) = app().await {
		scenarios::the_cli_signs_in_with_the_device_flow(&app).await;
	}
}

#[tokio::test]
async fn tokens_never_grant_membership() {
	if let Some((_postgres, app)) = app().await {
		scenarios::tokens_never_grant_membership(&app).await;
	}
}

#[tokio::test]
async fn an_invitation_needs_its_inviters_authority() {
	if let Some((_postgres, app)) = app().await {
		scenarios::an_invitation_needs_its_inviters_authority(&app).await;
	}
}

#[tokio::test]
async fn replacing_an_invitation_needs_authority_over_it() {
	if let Some((_postgres, app)) = app().await {
		scenarios::replacing_an_invitation_needs_authority_over_it(&app).await;
	}
}

#[tokio::test]
async fn personal_organizations_cannot_be_squatted() {
	if let Some((_postgres, app)) = app().await {
		scenarios::personal_organizations_cannot_be_squatted(&app).await;
	}
}

#[tokio::test]
async fn token_secrets_are_never_stored() {
	if let Some((_postgres, app)) = app().await {
		scenarios::token_secrets_are_never_stored(&app).await;
	}
}

#[tokio::test]
async fn the_last_owner_stays_under_concurrent_changes() {
	if let Some((_postgres, app)) = app().await {
		scenarios::the_last_owner_stays_under_concurrent_changes(&app).await;
	}
}

#[tokio::test]
async fn caps_hold_under_concurrent_creates() {
	if let Some((_postgres, app)) = app().await {
		scenarios::caps_hold_under_concurrent_creates(&app).await;
	}
}

#[tokio::test]
async fn a_members_tokens_follow_their_role() {
	if let Some((_postgres, app)) = app().await {
		scenarios::a_members_tokens_follow_their_role(&app).await;
	}
}

/// The shipping scenarios on Postgres: its serializable transactions
/// refuse the claim races and the in-flight races differently.
mod shipping {
	use super::app;
	use super::support::shipping;

	#[tokio::test]
	async fn a_deployment_goes_live() {
		if let Some((_postgres, app)) = app().await {
			shipping::a_deployment_goes_live(&app).await;
		}
	}

	#[tokio::test]
	async fn the_cluster_reports_what_it_runs() {
		if let Some((_postgres, app)) = app().await {
			shipping::the_cluster_reports_what_it_runs(&app).await;
		}
	}

	#[tokio::test]
	async fn a_lapsed_build_is_retried_then_abandoned() {
		if let Some((_postgres, app)) = app().await {
			shipping::a_lapsed_build_is_retried_then_abandoned(&app).await;
		}
	}

	#[tokio::test]
	async fn a_lapsed_release_waits_for_reconciliation() {
		if let Some((_postgres, app)) = app().await {
			shipping::a_lapsed_release_waits_for_reconciliation(&app).await;
		}
	}

	#[tokio::test]
	async fn cancelling_stops_a_deployment() {
		if let Some((_postgres, app)) = app().await {
			shipping::cancelling_stops_a_deployment(&app).await;
		}
	}

	#[tokio::test]
	async fn uploads_are_verified_per_organization() {
		if let Some((_postgres, app)) = app().await {
			shipping::uploads_are_verified_per_organization(&app).await;
		}
	}

	#[tokio::test]
	async fn deploying_needs_a_cluster() {
		if let Some((_postgres, app)) = app().await {
			shipping::deploying_needs_a_cluster(&app).await;
		}
	}

	#[tokio::test]
	async fn workers_pair_once_and_can_be_revoked() {
		if let Some((_postgres, app)) = app().await {
			shipping::workers_pair_once_and_can_be_revoked(&app).await;
		}
	}

	#[tokio::test]
	async fn deleting_an_environment_tears_it_down() {
		if let Some((_postgres, app)) = app().await {
			shipping::deleting_an_environment_tears_it_down(&app).await;
		}
	}

	#[tokio::test]
	async fn build_logs_are_ordered_paged_and_expire() {
		if let Some((_postgres, app)) = app().await {
			shipping::build_logs_are_ordered_paged_and_expire(&app).await;
		}
	}

	#[tokio::test]
	async fn a_build_that_forgets_migrations_fails() {
		if let Some((_postgres, app)) = app().await {
			shipping::a_build_that_forgets_migrations_fails(&app).await;
		}
	}

	#[tokio::test]
	async fn races_have_one_winner() {
		if let Some((_postgres, app)) = app().await {
			shipping::races_have_one_winner(&app).await;
		}
	}
}

/// What Control 2's security review found (`support/review.rs`), on
/// Postgres.
mod review {
	use super::app;
	use super::postgres;
	use super::support;
	use super::support::review;

	#[tokio::test]
	async fn an_upload_keeps_the_size_it_was_begun_with() {
		if let Some((_postgres, app)) = app().await {
			review::an_upload_keeps_the_size_it_was_begun_with(&app).await;
		}
	}

	#[tokio::test]
	async fn a_full_queue_never_hides_a_job_a_worker_may_take() {
		if let Some((_postgres, app)) = app().await {
			review::a_full_queue_never_hides_a_job_a_worker_may_take(&app).await;
		}
	}

	#[tokio::test]
	async fn a_build_attempt_has_a_deadline() {
		if let Some((_postgres, app)) = app().await {
			review::a_build_attempt_has_a_deadline(&app).await;
		}
	}

	#[tokio::test]
	async fn an_expired_lease_holds_nothing() {
		if let Some((_postgres, app)) = app().await {
			review::an_expired_lease_holds_nothing(&app).await;
		}
	}

	#[tokio::test]
	async fn a_build_cannot_name_another_repository() {
		if let Some((_postgres, app)) = app().await {
			review::a_build_cannot_name_another_repository(&app).await;
		}
	}

	#[tokio::test]
	async fn rollback_waits_for_a_certain_schema() {
		if let Some((_postgres, app)) = app().await {
			review::rollback_waits_for_a_certain_schema(&app).await;
		}
	}

	#[tokio::test]
	async fn the_largest_status_report_reads_back() {
		if let Some((_postgres, app)) = app().await {
			review::the_largest_status_report_reads_back(&app).await;
		}
	}

	#[tokio::test]
	async fn a_failed_teardown_stays_actionable() {
		if let Some((_postgres, app)) = app().await {
			review::a_failed_teardown_stays_actionable(&app).await;
		}
	}

	#[tokio::test]
	async fn a_swept_archive_never_takes_its_replacement() {
		if let Some((_postgres, app)) = app().await {
			review::a_swept_archive_never_takes_its_replacement(&app).await;
		}
	}

	#[tokio::test]
	async fn logs_hide_split_secrets_and_control_characters() {
		if let Some((_postgres, app)) = app().await {
			review::logs_hide_split_secrets_and_control_characters(&app).await;
		}
	}

	#[tokio::test]
	async fn a_release_is_reported_after_its_lease() {
		if let Some((_postgres, app)) = app().await {
			review::a_release_is_reported_after_its_lease(&app).await;
		}
	}

	#[tokio::test]
	async fn a_removal_that_then_works_is_done() {
		if let Some((_postgres, app)) = app().await {
			review::a_removal_that_then_works_is_done(&app).await;
		}
	}

	#[tokio::test]
	async fn build_logs_are_capped() {
		let Some(server) = postgres::Postgres::start() else {
			return;
		};
		let db = leptoskit_db::Db::connect(&server.database())
			.await
			.expect("connect");
		db.migrate(leptoskit_cloud::POSTGRES_MIGRATIONS)
			.await
			.expect("the Postgres migrations apply");
		let app = support::app_with(db, |config| review::settings(&mut config.shipping)).await;
		review::build_logs_are_capped(&app).await;
	}
}

/// Variables and the secrets vault (`support/config.rs`), on Postgres.
mod config {
	use super::app;
	use super::support::config;

	#[tokio::test]
	async fn variables_and_secrets_reach_the_release() {
		if let Some((_postgres, app)) = app().await {
			config::variables_and_secrets_reach_the_release(&app).await;
		}
	}

	#[tokio::test]
	async fn names_and_values_are_checked() {
		if let Some((_postgres, app)) = app().await {
			config::names_and_values_are_checked(&app).await;
		}
	}

	#[tokio::test]
	async fn configuration_is_the_tenants_alone() {
		if let Some((_postgres, app)) = app().await {
			config::configuration_is_the_tenants_alone(&app).await;
		}
	}

	#[tokio::test]
	async fn a_release_always_finds_its_secrets() {
		if let Some((_postgres, app)) = app().await {
			config::a_release_always_finds_its_secrets(&app).await;
		}
	}

	#[tokio::test]
	async fn a_copied_secret_opens_nowhere_else() {
		if let Some((_postgres, app)) = app().await {
			config::a_copied_secret_opens_nowhere_else(&app).await;
		}
	}

	#[tokio::test]
	async fn first_secrets_race_to_one_key() {
		if let Some((_postgres, app)) = app().await {
			config::first_secrets_race_to_one_key(&app).await;
		}
	}

	#[tokio::test]
	async fn configuration_has_a_size() {
		if let Some((_postgres, app)) = app().await {
			config::configuration_has_a_size(&app).await;
		}
	}

	#[tokio::test]
	async fn a_removed_secret_never_comes_back() {
		if let Some((_postgres, app)) = app().await {
			config::a_removed_secret_never_comes_back(&app).await;
		}
	}
}

/// Custom domains (`support/domains.rs`), on Postgres.
mod domains {
	use super::app;
	use super::support::domains;

	#[tokio::test]
	async fn a_verified_domain_reaches_the_release() {
		if let Some((_postgres, app)) = app().await {
			domains::a_verified_domain_reaches_the_release(&app).await;
		}
	}

	#[tokio::test]
	async fn domains_are_checked_and_the_tenants_alone() {
		if let Some((_postgres, app)) = app().await {
			domains::domains_are_checked_and_the_tenants_alone(&app).await;
		}
	}

	#[tokio::test]
	async fn a_hostname_is_one_environments() {
		if let Some((_postgres, app)) = app().await {
			domains::a_hostname_is_one_environments(&app).await;
		}
	}

	#[tokio::test]
	async fn verifying_races_to_one_holder() {
		if let Some((_postgres, app)) = app().await {
			domains::verifying_races_to_one_holder(&app).await;
		}
	}

	#[tokio::test]
	async fn a_detached_domain_is_held_until_it_stops_serving() {
		if let Some((_postgres, app)) = app().await {
			domains::a_detached_domain_is_held_until_it_stops_serving(&app).await;
		}
	}

	#[tokio::test]
	async fn a_hostname_is_held_while_the_cluster_may_route_it() {
		if let Some((_postgres, app)) = app().await {
			domains::a_hostname_is_held_while_the_cluster_may_route_it(&app).await;
		}
	}
}

/// Managed Postgres databases (`support/databases.rs`), on Postgres.
mod databases {
	use super::app;
	use super::support::databases;

	#[tokio::test]
	async fn a_postgres_app_gets_a_database() {
		if let Some((_postgres, app)) = app().await {
			databases::a_postgres_app_gets_a_database(&app).await;
		}
	}

	#[tokio::test]
	async fn a_database_password_opens_nowhere_else() {
		if let Some((_postgres, app)) = app().await {
			databases::a_database_password_opens_nowhere_else(&app).await;
		}
	}

	#[tokio::test]
	async fn a_rollback_finds_its_database_where_it_is_now() {
		if let Some((_postgres, app)) = app().await {
			databases::a_rollback_finds_its_database_where_it_is_now(&app).await;
		}
	}

	#[tokio::test]
	async fn a_deleted_environment_takes_its_database() {
		if let Some((_postgres, app)) = app().await {
			databases::a_deleted_environment_takes_its_database(&app).await;
		}
	}

	#[tokio::test]
	async fn a_clusters_postgres_is_checked() {
		if let Some((_postgres, app)) = app().await {
			databases::a_clusters_postgres_is_checked(&app).await;
		}
	}

	#[tokio::test]
	async fn a_failed_release_does_not_print_the_password() {
		if let Some((_postgres, app)) = app().await {
			databases::a_failed_release_does_not_print_the_password(&app).await;
		}
	}
}

/// Backups and restores (`support/backups.rs`), on Postgres.
mod backups {
	use super::app;
	use super::support::backups;

	#[tokio::test]
	async fn a_backup_is_taken_and_listed() {
		if let Some((_postgres, app)) = app().await {
			backups::a_backup_is_taken_and_listed(&app).await;
		}
	}

	#[tokio::test]
	async fn a_backup_is_restored() {
		if let Some((_postgres, app)) = app().await {
			backups::a_backup_is_restored(&app).await;
		}
	}

	#[tokio::test]
	async fn a_quiet_worker_fails_a_backup_and_leaves_a_restore_unknown() {
		if let Some((_postgres, app)) = app().await {
			backups::a_quiet_worker_fails_a_backup_and_leaves_a_restore_unknown(&app).await;
		}
	}

	#[tokio::test]
	async fn a_deleted_environment_takes_its_backups() {
		if let Some((_postgres, app)) = app().await {
			backups::a_deleted_environment_takes_its_backups(&app).await;
		}
	}
}

/// Usage, the spend cap and plans' limits (`support/billing.rs`), on
/// Postgres, with the tests' price catalogue.
mod billing {
	use super::postgres;
	use super::support;
	use super::support::App;
	use super::support::billing;

	async fn app() -> Option<(postgres::Postgres, App)> {
		let server = postgres::Postgres::start()?;
		let db = leptoskit_db::Db::connect(&server.database())
			.await
			.expect("connect");
		db.migrate(leptoskit_cloud::POSTGRES_MIGRATIONS)
			.await
			.expect("the Postgres migrations apply");
		let app =
			support::app_with(db, |config| config.catalogue = Some(billing::catalogue())).await;
		Some((server, app))
	}

	#[tokio::test]
	async fn usage_is_counted_priced_and_capped() {
		if let Some((_postgres, app)) = app().await {
			billing::usage_is_counted_priced_and_capped(&app).await;
		}
	}

	#[tokio::test]
	async fn a_plans_limits_are_counted() {
		if let Some((_postgres, app)) = app().await {
			billing::a_plans_limits_are_counted(&app).await;
		}
	}

	#[tokio::test]
	async fn a_build_whose_lease_ran_out_is_counted() {
		if let Some((_postgres, app)) = app().await {
			billing::a_build_whose_lease_ran_out_is_counted(&app).await;
		}
	}
}
