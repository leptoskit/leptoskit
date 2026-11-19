//! Custom domains, written once and run on SQLite (`deployments.rs`) and
//! Postgres (`postgres.rs`), as `shipping.rs`'s scenarios are. DNS is
//! [`super::Dns`]: what the test publishes there is what the control plane
//! reads.

use leptoskit_cloud::domains::MAX_DOMAINS;
use leptoskit_cloud::tables::AuditEvent;
use leptoskit_cloud::tables::Domain;
use serde_json::Value;
use serde_json::json;

use super::Answer;
use super::App;
use super::As;
use super::shipping::Shop;
use super::shipping::applied;
use super::shipping::built;

const HOSTNAME: &str = "notes.example.org";

fn place(organization: &str, project: &str) -> Value {
	json!({ "organization": organization, "project": project, "environment": "production" })
}

fn named(organization: &str, project: &str, hostname: &str) -> Value {
	let mut args = place(organization, project);
	args["hostname"] = json!(hostname);
	args
}

async fn on(shop: &Shop<'_>, operation: &str, hostname: &str) -> Answer {
	shop.app
		.call(
			operation,
			named(&shop.org, "notes", hostname),
			shop.as_ana(),
		)
		.await
}

async fn listed(shop: &Shop<'_>) -> Value {
	shop.app
		.call("domains", place(&shop.org, "notes"), shop.as_ana())
		.await
		.expect(200)
		.json()
}

/// Publishes the challenge record a domain's summary asks for.
fn publish(app: &App, summary: &Value) {
	let record = &summary["records"][0];
	assert_eq!(record["kind"], "TXT", "{summary}");
	app.dns.publish(
		record["name"].as_str().expect("a name"),
		record["value"].as_str().expect("a value"),
	);
}

/// Attaches `hostname` to Ana's production, publishes its record, and
/// verifies it.
async fn verified(shop: &Shop<'_>, hostname: &str) {
	let attached = on(shop, "attach_domain", hostname).await;
	publish(shop.app, &attached.expect(200).json());
	let verified = on(shop, "verify_domain", hostname).await;
	assert_eq!(verified.expect(200).json()["status"], "verified");
}

/// Deploys `bytes` to Ana's production, builds it, and returns its number
/// and the release the cluster's worker claims.
async fn release(shop: &Shop<'_>, bytes: &[u8]) -> (u64, Value) {
	let digest = shop.upload(bytes).await;
	let deployed = shop
		.deploy("production", &digest, json!({ "database": "sqlite" }))
		.await;
	let number = deployed.expect(200).json()["number"]
		.as_u64()
		.expect("a number");
	let job = shop.build.take().await;
	shop.build
		.finish(&job, built(&job, bytes, &[]))
		.await
		.expect(200);
	(number, shop.cluster.take().await)
}

/// The hostnames a release routes, with each one's TLS.
fn routes(release: &Value) -> Vec<(String, String)> {
	let mut routes: Vec<(String, String)> = release["input"]["spec"]["routes"]
		.as_array()
		.expect("routes")
		.iter()
		.map(|route| {
			(
				route["hostname"].as_str().expect("a hostname").to_owned(),
				route["tls"].as_str().expect("tls").to_owned(),
			)
		})
		.collect();
	routes.sort();
	routes.dedup();
	routes
}

/// Another person with an organisation and a project of their own.
async fn rival(app: &App) -> (String, String) {
	let mallory = app.sign_in("mallory@example.com").await;
	let org = app.personal(As::Session(&mallory)).await;
	app.project(As::Session(&mallory), &org, "blog").await;
	(mallory, org)
}

/// A domain routes nothing until DNS proves it: attached, it is pending
/// with a record to publish; verified, the next release routes it with a
/// certificate of its own, beside the hostname the cloud gave.
#[expect(
	clippy::cognitive_complexity,
	reason = "a scenario is one story, read top to bottom; its asserts are what count"
)]
pub async fn a_verified_domain_reaches_the_release(app: &App) {
	let shop = Shop::open(app).await;
	let attached = on(&shop, "attach_domain", HOSTNAME).await;
	let attached = attached.expect(200).json();
	assert_eq!(attached["status"], "pending", "{attached}");
	assert_eq!(
		attached["records"][0]["name"],
		format!("_leptoskit-challenge.{HOSTNAME}")
	);
	let value = attached["records"][0]["value"].as_str().expect("a value");
	assert!(value.starts_with("leptoskit-verify="), "{value}");
	// Nowhere to point it before the environment has been deployed.
	assert_eq!(attached["records"].as_array().map(Vec::len), Some(1));
	// Attaching it again changes nothing: the same record.
	let again = on(&shop, "attach_domain", HOSTNAME).await;
	assert_eq!(again.expect(200).json(), attached);

	// Nothing published, the wrong thing published, and DNS not answering.
	on(&shop, "verify_domain", HOSTNAME)
		.await
		.refused(409, "domain_not_verified");
	app.dns.publish(
		&format!("_leptoskit-challenge.{HOSTNAME}"),
		"leptoskit-verify=someone-elses",
	);
	on(&shop, "verify_domain", HOSTNAME)
		.await
		.refused(409, "domain_not_verified");
	app.dns.down(true);
	on(&shop, "verify_domain", HOSTNAME)
		.await
		.refused(409, "dns_unavailable");
	app.dns.down(false);
	assert!(
		app.dns
			.asked()
			.iter()
			.all(|name| name == &format!("_leptoskit-challenge.{HOSTNAME}")),
		"{:?}",
		app.dns.asked()
	);

	// Pending, it isn't in a release.
	let (_, first) = release(&shop, b"first").await;
	let default = first["input"]["spec"]["routes"][0]["hostname"]
		.as_str()
		.expect("a hostname")
		.to_owned();
	assert_eq!(routes(&first), [(default.clone(), "wildcard".to_owned())]);
	shop.cluster.finish(&first, applied()).await.expect(200);

	// Published, it verifies, and now there is somewhere to point it.
	publish(app, &attached);
	let verified = on(&shop, "verify_domain", HOSTNAME).await;
	let verified = verified.expect(200).json();
	assert_eq!(verified["status"], "verified", "{verified}");
	assert!(verified["verified_at"].is_string(), "{verified}");
	assert_eq!(
		verified["records"][1],
		json!({ "kind": "CNAME", "name": HOSTNAME, "value": default })
	);
	// Verifying again asks nothing.
	let asked = app.dns.asked().len();
	on(&shop, "verify_domain", HOSTNAME).await.expect(200);
	assert_eq!(app.dns.asked().len(), asked);

	// What is running hasn't changed; the next release routes it.
	let (_, second) = release(&shop, b"second").await;
	assert_eq!(
		routes(&second),
		[
			(default, "wildcard".to_owned()),
			(HOSTNAME.to_owned(), "issued".to_owned()),
		]
	);
	shop.cluster.finish(&second, applied()).await.expect(200);

	let audit = AuditEvent::query()
		.all(&app.system())
		.await
		.expect("the audit log");
	for action in ["domain.attach", "domain.verify"] {
		let rows: Vec<_> = audit.iter().filter(|row| row.action == action).collect();
		assert_eq!(rows.len(), 1, "{action}");
		assert_eq!(
			rows[0].target.as_deref(),
			Some(format!("production {HOSTNAME}").as_str())
		);
	}
}

/// Hostnames follow the rules, the cloud's own are refused, an environment
/// has so many, and only whoever manages its configuration changes them.
#[expect(
	clippy::cognitive_complexity,
	reason = "a scenario is one story, read top to bottom; its asserts are what count"
)]
pub async fn domains_are_checked_and_the_tenants_alone(app: &App) {
	let shop = Shop::open(app).await;
	let own_host = app
		.origin
		.trim_start_matches("http://")
		.split(':')
		.next()
		.expect("a host")
		.to_owned();

	for bad in [
		"*.example.org",
		"Notes.example.org",
		"notes.example.org.",
		"notes.example.org:443",
		"localhost",
		"10.0.0.1",
		"",
		&format!("notes.{}", super::APPS_DOMAIN),
		super::APPS_DOMAIN,
		"notes.leptoskit.app",
		&own_host,
		&format!("{}.org", "a".repeat(300)),
	] {
		on(&shop, "attach_domain", bad)
			.await
			.refused(400, "invalid_domain");
	}

	on(&shop, "verify_domain", "never.example.org")
		.await
		.refused(404, "domain_not_found");
	on(&shop, "detach_domain", "never.example.org")
		.await
		.refused(404, "domain_not_found");
	assert!(app.dns.asked().is_empty(), "{:?}", app.dns.asked());

	on(&shop, "attach_domain", HOSTNAME).await.expect(200);

	// A viewer sees them and changes nothing.
	let viewer = app
		.join(shop.as_ana(), &shop.org, "vera@example.com", "viewer")
		.await;
	let viewer = As::Session(&viewer);
	let seen = app.call("domains", place(&shop.org, "notes"), viewer).await;
	assert_eq!(seen.expect(200).json()[0]["hostname"], HOSTNAME);
	for operation in ["attach_domain", "verify_domain", "detach_domain"] {
		app.call(operation, named(&shop.org, "notes", HOSTNAME), viewer)
			.await
			.refused(403, "insufficient_role");
	}

	// Someone outside is told nothing is there, and nobody at all is asked
	// to sign in. Neither causes a lookup.
	let (mallory, _) = rival(app).await;
	for operation in ["attach_domain", "verify_domain", "detach_domain"] {
		app.call(
			operation,
			named(&shop.org, "notes", HOSTNAME),
			As::Session(&mallory),
		)
		.await
		.refused(404, "organization_not_found");
		app.call(operation, named(&shop.org, "notes", HOSTNAME), As::Nobody)
			.await
			.expect(401);
	}
	app.call("domains", place(&shop.org, "notes"), As::Session(&mallory))
		.await
		.refused(404, "organization_not_found");

	// Tokens: reading with any scope, changing with `manage` alone.
	let (read, _) = app.token(shop.as_ana(), &shop.org, &["read"], None).await;
	let (manage, _) = app.token(shop.as_ana(), &shop.org, &["manage"], None).await;
	app.call("domains", place(&shop.org, "notes"), As::Bearer(&read))
		.await
		.expect(200);
	for operation in ["attach_domain", "verify_domain", "detach_domain"] {
		app.call(
			operation,
			named(&shop.org, "notes", HOSTNAME),
			As::Bearer(&read),
		)
		.await
		.refused(403, "insufficient_scope");
	}
	assert!(app.dns.asked().is_empty(), "{:?}", app.dns.asked());
	app.call(
		"attach_domain",
		named(&shop.org, "notes", "token.example.org"),
		As::Bearer(&manage),
	)
	.await
	.expect(200);

	// So many, and no more.
	for index in 2..MAX_DOMAINS {
		on(&shop, "attach_domain", &format!("n{index}.example.org"))
			.await
			.expect(200);
	}
	on(&shop, "attach_domain", "one-too-many.example.org")
		.await
		.refused(409, "too_many_domains");
	// One it already has is still answered.
	on(&shop, "attach_domain", HOSTNAME).await.expect(200);

	// A pending domain is simply gone when detached.
	on(&shop, "detach_domain", HOSTNAME).await.expect(200);
	let left = listed(&shop).await;
	assert_eq!(left.as_array().map(Vec::len), Some(MAX_DOMAINS - 1));
}

/// A hostname is one environment's. Anyone may attach it, each with a
/// challenge of their own; only whoever publishes theirs verifies it, and
/// once it is verified nobody else can, whatever DNS says.
pub async fn a_hostname_is_one_environments(app: &App) {
	let shop = Shop::open(app).await;
	let (mallory, rival_org) = rival(app).await;
	let mallory = As::Session(&mallory);

	let ours = on(&shop, "attach_domain", HOSTNAME).await;
	let ours = ours.expect(200).json();
	// Attaching a hostname someone else has attached says nothing of them.
	let theirs = app
		.call(
			"attach_domain",
			named(&rival_org, "blog", HOSTNAME),
			mallory,
		)
		.await;
	let theirs = theirs.expect(200).json();
	assert_eq!(theirs["status"], "pending");
	assert_ne!(theirs["records"][0]["value"], ours["records"][0]["value"]);

	// Ana's record is in DNS. It proves the domain for Ana, not Mallory.
	publish(app, &ours);
	app.call(
		"verify_domain",
		named(&rival_org, "blog", HOSTNAME),
		mallory,
	)
	.await
	.refused(409, "domain_not_verified");
	on(&shop, "verify_domain", HOSTNAME).await.expect(200);

	// Even with a record of her own published, the hostname is held.
	publish(app, &theirs);
	app.call(
		"verify_domain",
		named(&rival_org, "blog", HOSTNAME),
		mallory,
	)
	.await
	.refused(409, "domain_taken");
	let theirs = app
		.call("domains", place(&rival_org, "blog"), mallory)
		.await;
	assert_eq!(theirs.expect(200).json()[0]["status"], "pending");

	// Ana's other environment can't hold it either.
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
	let mut staging = named(&shop.org, "notes", HOSTNAME);
	staging["environment"] = json!("staging");
	let attached = app
		.call("attach_domain", staging.clone(), shop.as_ana())
		.await;
	publish(app, &attached.expect(200).json());
	app.call("verify_domain", staging, shop.as_ana())
		.await
		.refused(409, "domain_taken");

	let held = Domain::query()
		.filter(|row| row.claim.is_not_null())
		.all(&app.system())
		.await
		.expect("the domains");
	assert_eq!(held.len(), 1, "{held:?}");
}

/// Two environments that both publish their record and verify at the same
/// moment: one holds the hostname, and the other is told it is taken.
pub async fn verifying_races_to_one_holder(app: &App) {
	let shop = Shop::open(app).await;
	let (mallory, rival_org) = rival(app).await;
	let mallory = As::Session(&mallory);
	let ours = on(&shop, "attach_domain", HOSTNAME).await;
	publish(app, &ours.expect(200).json());
	let theirs = app
		.call(
			"attach_domain",
			named(&rival_org, "blog", HOSTNAME),
			mallory,
		)
		.await;
	publish(app, &theirs.expect(200).json());

	let (ours, theirs) = tokio::join!(
		on(&shop, "verify_domain", HOSTNAME),
		app.call(
			"verify_domain",
			named(&rival_org, "blog", HOSTNAME),
			mallory
		),
	);
	let won = [&ours, &theirs]
		.iter()
		.filter(|answer| answer.status == 200)
		.count();
	assert_eq!(won, 1, "{} / {}", ours.text, theirs.text);
	for answer in [&ours, &theirs] {
		if answer.status != 200 {
			answer.refused(409, "domain_taken");
		}
	}

	let held = Domain::query()
		.filter(|row| row.claim.is_not_null())
		.all(&app.system())
		.await
		.expect("the domains");
	assert_eq!(held.len(), 1, "{held:?}");
}

/// A detached domain is still served by what is running, so the hostname
/// stays its environment's until a deployment without it is live. Only
/// then can someone else verify it, and no later rollback routes it again.
#[expect(
	clippy::cognitive_complexity,
	reason = "a scenario is one story, read top to bottom; its asserts are what count"
)]
pub async fn a_detached_domain_is_held_until_it_stops_serving(app: &App) {
	let shop = Shop::open(app).await;
	let (mallory, rival_org) = rival(app).await;
	let mallory = As::Session(&mallory);

	// Verified but never deployed: detached, it is gone at once.
	verified(&shop, "early.example.org").await;
	on(&shop, "detach_domain", "early.example.org")
		.await
		.expect(200);
	assert_eq!(listed(&shop).await, json!([]));

	verified(&shop, HOSTNAME).await;
	let (first, serving) = release(&shop, b"first").await;
	assert!(
		routes(&serving).iter().any(|(host, _)| host == HOSTNAME),
		"{serving}"
	);
	shop.cluster.finish(&serving, applied()).await.expect(200);

	// Detached while it serves: removing, and still held.
	on(&shop, "detach_domain", HOSTNAME).await.expect(200);
	assert_eq!(listed(&shop).await[0]["status"], "removing");
	on(&shop, "detach_domain", HOSTNAME).await.expect(200);
	let theirs = app
		.call(
			"attach_domain",
			named(&rival_org, "blog", HOSTNAME),
			mallory,
		)
		.await;
	publish(app, &theirs.expect(200).json());
	app.call(
		"verify_domain",
		named(&rival_org, "blog", HOSTNAME),
		mallory,
	)
	.await
	.refused(409, "domain_taken");

	// Attached again before it went, it is simply verified again.
	let back = on(&shop, "attach_domain", HOSTNAME).await;
	assert_eq!(back.expect(200).json()["status"], "verified");
	on(&shop, "detach_domain", HOSTNAME).await.expect(200);

	// The next release doesn't route it, and once that is live it is gone.
	let (_, without) = release(&shop, b"second").await;
	assert!(
		routes(&without).iter().all(|(host, _)| host != HOSTNAME),
		"{without}"
	);
	assert_eq!(listed(&shop).await[0]["status"], "removing");
	shop.cluster.finish(&without, applied()).await.expect(200);
	assert_eq!(listed(&shop).await, json!([]));

	// Now it is Mallory's to verify.
	let theirs = app
		.call(
			"verify_domain",
			named(&rival_org, "blog", HOSTNAME),
			mallory,
		)
		.await;
	assert_eq!(theirs.expect(200).json()["status"], "verified");

	// A rollback to the release that routed it leaves it out: it isn't
	// this environment's any more.
	app.call(
		"rollback",
		json!({
			"organization": shop.org,
			"project": "notes",
			"environment": "production",
			"to": first,
		}),
		shop.as_ana(),
	)
	.await
	.expect(200);
	let rolled = shop.cluster.take().await;
	assert!(
		routes(&rolled).iter().all(|(host, _)| host != HOSTNAME),
		"{rolled}"
	);
	assert_eq!(routes(&rolled).len(), 1, "{rolled}");
	shop.cluster.finish(&rolled, applied()).await.expect(200);
}

/// The cluster may answer on a hostname that no live deployment routes: a
/// release that failed part-way put it there, or its environment was
/// deleted and the cluster hasn't removed it yet. Either way the hostname
/// stays held until the cluster is known to be without it.
#[expect(
	clippy::cognitive_complexity,
	reason = "a scenario is one story, read top to bottom; its asserts are what count"
)]
pub async fn a_hostname_is_held_while_the_cluster_may_route_it(app: &App) {
	let shop = Shop::open(app).await;
	let (mallory, rival_org) = rival(app).await;
	let mallory = As::Session(&mallory);
	let theirs = |hostname: &str| named(&rival_org, "blog", hostname);

	// Live without the domain, then a release with it that never became
	// ready: the cluster has its route, and the live deployment doesn't.
	let (_, first) = release(&shop, b"first").await;
	shop.cluster.finish(&first, applied()).await.expect(200);
	verified(&shop, HOSTNAME).await;
	let (failed, second) = release(&shop, b"second").await;
	assert!(
		routes(&second).iter().any(|(host, _)| host == HOSTNAME),
		"{second}"
	);
	shop.cluster
		.finish(
			&second,
			json!({
				"kind": "release",
				"result": "not_ready",
				"process": "app",
				"reason": "it never answered",
			}),
		)
		.await
		.expect(200);
	assert_eq!(
		shop.state("production", failed).await["state"],
		"failed",
		"the release failed"
	);

	// Detached now, it is still held.
	on(&shop, "detach_domain", HOSTNAME).await.expect(200);
	assert_eq!(listed(&shop).await[0]["status"], "removing");
	let attached = app.call("attach_domain", theirs(HOSTNAME), mallory).await;
	publish(app, &attached.expect(200).json());
	app.call("verify_domain", theirs(HOSTNAME), mallory)
		.await
		.refused(409, "domain_taken");

	// A release without it, applied, takes it off the cluster.
	let (_, third) = release(&shop, b"third").await;
	assert!(
		routes(&third).iter().all(|(host, _)| host != HOSTNAME),
		"{third}"
	);
	shop.cluster.finish(&third, applied()).await.expect(200);
	assert_eq!(listed(&shop).await, json!([]));
	app.call("verify_domain", theirs(HOSTNAME), mallory)
		.await
		.expect(200);

	// A deleted environment's hostname is held until its cluster's worker
	// has removed it.
	let other = "staging.example.org";
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
	let mut staging = named(&shop.org, "notes", other);
	staging["environment"] = json!("staging");
	let attached = app
		.call("attach_domain", staging.clone(), shop.as_ana())
		.await;
	publish(app, &attached.expect(200).json());
	app.call("verify_domain", staging, shop.as_ana())
		.await
		.expect(200);
	shop.ship("staging", b"short-lived", &[]).await;
	app.call(
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
	let attached = app.call("attach_domain", theirs(other), mallory).await;
	publish(app, &attached.expect(200).json());
	app.call("verify_domain", theirs(other), mallory)
		.await
		.refused(409, "domain_taken");

	let destroy = shop.cluster.take().await;
	assert_eq!(destroy["input"]["kind"], "destroy_environment");
	shop.cluster
		.finish(
			&destroy,
			json!({ "kind": "destroy_environment", "result": "destroyed" }),
		)
		.await
		.expect(200);
	app.call("verify_domain", theirs(other), mallory)
		.await
		.expect(200);
	let rows = Domain::query().all(&app.system()).await.expect("domains");
	assert_eq!(rows.len(), 2, "{rows:?}");
}
