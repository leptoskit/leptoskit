//! The durable fanout over audiences of thousands — ten and eleven
//! batches, not two or three — on SQLite and, behind `postgres`, on a
//! private Postgres. A topic send and an everyone send each reach every
//! person exactly once, at an audience size that is a multiple of the
//! batch (5,000) and at one that is not (5,237), with no one missed at a
//! batch boundary; a fanout step that runs twice, in a row or at the same
//! time, or replays after its bookkeeping was lost, doubles nothing; a
//! condition that matches about half walks both audiences and reaches
//! exactly the matching half; and a cancel part-way through cancels every
//! row written so far, stops the rest, and answers `cancelled`.
//!
//! The audiences are built with bulk `INSERT`s, not one RPC call per
//! person; the sends and the checks go through the service's own
//! operations, and the workers' job runs in-process on a test-fast poll.
//!
//! The Postgres halves skip without Postgres binaries, unless
//! `LEPTOSKIT_REQUIRE_POSTGRES` is set, as `tests/postgres.rs` does.

#![cfg(not(target_arch = "wasm32"))]

use std::time::Duration;

use leptoskit_db::Db;
use leptoskit_db::Dialect;
use leptoskit_db::Read as _;
use leptoskit_db::Statement;
use leptoskit_db::Value;
use leptoskit_notifications::FANOUT_BATCH;
use leptoskit_notifications::FanoutSend;
use leptoskit_notifications::SERVICE;
use leptoskit_notifications::SendProgress;
use leptoskit_notifications::SendResult;
use leptoskit_notifications::SendState;
use leptoskit_rpc as rpc;
use leptoskit_rpc::server::Anonymous;
use leptoskit_rpc::server::ServiceCallers;
use serde_json::Value as Json;
use serde_json::json;
use uuid::Uuid;

#[path = "support/sql.rs"]
mod sql;
use sql::Sql;

#[cfg(feature = "postgres")]
#[path = "support/postgres.rs"]
mod postgres;

const TOKEN: &str = "app-token";
const ORIGIN: &str = "http://app.test";
const PREFIX: &str = "/_leptoskit/services/notifications";

/// An audience that is not a multiple of the batch: ten full batches and a
/// batch of 237.
const UNEVEN: usize = 5_237;

/// An audience that is exactly ten batches.
const EVEN: usize = 5_000;

/// How long a fanout may take to walk its batches. Each step is a job on a
/// test-fast poll, but a loaded machine can be slower.
const SETTLES: Duration = Duration::from_secs(60);

struct Service {
	address: std::net::SocketAddr,
	db: Db,
	sql: Sql,
	jobs: Option<rpc::server::JobsHandle>,
}

/// A migrated service on a free port. With `workers`, its job runner is
/// started on a test-fast poll and a fanout walks itself; without, a
/// fanout holds still until the test runs its steps.
async fn start(db: Db, workers: bool) -> Service {
	let open_topics = leptoskit_notifications::service::config::OpenTopics::from_vars([(
		"LEPTOSKIT_NOTIFICATIONS_OPEN_TOPICS",
		"",
	)])
	.expect("the patterns are usable");
	let callers = ServiceCallers::new(Anonymous)
		.caller(TOKEN, rpc::server::APP_CALLER)
		.forwarded_identity(SERVICE, TOKEN);
	let (router, live) = leptoskit_notifications::service::routes_with_settings(
		db.clone(),
		Some(ORIGIN),
		callers,
		leptoskit_notifications::service::push::Push { runtime: None },
		open_topics,
	);
	live.start().await.expect("live starts");
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let address = listener.local_addr().unwrap();
	tokio::spawn(async move {
		leptoskit_axum::axum::serve(listener, router).await.unwrap();
	});
	let jobs = workers.then(|| {
		leptoskit_notifications::service::workers(db.clone())
			.poll(Duration::from_millis(50))
			.start()
	});

	Service {
		address,
		sql: Sql(db.dialect()),
		db,
		jobs,
	}
}

async fn on_sqlite(workers: bool) -> Service {
	start(
		leptoskit_db::testing::sqlite_migrated(leptoskit_notifications::MIGRATIONS).await,
		workers,
	)
	.await
}

impl Drop for Service {
	fn drop(&mut self) {
		if let Some(jobs) = self.jobs.take() {
			jobs.stop();
		}
	}
}

fn signed_in(person: Uuid) -> String {
	rpc::server::IdentityKey::new(SERVICE, TOKEN)
		.expect("a key")
		.mint(
			person,
			std::time::SystemTime::now(),
			rpc::server::IDENTITY_LIFETIME,
		)
}

/// `count` people, with ids that order the way their positions do, so the
/// person at position 500 is the first of the second batch.
fn people(count: usize) -> Vec<Uuid> {
	(0..count)
		.map(|n| {
			Uuid::from_u128(
				0x1A46_0000_0000_0000_0000_0000_0000_0000 | u128::try_from(n).expect("small"),
			)
		})
		.collect()
}

/// How many batches walking `audience` people takes.
fn batches(audience: usize) -> i64 {
	i64::try_from(audience.div_ceil(FANOUT_BATCH)).expect("small")
}

impl Service {
	async fn post(&self, operation: &str, args: Json, token: &str) -> (u16, String) {
		let response = reqwest::Client::new()
			.post(format!("http://{}{PREFIX}/rpc/{operation}", self.address))
			.json(&args)
			.bearer_auth(token)
			.send()
			.await
			.unwrap();

		(response.status().as_u16(), response.text().await.unwrap())
	}

	/// Sends as the app, checking it lands.
	async fn send(&self, key: &str, target: Json) -> SendResult {
		let (status, body) = self
			.post(
				"send",
				json!({
					"idempotency_key": key,
					"spec": { "category": "news", "title": "Hello, everyone" },
					"target": target,
				}),
				TOKEN,
			)
			.await;
		assert_eq!(status, 200, "{body}");

		serde_json::from_str(&body).unwrap()
	}

	/// What a send to `target` would reach right now, from a dry run.
	async fn would_reach(&self, target: Json) -> SendResult {
		let (status, body) = self
			.post(
				"send",
				json!({
					"idempotency_key": "dry-run",
					"spec": { "category": "news", "title": "Hello, everyone" },
					"target": target,
					"validate_only": true,
				}),
				TOKEN,
			)
			.await;
		assert_eq!(status, 200, "{body}");

		serde_json::from_str(&body).unwrap()
	}

	async fn status(&self, key: &str) -> SendProgress {
		let (status, body) = self
			.post("send_status", json!({ "idempotency_key": key }), TOKEN)
			.await;
		assert_eq!(status, 200, "{body}");

		serde_json::from_str(&body).unwrap()
	}

	/// `send_status` until the send leaves `fanning_out`, or panics.
	async fn settled(&self, key: &str) -> SendProgress {
		let deadline = tokio::time::Instant::now() + SETTLES;

		loop {
			let progress = self.status(key).await;

			if progress.state != SendState::FanningOut {
				return progress;
			}

			assert!(
				tokio::time::Instant::now() < deadline,
				"the fanout never settled: {progress:?}"
			);
			tokio::time::sleep(Duration::from_millis(25)).await;
		}
	}

	/// Runs one fanout step directly, as a worker would, and answers how
	/// it went.
	async fn try_step(&self, send_key: &str, step: u32) -> rpc::Result<()> {
		use leptoskit_rpc::Job as _;

		// The worker hands a job a handle that bypasses the policies, which
		// is what lets a key's second enqueue find the job under it.
		let cx = rpc::JobCx::new(
			self.db.system("a test running a fanout step"),
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
	}

	async fn run_step(&self, send_key: &str, step: u32) {
		self.try_step(send_key, step)
			.await
			.unwrap_or_else(|error| panic!("step {step} of `{send_key}` failed: {error}"));
	}

	async fn read(&self, sql: impl Into<String>) -> Vec<Vec<Value>> {
		let sql = sql.into();

		self.db
			.fetch(Statement {
				sql: sql.clone(),
				args: Vec::new(),
			})
			.await
			.unwrap_or_else(|error| panic!("{error}\n{sql}"))
	}

	async fn count(&self, sql: impl Into<String>) -> i64 {
		let sql = sql.into();

		match self.read(sql.clone()).await.first().and_then(|r| r.first()) {
			Some(Value::Int(count)) => *count,
			other => panic!("expected a count from {sql}, found {other:?}"),
		}
	}

	/// The table the job runner keeps its jobs in.
	fn jobs_table(&self) -> &'static str {
		match self.sql.0 {
			Dialect::Sqlite => "rpc_jobs",
			Dialect::Postgres => "\"rpc\".\"jobs\"",
		}
	}

	/// Inserts one row per person with a bulk `INSERT`, a few hundred rows
	/// to a statement. `row` makes one row's values from the person's
	/// position and literal.
	async fn insert(
		&self,
		table: &str,
		columns: &str,
		audience: &[Uuid],
		row: impl Fn(usize, String) -> Option<String>,
	) {
		let rows: Vec<String> = audience
			.iter()
			.enumerate()
			.filter_map(|(position, person)| row(position, self.sql.uuid(*person)))
			.collect();

		for chunk in rows.chunks(250) {
			self.read(format!(
				"INSERT INTO {table} ({columns}) VALUES {}",
				chunk.join(", ")
			))
			.await;
		}
	}

	/// Everyone in `audience` subscribed to `topic`, for the positions
	/// `which` accepts.
	async fn subscribe(&self, topic: &str, audience: &[Uuid], which: impl Fn(usize) -> bool) {
		let now = self.sql.timestamp(&leptoskit_db::now().to_rfc3339());

		self.insert(
			"topic_subscriptions",
			"id, person, topic, created_at",
			audience,
			|position, person| {
				which(position)
					.then(|| format!("({}, {person}, '{topic}', {now})", self.sql.new_id()))
			},
		)
		.await;
	}

	/// Everyone in `audience` in the registry.
	async fn register(&self, audience: &[Uuid]) {
		let now = self.sql.timestamp(&leptoskit_db::now().to_rfc3339());

		self.insert(
			"audience_members",
			"id, person, first_seen, updated_at",
			audience,
			|_, person| Some(format!("({}, {person}, {now}, {now})", self.sql.new_id())),
		)
		.await;
	}

	/// An attribute every person in `audience` has, valued by position.
	async fn attribute(&self, key: &str, audience: &[Uuid], value: impl Fn(usize) -> &'static str) {
		let now = self.sql.timestamp(&leptoskit_db::now().to_rfc3339());

		self.insert(
			"person_attributes",
			"id, person, key, value, created_at, updated_at",
			audience,
			|position, person| {
				Some(format!(
					"({}, {person}, '{key}', '{}', {now}, {now})",
					self.sql.new_id(),
					value(position)
				))
			},
		)
		.await;
	}

	/// Asserts the send `key` wrote exactly one row to each person
	/// `audience_sql` selects — as many rows as people, none twice, no
	/// one missed, no one else reached — and that its status is complete
	/// with the right totals, `expected` of them.
	async fn assert_reached_exactly(&self, key: &str, audience_sql: &str, expected: usize) {
		let expected = i64::try_from(expected).expect("small");
		let progress = self.status(key).await;

		assert_eq!(progress.state, SendState::Complete, "{key}: {progress:?}");
		assert_eq!(
			(
				i64::from(progress.targeted),
				i64::from(progress.delivered),
				progress.skipped_opted_out
			),
			(expected, expected, 0),
			"{key}'s totals: {progress:?}"
		);

		let rows = self
			.count(format!(
				"SELECT count(*) FROM notifications WHERE send_key = '{key}'"
			))
			.await;
		let distinct = self
			.count(format!(
				"SELECT count(DISTINCT person) FROM notifications WHERE send_key = '{key}'"
			))
			.await;
		assert_eq!(
			(rows, distinct),
			(expected, expected),
			"{key}: one row per person, nobody twice"
		);

		let missed = self
			.count(format!(
				"SELECT count(*) FROM ({audience_sql}) a WHERE NOT EXISTS (SELECT 1 FROM \
				 notifications n WHERE n.send_key = '{key}' AND n.person = a.person)"
			))
			.await;
		assert_eq!(
			missed, 0,
			"{key}: people the audience holds and the send missed"
		);

		let strays = self
			.count(format!(
				"SELECT count(*) FROM notifications WHERE send_key = '{key}' AND person NOT IN \
				 ({audience_sql})"
			))
			.await;
		assert_eq!(
			strays, 0,
			"{key}: people reached who aren't in the audience"
		);

		let live = self
			.count(format!(
				"SELECT count(*) FROM notifications WHERE send_key = '{key}' AND cancelled_at IS \
				 NOT NULL"
			))
			.await;
		assert_eq!(live, 0, "{key}: a completed send cancels nothing");
	}

	/// How many fanout steps and per-batch wakes the send enqueued: one of
	/// each per batch of the audience, and no step walking an empty page.
	async fn assert_walked_in_batches(&self, key: &str, audience: usize) {
		let steps = self
			.count(format!(
				"SELECT count(*) FROM {} WHERE key LIKE 'fanout:{key}:%'",
				self.jobs_table()
			))
			.await;
		let wakes = self
			.count(format!(
				"SELECT count(*) FROM {} WHERE key LIKE 'wake:{key}:fanout:%'",
				self.jobs_table()
			))
			.await;

		assert_eq!(
			(steps, wakes),
			(batches(audience), batches(audience)),
			"{key}: {audience} people walk in {} batches of {FANOUT_BATCH}",
			batches(audience)
		);
	}

	/// The people at `positions` open their inbox and find nothing.
	async fn assert_inbox_empty(&self, audience: &[Uuid], positions: &[usize]) {
		for &position in positions {
			let (_, body) = self
				.post(
					"inbox",
					json!({ "page": { "limit": 10 }, "show": "all" }),
					&signed_in(audience[position]),
				)
				.await;
			let page: Json = serde_json::from_str(&body).unwrap();

			assert_eq!(
				page["items"].as_array().map(Vec::len),
				Some(0),
				"person {position} sees nothing of a cancelled send: {page}"
			);
		}
	}

	/// The people at the edges of every batch — the last of one, the first
	/// of the next — and the very last each hold exactly one visible
	/// notification when they open their inbox.
	async fn assert_edges_see_one(&self, audience: &[Uuid]) {
		let mut edges: Vec<usize> = vec![0, audience.len() - 1];

		for batch in 1..audience.len().div_ceil(FANOUT_BATCH) {
			edges.push(batch * FANOUT_BATCH - 1);
			edges.push(batch * FANOUT_BATCH);
		}

		for position in edges {
			let (status, body) = self
				.post(
					"inbox",
					json!({ "page": { "limit": 10 }, "show": "all" }),
					&signed_in(audience[position]),
				)
				.await;
			assert_eq!(status, 200, "{body}");
			let page: Json = serde_json::from_str(&body).unwrap();

			assert_eq!(
				page["items"].as_array().map(Vec::len),
				Some(1),
				"person {position} of {} sees their one notification: {page}",
				audience.len()
			);
		}
	}
}

/// A topic send to `count` subscribers reaches each exactly once, however
/// the batches fall.
async fn topic_send_reaches_each_once(service: Service, count: usize) {
	let audience = people(count);
	service.subscribe("news.all", &audience, |_| true).await;

	let sent = service
		.send("everyone-who-asked", json!({ "topic": "news.all" }))
		.await;
	assert_eq!(sent.state, SendState::FanningOut, "{sent:?}");
	assert_eq!((sent.delivered, sent.targeted), (0, 0), "nothing yet");

	let progress = service.settled("everyone-who-asked").await;
	assert_eq!(progress.state, SendState::Complete, "{progress:?}");

	service
		.assert_reached_exactly(
			"everyone-who-asked",
			"SELECT person FROM topic_subscriptions WHERE topic = 'news.all'",
			count,
		)
		.await;
	service
		.assert_walked_in_batches("everyone-who-asked", count)
		.await;
	service.assert_edges_see_one(&audience).await;
}

/// An everyone send to `count` registered people reaches each exactly
/// once — and not the person a recipients send once named, who is not in
/// the registry.
async fn everyone_send_reaches_each_once(service: Service, count: usize) {
	let audience = people(count);
	service.register(&audience).await;

	// Someone a send named but no act ever registered.
	let stranger = Uuid::from_u128(0x57A4_0000_0000_0000_0000_0000_0000_0001);
	let named = service
		.send("only-the-stranger", json!({ "recipients": [stranger] }))
		.await;
	assert_eq!((named.delivered, named.state), (1, SendState::Complete));

	let dry = service.would_reach(json!("everyone")).await;
	assert_eq!(
		(dry.targeted, dry.delivered, dry.skipped_opted_out),
		(
			u32::try_from(count).unwrap(),
			u32::try_from(count).unwrap(),
			0
		),
		"a dry run walks the whole registry and counts it"
	);
	assert_eq!(
		service.count("SELECT count(*) FROM notifications").await,
		1,
		"a dry run writes nothing"
	);

	let sent = service.send("to-everyone", json!("everyone")).await;
	assert_eq!(sent.state, SendState::FanningOut, "{sent:?}");

	service.settled("to-everyone").await;
	service
		.assert_reached_exactly("to-everyone", "SELECT person FROM audience_members", count)
		.await;
	service.assert_walked_in_batches("to-everyone", count).await;
	service.assert_edges_see_one(&audience).await;

	let reached = service
		.count(format!(
			"SELECT count(*) FROM notifications WHERE send_key = 'to-everyone' AND person = {}",
			service.sql.uuid(stranger)
		))
		.await;
	assert_eq!(reached, 0, "the stranger isn't in the registry");
}

/// A fanout step that runs twice for the same send and step — a duplicated
/// or retried job — doubles no row, no count, and no job.
async fn a_duplicated_step_doubles_nothing(service: Service) {
	let audience = people(UNEVEN);
	service.subscribe("news.all", &audience, |_| true).await;
	service.send("twice", json!({ "topic": "news.all" })).await;

	service.run_step("twice", 0).await;
	let after_one = service
		.count("SELECT count(*) FROM notifications WHERE send_key = 'twice'")
		.await;
	assert_eq!(after_one, 500, "a step delivers one batch");

	// The same step again, as a retried job would run it.
	service.run_step("twice", 0).await;

	let rows = service
		.count("SELECT count(*) FROM notifications WHERE send_key = 'twice'")
		.await;
	let distinct = service
		.count("SELECT count(DISTINCT person) FROM notifications WHERE send_key = 'twice'")
		.await;
	assert_eq!(rows, distinct, "no one holds two rows");

	let progress = service.status("twice").await;
	assert_eq!(
		(i64::from(progress.delivered), i64::from(progress.targeted)),
		(rows, rows),
		"the counts are the rows: {progress:?}"
	);

	let (successors, wakes) = (
		service
			.count(format!(
				"SELECT count(*) FROM {} WHERE key = 'fanout:twice:1'",
				service.jobs_table()
			))
			.await,
		service
			.count(format!(
				"SELECT count(*) FROM {} WHERE key = 'wake:twice:fanout:0'",
				service.jobs_table()
			))
			.await,
	);
	assert_eq!(
		(successors, wakes),
		(1, 1),
		"one successor and one wake per step key, run twice or not"
	);

	// Walk the rest: the audience still ends complete, once each.
	for step in 1..20 {
		if service.status("twice").await.state != SendState::FanningOut {
			break;
		}

		service.run_step("twice", step).await;
	}

	service
		.assert_reached_exactly(
			"twice",
			"SELECT person FROM topic_subscriptions WHERE topic = 'news.all'",
			UNEVEN,
		)
		.await;
}

/// A step's bookkeeping lost — the cursor back at the start, the rows
/// already written — replays the batches already delivered: each person
/// is skipped, no count moves, and the walk still completes with every
/// person exactly once.
async fn a_replayed_walk_doubles_nothing(service: Service) {
	let audience = people(UNEVEN);
	service.subscribe("news.all", &audience, |_| true).await;
	service
		.send("replayed", json!({ "topic": "news.all" }))
		.await;

	for step in 0..4 {
		service.run_step("replayed", step).await;
	}

	assert_eq!(
		service
			.count("SELECT count(*) FROM notifications WHERE send_key = 'replayed'")
			.await,
		2_000
	);

	// As a crash after the rows committed and before the cursor did would
	// leave it.
	service
		.read("UPDATE sends SET \"cursor\" = NULL WHERE idempotency_key = 'replayed'")
		.await;

	for step in 100..120 {
		if service.status("replayed").await.state != SendState::FanningOut {
			break;
		}

		service.run_step("replayed", step).await;
	}

	service
		.assert_reached_exactly(
			"replayed",
			"SELECT person FROM topic_subscriptions WHERE topic = 'news.all'",
			UNEVEN,
		)
		.await;
}

/// Two copies of the same step running at the same moment. A job
/// framework can hand a step to two workers when a lease expires; one of
/// them may fail and be retried, but between them no person is written
/// twice and the walk completes exactly.
async fn concurrent_duplicate_steps_never_double_anyone(service: Service) {
	let audience = people(2_300);
	service.subscribe("news.all", &audience, |_| true).await;
	service.send("racing", json!({ "topic": "news.all" })).await;

	for step in 0..30 {
		if service.status("racing").await.state != SendState::FanningOut {
			break;
		}

		// Either copy may lose the race and fail; the job framework
		// would retry it. What may not happen is a double write.
		let (..) = tokio::join!(
			service.try_step("racing", step),
			service.try_step("racing", step)
		);

		let rows = service
			.count("SELECT count(*) FROM notifications WHERE send_key = 'racing'")
			.await;
		let distinct = service
			.count("SELECT count(DISTINCT person) FROM notifications WHERE send_key = 'racing'")
			.await;
		assert_eq!(
			rows, distinct,
			"after the pair at step {step}: nobody twice"
		);
	}

	service
		.assert_reached_exactly(
			"racing",
			"SELECT person FROM topic_subscriptions WHERE topic = 'news.all'",
			2_300,
		)
		.await;
}

/// Conditions that match about half the audience reach exactly that half:
/// one anchored on an attribute walks the registry, two anchored on
/// topics walk the topic index and split the audience between them.
async fn conditions_reach_the_matching_half(service: Service) {
	let audience = people(UNEVEN);
	let pros = UNEVEN.div_ceil(2);
	service.register(&audience).await;
	service.subscribe("news.all", &audience, |_| true).await;
	service
		.subscribe("vip", &audience, |position| position % 2 == 0)
		.await;
	service
		.attribute("plan", &audience, |position| {
			if position % 2 == 0 { "pro" } else { "free" }
		})
		.await;

	let dry = service
		.would_reach(json!({ "condition": "user.plan == 'pro'" }))
		.await;
	assert_eq!(
		usize::try_from(dry.targeted).unwrap(),
		pros,
		"a dry run counts the matching half"
	);

	service
		.send("by-attribute", json!({ "condition": "user.plan == 'pro'" }))
		.await;
	service
		.send(
			"by-topics",
			json!({ "condition": "'news.all' in topics && 'vip' in topics" }),
		)
		.await;
	service
		.send(
			"by-absence",
			json!({ "condition": "'news.all' in topics && !('vip' in topics)" }),
		)
		.await;

	for key in ["by-attribute", "by-topics", "by-absence"] {
		service.settled(key).await;
	}

	service
		.assert_reached_exactly(
			"by-attribute",
			"SELECT person FROM person_attributes WHERE key = 'plan' AND value = 'pro'",
			pros,
		)
		.await;
	service
		.assert_reached_exactly(
			"by-topics",
			"SELECT person FROM topic_subscriptions WHERE topic = 'vip'",
			pros,
		)
		.await;
	service
		.assert_reached_exactly(
			"by-absence",
			"SELECT s.person FROM topic_subscriptions s WHERE s.topic = 'news.all' AND NOT EXISTS \
			 (SELECT 1 FROM topic_subscriptions v WHERE v.topic = 'vip' AND v.person = s.person)",
			UNEVEN - pros,
		)
		.await;
	service
		.assert_walked_in_batches("by-attribute", UNEVEN)
		.await;
	service.assert_walked_in_batches("by-topics", UNEVEN).await;
}

/// `cancel` part-way through a large fanout, as documented: the rows
/// written so far are cancelled — all of them, however many batches that
/// is, though they are already visible — the next steps write nothing, the
/// send answers cancelled from then on with the counts it had reached, and
/// cancelling again answers `0`.
async fn cancelling_mid_fanout_cancels_every_row_written(service: Service) {
	let audience = people(UNEVEN);
	service.subscribe("news.all", &audience, |_| true).await;
	service
		.send("stopped", json!({ "topic": "news.all" }))
		.await;

	for step in 0..4 {
		service.run_step("stopped", step).await;
	}

	let written = service
		.count("SELECT count(*) FROM notifications WHERE send_key = 'stopped'")
		.await;
	assert_eq!(written, 2_000, "four batches delivered");
	assert_eq!(
		service.status("stopped").await.state,
		SendState::FanningOut,
		"the audience is only part-way walked"
	);

	let (status, body) = service
		.post("cancel", json!({ "idempotency_key": "stopped" }), TOKEN)
		.await;
	assert_eq!(status, 200, "a fanning send cancels: {body}");
	assert_eq!(body, "2000", "every row written so far was cancelled");

	assert_eq!(
		service
			.count(
				"SELECT count(*) FROM notifications WHERE send_key = 'stopped' AND cancelled_at \
				 IS NULL"
			)
			.await,
		0,
		"no row of the send is left standing"
	);

	// The rest of the walk, as the workers would have gone on to run it.
	for step in 4..16 {
		service.run_step("stopped", step).await;
	}

	assert_eq!(
		service
			.count("SELECT count(*) FROM notifications WHERE send_key = 'stopped'")
			.await,
		2_000,
		"the cancelled fanout wrote nothing more"
	);

	let progress = service.status("stopped").await;
	assert_eq!(progress.state, SendState::Cancelled, "{progress:?}");
	assert_eq!(
		(
			progress.targeted,
			progress.delivered,
			progress.skipped_opted_out
		),
		(2_000, 2_000, 0),
		"the counts are what the fanout had reached when it was cancelled"
	);

	// A person the fanout reached sees nothing, and neither does one it
	// never reached.
	service
		.assert_inbox_empty(&audience, &[0, 1_999, 2_000, UNEVEN - 1])
		.await;

	let (status, body) = service
		.post("cancel", json!({ "idempotency_key": "stopped" }), TOKEN)
		.await;
	assert_eq!((status, body.as_str()), (200, "0"), "cancelling again");
}

#[tokio::test(flavor = "multi_thread")]
async fn sqlite_a_topic_send_to_5237_people_reaches_each_once() {
	topic_send_reaches_each_once(on_sqlite(true).await, UNEVEN).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn sqlite_a_topic_send_to_exactly_5000_people_reaches_each_once() {
	topic_send_reaches_each_once(on_sqlite(true).await, EVEN).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn sqlite_an_everyone_send_to_5237_people_reaches_each_once() {
	everyone_send_reaches_each_once(on_sqlite(true).await, UNEVEN).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn sqlite_an_everyone_send_to_exactly_5000_people_reaches_each_once() {
	everyone_send_reaches_each_once(on_sqlite(true).await, EVEN).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn sqlite_a_duplicated_fanout_step_doubles_nothing() {
	a_duplicated_step_doubles_nothing(on_sqlite(false).await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn sqlite_a_replayed_walk_doubles_nothing() {
	a_replayed_walk_doubles_nothing(on_sqlite(false).await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn sqlite_concurrent_duplicate_steps_never_double_anyone() {
	concurrent_duplicate_steps_never_double_anyone(on_sqlite(false).await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn sqlite_conditions_over_5237_people_reach_the_matching_half() {
	conditions_reach_the_matching_half(on_sqlite(true).await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn sqlite_cancelling_mid_fanout_cancels_every_row_written() {
	cancelling_mid_fanout_cancels_every_row_written(on_sqlite(false).await).await;
}

#[cfg(feature = "postgres")]
mod on_postgres {
	use super::*;
	use crate::postgres::Postgres;

	async fn on_postgres(postgres: &Postgres, workers: bool) -> Service {
		let db = Db::connect(&postgres.database())
			.await
			.expect("the database opens");
		db.migrate(leptoskit_notifications::POSTGRES_MIGRATIONS)
			.await
			.expect("migrations apply");

		start(db, workers).await
	}

	#[tokio::test(flavor = "multi_thread")]
	async fn postgres_a_topic_send_to_5237_people_reaches_each_once() {
		let Some(postgres) = Postgres::start() else {
			return;
		};

		topic_send_reaches_each_once(on_postgres(&postgres, true).await, UNEVEN).await;
	}

	#[tokio::test(flavor = "multi_thread")]
	async fn postgres_an_everyone_send_to_exactly_5000_people_reaches_each_once() {
		let Some(postgres) = Postgres::start() else {
			return;
		};

		everyone_send_reaches_each_once(on_postgres(&postgres, true).await, EVEN).await;
	}

	#[tokio::test(flavor = "multi_thread")]
	async fn postgres_a_duplicated_fanout_step_doubles_nothing() {
		let Some(postgres) = Postgres::start() else {
			return;
		};

		a_duplicated_step_doubles_nothing(on_postgres(&postgres, false).await).await;
	}

	#[tokio::test(flavor = "multi_thread")]
	async fn postgres_concurrent_duplicate_steps_never_double_anyone() {
		let Some(postgres) = Postgres::start() else {
			return;
		};

		concurrent_duplicate_steps_never_double_anyone(on_postgres(&postgres, false).await).await;
	}

	#[tokio::test(flavor = "multi_thread")]
	async fn postgres_conditions_over_5237_people_reach_the_matching_half() {
		let Some(postgres) = Postgres::start() else {
			return;
		};

		conditions_reach_the_matching_half(on_postgres(&postgres, true).await).await;
	}

	#[tokio::test(flavor = "multi_thread")]
	async fn postgres_cancelling_mid_fanout_cancels_every_row_written() {
		let Some(postgres) = Postgres::start() else {
			return;
		};

		cancelling_mid_fanout_cancels_every_row_written(on_postgres(&postgres, false).await).await;
	}
}
