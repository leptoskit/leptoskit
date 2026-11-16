//! Topics, conditions, and the durable fanout, end to end on a real
//! port: a send to more subscribers than one batch holds reaches exactly
//! the matching people once; a condition's grammar decides who; a
//! replayed fanout writes nothing twice; a cancel mid-fanout stops the
//! rest; opt-outs are skipped and counted; a scheduled topic send hides
//! until its time; self-subscription is open only to the topics the
//! operator opened; one person never sees or changes another's topics; a
//! dry run counts without writing; and a send answers the same for the
//! same key and intent, however often it is asked.

#![cfg(not(target_arch = "wasm32"))]

use std::time::Duration;

use leptoskit_db::Db;
use leptoskit_db::Read as _;
use leptoskit_db::Statement;
use leptoskit_notifications::FanoutSend;
use leptoskit_notifications::SERVICE;
use leptoskit_notifications::SendProgress;
use leptoskit_notifications::SendResult;
use leptoskit_notifications::SendState;
use leptoskit_notifications::TopicChanges;
use leptoskit_rpc as rpc;
use leptoskit_rpc::server::Anonymous;
use leptoskit_rpc::server::ServiceCallers;
use serde_json::Value;
use serde_json::json;
use uuid::Uuid;

const TOKEN: &str = "app-token";
const ORIGIN: &str = "http://app.test";
const PREFIX: &str = "/_leptoskit/services/notifications";

const ALICE: Uuid = Uuid::from_u128(0xA110_0000_0000_0000_0000_0000_0000_0001);
const BOB: Uuid = Uuid::from_u128(0xB0B0_0000_0000_0000_0000_0000_0000_0002);

/// How long a fanout may take to walk its batches: each step is a job on
/// a test-fast poll, but a loaded machine can be slower.
const SETTLES: Duration = Duration::from_secs(20);

struct Service {
	address: std::net::SocketAddr,
	db: Db,
	jobs: Option<rpc::server::JobsHandle>,
}

/// A fresh, migrated service on a free port, with `open` declaring which
/// topics self-subscription may reach, and its workers on a test-fast
/// poll so a fanout walks its batches promptly.
async fn start(db: Db, open: &str) -> Service {
	let open_topics = leptoskit_notifications::service::config::OpenTopics::from_vars([(
		"LEPTOSKIT_NOTIFICATIONS_OPEN_TOPICS",
		open,
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
	let jobs = leptoskit_notifications::service::workers(db.clone())
		.poll(Duration::from_millis(50))
		.start();
	Service {
		address,
		db,
		jobs: Some(jobs),
	}
}

impl Service {
	/// Stops the workers, so a fanout holds still between steps and a
	/// test can drive them itself.
	async fn stop_jobs(&mut self) {
		if let Some(jobs) = self.jobs.take() {
			jobs.shutdown().await;
		}
	}

	/// Runs one fanout step directly, as the worker would.
	async fn run_step(&self, send_key: &str, step: u32) {
		use leptoskit_rpc::Job as _;

		let cx = rpc::JobCx::new(
			self.db.clone(),
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
}

impl Drop for Service {
	fn drop(&mut self) {
		if let Some(jobs) = self.jobs.take() {
			jobs.stop();
		}
	}
}

async fn service() -> Service {
	start(
		leptoskit_db::testing::sqlite_migrated(leptoskit_notifications::MIGRATIONS).await,
		"",
	)
	.await
}

/// [`service`], with no workers started: a fanout holds still between
/// steps, and the test drives them itself.
async fn idle() -> Service {
	let mut service = service().await;
	service.stop_jobs().await;
	service
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

async fn post(
	service: &Service,
	operation: &str,
	args: Value,
	token: Option<&str>,
) -> (u16, String) {
	let mut request = reqwest::Client::new()
		.post(format!(
			"http://{}{PREFIX}/rpc/{operation}",
			service.address
		))
		.json(&args);

	if let Some(token) = token {
		request = request.bearer_auth(token);
	}

	let response = request.send().await.unwrap();
	(response.status().as_u16(), response.text().await.unwrap())
}

async fn subscribe(service: &Service, topic: &str, persons: &[Uuid]) -> TopicChanges {
	let (status, body) = post(
		service,
		"subscribe_to_topic",
		json!({ "topic": topic, "persons": persons }),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	serde_json::from_str(&body).unwrap()
}

/// Sends to `target` as the app, checking it lands. The category is
/// `news`.
async fn send(service: &Service, key: &str, target: Value) -> SendResult {
	let (status, body) = post(
		service,
		"send",
		json!({
			"idempotency_key": key,
			"spec": { "category": "news", "title": "Hello topics" },
			"target": target,
		}),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	serde_json::from_str(&body).unwrap()
}

/// `send_status` until the send leaves `fanning_out`, or panics.
async fn settled(service: &Service, key: &str) -> SendProgress {
	let deadline = tokio::time::Instant::now() + SETTLES;

	loop {
		let (status, body) = post(
			service,
			"send_status",
			json!({ "idempotency_key": key }),
			Some(TOKEN),
		)
		.await;
		assert_eq!(status, 200, "{body}");
		let progress: SendProgress = serde_json::from_str(&body).unwrap();

		if progress.state != SendState::FanningOut {
			return progress;
		}

		assert!(
			tokio::time::Instant::now() < deadline,
			"the fanout never settled: {progress:?}"
		);
		tokio::time::sleep(Duration::from_millis(50)).await;
	}
}

/// A person's inbox page, parsed.
async fn inbox(service: &Service, token: &str) -> Value {
	let (_, body) = post(
		service,
		"inbox",
		json!({ "page": { "limit": 10 }, "show": "all" }),
		Some(token),
	)
	.await;
	serde_json::from_str(&body).unwrap_or(Value::Null)
}

async fn rows(service: &Service, sql: &str) -> Vec<Vec<leptoskit_db::Value>> {
	service
		.db
		.fetch(Statement {
			sql: sql.into(),
			args: Vec::new(),
		})
		.await
		.expect("a read")
}

fn text(value: &leptoskit_db::Value) -> String {
	match value {
		leptoskit_db::Value::Text(text) => text.clone(),
		leptoskit_db::Value::Int(int) => int.to_string(),
		other => format!("{other:?}"),
	}
}

/// `count` people the app knows, as distinct ids in `group`'s range.
fn group(group: u128, count: usize) -> Vec<Uuid> {
	(0..count)
		.map(move |n| {
			Uuid::from_u128(
				(group << 48)
					| 0x0F70_0000_0000_0000_0000_0000_0000_0000
					| u128::try_from(n).expect("small"),
			)
		})
		.collect()
}

/// A send to 1,201 subscribers — more than two batches — reaches exactly
/// the subscribers, once each, and its status says so.
#[tokio::test(flavor = "multi_thread")]
async fn a_topic_send_reaches_more_than_one_batch_exactly_once() {
	let service = service().await;
	let audience = group(0, 1_201);

	// The batch call is bounded at 1,000, the way FCM's is.
	let changes = subscribe(&service, "news.all", &audience[..1_000]).await;
	assert_eq!(
		(changes.subscribed, changes.skipped_full),
		(1_000, 0),
		"the first batch subscribes"
	);
	let changes = subscribe(&service, "news.all", &audience[1_000..]).await;
	assert_eq!(
		(changes.subscribed, changes.skipped_full),
		(201, 0),
		"the rest subscribe"
	);
	let changes = subscribe(&service, "news.all", &audience[..1_000]).await;
	assert_eq!(
		(changes.subscribed, changes.skipped_full),
		(0, 0),
		"a retried batch changes nothing"
	);

	// More than the bound is refused whole.
	let (status, body) = post(
		&service,
		"subscribe_to_topic",
		json!({ "topic": "news.all", "persons": group(9, 1_001) }),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 400, "{body}");

	let sent = send(
		&service,
		"to-everyone-who-asked",
		json!({ "topic": "news.all" }),
	)
	.await;
	assert_eq!(sent.state, SendState::FanningOut, "{sent:?}");
	assert_eq!((sent.delivered, sent.targeted), (0, 0), "nothing yet");

	let progress = settled(&service, "to-everyone-who-asked").await;
	assert_eq!(progress.state, SendState::Complete);
	assert_eq!(progress.targeted, 1_201);
	assert_eq!(progress.delivered, 1_201);
	assert_eq!(progress.skipped_opted_out, 0);

	let stored = rows(
		&service,
		"SELECT count(*), count(DISTINCT person) FROM notifications WHERE send_key = \
		 'to-everyone-who-asked'",
	)
	.await;
	assert_eq!(
		(text(&stored[0][0]), text(&stored[0][1])),
		("1201".to_owned(), "1201".to_owned()),
		"one row per person, nobody twice"
	);

	// A person who never subscribed — bob — has nothing.
	let page = inbox(&service, &signed_in(BOB)).await;
	assert_eq!(page["items"].as_array().map(Vec::len), Some(0), "{page}");
}

/// A condition decides by the grammar: `'a' && ('b' || !'c')` reaches
/// exactly the people it holds for, and nobody else.
#[tokio::test(flavor = "multi_thread")]
async fn a_condition_send_matches_the_grammar() {
	let service = service().await;
	let both = group(3, 2);
	let a_and_c = group(4, 2);
	let b_and_c = group(5, 2);

	subscribe(&service, "a", &both).await;
	subscribe(&service, "b", &both).await;
	subscribe(&service, "a", &a_and_c).await;
	subscribe(&service, "c", &a_and_c).await;
	subscribe(&service, "b", &b_and_c).await;
	subscribe(&service, "c", &b_and_c).await;

	let sent = send(
		&service,
		"by-condition",
		json!({ "condition": "'a' in topics && ('b' in topics || !('c' in topics))" }),
	)
	.await;
	assert_eq!(sent.state, SendState::FanningOut);

	let progress = settled(&service, "by-condition").await;
	// `a && b` holds for both; `a && !c` holds for nobody extra here; the
	// b-and-c people don't hold `a` at all.
	assert_eq!(progress.targeted, 2, "{progress:?}");
	assert_eq!(progress.delivered, 2);

	for person in &both {
		let page = inbox(&service, &signed_in(*person)).await;
		assert_eq!(page["items"].as_array().map(Vec::len), Some(1), "{person}");
	}

	for person in a_and_c.iter().chain(&b_and_c) {
		let page = inbox(&service, &signed_in(*person)).await;
		assert_eq!(
			page["items"].as_array().map(Vec::len),
			Some(0),
			"{person} doesn't hold `a`"
		);
	}

	// A condition that matches nobody still completes, with zeros.
	send(
		&service,
		"nobody-matches",
		json!({ "condition": "'a' in topics && 'b' in topics && 'c' in topics" }),
	)
	.await;
	let progress = settled(&service, "nobody-matches").await;
	assert_eq!(
		(
			progress.targeted,
			progress.delivered,
			progress.skipped_opted_out
		),
		(0, 0, 0),
		"{progress:?}"
	);

	// An unanchored condition never gets as far as a send.
	let (status, body) = post(
		&service,
		"send",
		json!({
			"idempotency_key": "unanchored",
			"spec": { "category": "news", "title": "No" },
			"target": { "condition": "!('a' in topics)" },
		}),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 400, "refused: {body}");
}

/// A replayed fanout — the cursor reset, the state set back — writes no
/// duplicate row and changes no count: the send's unique row per person
/// is what the batch runs into.
#[tokio::test(flavor = "multi_thread")]
async fn a_replayed_fanout_writes_no_duplicates() {
	let mut service = service().await;
	let audience = group(2, 700);
	subscribe(&service, "news.replay", &audience).await;

	send(&service, "replayed", json!({ "topic": "news.replay" })).await;
	let progress = settled(&service, "replayed").await;
	assert_eq!(progress.delivered, 700);

	// Stop the workers, wind the bookkeeping back as a crash between the
	// rows and the cursor would leave it, and walk the steps again. A
	// real replay re-runs the job rows the crash left, under their own
	// keys; the test re-walks under fresh step numbers so the successor
	// keys never collide with the first walk's finished jobs.
	service.stop_jobs().await;
	service
		.db
		.fetch(Statement {
			sql: "UPDATE sends SET state = 'fanning_out', cursor = NULL WHERE idempotency_key = \
			      'replayed'"
				.into(),
			args: Vec::new(),
		})
		.await
		.expect("the reset");

	for step in 100..104 {
		service.run_step("replayed", step).await;
	}

	let stored = rows(
		&service,
		"SELECT count(*), count(DISTINCT person) FROM notifications WHERE send_key = 'replayed'",
	)
	.await;
	assert_eq!(
		(text(&stored[0][0]), text(&stored[0][1])),
		("700".to_owned(), "700".to_owned()),
		"no second copy of anyone"
	);

	let (status, body) = post(
		&service,
		"send_status",
		json!({ "idempotency_key": "replayed" }),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	let progress: SendProgress = serde_json::from_str(&body).unwrap();
	assert_eq!(progress.state, SendState::Complete);
	assert_eq!(
		(progress.targeted, progress.delivered),
		(700, 700),
		"the counts didn't move: {progress:?}"
	);
}

/// Cancelling mid-fanout stops the audience where it stood: the rows
/// written so far are cancelled, the rest never arrive, and the send
/// answers cancelled from then on.
#[tokio::test(flavor = "multi_thread")]
async fn cancelling_mid_fanout_stops_the_rest() {
	let service = idle().await;
	let audience = group(1, 1_100);
	subscribe(&service, "news.stoppable", &audience[..1_000]).await;
	subscribe(&service, "news.stoppable", &audience[1_000..]).await;

	send(&service, "stopped", json!({ "topic": "news.stoppable" })).await;

	// The first step delivers its batch of 500 and stops there, because
	// no worker picks up the successor.
	service.run_step("stopped", 0).await;
	let stored = rows(
		&service,
		"SELECT count(*) FROM notifications WHERE send_key = 'stopped'",
	)
	.await;
	assert_eq!(text(&stored[0][0]), "500", "one batch delivered");

	let (status, body) = post(
		&service,
		"cancel",
		json!({ "idempotency_key": "stopped" }),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "a fanning send cancels: {body}");
	assert_eq!(body, "500", "the batch's rows were cancelled");

	for step in 1..4 {
		service.run_step("stopped", step).await;
	}

	let stored = rows(
		&service,
		"SELECT count(*) FROM notifications WHERE send_key = 'stopped'",
	)
	.await;
	assert_eq!(
		text(&stored[0][0]),
		"500",
		"the cancelled fanout wrote nothing more"
	);

	let (_, body) = post(
		&service,
		"send_status",
		json!({ "idempotency_key": "stopped" }),
		Some(TOKEN),
	)
	.await;
	let progress: SendProgress = serde_json::from_str(&body).unwrap();
	assert_eq!(progress.state, SendState::Cancelled, "{progress:?}");

	// Cancelling again answers 0, the way a known send always does.
	let (status, body) = post(
		&service,
		"cancel",
		json!({ "idempotency_key": "stopped" }),
		Some(TOKEN),
	)
	.await;
	assert_eq!((status, body.as_str()), (200, "0"));
}

/// An opted-out subscriber is skipped and counted, exactly as a
/// recipients send skips them.
#[tokio::test(flavor = "multi_thread")]
async fn opted_out_subscribers_are_skipped_and_counted() {
	let service = service().await;
	let audience = group(6, 3);
	subscribe(&service, "marketing", &audience).await;

	let (status, body) = post(
		&service,
		"set_category_preference",
		json!({ "category": "news", "opted_out": true }),
		Some(&signed_in(audience[0])),
	)
	.await;
	assert_eq!(status, 200, "{body}");

	send(&service, "with-opt-out", json!({ "topic": "marketing" })).await;
	let progress = settled(&service, "with-opt-out").await;
	assert_eq!(
		(
			progress.targeted,
			progress.delivered,
			progress.skipped_opted_out
		),
		(3, 2, 1),
		"{progress:?}"
	);

	let page = inbox(&service, &signed_in(audience[0])).await;
	assert_eq!(
		page["items"].as_array().map(Vec::len),
		Some(0),
		"the opted-out person has nothing"
	);
}

/// A scheduled topic send is invisible until its time, then arrives.
#[tokio::test(flavor = "multi_thread")]
async fn a_scheduled_topic_send_hides_until_its_time() {
	let service = service().await;
	subscribe(&service, "news.scheduled", &[ALICE]).await;

	let due = (chrono::Utc::now() + chrono::Duration::seconds(2)).to_rfc3339();
	let (status, body) = post(
		&service,
		"send",
		json!({
			"idempotency_key": "scheduled-topic",
			"spec": {
				"category": "news",
				"title": "Later",
				"visible_at": due,
			},
			"target": { "topic": "news.scheduled" },
		}),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	settled(&service, "scheduled-topic").await;

	let alice = signed_in(ALICE);
	let page = inbox(&service, &alice).await;
	assert_eq!(page["items"].as_array().map(Vec::len), Some(0), "not yet");

	tokio::time::sleep(Duration::from_millis(2_600)).await;

	let page = inbox(&service, &alice).await;
	assert_eq!(
		page["items"].as_array().map(Vec::len),
		Some(1),
		"due: {page}"
	);
	assert_eq!(page["items"][0]["title"], "Later");
}

/// Self-subscription follows the operator's open patterns: a closed
/// topic is forbidden with a clear error, an open pattern accepts, and
/// one person's topics are nobody else's to see or change.
#[tokio::test(flavor = "multi_thread")]
async fn self_subscription_follows_the_open_patterns() {
	let service = start(
		leptoskit_db::testing::sqlite_migrated(leptoskit_notifications::MIGRATIONS).await,
		"news.*, inbox",
	)
	.await;
	let alice = signed_in(ALICE);
	let bob = signed_in(BOB);

	// An anonymous caller can't subscribe at all.
	let (status, _) = post(
		&service,
		"subscribe",
		json!({ "topic": "news.breaking" }),
		None,
	)
	.await;
	assert_eq!(status, 403, "a person is read from the identity");

	// A bad name is invalid, not forbidden.
	let (status, _) = post(
		&service,
		"subscribe",
		json!({ "topic": "not a topic" }),
		Some(&alice),
	)
	.await;
	assert_eq!(status, 400);

	// Closed topic, whoever asks.
	let (status, body) = post(
		&service,
		"subscribe",
		json!({ "topic": "alerts.major" }),
		Some(&alice),
	)
	.await;
	assert_eq!(status, 403, "a closed topic is forbidden: {body}");
	assert!(
		body.contains("open"),
		"the refusal names the rule, not the person: {body}"
	);

	// An open pattern accepts, and a pattern without a star is exact.
	let (status, body) = post(
		&service,
		"subscribe",
		json!({ "topic": "news.breaking" }),
		Some(&alice),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	let (status, body) = post(
		&service,
		"subscribe",
		json!({ "topic": "inbox" }),
		Some(&alice),
	)
	.await;
	assert_eq!(status, 200, "{body}");

	// `my_topics` is the person's own list, oldest first, paginated.
	let (status, body) = post(
		&service,
		"my_topics",
		json!({ "page": { "limit": 1 } }),
		Some(&alice),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	let page: Value = serde_json::from_str(&body).unwrap();
	assert_eq!(
		page["items"],
		json!(["news.breaking"]),
		"oldest first, one per page: {page}"
	);
	assert!(page["has_more"].as_bool().unwrap(), "{page}");

	// Bob sees none of alice's topics, and a send to one of them reaches
	// only alice.
	let (_, body) = post(
		&service,
		"my_topics",
		json!({ "page": { "limit": 10 } }),
		Some(&bob),
	)
	.await;
	let page: Value = serde_json::from_str(&body).unwrap();
	assert_eq!(
		page["items"].as_array().map(Vec::len),
		Some(0),
		"alice's topics are alice's"
	);

	send(
		&service,
		"self-subscribed",
		json!({ "topic": "news.breaking" }),
	)
	.await;
	settled(&service, "self-subscribed").await;

	let (_, body) = post(&service, "unread_count", json!({}), Some(&alice)).await;
	assert_eq!(body, "1", "alice asked for it");
	let (_, body) = post(&service, "unread_count", json!({}), Some(&bob)).await;
	assert_eq!(body, "0", "bob didn't");

	// A person unsubscribes from any topic they hold — open or not.
	let (status, _) = post(
		&service,
		"unsubscribe",
		json!({ "topic": "alerts.major" }),
		Some(&alice),
	)
	.await;
	assert_eq!(status, 200, "not holding it is fine");

	let (status, _) = post(
		&service,
		"unsubscribe",
		json!({ "topic": "news.breaking" }),
		Some(&alice),
	)
	.await;
	assert_eq!(status, 200);

	let (_, body) = post(
		&service,
		"my_topics",
		json!({ "page": { "limit": 10 } }),
		Some(&alice),
	)
	.await;
	let page: Value = serde_json::from_str(&body).unwrap();
	assert_eq!(page["items"].as_array().map(Vec::len), Some(1), "{page}");

	// The app's batch unsubscribe takes the other one, and answers the
	// count it removed.
	let (status, body) = post(
		&service,
		"unsubscribe_from_topic",
		json!({ "topic": "inbox", "persons": [ALICE, BOB] }),
		Some(TOKEN),
	)
	.await;
	assert_eq!((status, body.as_str()), (200, "1"));

	// Only the app batches people around.
	for operation in ["subscribe_to_topic", "unsubscribe_from_topic"] {
		let (status, _) = post(
			&service,
			operation,
			json!({ "topic": "inbox", "persons": [ALICE] }),
			Some(&alice),
		)
		.await;
		assert_eq!(status, 403, "a person can't batch anyone");
	}
}

/// A dry run counts the audience a send would reach now — matched,
/// skipped, and delivered — and writes nothing.
#[tokio::test(flavor = "multi_thread")]
async fn validate_only_counts_and_writes_nothing() {
	let service = service().await;
	let audience = group(7, 2);
	subscribe(&service, "news.dry", &audience).await;
	subscribe(&service, "news.dry", &[BOB]).await;

	let (status, _) = post(
		&service,
		"set_category_preference",
		json!({ "category": "news", "opted_out": true }),
		Some(&signed_in(audience[0])),
	)
	.await;
	assert_eq!(status, 200);

	for (name, target, expected) in [
		(
			"a topic",
			json!({ "topic": "news.dry" }),
			(3_u32, 2_u32, 1_u32),
		),
		(
			"a condition",
			json!({ "condition": "'news.dry' in topics" }),
			(3, 2, 1),
		),
		(
			"recipients",
			json!({ "recipients": [ALICE, BOB, BOB] }),
			(2, 2, 0),
		),
	] {
		let (status, body) = post(
			&service,
			"send",
			json!({
				"idempotency_key": format!("dry-{name}"),
				"spec": { "category": "news", "title": "Just looking" },
				"target": target,
				"validate_only": true,
			}),
			Some(TOKEN),
		)
		.await;
		assert_eq!(status, 200, "{name}: {body}");
		let result: SendResult = serde_json::from_str(&body).unwrap();
		assert_eq!(
			(result.targeted, result.delivered, result.skipped_opted_out),
			expected,
			"{name}: {result:?}"
		);
		assert!(!result.duplicate, "{name}");
	}

	// A dry run of a bad condition still refuses.
	let (status, _) = post(
		&service,
		"send",
		json!({
			"idempotency_key": "dry-bad",
			"spec": { "category": "news", "title": "No" },
			"target": { "condition": "news" },
			"validate_only": true,
		}),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 400);

	for table in ["notifications", "sends"] {
		let stored = rows(&service, &format!("SELECT count(*) FROM {table}")).await;
		assert_eq!(text(&stored[0][0]), "0", "{table} stayed empty");
	}
}

/// A topic send answers its stored outcome for the same key and intent,
/// conflicts on a different one, and its status follows the fanout.
#[tokio::test(flavor = "multi_thread")]
async fn a_topic_send_is_idempotent_by_key_and_intent() {
	let service = service().await;
	subscribe(&service, "news.idempotent", &[ALICE, BOB]).await;

	let target = json!({ "topic": "news.idempotent" });
	let first = send(&service, "idempotent-topic", target.clone()).await;
	assert!(!first.duplicate);
	settled(&service, "idempotent-topic").await;

	let again = send(&service, "idempotent-topic", target).await;
	assert!(again.duplicate, "{again:?}");
	assert_eq!(again.state, SendState::Complete, "the state comes back");
	assert_eq!(again.delivered, 2);

	let (status, body) = post(
		&service,
		"send",
		json!({
			"idempotency_key": "idempotent-topic",
			"spec": { "category": "news", "title": "A different send" },
			"target": { "topic": "news.idempotent" },
		}),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 409, "a changed intent conflicts: {body}");

	let (status, body) = post(
		&service,
		"send",
		json!({
			"idempotency_key": "idempotent-topic",
			"spec": { "category": "news", "title": "Hello topics" },
			"target": { "condition": "'news.idempotent' in topics" },
		}),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 409, "a changed target conflicts too: {body}");

	// An unknown key's status is not found; only the app may ask.
	let (status, _) = post(
		&service,
		"send_status",
		json!({ "idempotency_key": "never-sent" }),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 404);

	let (status, _) = post(
		&service,
		"send_status",
		json!({ "idempotency_key": "idempotent-topic" }),
		Some(&signed_in(ALICE)),
	)
	.await;
	assert_eq!(status, 403, "a person doesn't read send status");

	let stored = rows(
		&service,
		"SELECT count(*) FROM notifications WHERE send_key = 'idempotent-topic'",
	)
	.await;
	assert_eq!(text(&stored[0][0]), "2", "nothing was written twice");
}
