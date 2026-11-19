//! Checks of the fanout and cancel engine that both databases run, against
//! a running service over a pool of one connection: that pool hands its
//! connection out in the order callers asked for it, which is what makes
//! the interleavings below deterministic.

use std::time::Duration;

use leptoskit_db::Db;
use leptoskit_db::Read as _;
use leptoskit_db::Statement;
use leptoskit_notifications::FanoutSend;
use leptoskit_notifications::SERVICE;
use leptoskit_notifications::SendProgress;
use leptoskit_notifications::SendState;
use leptoskit_rpc as rpc;
use leptoskit_rpc::server::jobs::JobRow;
use leptoskit_rpc::server::jobs::JobState;
use serde_json::Value;
use serde_json::json;
use uuid::Uuid;

const TOKEN: &str = "app-token";
const PREFIX: &str = "/_leptoskit/services/notifications";

/// `count` people in `group`'s own range of ids.
pub fn people(group: u128, count: usize) -> Vec<Uuid> {
	(0..count)
		.map(|n| {
			Uuid::from_u128(
				(group << 48)
					| 0x0E70_0000_0000_0000_0000_0000_0000_0000
					| u128::try_from(n).expect("small"),
			)
		})
		.collect()
}

/// Calls `operation` as the app.
pub async fn call(address: std::net::SocketAddr, operation: &str, args: Value) -> (u16, String) {
	let response = reqwest::Client::new()
		.post(format!("http://{address}{PREFIX}/rpc/{operation}"))
		.bearer_auth(TOKEN)
		.json(&args)
		.send()
		.await
		.unwrap();
	(response.status().as_u16(), response.text().await.unwrap())
}

/// Subscribes `people` to `topic`, a thousand at a time.
pub async fn subscribe(address: std::net::SocketAddr, topic: &str, people: &[Uuid]) {
	for chunk in people.chunks(1_000) {
		let (status, body) = call(
			address,
			"subscribe_to_topic",
			json!({ "topic": topic, "persons": chunk }),
		)
		.await;
		assert_eq!(status, 200, "{body}");
	}
}

/// Sends to everyone in `topic`, as `key`, and checks it was accepted.
pub async fn send_topic(address: std::net::SocketAddr, key: &str, topic: &str) {
	let (status, body) = call(
		address,
		"send",
		json!({
			"idempotency_key": key,
			"spec": { "category": "news", "title": "To a topic" },
			"target": { "topic": topic },
		}),
	)
	.await;
	assert_eq!(status, 200, "{body}");
}

/// The send's progress.
pub async fn progress(address: std::net::SocketAddr, key: &str) -> SendProgress {
	let (status, body) = call(address, "send_status", json!({ "idempotency_key": key })).await;
	assert_eq!(status, 200, "{body}");
	serde_json::from_str(&body).unwrap()
}

/// One number a query counts.
pub async fn count(db: &Db, sql: &str) -> i64 {
	let rows = db
		.fetch(Statement {
			sql: sql.into(),
			args: Vec::new(),
		})
		.await
		.expect("a read");

	match rows[0][0] {
		leptoskit_db::Value::Int(count) => count,
		ref other => panic!("not a count: {other:?}"),
	}
}

/// Runs one fanout step's body directly, as a worker would.
pub async fn run_step(db: &Db, send_key: &str, step: u32) {
	use leptoskit_rpc::Job as _;

	let cx = rpc::JobCx::new(
		db.system("run a fanout step in a test"),
		rpc::server::Identity::anonymous(),
		format!("test:{send_key}:{step}"),
		1,
	);
	FanoutSend {
		send_key: send_key.to_owned(),
		step,
	}
	.run(&cx)
	.await
	.expect("the step runs");
}

/// Runs every job that is due, one at a time, until none is.
pub async fn drain(db: &Db) {
	rpc::testing::TestJobs::new(db)
		.service(Some(SERVICE))
		.run_jobs_until_idle()
		.await;
}

/// The jobs still waiting to run.
async fn pending(db: &Db, name: &str) -> Vec<JobRow> {
	JobRow::query()
		.filter(|job| job.name.eq(name.to_owned()) & job.state.eq(JobState::Pending))
		.all(&db.system("read the pending jobs in a test"))
		.await
		.expect("the jobs read")
}

/// Records `job` done, as the worker that ran its body would.
async fn record_done(db: &Db, job: &JobRow) {
	let (jobs, placeholder) = match db.dialect() {
		leptoskit_db::Dialect::Postgres => ("\"rpc\".\"jobs\"", "$1"),
		leptoskit_db::Dialect::Sqlite => ("\"rpc_jobs\"", "?1"),
	};
	db.fetch(Statement {
		sql: format!(
			"UPDATE {jobs} SET \"state\" = 'done', \"locked_until\" = NULL WHERE \"key\" = \
			 {placeholder}"
		),
		args: vec![leptoskit_db::Value::Text(
			job.key.clone().expect("a keyed step"),
		)],
	})
	.await
	.expect("the job is recorded done");
}

/// A replayed step that commits the live step's batch first must not end
/// the fanout: the live step finds its batch taken, and the send still
/// completes with its whole audience once each.
///
/// Step 0 runs; the one job it leaves is the live chain's next step. Then
/// step 0 runs again beside that step, against the same stored cursor, as
/// a replay does when step 0's worker died after its commit but before its
/// job was recorded done. With the pool's connection held until both have
/// asked for it, the replay — first in line — reads first and commits
/// first, so the live step reads the cursor before the replay's commit and
/// writes after it. Its worker records its `Ok` as done; whatever jobs are
/// left then have to carry the fanout to its end.
pub async fn a_lost_batch_keeps_the_fanout_going(address: std::net::SocketAddr, db: &Db) {
	let audience = people(0xF4, 1_100);
	subscribe(address, "news.race", &audience).await;
	send_topic(address, "raced", "news.race").await;
	let first = pending(db, "fanout_send").await;
	assert_eq!(first.len(), 1, "the send enqueued its first step");
	run_step(db, "raced", 0).await;
	record_done(db, &first[0]).await;

	let live = pending(db, "fanout_send").await;
	assert_eq!(live.len(), 1, "step 0 leaves one next step");
	let live = live.into_iter().next().expect("one job");
	let step = u32::try_from(live.arguments.0["step"].as_u64().expect("a step")).expect("small");

	let hold = db.begin().await.expect("the connection is free");
	let ((), (), ()) = tokio::join!(
		Box::pin(run_step(db, "raced", 0)),
		Box::pin(run_step(db, "raced", step)),
		async {
			hold.rollback().await.expect("the hold lets go");
		}
	);

	record_done(db, &live).await;

	drain(db).await;

	let progress = progress(address, "raced").await;
	assert_eq!(
		progress.state,
		SendState::Complete,
		"the fanout reached its end: {progress:?}"
	);
	assert_eq!((progress.targeted, progress.delivered), (1_100, 1_100));
	assert_eq!(
		count(
			db,
			"SELECT count(DISTINCT person) FROM notifications WHERE send_key = 'raced'"
		)
		.await,
		1_100,
		"everyone once"
	);
	assert_eq!(
		count(
			db,
			"SELECT count(*) FROM notifications WHERE send_key = 'raced'"
		)
		.await,
		1_100,
		"nobody twice"
	);
}

/// A dry run over a large audience holds up no write: a write that asks
/// for the connection after the dry run started finishes while the dry run
/// is still walking, rather than after it.
pub async fn a_dry_run_holds_up_no_write(address: std::net::SocketAddr, db: &Db) {
	let audience = people(0xD5, 4_000);
	subscribe(address, "news.dry", &audience).await;

	let hold = db.begin().await.expect("the connection is free");
	let dry = tokio::spawn(async move {
		let (status, body) = call(
			address,
			"send",
			json!({
				"idempotency_key": "dry-and-large",
				"spec": { "category": "news", "title": "Just looking" },
				"target": { "topic": "news.dry" },
				"validate_only": true,
			}),
		)
		.await;
		(status, body, tokio::time::Instant::now())
	});
	// The dry run is in line for the connection first, then the write.
	tokio::time::sleep(Duration::from_millis(300)).await;
	let write = tokio::spawn(async move {
		let (status, body) = call(
			address,
			"subscribe_to_topic",
			json!({ "topic": "news.other", "persons": [Uuid::from_u128(0xD5)] }),
		)
		.await;
		(status, body, tokio::time::Instant::now())
	});
	tokio::time::sleep(Duration::from_millis(300)).await;
	hold.rollback().await.expect("the hold lets go");

	let (status, body, written) = write.await.expect("the write answers");
	assert_eq!(status, 200, "{body}");
	let (status, body, counted) = dry.await.expect("the dry run answers");
	assert_eq!(status, 200, "{body}");
	let result: Value = serde_json::from_str(&body).unwrap();
	assert_eq!(result["targeted"], 4_000, "{result}");
	assert!(
		written < counted,
		"the write finished while the dry run was still walking, not after it"
	);
}

/// Cancelling a fanout send with more than a batch of rows cancels them in
/// batches, each its own transaction, rather than in the cancel's own: the
/// cancel answers how many it stops, stops the fanout and its pushes at
/// once, and the rows follow.
pub async fn a_large_cancel_is_batched(address: std::net::SocketAddr, db: &Db) {
	let audience = people(0xCA, 1_100);
	subscribe(address, "news.big", &audience).await;
	send_topic(address, "big-cancel", "news.big").await;
	run_step(db, "big-cancel", 0).await;
	run_step(db, "big-cancel", 1).await;
	assert_eq!(
		count(
			db,
			"SELECT count(*) FROM notifications WHERE send_key = 'big-cancel'"
		)
		.await,
		1_000,
		"two batches written"
	);

	let (status, body) = call(
		address,
		"cancel",
		json!({ "idempotency_key": "big-cancel" }),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	assert_eq!(body, "1000", "it stops every row written so far");
	assert_eq!(
		progress(address, "big-cancel").await.state,
		SendState::Cancelled
	);
	assert_eq!(
		count(
			db,
			"SELECT count(*) FROM notifications WHERE send_key = 'big-cancel' AND cancelled_at IS \
			 NOT NULL"
		)
		.await,
		0,
		"the cancel's own transaction wrote no row"
	);
	assert_eq!(pending(db, "cancel_rows").await.len(), 1, "a job does");

	drain(db).await;

	assert_eq!(
		count(
			db,
			"SELECT count(*) FROM notifications WHERE send_key = 'big-cancel' AND cancelled_at IS \
			 NULL"
		)
		.await,
		0,
		"every row is cancelled"
	);
	assert_eq!(
		count(
			db,
			"SELECT count(*) FROM notifications WHERE send_key = 'big-cancel'"
		)
		.await,
		1_000,
		"and the fanout wrote nothing more"
	);

	let (status, body) = call(
		address,
		"cancel",
		json!({ "idempotency_key": "big-cancel" }),
	)
	.await;
	assert_eq!(
		(status, body.as_str()),
		(200, "0"),
		"asked again: nothing left"
	);
}
