//! Managed Postgres databases, written once and run on SQLite
//! (`deployments.rs`) and Postgres (`postgres.rs`), as `shipping.rs`'s
//! scenarios are.

use leptoskit_cloud::tables::AuditEvent;
use leptoskit_cloud::tables::Database;
use leptoskit_cloud::tables::SecretLease;
use serde_json::Value;
use serde_json::json;

use super::App;
use super::As;
use super::shipping::Shop;
use super::shipping::applied;
use super::shipping::built;

const HOST: &str = "postgres-rw.leptoskit-postgres.svc";

fn place(shop: &Shop<'_>, environment: &str) -> Value {
	json!({ "organization": shop.org, "project": "notes", "environment": environment })
}

fn facts(port: u16) -> Value {
	json!({
		"runtime_class": "gvisor",
		"wakes_sleeping": true,
		"postgres": {
			"namespace": "leptoskit-postgres",
			"host": HOST,
			"port": port,
			"tls": true,
		},
	})
}

/// Tells the control plane the shop's cluster has a Postgres server.
async fn with_postgres(shop: &Shop<'_>, port: u16) {
	shop.app
		.call(
			"set_cluster_facts",
			json!({ "name": "eu-one", "facts": facts(port) }),
			shop.as_ops(),
		)
		.await
		.expect(200);
}

async fn staging(shop: &Shop<'_>) {
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
}

/// Deploys `bytes`, a Postgres app, to `environment` and builds it, leaving
/// its release queued. Returns the deployment's number.
async fn build(shop: &Shop<'_>, environment: &str, bytes: &[u8]) -> u64 {
	let digest = shop.upload(bytes).await;
	let deployed = shop
		.deploy(environment, &digest, json!({ "database": "postgres" }))
		.await;
	let number = deployed.expect(200).json()["number"]
		.as_u64()
		.expect("a number");
	let job = shop.build.take().await;
	assert!(job.get("secrets").is_none(), "{job}");
	shop.build
		.finish(&job, built(&job, bytes, &[]))
		.await
		.expect(200);
	number
}

/// Builds and releases `bytes` to `environment`. Returns its number and
/// the release the cluster's worker was given.
async fn ship(shop: &Shop<'_>, environment: &str, bytes: &[u8]) -> (u64, Value) {
	let number = build(shop, environment, bytes).await;
	let release = shop.cluster.take().await;
	shop.cluster.finish(&release, applied()).await.expect(200);
	(number, release)
}

async fn summary(shop: &Shop<'_>, environment: &str) -> Value {
	shop.app
		.call("database", place(shop, environment), shop.as_ana())
		.await
		.expect(200)
		.json()
}

/// The password in a `DATABASE_URL` for the database `name`.
fn password(url: &str, name: &str, port: u16) -> String {
	let rest = url
		.strip_prefix(&format!("postgres://{name}:"))
		.unwrap_or_else(|| panic!("a URL for {name}: {url}"));
	let (password, rest) = rest.split_once('@').expect("a host");
	assert_eq!(rest, format!("{HOST}:{port}/{name}?sslmode=require"));
	assert_eq!(password.len(), 48, "{password}");
	assert!(
		password
			.chars()
			.all(|character| character.is_ascii_hexdigit()),
		"{password}"
	);
	password.to_owned()
}

/// An app whose `[db]` is Postgres gets a database of its own the first
/// time it is deployed to a cluster with a Postgres server. Its address
/// reaches the cluster's worker with the release, as a secret, and nothing
/// else: no answer, sheet, or table holds the password.
#[expect(
	clippy::cognitive_complexity,
	reason = "a scenario is one story, read top to bottom; its asserts are what count"
)]
pub async fn a_postgres_app_gets_a_database(app: &App) {
	let shop = Shop::open(app).await;
	let digest = shop.upload(b"first").await;

	// The cluster has no Postgres server yet.
	shop.deploy("production", &digest, json!({ "database": "postgres" }))
		.await
		.refused(400, "postgres_unavailable");
	assert_eq!(summary(&shop, "production").await, Value::Null);
	with_postgres(&shop, 5432).await;

	// A dedicated instance, and a service's own database, come later.
	let mut dedicated = place(&shop, "production");
	dedicated["source"] = json!({ "kind": "upload", "archive": digest });
	dedicated["config"] = json!({
		"app": { "database": "postgres" },
		"cloud": { "database": { "size": "m" } },
	});
	app.call("deploy", dedicated, shop.as_ana())
		.await
		.refused(400, "dedicated_database_unavailable");
	let mut service = place(&shop, "production");
	service["source"] = json!({ "kind": "upload", "archive": digest });
	service["config"] = json!({
		"app": { "database": "postgres", "services": { "search": { "database": true } } },
	});
	app.call("deploy", service, shop.as_ana())
		.await
		.refused(400, "service_database_unavailable");
	assert_eq!(summary(&shop, "production").await, Value::Null);

	let first = build(&shop, "production", b"first").await;
	let made = summary(&shop, "production").await;
	let name = made["name"].as_str().expect("a name").to_owned();
	assert!(name.starts_with("env_") && name.len() == 36, "{name}");
	assert_eq!(made["size"], "shared");
	assert_eq!(made["backups"], "daily");
	assert_eq!(made["ready"], false);

	let sheet = shop.on("deployment", "production", first).await;
	assert!(
		sheet.expect(200).text.contains("isn't backed up yet"),
		"{}",
		sheet.text
	);

	// The release names the database to make, the address as a secret, and
	// the door to the server. The value comes with the claim alone.
	let release = shop.cluster.take().await;
	let input = &release["input"];
	assert_eq!(
		input["database"],
		json!({ "name": name, "role": name, "size": "shared", "backups": "daily" })
	);
	let process = &input["spec"]["processes"][0];
	assert_eq!(
		process["secret_env"],
		json!({ "DATABASE_URL": "DATABASE_URL" })
	);
	assert!(process["env"].get("DATABASE_URL").is_none(), "{process}");
	assert_eq!(
		input["spec"]["policy"]["database"],
		json!({ "namespace": "leptoskit-postgres", "port": 5432 })
	);
	let secrets = release["secrets"].as_object().expect("secrets");
	assert_eq!(secrets.len(), 1, "{:?}", secrets.keys());
	let url = secrets["DATABASE_URL"].as_str().expect("an address");
	let password = password(url, &name, 5432);

	let system = app.system();
	let leases = SecretLease::query().all(&system).await.expect("leases");
	assert_eq!(leases.len(), 1, "{leases:?}");
	assert_eq!(leases[0].name, "DATABASE_URL");
	assert_eq!(leases[0].version, 1);

	shop.cluster.finish(&release, applied()).await.expect(200);
	assert_eq!(summary(&shop, "production").await["ready"], true);

	// Nobody is told the password, and no table holds it.
	for answer in [
		app.call("database", place(&shop, "production"), shop.as_ana())
			.await,
		shop.on("deployment", "production", first).await,
		shop.on("deployment_events", "production", first).await,
		app.call("jobs", json!({ "state": "done" }), shop.as_ops())
			.await,
	] {
		assert!(
			!answer.expect(200).text.contains(&password),
			"{}",
			answer.text
		);
	}

	assert_eq!(
		app.find_in_every_table(&password).await,
		Vec::<String>::new()
	);

	// The next deployment keeps the database and its password.
	let (_, release) = ship(&shop, "production", b"second").await;
	assert_eq!(release["input"]["database"]["name"], name.as_str());
	assert_eq!(release["secrets"]["DATABASE_URL"], url);
	let rows = Database::query().all(&system).await.expect("databases");
	assert_eq!(rows.len(), 1, "{}", rows.len());

	// Another environment gets another, with a password of its own.
	staging(&shop).await;
	let (_, release) = ship(&shop, "staging", b"staging").await;
	let other = release["input"]["database"]["name"]
		.as_str()
		.expect("a name");
	assert_ne!(other, name);
	let other_url = release["secrets"]["DATABASE_URL"]
		.as_str()
		.expect("an address");
	assert_ne!(self::password(other_url, other, 5432), password);

	// Someone else's organisation sees none of it.
	let mallory = app.sign_in("mallory@example.com").await;
	app.call(
		"database",
		place(&shop, "production"),
		As::Session(&mallory),
	)
	.await
	.expect(404);
}

/// A sealed password opens only for its own database: copied onto another
/// environment's row, or changed, it is handed to nobody. The release fails
/// as `secret_unavailable` before a worker is given it.
pub async fn a_database_password_opens_nowhere_else(app: &App) {
	let shop = Shop::open(app).await;
	with_postgres(&shop, 5432).await;
	staging(&shop).await;
	let (_, release) = ship(&shop, "production", b"production").await;
	let name = release["input"]["database"]["name"]
		.as_str()
		.expect("a name")
		.to_owned();
	let production_url = release["secrets"]["DATABASE_URL"].clone();
	ship(&shop, "staging", b"staging").await;

	// Someone with the database copies production's sealed password onto
	// staging's row, hoping a staging release will open it for them.
	let system = app.system();
	let rows = Database::query().all(&system).await.expect("databases");
	assert_eq!(rows.len(), 2, "{}", rows.len());
	let production = rows
		.iter()
		.find(|row| row.name == name)
		.expect("production's");
	let staging = rows.iter().find(|row| row.name != name).expect("staging's");
	Database::update()
		.filter(|row| row.id.eq(staging.id))
		.set(
			|row| row.sealed_password,
			production.sealed_password.clone(),
		)
		.run(&system)
		.await
		.expect("copy the sealed password");
	let leases_before = SecretLease::query()
		.all(&system)
		.await
		.expect("leases")
		.len();

	let number = build(&shop, "staging", b"staging-again").await;
	assert_eq!(shop.cluster.claim().await, None);
	assert_eq!(
		shop.state("staging", number).await,
		json!({ "state": "failed", "stage": "rolling_out", "blocker": "secret_unavailable" })
	);
	let leases = SecretLease::query().all(&system).await.expect("leases");
	assert_eq!(leases.len(), leases_before, "{leases:?}");

	// Production's own release still gets production's address.
	let (_, release) = ship(&shop, "production", b"production-again").await;
	assert_eq!(release["secrets"]["DATABASE_URL"], production_url);
}

/// A rollback to a deployment that used the database is given it again,
/// with the door and the address where the cluster's server is now. A
/// cluster can't forget its server while environments keep databases on it.
pub async fn a_rollback_finds_its_database_where_it_is_now(app: &App) {
	let shop = Shop::open(app).await;
	with_postgres(&shop, 5432).await;
	let (first, release) = ship(&shop, "production", b"first").await;
	let name = release["input"]["database"]["name"]
		.as_str()
		.expect("a name")
		.to_owned();
	let was = password(
		release["secrets"]["DATABASE_URL"]
			.as_str()
			.expect("an address"),
		&name,
		5432,
	);
	ship(&shop, "production", b"second").await;

	app.call(
		"set_cluster_facts",
		json!({ "name": "eu-one", "facts": { "wakes_sleeping": true } }),
		shop.as_ops(),
	)
	.await
	.refused(409, "cluster_has_databases");
	// The operator moves the server to another port.
	with_postgres(&shop, 6432).await;

	let mut rollback = place(&shop, "production");
	rollback["to"] = json!(first);
	app.call("rollback", rollback, shop.as_ana())
		.await
		.expect(200);
	let release = shop.cluster.take().await;
	assert_eq!(release["input"]["database"]["name"], name.as_str());
	assert_eq!(
		release["input"]["spec"]["policy"]["database"],
		json!({ "namespace": "leptoskit-postgres", "port": 6432 })
	);
	let now = password(
		release["secrets"]["DATABASE_URL"]
			.as_str()
			.expect("an address"),
		&name,
		6432,
	);
	assert_eq!(now, was);
	shop.cluster.finish(&release, applied()).await.expect(200);

	// A release built before the server moved has the old door in its
	// spec: it is never handed an address the door doesn't open to.
	let stale = build(&shop, "production", b"third").await;
	with_postgres(&shop, 7432).await;
	assert_eq!(shop.cluster.claim().await, None);
	let sheet = shop.on("deployment", "production", stale).await;
	assert_eq!(
		sheet.expect(200).json()["deployment"]["state"],
		json!({ "state": "failed", "stage": "rolling_out", "blocker": "secret_unavailable" }),
		"{}",
		sheet.text
	);
	assert!(
		sheet.text.contains("moved after this release was built"),
		"{}",
		sheet.text
	);
	let (_, release) = ship(&shop, "production", b"fourth").await;
	assert_eq!(release["input"]["spec"]["policy"]["database"]["port"], 7432);
}

/// A reason a worker gives for a failed release is shown on the sheet with
/// any address's password taken out.
pub async fn a_failed_release_does_not_print_the_password(app: &App) {
	let shop = Shop::open(app).await;
	with_postgres(&shop, 5432).await;
	let number = build(&shop, "production", b"first").await;
	let release = shop.cluster.take().await;
	let url = release["secrets"]["DATABASE_URL"]
		.as_str()
		.expect("an address")
		.to_owned();
	let name = release["input"]["database"]["name"]
		.as_str()
		.expect("a name");
	let password = password(&url, name, 5432);
	shop.cluster
		.finish(
			&release,
			json!({
				"kind": "release",
				"result": "not_ready",
				"process": "app",
				"reason": format!("could not connect to {url}: connection refused"),
			}),
		)
		.await
		.expect(200);

	for answer in [
		shop.on("deployment", "production", number).await,
		shop.on("deployment_events", "production", number).await,
	] {
		assert!(
			!answer.expect(200).text.contains(&password),
			"{}",
			answer.text
		);
	}

	assert_eq!(
		app.find_in_every_table(&password).await,
		Vec::<String>::new()
	);
}

/// Deleting an environment tells the cluster's worker which database to
/// drop with it, and the control plane keeps nothing of it.
pub async fn a_deleted_environment_takes_its_database(app: &App) {
	let shop = Shop::open(app).await;
	with_postgres(&shop, 5432).await;
	staging(&shop).await;
	let (_, release) = ship(&shop, "staging", b"short-lived").await;
	let name = release["input"]["database"]["name"].clone();
	let mut delete = place(&shop, "staging");
	delete["confirm"] = json!("staging");
	app.call("delete_environment", delete, shop.as_ana())
		.await
		.expect(200);
	let destroy = shop.cluster.take().await;
	assert_eq!(destroy["input"]["kind"], "destroy_environment");
	assert_eq!(destroy["input"]["database"], name);
	assert!(destroy.get("secrets").is_none(), "{destroy}");
	shop.cluster
		.finish(
			&destroy,
			json!({ "kind": "destroy_environment", "result": "destroyed" }),
		)
		.await
		.expect(200);
	let rows = Database::query()
		.all(&app.system())
		.await
		.expect("databases");
	assert_eq!(rows.len(), 0, "{}", rows.len());

	// With no database left on it, the cluster may forget its server.
	app.call(
		"set_cluster_facts",
		json!({ "name": "eu-one", "facts": { "wakes_sleeping": true } }),
		shop.as_ops(),
	)
	.await
	.expect(200);
}

/// What an operator says of a cluster's Postgres server is checked: it
/// becomes part of every address and every release's network policy.
pub async fn a_clusters_postgres_is_checked(app: &App) {
	let shop = Shop::open(app).await;
	let set = |postgres: Value| {
		app.call(
			"set_cluster_facts",
			json!({ "name": "eu-one", "facts": { "postgres": postgres } }),
			shop.as_ops(),
		)
	};
	let server = |namespace: &str, host: &str, port: u16| json!({ "namespace": namespace, "host": host, "port": port, "tls": false });

	for host in [
		"",
		"Postgres.svc",
		"postgres.svc/other",
		"postgres.svc:5432",
		"user:pw@postgres.svc",
		"postgres.svc?sslmode=disable",
		"-postgres.svc",
		"postgres..svc",
	] {
		set(server("leptoskit-postgres", host, 5432))
			.await
			.refused(400, "invalid_facts");
	}

	set(server("Not A Namespace", HOST, 5432))
		.await
		.refused(400, "invalid_facts");
	set(server("leptoskit-postgres", HOST, 0))
		.await
		.refused(400, "invalid_facts");
	app.call(
		"set_cluster_facts",
		json!({ "name": "nowhere", "facts": {} }),
		shop.as_ops(),
	)
	.await
	.refused(404, "cluster_not_found");
	// Only an operator may.
	app.call(
		"set_cluster_facts",
		json!({ "name": "eu-one", "facts": facts(5432) }),
		shop.as_ana(),
	)
	.await
	.refused(403, "operator_required");

	// Whether the server takes TLS must be said.
	let silent =
		set(json!({ "namespace": "leptoskit-postgres", "host": HOST, "port": 5432 })).await;
	assert_ne!(silent.status, 200, "{}", silent.text);

	let answer = set(server("leptoskit-postgres", HOST, 5432)).await;
	let cluster = answer.expect(200).json();
	assert_eq!(cluster["facts"]["postgres"]["host"], HOST);
	assert_eq!(cluster["facts"]["postgres"]["tls"], false);

	// The audit log says where the server is now, and where it was.
	let rows = AuditEvent::query()
		.filter(|row| row.action.eq("cluster.set_facts"))
		.all(&app.system())
		.await
		.expect("the audit log");
	assert_eq!(rows.len(), 1, "{}", rows.len());
	let target = rows[0].target.clone().unwrap_or_default();
	assert!(
		target.starts_with(&format!(
			"eu-one: now runtime class none, never sleeps, postgres {HOST}:5432 in \
			 leptoskit-postgres, no tls; was runtime class gvisor, wakes sleeping, postgres none"
		)),
		"{target}"
	);
}
