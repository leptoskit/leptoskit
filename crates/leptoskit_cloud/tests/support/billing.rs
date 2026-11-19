//! Usage, the spend cap, and plans' limits, written once and run on SQLite
//! (`deployments.rs`) and Postgres (`postgres.rs`). Every price here is
//! made up for the tests; the crate holds none.

use leptoskit_cloud::pricing::Catalogue;
use leptoskit_cloud::tables::AuditEvent;
use leptoskit_cloud::tables::Job;
use leptoskit_cloud::tables::Organization;
use leptoskit_cloud::tables::StatusReport;
use leptoskit_cloud::tables::UsageTotal;
use serde_json::Value;
use serde_json::json;

use super::App;
use super::As;
use super::shipping::Shop;
use super::shipping::applied;
use super::shipping::built;

/// A catalogue for the tests: ten minor units a build minute, one for a
/// hundred seconds of the smallest size. Hobby is free, with two build
/// minutes, one project, no custom domains, and two organisations a person. Pro
/// includes twenty minor units of usage and caps what comes after at fifty.
pub fn catalogue() -> Catalogue {
	leptoskit_cloud::server::catalogue_from_json(
		r#"{
			"currency": "usd",
			"plans": [
				{
					"id": "hobby", "name": "Hobby", "monthly": 0,
					"allowances": { "build_minutes_linux": 2 },
					"limits": { "organizations": 2, "projects": 1, "custom_domains": 0 }
				},
				{
					"id": "pro", "name": "Pro", "monthly": 2000, "included_usage": 20,
					"limits": { "custom_domains": 2 },
					"spend_cap": 50
				}
			],
			"meters": {
				"build_minutes_linux": { "minor": 10 },
				"compute_seconds_xs": { "minor": 1, "per": 100 }
			}
		}"#,
	)
	.expect("a catalogue")
}

/// An estimate prices a whole `[cloud]` table before anything is
/// deployed: on the organisation's plan or another, for members only.
pub async fn an_estimate_prices_a_table(app: &App) {
	let ana = app.sign_in("ana@example.com").await;
	let org = app.personal(As::Session(&ana)).await;
	let estimate = |config: Value, plan: Value, caller| {
		app.call(
			"estimate",
			json!({ "organization": org, "config": config, "plan": plan }),
			caller,
		)
	};

	// An empty table is production alone, with the defaults.
	let free = estimate(json!({}), Value::Null, As::Session(&ana)).await;
	let free = free.expect(200).json();
	assert_eq!(
		(&free["plan"], &free["plan_price"], &free["currency"]),
		(&json!("hobby"), &json!(0), &json!("usd"))
	);
	let lines = free["lines"].as_array().expect("lines");
	assert!(
		lines
			.iter()
			.any(|line| line["meter"] == "build_minutes_linux"),
		"{free}"
	);
	assert!(
		free["assumptions"]
			.as_array()
			.is_some_and(|said| !said.is_empty()),
		"{free}"
	);
	// The same table on a plan that is paid for: the plan's price, and
	// usage beyond what it includes.
	let paid = estimate(json!({}), json!("pro"), As::Session(&ana)).await;
	let paid = paid.expect(200).json();
	assert_eq!(
		(&paid["plan"], &paid["plan_price"]),
		(&json!("pro"), &json!(2000))
	);
	let number = |value: &Value| value.as_u64().expect("a number");
	assert_eq!(
		number(&paid["total"]),
		2000 + number(&paid["usage"]).saturating_sub(number(&paid["included"]))
	);
	// More environments cost more.
	let two = json!({ "cloud": { "environments": { "staging": {} } } });
	let both = estimate(two, json!("pro"), As::Session(&ana)).await;
	let both = both.expect(200).json();
	assert!(number(&both["usage"]) > number(&paid["usage"]), "{both}");

	estimate(json!({}), json!("enterprise"), As::Session(&ana))
		.await
		.refused(404, "plan_not_found");
	let wrong = json!({ "cloud": { "environments": { "Not A Slug": {} } } });
	let refused = estimate(wrong, Value::Null, As::Session(&ana)).await;
	assert_eq!(refused.status, 400, "{}", refused.text);
	// A deploy token may ask; a stranger learns nothing of the organisation.
	let (token, _) = app.token(As::Session(&ana), &org, &["deploy"], None).await;
	estimate(json!({}), Value::Null, As::Bearer(&token))
		.await
		.expect(200);
	let mallory = app.sign_in("mallory@example.com").await;
	estimate(json!({}), Value::Null, As::Session(&mallory))
		.await
		.refused(404, "organization_not_found");
	estimate(json!({}), Value::Null, As::Nobody)
		.await
		.expect(401);
}

async fn usage(shop: &Shop<'_>) -> Value {
	shop.app
		.call("usage", json!({ "organization": shop.org }), shop.as_ana())
		.await
		.expect(200)
		.json()
}

async fn put_on(shop: &Shop<'_>, plan: &str) {
	let slug = shop.org.clone();
	Organization::update()
		.filter(|organization| organization.slug.eq(slug.as_str()))
		.set(|organization| organization.plan, plan.to_owned())
		.run(&shop.app.system())
		.await
		.expect("change the plan");
}

/// Deploys `bytes` and has a build worker hold the build for `seconds`
/// before finishing it; the release is applied.
async fn ship_after(shop: &Shop<'_>, bytes: &[u8], seconds: i64) {
	let digest = shop.upload(bytes).await;
	shop.deploy("production", &digest, json!({ "database": "sqlite" }))
		.await
		.expect(200);
	let job = shop.build.take().await;
	Job::update()
		.filter(|job| job.claimed_at.is_not_null())
		.set(
			|job| job.claimed_at,
			Some(leptoskit_db::now() - chrono::TimeDelta::seconds(seconds)),
		)
		.run(&shop.app.system())
		.await
		.expect("age the claim");
	shop.build
		.finish(&job, built(&job, bytes, &[]))
		.await
		.expect(200);
	let release = shop.cluster.take().await;
	shop.cluster.finish(&release, applied()).await.expect(200);
}

fn quantity(report: &Value, meter: &str) -> u64 {
	report["quantities"]
		.as_array()
		.expect("quantities")
		.iter()
		.find(|line| line["meter"] == meter)
		.map_or(0, |line| line["quantity"].as_u64().expect("a quantity"))
}

/// Has the cluster's worker report `replicas` ready replicas of the app,
/// `seconds` after its usage was last counted. The report names the app
/// twice, which counts it no more than the release may run.
async fn report(shop: &Shop<'_>, environment: &str, replicas: u32, seconds: i64) {
	StatusReport::update()
		.filter(|row| row.id.is_not_null())
		.set(
			|row| row.metered_at,
			Some(leptoskit_db::now() - chrono::TimeDelta::seconds(seconds)),
		)
		.run(&shop.app.system())
		.await
		.expect("age the report");
	let observed = json!({
		"environment": {
			"organization": { "slug": shop.org, "id": uuid::Uuid::now_v7() },
			"project": { "slug": "notes", "id": uuid::Uuid::now_v7() },
			"environment": { "slug": "production", "id": environment },
		},
		"processes": [
			{
				"name": "app",
				"ready_replicas": replicas,
				"desired_replicas": replicas,
				"image": "registry.example.test/app@sha256:aa",
				"state": "Ready",
			},
			{
				"name": "app",
				"ready_replicas": replicas,
				"desired_replicas": replicas,
				"image": "registry.example.test/app@sha256:aa",
				"state": "Ready",
			},
			{
				"name": "not-in-the-release",
				"ready_replicas": 50,
				"desired_replicas": 50,
				"image": "registry.example.test/app@sha256:aa",
				"state": "Ready",
			},
		],
	});
	shop.cluster
		.call(
			"report_status",
			json!({ "environment": environment, "observed": observed, "logs": [] }),
		)
		.await
		.expect(200);
}

/// Build minutes and compute seconds are counted as they happen, priced
/// on the organisation's plan, and stop new deployments at the spend cap,
/// which a free plan keeps at zero and a paid plan's owners can move.
#[expect(
	clippy::cognitive_complexity,
	clippy::too_many_lines,
	reason = "a scenario is one story, read top to bottom; its asserts are what count"
)]
pub async fn usage_is_counted_priced_and_capped(app: &App) {
	let shop = Shop::open(app).await;
	let plans = app.call("plans", json!({}), shop.as_ana()).await;
	let plans = plans.expect(200).json();
	assert_eq!(plans["currency"], "usd");
	assert_eq!(plans["plans"][0]["id"], "hobby");
	// A free plan's cap is zero, whatever the catalogue says.
	assert_eq!(plans["plans"][0]["spend_cap"], 0);
	assert_eq!(plans["plans"][0]["limits"]["projects"], 1);
	assert_eq!(plans["plans"][1]["spend_cap"], 50);
	assert_eq!(
		plans["prices"][0],
		json!({ "meter": "compute_seconds_xs", "minor": 1, "per": 100 })
	);

	let empty = usage(&shop).await;
	assert_eq!(empty["plan"], "hobby");
	assert_eq!(empty["quantities"], json!([]));
	assert_eq!(empty["cost"]["kind"], "bill");
	assert_eq!(empty["cost"]["total"], 0);
	assert_eq!(empty["cost"]["cap_reached"], false);

	// Two builds: under a minute counts as one, and 61 seconds as two.
	ship_after(&shop, b"first", 5).await;
	let after_one = usage(&shop).await;
	assert_eq!(quantity(&after_one, "build_minutes_linux"), 1);
	assert_eq!(after_one["projects"][0]["slug"], "notes");
	assert_eq!(
		after_one["projects"][0]["quantities"],
		json!([{ "meter": "build_minutes_linux", "quantity": 1 }])
	);
	ship_after(&shop, b"second", 61).await;
	let after_two = usage(&shop).await;
	assert_eq!(quantity(&after_two, "build_minutes_linux"), 3);
	// Hobby includes two minutes; the third costs ten, past a cap of zero.
	let cost = &after_two["cost"];
	assert_eq!(cost["lines"][0]["allowance"], 2);
	assert_eq!(cost["lines"][0]["billable"], 1);
	assert_eq!(
		(&cost["usage"], &cost["over"], &cost["spend_cap"]),
		(&json!(10), &json!(10), &json!(0))
	);
	assert_eq!(cost["cap_reached"], true);
	let digest = shop.upload(b"third").await;
	let refused = shop
		.deploy("production", &digest, json!({ "database": "sqlite" }))
		.await;
	refused.refused(403, "spend_cap_reached");
	// Nothing was made, and nothing that runs was touched.
	assert_eq!(
		shop.state("production", 2).await,
		json!({ "state": "live" })
	);
	shop.on("deployment", "production", 3).await.expect(404);
	// Nothing pays for more on a free plan.
	let cap = |cap: Value, caller| {
		app.call(
			"set_spend_cap",
			json!({ "organization": shop.org, "cap": cap }),
			caller,
		)
	};
	cap(json!(100), shop.as_ana())
		.await
		.refused(403, "payment_required");

	// On a paid plan the same usage is inside what the plan includes.
	put_on(&shop, "pro").await;
	let on_pro = usage(&shop).await;
	assert_eq!(on_pro["plan"], "pro");
	let cost = &on_pro["cost"];
	assert_eq!(
		(
			&cost["usage"],
			&cost["included"],
			&cost["over"],
			&cost["total"]
		),
		(&json!(30), &json!(20), &json!(10), &json!(2010))
	);
	assert_eq!(
		(
			&cost["spend_cap"],
			&cost["own_spend_cap"],
			&cost["cap_reached"]
		),
		(&json!(50), &json!(false), &json!(false))
	);
	ship_after(&shop, b"third", 1).await;

	// Compute: nothing for a first report, then ready replicas times the
	// seconds since the last one, for processes the release has.
	let environment = app
		.call(
			"environments",
			json!({ "organization": shop.org, "project": "notes" }),
			shop.as_ana(),
		)
		.await
		.json()[0]["id"]
		.as_str()
		.expect("an id")
		.to_owned();
	report(&shop, &environment, 1, 0).await;
	assert_eq!(quantity(&usage(&shop).await, "compute_seconds_xs"), 0);
	report(&shop, &environment, 1, 100).await;
	let counted = quantity(&usage(&shop).await, "compute_seconds_xs");
	assert!((100..=105).contains(&counted), "{counted}");
	// A worker that was away for a day is counted for five minutes, and
	// never for more replicas than the release may run.
	report(&shop, &environment, 40, 24 * 60 * 60).await;
	let capped = quantity(&usage(&shop).await, "compute_seconds_xs");
	assert_eq!(capped, counted + 300);

	// Four builds and a few minutes of compute: 40 + 4, 24 past what Pro
	// includes. The plan's cap of 50 isn't reached; one of 20 is.
	let before = usage(&shop).await;
	assert_eq!(before["cost"]["over"], 24, "{before}");
	let dev = app
		.join(shop.as_ana(), &shop.org, "dev@example.com", "developer")
		.await;
	let billing = app
		.join(shop.as_ana(), &shop.org, "books@example.com", "billing")
		.await;
	cap(json!(20), As::Session(&dev))
		.await
		.refused(403, "insufficient_role");
	app.call(
		"usage",
		json!({ "organization": shop.org }),
		As::Session(&dev),
	)
	.await
	.refused(403, "insufficient_role");
	// Nobody raises their own cap past the plan's: nothing invoices it.
	cap(json!(51), As::Session(&billing))
		.await
		.refused(403, "cap_above_plan");
	let set = cap(json!(20), As::Session(&billing)).await;
	assert_eq!(set.expect(200).json(), json!({ "cap": 20, "own": true }));
	let capped = usage(&shop).await;
	assert_eq!(capped["cost"]["own_spend_cap"], true);
	assert_eq!(capped["cost"]["cap_reached"], true);
	let digest = shop.upload(b"fourth").await;
	let refused = shop
		.deploy("production", &digest, json!({ "database": "sqlite" }))
		.await;
	refused.refused(403, "spend_cap_reached");
	assert!(refused.text.contains("set_spend_cap"), "{}", refused.text);
	// The cap itself is for those who may see the bill.
	assert!(!refused.text.contains("minor units"), "{}", refused.text);
	// Back to the plan's own cap, and it deploys.
	let unset = cap(Value::Null, shop.as_ana()).await;
	assert_eq!(unset.expect(200).json(), json!({ "cap": 50, "own": false }));
	shop.deploy("production", &digest, json!({ "database": "sqlite" }))
		.await
		.expect(200);
	let audited = AuditEvent::query()
		.filter(|event| event.action.eq("billing.set_spend_cap"))
		.all(&app.system())
		.await
		.expect("audit");
	assert_eq!(audited.len(), 2);

	// Another month is another count; a month is written one way.
	let past = app
		.call(
			"usage",
			json!({ "organization": shop.org, "period": "2020-01" }),
			shop.as_ana(),
		)
		.await;
	assert_eq!(past.expect(200).json()["quantities"], json!([]));
	app.call(
		"usage",
		json!({ "organization": shop.org, "period": "last month" }),
		shop.as_ana(),
	)
	.await
	.refused(400, "invalid_period");
	// Nobody outside sees any of it.
	let stranger = app.sign_in("mallory@example.com").await;
	let answer = app
		.call(
			"usage",
			json!({ "organization": shop.org }),
			As::Session(&stranger),
		)
		.await;
	assert_eq!(answer.status, 404, "{}", answer.text);
	// One row per project, month and meter, however many times it grew.
	let rows = UsageTotal::query().all(&app.system()).await.expect("usage");
	assert_eq!(rows.len(), 2, "{}", rows.len());
}

/// A plan's limits are counted when something is made: projects, and
/// custom domains over every environment.
pub async fn a_plans_limits_are_counted(app: &App) {
	let shop = Shop::open(app).await;
	let create = |slug: &str| {
		app.call(
			"create_project",
			json!({ "organization": shop.org, "slug": slug, "name": slug }),
			shop.as_ana(),
		)
	};
	let attach = |project: &str, hostname: &str| {
		app.call(
			"attach_domain",
			json!({
				"organization": shop.org,
				"project": project,
				"environment": "production",
				"hostname": hostname,
			}),
			shop.as_ana(),
		)
	};
	// Hobby: one person makes two organisations on it, their own included.
	let found = |slug: &str| {
		app.call(
			"create_organization",
			json!({ "slug": slug, "name": slug }),
			shop.as_ana(),
		)
	};
	found("ana-and-co").await.expect(200);
	found("ana-and-more").await.refused(403, "quota_exceeded");
	// Hobby: the one project `Shop` made, and no custom domain.
	create("second").await.refused(403, "quota_exceeded");
	let refused = attach("notes", "www.notes.example").await;
	refused.refused(403, "quota_exceeded");
	assert!(refused.text.contains("aren't part of"), "{}", refused.text);

	// Pro: any number of projects, and two domains between them.
	put_on(&shop, "pro").await;
	create("second").await.expect(200);
	attach("notes", "www.notes.example").await.expect(200);
	// Attaching one again is the same domain, not another.
	attach("notes", "www.notes.example").await.expect(200);
	attach("second", "www.second.example").await.expect(200);
	attach("second", "api.second.example")
		.await
		.refused(403, "quota_exceeded");
}

/// A build attempt whose worker went quiet is counted up to the end of the
/// lease it held: a build that never ends isn't free.
pub async fn a_build_whose_lease_ran_out_is_counted(app: &App) {
	let shop = Shop::open(app).await;
	let digest = shop.upload(b"never ends").await;
	shop.deploy("production", &digest, json!({ "database": "sqlite" }))
		.await
		.expect(200);
	let job = shop.build.take().await;
	Job::update()
		.filter(|job| job.claimed_at.is_not_null())
		.set(
			|job| job.claimed_at,
			Some(leptoskit_db::now() - chrono::TimeDelta::seconds(9 * 60 + 30)),
		)
		.run(&shop.app.system())
		.await
		.expect("age the claim");
	assert_eq!(quantity(&usage(&shop).await, "build_minutes_linux"), 0);
	shop.lapse_leases().await;
	shop.sweep().await;
	// Nine and a half minutes, less the second the lease ended before now.
	assert_eq!(quantity(&usage(&shop).await, "build_minutes_linux"), 10);
	// Its late answer isn't counted again, and neither is another sweep.
	shop.build
		.finish(&job, built(&job, b"never ends", &[]))
		.await
		.refused(404, "job_not_held");
	shop.sweep().await;
	assert_eq!(quantity(&usage(&shop).await, "build_minutes_linux"), 10);
	// Ten minutes on a plan that includes two: the cap is reached.
	assert_eq!(usage(&shop).await["cost"]["cap_reached"], true);
}
