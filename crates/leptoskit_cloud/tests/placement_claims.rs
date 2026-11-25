//! A platform hostname is held by its environment's placement row, and the
//! cluster may still route it once the environment, or its project, is
//! deleted. The row outlives them until the removal is reported done, by
//! the cluster's worker or by an operator. Until then another organisation
//! that spells the same label is placed at the fallback hostname. A row
//! that is being removed stands for no live environment: it doesn't load
//! its cluster, and its cluster's worker is refused a report about it.
//!
//! Each scenario runs on SQLite and, when Postgres binaries are installed,
//! on a private, throwaway Postgres, as `postgres.rs` runs.

#![cfg(not(target_arch = "wasm32"))]

mod support;

#[path = "support/postgres.rs"]
mod postgres;

use leptoskit_cloud::operator::JobState;
use leptoskit_cloud::tables::Cluster;
use leptoskit_cloud::tables::Job;
use leptoskit_cloud::tables::Placement;
use leptoskit_cloud::workers::JobKind;
use leptoskit_db::Db;
use serde_json::Value;
use serde_json::json;
use support::APPS_DOMAIN;
use support::App;
use support::As;
use support::shipping::Shop;
use support::shipping::applied;
use support::shipping::built;
use support::shipping::register_cluster;
use support::shipping::upload;

/// A control plane over a fresh, migrated database on its own Postgres,
/// which stops when the returned guard drops, or `None` when there is no
/// Postgres to run on.
async fn on_postgres() -> Option<(postgres::Postgres, App)> {
	let server = postgres::Postgres::start()?;
	let url = server.database();
	let db = Db::connect(&url).await.expect("connect");
	db.migrate(leptoskit_cloud::POSTGRES_MIGRATIONS)
		.await
		.expect("the Postgres migrations apply");
	Some((server, support::app_on(db).await))
}

/// Uploads `bytes` to `environment` and deploys them: the deployment's
/// number.
async fn deploy(
	app: &App,
	caller: As<'_>,
	organization: &str,
	project: &str,
	environment: &str,
	bytes: &[u8],
) -> u64 {
	let digest = upload(app, caller, organization, project, bytes).await;

	app.call(
		"deploy",
		json!({
			"organization": organization,
			"project": project,
			"environment": environment,
			"source": { "kind": "upload", "archive": digest },
			"config": { "app": { "database": "sqlite" } },
			"host": "some-laptop",
		}),
		caller,
	)
	.await
	.expect(200)
	.json()["number"]
		.as_u64()
		.expect("a number")
}

/// The hostname deployment `number` of `environment` was placed at, as its
/// sheet names it.
async fn deployed_hostname(
	app: &App,
	caller: As<'_>,
	organization: &str,
	project: &str,
	environment: &str,
	number: u64,
) -> String {
	app.call(
		"deployment",
		json!({
			"organization": organization,
			"project": project,
			"environment": environment,
			"number": number,
		}),
		caller,
	)
	.await
	.expect(200)
	.json()["hostname"]
		.as_str()
		.expect("a hostname")
		.to_owned()
}

/// How many placement rows hold `hostname`.
async fn held(db: &Db, hostname: &str) -> u64 {
	Placement::query()
		.filter(|placement| placement.hostname.eq(hostname))
		.count(db)
		.await
		.expect("placements")
}

/// The name of the cluster `hostname` is placed on.
async fn cluster_of(db: &Db, hostname: &str) -> String {
	let placement = Placement::query()
		.filter(|placement| placement.hostname.eq(hostname))
		.first(db)
		.await
		.expect("placements")
		.expect("a placement");

	Cluster::query()
		.filter(|cluster| cluster.id.eq(placement.cluster_id))
		.one(db)
		.await
		.expect("clusters")
		.name
}

/// Whether an environment removal is waiting for a cluster's worker.
async fn removal_is_queued(db: &Db) -> bool {
	Job::query()
		.filter(|job| job.kind.eq(JobKind::DestroyEnvironment) & job.state.eq(JobState::Queued))
		.exists(db)
		.await
		.expect("jobs")
}

/// The hostnames the release `assignment` carries route.
fn routed(assignment: &Value) -> Vec<&str> {
	assignment["input"]["spec"]["routes"]
		.as_array()
		.expect("the release's routes")
		.iter()
		.map(|route| route["hostname"].as_str().expect("a hostname"))
		.collect()
}

/// Ana makes `staging` in `notes` and ships it live: the hostname it was
/// placed at.
async fn ana_ships_staging(shop: &Shop<'_>) -> String {
	shop.app
		.call(
			"create_environment",
			json!({
				"organization": shop.org,
				"project": "notes",
				"name": "staging",
				"kind": "staging",
			}),
			shop.as_ana(),
		)
		.await
		.expect(200);
	let number = shop.ship("staging", b"victim", &[]).await;
	shop.sheet("staging", number).await["hostname"]
		.as_str()
		.expect("a hostname")
		.to_owned()
}

/// Ana deletes `staging` in `notes`, which queues its removal.
async fn ana_deletes_staging(shop: &Shop<'_>) {
	shop.app
		.call(
			"delete_environment",
			json!({
				"organization": shop.org,
				"project": "notes",
				"environment": "staging",
				"confirm": "staging",
			}),
			shop.as_ana(),
		)
		.await
		.expect(200);
}

/// Ana makes the project `notes-app` and ships its production live: the
/// hostname it was placed at. A production is deleted with its project.
async fn ana_ships_notes_app(shop: &Shop<'_>) -> String {
	let app = shop.app;
	app.project(shop.as_ana(), &shop.org, "notes-app").await;
	let number = deploy(
		app,
		shop.as_ana(),
		&shop.org,
		"notes-app",
		"production",
		b"victim",
	)
	.await;
	let build = shop.build.take().await;
	shop.build
		.finish(&build, built(&build, b"victim", &[]))
		.await
		.expect(200);
	let release = shop.cluster.take().await;
	shop.cluster.finish(&release, applied()).await.expect(200);
	deployed_hostname(
		app,
		shop.as_ana(),
		&shop.org,
		"notes-app",
		"production",
		number,
	)
	.await
}

/// Another person, Mallory, makes the organisation `organization` with one
/// project `project` and deploys its production: the hostname it was placed
/// at.
async fn rival_deploys(app: &App, organization: &str, project: &str) -> String {
	let mallory = app.sign_in("mallory@example.com").await;
	let mallory = As::Session(&mallory);

	app.call(
		"create_organization",
		json!({ "slug": organization, "name": "Rival" }),
		mallory,
	)
	.await
	.expect(200);
	app.project(mallory, organization, project).await;
	let number = deploy(app, mallory, organization, project, "production", b"rival").await;
	deployed_hostname(app, mallory, organization, project, "production", number).await
}

/// While Ana's removal is still queued, an organisation whose slugs spell
/// `ana`'s label deploys: Ana's row is the one holding the hostname, so the
/// newcomer is placed at the fallback. Its release is built at once, to
/// wait behind the removal.
async fn a_rival_is_placed_at_the_fallback(
	shop: &Shop<'_>,
	ana: &str,
	organization: &str,
	project: &str,
) {
	let app = shop.app;
	assert_eq!(
		held(&app.system(), ana).await,
		1,
		"the hostname was released while its removal was still queued"
	);
	assert!(removal_is_queued(&app.system()).await);
	let clusters = app.call("clusters", json!({}), shop.as_ops()).await;
	assert_eq!(
		clusters.expect(200).json()[0]["environments"],
		0,
		"a placement being removed isn't an environment the cluster runs"
	);

	let theirs = rival_deploys(app, organization, project).await;
	assert_ne!(
		theirs, ana,
		"another organisation was placed at the hostname the cluster may still route"
	);
	// The fallback is the label it wanted, then twelve hex digits.
	let label = theirs
		.strip_suffix(&format!(".{APPS_DOMAIN}"))
		.expect("under the apps domain");
	let suffix = label
		.strip_prefix(&format!("{project}-{organization}-"))
		.expect("the fallback for the label it wanted");
	assert_eq!(suffix.len(), 12, "{theirs}");

	let build = shop.build.take().await;
	shop.build
		.finish(&build, built(&build, b"rival", &[]))
		.await
		.expect(200);
}

/// The cluster's worker takes Ana's removal and reports the environment
/// destroyed: the row, and its hold on `ana`, go.
async fn removal_reported_destroyed(shop: &Shop<'_>, ana: &str) {
	let destroy = shop.cluster.take().await;
	assert_eq!(destroy["input"]["kind"], "destroy_environment", "{destroy}");
	shop.cluster
		.finish(
			&destroy,
			json!({ "kind": "destroy_environment", "result": "destroyed" }),
		)
		.await
		.expect(200);
	assert_eq!(held(&shop.app.system(), ana).await, 0);
}

/// The cluster's worker is handed the rival's release, placed after Ana's
/// deletion: it routes nothing of Ana's. The worker applies it.
async fn rivals_release_is_applied(shop: &Shop<'_>, ana: &str) {
	let release = shop.cluster.take().await;
	assert_eq!(release["input"]["kind"], "release", "{release}");
	assert!(
		!routed(&release).contains(&ana),
		"another organisation's release routes the hostname: {release}"
	);
	shop.cluster.finish(&release, applied()).await.expect(200);
}

/// Deleting an environment frees its platform hostname only once the
/// cluster's worker reports the removal done.
async fn a_deleted_environment(app: &App) {
	let shop = Shop::open(app).await;
	let ana = ana_ships_staging(&shop).await;
	assert_eq!(ana, format!("staging-notes-{}.{APPS_DOMAIN}", shop.org));
	ana_deletes_staging(&shop).await;

	a_rival_is_placed_at_the_fallback(&shop, &ana, "notes-ana", "staging").await;
	removal_reported_destroyed(&shop, &ana).await;
	rivals_release_is_applied(&shop, &ana).await;

	// Off the cluster, the label is Ana's to place again.
	assert_eq!(ana_ships_staging(&shop).await, ana);
}

/// Deleting a project does the same for its production's hostname.
async fn a_deleted_project(app: &App) {
	let shop = Shop::open(app).await;
	let ana = ana_ships_notes_app(&shop).await;
	assert_eq!(ana, format!("notes-app-{}.{APPS_DOMAIN}", shop.org));
	shop.app
		.call(
			"delete_project",
			json!({
				"organization": shop.org,
				"project": "notes-app",
				"confirm": "notes-app",
			}),
			shop.as_ana(),
		)
		.await
		.expect(200);

	a_rival_is_placed_at_the_fallback(&shop, &ana, "app-ana", "notes").await;
	removal_reported_destroyed(&shop, &ana).await;
	rivals_release_is_applied(&shop, &ana).await;

	assert_eq!(ana_ships_notes_app(&shop).await, ana);
}

/// A removal that failed may have left the environment on the cluster, so
/// the hostname stays held until an operator finds the cluster without it.
async fn a_failed_removal(app: &App) {
	let shop = Shop::open(app).await;
	let ana = ana_ships_staging(&shop).await;
	ana_deletes_staging(&shop).await;
	a_rival_is_placed_at_the_fallback(&shop, &ana, "notes-ana", "staging").await;

	let destroy = shop.cluster.take().await;
	shop.cluster
		.finish(
			&destroy,
			json!({
				"kind": "destroy_environment",
				"result": "failed",
				"reason": "a volume wouldn't detach",
			}),
		)
		.await
		.expect(200);
	assert_eq!(held(&app.system(), &ana).await, 1);

	let settled = shop
		.app
		.call(
			"reconcile",
			json!({ "job": destroy["job"], "found": { "found": "applied" } }),
			shop.as_ops(),
		)
		.await;
	assert_eq!(settled.expect(200).json()["state"], "done");
	assert_eq!(held(&app.system(), &ana).await, 0);

	rivals_release_is_applied(&shop, &ana).await;
	assert_eq!(ana_ships_staging(&shop).await, ana);
}

/// A placement being removed stands for no environment its cluster's worker
/// may report on: the report is refused as for one the worker doesn't run.
async fn a_removed_environment_is_not_reported(app: &App) {
	let shop = Shop::open(app).await;
	ana_ships_staging(&shop).await;
	ana_deletes_staging(&shop).await;
	let destroy = shop.cluster.take().await;
	let id = destroy["input"]["environment"]["environment"]["id"]
		.as_str()
		.expect("the environment's id");

	let reported = shop
		.cluster
		.call(
			"report_status",
			json!({
				"environment": id,
				"observed": {
					"environment": {
						"organization": { "slug": shop.org, "id": uuid::Uuid::now_v7() },
						"project": { "slug": "notes", "id": uuid::Uuid::now_v7() },
						"environment": { "slug": "staging", "id": id },
					},
					"processes": [],
				},
			}),
		)
		.await;
	reported.refused(404, "environment_not_found");
}

/// A placement that is being removed doesn't load its cluster: a new
/// environment goes to the cluster with the fewest live ones.
async fn a_removed_placement_loads_no_cluster(app: &App) {
	let shop = Shop::open(app).await;
	register_cluster(app, &shop.ops, "eu-two").await;
	let preview = format!("preview-notes-{}.{APPS_DOMAIN}", shop.org);
	let staging = format!("staging-notes-{}.{APPS_DOMAIN}", shop.org);

	// Both clusters are empty, so `preview` goes to the older one, and
	// `staging`, placed as soon as it is deployed, to the other.
	for name in ["preview", "staging"] {
		shop.app
			.call(
				"create_environment",
				json!({
					"organization": shop.org,
					"project": "notes",
					"name": name,
					"kind": name,
				}),
				shop.as_ana(),
			)
			.await
			.expect(200);
	}
	shop.ship("preview", b"staying", &[]).await;
	let digest = shop.upload(b"leaving").await;
	shop.deploy("staging", &digest, json!({ "database": "sqlite" }))
		.await
		.expect(200);
	assert_eq!(cluster_of(&app.system(), &preview).await, "eu-one");
	assert_eq!(cluster_of(&app.system(), &staging).await, "eu-two");
	ana_deletes_staging(&shop).await;

	// `eu-one` runs one environment and `eu-two` none: its only placement
	// is being removed, by a worker that was never paired. So a newcomer
	// goes to `eu-two`, the less loaded.
	let theirs = rival_deploys(app, "rival", "web").await;
	assert_eq!(cluster_of(&app.system(), &theirs).await, "eu-two");
}

#[tokio::test]
async fn a_deleted_environments_hostname_stays_held_until_its_removal_is_reported() {
	a_deleted_environment(&support::app().await).await;
}

#[tokio::test]
async fn a_deleted_projects_hostname_stays_held_until_its_removal_is_reported() {
	a_deleted_project(&support::app().await).await;
}

#[tokio::test]
async fn a_failed_removal_keeps_the_hostname_until_an_operator_settles_it() {
	a_failed_removal(&support::app().await).await;
}

#[tokio::test]
async fn a_placement_being_removed_does_not_load_its_cluster() {
	a_removed_placement_loads_no_cluster(&support::app().await).await;
}

#[tokio::test]
async fn a_removed_environment_is_not_its_clusters_to_report() {
	a_removed_environment_is_not_reported(&support::app().await).await;
}

#[tokio::test]
async fn a_deleted_environments_hostname_stays_held_until_its_removal_is_reported_on_postgres() {
	if let Some((_postgres, app)) = on_postgres().await {
		a_deleted_environment(&app).await;
	}
}

#[tokio::test]
async fn a_deleted_projects_hostname_stays_held_until_its_removal_is_reported_on_postgres() {
	if let Some((_postgres, app)) = on_postgres().await {
		a_deleted_project(&app).await;
	}
}

#[tokio::test]
async fn a_failed_removal_keeps_the_hostname_until_an_operator_settles_it_on_postgres() {
	if let Some((_postgres, app)) = on_postgres().await {
		a_failed_removal(&app).await;
	}
}

#[tokio::test]
async fn a_placement_being_removed_does_not_load_its_cluster_on_postgres() {
	if let Some((_postgres, app)) = on_postgres().await {
		a_removed_placement_loads_no_cluster(&app).await;
	}
}

#[tokio::test]
async fn a_removed_environment_is_not_its_clusters_to_report_on_postgres() {
	if let Some((_postgres, app)) = on_postgres().await {
		a_removed_environment_is_not_reported(&app).await;
	}
}
