//! Hosted notifications, written once and run on SQLite (`deployments.rs`)
//! and Postgres (`postgres.rs`), as `databases.rs`'s scenarios are.

use leptoskit_cloud::tables::NotificationLease;
use leptoskit_cloud::tables::NotificationTenant;
use leptoskit_cloud::tables::SecretLease;
use leptoskit_cloud::tables::UsageTotal;
use serde_json::Value;
use serde_json::json;

use super::App;
use super::As;
use super::databases::place;
use super::databases::staging;
use super::shipping::Shop;
use super::shipping::Worker;
use super::shipping::applied;
use super::shipping::built;
use super::shipping::pair;
use super::shipping::register_cluster;

const HUB_HOST: &str = "hub.leptoskit-notifications.svc.cluster.local";

/// The app's `[services.notifications]`, as the CLI reports it.
fn with_notifications() -> Value {
	json!({
		"database": "sqlite",
		"services": {
			"notifications": {
				"public": true,
				"database": true,
				"forward_identity": true,
				"hosted": "notifications",
			},
		},
	})
}

/// Tells the control plane `cluster` has a Postgres server and a
/// notifications hub.
async fn with_hub(shop: &Shop<'_>, cluster: &str) {
	shop.app
		.call(
			"set_cluster_facts",
			json!({
				"name": cluster,
				"facts": {
					"runtime_class": "gvisor",
					"wakes_sleeping": true,
					"postgres": {
						"namespace": "leptoskit-postgres",
						"host": "postgres-rw.leptoskit-postgres.svc",
						"port": 5432,
						"tls": true,
					},
					"notifications": {
						"namespace": "leptoskit-notifications",
						"host": HUB_HOST,
						"port": 8080,
					},
				},
			}),
			shop.as_ops(),
		)
		.await
		.expect(200);
}

/// Pairs `cluster`'s notifications hub.
async fn hub<'a>(shop: &Shop<'a>, cluster: &str) -> Worker<'a> {
	pair(
		shop.app,
		&shop.ops,
		json!({ "role": "notifications", "cluster": cluster }),
		&format!("{cluster}-notifications"),
	)
	.await
}

/// Deploys `bytes` with `app` to `environment`, builds it, and applies its
/// release. Returns the release the cluster's worker was given.
async fn ship(shop: &Shop<'_>, environment: &str, bytes: &[u8], app: Value) -> Value {
	let digest = shop.upload(bytes).await;
	shop.deploy(environment, &digest, app).await.expect(200);
	let job = shop.build.take().await;
	shop.build
		.finish(&job, built(&job, bytes, &[]))
		.await
		.expect(200);
	let release = shop.cluster.take().await;
	shop.cluster.finish(&release, applied()).await.expect(200);
	release
}

async fn summary(shop: &Shop<'_>, environment: &str) -> Value {
	shop.app
		.call("notifications", place(shop, environment), shop.as_ana())
		.await
		.expect(200)
		.json()
}

async fn set(shop: &Shop<'_>, operation: &str, name: &str, value: &str) {
	let mut args = place(shop, "production");
	args["name"] = json!(name);
	args["value"] = json!(value);
	shop.app
		.call(operation, args, shop.as_ana())
		.await
		.expect(200);
}

/// What the hub is told, holding `known`.
async fn ask(hub: &Worker<'_>, known: Value) -> Value {
	hub.call("notification_tenants", json!({ "known": known }))
		.await
		.expect(200)
		.json()
}

/// An app that declares `[services.notifications]` becomes a tenant of its
/// cluster's hub: no process of its own, a database on the cluster's
/// Postgres, a token the app and the hub share, and the push settings
/// given to the hub alone. The hub's word and its pushes are kept.
#[expect(
	clippy::cognitive_complexity,
	clippy::too_many_lines,
	reason = "a scenario is one story, read top to bottom; its asserts are what count"
)]
pub async fn an_app_with_notifications_is_served_by_the_hub(app: &App) {
	let shop = Shop::open(app).await;
	let digest = shop.upload(b"first").await;

	// The cluster has no hub yet.
	shop.deploy("production", &digest, with_notifications())
		.await
		.refused(400, "notifications_unavailable");
	assert_eq!(summary(&shop, "production").await, Value::Null);
	with_hub(&shop, "eu-one").await;

	// The push settings are the environment's, under the service's own
	// names, which the reserved prefix lets through and nothing else.
	set(
		&shop,
		"set_secret",
		"LEPTOSKIT_NOTIFICATIONS_APNS_KEY_P8",
		"-----BEGIN PRIVATE KEY-----apns",
	)
	.await;
	set(
		&shop,
		"set_variable",
		"LEPTOSKIT_NOTIFICATIONS_APNS_KEY_ID",
		"KEY123",
	)
	.await;
	let mut reserved = place(&shop, "production");
	reserved["name"] = json!("LEPTOSKIT_NOTIFICATIONS_DATABASE_URL");
	reserved["value"] = json!("postgres://elsewhere");
	app.call("set_secret", reserved.clone(), shop.as_ana())
		.await
		.refused(400, "invalid_name");
	// Which hosts it may send to is the operator's to widen, not a tenant's.
	reserved["name"] = json!("LEPTOSKIT_NOTIFICATIONS_WEB_PUSH_EXTRA_HOSTS");
	app.call("set_secret", reserved.clone(), shop.as_ana())
		.await
		.refused(400, "invalid_name");
	// A private key is a secret, never a variable anyone viewing can read.
	reserved["name"] = json!("LEPTOSKIT_NOTIFICATIONS_APNS_KEY_P8");
	app.call("set_variable", reserved, shop.as_ana())
		.await
		.refused(400, "invalid_name");

	let release = ship(&shop, "production", b"first", with_notifications()).await;
	let made = summary(&shop, "production").await;
	let tenant = made["tenant"].as_str().expect("a tenant").to_owned();
	let database = made["database"].as_str().expect("a database").to_owned();
	assert!(
		database.starts_with("ntf_") && database.len() == 36,
		"{database}"
	);
	assert_eq!(made["providers"], json!(["apns"]));
	assert_eq!(
		made["settings"],
		json!([
			"LEPTOSKIT_NOTIFICATIONS_APNS_KEY_ID",
			"LEPTOSKIT_NOTIFICATIONS_APNS_KEY_P8"
		])
	);
	// Applied, and the hub hasn't said it serves it yet.
	assert_eq!(made["state"], json!({ "state": "waiting" }));

	// The release makes the database, runs no process for the service,
	// bridges to the hub, and gives the app its address and token.
	let input = &release["input"];
	assert_eq!(
		input["notifications"],
		json!({ "name": database, "role": database, "size": "shared", "backups": "daily" })
	);
	let spec = &input["spec"];
	assert_eq!(spec["processes"].as_array().map(Vec::len), Some(1));
	assert_eq!(
		spec["platform"],
		json!([{
			"name": "notifications",
			"namespace": "leptoskit-notifications",
			"host": HUB_HOST,
			"port": 8080,
		}])
	);
	assert!(
		spec["routes"]
			.as_array()
			.expect("routes")
			.iter()
			.any(|route| {
				route["process"] == "notifications"
					&& route["path_prefix"] == "/_leptoskit/services/notifications/"
			}),
		"{spec}"
	);
	let process = &spec["processes"][0];
	let url = format!("http://{HUB_HOST}:8080/t/{}", tenant.replace('-', ""));
	assert_eq!(
		process["env"]["LEPTOSKIT_SERVICE_NOTIFICATIONS_URL"],
		url.as_str()
	);
	assert_eq!(
		process["env"]["LEPTOSKIT_SERVICE_NOTIFICATIONS_IDENTITY"],
		"forward"
	);
	// The settings are the hub's, never the app's.
	assert_eq!(
		process["secret_env"],
		json!({ "LEPTOSKIT_SERVICE_NOTIFICATIONS_TOKEN": "LEPTOSKIT_SERVICE_NOTIFICATIONS_TOKEN" })
	);
	assert!(
		process["env"]
			.get("LEPTOSKIT_NOTIFICATIONS_APNS_KEY_ID")
			.is_none(),
		"{process}"
	);
	let secrets = release["secrets"].as_object().expect("secrets");
	let mut names: Vec<&String> = secrets.keys().collect();
	names.sort_unstable();
	assert_eq!(
		names,
		[
			"LEPTOSKIT_NOTIFICATIONS_DATABASE_URL",
			"LEPTOSKIT_SERVICE_NOTIFICATIONS_TOKEN"
		]
	);
	let token = secrets["LEPTOSKIT_SERVICE_NOTIFICATIONS_TOKEN"]
		.as_str()
		.expect("a token")
		.to_owned();
	assert_eq!(token.len(), 64, "{token}");
	let address = secrets["LEPTOSKIT_NOTIFICATIONS_DATABASE_URL"]
		.as_str()
		.expect("an address")
		.to_owned();
	assert!(
		address.starts_with(&format!("postgres://{database}:")),
		"{address}"
	);
	let system = app.system();
	let leases = SecretLease::query().all(&system).await.expect("leases");
	assert_eq!(leases.len(), 2, "{leases:?}");

	// Only a notifications worker asks for tenants.
	shop.cluster
		.call("notification_tenants", json!({}))
		.await
		.refused(403, "notifications_worker_required");
	app.call("notification_tenants", json!({}), shop.as_ana())
		.await
		.refused(403, "worker_required");

	// The hub takes no jobs; it asks, and is given the tenant whole.
	let hub = hub(&shop, "eu-one").await;
	hub.call("claim", json!({ "wait_seconds": 0 }))
		.await
		.refused(403, "notifications_worker_takes_no_jobs");
	let answer = ask(&hub, json!([])).await;
	assert_eq!(answer["removed"], json!([]));
	assert_eq!(answer["more"], false);
	let given = &answer["changed"][0];
	assert_eq!(given["tenant"], tenant.as_str());
	assert_eq!(given["environment"]["environment"]["slug"], "production");
	let hostnames = given["hostnames"].as_array().expect("hostnames");
	assert_eq!(hostnames.len(), 1, "{given}");
	assert_eq!(
		given["origin"],
		format!("https://{}", hostnames[0].as_str().expect("a hostname"))
	);
	assert_eq!(given["usage_cursor"], Value::Null);
	assert_eq!(given["secrets"]["token"], token.as_str());
	assert_eq!(given["secrets"]["database_url"], address.as_str());
	assert_eq!(
		given["secrets"]["settings"],
		json!({
			"LEPTOSKIT_NOTIFICATIONS_APNS_KEY_ID": "KEY123",
			"LEPTOSKIT_NOTIFICATIONS_APNS_KEY_P8": "-----BEGIN PRIVATE KEY-----apns",
		})
	);
	let version = given["version"].as_i64().expect("a version");
	let given_leases = NotificationLease::query()
		.all(&system)
		.await
		.expect("leases");
	assert_eq!(given_leases.len(), 1, "{given_leases:?}");
	assert_eq!(given_leases[0].version, version);
	assert_eq!(given_leases[0].names.0.len(), 4);

	// Holding it, the hub is told nothing new.
	let known = json!([{ "tenant": tenant, "version": version }]);
	let again = ask(&hub, known.clone()).await;
	assert_eq!(again["changed"], json!([]));
	assert_eq!(again["removed"], json!([]));

	// It says it serves it, and counts its pushes once.
	let reported = hub
		.call(
			"report_notifications",
			json!({
				"tenants": [{ "tenant": tenant, "version": version, "state": "serving" }],
				"usage": [{ "tenant": tenant, "after": null, "until": "c1", "pushes": 5 }],
			}),
		)
		.await
		.expect(200)
		.json();
	assert_eq!(reported["tenants"], 1);
	assert_eq!(reported["cursors"][tenant.as_str()], "c1");
	let replayed = hub
		.call(
			"report_notifications",
			json!({ "usage": [{ "tenant": tenant, "after": null, "until": "c1", "pushes": 5 }] }),
		)
		.await
		.expect(200)
		.json();
	assert_eq!(replayed["cursors"][tenant.as_str()], "c1");
	let served = summary(&shop, "production").await;
	assert_eq!(served["state"], json!({ "state": "serving" }));
	assert_eq!(served["project_pushes_this_month"], 5);
	let pushes = UsageTotal::query()
		.filter(|row| row.meter.eq("notification_pushes"))
		.all(&system)
		.await
		.expect("usage");
	assert_eq!(pushes.len(), 1, "{pushes:?}");
	assert_eq!(pushes[0].quantity, 5);

	// A setting changed is a new version, given again without a deploy.
	set(
		&shop,
		"set_variable",
		"LEPTOSKIT_NOTIFICATIONS_APNS_KEY_ID",
		"KEY456",
	)
	.await;
	assert_eq!(
		summary(&shop, "production").await["state"],
		json!({ "state": "waiting" })
	);
	let reloaded = ask(&hub, known).await;
	let given = &reloaded["changed"][0];
	assert!(given["version"].as_i64() > Some(version), "{given}");
	assert_eq!(
		given["secrets"]["settings"]["LEPTOSKIT_NOTIFICATIONS_APNS_KEY_ID"],
		"KEY456"
	);
	assert_eq!(given["usage_cursor"], "c1");

	// A report about an older version is stale.
	let stale = hub
		.call(
			"report_notifications",
			json!({
				"tenants": [{ "tenant": tenant, "version": version, "state": "failed", "reason": "old" }],
			}),
		)
		.await
		.expect(200)
		.json();
	assert_eq!(stale["tenants"], 0);

	// The service's own words can quote a value: what people read hides
	// every one of the tenant's.
	let current = given["version"].clone();
	hub.call(
		"report_notifications",
		json!({
			"tenants": [{
				"tenant": tenant,
				"version": current,
				"state": "failed",
				"reason": format!(
					"apns: bad key -----BEGIN PRIVATE KEY-----apns\u{1b}[31m; token {token}; {address}"
				),
			}],
		}),
	)
	.await
	.expect(200);
	let failed = summary(&shop, "production").await["state"].clone();
	assert_eq!(failed["state"], "failed", "{failed}");
	let reason = failed["reason"].as_str().expect("a reason");
	assert!(reason.starts_with("apns: bad key"), "{reason}");
	assert!(!reason.contains("BEGIN PRIVATE KEY"), "{reason}");
	assert!(!reason.contains(&token), "{reason}");
	let password = address
		.split_once(':')
		.and_then(|(_, rest)| rest.split_once(':'))
		.and_then(|(_, rest)| rest.split_once('@'))
		.map(|(password, _)| password.to_owned())
		.expect("a password");
	assert!(!reason.contains(&password), "{reason}");
	assert!(!reason.contains('\u{1b}'), "{reason}");

	// No answer but the hub's and the release's holds a value, and no table
	// holds one unsealed.
	for answer in [
		app.call("notifications", place(&shop, "production"), shop.as_ana())
			.await,
		app.call("secrets", place(&shop, "production"), shop.as_ana())
			.await,
		app.call("jobs", json!({ "state": "done" }), shop.as_ops())
			.await,
	] {
		let text = &answer.expect(200).text;
		assert!(!text.contains(&token), "{text}");
		assert!(!text.contains("BEGIN PRIVATE KEY"), "{text}");
	}

	assert_eq!(app.find_in_every_table(&token).await, Vec::<String>::new());
	assert_eq!(
		app.find_in_every_table("BEGIN PRIVATE KEY").await,
		Vec::<String>::new()
	);

	// Someone else's organisation sees none of it.
	let mallory = app.sign_in("mallory@example.com").await;
	app.call(
		"notifications",
		place(&shop, "production"),
		As::Session(&mallory),
	)
	.await
	.expect(404);
}

/// A release without the service stops the hub serving the tenant, and
/// keeps its database; a release with it again serves the same tenant. A
/// deleted environment's tenant goes, and its database with the removal.
#[expect(
	clippy::cognitive_complexity,
	reason = "a scenario is one story, read top to bottom; its asserts are what count"
)]
pub async fn a_tenant_lives_with_its_releases_and_its_environment(app: &App) {
	let shop = Shop::open(app).await;
	with_hub(&shop, "eu-one").await;
	staging(&shop).await;
	let hub = hub(&shop, "eu-one").await;

	let first = ship(&shop, "staging", b"with", with_notifications()).await;
	let database = first["input"]["notifications"]["name"].clone();
	let given = ask(&hub, json!([])).await;
	let tenant = given["changed"][0]["tenant"].clone();
	let known = json!([{ "tenant": tenant, "version": given["changed"][0]["version"] }]);

	// Without the service, the hub stops; the inbox is kept.
	let without = ship(
		&shop,
		"staging",
		b"without",
		json!({ "database": "sqlite" }),
	)
	.await;
	assert_eq!(without["input"].get("notifications"), None);
	assert_eq!(without["input"]["spec"].get("platform"), None);
	assert_eq!(
		summary(&shop, "staging").await["state"],
		json!({ "state": "unused" })
	);
	let stopped = ask(&hub, known).await;
	assert_eq!(stopped["changed"], json!([]));
	assert_eq!(stopped["removed"], json!([tenant]));

	// With it again, the same tenant and database.
	let again = ship(&shop, "staging", b"again", with_notifications()).await;
	assert_eq!(again["input"]["notifications"]["name"], database);
	let back = ask(&hub, json!([])).await;
	assert_eq!(back["changed"][0]["tenant"], tenant);

	// The cluster can't forget its hub while it has tenants.
	app.call(
		"set_cluster_facts",
		json!({ "name": "eu-one", "facts": { "wakes_sleeping": true } }),
		shop.as_ops(),
	)
	.await
	.refused(409, "cluster_has_notifications");

	let mut delete = place(&shop, "staging");
	delete["confirm"] = json!("staging");
	app.call("delete_environment", delete, shop.as_ana())
		.await
		.expect(200);
	let gone = ask(&hub, json!([{ "tenant": tenant, "version": 1 }])).await;
	assert_eq!(gone["removed"], json!([tenant]));
	let destroy = shop.cluster.take().await;
	assert_eq!(destroy["input"]["notifications"], database);
	shop.cluster
		.finish(
			&destroy,
			json!({ "kind": "destroy_environment", "result": "destroyed" }),
		)
		.await
		.expect(200);
	let rows = NotificationTenant::query()
		.all(&app.system())
		.await
		.expect("tenants");
	assert_eq!(rows.len(), 0, "{}", rows.len());
	app.call(
		"set_cluster_facts",
		json!({ "name": "eu-one", "facts": { "wakes_sleeping": true } }),
		shop.as_ops(),
	)
	.await
	.expect(200);
}

/// A hub serves its own cluster's tenants and no other's: another
/// cluster's hub is given none of them, and its reports about them are
/// kept by nobody.
pub async fn a_hub_serves_only_its_own_cluster(app: &App) {
	let shop = Shop::open(app).await;
	with_hub(&shop, "eu-one").await;
	ship(&shop, "production", b"first", with_notifications()).await;
	let tenant = summary(&shop, "production").await["tenant"].clone();

	register_cluster(app, &shop.ops, "eu-two").await;
	let other = hub(&shop, "eu-two").await;
	let answer = ask(&other, json!([{ "tenant": tenant, "version": 1 }])).await;
	assert_eq!(answer["changed"], json!([]));
	// It is told to drop what it says it holds and isn't its own.
	assert_eq!(answer["removed"], json!([tenant]));
	let reported = other
		.call(
			"report_notifications",
			json!({
				"tenants": [{ "tenant": tenant, "version": 1, "state": "serving" }],
				"usage": [{ "tenant": tenant, "after": null, "until": "x", "pushes": 1000 }],
			}),
		)
		.await
		.expect(200)
		.json();
	assert_eq!(reported, json!({ "tenants": 0, "cursors": {} }));
	assert_eq!(
		summary(&shop, "production").await["project_pushes_this_month"],
		0
	);
	let leases = NotificationLease::query()
		.all(&app.system())
		.await
		.expect("leases");
	assert_eq!(leases.len(), 0, "{leases:?}");

	// Reports are bounded.
	let hub = hub(&shop, "eu-one").await;
	hub.call(
		"report_notifications",
		json!({ "usage": [{ "tenant": tenant, "after": "a", "until": "a", "pushes": 1 }] }),
	)
	.await
	.refused(400, "invalid_cursor");
	hub.call(
		"report_notifications",
		json!({ "usage": [{ "tenant": tenant, "after": null, "until": "b", "pushes": 100_000_001 }] }),
	)
	.await
	.refused(400, "too_many_pushes");
}

/// The token and database address a release was given.
fn values_of(release: &Value) -> (String, String) {
	let secrets = &release["secrets"];
	(
		secrets["LEPTOSKIT_SERVICE_NOTIFICATIONS_TOKEN"]
			.as_str()
			.expect("a token")
			.to_owned(),
		secrets["LEPTOSKIT_NOTIFICATIONS_DATABASE_URL"]
			.as_str()
			.expect("an address")
			.to_owned(),
	)
}

/// Revoking a worker that was given a cluster's tenants' values gives each
/// new ones to move to. The hub takes the old and the new until a release
/// that leased the new is applied; a release that fails tells it to open
/// the database again, as it may have the new password already.
#[expect(
	clippy::cognitive_complexity,
	reason = "a scenario is one story, read top to bottom; its asserts are what count"
)]
pub async fn revoking_a_worker_gives_its_tenants_new_values(app: &App) {
	let shop = Shop::open(app).await;
	with_hub(&shop, "eu-one").await;
	let first = ship(&shop, "production", b"first", with_notifications()).await;
	let (token, address) = values_of(&first);
	let revoked = hub(&shop, "eu-one").await;
	let given = ask(&revoked, json!([])).await;
	let tenant = given["changed"][0]["tenant"].clone();
	assert_eq!(given["changed"][0]["secrets"].get("next"), None);
	assert_eq!(summary(&shop, "production").await["rotating"], false);

	// Revoking a build worker gives nobody new values.
	let builds = pair(
		app,
		&shop.ops,
		json!({ "role": "build" }),
		"another-build-worker",
	)
	.await;
	app.call(
		"revoke_worker",
		json!({ "worker": builds.id }),
		shop.as_ops(),
	)
	.await
	.expect(200);
	assert_eq!(summary(&shop, "production").await["rotating"], false);

	// Revoking the hub does: the new hub is given both.
	app.call(
		"revoke_worker",
		json!({ "worker": revoked.id }),
		shop.as_ops(),
	)
	.await
	.expect(200);
	assert_eq!(summary(&shop, "production").await["rotating"], true);
	let told = leptoskit_cloud::tables::AuditEvent::query()
		.filter(|event| event.action.eq("notifications.rotate"))
		.all(&app.system())
		.await
		.expect("events");
	assert_eq!(told.len(), 1, "the tenant's owners are told");
	let hub = hub(&shop, "eu-one").await;
	let given = ask(&hub, json!([])).await;
	let secrets = &given["changed"][0]["secrets"];
	assert_eq!(secrets["token"], token.as_str());
	assert_eq!(secrets["database_url"], address.as_str());
	let next_token = secrets["next"]["token"]
		.as_str()
		.expect("a token")
		.to_owned();
	let next_address = secrets["next"]["database_url"]
		.as_str()
		.expect("an address")
		.to_owned();
	assert_eq!(next_token.len(), 64);
	assert_ne!(next_token, token);
	assert_ne!(next_address, address);
	let database = first["input"]["notifications"]["name"]
		.as_str()
		.expect("a name");
	assert!(
		next_address.starts_with(&format!("postgres://{database}:")),
		"{next_address}"
	);
	let leases = NotificationLease::query()
		.all(&app.system())
		.await
		.expect("leases");
	assert!(
		leases
			.last()
			.expect("a lease")
			.names
			.0
			.iter()
			.any(|name| name.ends_with("(next)")),
		"{leases:?}"
	);
	let known = json!([{ "tenant": tenant, "version": given["changed"][0]["version"] }]);

	// The next release is given the new values; it fails, and the hub is
	// told to open the tenant again.
	let digest = shop.upload(b"failing").await;
	shop.deploy("production", &digest, with_notifications())
		.await
		.expect(200);
	let job = shop.build.take().await;
	shop.build
		.finish(&job, built(&job, b"failing", &[]))
		.await
		.expect(200);
	let release = shop.cluster.take().await;
	assert_eq!(
		values_of(&release),
		(next_token.clone(), next_address.clone())
	);
	shop.cluster
		.finish(
			&release,
			json!({
				"kind": "release",
				"result": "not_ready",
				"process": "app",
				"reason": "it didn't start",
			}),
		)
		.await
		.expect(200);
	assert_eq!(summary(&shop, "production").await["rotating"], true);
	let reopened = ask(&hub, known).await;
	assert_eq!(
		reopened["changed"][0]["secrets"]["next"]["token"],
		next_token.as_str()
	);

	// One that is applied finishes it: the new values are the tenant's
	// alone, given to the hub and to every release after.
	let applied_release = ship(&shop, "production", b"second", with_notifications()).await;
	assert_eq!(
		values_of(&applied_release),
		(next_token.clone(), next_address.clone())
	);
	assert_eq!(summary(&shop, "production").await["rotating"], false);
	let moved = ask(&hub, json!([])).await;
	let secrets = &moved["changed"][0]["secrets"];
	assert_eq!(secrets["token"], next_token.as_str());
	assert_eq!(secrets["database_url"], next_address.as_str());
	assert_eq!(secrets.get("next"), None);
	let third = ship(&shop, "production", b"third", with_notifications()).await;
	assert_eq!(values_of(&third), (next_token.clone(), next_address));

	// Revoking the cluster's own worker, which leased them to every
	// release, gives new ones again.
	app.call(
		"revoke_worker",
		json!({ "worker": shop.cluster.id }),
		shop.as_ops(),
	)
	.await
	.expect(200);
	assert_eq!(summary(&shop, "production").await["rotating"], true);
	let again = ask(&hub, json!([])).await;
	let next = &again["changed"][0]["secrets"]["next"];
	assert_ne!(next["token"], next_token.as_str());
	assert_ne!(next["token"], Value::Null);
}

/// [`super::billing::catalogue`], with hosted pushes priced at ten minor
/// units a thousand and ten of them included in Hobby, which costs nothing.
pub fn catalogue() -> leptoskit_cloud::pricing::Catalogue {
	leptoskit_cloud::server::catalogue_from_json(
		r#"{
			"currency": "usd",
			"plans": [
				{
					"id": "hobby", "name": "Hobby", "monthly": 0,
					"allowances": { "build_minutes_linux": 1000, "notification_pushes": 10 }
				},
				{
					"id": "pro", "name": "Pro", "monthly": 2000, "included_usage": 20,
					"allowances": { "notification_pushes": 10 },
					"spend_cap": 50
				}
			],
			"meters": {
				"build_minutes_linux": { "minor": 10 },
				"compute_seconds_xs": { "minor": 0 },
				"notification_pushes": { "minor": 10, "per": 1000 }
			}
		}"#,
	)
	.expect("a catalogue")
}

/// On a plan that costs nothing, an organisation's pushes stop once they
/// reach the plan's allowance: every answer names its tenants as paused
/// until it is on a paid plan. What the hub sent before it heard is never
/// charged, so it never reaches the free plan's cap. Run with [`catalogue`].
#[expect(
	clippy::cognitive_complexity,
	reason = "a scenario is one story, read top to bottom; its asserts are what count"
)]
pub async fn a_free_plan_stops_pushing_at_its_allowance(app: &App) {
	let shop = Shop::open(app).await;
	with_hub(&shop, "eu-one").await;
	ship(&shop, "production", b"first", with_notifications()).await;
	let made = summary(&shop, "production").await;
	let tenant = made["tenant"].as_str().expect("a tenant").to_owned();
	assert_eq!(made["pushes_paused"], false);

	let hub = hub(&shop, "eu-one").await;
	let answer = ask(&hub, json!([])).await;
	assert_eq!(answer["paused"], json!([]));
	let version = answer["changed"][0]["version"].clone();
	let known = json!([{ "tenant": tenant, "version": version }]);
	let report = |after: Value, until: &str, pushes: u64| {
		hub.call(
			"report_notifications",
			json!({
				"usage": [{ "tenant": tenant, "after": after, "until": until, "pushes": pushes }],
			}),
		)
	};

	// Under the allowance, it pushes.
	report(Value::Null, "c1", 9).await.expect(200);
	assert_eq!(ask(&hub, known.clone()).await["paused"], json!([]));
	assert_eq!(summary(&shop, "production").await["pushes_paused"], false);

	// At it, every answer names the tenant, with nothing else changed.
	report(json!("c1"), "c2", 1).await.expect(200);
	let stopped = ask(&hub, known.clone()).await;
	assert_eq!(stopped["paused"], json!([tenant]));
	assert_eq!(stopped["changed"], json!([]));
	assert_eq!(ask(&hub, known.clone()).await["paused"], json!([tenant]));
	assert_eq!(summary(&shop, "production").await["pushes_paused"], true);

	// What the hub sent before it heard is counted and never charged: the
	// free plan's cap isn't reached, and deploys go on.
	report(json!("c2"), "c3", 500).await.expect(200);
	assert_eq!(
		summary(&shop, "production").await["project_pushes_this_month"],
		510
	);
	let cost = super::billing::usage(&shop).await["cost"].clone();
	assert_eq!(
		(&cost["usage"], &cost["cap_reached"], &cost["unpriced"]),
		(&json!(0), &json!(false), &json!([])),
		"{cost}"
	);
	assert_eq!(
		pushes_line(&cost),
		json!({ "meter": "notification_pushes", "quantity": 10, "allowance": 10, "billable": 0, "amount": 0 })
	);
	ship(&shop, "production", b"second", with_notifications()).await;

	// A paid plan pays for pushes past its allowance, and starts them again.
	super::billing::put_on(&shop, "pro").await;
	let answer = ask(&hub, json!([])).await;
	assert_eq!(answer["paused"], json!([]));
	assert_eq!(summary(&shop, "production").await["pushes_paused"], false);
	let cost = super::billing::usage(&shop).await["cost"].clone();
	assert_eq!(
		pushes_line(&cost),
		json!({ "meter": "notification_pushes", "quantity": 510, "allowance": 10, "billable": 500, "amount": 5 })
	);
}

/// The bill's line for hosted pushes.
fn pushes_line(cost: &Value) -> Value {
	cost["lines"]
		.as_array()
		.and_then(|lines| {
			lines
				.iter()
				.find(|line| line["meter"] == "notification_pushes")
		})
		.cloned()
		.unwrap_or_else(|| panic!("no line for pushes: {cost}"))
}
