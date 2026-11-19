//! Backups of managed databases and restoring them, written once and run
//! on SQLite (`deployments.rs`) and Postgres (`postgres.rs`), as
//! `shipping.rs`'s scenarios are.

use leptoskit_cloud::tables::AuditEvent;
use leptoskit_cloud::tables::DatabaseBackup;
use leptoskit_cloud::tables::Job;
use leptoskit_cloud::tables::Placement;
use leptoskit_db::Json;
use serde_json::Value;
use serde_json::json;

use super::App;
use super::As;
use super::databases::place;
use super::databases::staging;
use super::databases::with_postgres;
use super::shipping::Shop;
use super::shipping::applied;
use super::shipping::built;

/// Where the worker in these tests says it keeps a dump.
fn location(name: &str) -> String {
	format!("s3://lk-dumps-eu-one/{name}.dump")
}

/// Builds and releases `bytes`, a Postgres app with `migrations`, to
/// `environment`.
async fn ship(shop: &Shop<'_>, environment: &str, bytes: &[u8], migrations: &[&str]) {
	let digest = shop.upload(bytes).await;
	shop.deploy(environment, &digest, json!({ "database": "postgres" }))
		.await
		.expect(200);
	let job = shop.build.take().await;
	shop.build
		.finish(&job, built(&job, bytes, migrations))
		.await
		.expect(200);
	let release = shop.cluster.take().await;
	shop.cluster.finish(&release, applied()).await.expect(200);
}

async fn list(shop: &Shop<'_>, environment: &str) -> Value {
	shop.app
		.call("database_backups", place(shop, environment), shop.as_ana())
		.await
		.expect(200)
		.json()
}

fn taken(name: &str) -> Value {
	json!({
		"kind": "backup_database",
		"result": "taken",
		"location": location(name),
		"bytes": 4096,
	})
}

/// Asks for a backup of `environment` and has the cluster's worker take
/// it, keeping it as `name`. Returns the backup's ID.
async fn back_up(shop: &Shop<'_>, environment: &str, name: &str) -> Value {
	let asked = shop
		.app
		.call("create_backup", place(shop, environment), shop.as_ana())
		.await;
	let id = asked.expect(200).json()["id"].clone();
	let job = shop.cluster.take().await;
	assert_eq!(job["input"]["backup"], id);
	shop.cluster.finish(&job, taken(name)).await.expect(200);
	id
}

fn restore_args(shop: &Shop<'_>, environment: &str, backup: &Value) -> Value {
	let mut args = place(shop, environment);
	args["backup"] = backup.clone();
	args["confirm"] = json!(format!("notes/{environment}"));
	args
}

/// A backup is asked for, taken by the cluster's worker, and listed: by
/// its size and dates, never by where it is kept. One runs at a time, an
/// environment keeps ten, and a backup's row goes when it expires.
#[expect(
	clippy::cognitive_complexity,
	clippy::too_many_lines,
	reason = "a scenario is one story, read top to bottom; its asserts are what count"
)]
pub async fn a_backup_is_taken_and_listed(app: &App) {
	let shop = Shop::open(app).await;
	with_postgres(&shop, 5432).await;
	staging(&shop).await;

	// Nothing to back up: no database, and then one no release has made.
	app.call("create_backup", place(&shop, "staging"), shop.as_ana())
		.await
		.refused(409, "no_database");
	let digest = shop.upload(b"first").await;
	shop.deploy("production", &digest, json!({ "database": "postgres" }))
		.await
		.expect(200);
	app.call("create_backup", place(&shop, "production"), shop.as_ana())
		.await
		.refused(409, "database_not_ready");
	let job = shop.build.take().await;
	shop.build
		.finish(&job, built(&job, b"first", &["a"]))
		.await
		.expect(200);
	let release = shop.cluster.take().await;
	let database = release["input"]["database"]["name"].clone();
	shop.cluster.finish(&release, applied()).await.expect(200);
	assert_eq!(
		list(&shop, "production").await,
		json!({ "backups": [], "restore": null })
	);

	let asked = app
		.call("create_backup", place(&shop, "production"), shop.as_ana())
		.await;
	let backup = asked.expect(200).json();
	assert_eq!(backup["state"], "pending");
	assert_eq!(backup["kind"], "manual");
	assert_eq!(backup["bytes"], Value::Null);
	// One at a time.
	app.call("create_backup", place(&shop, "production"), shop.as_ana())
		.await
		.refused(409, "backup_in_progress");

	// The worker is told which database, and is given no secret: it dumps
	// with its own connection to its cluster's server.
	let job = shop.cluster.take().await;
	assert_eq!(job["input"]["kind"], "backup_database");
	assert_eq!(job["input"]["database"], database);
	assert_eq!(job["input"]["backup"], backup["id"]);
	assert_eq!(job["input"]["expires_at"], backup["expires_at"]);
	// And how large a dump may be.
	let cap: u64 = 20 * 1024 * 1024 * 1024;
	assert_eq!(job["input"]["max_bytes"], cap);
	assert_eq!(
		job["input"]["environment"]["environment"]["slug"],
		"production"
	);
	assert!(job.get("secrets").is_none(), "{job}");
	shop.cluster.finish(&job, taken("one")).await.expect(200);

	let listed = app
		.call(
			"database_backups",
			place(&shop, "production"),
			shop.as_ana(),
		)
		.await;
	let backups = listed.expect(200).json();
	assert_eq!(backups["backups"].as_array().map(Vec::len), Some(1));
	assert_eq!(backups["backups"][0]["id"], backup["id"]);
	assert_eq!(backups["backups"][0]["state"], "complete");
	assert_eq!(backups["backups"][0]["bytes"], 4096);
	assert!(
		backups["backups"][0]["completed_at"].is_string(),
		"{backups}"
	);
	// Where it is kept is the worker's to know.
	assert!(!listed.text.contains("lk-dumps"), "{}", listed.text);
	let row = DatabaseBackup::query()
		.one(&app.system())
		.await
		.expect("the backup");
	assert_eq!(row.location, Some(location("one")));
	assert_eq!(
		row.migrations
			.0
			.iter()
			.map(|migration| migration.name.as_str())
			.collect::<Vec<_>>(),
		["a"]
	);
	let audited = AuditEvent::query()
		.filter(|event| event.action.eq("database.backup"))
		.all(&app.system())
		.await
		.expect("audit");
	assert_eq!(audited.len(), 1);

	// A backup that failed says why, without any address's password, and
	// doesn't count against the ten.
	let failing = app
		.call("create_backup", place(&shop, "production"), shop.as_ana())
		.await;
	let failing = failing.expect(200).json()["id"].clone();
	let job = shop.cluster.take().await;
	shop.cluster
		.finish(
			&job,
			json!({
				"kind": "backup_database",
				"result": "failed",
				"reason": "pg_dump: postgres://env_ab:c0ffee11@postgres-rw:5432/env_ab: disk full",
			}),
		)
		.await
		.expect(200);
	let listed = app
		.call(
			"database_backups",
			place(&shop, "production"),
			shop.as_ana(),
		)
		.await;
	let failed = listed.expect(200).json()["backups"][0].clone();
	assert_eq!(failed["id"], failing);
	assert_eq!(failed["state"], "failed");
	assert!(
		failed["failure"]
			.as_str()
			.is_some_and(|failure| failure.contains("disk full")),
		"{failed}"
	);
	assert_eq!(
		app.find_in_every_table("c0ffee11").await,
		Vec::<String>::new()
	);
	// A location that couldn't be shown safely is never kept.
	app.call("create_backup", place(&shop, "production"), shop.as_ana())
		.await
		.expect(200);
	let job = shop.cluster.take().await;
	shop.cluster
		.finish(
			&job,
			json!({
				"kind": "backup_database",
				"result": "taken",
				"location": "dumps/\u{1b}[2Jone",
				"bytes": 1,
			}),
		)
		.await
		.expect(200);
	assert_eq!(
		list(&shop, "production").await["backups"][0]["state"],
		"failed"
	);
	// Neither is a dump larger than a backup may be, whatever the worker
	// did about being told so.
	app.call("create_backup", place(&shop, "production"), shop.as_ana())
		.await
		.expect(200);
	let job = shop.cluster.take().await;
	shop.cluster
		.finish(
			&job,
			json!({
				"kind": "backup_database",
				"result": "taken",
				"location": location("vast"),
				"bytes": cap + 1,
			}),
		)
		.await
		.expect(200);
	let vast = list(&shop, "production").await["backups"][0].clone();
	assert_eq!(vast["state"], "failed");
	assert_eq!(vast["bytes"], Value::Null);
	assert!(
		vast["failure"]
			.as_str()
			.is_some_and(|failure| failure.contains("larger than a backup may be")),
		"{vast}"
	);
	assert_eq!(
		app.find_in_every_table(&location("vast")).await,
		Vec::<String>::new()
	);
	// Neither is an address with a password in it, anywhere.
	app.call("create_backup", place(&shop, "production"), shop.as_ana())
		.await
		.expect(200);
	let job = shop.cluster.take().await;
	shop.cluster
		.finish(
			&job,
			json!({
				"kind": "backup_database",
				"result": "taken",
				"location": "https://dumper:c0ffee33@dumps.example/one",
				"bytes": 1,
			}),
		)
		.await
		.expect(200);
	assert_eq!(
		list(&shop, "production").await["backups"][0]["state"],
		"failed"
	);
	assert_eq!(
		app.find_in_every_table("c0ffee33").await,
		Vec::<String>::new()
	);
	assert_eq!(
		app.find_in_every_table("[2Jone").await,
		Vec::<String>::new()
	);

	for count in 2..=10 {
		back_up(&shop, "production", &format!("more-{count}")).await;
	}

	app.call("create_backup", place(&shop, "production"), shop.as_ana())
		.await
		.refused(409, "too_many_backups");
	assert_eq!(
		list(&shop, "production").await["backups"]
			.as_array()
			.map(Vec::len),
		Some(14)
	);

	// Expired backups are forgotten, and make room.
	DatabaseBackup::update()
		.filter(|row| row.id.is_not_null())
		.set(
			|row| row.expires_at,
			leptoskit_db::now() - chrono::TimeDelta::seconds(1),
		)
		.run(&app.system())
		.await
		.expect("expire them");
	assert_eq!(list(&shop, "production").await["backups"], json!([]));
	let swept = shop.sweep().await;
	assert_eq!(swept.backups, 14, "{swept:?}");
	back_up(&shop, "production", "after").await;
}

/// A backup goes back over the database it was taken from, when an owner
/// or admin types the environment's name, nothing else is under way, and
/// the live deployment knows the backup's migrations. The worker's dump of
/// what it replaced becomes a backup too.
#[expect(
	clippy::cognitive_complexity,
	clippy::too_many_lines,
	reason = "a scenario is one story, read top to bottom; its asserts are what count"
)]
pub async fn a_backup_is_restored(app: &App) {
	let shop = Shop::open(app).await;
	with_postgres(&shop, 5432).await;
	staging(&shop).await;
	ship(&shop, "production", b"first", &["a"]).await;
	ship(&shop, "staging", b"staging", &["a"]).await;
	let early = back_up(&shop, "production", "early").await;
	let elsewhere = back_up(&shop, "staging", "elsewhere").await;
	ship(&shop, "production", b"second", &["a", "b"]).await;
	let late = back_up(&shop, "production", "late").await;
	let restore = |args: Value| app.call("restore_backup", args, shop.as_ana());

	// The name, typed back.
	let mut unconfirmed = restore_args(&shop, "production", &early);
	unconfirmed["confirm"] = json!("production");
	restore(unconfirmed)
		.await
		.refused(400, "confirmation_mismatch");
	// Another environment's backup isn't this one's, even in one project.
	restore(restore_args(&shop, "production", &elsewhere))
		.await
		.refused(404, "backup_not_found");
	// Developers take backups; replacing a database is for owners and
	// admins, and never for a token without the `admin` scope.
	let dev = app
		.join(shop.as_ana(), &shop.org, "dev@example.com", "developer")
		.await;
	app.call("create_backup", place(&shop, "staging"), As::Session(&dev))
		.await
		.expect(200);
	let job = shop.cluster.take().await;
	shop.cluster.finish(&job, taken("by-dev")).await.expect(200);
	app.call(
		"restore_backup",
		restore_args(&shop, "production", &early),
		As::Session(&dev),
	)
	.await
	.refused(403, "insufficient_role");
	// Nobody outside the organisation sees any of it.
	let stranger = app.sign_in("mallory@example.com").await;

	for (operation, args) in [
		("database_backups", place(&shop, "production")),
		("create_backup", place(&shop, "production")),
		("restore_backup", restore_args(&shop, "production", &early)),
	] {
		let answer = app.call(operation, args, As::Session(&stranger)).await;
		assert_eq!(answer.status, 404, "{operation}: {}", answer.text);
	}

	// A backup that holds a migration the live server doesn't know would
	// leave it refusing to start.
	let was = Placement::query()
		.all(&app.system())
		.await
		.expect("placements");
	let production = was
		.iter()
		.find(|placement| placement.migrations.0.len() == 2)
		.expect("production's placement");
	let production_id = production.id;
	let known = production.migrations.0.clone();
	let set = |migrations: Vec<_>, uncertain: bool| {
		async move {
			Placement::update()
				.filter(|placement| placement.id.eq(production_id))
				.set(|placement| placement.migrations, Json(migrations))
				.set(|placement| placement.schema_uncertain, uncertain)
				.run(&app.system())
				.await
				.expect("set the placement");
		}
	};
	set(known[..1].to_vec(), false).await;
	let refused = restore(restore_args(&shop, "production", &late)).await;
	refused.refused(409, "restore_unavailable");
	assert!(
		refused.text.contains("doesn't know (b)"),
		"{}",
		refused.text
	);
	set(known.clone(), true).await;
	restore(restore_args(&shop, "production", &early))
		.await
		.refused(409, "restore_unavailable");
	set(known, false).await;

	// Not over a deployment in flight.
	let digest = shop.upload(b"third").await;
	shop.deploy("production", &digest, json!({ "database": "postgres" }))
		.await
		.expect(200);
	restore(restore_args(&shop, "production", &early))
		.await
		.refused(409, "deployment_in_flight");
	let job = shop.build.take().await;
	shop.build
		.finish(&job, built(&job, b"third", &["a", "b"]))
		.await
		.expect(200);
	let release = shop.cluster.take().await;
	shop.cluster.finish(&release, applied()).await.expect(200);

	// The early backup holds `a`, and the live server knows `a` and `b`: it
	// applies `b` again as it starts on the restored data.
	let asked = restore(restore_args(&shop, "production", &early)).await;
	let asked = asked.expect(200).json();
	assert_eq!(asked["backup"], early);
	assert_eq!(asked["state"], "pending");
	assert_eq!(
		list(&shop, "production").await["restore"]["state"],
		"pending"
	);
	// Nothing else touches the database meanwhile.
	restore(restore_args(&shop, "production", &late))
		.await
		.refused(409, "backup_in_progress");
	app.call("create_backup", place(&shop, "production"), shop.as_ana())
		.await
		.refused(409, "backup_in_progress");
	shop.deploy("production", &digest, json!({ "database": "postgres" }))
		.await
		.refused(409, "database_restoring");
	let mut rollback = place(&shop, "production");
	rollback["to"] = json!(2);
	app.call("rollback", rollback, shop.as_ana())
		.await
		.refused(409, "database_restoring");

	// The worker gets the dump's location back, and nothing secret.
	let job = shop.cluster.take().await;
	assert_eq!(job["input"]["kind"], "restore_database");
	assert_eq!(job["input"]["backup"], early);
	assert_eq!(job["input"]["location"], location("early"));
	assert!(
		job["input"]["database"]
			.as_str()
			.is_some_and(|name| name.starts_with("env_")),
		"{job}"
	);
	assert!(job.get("secrets").is_none(), "{job}");
	shop.cluster
		.finish(
			&job,
			json!({
				"kind": "restore_database",
				"result": "restored",
				"before_restore": { "location": location("before"), "bytes": 8192 },
			}),
		)
		.await
		.expect(200);
	let listed = app
		.call(
			"database_backups",
			place(&shop, "production"),
			shop.as_ana(),
		)
		.await;
	let after = listed.expect(200).json();
	assert_eq!(after["restore"]["state"], "restored");
	assert_eq!(after["restore"]["backup"], early);
	assert_eq!(after["backups"][0]["kind"], "before_restore");
	assert_eq!(after["backups"][0]["state"], "complete");
	assert_eq!(after["backups"][0]["bytes"], 8192);
	assert!(!listed.text.contains("lk-dumps"), "{}", listed.text);
	let audited = AuditEvent::query()
		.filter(|event| event.action.eq("database.restore"))
		.all(&app.system())
		.await
		.expect("audit");
	assert_eq!(audited.len(), 1);

	// What the restore replaced can be put back in turn, and a restore
	// that fails says why and leaves no backup behind.
	let undo = after["backups"][0]["id"].clone();
	restore(restore_args(&shop, "production", &undo))
		.await
		.expect(200);
	let job = shop.cluster.take().await;
	assert_eq!(job["input"]["location"], location("before"));
	shop.cluster
		.finish(
			&job,
			json!({
				"kind": "restore_database",
				"result": "failed",
				"reason": "app never became ready on postgres://env_ab:c0ffee22@postgres-rw:5432/env_ab",
			}),
		)
		.await
		.expect(200);
	let failed = list(&shop, "production").await;
	assert_eq!(failed["restore"]["state"], "failed");
	assert!(
		failed["restore"]["failure"]
			.as_str()
			.is_some_and(|failure| failure.contains("never became ready")),
		"{failed}"
	);
	assert_eq!(failed["backups"], after["backups"]);
	assert_eq!(
		app.find_in_every_table("c0ffee22").await,
		Vec::<String>::new()
	);
	// With the restore settled, deploying works again.
	ship(&shop, "production", b"fourth", &["a", "b"]).await;
}

/// A worker that goes quiet fails a backup, which can be asked for again,
/// and leaves a restore unknown until an operator says which database the
/// environment runs on, or the worker says after all.
#[expect(
	clippy::cognitive_complexity,
	reason = "a scenario is one story, read top to bottom; its asserts are what count"
)]
pub async fn a_quiet_worker_fails_a_backup_and_leaves_a_restore_unknown(app: &App) {
	let shop = Shop::open(app).await;
	with_postgres(&shop, 5432).await;
	ship(&shop, "production", b"first", &["a"]).await;
	let kept = back_up(&shop, "production", "kept").await;

	app.call("create_backup", place(&shop, "production"), shop.as_ana())
		.await
		.expect(200);
	let quiet = shop.cluster.take().await;
	shop.lapse_leases().await;
	let swept = shop.sweep().await;
	assert_eq!((swept.abandoned, swept.unknown), (1, 0), "{swept:?}");
	let listed = list(&shop, "production").await;
	assert_eq!(listed["backups"][0]["state"], "failed");
	assert!(
		listed["backups"][0]["failure"]
			.as_str()
			.is_some_and(|failure| failure.contains("lease ran out")),
		"{listed}"
	);
	// What it says later changes nothing: it deletes its dump.
	shop.cluster
		.finish(&quiet, taken("late"))
		.await
		.refused(404, "job_not_held");
	assert_eq!(
		list(&shop, "production").await["backups"][0]["state"],
		"failed"
	);

	let restore = || {
		app.call(
			"restore_backup",
			restore_args(&shop, "production", &kept),
			shop.as_ana(),
		)
	};
	let reconcile = |job: Value, found: Value| {
		app.call(
			"reconcile",
			json!({ "job": job, "found": found }),
			shop.as_ops(),
		)
	};

	// Unknown, and found not restored.
	restore().await.expect(200);
	let job = shop.cluster.take().await;
	shop.lapse_leases().await;
	let swept = shop.sweep().await;
	assert_eq!((swept.abandoned, swept.unknown), (0, 1), "{swept:?}");
	assert_eq!(
		list(&shop, "production").await["restore"]["state"],
		"unknown"
	);
	restore().await.refused(409, "backup_in_progress");
	let digest = shop.upload(b"second").await;
	shop.deploy("production", &digest, json!({ "database": "postgres" }))
		.await
		.refused(409, "database_restoring");
	assert_eq!(shop.cluster.claim().await, None);
	reconcile(
		job["job"].clone(),
		json!({ "found": "not_applied", "detail": "the old database is in place" }),
	)
	.await
	.expect(200);
	let found = list(&shop, "production").await["restore"].clone();
	assert_eq!(found["state"], "failed");
	assert!(
		found["failure"]
			.as_str()
			.is_some_and(|failure| failure.contains("the old database is in place")),
		"{found}"
	);

	// Unknown, and found restored.
	restore().await.expect(200);
	let job = shop.cluster.take().await;
	shop.lapse_leases().await;
	shop.sweep().await;
	reconcile(job["job"].clone(), json!({ "found": "applied" }))
		.await
		.expect(200);
	assert_eq!(
		list(&shop, "production").await["restore"]["state"],
		"restored"
	);
	shop.cluster
		.finish(
			&job,
			json!({ "kind": "restore_database", "result": "failed", "reason": "too late" }),
		)
		.await
		.refused(409, "outcome_conflict");

	// Unknown, and then its worker says.
	restore().await.expect(200);
	let job = shop.cluster.take().await;
	shop.lapse_leases().await;
	shop.sweep().await;
	shop.cluster
		.finish(
			&job,
			json!({
				"kind": "restore_database",
				"result": "restored",
				"before_restore": { "location": location("before"), "bytes": 1 },
			}),
		)
		.await
		.expect(200);
	let settled = list(&shop, "production").await;
	assert_eq!(settled["restore"]["state"], "restored");
	assert_eq!(settled["backups"][0]["kind"], "before_restore");

	// A restore, and a backup, that no worker takes for an hour is given
	// up on: it never runs later, over what was written since.
	let wait_an_hour = || {
		async {
			Job::update()
				.filter(|job| job.state.eq(leptoskit_cloud::operator::JobState::Queued))
				.set(
					|job| job.queued_at,
					leptoskit_db::now() - chrono::TimeDelta::minutes(61),
				)
				.run(&app.system())
				.await
				.expect("age the queue");
		}
	};
	restore().await.expect(200);
	assert_eq!(shop.sweep().await.abandoned, 0);
	wait_an_hour().await;
	assert_eq!(shop.sweep().await.abandoned, 1);
	assert_eq!(shop.cluster.claim().await, None);
	let given_up = list(&shop, "production").await["restore"].clone();
	assert_eq!(given_up["state"], "failed");
	assert!(
		given_up["failure"]
			.as_str()
			.is_some_and(|failure| failure.contains("didn't take it in time")),
		"{given_up}"
	);
	app.call("create_backup", place(&shop, "production"), shop.as_ana())
		.await
		.expect(200);
	wait_an_hour().await;
	assert_eq!(shop.sweep().await.abandoned, 1);
	assert_eq!(shop.cluster.claim().await, None);
	assert_eq!(
		list(&shop, "production").await["backups"][0]["state"],
		"failed"
	);
	shop.deploy("production", &digest, json!({ "database": "postgres" }))
		.await
		.expect(200);
}

/// Deleting an environment gives up on a backup nobody has taken, forgets
/// its backups, and tells the cluster's worker to drop the database, whose
/// dumps go with it.
pub async fn a_deleted_environment_takes_its_backups(app: &App) {
	let shop = Shop::open(app).await;
	with_postgres(&shop, 5432).await;
	staging(&shop).await;
	ship(&shop, "staging", b"short-lived", &[]).await;
	back_up(&shop, "staging", "one").await;
	app.call("create_backup", place(&shop, "staging"), shop.as_ana())
		.await
		.expect(200);

	let mut delete = place(&shop, "staging");
	delete["confirm"] = json!("staging");
	app.call("delete_environment", delete, shop.as_ana())
		.await
		.expect(200);
	// The removal, and not the backup that was queued before it.
	let destroy = shop.cluster.take().await;
	assert_eq!(destroy["input"]["kind"], "destroy_environment");
	assert!(destroy["input"]["database"].is_string(), "{destroy}");
	shop.cluster
		.finish(
			&destroy,
			json!({ "kind": "destroy_environment", "result": "destroyed" }),
		)
		.await
		.expect(200);
	assert_eq!(shop.cluster.claim().await, None);
	let rows = DatabaseBackup::query()
		.all(&app.system())
		.await
		.expect("backups");
	assert_eq!(rows.len(), 0);
	let jobs = Job::query().all(&app.system()).await.expect("jobs");
	let abandoned = jobs
		.iter()
		.filter(|job| job.state == leptoskit_cloud::operator::JobState::Abandoned)
		.count();
	assert_eq!(abandoned, 1);
}
