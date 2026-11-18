//! Variables and the secrets vault, written once and run on SQLite
//! (`deployments.rs`) and Postgres (`postgres.rs`), as `shipping.rs`'s
//! scenarios are.

use leptoskit_cloud::tables::AuditEvent;
use leptoskit_cloud::tables::DataKey;
use leptoskit_cloud::tables::Job;
use leptoskit_cloud::tables::Secret;
use leptoskit_cloud::tables::SecretLease;
use leptoskit_db::Id;
use serde_json::Value;
use serde_json::json;

use super::Answer;
use super::App;
use super::As;
use super::shipping::Shop;
use super::shipping::applied;
use super::shipping::built;

/// A value nothing else in a test's database could hold by chance.
const SECRET: &str = "sk-live-7d1f0c2e9a4b-hunter2-vault";
const ROTATED: &str = "sk-live-55aa91c3e7d0-rotated-vault";

fn place(shop: &Shop<'_>, environment: &str) -> Value {
	json!({ "organization": shop.org, "project": "notes", "environment": environment })
}

fn named(shop: &Shop<'_>, environment: &str, name: &str) -> Value {
	let mut args = place(shop, environment);
	args["name"] = json!(name);
	args
}

fn valued(shop: &Shop<'_>, environment: &str, name: &str, value: &str) -> Value {
	let mut args = named(shop, environment, name);
	args["value"] = json!(value);
	args
}

async fn set(shop: &Shop<'_>, operation: &str, name: &str, value: &str) -> Answer {
	shop.app
		.call(
			operation,
			valued(shop, "production", name, value),
			shop.as_ana(),
		)
		.await
}

/// Deploys `bytes` to production and builds it, leaving its release queued.
/// Returns the deployment's number.
async fn build(shop: &Shop<'_>, bytes: &[u8]) -> u64 {
	let digest = shop.upload(bytes).await;
	let deployed = shop
		.deploy("production", &digest, json!({ "database": "sqlite" }))
		.await;
	let number = deployed.expect(200).json()["number"]
		.as_u64()
		.expect("a number");
	let job = shop.build.take().await;
	// A build is never given a secret.
	assert!(job.get("secrets").is_none(), "{job}");
	shop.build
		.finish(&job, built(&job, bytes, &[]))
		.await
		.expect(200);
	number
}

/// Variables and secrets set on an environment reach its next release: the
/// variables in the spec, the secrets by name in the spec and by value only
/// in the cluster worker's claim, with a lease recorded for each. No
/// answer, table, receipt, or audit entry holds a secret's value.
#[expect(
	clippy::cognitive_complexity,
	reason = "a scenario is one story, read top to bottom; its asserts are what count"
)]
pub async fn variables_and_secrets_reach_the_release(app: &App) {
	let shop = Shop::open(app).await;
	let greeting = set(&shop, "set_variable", "GREETING", "hello there").await;
	assert_eq!(greeting.expect(200).json()["value"], "hello there");
	// A client retrying `set_secret` with an idempotency key leaves no
	// receipt, whose arguments' hash would be a way to guess the value.
	let stored = app
		.call_with_key(
			"set_secret",
			valued(&shop, "production", "API_KEY", SECRET),
			shop.as_ana(),
			"retry-me",
		)
		.await;
	let stored = stored.expect(200);
	assert!(!stored.text.contains(SECRET), "{}", stored.text);
	assert_eq!(stored.json()["name"], "API_KEY");
	assert_eq!(stored.json()["version"], 1);

	let variables = app
		.call("variables", place(&shop, "production"), shop.as_ana())
		.await;
	let variables = variables.expect(200).json();
	assert_eq!(variables.as_array().map(Vec::len), Some(1), "{variables}");
	assert_eq!(variables[0]["name"], "GREETING");
	assert_eq!(variables[0]["value"], "hello there");

	let secrets = app
		.call("secrets", place(&shop, "production"), shop.as_ana())
		.await;
	let secrets = secrets.expect(200);
	assert!(!secrets.text.contains(SECRET), "{}", secrets.text);
	let listed = secrets.json();
	assert_eq!(listed.as_array().map(Vec::len), Some(1), "{listed}");
	assert_eq!(listed[0]["name"], "API_KEY");
	assert!(listed[0].get("value").is_none(), "{listed}");

	// Another environment has neither.
	app.call(
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
	for operation in ["variables", "secrets"] {
		let other = app
			.call(operation, place(&shop, "staging"), shop.as_ana())
			.await;
		assert_eq!(other.expect(200).json(), json!([]), "{operation}");
	}

	let number = build(&shop, b"with config").await;
	let release = shop.cluster.take().await;
	let process = &release["input"]["spec"]["processes"][0];
	assert_eq!(process["env"]["GREETING"], "hello there", "{release}");
	assert_eq!(process["env"]["LEPTOSKIT_ENVIRONMENT"], "production");
	// The spec names the secret; only the claim's answer has its value.
	assert_eq!(process["secret_env"]["API_KEY"], "API_KEY", "{release}");
	assert!(!release["input"].to_string().contains(SECRET), "{release}");
	assert_eq!(release["secrets"], json!({ "API_KEY": SECRET }));

	// The lease says which version went to which job and worker.
	let job = Id::<Job>::parse(release["job"].as_str().expect("a job")).expect("an id");
	let leases = SecretLease::query()
		.filter(|lease| lease.job_id.eq(job))
		.all(&app.system())
		.await
		.expect("leases");
	assert_eq!(leases.len(), 1, "{leases:?}");
	assert_eq!(leases[0].name, "API_KEY");
	assert_eq!(leases[0].version, 1);
	assert_eq!(leases[0].attempt, 1);
	assert_eq!(leases[0].worker_id.uuid().to_string(), shop.cluster.id);

	shop.cluster.finish(&release, applied()).await.expect(200);
	assert_eq!(
		shop.state("production", number).await,
		json!({ "state": "live" })
	);

	// The sheet shows the spec, with the secret's name and not its value.
	let sheet = shop.on("deployment", "production", number).await;
	let sheet = sheet.expect(200);
	assert!(sheet.text.contains("API_KEY"), "{}", sheet.text);
	assert!(!sheet.text.contains(SECRET), "{}", sheet.text);

	// Nothing stored holds the value: not the secret's row, a receipt, the
	// job, the deployment, an event, or the audit log.
	let found = app.find_in_every_table(SECRET).await;
	assert!(found.is_empty(), "the secret's value is stored: {found:?}");
	let receipts = leptoskit_rpc::server::receipts::Receipt::query()
		.filter(|receipt| receipt.operation.eq("set_secret"))
		.all(&app.system())
		.await
		.expect("receipts");
	assert!(receipts.is_empty(), "{receipts:?}");
	let audit = AuditEvent::query()
		.filter(|event| event.action.eq("secret.set"))
		.all(&app.system())
		.await
		.expect("the audit log");
	assert_eq!(audit.len(), 1, "{audit:?}");
	assert_eq!(
		audit[0].target.as_deref(),
		Some("production API_KEY version 1")
	);

	// A new value is a new version, which the next release gets.
	let rotated = set(&shop, "set_secret", "API_KEY", ROTATED).await;
	assert_eq!(rotated.expect(200).json()["version"], 2);
	build(&shop, b"with a rotated secret").await;
	let release = shop.cluster.take().await;
	assert_eq!(release["secrets"], json!({ "API_KEY": ROTATED }));
	shop.cluster.finish(&release, applied()).await.expect(200);
	let job = Id::<Job>::parse(release["job"].as_str().expect("a job")).expect("an id");
	let leases = SecretLease::query()
		.filter(|lease| lease.job_id.eq(job))
		.all(&app.system())
		.await
		.expect("leases");
	assert_eq!(
		leases.iter().map(|lease| lease.version).collect::<Vec<_>>(),
		[2]
	);
	let found = app.find_in_every_table(ROTATED).await;
	assert!(found.is_empty(), "the secret's value is stored: {found:?}");

	// Removing a variable takes it out of the next release.
	app.call(
		"unset_variable",
		named(&shop, "production", "GREETING"),
		shop.as_ana(),
	)
	.await
	.expect(200);
	app.call(
		"unset_variable",
		named(&shop, "production", "GREETING"),
		shop.as_ana(),
	)
	.await
	.refused(404, "variable_not_found");
	build(&shop, b"without the greeting").await;
	let release = shop.cluster.take().await;
	let process = &release["input"]["spec"]["processes"][0];
	assert!(process["env"].get("GREETING").is_none(), "{release}");
	shop.cluster.finish(&release, applied()).await.expect(200);
}

/// Names follow the rules, a name is a variable or a secret and never both,
/// and values are bounded. An answer never repeats what might be a value.
pub async fn names_and_values_are_checked(app: &App) {
	let shop = Shop::open(app).await;

	for operation in ["set_variable", "set_secret"] {
		for bad in [
			"lower_case",
			"1STARTS_WITH_A_DIGIT",
			"HAS SPACE",
			"",
			"KEY=sk-live-oops",
		] {
			let answer = set(&shop, operation, bad, "value").await;
			answer.refused(400, "invalid_name");
			assert!(!answer.text.contains("sk-live-oops"), "{}", answer.text);
		}

		for reserved in ["DATABASE_URL", "PORT", "LEPTOSKIT_ENVIRONMENT"] {
			let answer = set(&shop, operation, reserved, "value").await;
			answer.refused(400, "invalid_name");
			assert!(answer.text.contains("set by the cloud"), "{}", answer.text);
		}

		set(&shop, operation, "HAS_NUL", "a\0b")
			.await
			.refused(400, "value_not_text");
	}

	set(&shop, "set_secret", "EMPTY", "")
		.await
		.refused(400, "value_empty");
	// An empty variable is a value like any other.
	set(&shop, "set_variable", "EMPTY", "").await.expect(200);

	let long = "x".repeat(leptoskit_cloud::config::MAX_VARIABLE_VALUE_LEN + 1);
	set(&shop, "set_variable", "LONG", &long)
		.await
		.refused(400, "value_too_long");
	let long = "x".repeat(leptoskit_cloud::config::MAX_SECRET_VALUE_LEN + 1);
	let answer = set(&shop, "set_secret", "LONG", &long).await;
	answer.refused(400, "value_too_long");
	assert!(!answer.text.contains("xxxxxxxx"), "{}", answer.text);

	// One name, one kind.
	set(&shop, "set_variable", "MODE", "fast").await.expect(200);
	set(&shop, "set_secret", "MODE", SECRET)
		.await
		.refused(409, "name_is_variable");
	set(&shop, "set_secret", "API_KEY", SECRET)
		.await
		.expect(200);
	let answer = set(&shop, "set_variable", "API_KEY", "plain").await;
	answer.refused(409, "name_is_secret");

	// Setting a variable again replaces it.
	set(&shop, "set_variable", "MODE", "careful")
		.await
		.expect(200);
	let variables = app
		.call("variables", place(&shop, "production"), shop.as_ana())
		.await;
	let variables = variables.expect(200).json();
	let mode = variables
		.as_array()
		.expect("variables")
		.iter()
		.find(|variable| variable["name"] == "MODE")
		.expect("MODE");
	assert_eq!(mode["value"], "careful");

	app.call(
		"unset_secret",
		named(&shop, "production", "NEVER_SET"),
		shop.as_ana(),
	)
	.await
	.refused(404, "secret_not_found");
	// An environment that isn't there.
	app.call("secrets", place(&shop, "nowhere"), shop.as_ana())
		.await
		.refused(404, "environment_not_found");
}

/// Reading is for whoever sees the project, writing for whoever manages its
/// configuration, and nobody outside the organisation learns that either
/// exists. A token needs `manage`, and its own project.
#[expect(
	clippy::cognitive_complexity,
	reason = "a scenario is one story, read top to bottom; its asserts are what count"
)]
pub async fn configuration_is_the_tenants_alone(app: &App) {
	let shop = Shop::open(app).await;
	set(&shop, "set_variable", "GREETING", "hello")
		.await
		.expect(200);
	set(&shop, "set_secret", "API_KEY", SECRET)
		.await
		.expect(200);

	let viewer = app
		.join(shop.as_ana(), &shop.org, "vera@example.com", "viewer")
		.await;
	let viewer = As::Session(&viewer);

	// A viewer reads variables and secrets' names …
	let variables = app
		.call("variables", place(&shop, "production"), viewer)
		.await;
	assert_eq!(variables.expect(200).json()[0]["value"], "hello");
	let secrets = app
		.call("secrets", place(&shop, "production"), viewer)
		.await;
	let secrets = secrets.expect(200);
	assert_eq!(secrets.json()[0]["name"], "API_KEY");
	assert!(!secrets.text.contains(SECRET));

	// … and changes nothing.
	for (operation, args) in [
		(
			"set_variable",
			valued(&shop, "production", "GREETING", "bye"),
		),
		("unset_variable", named(&shop, "production", "GREETING")),
		("set_secret", valued(&shop, "production", "API_KEY", "mine")),
		("unset_secret", named(&shop, "production", "API_KEY")),
	] {
		app.call(operation, args, viewer)
			.await
			.refused(403, "insufficient_role");
	}

	// Someone outside the organisation is told nothing is there, for a
	// read and a write alike, and their write changes nothing.
	let stranger = app.sign_in("mallory@example.com").await;
	let stranger = As::Session(&stranger);
	for (operation, args) in [
		("variables", place(&shop, "production")),
		("secrets", place(&shop, "production")),
		(
			"set_variable",
			valued(&shop, "production", "GREETING", "bye"),
		),
		("set_secret", valued(&shop, "production", "API_KEY", "mine")),
		("unset_secret", named(&shop, "production", "API_KEY")),
	] {
		app.call(operation, args, stranger)
			.await
			.refused(404, "organization_not_found");
	}

	// Nobody at all.
	app.call(
		"set_secret",
		valued(&shop, "production", "API_KEY", "x"),
		As::Nobody,
	)
	.await
	.expect(401);

	// Tokens: reading with any scope, writing with `manage` alone.
	let (read, _) = app.token(shop.as_ana(), &shop.org, &["read"], None).await;
	let (deploy, _) = app.token(shop.as_ana(), &shop.org, &["deploy"], None).await;
	let (manage, _) = app.token(shop.as_ana(), &shop.org, &["manage"], None).await;

	for token in [&read, &deploy] {
		let secrets = app
			.call("secrets", place(&shop, "production"), As::Bearer(token))
			.await;
		assert!(!secrets.expect(200).text.contains(SECRET));
		app.call(
			"set_secret",
			valued(&shop, "production", "API_KEY", "theirs"),
			As::Bearer(token),
		)
		.await
		.refused(403, "insufficient_scope");
		app.call(
			"set_variable",
			valued(&shop, "production", "GREETING", "theirs"),
			As::Bearer(token),
		)
		.await
		.refused(403, "insufficient_scope");
	}

	app.call(
		"set_secret",
		valued(&shop, "production", "TOKEN_SET", ROTATED),
		As::Bearer(&manage),
	)
	.await
	.expect(200);

	// A token bound to another project sees no `notes`.
	app.project(shop.as_ana(), &shop.org, "other").await;
	let (bound, _) = app
		.token(shop.as_ana(), &shop.org, &["manage"], Some("other"))
		.await;
	for (operation, args) in [
		("secrets", place(&shop, "production")),
		("set_secret", valued(&shop, "production", "API_KEY", "x")),
	] {
		app.call(operation, args, As::Bearer(&bound))
			.await
			.refused(404, "project_not_found");
	}

	// Ana's values are what they were.
	let variables = app
		.call("variables", place(&shop, "production"), shop.as_ana())
		.await;
	assert_eq!(variables.expect(200).json()[0]["value"], "hello");
	let row = Secret::query()
		.filter(|secret| secret.name.eq("API_KEY"))
		.one(&app.system())
		.await
		.expect("the secret");
	assert_eq!(row.version, 1);

	// A cluster worker's key, which is given secrets with a release, reads
	// and writes none through these operations.
	for (operation, args) in [
		("secrets", place(&shop, "production")),
		("variables", place(&shop, "production")),
		("set_secret", valued(&shop, "production", "API_KEY", "x")),
	] {
		shop.cluster
			.call(operation, args)
			.await
			.refused(403, "not_for_workers");
	}
}

/// A secret can't be removed from under a deployment in flight, and a
/// rollback to a release that starts with a removed secret is refused until
/// the secret is set again; it then gets the value as it is now.
pub async fn a_release_always_finds_its_secrets(app: &App) {
	let shop = Shop::open(app).await;
	set(&shop, "set_secret", "API_KEY", SECRET)
		.await
		.expect(200);
	let first = build(&shop, b"first").await;

	// In flight: the release is queued, naming the secret.
	app.call(
		"unset_secret",
		named(&shop, "production", "API_KEY"),
		shop.as_ana(),
	)
	.await
	.refused(409, "deployment_in_flight");
	let release = shop.cluster.take().await;
	assert_eq!(release["secrets"], json!({ "API_KEY": SECRET }));
	shop.cluster.finish(&release, applied()).await.expect(200);

	// Live: it can go. What runs keeps it until the next deployment.
	app.call(
		"unset_secret",
		named(&shop, "production", "API_KEY"),
		shop.as_ana(),
	)
	.await
	.expect(200);
	let second = build(&shop, b"second").await;
	let release = shop.cluster.take().await;
	assert!(release.get("secrets").is_none(), "{release}");
	let process = &release["input"]["spec"]["processes"][0];
	assert_eq!(process["secret_env"], json!({}), "{release}");
	shop.cluster.finish(&release, applied()).await.expect(200);
	assert_eq!(
		shop.state("production", second).await,
		json!({ "state": "live" })
	);

	// The first release starts with a secret that is gone.
	let rollback = json!({
		"organization": shop.org,
		"project": "notes",
		"environment": "production",
		"to": first,
	});
	let refused = app.call("rollback", rollback.clone(), shop.as_ana()).await;
	refused.refused(409, "secret_missing");
	assert!(refused.text.contains("API_KEY"), "{}", refused.text);

	// Set again, the rollback goes out with the value as it is now.
	set(&shop, "set_secret", "API_KEY", ROTATED)
		.await
		.expect(200);
	app.call("rollback", rollback, shop.as_ana())
		.await
		.expect(200);
	let release = shop.cluster.take().await;
	assert_eq!(release["secrets"], json!({ "API_KEY": ROTATED }));
	shop.cluster.finish(&release, applied()).await.expect(200);
}

/// A sealed value opens only where it was sealed: a row copied onto another
/// environment's secret, or changed, isn't handed to anyone. Its release
/// fails as `secret_unavailable` before a worker is given it, and the
/// worker's queue moves on.
#[expect(
	clippy::cognitive_complexity,
	reason = "a scenario is one story, read top to bottom; its asserts are what count"
)]
pub async fn a_copied_secret_opens_nowhere_else(app: &App) {
	let shop = Shop::open(app).await;
	app.call(
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
	set(&shop, "set_secret", "API_KEY", SECRET)
		.await
		.expect(200);
	app.call(
		"set_secret",
		valued(&shop, "staging", "API_KEY", "staging-only-value"),
		shop.as_ana(),
	)
	.await
	.expect(200);

	// Someone with the database copies production's sealed value onto
	// staging's row, hoping a staging release will open it for them.
	let system = app.system();
	let rows = Secret::query()
		.order_by(|secret| secret.created_at.asc())
		.all(&system)
		.await
		.expect("the secrets");
	assert_eq!(rows.len(), 2, "{rows:?}");
	let production = rows[0].sealed.clone();
	let staging = rows[1].id;
	Secret::update()
		.filter(|secret| secret.id.eq(staging))
		.set(|secret| secret.sealed, production)
		.run(&system)
		.await
		.expect("copy the sealed value");

	let digest = shop.upload(b"staging").await;
	let deployed = shop
		.deploy("staging", &digest, json!({ "database": "sqlite" }))
		.await;
	let number = deployed.expect(200).json()["number"]
		.as_u64()
		.expect("a number");
	let job = shop.build.take().await;
	shop.build
		.finish(&job, built(&job, b"staging", &[]))
		.await
		.expect(200);

	// The claim gives the worker nothing, and the release fails.
	assert_eq!(shop.cluster.claim().await, None);
	let sheet = shop.on("deployment", "staging", number).await;
	let sheet = sheet.expect(200);
	assert_eq!(
		sheet.json()["deployment"]["state"],
		json!({ "state": "failed", "stage": "rolling_out", "blocker": "secret_unavailable" }),
		"{}",
		sheet.text
	);
	assert!(!sheet.text.contains(SECRET), "{}", sheet.text);
	assert!(!sheet.text.contains("staging-only-value"), "{}", sheet.text);
	let leases = SecretLease::query().all(&system).await.expect("leases");
	assert!(leases.is_empty(), "{leases:?}");

	// Production's own release still gets production's value, and the
	// worker isn't stuck behind the failed one.
	build(&shop, b"production").await;
	let release = shop.cluster.take().await;
	assert_eq!(release["secrets"], json!({ "API_KEY": SECRET }));
	shop.cluster.finish(&release, applied()).await.expect(200);

	// An organisation's secrets are sealed with its own key, wrapped: the
	// table holds one row for it, and not the key.
	let keys = DataKey::query().all(&system).await.expect("the data keys");
	assert_eq!(keys.len(), 1, "{}", keys.len());
}

/// The first secrets of an organisation, stored at the same moment, share
/// one data key: every one of them is stored or told to try again, and all
/// that were stored open.
pub async fn first_secrets_race_to_one_key(app: &App) {
	let shop = Shop::open(app).await;
	let racing = (0..6).map(|index| {
		let args = valued(
			&shop,
			"production",
			&format!("RACED_{index}"),
			&format!("{SECRET}-{index}"),
		);
		let caller = shop.as_ana();
		async move { app.call("set_secret", args, caller).await }
	});
	let answers = futures::future::join_all(racing).await;

	for answer in &answers {
		assert!(
			answer.status == 200
				|| (answer.status == 409
					&& (answer.message().starts_with("vault_busy")
						|| answer.message().starts_with("contended"))),
			"{answer:?}"
		);
	}

	let stored = answers.iter().filter(|answer| answer.status == 200).count();
	assert!(stored >= 1, "{answers:?}");
	let keys = DataKey::query()
		.all(&app.system())
		.await
		.expect("the data keys");
	assert_eq!(keys.len(), 1);

	build(&shop, b"raced").await;
	let release = shop.cluster.take().await;
	let leased = release["secrets"].as_object().expect("secrets").clone();
	assert_eq!(leased.len(), stored, "{answers:?}");

	for (name, value) in leased {
		let index = name.strip_prefix("RACED_").expect("a raced name");
		assert_eq!(value, format!("{SECRET}-{index}"));
	}

	shop.cluster.finish(&release, applied()).await.expect(200);
}
