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
