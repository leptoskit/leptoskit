//! What Control 2's security review found, each as a scenario that failed
//! before its fix. Written once and run on SQLite (`deployments.rs`) and
//! Postgres (`postgres.rs`), as `shipping.rs`'s are.

use leptoskit_cloud::operator::JobState;
use leptoskit_cloud::server::StoredInput;
use leptoskit_cloud::tables::Cluster;
use leptoskit_cloud::tables::Job;
use leptoskit_cloud::tables::LogChunk;
use leptoskit_cloud::tables::NewJob;
use leptoskit_cloud::tables::Organization;
use leptoskit_cloud::tables::SourceArchive;
use leptoskit_cloud::workers::JobKind;
use leptoskit_db::Id;
use leptoskit_db::Json;
use serde_json::Value;
use serde_json::json;

use super::App;
use super::shipping::Shop;
use super::shipping::applied;
use super::shipping::begin;
use super::shipping::built;
use super::shipping::complete;
use super::shipping::digest_of;
use super::shipping::pair;
use super::shipping::put;
use super::shipping::register_cluster;

/// Settings small enough for the scenarios here to reach their caps: 200
/// bytes of log per build and 250 per organisation, counted as stored.
pub fn settings(settings: &mut leptoskit_cloud::server::ShippingSettings) {
	settings.build_log_cap = 200;
	settings.organization_log_cap = 250;
}

/// Finding 1. A digest's upload has one size: beginning it again with
/// another is refused, so a ticket already given out never uploads more
/// than its organisation's reservation counts.
pub async fn an_upload_keeps_the_size_it_was_begun_with(app: &App) {
	let shop = Shop::open(app).await;
	let bytes = [7_u8; 64];
	let began = begin(app, shop.as_ana(), &shop.org, "notes", &bytes).await;
	let ticket = began.expect(200).json()["ticket"].clone();

	// The second `begin_upload` used to shrink the reservation to a byte
	// while the first ticket still took all 64.
	app.call(
		"begin_upload",
		json!({
			"organization": shop.org,
			"project": "notes",
			"digest": digest_of(&bytes),
			"size": 1,
		}),
		shop.as_ana(),
	)
	.await
	.refused(409, "archive_size_conflict");
	let reserved = SourceArchive::query()
		.filter(|archive| archive.digest.eq(digest_of(&bytes)))
		.one(&app.system())
		.await
		.expect("the reservation");
	assert_eq!(reserved.size, 64);

	// The same size again is the same upload, and the ticket still works.
	let again = begin(app, shop.as_ana(), &shop.org, "notes", &bytes).await;
	assert_eq!(again.expect(200).json()["status"], "upload");
	assert_eq!(put(app, &ticket, &bytes).await, 200);
	let completed = complete(app, shop.as_ana(), &shop.org, "notes", &digest_of(&bytes)).await;
	assert_eq!(completed.expect(200).json()["size"], 64);
}

/// A queued job nothing in these scenarios can finish, to fill a queue
/// with.
async fn filler(
	app: &App,
	organization: Id<Organization>,
	kind: JobKind,
	cluster: Option<Id<Cluster>>,
) {
	let input: StoredInput = serde_json::from_value(json!({
		"kind": "destroy_environment",
		"environment": {
			"organization": { "slug": "filler", "id": uuid::Uuid::now_v7() },
			"project": { "slug": "filler", "id": uuid::Uuid::now_v7() },
			"environment": { "slug": "filler", "id": uuid::Uuid::now_v7() },
		},
	}))
	.expect("an input");
	NewJob {
		kind,
		state: JobState::Queued,
		organization_id: organization,
		deployment_id: None,
		environment_id: None,
		cluster_id: cluster,
		requirements: Json(if kind == JobKind::BuildServer {
			vec!["a-capability-no-worker-has".to_owned()]
		} else {
			Vec::new()
		}),
		input: Json(input),
		attempt: 0,
		max_attempts: 1,
		cancel_requested: false,
		worker_id: None,
		lease_expires_at: None,
		claimed_at: None,
		log_bytes: 0,
		outcome: Json(None),
		queued_at: leptoskit_db::now(),
	}
	.insert(&app.system())
	.await
	.expect("a filler job");
}

/// Finding 2. More queued jobs than one page, none of which a worker may
/// take, don't hide the one it may: another cluster's releases, and builds
/// that need a capability the worker lacks.
pub async fn a_full_queue_never_hides_a_job_a_worker_may_take(app: &App) {
	let shop = Shop::open(app).await;
	register_cluster(app, &shop.ops, "eu-two").await;
	let other = Cluster::query()
		.filter(|cluster| cluster.name.eq("eu-two"))
		.one(&app.system())
		.await
		.expect("the other cluster");
	let stranger: Id<Organization> = Id::new();

	for _ in 0..130 {
		filler(app, stranger, JobKind::DestroyEnvironment, Some(other.id)).await;
		filler(app, stranger, JobKind::BuildServer, None).await;
	}

	// Both workers used to be handed nothing: the first hundred queued
	// jobs of their role were all someone else's.
	shop.ship("production", b"behind a long queue", &[]).await;

	// A worker bound to one organisation is no different.
	let dedicated = pair(
		app,
		&shop.ops,
		json!({ "role": "build", "organization": shop.org }),
		"ana-builder",
	)
	.await;
	let digest = shop.upload(b"for the dedicated builder").await;
	shop.deploy("production", &digest, json!({}))
		.await
		.expect(200);
	let build = dedicated.take().await;
	assert_eq!(build["input"]["kind"], "build_server");
}

/// Finding 3. A build's log is capped, an organisation's build logs are
/// capped together, and the cap is given back as chunks expire. Needs
/// [`settings`].
pub async fn build_logs_are_capped(app: &App) {
	let shop = Shop::open(app).await;
	let digest = shop.upload(b"a chatty build").await;
	shop.deploy("production", &digest, json!({}))
		.await
		.expect(200);
	let build = shop.build.take().await;
	let sixty = "x".repeat(60);
	shop.build.log(&build, 0, &[&sixty]).await.expect(200);
	// Sent again, a chunk isn't counted again.
	shop.build.log(&build, 0, &[&sixty]).await.expect(200);
	// The caps count a line as stored, not as sent: two are more than 200
	// bytes, past the build's 200.
	shop.build
		.log(&build, 1, &[&sixty])
		.await
		.refused(409, "log_full");
	shop.build
		.log(&build, 1, &[&"w".repeat(39)])
		.await
		.expect(200);
	shop.build
		.finish(&build, built(&build, b"chatty", &[]))
		.await
		.expect(200);
	let release = shop.cluster.take().await;
	shop.cluster.finish(&release, applied()).await.expect(200);

	// The organisation keeps 181 bytes or more of its 250 as stored: a
	// second build can't add a line.
	let digest = shop.upload(b"another chatty build").await;
	shop.deploy("production", &digest, json!({}))
		.await
		.expect(200);
	let build = shop.build.take().await;
	shop.build
		.log(&build, 0, &[&sixty])
		.await
		.refused(409, "log_full");

	// Expired chunks give their bytes back.
	LogChunk::update()
		.filter(|chunk| chunk.id.is_not_null())
		.set(
			|chunk| chunk.expires_at,
			leptoskit_db::now() - chrono::TimeDelta::seconds(1),
		)
		.run(&app.system())
		.await
		.expect("expire them");
	let swept = shop.sweep().await;
	assert_eq!(swept.log_chunks, 2, "{swept:?}");
	shop.build.log(&build, 0, &[&sixty]).await.expect(200);
}

/// Finding 3. Renewing never takes a build's lease past its deadline, so
/// a worker can't hold a build, and write its log, for ever.
pub async fn a_build_attempt_has_a_deadline(app: &App) {
	let shop = Shop::open(app).await;
	let digest = shop.upload(b"a slow build").await;
	shop.deploy("production", &digest, json!({}))
		.await
		.expect(200);
	let build = shop.build.take().await;
	shop.build.renew(&build).await.expect(200);

	// Three hours on, past the two a build gets, with its lease still
	// live: a renewal used to give it two more minutes, every time.
	Job::update()
		.filter(|job| job.claimed_at.is_not_null())
		.set(
			|job| job.claimed_at,
			Some(leptoskit_db::now() - chrono::TimeDelta::hours(3)),
		)
		.run(&app.system())
		.await
		.expect("age the claim");
	shop.build.renew(&build).await.refused(404, "job_not_held");
}

/// Finding 5. A lease past its end holds nothing, before the sweeper has
/// seen it: no renewal, no log, and no result.
pub async fn an_expired_lease_holds_nothing(app: &App) {
	let shop = Shop::open(app).await;
	let digest = shop.upload(b"left too long").await;
	shop.deploy("production", &digest, json!({}))
		.await
		.expect(200);
	let build = shop.build.take().await;
	shop.lapse_leases().await;
	shop.build.renew(&build).await.refused(404, "job_not_held");
	shop.build
		.log(&build, 0, &["too late"])
		.await
		.refused(404, "job_not_held");
	shop.build
		.finish(&build, built(&build, b"late", &[]))
		.await
		.refused(404, "job_not_held");
	assert_eq!(
		shop.state("production", 1).await,
		json!({ "state": "building" })
	);
	assert!(
		shop.cluster.claim().await.is_none(),
		"no release was queued"
	);

	// The sweeper offers the build again, and that attempt counts.
	let swept = shop.sweep().await;
	assert_eq!(swept.retried, 1, "{swept:?}");
	let again = shop.build.take().await;
	assert_eq!(again["attempt"], 2);
	shop.build
		.finish(&again, built(&again, b"in time", &[]))
		.await
		.expect(200);
}

/// Finding 4. A build's images stay in the repository its job was given:
/// one that names another fails, and nothing is released.
pub async fn a_build_cannot_name_another_repository(app: &App) {
	let shop = Shop::open(app).await;

	for stray in ["image", "process"] {
		let digest = shop.upload(format!("strays by {stray}").as_bytes()).await;
		let deployed = shop.deploy("production", &digest, json!({})).await;
		let number = deployed.expect(200).json()["number"]
			.as_u64()
			.expect("a number");
		let build = shop.build.take().await;
		let mut outcome = built(&build, b"elsewhere", &[]);
		let elsewhere = json!({
			"repository": "registry.example.test/leptoskit/another-organisation/their-project",
			"digest": digest_of(b"theirs"),
		});

		if stray == "image" {
			outcome["image"] = elsewhere;
		} else {
			outcome["processes"][0]["image"] = elsewhere;
		}

		shop.build.finish(&build, outcome).await.expect(200);
		assert_eq!(
			shop.state("production", number).await,
			json!({ "state": "failed", "stage": "building", "blocker": "build_failed" }),
			"a stray {stray}"
		);
		assert!(shop.cluster.claim().await.is_none(), "nothing is released");
	}
}

/// Finding 6. While a failed release may have applied migrations nobody
/// recorded, no rollback is offered: the database may be past what the
/// target knows.
pub async fn rollback_waits_for_a_certain_schema(app: &App) {
	let shop = Shop::open(app).await;
	shop.ship("production", b"v1", &["0001_notes"]).await;
	shop.ship("production", b"v2", &["0001_notes"]).await;
	let digest = shop.upload(b"v3 adds a migration").await;
	shop.deploy("production", &digest, json!({ "database": "sqlite" }))
		.await
		.expect(200);
	let build = shop.build.take().await;
	shop.build
		.finish(&build, built(&build, b"v3", &["0001_notes", "0002_tags"]))
		.await
		.expect(200);
	let release = shop.cluster.take().await;
	shop.cluster
		.finish(
			&release,
			json!({
				"kind": "release",
				"result": "not_ready",
				"process": "app",
				"reason": "it stopped after migrating",
			}),
		)
		.await
		.expect(200);
	let rollback = app
		.call(
			"rollback",
			json!({
				"organization": shop.org,
				"project": "notes",
				"environment": "production",
				"to": 1,
			}),
			shop.as_ana(),
		)
		.await;
	rollback.refused(409, "rollback_unavailable");
	assert!(
		rollback.message().contains("uncertain"),
		"{}",
		rollback.text
	);
}

/// The environment's ID, as `environments` lists it.
async fn environment_id(shop: &Shop<'_>, name: &str) -> String {
	let environments = shop
		.app
		.call(
			"environments",
			json!({ "organization": shop.org, "project": "notes" }),
			shop.as_ana(),
		)
		.await
		.expect(200)
		.json();
	environments
		.as_array()
		.expect("environments")
		.iter()
		.find(|environment| environment["name"] == name)
		.unwrap_or_else(|| panic!("`{name}` in {environments}"))["id"]
		.as_str()
		.expect("an id")
		.to_owned()
}

/// Finding 7. A report with as many lines as a report may carry, each as
/// long as a line may be, is kept in objects small enough to read back,
/// and every line is read.
pub async fn the_largest_status_report_reads_back(app: &App) {
	let shop = Shop::open(app).await;
	shop.ship("production", b"a loud app", &[]).await;
	let environment = environment_id(&shop, "production").await;
	let now = chrono::Utc::now().timestamp_millis();
	let long = "m".repeat(leptoskit_cloud::workers::MAX_LOG_LINE_BYTES);
	let logs: Vec<Value> = (0..leptoskit_cloud::workers::MAX_STATUS_LOG_LINES)
		.map(|_| json!({ "process": "app", "pod": "app-1", "at_millis": now, "message": long }))
		.collect();
	let reported = shop
		.cluster
		.call(
			"report_status",
			json!({
				"environment": environment,
				"observed": {
					"environment": {
						"organization": { "slug": shop.org, "id": uuid::Uuid::now_v7() },
						"project": { "slug": "notes", "id": uuid::Uuid::now_v7() },
						"environment": { "slug": "production", "id": environment },
					},
					"processes": [],
				},
				"logs": logs,
			}),
		)
		.await;
	assert_eq!(
		reported.expect(200).json()["log_lines"],
		leptoskit_cloud::workers::MAX_STATUS_LOG_LINES
	);
	let mut read = 0;
	let mut after: Option<u64> = None;

	// This used to answer a server error: the one object was past what the
	// store's readers read.
	loop {
		let page = app
			.call(
				"runtime_logs",
				json!({
					"organization": shop.org,
					"project": "notes",
					"environment": "production",
					"after": after,
				}),
				shop.as_ana(),
			)
			.await;
		let page = page.expect(200).json();
		read += page["lines"].as_array().map_or(0, Vec::len);

		match page["next"].as_u64() {
			Some(next) => after = Some(next),
			None => break,
		}
	}

	assert_eq!(read, leptoskit_cloud::workers::MAX_STATUS_LOG_LINES);
}

/// Finding 8. A removal that failed isn't done: it stays in front of
/// operators with its reason, and reconciling it runs it again.
pub async fn a_failed_teardown_stays_actionable(app: &App) {
	let shop = Shop::open(app).await;
	app.call(
		"create_environment",
		json!({ "organization": shop.org, "project": "notes", "name": "staging", "kind": "staging" }),
		shop.as_ana(),
	)
	.await
	.expect(200);
	shop.ship("staging", b"hard to remove", &[]).await;
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

	// It used to be `done`, and in no operator's list.
	let jobs = app.call("jobs", json!({}), shop.as_ops()).await;
	let jobs = jobs.expect(200).json();
	let job = jobs
		.as_array()
		.expect("jobs")
		.iter()
		.find(|job| job["id"] == destroy["job"])
		.unwrap_or_else(|| panic!("the removal in {jobs}"));
	assert_eq!(job["state"], "unknown");
	assert_eq!(job["failure"], "a volume wouldn't detach");
	assert!(shop.cluster.claim().await.is_none(), "not until reconciled");

	// An operator sends it round again.
	let reconciled = app
		.call(
			"reconcile",
			json!({
				"job": destroy["job"],
				"found": { "found": "not_applied", "detail": "the namespace is still there" },
			}),
			shop.as_ops(),
		)
		.await;
	assert_eq!(reconciled.expect(200).json()["state"], "queued");
	let again = shop.cluster.take().await;
	assert_eq!(again["job"], destroy["job"]);
	assert_eq!(again["attempt"], 2);
	shop.cluster
		.finish(
			&again,
			json!({ "kind": "destroy_environment", "result": "destroyed" }),
		)
		.await
		.expect(200);
	let left = app.call("jobs", json!({}), shop.as_ops()).await;
	assert_eq!(left.expect(200).json(), json!([]));
}

/// Finding 9. An archive uploaded again after the sweeper deleted it is
/// kept under a key of its own, so the deletion of the old one's object
/// can never remove the new one's; and an archive renewed since it expired
/// isn't swept.
pub async fn a_swept_archive_never_takes_its_replacement(app: &App) {
	let shop = Shop::open(app).await;
	let bytes = b"uploaded, swept, and uploaded again";
	let began = begin(app, shop.as_ana(), &shop.org, "notes", bytes).await;
	let first = began.expect(200).json()["ticket"]["url"]
		.as_str()
		.expect("an address")
		.split('?')
		.next()
		.expect("a path")
		.to_owned();
	let expire = || {
		async {
			SourceArchive::update()
				.filter(|archive| archive.digest.eq(digest_of(bytes)))
				.set(
					|archive| archive.expires_at,
					leptoskit_db::now() - chrono::TimeDelta::seconds(1),
				)
				.run(&app.system())
				.await
				.expect("expire it");
		}
	};
	expire().await;
	assert_eq!(shop.sweep().await.archives, 1);
	let began = begin(app, shop.as_ana(), &shop.org, "notes", bytes).await;
	let ticket = began.expect(200).json()["ticket"].clone();
	let second = ticket["url"]
		.as_str()
		.expect("an address")
		.split('?')
		.next()
		.expect("a path")
		.to_owned();
	assert_ne!(first, second, "each archive row has its own key");
	assert_eq!(put(app, &ticket, bytes).await, 200);
	complete(app, shop.as_ana(), &shop.org, "notes", &digest_of(bytes))
		.await
		.expect(200);

	// Expired, then begun again: `begin_upload` renews it, and the sweep
	// that follows leaves it.
	expire().await;
	let renewed = begin(app, shop.as_ana(), &shop.org, "notes", bytes).await;
	assert_eq!(renewed.expect(200).json()["status"], "stored");
	assert_eq!(shop.sweep().await.archives, 0);
	shop.deploy("production", &digest_of(bytes), json!({}))
		.await
		.expect(200);
}

/// A secret split over a chunk's lines is hidden in each line it touches,
/// and control characters never reach a reader's terminal.
pub async fn logs_hide_split_secrets_and_control_characters(app: &App) {
	let shop = Shop::open(app).await;
	let digest = shop.upload(b"a leaky build").await;
	shop.deploy("production", &digest, json!({}))
		.await
		.expect(200);
	let build = shop.build.take().await;
	let key = &shop.build.key;
	let (start, end) = key.split_at(key.len() - 20);
	shop.build
		.log(
			&build,
			0,
			&[
				&format!("the key is {start}"),
				&format!("{end} and that was it"),
				"\u{1b}]0;owned\u{7}\u{1b}[2Jcleared\rover",
			],
		)
		.await
		.expect(200);
	let page = app
		.call(
			"build_logs",
			json!({
				"organization": shop.org,
				"project": "notes",
				"environment": "production",
				"number": 1,
			}),
			shop.as_ana(),
		)
		.await;
	let page = page.expect(200).json();
	let text: Vec<&str> = page["lines"]
		.as_array()
		.expect("lines")
		.iter()
		.map(|line| line["text"].as_str().expect("text"))
		.collect();
	assert_eq!(text.len(), 3);
	assert!(!text[1].contains(end), "{}", text[1]);
	assert!(text[1].ends_with(" and that was it"), "{}", text[1]);
	assert!(
		!text[2].chars().any(char::is_control),
		"control characters in {:?}",
		text[2]
	);
	assert!(text[2].contains("cleared"), "{}", text[2]);
}

/// From the second review. A cluster's worker reports a release it applied
/// even when its lease ran out first and the sweeper hasn't looked: the
/// cluster did it either way, and nothing else would ever say so.
pub async fn a_release_is_reported_after_its_lease(app: &App) {
	let shop = Shop::open(app).await;
	let digest = shop.upload(b"applied, reported late").await;
	shop.deploy("production", &digest, json!({}))
		.await
		.expect(200);
	let build = shop.build.take().await;
	shop.build
		.finish(&build, built(&build, b"late", &[]))
		.await
		.expect(200);
	let release = shop.cluster.take().await;
	shop.lapse_leases().await;
	shop.cluster.finish(&release, applied()).await.expect(200);
	assert_eq!(
		shop.state("production", 1).await,
		json!({ "state": "live" })
	);
}

/// From the second review. A worker whose removal failed, and then worked
/// when it tried again itself, says so, and the job is done without an
/// operator.
pub async fn a_removal_that_then_works_is_done(app: &App) {
	let shop = Shop::open(app).await;
	app.call(
		"create_environment",
		json!({ "organization": shop.org, "project": "notes", "name": "staging", "kind": "staging" }),
		shop.as_ana(),
	)
	.await
	.expect(200);
	shop.ship("staging", b"removed on the second try", &[])
		.await;
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
	let destroy = shop.cluster.take().await;
	let failed = json!({
		"kind": "destroy_environment",
		"result": "failed",
		"reason": "a volume wouldn't detach",
	});
	shop.cluster
		.finish(&destroy, failed.clone())
		.await
		.expect(200);
	shop.cluster
		.finish(
			&destroy,
			json!({ "kind": "destroy_environment", "result": "destroyed" }),
		)
		.await
		.expect(200);
	let left = app.call("jobs", json!({}), shop.as_ops()).await;
	assert_eq!(left.expect(200).json(), json!([]));
	// And it can't then be called failed again.
	shop.cluster
		.finish(&destroy, failed)
		.await
		.refused(409, "outcome_conflict");
}
