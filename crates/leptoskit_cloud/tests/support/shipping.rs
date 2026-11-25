//! Uploads, deployments, and the worker protocol, written once and run on
//! SQLite (`deployments.rs`) and Postgres (`postgres.rs`). A scripted fake
//! worker claims, renews, logs, and finishes jobs over HTTP, as the real
//! build and cluster workers do.

use leptoskit_cloud::server::DEFAULT_REGION;
use leptoskit_cloud::tables::DataKey;
use leptoskit_cloud::tables::Job;
use leptoskit_cloud::tables::LogChunk;
use leptoskit_cloud::tables::SourceArchive;
use serde_json::Value;
use serde_json::json;
use sha2::Digest as _;

use super::APPS_DOMAIN;
use super::Answer;
use super::App;
use super::As;

/// `sha256:<hex>` of `bytes`, as an archive's digest is written.
pub fn digest_of(bytes: &[u8]) -> String {
	format!("sha256:{}", hex::encode(sha2::Sha256::digest(bytes)))
}

/// A worker, calling with its key.
pub struct Worker<'a> {
	pub app: &'a App,
	pub key: String,
	pub id: String,
}

impl Worker<'_> {
	pub fn as_worker(&self) -> As<'_> {
		As::Bearer(&self.key)
	}

	pub async fn call(&self, operation: &str, args: Value) -> Answer {
		self.app.call(operation, args, self.as_worker()).await
	}

	/// The job it is given now, if any, without waiting.
	pub async fn claim(&self) -> Option<Value> {
		let answer = self.call("claim", json!({ "wait_seconds": 0 })).await;
		let assignment = answer.expect(200).json();
		(!assignment.is_null()).then_some(assignment)
	}

	/// The job it is given now, failing the test without one.
	pub async fn take(&self) -> Value {
		self.claim().await.expect("a job to claim")
	}

	/// `finish` for `assignment`'s attempt.
	pub async fn finish(&self, assignment: &Value, outcome: Value) -> Answer {
		self.call(
			"finish",
			json!({
				"job": assignment["job"],
				"attempt": assignment["attempt"],
				"outcome": outcome,
			}),
		)
		.await
	}

	/// `append_log` for `assignment`'s attempt.
	pub async fn log(&self, assignment: &Value, seq: u32, lines: &[&str]) -> Answer {
		self.call(
			"append_log",
			json!({
				"job": assignment["job"],
				"attempt": assignment["attempt"],
				"seq": seq,
				"lines": lines,
			}),
		)
		.await
	}

	pub async fn renew(&self, assignment: &Value) -> Answer {
		self.call("renew", json!({ "job": assignment["job"] }))
			.await
	}
}

/// A build worker's successful outcome for the build `assignment`: one
/// image in the repository it was given, the app's process, and
/// `migrations` by name.
pub fn built(assignment: &Value, image: &[u8], migrations: &[&str]) -> Value {
	let input = &assignment["input"];
	assert_eq!(input["kind"], "build_server", "{assignment}");
	json!({
		"kind": "build_server",
		"result": "succeeded",
		"image": {
			"repository": input["image_repository"],
			"digest": digest_of(image),
		},
		"processes": [{
			"name": "app",
			"command": ["/app/server"],
			"port": 8080,
			"ready_path": "/readyz",
			"live_path": "/healthz",
		}],
		"migrations": migrations
			.iter()
			.map(|name| json!({ "name": name, "checksum": digest_of(name.as_bytes()) }))
			.collect::<Vec<_>>(),
		"steps": [{ "name": "cargo build", "millis": 1200, "cached_percent": 80 }],
	})
}

/// A cluster worker's outcome: the release applied.
pub fn applied() -> Value {
	json!({ "kind": "release", "result": "applied" })
}

/// An organisation that ships: its owner Ana, her personal organisation,
/// the project `notes`, an operator, a cluster in the default region, and
/// a shared build worker and that cluster's worker.
pub struct Shop<'a> {
	pub app: &'a App,
	pub ana: String,
	pub org: String,
	pub ops: String,
	pub build: Worker<'a>,
	pub cluster: Worker<'a>,
}

impl<'a> Shop<'a> {
	pub async fn open(app: &'a App) -> Self {
		let ana = app.sign_in("ana@example.com").await;
		let org = app.personal(As::Session(&ana)).await;
		app.project(As::Session(&ana), &org, "notes").await;
		let ops = operator(app).await;
		register_cluster(app, &ops, "eu-one").await;
		let build = pair(app, &ops, json!({ "role": "build" }), "builder-1").await;
		let cluster = pair(
			app,
			&ops,
			json!({ "role": "cluster", "cluster": "eu-one" }),
			"eu-one-worker",
		)
		.await;

		Self {
			app,
			ana,
			org,
			ops,
			build,
			cluster,
		}
	}

	pub fn as_ana(&self) -> As<'_> {
		As::Session(&self.ana)
	}

	pub fn as_ops(&self) -> As<'_> {
		As::Session(&self.ops)
	}

	/// Uploads `bytes` as Ana and returns their digest.
	pub async fn upload(&self, bytes: &[u8]) -> String {
		upload(self.app, self.as_ana(), &self.org, "notes", bytes).await
	}

	/// Deploys `digest` to `environment` as Ana.
	pub async fn deploy(&self, environment: &str, digest: &str, app: Value) -> Answer {
		self.app
			.call(
				"deploy",
				json!({
					"organization": self.org,
					"project": "notes",
					"environment": environment,
					"source": { "kind": "upload", "archive": digest },
					"config": { "app": app },
					"host": "ana-laptop",
				}),
				self.as_ana(),
			)
			.await
	}

	/// `operation` on deployment `number` of `environment`, as Ana.
	pub async fn on(&self, operation: &str, environment: &str, number: u64) -> Answer {
		self.app
			.call(
				operation,
				json!({
					"organization": self.org,
					"project": "notes",
					"environment": environment,
					"number": number,
				}),
				self.as_ana(),
			)
			.await
	}

	/// Deployment `number`'s sheet.
	pub async fn sheet(&self, environment: &str, number: u64) -> Value {
		self.on("deployment", environment, number)
			.await
			.expect(200)
			.json()
	}

	/// Deployment `number`'s state, as its `state` tag.
	pub async fn state(&self, environment: &str, number: u64) -> Value {
		self.sheet(environment, number).await["deployment"]["state"].clone()
	}

	/// Deploys `bytes` to `environment` and takes it live through both
	/// workers. Returns its number.
	pub async fn ship(&self, environment: &str, bytes: &[u8], migrations: &[&str]) -> u64 {
		let digest = self.upload(bytes).await;
		let deployed = self
			.deploy(environment, &digest, json!({ "database": "sqlite" }))
			.await;
		let number = deployed.expect(200).json()["number"]
			.as_u64()
			.expect("a number");
		let build = self.build.take().await;
		self.build
			.finish(&build, built(&build, bytes, migrations))
			.await
			.expect(200);
		let release = self.cluster.take().await;
		self.cluster.finish(&release, applied()).await.expect(200);
		assert_eq!(
			self.state(environment, number).await,
			json!({ "state": "live" })
		);
		number
	}

	/// Runs the sweeper now.
	pub async fn sweep(&self) -> leptoskit_cloud::server::Swept {
		self.app.control.sweep().await.expect("the sweep runs")
	}

	/// Makes every lease that's held lapse now.
	pub async fn lapse_leases(&self) {
		Job::update()
			.filter(|job| job.lease_expires_at.is_not_null())
			.set(
				|job| job.lease_expires_at,
				Some(leptoskit_db::now() - chrono::TimeDelta::seconds(1)),
			)
			.run(&self.app.system())
			.await
			.expect("lapse the leases");
	}
}

/// Signs in `ops@example.com`, makes them an operator, and returns their
/// session.
pub async fn operator(app: &App) -> String {
	let ops = app.sign_in("ops@example.com").await;
	leptoskit_cloud::server::grant_operator(&app.db, "ops@example.com")
		.await
		.expect("an operator");
	ops
}

/// Registers the cluster `name` in the default region.
pub async fn register_cluster(app: &App, ops: &str, name: &str) -> Value {
	app.call(
		"register_cluster",
		json!({
			"name": name,
			"region": DEFAULT_REGION,
			"facts": { "runtime_class": "gvisor", "wakes_sleeping": true },
			"capacity_notes": "three nodes",
		}),
		As::Session(ops),
	)
	.await
	.expect(200)
	.json()
}

/// Issues a pairing with `pairing` as the operator, and pairs a worker
/// called `name` with it.
pub async fn pair<'a>(app: &'a App, ops: &str, pairing: Value, name: &str) -> Worker<'a> {
	pair_with(app, ops, pairing, name, &["linux", "x86_64"]).await
}

/// [`pair`], declaring `capabilities`.
pub async fn pair_with<'a>(
	app: &'a App,
	ops: &str,
	pairing: Value,
	name: &str,
	capabilities: &[&str],
) -> Worker<'a> {
	let issued = app
		.call("issue_pairing", pairing, As::Session(ops))
		.await
		.expect(200)
		.json();
	let code = issued["code"].as_str().expect("a code");
	let paired = app
		.call(
			"pair",
			json!({ "code": code, "name": name, "capabilities": capabilities }),
			As::Nobody,
		)
		.await
		.expect(200)
		.json();

	Worker {
		app,
		key: paired["key"].as_str().expect("a key").to_owned(),
		id: paired["worker"]["id"].as_str().expect("an id").to_owned(),
	}
}

/// `begin_upload` for `bytes`.
pub async fn begin(app: &App, caller: As<'_>, org: &str, project: &str, bytes: &[u8]) -> Answer {
	app.call(
		"begin_upload",
		json!({
			"organization": org,
			"project": project,
			"digest": digest_of(bytes),
			"size": bytes.len(),
		}),
		caller,
	)
	.await
}

/// `complete_upload` for `digest`.
pub async fn complete(app: &App, caller: As<'_>, org: &str, project: &str, digest: &str) -> Answer {
	app.call(
		"complete_upload",
		json!({ "organization": org, "project": project, "digest": digest }),
		caller,
	)
	.await
}

/// PUTs `body` to `ticket`'s address, and returns the store's status.
pub async fn put(app: &App, ticket: &Value, body: &[u8]) -> u16 {
	let (status, _) = app
		.presigned(
			ticket["method"].as_str().expect("a method"),
			ticket["url"].as_str().expect("an address"),
			&ticket["headers"],
			body.to_vec(),
		)
		.await;
	status
}

/// Uploads `bytes` through a ticket, unless they're stored already, and
/// returns their digest.
pub async fn upload(app: &App, caller: As<'_>, org: &str, project: &str, bytes: &[u8]) -> String {
	let began = begin(app, caller, org, project, bytes).await;
	let began = began.expect(200).json();
	let digest = digest_of(bytes);

	if began["status"] == "upload" {
		assert_eq!(put(app, &began["ticket"], bytes).await, 200);
		let completed = complete(app, caller, org, project, &digest).await;
		assert_eq!(completed.expect(200).json()["digest"], digest.as_str());
	} else {
		assert_eq!(began["status"], "stored", "{began}");
	}

	digest
}

/// The `event` tags of a deployment's events, in order.
fn event_kinds(events: &Value) -> Vec<String> {
	events
		.as_array()
		.expect("events")
		.iter()
		.map(|event| event["event"]["event"].as_str().expect("a kind").to_owned())
		.collect()
}

/// The `to` states of a deployment's transitions, in order.
fn transitions(events: &Value) -> Vec<String> {
	events
		.as_array()
		.expect("events")
		.iter()
		.filter(|event| event["event"]["event"] == "transition")
		.map(|event| {
			event["event"]["to"]["state"]
				.as_str()
				.expect("a state")
				.to_owned()
		})
		.collect()
}

/// A line of a deployment's sheet, by its kind.
fn line<'v>(sheet: &'v Value, kind: &str) -> &'v Value {
	sheet["lines"]
		.as_array()
		.expect("lines")
		.iter()
		.find(|line| line["kind"]["kind"] == kind)
		.unwrap_or_else(|| panic!("a `{kind}` line in {sheet}"))
}

/// Upload, deploy, build, migrate, release, and go live: the whole path,
/// with the deployment's events, sheet, and status as a client sees them,
/// then a second release superseding the first and a rollback to it.
#[expect(
	clippy::cognitive_complexity,
	clippy::too_many_lines,
	reason = "a scenario is one story, read top to bottom; its asserts are what count"
)]
pub async fn a_deployment_goes_live(app: &App) {
	let shop = Shop::open(app).await;
	let archive = b"the notes app, packed";
	let digest = shop.upload(archive).await;

	// Bytes the organisation already has aren't uploaded again.
	let again = begin(app, shop.as_ana(), &shop.org, "notes", archive).await;
	assert_eq!(again.expect(200).json()["status"], "stored");

	let deployed = shop
		.deploy("production", &digest, json!({ "database": "sqlite" }))
		.await;
	let deployed = deployed.expect(200).json();
	assert_eq!(deployed["number"], 1);
	assert_eq!(deployed["state"], json!({ "state": "queued" }));
	assert_eq!(deployed["signer"]["by"]["kind"], "person");
	// A browser names no host: only the CLI and tokens report one.
	assert_eq!(deployed["signer"]["via"], "web");
	assert_eq!(deployed["signer"]["host"], Value::Null);

	// One deployment in flight per environment.
	shop.deploy("production", &digest, json!({}))
		.await
		.refused(409, "deployment_in_flight");
	let in_flight = app
		.call(
			"in_flight",
			json!({ "organization": shop.org }),
			shop.as_ana(),
		)
		.await;
	let in_flight = in_flight.expect(200).json();
	assert_eq!(in_flight.as_array().map(Vec::len), Some(1));
	assert_eq!(in_flight[0]["environment"], "production");

	// The cluster has nothing yet; the build worker gets the build with an
	// address that downloads the archive.
	assert!(shop.cluster.claim().await.is_none());
	let build = shop.build.take().await;
	assert_eq!(build["attempt"], 1);
	let input = &build["input"];
	assert_eq!(input["deployment"]["number"], 1);
	assert_eq!(input["digest"], digest.as_str());
	assert!(
		input["image_repository"]
			.as_str()
			.is_some_and(|repository| repository.starts_with(super::REGISTRY)),
		"{input}"
	);
	let (status, downloaded) = app
		.presigned(
			"GET",
			input["archive"]["url"].as_str().expect("an address"),
			&json!({}),
			Vec::new(),
		)
		.await;
	assert_eq!((status, downloaded.as_slice()), (200, &archive[..]));
	assert_eq!(
		shop.state("production", 1).await,
		json!({ "state": "building" })
	);
	let renewed = shop.build.renew(&build).await.expect(200).json();
	assert_eq!(renewed["cancelled"], false);

	// Its logs, redacted.
	shop.build
		.log(
			&build,
			0,
			&[
				"Compiling notes v0.1.0",
				"curl -H 'Authorization: Bearer hunter2-token' https://example.com", // gitleaks:allow (fake token)
			],
		)
		.await
		.expect(200);
	let logs = shop.on("build_logs", "production", 1).await;
	let logs = logs.expect(200).json();
	let text: Vec<&str> = logs["lines"]
		.as_array()
		.expect("lines")
		.iter()
		.map(|line| line["text"].as_str().expect("text"))
		.collect();
	assert_eq!(text[0], "Compiling notes v0.1.0");
	assert!(!text[1].contains("hunter2"), "{}", text[1]);

	// The build finishes: the schema check passes, and the release waits
	// for the cluster.
	let outcome = built(&build, b"image one", &["0001_notes"]);
	let finished = shop.build.finish(&build, outcome.clone()).await;
	assert_eq!(finished.expect(200).json()["repeated"], false);
	let repeated = shop.build.finish(&build, outcome).await;
	assert_eq!(repeated.expect(200).json()["repeated"], true);
	shop.build
		.finish(
			&build,
			json!({ "kind": "build_server", "result": "failed", "reason": "changed my mind" }),
		)
		.await
		.refused(409, "outcome_conflict");
	assert_eq!(
		shop.state("production", 1).await,
		json!({ "state": "rolling_out" })
	);
	let sheet = shop.sheet("production", 1).await;
	assert_eq!(line(&sheet, "server")["stamp"], "built");
	assert_eq!(line(&sheet, "server")["steps"][0]["name"], "cargo build");
	let migrations = line(&sheet, "migrations");
	assert_eq!(migrations["stamp"], "working");
	assert!(
		migrations["detail"]
			.as_str()
			.is_some_and(|detail| detail.contains("as it starts")),
		"{migrations}"
	);
	let hostname = format!("notes-{}.{APPS_DOMAIN}", shop.org);
	assert_eq!(sheet["hostname"], hostname.as_str());

	// The cluster worker applies the spec and the deployment is live.
	let release = shop.cluster.take().await;
	let input = &release["input"];
	assert_eq!(input["kind"], "release");
	assert_eq!(input["migration"]["kind"], "at_startup");
	assert_eq!(input["migration"]["pending"], json!(["0001_notes"]));
	let spec = &input["spec"];
	assert_eq!(spec["deployment"], 1);
	assert_eq!(spec["processes"][0]["name"], "app");
	assert_eq!(spec["policy"]["runtime_class"], "gvisor");
	assert!(spec.to_string().contains(&hostname), "{spec}");
	shop.cluster.finish(&release, applied()).await.expect(200);
	let sheet = shop.sheet("production", 1).await;
	assert_eq!(sheet["deployment"]["state"], json!({ "state": "live" }));
	assert_eq!(line(&sheet, "server")["stamp"], "live");
	assert_eq!(line(&sheet, "migrations")["stamp"], "migrated");
	assert_eq!(sheet["spec"]["deployment"], 1);

	let events = shop.on("deployment_events", "production", 1).await;
	let events = events.expect(200).json();
	assert_eq!(
		transitions(&events),
		["building", "migrating", "rolling_out", "live"]
	);
	let kinds = event_kinds(&events);
	assert_eq!(kinds.first().map(String::as_str), Some("created"));
	assert!(kinds.iter().any(|kind| kind == "claimed"));
	let seqs: Vec<u64> = events
		.as_array()
		.expect("events")
		.iter()
		.map(|event| event["seq"].as_u64().expect("a seq"))
		.collect();
	assert!(seqs.windows(2).all(|pair| pair[0] < pair[1]), "{seqs:?}");
	assert!(
		app.call(
			"in_flight",
			json!({ "organization": shop.org }),
			shop.as_ana()
		)
		.await
		.json()
		.as_array()
		.is_some_and(Vec::is_empty)
	);

	// A second release supersedes the first; its migrations start from
	// where the database is.
	let second = shop
		.ship(
			"production",
			b"the notes app, again",
			&["0001_notes", "0002_tags"],
		)
		.await;
	assert_eq!(second, 2);
	assert_eq!(
		shop.state("production", 1).await,
		json!({ "state": "superseded" })
	);
	assert_eq!(shop.sheet("production", 1).await["superseded_by"], 2);

	// A preview says what the next release would do, with no prices.
	let preview = app
		.call(
			"preview_release",
			json!({
				"organization": shop.org,
				"project": "notes",
				"environment": "production",
			}),
			shop.as_ana(),
		)
		.await;
	let preview = preview.expect(200).json();
	assert_eq!(preview["number"], 3);
	assert_eq!(preview["hostname"], hostname.as_str());
	assert_eq!(preview["cost"]["kind"], "unpriced");
	assert!(
		preview["cost"]["reason"]
			.as_str()
			.is_some_and(|reason| reason.contains("price catalogue")),
		"{preview}"
	);

	// Rolling back to #1 releases its image again with no build, once the
	// database's migrations are checked: #1 doesn't know `0002_tags`.
	let rollback = |to: u64| {
		app.call(
			"rollback",
			json!({
				"organization": shop.org,
				"project": "notes",
				"environment": "production",
				"to": to,
			}),
			shop.as_ana(),
		)
	};
	rollback(1).await.refused(409, "rollback_unavailable");
	rollback(2).await.refused(409, "rollback_unavailable");
	let third = shop
		.ship(
			"production",
			b"the notes app, thrice",
			&["0001_notes", "0002_tags"],
		)
		.await;
	let rolled = rollback(2).await;
	let rolled = rolled.expect(200).json();
	assert_eq!(rolled["number"], third + 1);
	assert_eq!(rolled["rollback_of"], 2);
	assert_eq!(rolled["state"], json!({ "state": "rolling_out" }));
	assert!(
		shop.build.claim().await.is_none(),
		"a rollback builds nothing"
	);
	let release = shop.cluster.take().await;
	assert_eq!(release["input"]["migration"]["kind"], "none");
	assert_eq!(release["input"]["previous"], third);
	shop.cluster.finish(&release, applied()).await.expect(200);
	assert_eq!(
		shop.state("production", third + 1).await,
		json!({ "state": "live" })
	);

	// The list, newest first, with its pages.
	let listed = app
		.call(
			"deployments",
			json!({ "organization": shop.org, "project": "notes", "limit": 2 }),
			shop.as_ana(),
		)
		.await;
	let listed = listed.expect(200).json();
	let numbers: Vec<u64> = listed
		.as_array()
		.expect("a list")
		.iter()
		.map(|deployment| deployment["number"].as_u64().expect("a number"))
		.collect();
	assert_eq!(numbers, [third + 1, third]);
	let older = app
		.call(
			"deployments",
			json!({
				"organization": shop.org,
				"project": "notes",
				"before": listed[1]["id"],
			}),
			shop.as_ana(),
		)
		.await;
	assert_eq!(older.expect(200).json().as_array().map(Vec::len), Some(2));
}

/// What a cluster says it runs reaches `environment_status` and the
/// runtime logs, redacted and filtered; a cluster worker can't report on an
/// environment that isn't on its cluster.
pub async fn the_cluster_reports_what_it_runs(app: &App) {
	let shop = Shop::open(app).await;
	shop.ship("production", b"status app", &[]).await;
	let status = app
		.call(
			"environment_status",
			json!({ "organization": shop.org, "project": "notes", "environment": "production" }),
			shop.as_ana(),
		)
		.await;
	let status = status.expect(200).json();
	assert_eq!(status["cluster"], "eu-one");
	assert!(status["reported_at"].is_null());
	let environment = app
		.call(
			"environments",
			json!({ "organization": shop.org, "project": "notes" }),
			shop.as_ana(),
		)
		.await
		.json()[0]
		.clone();
	let environment_id = environment["id"].as_str().expect("an id");
	let observed = json!({
		"environment": {
			"organization": { "slug": shop.org, "id": uuid::Uuid::now_v7() },
			"project": { "slug": "notes", "id": uuid::Uuid::now_v7() },
			"environment": { "slug": "production", "id": environment_id },
		},
		"processes": [{
			"name": "app",
			"ready_replicas": 1,
			"desired_replicas": 1,
			"image": "registry.example.test/app@sha256:aa",
			"observed_restarts": 2,
			"state": "Ready",
		}],
	});
	let now = chrono::Utc::now().timestamp_millis();
	let reported = shop
		.cluster
		.call(
			"report_status",
			json!({
				"environment": environment_id,
				"observed": observed,
				"logs": [
					{ "process": "app", "pod": "app-1", "at_millis": now, "message": "listening on :8080" },
					{ "process": "app", "pod": "app-1", "at_millis": now, "message": "token lkc_0123456789ab_0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef" },
				],
			}),
		)
		.await;
	assert_eq!(reported.expect(200).json()["log_lines"], 2);
	let status = app
		.call(
			"environment_status",
			json!({ "organization": shop.org, "project": "notes", "environment": "production" }),
			shop.as_ana(),
		)
		.await;
	let status = status.expect(200).json();
	assert_eq!(status["processes"][0]["state"], json!({ "state": "ready" }));
	assert_eq!(status["processes"][0]["restarts"], 2);

	let runtime = |contains: Option<&str>| {
		app.call(
			"runtime_logs",
			json!({
				"organization": shop.org,
				"project": "notes",
				"environment": "production",
				"contains": contains,
			}),
			shop.as_ana(),
		)
	};
	let all = runtime(None).await;
	let all = all.expect(200).json();
	assert_eq!(all["lines"].as_array().map(Vec::len), Some(2));
	assert_eq!(all["retention_days"], 7);
	let leaked = all["lines"][1]["text"].as_str().expect("text");
	assert!(!leaked.contains("0123456789abcdef0123"), "{leaked}");
	let filtered = runtime(Some("listening")).await;
	let filtered = filtered.expect(200).json();
	assert_eq!(filtered["lines"].as_array().map(Vec::len), Some(1));
	assert_eq!(filtered["lines"][0]["pod"], "app-1");

	// The build worker isn't a cluster's, and another cluster's worker
	// doesn't run this environment.
	shop.build
		.call(
			"report_status",
			json!({ "environment": environment_id, "observed": observed }),
		)
		.await
		.refused(403, "cluster_worker_required");
	register_cluster(app, &shop.ops, "eu-two").await;
	let other = pair(
		app,
		&shop.ops,
		json!({ "role": "cluster", "cluster": "eu-two" }),
		"eu-two-worker",
	)
	.await;
	other
		.call(
			"report_status",
			json!({ "environment": environment_id, "observed": observed }),
		)
		.await
		.refused(404, "environment_not_found");
}

/// A secret an environment's process printed, in no shape the redactor
/// looks for.
const PRINTED: &str = "q7Zz44statusLeakValue";

/// A secret too short to hide anything by: under eight bytes.
const SHORT: &str = "pw-42";

/// The owner stores `value` as production's secret `name`.
async fn store_secret(shop: &Shop<'_>, name: &str, value: &str) -> Answer {
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
}

/// Production as its cluster's worker is told it is placed: its reference,
/// and its ID.
async fn placed(shop: &Shop<'_>) -> (Value, String) {
	let listed = shop
		.cluster
		.call("cluster_environments", json!({}))
		.await
		.expect(200)
		.json();
	let reference = &listed[0]["environment"];
	(
		reference.clone(),
		reference["environment"]["id"]
			.as_str()
			.expect("an ID")
			.to_owned(),
	)
}

/// A report of production from its cluster: its process `app` failed by
/// `reason`, and its pod wrote `lines`.
fn report(environment: &Value, id: &str, reason: &str, lines: [String; 2]) -> Value {
	let now = chrono::Utc::now().timestamp_millis();
	let [first, second] = lines;

	json!({
		"environment": id,
		"observed": {
			"environment": environment,
			"processes": [{
				"name": "app",
				"ready_replicas": 0,
				"desired_replicas": 1,
				"image": "registry.example.test/app@sha256:aa",
				"state": { "Failed": { "reason": reason } },
			}],
		},
		"logs": [
			{ "process": "app", "pod": "app-1", "at_millis": now, "message": first },
			{ "process": "app", "pod": "app-1", "at_millis": now + 1, "message": second },
		],
	})
}

/// `runtime_logs` of production for `caller`.
async fn runtime_logs(shop: &Shop<'_>, caller: As<'_>) -> Answer {
	shop.app
		.call(
			"runtime_logs",
			json!({
				"organization": shop.org,
				"project": "notes",
				"environment": "production",
			}),
			caller,
		)
		.await
}

/// A secret an environment's process prints is hidden from what its
/// cluster's worker reports, as the control plane's own secrets are: the
/// line and a failed process's reason are read back without the value, by
/// anyone who may view the project. A value too short to hide anything by
/// is read as printed.
pub async fn a_printed_secret_is_hidden_from_the_runtime_logs(app: &App) {
	let shop = Shop::open(app).await;
	shop.ship("production", b"a talkative app", &[]).await;

	// The owner stores two secrets; no answer carries either value.
	for (name, value) in [("PAYMENT_KEY", PRINTED), ("MODE", SHORT)] {
		let stored = store_secret(&shop, name, value).await;
		let stored = stored.expect(200);
		assert!(!stored.text.contains(value), "{}", stored.text);
	}

	// A process printed both, and failed saying one. The report is kept.
	let (environment, id) = placed(&shop).await;
	let reported = shop
		.cluster
		.call(
			"report_status",
			report(
				&environment,
				&id,
				&format!("it exited after reading {PRINTED}"),
				[format!("key {PRINTED} ok"), format!("mode {SHORT} ok")],
			),
		)
		.await;
	let reported = reported.expect(200);
	assert_eq!(reported.json()["log_lines"], 2, "{}", reported.text);

	// A viewer, a read-scoped token, and the owner all read the same lines:
	// the value hidden, the short one as printed, and each line otherwise
	// whole.
	let viewer = app
		.join(shop.as_ana(), &shop.org, "viewer@example.com", "viewer")
		.await;
	let (read, _) = app.token(shop.as_ana(), &shop.org, &["read"], None).await;

	for caller in [As::Session(&viewer), As::Bearer(&read), shop.as_ana()] {
		let logs = runtime_logs(&shop, caller).await;
		let logs = logs.expect(200);
		let lines = &logs.json()["lines"];
		assert_eq!(lines.as_array().map(Vec::len), Some(2), "{}", logs.text);
		assert_eq!(
			lines[0]["text"], "key [redacted] ok",
			"the value is read: {}",
			logs.text
		);
		assert_eq!(lines[0]["pod"], "app-1", "{}", logs.text);
		assert_eq!(
			lines[1]["text"],
			format!("mode {SHORT} ok"),
			"{}",
			logs.text
		);

		let status = shop
			.app
			.call(
				"environment_status",
				json!({
					"organization": shop.org,
					"project": "notes",
					"environment": "production",
				}),
				caller,
			)
			.await;
		let status = status.expect(200);
		assert_eq!(
			status.json()["processes"][0]["state"]["reason"],
			"it exited after reading [redacted]",
			"the value is read: {}",
			status.text
		);
	}
}

/// A cluster the environment isn't placed on is refused its report as
/// before, and no secret is opened for it. The organisation's data key is
/// spoiled first, so that opening one would fail the report another way:
/// the environment's own cluster's report then is, and nothing of it is
/// kept, rather than kept with the value as printed.
pub async fn a_report_from_another_cluster_opens_no_secret(app: &App) {
	let shop = Shop::open(app).await;
	shop.ship("production", b"a quiet app", &[]).await;
	store_secret(&shop, "PAYMENT_KEY", PRINTED)
		.await
		.expect(200);
	let (environment, id) = placed(&shop).await;

	// The organisation's key no longer opens, as after its master key is
	// lost: a secret of its environments can't be opened.
	let system = app.system();
	let key = DataKey::query()
		.one(&system)
		.await
		.expect("the organisation's data key");
	let mut wrapped = key.wrapped.clone();
	*wrapped.last_mut().expect("a wrapped key") ^= 0xff;
	DataKey::update()
		.filter(|row| row.id.eq(key.id))
		.set(|row| row.wrapped, wrapped)
		.run(&system)
		.await
		.expect("spoil the data key");

	// Another cluster's worker is refused as before, and would have been
	// refused another way had a secret been opened for its report.
	let lines = || [format!("key {PRINTED} ok"), "nothing else".to_owned()];
	register_cluster(app, &shop.ops, "eu-two").await;
	let other = pair(
		app,
		&shop.ops,
		json!({ "role": "cluster", "cluster": "eu-two" }),
		"eu-two-worker",
	)
	.await;
	other
		.call(
			"report_status",
			report(&environment, &id, "it crashed", lines()),
		)
		.await
		.refused(404, "environment_not_found");

	// The environment's own cluster's report is where the secrets are
	// opened: it is refused rather than kept as printed.
	shop.cluster
		.call(
			"report_status",
			report(&environment, &id, "it crashed", lines()),
		)
		.await
		.refused(500, "vault_unavailable");

	// Nothing of either report was kept.
	let status = app
		.call(
			"environment_status",
			json!({
				"organization": shop.org,
				"project": "notes",
				"environment": "production",
			}),
			shop.as_ana(),
		)
		.await;
	let status = status.expect(200);
	assert!(status.json()["reported_at"].is_null(), "{}", status.text);
	let logs = runtime_logs(&shop, shop.as_ana()).await;
	let logs = logs.expect(200);
	assert_eq!(
		logs.json()["lines"].as_array().map(Vec::len),
		Some(0),
		"{}",
		logs.text
	);
}

/// A build whose worker stops renewing is offered again, and after its
/// last attempt fails with `builder_unavailable`; the stale worker's late
/// finish is refused, and the environment takes a new deployment.
#[expect(
	clippy::cognitive_complexity,
	reason = "a scenario is one story, read top to bottom; its asserts are what count"
)]
pub async fn a_lapsed_build_is_retried_then_abandoned(app: &App) {
	let shop = Shop::open(app).await;
	let digest = shop.upload(b"a build that crashes").await;
	shop.deploy("production", &digest, json!({}))
		.await
		.expect(200);
	let first = shop.build.take().await;
	shop.lapse_leases().await;
	let swept = shop.sweep().await;
	assert_eq!(swept.retried, 1, "{swept:?}");
	assert_eq!(
		shop.state("production", 1).await,
		json!({ "state": "building" })
	);

	// The first attempt's worker lost its lease: its finish is refused.
	let second = shop.build.take().await;
	assert_eq!(second["attempt"], 2);
	shop.build
		.finish(&first, built(&first, b"img", &[]))
		.await
		.refused(404, "job_not_held");

	shop.lapse_leases().await;
	shop.sweep().await;
	let third = shop.build.take().await;
	assert_eq!(third["attempt"], 3);
	shop.lapse_leases().await;
	let swept = shop.sweep().await;
	assert_eq!(swept.abandoned, 1, "{swept:?}");
	assert_eq!(
		shop.state("production", 1).await,
		json!({ "state": "failed", "stage": "building", "blocker": "builder_unavailable" })
	);
	let events = shop.on("deployment_events", "production", 1).await;
	let kinds = event_kinds(&events.expect(200).json());
	assert_eq!(kinds.iter().filter(|kind| *kind == "retrying").count(), 2);
	assert!(shop.build.claim().await.is_none());

	// The environment is free again.
	shop.deploy("production", &digest, json!({}))
		.await
		.expect(200);

	// A build nobody claims fails the same way once it waited too long.
	Job::update()
		.filter(|job| job.state.eq(leptoskit_cloud::operator::JobState::Queued))
		.set(
			|job| job.queued_at,
			leptoskit_db::now() - chrono::TimeDelta::hours(2),
		)
		.run(&app.system())
		.await
		.expect("age the job");
	let swept = shop.sweep().await;
	assert_eq!(swept.abandoned, 1, "{swept:?}");
	assert_eq!(
		shop.state("production", 2).await,
		json!({ "state": "failed", "stage": "queued", "blocker": "builder_unavailable" })
	);
}

/// A release whose worker stops renewing isn't retried: its effect is
/// unknown, so the environment stays in flight until the worker's saved
/// finish arrives or an operator reconciles it from what the cluster runs.
#[expect(
	clippy::cognitive_complexity,
	reason = "a scenario is one story, read top to bottom; its asserts are what count"
)]
pub async fn a_lapsed_release_waits_for_reconciliation(app: &App) {
	let shop = Shop::open(app).await;

	// The worker comes back and finishes from its saved result.
	let digest = shop.upload(b"release one").await;
	shop.deploy("production", &digest, json!({}))
		.await
		.expect(200);
	let build = shop.build.take().await;
	shop.build
		.finish(&build, built(&build, b"one", &[]))
		.await
		.expect(200);
	let release = shop.cluster.take().await;
	shop.lapse_leases().await;
	let swept = shop.sweep().await;
	assert_eq!(swept.unknown, 1, "{swept:?}");
	let sheet = shop.sheet("production", 1).await;
	assert_eq!(sheet["deployment"]["unknown"], true);
	assert_eq!(
		sheet["deployment"]["state"],
		json!({ "state": "rolling_out" })
	);
	assert!(shop.cluster.claim().await.is_none(), "never replayed");
	shop.deploy("production", &digest, json!({}))
		.await
		.refused(409, "deployment_in_flight");
	shop.cluster.finish(&release, applied()).await.expect(200);
	let sheet = shop.sheet("production", 1).await;
	assert_eq!(sheet["deployment"]["state"], json!({ "state": "live" }));
	assert_eq!(sheet["deployment"]["unknown"], false);

	// An operator finds the release didn't apply.
	let digest = shop.upload(b"release two").await;
	shop.deploy("production", &digest, json!({}))
		.await
		.expect(200);
	let build = shop.build.take().await;
	shop.build
		.finish(&build, built(&build, b"two", &[]))
		.await
		.expect(200);
	let release = shop.cluster.take().await;
	shop.lapse_leases().await;
	shop.sweep().await;
	let unknown = app
		.call("jobs", json!({ "state": "unknown" }), shop.as_ops())
		.await;
	let unknown = unknown.expect(200).json();
	assert_eq!(unknown.as_array().map(Vec::len), Some(1));
	assert_eq!(unknown[0]["id"], release["job"]);
	app.call(
		"reconcile",
		json!({ "job": build["job"], "found": { "found": "applied" } }),
		shop.as_ops(),
	)
	.await
	.refused(409, "not_unknown");
	let reconciled = app
		.call(
			"reconcile",
			json!({
				"job": release["job"],
				"found": { "found": "not_applied", "detail": "the namespace still runs #1" },
			}),
			shop.as_ops(),
		)
		.await;
	assert_eq!(reconciled.expect(200).json()["state"], "done");
	assert_eq!(
		shop.state("production", 2).await,
		json!({ "state": "failed", "stage": "rolling_out", "blocker": "interrupted" })
	);
	assert_eq!(
		shop.state("production", 1).await,
		json!({ "state": "live" })
	);
	// The worker's late finish can't contradict the operator.
	shop.cluster
		.finish(&release, applied())
		.await
		.refused(409, "already_reconciled");
	shop.deploy("production", &digest, json!({}))
		.await
		.expect(200);
}

/// Cancelling a queued deployment stops it before any worker sees it; a
/// building one stops at the worker's next renewal.
pub async fn cancelling_stops_a_deployment(app: &App) {
	let shop = Shop::open(app).await;
	let digest = shop.upload(b"cancel me").await;
	shop.deploy("production", &digest, json!({}))
		.await
		.expect(200);
	let cancelled = shop.on("cancel_deployment", "production", 1).await;
	assert_eq!(
		cancelled.expect(200).json()["state"],
		json!({ "state": "cancelled" })
	);
	assert!(shop.build.claim().await.is_none());
	shop.on("cancel_deployment", "production", 1)
		.await
		.refused(409, "not_cancellable");

	shop.deploy("production", &digest, json!({}))
		.await
		.expect(200);
	let build = shop.build.take().await;
	shop.on("cancel_deployment", "production", 2)
		.await
		.expect(200);
	let renewed = shop.build.renew(&build).await.expect(200).json();
	assert_eq!(renewed["cancelled"], true);
	shop.build
		.finish(
			&build,
			json!({ "kind": "build_server", "result": "stopped" }),
		)
		.await
		.expect(200);
	assert_eq!(
		shop.state("production", 2).await,
		json!({ "state": "cancelled" })
	);

	// Nothing cancels a deployment that's live.
	let live = shop.ship("production", b"keep me", &[]).await;
	shop.on("cancel_deployment", "production", live)
		.await
		.refused(409, "not_cancellable");
}

/// The store holds what was promised before anything deploys it: the
/// size and digest are checked, a ticket can't be reused for other bytes,
/// and one organisation's upload tells another nothing.
#[expect(
	clippy::cognitive_complexity,
	reason = "a scenario is one story, read top to bottom; its asserts are what count"
)]
pub async fn uploads_are_verified_per_organization(app: &App) {
	let shop = Shop::open(app).await;
	let bytes = b"what was promised";

	// Completing before uploading finds nothing.
	let began = begin(app, shop.as_ana(), &shop.org, "notes", bytes).await;
	let ticket = began.expect(200).json()["ticket"].clone();
	complete(app, shop.as_ana(), &shop.org, "notes", &digest_of(bytes))
		.await
		.refused(404, "upload_missing");

	// The store refuses other bytes at the ticket's address.
	assert_ne!(put(app, &ticket, b"something else entirely").await, 200);
	assert_eq!(put(app, &ticket, bytes).await, 200);
	let completed = complete(app, shop.as_ana(), &shop.org, "notes", &digest_of(bytes)).await;
	assert_eq!(completed.expect(200).json()["size"], bytes.len());

	// Nothing can deploy what wasn't uploaded and verified.
	shop.deploy("production", &digest_of(b"never uploaded"), json!({}))
		.await
		.refused(404, "archive_not_found");
	shop.app
		.call(
			"deploy",
			json!({
				"organization": shop.org,
				"project": "notes",
				"environment": "production",
				"source": { "kind": "template", "name": "blog" },
			}),
			shop.as_ana(),
		)
		.await
		.refused(400, "source_unavailable");

	// Bob's organisation uploads the same bytes for itself, and can't
	// complete or deploy Ana's.
	let bob = app.sign_in("bob@example.com").await;
	let bob_org = app.personal(As::Session(&bob)).await;
	app.project(As::Session(&bob), &bob_org, "notes").await;
	let bobs = begin(app, As::Session(&bob), &bob_org, "notes", bytes).await;
	assert_eq!(bobs.expect(200).json()["status"], "upload");
	complete(app, As::Session(&bob), &bob_org, "notes", &digest_of(bytes))
		.await
		.refused(404, "upload_missing");
	app.call(
		"deploy",
		json!({
			"organization": bob_org,
			"project": "notes",
			"environment": "production",
			"source": { "kind": "upload", "archive": digest_of(bytes) },
		}),
		As::Session(&bob),
	)
	.await
	.refused(404, "archive_not_found");
	begin(app, As::Session(&bob), &shop.org, "notes", bytes)
		.await
		.refused(404, "organization_not_found");

	// Bounds.
	app.call(
		"begin_upload",
		json!({
			"organization": shop.org,
			"project": "notes",
			"digest": digest_of(b""),
			"size": 0,
		}),
		shop.as_ana(),
	)
	.await
	.refused(400, "archive_empty");
	app.call(
		"begin_upload",
		json!({
			"organization": shop.org,
			"project": "notes",
			"digest": digest_of(b"huge"),
			"size": leptoskit_cloud::deployments::MAX_ARCHIVE_BYTES + 1,
		}),
		shop.as_ana(),
	)
	.await
	.refused(400, "archive_too_large");

	// An upload nobody finishes expires, and its row and bytes go.
	let abandoned = b"begun and forgotten";
	let began = begin(app, shop.as_ana(), &shop.org, "notes", abandoned).await;
	let ticket = began.expect(200).json()["ticket"].clone();
	assert_eq!(put(app, &ticket, abandoned).await, 200);
	let abandoned_digest = digest_of(abandoned);
	SourceArchive::update()
		.filter(|archive| archive.digest.eq(abandoned_digest.as_str()))
		.set(
			|archive| archive.expires_at,
			leptoskit_db::now() - chrono::TimeDelta::seconds(1),
		)
		.run(&app.system())
		.await
		.expect("expire it");
	let swept = shop.sweep().await;
	assert_eq!(swept.archives, 1, "{swept:?}");
	complete(app, shop.as_ana(), &shop.org, "notes", &abandoned_digest)
		.await
		.refused(404, "upload_not_found");

	// The address a ticket gave is no good once it expires: the store's
	// signature covers the expiry.
	let url = ticket["url"].as_str().expect("an address");
	let tampered = url.replace("expires=", "expires=9");
	let (status, _) = app
		.presigned("PUT", &tampered, &ticket["headers"], abandoned.to_vec())
		.await;
	assert_eq!(status, 403);
}

/// With no cluster in the project's region, nothing is created: the
/// deploy says why, and can be tried again.
pub async fn deploying_needs_a_cluster(app: &App) {
	let ana = app.sign_in("ana@example.com").await;
	let org = app.personal(As::Session(&ana)).await;
	app.project(As::Session(&ana), &org, "notes").await;
	let digest = upload(app, As::Session(&ana), &org, "notes", b"homeless").await;
	let deploy = || {
		app.call(
			"deploy",
			json!({
				"organization": org,
				"project": "notes",
				"environment": "production",
				"source": { "kind": "upload", "archive": digest },
			}),
			As::Session(&ana),
		)
	};
	deploy().await.refused(503, "cluster_unavailable");
	let listed = app
		.call(
			"deployments",
			json!({ "organization": org, "project": "notes" }),
			As::Session(&ana),
		)
		.await;
	assert_eq!(listed.expect(200).json(), json!([]));

	let ops = operator(app).await;
	register_cluster(app, &ops, "eu-one").await;
	assert_eq!(deploy().await.expect(200).json()["number"], 1);
	let clusters = app.call("clusters", json!({}), As::Session(&ops)).await;
	assert_eq!(clusters.expect(200).json()[0]["environments"], 1);

	// A token reports the host it deployed from, shown as it said it.
	app.call(
		"create_environment",
		json!({ "organization": org, "project": "notes", "name": "staging", "kind": "staging" }),
		As::Session(&ana),
	)
	.await
	.expect(200);
	let (token, _) = app.token(As::Session(&ana), &org, &["deploy"], None).await;
	let by_token = app
		.call(
			"deploy",
			json!({
				"organization": org,
				"project": "notes",
				"environment": "staging",
				"source": { "kind": "upload", "archive": digest },
				"host": "ci-runner-7",
			}),
			As::Bearer(&token),
		)
		.await;
	let by_token = by_token.expect(200).json();
	assert_eq!(by_token["signer"]["by"]["kind"], "token");
	assert_eq!(by_token["signer"]["via"], "token");
	assert_eq!(by_token["signer"]["host"], "ci-runner-7");
}

/// A pairing code pairs one worker, once; a worker's key is shown once and
/// kept hashed; a revoked worker is nobody, and its job is settled.
#[expect(
	clippy::cognitive_complexity,
	reason = "a scenario is one story, read top to bottom; its asserts are what count"
)]
pub async fn workers_pair_once_and_can_be_revoked(app: &App) {
	let shop = Shop::open(app).await;
	let issued = app
		.call("issue_pairing", json!({ "role": "build" }), shop.as_ops())
		.await;
	let code = issued.expect(200).json()["code"]
		.as_str()
		.expect("a code")
		.to_owned();
	let pairing = json!({ "code": code, "name": "builder-2" });
	let paired = app.call("pair", pairing.clone(), As::Nobody).await;
	let paired = paired.expect(200).json();
	let key = paired["key"].as_str().expect("a key").to_owned();
	assert!(key.starts_with("lkw_"));
	app.call("pair", pairing, As::Nobody)
		.await
		.refused(404, "pairing_invalid");

	// Neither the key's secret nor the code is kept anywhere.
	let secret = key.rsplit('_').next().expect("a secret");
	assert_eq!(app.find_in_every_table(secret).await, Vec::<String>::new());
	assert_eq!(app.find_in_every_table(&code).await, Vec::<String>::new());

	// Workers do workers' things, people do people's.
	let worker = As::Bearer(&key);
	app.call("whoami", json!({}), worker)
		.await
		.refused(403, "not_for_workers");
	app.call("claim", json!({}), shop.as_ana())
		.await
		.refused(403, "worker_required");
	app.call("claim", json!({}), As::Nobody)
		.await
		.refused(401, "worker_key_required");
	app.call("clusters", json!({}), shop.as_ana())
		.await
		.refused(403, "operator_required");

	// A pairing must say what it pairs.
	app.call("issue_pairing", json!({ "role": "cluster" }), shop.as_ops())
		.await
		.refused(400, "invalid_pairing");

	// Revoking a worker that holds a build hands it to another.
	let digest = shop.upload(b"revoked mid-build").await;
	shop.deploy("production", &digest, json!({}))
		.await
		.expect(200);
	let build = shop.build.take().await;
	let revoked = app
		.call(
			"revoke_worker",
			json!({ "worker": shop.build.id }),
			shop.as_ops(),
		)
		.await;
	assert!(!revoked.expect(200).json()["revoked_at"].is_null());
	shop.build
		.renew(&build)
		.await
		.refused(401, "worker_key_required");
	let other = Worker {
		app,
		key,
		id: paired["worker"]["id"].as_str().expect("an id").to_owned(),
	};
	let retried = other.take().await;
	assert_eq!(retried["job"], build["job"]);
	assert_eq!(retried["attempt"], 2);

	let workers = app.call("workers", json!({}), shop.as_ops()).await;
	let workers = workers.expect(200).json();
	assert_eq!(workers.as_array().map(Vec::len), Some(3));
	assert!(!workers.to_string().contains("lkw_"), "{workers}");
}

/// Deleting an environment that ran queues its teardown for its cluster.
pub async fn deleting_an_environment_tears_it_down(app: &App) {
	let shop = Shop::open(app).await;
	app.call(
		"create_environment",
		json!({ "organization": shop.org, "project": "notes", "name": "staging", "kind": "staging" }),
		shop.as_ana(),
	)
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
	let destroy = shop.cluster.take().await;
	assert_eq!(destroy["input"]["kind"], "destroy_environment");
	assert_eq!(
		destroy["input"]["environment"]["environment"]["slug"],
		"staging"
	);
	shop.cluster
		.finish(
			&destroy,
			json!({ "kind": "destroy_environment", "result": "destroyed" }),
		)
		.await
		.expect(200);
	let clusters = app.call("clusters", json!({}), shop.as_ops()).await;
	assert_eq!(clusters.expect(200).json()[0]["environments"], 0);
}

/// Build logs come in order, a page at a time, from attempt to attempt;
/// a chunk can't be skipped or rewritten; and they expire.
pub async fn build_logs_are_ordered_paged_and_expire(app: &App) {
	let shop = Shop::open(app).await;
	let digest = shop.upload(b"chatty build").await;
	shop.deploy("production", &digest, json!({}))
		.await
		.expect(200);
	let build = shop.build.take().await;
	let lines: Vec<String> = (0..30).map(|n| format!("line {n}")).collect();
	let first: Vec<&str> = lines[..20].iter().map(String::as_str).collect();
	let second: Vec<&str> = lines[20..].iter().map(String::as_str).collect();
	shop.build.log(&build, 0, &first).await.expect(200);
	shop.build
		.log(&build, 2, &second)
		.await
		.refused(409, "log_out_of_order");
	// The same chunk again is fine; other lines under its number aren't.
	shop.build.log(&build, 0, &first).await.expect(200);
	shop.build
		.log(&build, 0, &second)
		.await
		.refused(409, "log_chunk_conflict");
	shop.build.log(&build, 1, &second).await.expect(200);
	let too_long = vec!["x"; leptoskit_cloud::workers::MAX_LOG_LINES + 1];
	shop.build
		.log(&build, 2, &too_long)
		.await
		.refused(400, "log_too_large");

	let page = |after: Option<u64>| {
		app.call(
			"build_logs",
			json!({
				"organization": shop.org,
				"project": "notes",
				"environment": "production",
				"number": 1,
				"after": after,
				"limit": 12,
			}),
			shop.as_ana(),
		)
	};
	let mut seen = Vec::new();
	let mut after = None;

	loop {
		let answer = page(after).await;
		let answer = answer.expect(200).json();

		for line in answer["lines"].as_array().expect("lines") {
			seen.push(line["text"].as_str().expect("text").to_owned());
			assert_eq!(line["attempt"], 1);
		}

		match answer["next"].as_u64() {
			Some(next) => after = Some(next),
			None => break,
		}
	}

	assert_eq!(seen, lines);

	// Old logs go.
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
	let empty = page(None).await;
	assert_eq!(empty.expect(200).json()["lines"], json!([]));
}

/// A build whose migrations don't start where the database is fails at
/// `migrating`, and the live release keeps serving.
pub async fn a_build_that_forgets_migrations_fails(app: &App) {
	let shop = Shop::open(app).await;
	shop.ship("production", b"v1", &["0001_notes", "0002_tags"])
		.await;
	let digest = shop.upload(b"v2 from an old branch").await;
	shop.deploy("production", &digest, json!({ "database": "sqlite" }))
		.await
		.expect(200);
	let build = shop.build.take().await;
	shop.build
		.finish(&build, built(&build, b"v2", &["0001_notes"]))
		.await
		.expect(200);
	assert_eq!(
		shop.state("production", 2).await,
		json!({ "state": "failed", "stage": "migrating", "blocker": "schema_incompatible" })
	);
	let sheet = shop.sheet("production", 2).await;
	let migrations = line(&sheet, "migrations");
	assert_eq!(migrations["stamp"], "failed");
	assert!(
		migrations["detail"]
			.as_str()
			.is_some_and(|detail| detail.contains("0002_tags")),
		"{migrations}"
	);
	assert!(shop.cluster.claim().await.is_none());
	assert_eq!(
		shop.state("production", 1).await,
		json!({ "state": "live" })
	);
}

/// Workers racing for one job: exactly one gets it. Deploys racing for one
/// environment: exactly one is accepted.
pub async fn races_have_one_winner(app: &App) {
	let shop = Shop::open(app).await;
	let mut builders = vec![
		pair(app, &shop.ops, json!({ "role": "build" }), "racer-1").await,
		pair(app, &shop.ops, json!({ "role": "build" }), "racer-2").await,
		pair(app, &shop.ops, json!({ "role": "build" }), "racer-3").await,
	];
	builders.push(shop.build);
	let digest = upload(app, As::Session(&shop.ana), &shop.org, "notes", b"race").await;
	let deploys = (0..4).map(|_| {
		app.call(
			"deploy",
			json!({
				"organization": shop.org,
				"project": "notes",
				"environment": "production",
				"source": { "kind": "upload", "archive": digest },
			}),
			As::Session(&shop.ana),
		)
	});
	let answers = futures::future::join_all(deploys).await;
	let accepted = answers.iter().filter(|answer| answer.status == 200).count();
	assert_eq!(accepted, 1, "{answers:?}");
	assert!(
		answers
			.iter()
			.filter(|answer| answer.status != 200)
			.all(|answer| answer.status == 409),
		"{answers:?}"
	);

	let claims = builders.iter().map(Worker::claim);
	let claimed: Vec<Option<Value>> = futures::future::join_all(claims).await;
	assert_eq!(claimed.iter().flatten().count(), 1, "{claimed:?}");
}
