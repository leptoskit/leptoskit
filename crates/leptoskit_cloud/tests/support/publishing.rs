//! Publishing to app and extension stores from a build worker, written
//! once and run on SQLite (`deployments.rs`) and Postgres (`postgres.rs`).

use serde_json::Value;
use serde_json::json;

use super::App;
use super::shipping::Shop;
use super::shipping::Worker;
use super::shipping::applied;
use super::shipping::built;
use super::shipping::pair_with;

/// A value nothing else in a test's database could hold by chance.
const CHROME_KEY: &str = "{\"private_key\":\"pk-7c1e0d5a9b3f-chrome-publisher\"}";
const CHROME: &str = "LEPTOSKIT_PUBLISH_CHROME_SERVICE_ACCOUNT";

/// Sets the secret `name` on production, as Ana.
async fn secret(shop: &Shop<'_>, name: &str, value: &str) {
	shop.app
		.call(
			"set_secret",
			json!({
				"organization": shop.org,
				"project": "notes",
				"environment": "production",
				"name": name,
				"value": value,
			}),
			shop.as_ana(),
		)
		.await
		.expect(200);
}

/// `publish` of `bytes` to `stores` on production, as Ana.
async fn publish(shop: &Shop<'_>, bytes: &[u8], stores: &[&str]) -> super::Answer {
	let digest = shop.upload(bytes).await;
	shop.app
		.call(
			"publish",
			json!({
				"organization": shop.org,
				"project": "notes",
				"environment": "production",
				"archive": digest,
				"stores": stores,
			}),
			shop.as_ana(),
		)
		.await
}

/// `operation` on the publish `id`, as Ana.
async fn on(shop: &Shop<'_>, operation: &str, id: &Value) -> Value {
	shop.app
		.call(
			operation,
			json!({
				"organization": shop.org,
				"project": "notes",
				"environment": "production",
				"id": id,
			}),
			shop.as_ana(),
		)
		.await
		.expect(200)
		.json()
}

/// A build worker that publishes.
async fn publisher<'a>(shop: &Shop<'a>, name: &str, capabilities: &[&str]) -> Worker<'a> {
	pair_with(
		shop.app,
		&shop.ops,
		json!({ "role": "build", "organization": shop.org }),
		name,
		capabilities,
	)
	.await
}

/// A publish goes to a build worker that declares it publishes, with the
/// environment's credentials for its stores and nothing else; its log and
/// its report come back to the organisation, and no table holds a value.
#[expect(
	clippy::cognitive_complexity,
	reason = "a scenario is one story, read top to bottom; its asserts are what count"
)]
pub async fn a_publish_carries_its_stores_credentials(app: &App) {
	let shop = Shop::open(app).await;
	secret(&shop, CHROME, CHROME_KEY).await;
	secret(&shop, "API_KEY", "sk-live-unrelated-app-secret-91").await;

	// A store credential is a publish's: the app's release never gets it.
	let digest = shop.upload(b"notes app").await;
	shop.deploy("production", &digest, json!({ "database": "sqlite" }))
		.await
		.expect(200);
	let build = shop.build.take().await;
	shop.build
		.finish(&build, built(&build, b"notes app", &[]))
		.await
		.expect(200);
	let release = shop.cluster.take().await;
	assert!(release["secrets"].get("API_KEY").is_some(), "{release}");
	assert!(release["secrets"].get(CHROME).is_none(), "{release}");
	shop.cluster.finish(&release, applied()).await.expect(200);

	// Unknown stores, none, or one twice are refused before anything is
	// queued.
	publish(&shop, b"bad stores", &["steam"])
		.await
		.refused(400, "invalid_stores");
	publish(&shop, b"bad stores", &[])
		.await
		.refused(400, "invalid_stores");
	publish(&shop, b"bad stores", &["play", "play"])
		.await
		.refused(400, "invalid_stores");

	let queued = publish(&shop, b"notes 1.4", &["play", "chrome"])
		.await
		.expect(200)
		.json();
	assert_eq!(queued["stores"], json!(["chrome", "play"]), "{queued}");
	assert_eq!(queued["credentials"], json!([CHROME]), "{queued}");
	assert_eq!(queued["requirements"], json!(["publish"]), "{queued}");
	assert_eq!(queued["state"], "queued");
	assert_eq!(queued["submit"], false);

	// One at a time per environment.
	publish(&shop, b"notes 1.5", &["chrome"])
		.await
		.refused(409, "publish_in_flight");

	// A build worker that doesn't publish is never offered it, nor is one
	// of the shared pool that does: nothing isolates one organisation's
	// job from the next there.
	assert!(shop.build.claim().await.is_none());
	let shared = pair_with(
		shop.app,
		&shop.ops,
		json!({ "role": "build" }),
		"shared-publisher",
		&["publish", "macos"],
	)
	.await;
	assert!(shared.claim().await.is_none());

	let worker = publisher(&shop, "publisher-1", &["linux", "publish"]).await;
	let job = worker.take().await;
	assert_eq!(job["input"]["kind"], "publish", "{job}");
	assert_eq!(job["input"]["stores"], json!(["chrome", "play"]));
	assert_eq!(job["input"]["organization"], shop.org);
	assert!(
		job["input"]["archive"]["url"].as_str().is_some(),
		"a download address: {job}"
	);
	assert_eq!(job["secrets"], json!({ CHROME: CHROME_KEY }), "{job}");

	worker
		.log(&job, 0, &["building play", "uploading chrome"])
		.await
		.expect(200);
	let log = shop
		.app
		.call(
			"publication_log",
			json!({
				"organization": shop.org,
				"project": "notes",
				"environment": "production",
				"id": queued["id"],
			}),
			shop.as_ana(),
		)
		.await
		.expect(200)
		.json();
	let lines: Vec<&str> = log["lines"]
		.as_array()
		.expect("lines")
		.iter()
		.map(|line| line["text"].as_str().expect("text"))
		.collect();
	assert_eq!(lines, ["building play", "uploading chrome"], "{log}");
	assert_eq!(log["lines"][0]["n"], 1);

	worker
		.finish(
			&job,
			json!({
				"kind": "publish",
				"result": "published",
				"stores": [
					{ "store": "chrome", "summary": "uploaded 1.4 as a draft" },
					{ "store": "play", "summary": "uploaded 1.4 (12) to internal" },
				],
				"steps": [{
					"store": "play",
					"by": "person",
					"what": "Fill in the data safety form",
					"why": "the API has no endpoint for it",
					"done_when": "the console shows it complete",
				}],
			}),
		)
		.await
		.expect(200);

	let done = on(&shop, "publication", &queued["id"]).await;
	assert_eq!(done["state"], "done", "{done}");
	assert_eq!(done["outcome"]["result"], "published");
	assert_eq!(
		done["outcome"]["stores"][1]["summary"],
		"uploaded 1.4 (12) to internal"
	);
	assert_eq!(done["outcome"]["steps"][0]["by"], "person");

	let listed = shop
		.app
		.call(
			"publications",
			json!({ "organization": shop.org, "project": "notes", "environment": "production" }),
			shop.as_ana(),
		)
		.await
		.expect(200)
		.json();
	assert_eq!(listed.as_array().map(Vec::len), Some(1), "{listed}");

	// The value is sealed in the vault and nowhere else.
	let found = shop.app.find_in_every_table("pk-7c1e0d5a9b3f").await;
	assert!(found.is_empty(), "{found:?}");

	// An Apple store needs a Mac, which this worker isn't.
	let apple = publish(&shop, b"notes 1.4 for iOS", &["app_store"])
		.await
		.expect(200)
		.json();
	assert_eq!(apple["requirements"], json!(["publish", "macos"]));
	assert!(worker.claim().await.is_none());
	let mac = publisher(&shop, "mac-1", &["macos", "publish"]).await;
	let job = mac.take().await;
	assert_eq!(job["input"]["stores"], json!(["app_store"]));
	// The environment has none of the App Store's credentials.
	assert!(job.get("secrets").is_none(), "{job}");
}

/// A publish whose worker goes quiet fails and is never run again: it may
/// have reached a store. One whose credential was removed before a worker
/// claimed it fails at the claim, and nothing is leased.
pub async fn a_publish_is_never_run_twice(app: &App) {
	let shop = Shop::open(app).await;
	secret(&shop, CHROME, CHROME_KEY).await;
	let worker = publisher(&shop, "publisher-1", &["publish"]).await;

	let first = publish(&shop, b"notes 2.0", &["chrome"])
		.await
		.expect(200)
		.json();
	let job = worker.take().await;
	shop.lapse_leases().await;
	shop.sweep().await;
	let lapsed = on(&shop, "publication", &first["id"]).await;
	assert_eq!(lapsed["state"], "abandoned", "{lapsed}");
	assert_eq!(lapsed["outcome"]["result"], "failed");
	assert!(
		lapsed["outcome"]["reason"]
			.as_str()
			.is_some_and(|reason| reason.contains("may have uploaded")),
		"{lapsed}"
	);
	assert!(worker.claim().await.is_none(), "run again");
	// Its outcome comes too late to count.
	worker
		.finish(
			&job,
			json!({ "kind": "publish", "result": "published", "stores": [] }),
		)
		.await
		.refused(409, "outcome_conflict");

	let second = publish(&shop, b"notes 2.1", &["chrome"])
		.await
		.expect(200)
		.json();
	shop.app
		.call(
			"unset_secret",
			json!({
				"organization": shop.org,
				"project": "notes",
				"environment": "production",
				"name": CHROME,
			}),
			shop.as_ana(),
		)
		.await
		.expect(200);
	assert!(worker.claim().await.is_none());
	let failed = on(&shop, "publication", &second["id"]).await;
	assert_eq!(failed["state"], "abandoned", "{failed}");
	assert!(
		failed["outcome"]["reason"]
			.as_str()
			.is_some_and(|reason| reason.contains("no longer has")),
		"{failed}"
	);
}
