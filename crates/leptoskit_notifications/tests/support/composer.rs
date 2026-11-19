//! Checks of the send history and the audience catalogue that both
//! databases run, against a service with an APNs transport over a
//! boundary that records every push — accepting it, or answering that the
//! device is gone for a token that says so — and with the topic pattern
//! `news.*` open to self-subscription.
//!
//! Every answer is read as JSON over HTTP, as the app's server reads it,
//! and what the checks need to arrange that no operation offers — an
//! endpoint its provider invalidated, a row read at a chosen instant, a
//! send from before the history — is written with plain SQL.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use leptoskit_db::Db;
use leptoskit_db::Read as _;
use leptoskit_db::Statement;
use leptoskit_notifications::SERVICE;
use leptoskit_notifications::service::config;
use leptoskit_notifications::service::push;
use leptoskit_notifications::service::retention;
use leptoskit_notifications::service::transport::Boundary;
use leptoskit_notifications::service::transport::PushRuntime;
use leptoskit_notifications::service::transport::WireError;
use leptoskit_notifications::service::transport::WireRequest;
use leptoskit_notifications::service::transport::WireResponse;
use leptoskit_rpc as rpc;
use leptoskit_rpc::server::Anonymous;
use leptoskit_rpc::server::ServiceCallers;
use serde_json::Value;
use serde_json::json;
use uuid::Uuid;

use super::sql::Sql;

const TOKEN: &str = "app-token";
const ORIGIN: &str = "http://app.test";
const PREFIX: &str = "/_leptoskit/services/notifications";

/// How long a fanout or a push round may take on a loaded machine.
const SETTLES: Duration = Duration::from_secs(60);

fn person(n: u128) -> Uuid {
	Uuid::from_u128(0xC0A5_0000_0000_0000_0000_0000_0000_0000 | n)
}

/// The provider boundary: it accepts every push, except to a device token
/// that starts `dead`, which it answers is gone.
#[derive(Default)]
pub struct Provider;

impl Boundary for Provider {
	fn post(
		&self,
		request: WireRequest,
	) -> Pin<Box<dyn Future<Output = Result<WireResponse, WireError>> + Send + '_>> {
		let gone = request
			.url
			.rsplit('/')
			.next()
			.is_some_and(|token| token.starts_with("dead"));

		Box::pin(async move {
			Ok(if gone {
				WireResponse {
					status: 410,
					headers: Vec::new(),
					body: br#"{"reason":"Unregistered"}"#.to_vec(),
				}
			} else {
				WireResponse {
					status: 200,
					headers: Vec::new(),
					body: Vec::new(),
				}
			})
		})
	}
}

/// A running service over one database.
pub struct Service {
	pub address: std::net::SocketAddr,
	pub db: Db,
	jobs: Option<rpc::server::JobsHandle>,
}

impl Drop for Service {
	fn drop(&mut self) {
		if let Some(jobs) = self.jobs.take() {
			jobs.stop();
		}
	}
}

/// Serves the notifications routes over `db`, already migrated — and,
/// with `workers`, the service's own workers on a test-fast poll. Without
/// them a topic send stays as it was made: fanning out.
pub async fn serve(db: &Db, workers: bool) -> Service {
	let key_p8 = {
		let rng = ring::rand::SystemRandom::new();
		let document = ring::signature::EcdsaKeyPair::generate_pkcs8(
			&ring::signature::ECDSA_P256_SHA256_FIXED_SIGNING,
			&rng,
		)
		.expect("a key");
		format!(
			"-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\n",
			base64::Engine::encode(
				&base64::engine::general_purpose::STANDARD,
				document.as_ref()
			)
		)
	};
	let config = config::PushConfig {
		apns: Some(config::ApnsConfig {
			key_p8,
			key_id: "KEY".into(),
			team_id: "TEAM".into(),
			topic: "dev.leptoskit.app".into(),
			environment: config::ApnsEnvironment::Sandbox,
		}),
		fcm: None,
		web: None,
	};
	let push = push::Push {
		runtime: Some(Arc::new(
			PushRuntime::build_over(&config, Arc::new(Provider)).expect("the runtime builds"),
		)),
	};
	let open_topics =
		config::OpenTopics::from_vars([("LEPTOSKIT_NOTIFICATIONS_OPEN_TOPICS", "news.*")])
			.expect("the pattern is one");
	let callers = ServiceCallers::new(Anonymous)
		.caller(TOKEN, rpc::server::APP_CALLER)
		.forwarded_identity(SERVICE, TOKEN);
	let (router, live) = leptoskit_notifications::service::routes_with_settings(
		db.clone(),
		Some(ORIGIN),
		callers,
		push.clone(),
		open_topics,
	);
	live.start().await.expect("live starts");
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let address = listener.local_addr().unwrap();
	tokio::spawn(async move {
		leptoskit_axum::axum::serve(listener, router).await.unwrap();
	});
	let jobs = workers.then(|| {
		leptoskit_notifications::service::workers_with_push(db.clone(), push)
			.poll(Duration::from_millis(50))
			.start()
	});

	Service {
		address,
		db: db.clone(),
		jobs,
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

/// Who makes a call.
#[derive(Clone, Copy)]
enum As {
	App,
	Person(Uuid),
	Nobody,
}

async fn call(service: &Service, operation: &str, args: Value, caller: As) -> (u16, String) {
	let request = reqwest::Client::new()
		.post(format!(
			"http://{}{PREFIX}/rpc/{operation}",
			service.address
		))
		.json(&args)
		.timeout(Duration::from_secs(120));
	let request = match caller {
		As::App => request.bearer_auth(TOKEN),
		As::Person(person) => request.bearer_auth(signed_in(person)),
		As::Nobody => request,
	};
	let response = request.send().await.unwrap();

	(response.status().as_u16(), response.text().await.unwrap())
}

/// Calls `operation` as the app and answers its JSON, which must be a
/// success.
async fn app(service: &Service, operation: &str, args: Value) -> Value {
	let (status, body) = call(service, operation, args, As::App).await;
	assert_eq!(status, 200, "{operation}: {body}");
	serde_json::from_str(&body).unwrap()
}

/// Calls `operation` as `person`, which must succeed.
async fn as_person(service: &Service, person: Uuid, operation: &str, args: Value) -> Value {
	let (status, body) = call(service, operation, args, As::Person(person)).await;
	assert_eq!(status, 200, "{operation}: {body}");
	serde_json::from_str(&body).unwrap()
}

async fn send(service: &Service, key: &str, spec: Value, target: Value) -> Value {
	app(
		service,
		"send",
		json!({ "idempotency_key": key, "spec": spec, "target": target }),
	)
	.await
}

fn news(title: &str) -> Value {
	json!({ "category": "news", "title": title })
}

/// Runs `sql`, which must succeed.
async fn run(db: &Db, sql: String) -> Vec<Vec<leptoskit_db::Value>> {
	db.fetch(Statement::new(sql.clone(), Vec::new()))
		.await
		.unwrap_or_else(|error| panic!("{error}\n{sql}"))
}

/// Waits until `done` answers true.
async fn until<F, Fut>(what: &str, mut done: F)
where
	F: FnMut() -> Fut,
	Fut: Future<Output = bool>,
{
	let deadline = tokio::time::Instant::now() + SETTLES;

	loop {
		if done().await {
			return;
		}

		assert!(tokio::time::Instant::now() < deadline, "never: {what}");
		tokio::time::sleep(Duration::from_millis(50)).await;
	}
}

/// `person` enrolls an APNs device whose token is `token` padded to a
/// device token's length, reporting `device`.
async fn enroll(service: &Service, person: Uuid, token: &str, device: Value) -> Uuid {
	let registered = as_person(
		service,
		person,
		"register_push_endpoint",
		json!({ "credential": { "apns": { "device_token": device_token(token) } }, "device": device }),
	)
	.await;

	registered["id"]
		.as_str()
		.and_then(|id| id.parse().ok())
		.unwrap_or_else(|| panic!("an endpoint id in {registered}"))
}

/// A 64-hex device token that starts with `start`, itself hex.
fn device_token(start: &str) -> String {
	format!("{start:0<64}")
}

/// One page of `operation`.
async fn page(service: &Service, operation: &str, mut args: Value, page: Value) -> Value {
	args["page"] = page;
	app(service, operation, args).await
}

/// Every item of `operation`, read `limit` at a time, with how many pages
/// it took.
async fn every(service: &Service, operation: &str, args: Value, limit: u32) -> (Vec<Value>, usize) {
	let mut items = Vec::new();
	let mut after: Option<String> = None;
	let mut pages = 0;

	loop {
		let mut cursor = json!({ "limit": limit });

		if let Some(after) = &after {
			cursor["after"] = json!(after);
		}

		let answer = page(service, operation, args.clone(), cursor).await;
		pages += 1;
		let got = answer["items"].as_array().unwrap();
		assert!(
			got.len() <= usize::try_from(limit).unwrap(),
			"{operation}: a page holds at most {limit}: {answer}"
		);
		items.extend(got.iter().cloned());

		if answer["has_more"] != true {
			return (items, pages);
		}

		after = Some(
			answer["end"]
				.as_str()
				.unwrap_or_else(|| panic!("{operation}: a page with more has an end: {answer}"))
				.to_owned(),
		);
		assert!(pages < 200, "{operation} never ended");
	}
}

fn field<'a>(items: &'a [Value], name: &str) -> Vec<&'a str> {
	items
		.iter()
		.map(|item| item[name].as_str().unwrap_or_default())
		.collect()
}

// ── Send history ──────────────────────────────────────────────────────

/// Seven sends come back newest first, three to a page, in three pages,
/// each send once; a state filter keeps only the sends in that state.
pub async fn the_list_pages_newest_first(service: &Service) {
	for n in 1..=7 {
		send(
			service,
			&format!("list-{n}"),
			news(&format!("Number {n}")),
			json!({ "recipients": [person(1)] }),
		)
		.await;
		// A history id is minted from the clock: a moment apart, so the
		// order is the order they were made in on any machine.
		tokio::time::sleep(Duration::from_millis(3)).await;
	}

	let first = page(service, "list_sends", json!({}), json!({ "limit": 3 })).await;
	assert_eq!(
		field(first["items"].as_array().unwrap(), "key"),
		["list-7", "list-6", "list-5"],
		"{first}"
	);
	assert_eq!(first["has_more"], true);

	let (all, pages) = every(service, "list_sends", json!({}), 3).await;
	assert_eq!(pages, 3, "seven sends, three to a page");
	assert_eq!(
		field(&all, "key"),
		[
			"list-7", "list-6", "list-5", "list-4", "list-3", "list-2", "list-1"
		]
	);
	assert_eq!(field(&all, "title")[0], "Number 7", "each with its content");

	// A page loaded before a later one is pinned where it ended: a send
	// made since lands in it rather than pushing a row onto the next.
	send(
		service,
		"list-8",
		news("Number 8"),
		json!({ "recipients": [person(1)] }),
	)
	.await;
	let pinned = page(
		service,
		"list_sends",
		json!({}),
		json!({ "limit": 3, "until": first["end"] }),
	)
	.await;
	assert_eq!(
		field(pinned["items"].as_array().unwrap(), "key"),
		["list-8", "list-7", "list-6", "list-5"],
		"{pinned}"
	);

	// With no workers a topic send stays fanning out, which the filter
	// tells from the complete ones.
	send(
		service,
		"list-topic",
		news("Topic"),
		json!({ "topic": "weekly" }),
	)
	.await;
	let (fanning, _) = every(
		service,
		"list_sends",
		json!({ "filter": { "state": "fanning_out" } }),
		3,
	)
	.await;
	assert_eq!(field(&fanning, "key"), ["list-topic"]);
	let (complete, _) = every(
		service,
		"list_sends",
		json!({ "filter": { "state": "complete" } }),
		50,
	)
	.await;
	assert_eq!(complete.len(), 8, "{complete:?}");

	// A page is bounded whatever it asks for, and a cursor of another
	// list is refused.
	let (many, _) = every(service, "list_sends", json!({}), 100_000).await;
	assert_eq!(many.len(), 9);
	let (status, body) = call(
		service,
		"list_sends",
		json!({ "page": { "limit": 3, "after": "\"not-an-id\"" } }),
		As::App,
	)
	.await;
	assert_eq!(status, 400, "{body}");
}

/// Every kind of send is listed with its content, its audience, and its
/// schedule — and a recipients send without naming anyone.
#[expect(
	clippy::cognitive_complexity,
	reason = "one send of each kind, each read back whole"
)]
pub async fn each_kind_of_send_is_listed_with_its_content(service: &Service) {
	let (alice, bob) = (person(11), person(12));
	let later = (chrono::Utc::now() + chrono::Duration::hours(3))
		.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
	let expiry = (chrono::Utc::now() + chrono::Duration::days(2))
		.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
	let wall = (chrono::Utc::now() + chrono::Duration::days(3))
		.format("%Y-%m-%dT09:00")
		.to_string();

	send(
		service,
		"kind-recipients",
		json!({
			"category": "billing",
			"title": "Invoice paid",
			"body": "Thank you.",
			"label": "invoices",
			"expires_at": expiry,
		}),
		// A duplicate collapses: two people were named.
		json!({ "recipients": [alice, bob, alice] }),
	)
	.await;
	send(
		service,
		"kind-topic",
		json!({ "category": "news", "title": "Scheduled", "visible_at": later }),
		json!({ "topic": "news.sports" }),
	)
	.await;
	send(
		service,
		"kind-condition",
		news("For New Yorkers"),
		json!({ "condition": "user.city == 'New York' && 'news.local' in topics" }),
	)
	.await;
	send(
		service,
		"kind-everyone",
		json!({
			"category": "service",
			"title": "Good morning",
			"local_time": { "at": wall, "default_time_zone": "Europe/London" },
		}),
		json!("everyone"),
	)
	.await;
	send(
		service,
		"kind-data",
		json!({
			"category": "sync",
			"title": "sync",
			"data": "{\"refresh\":\"orders\"}",
			"options": { "kind": "data" },
		}),
		json!({ "recipients": [alice] }),
	)
	.await;

	let (status, body) = call(
		service,
		"list_sends",
		json!({ "page": { "limit": 50 } }),
		As::App,
	)
	.await;
	assert_eq!(status, 200, "{body}");

	for who in [alice, bob] {
		assert!(
			!body.contains(&who.to_string()) && !body.contains(&who.simple().to_string()),
			"the list names nobody: {body}"
		);
	}

	let listed: Value = serde_json::from_str(&body).unwrap();
	let entry = |key: &str| {
		listed["items"]
			.as_array()
			.unwrap()
			.iter()
			.find(|item| item["key"] == key)
			.unwrap_or_else(|| panic!("{key} is listed: {listed}"))
			.clone()
	};

	let recipients = entry("kind-recipients");
	assert_eq!(recipients["category"], "billing");
	assert_eq!(recipients["title"], "Invoice paid");
	assert_eq!(recipients["label"], "invoices");
	assert_eq!(recipients["kind"], "notification");
	assert_eq!(recipients["state"], "complete");
	assert_eq!(
		recipients["target"],
		json!({ "kind": "recipients", "recipients": 2 })
	);
	assert_eq!(
		(
			&recipients["targeted"],
			&recipients["delivered"],
			&recipients["skipped_opted_out"]
		),
		(&json!(2), &json!(2), &json!(0))
	);
	assert_eq!(recipients["expires_at"], expiry.as_str());
	assert!(recipients.get("visible_at").is_none(), "{recipients}");
	assert!(recipients["created_at"].is_string(), "{recipients}");

	let topic = entry("kind-topic");
	assert_eq!(
		topic["target"],
		json!({ "kind": "topic", "text": "news.sports", "recipients": 0 })
	);
	assert_eq!(topic["visible_at"], later.as_str());
	assert_eq!(topic["state"], "fanning_out");

	let condition = entry("kind-condition");
	assert_eq!(
		condition["target"],
		json!({
			"kind": "condition",
			"text": "user.city == 'New York' && 'news.local' in topics",
			"recipients": 0,
		})
	);

	let everyone = entry("kind-everyone");
	assert_eq!(
		everyone["target"],
		json!({ "kind": "everyone", "recipients": 0 })
	);
	assert_eq!(
		everyone["local_time"],
		json!({ "at": wall, "default_time_zone": "Europe/London" })
	);
	assert!(everyone.get("visible_at").is_none(), "{everyone}");

	let data = entry("kind-data");
	assert_eq!(data["kind"], "data");
	assert_eq!(data["category"], "sync");
}

/// A send's report describes the send — its spec and its audience — and
/// counts its push deliveries by platform and state with why the ended
/// ones ended, and its reads by the hour.
#[expect(
	clippy::too_many_lines,
	clippy::cognitive_complexity,
	reason = "one send followed through its pushes and its reads"
)]
pub async fn a_report_describes_the_send_and_counts_its_pushes_and_reads(service: &Service) {
	let sql = Sql(service.db.dialect());
	let people: Vec<Uuid> = (21..=25).map(person).collect();
	let [alice, bob, _carol, dave, erin] = people[..] else {
		unreachable!()
	};

	// Alice has one device, Bob's is gone, Carol has none, Dave has two,
	// and Erin opted out of the category.
	enroll(service, alice, "a1", json!({})).await;
	enroll(service, bob, "dead01", json!({})).await;
	enroll(service, dave, "d1", json!({})).await;
	enroll(service, dave, "d2", json!({})).await;
	enroll(service, erin, "e1", json!({})).await;
	as_person(
		service,
		erin,
		"set_category_preference",
		json!({ "category": "offers", "opted_out": true }),
	)
	.await;

	let spec = json!({
		"category": "offers",
		"title": "Half price",
		"body": "Today only.",
		"label": "spring-sale",
		"options": { "link": "/sale", "badge": { "set": 2 } },
	});
	let sent = send(
		service,
		"report-1",
		spec.clone(),
		json!({ "recipients": people }),
	)
	.await;
	assert_eq!(sent["delivered"], 4, "{sent}");
	assert_eq!(sent["skipped_opted_out"], 1, "{sent}");

	until("the pushes settle", || {
		async {
			let report = app(
				service,
				"send_report",
				json!({ "idempotency_key": "report-1" }),
			)
			.await;
			report["push"][0]["accepted"] == 3 && report["push"][0]["rejected"] == 1
		}
	})
	.await;

	// Three rows read: two in one hour, one in another, at instants this
	// check chooses.
	for (who, at) in [
		(alice, "2026-03-01T10:05:00.000Z"),
		(bob, "2026-03-01T10:59:59.000Z"),
		(dave, "2026-03-02T23:00:00.000Z"),
	] {
		run(
			&service.db,
			format!(
				"UPDATE notifications SET read_at = {} WHERE send_key = 'report-1' AND person = {}",
				sql.timestamp(at),
				sql.uuid(who)
			),
		)
		.await;
	}

	let report = app(
		service,
		"send_report",
		json!({ "idempotency_key": "report-1" }),
	)
	.await;
	assert_eq!(report["label"], "spring-sale");
	let mut made = spec.clone();
	for unset in ["data", "expires_at", "visible_at"] {
		made[unset] = Value::Null;
	}
	assert_eq!(report["spec"], made, "the send as it was made: {report}");
	assert_eq!(
		report["target"],
		json!({ "kind": "recipients", "recipients": 5 })
	);
	assert_eq!(
		report["progress"],
		json!({ "state": "complete", "targeted": 5, "delivered": 4, "skipped_opted_out": 1 })
	);
	assert_eq!(report["inbox"]["rows"], 4);
	assert_eq!(report["inbox"]["read"], 3);
	assert_eq!(
		report["push"],
		json!([{
			"platform": "apns",
			"enqueued": 0,
			"retrying": 0,
			"accepted": 3,
			"rejected": 1,
			"skipped": 0,
			"reasons": [{ "reason": "unregistered", "count": 1 }],
		}]),
		"{report}"
	);
	assert_eq!(
		report["reads_by_hour"],
		json!([
			{ "hour": "2026-03-01T10:00:00Z", "read": 2 },
			{ "hour": "2026-03-02T23:00:00Z", "read": 1 },
		]),
		"{report}"
	);

	// A scheduled send's deliveries wait, and the report says so.
	send(
		service,
		"report-later",
		json!({
			"category": "news",
			"title": "Later",
			"visible_at": (chrono::Utc::now() + chrono::Duration::hours(2)).to_rfc3339(),
		}),
		json!({ "recipients": [alice, dave] }),
	)
	.await;
	let later = app(
		service,
		"send_report",
		json!({ "idempotency_key": "report-later" }),
	)
	.await;
	assert_eq!(later["push"][0]["enqueued"], 3, "{later}");
	assert_eq!(later["push"][0]["accepted"], 0, "{later}");
	assert!(
		later.get("reads_by_hour").is_none(),
		"nothing read: {later}"
	);

	// A topic send's report carries its target as text.
	send(
		service,
		"report-topic",
		news("Topic"),
		json!({ "topic": "weekly" }),
	)
	.await;
	let topic = app(
		service,
		"send_report",
		json!({ "idempotency_key": "report-topic" }),
	)
	.await;
	assert_eq!(
		topic["target"],
		json!({ "kind": "topic", "text": "weekly", "recipients": 0 })
	);
	assert_eq!(topic["spec"]["title"], "Topic");
	assert!(topic.get("push").is_none(), "no deliveries: {topic}");

	// No report names a person, an endpoint, or a device.
	for key in ["report-1", "report-later", "report-topic"] {
		let (_, body) = call(
			service,
			"send_report",
			json!({ "idempotency_key": key }),
			As::App,
		)
		.await;

		for who in &people {
			assert!(
				!body.contains(&who.to_string()),
				"{key} names nobody: {body}"
			);
		}

		assert!(!body.contains(&device_token("a1")), "{body}");
	}
}

/// Only the app reads the history: an anonymous caller and a signed-in
/// person are both refused.
pub async fn only_the_app_reads_the_history(service: &Service) {
	send(
		service,
		"private-1",
		news("Private"),
		json!({ "recipients": [person(31)] }),
	)
	.await;

	for caller in [As::Nobody, As::Person(person(31))] {
		let (status, body) = call(
			service,
			"list_sends",
			json!({ "page": { "limit": 10 } }),
			caller,
		)
		.await;
		assert_eq!(status, 403, "{body}");
		assert!(!body.contains("private-1"), "{body}");

		let (status, body) = call(
			service,
			"send_report",
			json!({ "idempotency_key": "private-1" }),
			caller,
		)
		.await;
		assert_eq!(status, 403, "{body}");
	}
}

/// A recipients send now stores its spec for the history, and its retries
/// still answer as they always did: the same counts marked duplicate, the
/// people it named counted, and a changed intent refused.
pub async fn a_retried_recipients_send_still_answers_correctly(service: &Service) {
	let (alice, bob, carol) = (person(41), person(42), person(43));
	as_person(
		service,
		bob,
		"set_category_preference",
		json!({ "category": "news", "opted_out": true }),
	)
	.await;

	let target = json!({ "recipients": [alice, bob, carol] });
	let first = send(service, "retry-1", news("Once"), target.clone()).await;
	assert_eq!(
		first,
		json!({
			"delivered": 2,
			"skipped_opted_out": 1,
			"recipients": 3,
			"duplicate": false,
			"state": "complete",
			"targeted": 3,
		})
	);

	let mut again = send(service, "retry-1", news("Once"), target.clone()).await;
	assert_eq!(again["duplicate"], true);
	again["duplicate"] = json!(false);
	assert_eq!(again, first, "a retry answers the first run's counts");

	// The record holds the spec and still no target: nobody is named.
	let stored = run(
		&service.db,
		"SELECT target, spec, category FROM sends WHERE idempotency_key = 'retry-1'".to_owned(),
	)
	.await;
	assert_eq!(
		stored[0][0],
		leptoskit_db::Value::Null,
		"no target is stored"
	);
	assert_eq!(
		stored[0][1],
		leptoskit_db::Value::Text(r#"{"category":"news","title":"Once","body":"","data":null,"visible_at":null,"expires_at":null}"#.to_owned()),
	);
	assert_eq!(stored[0][2], leptoskit_db::Value::Text("news".to_owned()));

	for (spec, target) in [
		(news("Twice"), target.clone()),
		(news("Once"), json!({ "recipients": [alice, bob] })),
		(news("Once"), json!({ "topic": "weekly" })),
	] {
		let (status, body) = call(
			service,
			"send",
			json!({ "idempotency_key": "retry-1", "spec": spec, "target": target }),
			As::App,
		)
		.await;
		assert_eq!(status, 409, "{body}");
	}

	// One row each, however often it was asked.
	let rows = run(
		&service.db,
		"SELECT count(*) FROM notifications WHERE send_key = 'retry-1'".to_owned(),
	)
	.await;
	assert_eq!(rows[0][0], leptoskit_db::Value::Int(2));
}

/// A send leaves the list when retention prunes its record, and not
/// before.
pub async fn a_pruned_send_leaves_the_list(service: &Service) {
	send(
		service,
		"prune-1",
		news("Old"),
		json!({ "recipients": [person(51)] }),
	)
	.await;

	let retention = config::Retention::default();
	retention::prune(
		&service.db,
		&retention,
		leptoskit_db::now() + chrono::Duration::days(60),
	)
	.await
	.expect("a sweep");
	let (kept, _) = every(service, "list_sends", json!({}), 10).await;
	assert_eq!(
		field(&kept, "key"),
		["prune-1"],
		"the record outlives its rows"
	);

	let pruned = retention::prune(
		&service.db,
		&retention,
		leptoskit_db::now() + chrono::Duration::days(500),
	)
	.await
	.expect("a sweep");
	assert_eq!(pruned.sends, 1, "{pruned:?}");
	let (left, _) = every(service, "list_sends", json!({}), 10).await;
	assert!(left.is_empty(), "{left:?}");
}

/// A send made before the history existed is not listed, and its report
/// still answers — without a spec, for a recipients send that stored
/// none. `before` is every migration up to the one that adds the history,
/// and `all` is every migration.
pub async fn sends_from_before_the_history_are_not_listed(
	db: &Db,
	before: &[leptoskit_db::Migration],
	all: &[leptoskit_db::Migration],
) {
	let sql = Sql(db.dialect());
	let now = sql.timestamp(&chrono::Utc::now().to_rfc3339());

	db.migrate(before).await.expect("the earlier migrations");

	for (key, target, spec, state) in [
		("old-recipients", "NULL", "NULL", "complete"),
		(
			"old-topic",
			"'{\"topic\":\"weekly\"}'",
			"'{\"category\":\"news\",\"title\":\"Old \
			 topic\",\"body\":\"\",\"data\":null,\"visible_at\":null,\"expires_at\":null}'",
			"complete",
		),
	] {
		run(
			db,
			format!(
				"INSERT INTO sends (idempotency_key, intent, delivered, skipped_opted_out, \
				 targeted, state, target, spec, created_at) VALUES ('{key}', '{}', 3, 1, 4, \
				 '{state}', {target}, {spec}, {now})",
				"0".repeat(64)
			),
		)
		.await;
	}

	let applied = db.migrate(all).await.expect("the rest apply over rows");
	assert_eq!(applied.len(), all.len() - before.len(), "{applied:?}");

	let service = serve(db, false).await;
	let (listed, _) = every(&service, "list_sends", json!({}), 10).await;
	assert!(listed.is_empty(), "older sends are not listed: {listed:?}");

	let old = app(
		&service,
		"send_report",
		json!({ "idempotency_key": "old-recipients" }),
	)
	.await;
	assert!(old.get("spec").is_none(), "{old}");
	assert_eq!(
		old["target"],
		json!({ "kind": "recipients", "recipients": 4 })
	);
	assert_eq!(old["progress"]["delivered"], 3);

	let topic = app(
		&service,
		"send_report",
		json!({ "idempotency_key": "old-topic" }),
	)
	.await;
	assert_eq!(topic["spec"]["title"], "Old topic");
	assert_eq!(topic["target"]["text"], "weekly");

	// A send made now is listed, and the old ones are still not.
	send(
		&service,
		"new-1",
		news("New"),
		json!({ "recipients": [person(61)] }),
	)
	.await;
	let (listed, _) = every(&service, "list_sends", json!({}), 10).await;
	assert_eq!(field(&listed, "key"), ["new-1"]);

	// The categories list reads the new column, and an old send has none.
	let (categories, _) = every(&service, "categories", json!({}), 10).await;
	assert_eq!(field(&categories, "category"), ["news"]);
}

// ── Audience catalogue ────────────────────────────────────────────────

/// Marks `endpoint` as its provider would on answering that the device is
/// gone.
async fn invalidate(service: &Service, endpoint: Uuid) {
	let sql = Sql(service.db.dialect());
	run(
		&service.db,
		format!(
			"UPDATE push_endpoints SET invalidated_at = {} WHERE id = {}",
			sql.timestamp(&chrono::Utc::now().to_rfc3339()),
			sql.uuid(endpoint)
		),
	)
	.await;
}

async fn subscribe(service: &Service, topic: &str, people: &[Uuid]) {
	app(
		service,
		"subscribe_to_topic",
		json!({ "topic": topic, "persons": people }),
	)
	.await;
}

async fn attributes(service: &Service, people: &[Uuid], set: Value) {
	app(
		service,
		"set_person_attributes",
		json!({ "persons": people, "set": set, "remove": [] }),
	)
	.await;
}

fn exact(count: u32) -> Value {
	json!({ "count": count, "exact": true })
}

/// The overview counts the registry, the people who can be pushed to on
/// each platform — not through an endpoint its provider invalidated —
/// the topics with subscribers, and names the open topic patterns and
/// which transports are configured.
pub async fn the_overview_counts_what_the_service_knows(service: &Service) {
	let empty = app(service, "notifications_overview", json!({})).await;
	assert_eq!(empty["people_known"], exact(0));
	assert_eq!(empty["topics"], exact(0));
	assert_eq!(empty["open_topics"], json!(["news.*"]));
	assert_eq!(
		empty["people_with_push"],
		json!([
			{ "platform": "apns", "people": exact(0), "configured": true },
			{ "platform": "fcm", "people": exact(0), "configured": false },
			{ "platform": "web_push", "people": exact(0), "configured": false },
		])
	);

	let people: Vec<Uuid> = (101..=106).map(person).collect();
	attributes(service, &people[..4], json!({ "plan": "pro" })).await;
	subscribe(service, "alpha", &people[2..5]).await;
	subscribe(service, "beta", &people[..1]).await;
	// Two devices are one person; the sixth person is known only by
	// their enrollment.
	enroll(service, people[0], "aa01", json!({})).await;
	enroll(service, people[0], "aa02", json!({})).await;
	let gone = enroll(service, people[5], "aa03", json!({})).await;

	let overview = app(service, "notifications_overview", json!({})).await;
	assert_eq!(overview["people_known"], exact(6), "{overview}");
	assert_eq!(overview["topics"], exact(2), "{overview}");
	assert_eq!(overview["people_with_push"][0]["people"], exact(2));

	invalidate(service, gone).await;
	let overview = app(service, "notifications_overview", json!({})).await;
	assert_eq!(
		overview["people_with_push"][0]["people"],
		exact(1),
		"an invalidated endpoint reaches nobody: {overview}"
	);
	assert_eq!(overview["people_known"], exact(6), "they are still known");
}

/// Topics are listed in order with their subscribers and whether they are
/// open, a bounded page at a time; a prefix narrows the list.
pub async fn topics_are_listed_with_their_subscribers(service: &Service) {
	let people: Vec<Uuid> = (201..=205).map(person).collect();
	subscribe(service, "alpha", &people[..3]).await;
	subscribe(service, "beta", &people[..1]).await;
	subscribe(service, "news.local", &people[1..2]).await;
	subscribe(service, "news.sports", &people[..5]).await;
	subscribe(service, "newsletter", &people[3..5]).await;

	let (all, pages) = every(service, "topics", json!({}), 2).await;
	assert_eq!(pages, 3, "five topics, two to a page");
	assert_eq!(
		all,
		[
			json!({ "topic": "alpha", "subscribers": exact(3), "open": false }),
			json!({ "topic": "beta", "subscribers": exact(1), "open": false }),
			json!({ "topic": "news.local", "subscribers": exact(1), "open": true }),
			json!({ "topic": "news.sports", "subscribers": exact(5), "open": true }),
			json!({ "topic": "newsletter", "subscribers": exact(2), "open": false }),
		]
	);

	let (news, _) = every(service, "topics", json!({ "prefix": "news." }), 1).await;
	assert_eq!(field(&news, "topic"), ["news.local", "news.sports"]);
	let (news, _) = every(service, "topics", json!({ "prefix": "news" }), 50).await;
	assert_eq!(
		field(&news, "topic"),
		["news.local", "news.sports", "newsletter"]
	);
	let (none, _) = every(service, "topics", json!({ "prefix": "zzz" }), 50).await;
	assert!(none.is_empty(), "{none:?}");
	let (all, _) = every(service, "topics", json!({ "prefix": "" }), 50).await;
	assert_eq!(all.len(), 5, "an empty prefix is no prefix");

	// A topic with no subscriber left is no topic.
	app(
		service,
		"unsubscribe_from_topic",
		json!({ "topic": "beta", "persons": [people[0]] }),
	)
	.await;
	let (left, _) = every(service, "topics", json!({}), 50).await;
	assert_eq!(left.len(), 4);
	assert!(!field(&left, "topic").contains(&"beta"));

	// A page is bounded whatever it asks for, and a prefix is checked.
	let (status, body) = call(
		service,
		"topics",
		json!({ "page": { "limit": 10 }, "prefix": "two words" }),
		As::App,
	)
	.await;
	assert_eq!(status, 400, "{body}");
}

/// Attribute keys are listed with how many people hold each, and a key's
/// values with how many hold each — values with spaces and quotes
/// included, each of which a condition can name.
#[expect(
	clippy::cognitive_complexity,
	reason = "keys, values, prefixes and paging over one small set of people"
)]
pub async fn attributes_are_listed_with_their_values(service: &Service) {
	let people: Vec<Uuid> = (301..=306).map(person).collect();
	attributes(service, &people[..2], json!({ "plan": "pro" })).await;
	attributes(service, &people[2..3], json!({ "plan": "free" })).await;
	attributes(service, &people[..1], json!({ "city": "New York" })).await;
	attributes(service, &people[1..3], json!({ "city": "Coeur d'Alene" })).await;
	attributes(service, &people[3..4], json!({ "city": "Newark" })).await;
	attributes(service, &people[4..5], json!({ "city": "" })).await;
	attributes(service, &people[5..6], json!({ "team_2": "blue" })).await;

	let (keys, pages) = every(service, "attribute_keys", json!({}), 2).await;
	assert_eq!(pages, 2, "three keys, two to a page");
	assert_eq!(
		keys,
		[
			json!({ "key": "city", "people": exact(5) }),
			json!({ "key": "plan", "people": exact(3) }),
			json!({ "key": "team_2", "people": exact(1) }),
		]
	);

	let (plans, _) = every(service, "attribute_values", json!({ "key": "plan" }), 1).await;
	assert_eq!(
		plans,
		[
			json!({ "value": "free", "people": exact(1) }),
			json!({ "value": "pro", "people": exact(2) }),
		]
	);

	let (cities, _) = every(service, "attribute_values", json!({ "key": "city" }), 50).await;
	let mut named: Vec<(&str, u64)> = cities
		.iter()
		.map(|city| {
			(
				city["value"].as_str().unwrap(),
				city["people"]["count"].as_u64().unwrap(),
			)
		})
		.collect();
	named.sort_unstable();
	assert_eq!(
		named,
		[
			("", 1),
			("Coeur d'Alene", 2),
			("New York", 1),
			("Newark", 1)
		]
	);

	let (new, _) = every(
		service,
		"attribute_values",
		json!({ "key": "city", "prefix": "New" }),
		1,
	)
	.await;
	let mut found = field(&new, "value");
	found.sort_unstable();
	assert_eq!(found, ["New York", "Newark"]);

	// Every value listed can be written into a condition, and the
	// estimate for it is the count beside it.
	for city in &cities {
		let value = city["value"].as_str().unwrap();
		let condition = format!(
			"user.city == {}",
			leptoskit_notifications::condition::quoted(value)
		);
		let estimate = app(
			service,
			"estimate_audience",
			json!({ "target": { "condition": condition } }),
		)
		.await;
		assert_eq!(estimate["targeted"], city["people"]["count"], "{condition}");
		assert_eq!(estimate["exact"], true);
	}

	let (none, _) = every(
		service,
		"attribute_values",
		json!({ "key": "nobody_has" }),
		5,
	)
	.await;
	assert_eq!(none, Vec::<Value>::new());

	for bad in [
		json!({ "key": "Not A Key", "page": { "limit": 5 } }),
		json!({ "key": "city", "page": { "limit": 5 }, "prefix": "line\nbreak" }),
	] {
		let (status, body) = call(service, "attribute_values", bad, As::App).await;
		assert_eq!(status, 400, "{body}");
	}
}

/// Categories are listed with how many people opted out of each and when
/// the latest send in each was made: categories from sends and from
/// preferences, as one list.
pub async fn categories_are_listed_with_opt_outs_and_last_send(service: &Service) {
	let people: Vec<Uuid> = (401..=404).map(person).collect();

	for (who, category, opted_out) in [
		(people[0], "marketing", true),
		(people[1], "marketing", true),
		// Opting back in is not an opt-out.
		(people[2], "marketing", false),
		(people[0], "digest", true),
	] {
		as_person(
			service,
			who,
			"set_category_preference",
			json!({ "category": category, "opted_out": opted_out }),
		)
		.await;
	}

	send(
		service,
		"cat-1",
		json!({ "category": "marketing", "title": "First" }),
		json!({ "recipients": [people[3]] }),
	)
	.await;
	tokio::time::sleep(Duration::from_millis(5)).await;
	send(
		service,
		"cat-2",
		json!({ "category": "marketing", "title": "Second" }),
		json!({ "topic": "weekly" }),
	)
	.await;
	send(
		service,
		"cat-3",
		json!({ "category": "alerts", "title": "Third" }),
		json!({ "recipients": [people[3]] }),
	)
	.await;

	let (categories, pages) = every(service, "categories", json!({}), 2).await;
	assert_eq!(pages, 2, "three categories, two to a page");
	assert_eq!(
		field(&categories, "category"),
		["alerts", "digest", "marketing"]
	);
	assert_eq!(categories[0]["opted_out"], exact(0));
	assert!(categories[0]["last_sent"].is_string(), "{categories:?}");
	assert_eq!(categories[1]["opted_out"], exact(1));
	assert!(
		categories[1].get("last_sent").is_none(),
		"nothing was ever sent in it: {categories:?}"
	);
	assert_eq!(categories[2]["opted_out"], exact(2));

	// The latest send in a category is the latest one made.
	let report = app(
		service,
		"send_report",
		json!({ "idempotency_key": "cat-2" }),
	)
	.await;
	assert_eq!(categories[2]["last_sent"], report["created_at"]);
}

/// An estimate is exact for an audience it walks to the end — the counts
/// `validate_only` answers — and says it is not for a larger one, where
/// every count is of the people it walked. Reach counts live endpoints
/// only, of people who would get a row, and for a device condition only
/// the endpoints it holds for.
#[expect(
	clippy::cognitive_complexity,
	reason = "one registry, estimated every way a target can ask"
)]
pub async fn an_estimate_is_exact_below_its_cap_and_says_so_above_it(service: &Service) {
	// Twelve thousand people in the registry, a thousand a call.
	for batch in 0..12_u128 {
		let people: Vec<Uuid> = (0..1_000)
			.map(|n| person(10_000 + batch * 1_000 + n))
			.collect();
		attributes(service, &people, json!({ "cohort": "bulk" })).await;
	}

	// Forty of them hold a topic; five of those opted out of `offers`.
	let members: Vec<Uuid> = (0..40).map(|n| person(10_000 + n * 7)).collect();
	subscribe(service, "club", &members).await;

	for who in &members[..5] {
		as_person(
			service,
			*who,
			"set_category_preference",
			json!({ "category": "offers", "opted_out": true }),
		)
		.await;
	}

	// Devices: an opted-out member's, a member's on an old app version
	// with a second on a new one, another member's that its provider
	// invalidated, and one more member's on a new version.
	enroll(service, members[0], "b0", json!({ "app_version": "2.0.0" })).await;
	enroll(
		service,
		members[10],
		"b1",
		json!({ "app_version": "1.4.0" }),
	)
	.await;
	enroll(
		service,
		members[10],
		"b2",
		json!({ "app_version": "2.1.0" }),
	)
	.await;
	let gone = enroll(
		service,
		members[11],
		"b3",
		json!({ "app_version": "2.1.0" }),
	)
	.await;
	enroll(
		service,
		members[12],
		"b4",
		json!({ "app_version": "1.0.0" }),
	)
	.await;
	invalidate(service, gone).await;

	let estimate = |target: Value, category: Option<&'static str>| {
		async move {
			let mut args = json!({ "target": target });

			if let Some(category) = category {
				args["category"] = json!(category);
			}

			app(service, "estimate_audience", args).await
		}
	};

	// Below the cap: exact, and what a dry run counts.
	let club = estimate(json!({ "topic": "club" }), Some("offers")).await;
	assert_eq!(
		club,
		json!({
			"targeted": 40,
			"skipped_opted_out": 5,
			"delivered": 35,
			"exact": true,
			// Members 10 and 12: member 0 opted out, member 11's device
			// is gone.
			"reach": [{ "platform": "apns", "people": 2 }],
		})
	);
	let dry = app(
		service,
		"send",
		json!({
			"idempotency_key": "dry-1",
			"spec": { "category": "offers", "title": "t" },
			"target": { "topic": "club" },
			"validate_only": true,
		}),
	)
	.await;
	assert_eq!(
		(
			&dry["targeted"],
			&dry["skipped_opted_out"],
			&dry["delivered"]
		),
		(
			&club["targeted"],
			&club["skipped_opted_out"],
			&club["delivered"]
		)
	);

	// With no category nobody is skipped, and member 0's device counts.
	let club = estimate(json!({ "topic": "club" }), None).await;
	assert_eq!(club["skipped_opted_out"], 0);
	assert_eq!(club["delivered"], 40);
	assert_eq!(club["reach"], json!([{ "platform": "apns", "people": 3 }]));

	// A device condition reaches only the endpoints it holds for: member
	// 0 and member 10's newer device; member 11's is gone, and member 12
	// is out of the audience altogether.
	let newer = estimate(
		json!({ "condition": "'club' in topics && device.app_version >= '2.0'" }),
		None,
	)
	.await;
	assert_eq!(newer["targeted"], 2, "{newer}");
	assert_eq!(newer["exact"], true);
	assert_eq!(newer["reach"], json!([{ "platform": "apns", "people": 2 }]));

	// Recipients are counted as named, duplicates collapsed.
	let named = estimate(
		json!({ "recipients": [members[0], members[10], members[10], person(999_999)] }),
		Some("offers"),
	)
	.await;
	assert_eq!(
		named,
		json!({
			"targeted": 3,
			"skipped_opted_out": 1,
			"delivered": 2,
			"exact": true,
			"reach": [{ "platform": "apns", "people": 1 }],
		})
	);

	// Above the cap: it stops at five thousand people and says so.
	for target in [
		json!("everyone"),
		json!({ "condition": "user.cohort == 'bulk'" }),
	] {
		let large = estimate(target.clone(), Some("offers")).await;
		assert_eq!(large["exact"], false, "{target}: {large}");
		assert_eq!(large["targeted"], 5_000, "{target}: {large}");
		assert!(
			large["delivered"].as_u64().unwrap() <= 5_000,
			"{target}: {large}"
		);
	}

	// Nothing was written by any of it.
	let rows = run(&service.db, "SELECT count(*) FROM notifications".to_owned()).await;
	assert_eq!(rows[0][0], leptoskit_db::Value::Int(0));
	let sends = run(&service.db, "SELECT count(*) FROM sends".to_owned()).await;
	assert_eq!(sends[0][0], leptoskit_db::Value::Int(0));

	// What a send would refuse, an estimate refuses.
	for bad in [
		json!({ "target": { "condition": "!('club' in topics)" } }),
		json!({ "target": { "topic": "two words" } }),
		json!({ "target": { "recipients": [] } }),
		json!({ "target": "everyone", "category": "" }),
	] {
		let (status, body) = call(service, "estimate_audience", bad, As::App).await;
		assert_eq!(status, 400, "{body}");
	}
}

/// The app reads a person's endpoints — ids, platforms, device details,
/// and whether each was invalidated — and no answer of the catalogue or
/// the history carries a credential.
pub async fn a_persons_endpoints_are_listed_without_credentials(service: &Service) {
	let (alice, bob) = (person(501), person(502));
	let first = enroll(
		service,
		alice,
		"c0ffee01",
		json!({ "locale": "en-GB", "time_zone": "Europe/London", "app_version": "2.1.0" }),
	)
	.await;
	let second = enroll(service, alice, "c0ffee02", json!({})).await;
	enroll(service, bob, "c0ffee03", json!({})).await;
	invalidate(service, second).await;
	subscribe(service, "club", &[alice, bob]).await;
	attributes(service, &[alice], json!({ "plan": "pro" })).await;
	send(
		service,
		"creds-1",
		news("Hello"),
		json!({ "recipients": [alice, bob] }),
	)
	.await;

	let (endpoints, _) = every(service, "person_endpoints", json!({ "person": alice }), 1).await;
	assert_eq!(endpoints.len(), 2, "{endpoints:?}");
	let live = endpoints
		.iter()
		.find(|endpoint| endpoint["id"] == first.to_string())
		.expect("the first endpoint");
	assert_eq!(live["platform"], "apns");
	assert_eq!(
		live["device"],
		json!({ "locale": "en-GB", "time_zone": "Europe/London", "app_version": "2.1.0" })
	);
	assert!(live["invalidated_at"].is_null(), "{live}");
	let dead = endpoints
		.iter()
		.find(|endpoint| endpoint["id"] == second.to_string())
		.expect("the second endpoint");
	assert!(dead["invalidated_at"].is_string(), "{dead}");

	let (nobody, _) = every(
		service,
		"person_endpoints",
		json!({ "person": person(599) }),
		10,
	)
	.await;
	assert_eq!(nobody, Vec::<Value>::new());

	// No answer anywhere holds a device token, whole or in part.
	for (operation, args) in [
		(
			"person_endpoints",
			json!({ "person": alice, "page": { "limit": 20 } }),
		),
		("notifications_overview", json!({})),
		("topics", json!({ "page": { "limit": 20 } })),
		("attribute_keys", json!({ "page": { "limit": 20 } })),
		(
			"attribute_values",
			json!({ "key": "plan", "page": { "limit": 20 } }),
		),
		("categories", json!({ "page": { "limit": 20 } })),
		(
			"estimate_audience",
			json!({ "target": { "topic": "club" } }),
		),
		("list_sends", json!({ "page": { "limit": 20 } })),
		("send_report", json!({ "idempotency_key": "creds-1" })),
		(
			"preview_push",
			json!({ "spec": { "category": "news", "title": "Hello" } }),
		),
	] {
		let (status, body) = call(service, operation, args, As::App).await;
		assert_eq!(status, 200, "{operation}: {body}");
		let lower = body.to_lowercase();
		assert!(
			!lower.contains("c0ffee")
				&& !lower.contains("credential")
				&& !lower.contains("fingerprint"),
			"{operation} carries no credential: {body}"
		);
	}
}

/// Only the app reads the catalogue: an anonymous caller and a signed-in
/// person are refused by every operation, their own endpoints included.
pub async fn only_the_app_reads_the_catalogue(service: &Service) {
	let alice = person(601);
	enroll(service, alice, "f00d01", json!({})).await;

	for (operation, args) in [
		("notifications_overview", json!({})),
		("topics", json!({ "page": { "limit": 5 } })),
		("attribute_keys", json!({ "page": { "limit": 5 } })),
		(
			"attribute_values",
			json!({ "key": "plan", "page": { "limit": 5 } }),
		),
		("categories", json!({ "page": { "limit": 5 } })),
		("estimate_audience", json!({ "target": "everyone" })),
		(
			"person_endpoints",
			json!({ "person": alice, "page": { "limit": 5 } }),
		),
	] {
		for caller in [As::Nobody, As::Person(alice)] {
			let (status, body) = call(service, operation, args.clone(), caller).await;
			assert_eq!(status, 403, "{operation}: {body}");
		}

		let (status, body) = call(service, operation, args, As::App).await;
		assert_eq!(status, 200, "{operation}: {body}");
	}
}

// ── Campaigns ─────────────────────────────────────────────────────────

/// Who acts in these checks, as the app's server would name them.
const ADA: &str = "ada@example.test";
const GRACE: &str = "grace@example.test";

/// The count `sql` answers.
async fn counted(db: &Db, sql: String) -> i64 {
	match run(db, sql).await[0][0] {
		leptoskit_db::Value::Int(count) => count,
		ref other => panic!("a count, not {other:?}"),
	}
}

/// A draft that can be launched as it is: a message and two recipients.
fn ready(name: &str, recipients: &[Uuid]) -> Value {
	json!({
		"name": name,
		"spec": { "category": "news", "title": name, "body": "It is here." },
		"target": { "recipients": recipients },
	})
}

async fn create(service: &Service, draft: Value, by: &str) -> Value {
	app(
		service,
		"create_campaign",
		json!({ "draft": draft, "by": by }),
	)
	.await
}

/// The campaign's id, as the next call takes it.
fn id_of(campaign: &Value) -> &str {
	campaign["id"].as_str().expect("a campaign has an id")
}

/// Launches `id` at `revision`, and answers the status with the body.
async fn launch(service: &Service, id: &str, revision: u64, by: &str) -> (u16, Value) {
	let (status, body) = call(
		service,
		"launch_campaign",
		json!({ "id": id, "revision": revision, "by": by }),
		As::App,
	)
	.await;

	(
		status,
		serde_json::from_str(&body).unwrap_or(Value::String(body)),
	)
}

/// The kinds of the campaign's events, oldest first.
async fn event_kinds(service: &Service, id: &str) -> Vec<String> {
	let (events, _) = every(service, "campaign_events", json!({ "id": id }), 100).await;

	field(&events, "kind")
		.into_iter()
		.map(str::to_owned)
		.collect()
}

/// An RFC 3339 instant `hours` from now.
fn hours_from_now(hours: i64) -> String {
	(chrono::Utc::now() + chrono::Duration::hours(hours))
		.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// A draft saves with nothing but a name; every save names the revision
/// it was made from and is refused when the campaign has moved on; a
/// draft's bounds are refused by name; and the list shows it.
#[expect(
	clippy::cognitive_complexity,
	reason = "one draft, saved by two people in turn"
)]
pub async fn a_draft_is_saved_incomplete_and_revised(service: &Service) {
	// No title, no category, no audience: it is a draft.
	let saved = create(service, json!({ "name": "Spring sale" }), ADA).await;
	let id = id_of(&saved).to_owned();
	assert_eq!(saved["revision"], 1);
	assert_eq!(saved["state"], "draft");
	assert_eq!(saved["created_by"], ADA);
	assert_eq!(saved["updated_by"], ADA);
	assert!(saved["draft"].get("target").is_none(), "{saved}");
	assert!(saved.get("send_key").is_none(), "{saved}");

	let read = app(service, "campaign", json!({ "id": id })).await;
	assert_eq!(read, saved);

	// Grace fills it in, from revision 1.
	let filled = json!({
		"name": "Spring sale",
		"spec": { "category": "offers", "title": "Half price", "body": "Today only." },
		"target": { "condition": "user.plan == 'pro'" },
		"note": "Check the wording with legal.",
	});
	let second = app(
		service,
		"update_campaign",
		json!({ "id": id, "revision": 1, "draft": filled, "by": GRACE }),
	)
	.await;
	assert_eq!(second["revision"], 2);
	assert_eq!(second["created_by"], ADA);
	assert_eq!(second["updated_by"], GRACE);
	assert_eq!(second["draft"]["spec"]["title"], "Half price");
	assert_eq!(
		second["draft"]["target"],
		json!({ "condition": "user.plan == 'pro'" })
	);
	assert_eq!(second["draft"]["note"], "Check the wording with legal.");

	// Ada, still looking at revision 1, saves over it and is refused.
	let (status, body) = call(
		service,
		"update_campaign",
		json!({ "id": id, "revision": 1, "draft": { "name": "Ada's version" }, "by": ADA }),
		As::App,
	)
	.await;
	assert_eq!(status, 409, "{body}");
	assert!(body.contains("revision 2"), "{body}");
	let unchanged = app(service, "campaign", json!({ "id": id })).await;
	assert_eq!(unchanged, second, "a refused save changes nothing");

	// An unknown campaign is not found.
	let (status, body) = call(
		service,
		"update_campaign",
		json!({ "id": Uuid::nil(), "revision": 1, "draft": { "name": "x" }, "by": ADA }),
		As::App,
	)
	.await;
	assert_eq!(status, 404, "{body}");
	let (status, body) = call(service, "campaign", json!({ "id": Uuid::nil() }), As::App).await;
	assert_eq!(status, 404, "{body}");

	// Bounds are refused by name, and nothing is saved.
	for (draft, by, names) in [
		(json!({ "name": " " }), ADA, "name"),
		(json!({ "name": "n".repeat(121) }), ADA, "name"),
		(
			json!({ "name": "x", "note": "n".repeat(1_001) }),
			ADA,
			"note",
		),
		(
			json!({ "name": "x", "spec": { "category": "news", "title": "t", "body": "b".repeat(9_000) } }),
			ADA,
			"message",
		),
		(json!({ "name": "x" }), "", "`by`"),
		(json!({ "name": "x" }), "a\nb", "`by`"),
	] {
		let (status, body) = call(
			service,
			"create_campaign",
			json!({ "draft": draft, "by": by }),
			As::App,
		)
		.await;
		assert_eq!(status, 400, "{names}: {body}");
		assert!(body.contains(names), "{names}: {body}");
	}

	assert_eq!(
		counted(&service.db, "SELECT count(*) FROM campaigns".to_owned()).await,
		1
	);

	// The list shows it, and a state filter keeps or drops it.
	let (listed, _) = every(service, "campaigns", json!({}), 10).await;
	assert_eq!(listed.len(), 1, "{listed:?}");
	assert_eq!(listed[0]["id"], id.as_str());
	assert_eq!(listed[0]["name"], "Spring sale");
	assert_eq!(listed[0]["title"], "Half price");
	assert_eq!(listed[0]["category"], "offers");
	assert_eq!(listed[0]["revision"], 2);
	assert_eq!(
		listed[0]["target"],
		json!({ "kind": "condition", "text": "user.plan == 'pro'", "recipients": 0 })
	);
	assert!(
		listed[0].get("draft").is_none(),
		"a list row is not the whole draft"
	);

	let (drafts, _) = every(
		service,
		"campaigns",
		json!({ "filter": { "state": "draft" } }),
		10,
	)
	.await;
	assert_eq!(drafts.len(), 1);
	let (launched, _) = every(
		service,
		"campaigns",
		json!({ "filter": { "state": "launched" } }),
		10,
	)
	.await;
	assert_eq!(launched, Vec::<Value>::new());

	// Saving is not an event: the campaign says who saved it last.
	assert_eq!(event_kinds(service, &id).await, ["created"]);
}

/// A launch makes the send `send` would, once: launched again it answers
/// the same send as a duplicate, and nothing is written twice. A launched
/// campaign can no longer be changed.
#[expect(
	clippy::cognitive_complexity,
	reason = "one launch, pressed three times"
)]
pub async fn a_launch_makes_one_send_however_often_it_is_pressed(service: &Service) {
	let people = [person(701), person(702)];
	let saved = create(service, ready("It ships", &people), ADA).await;
	let id = id_of(&saved).to_owned();
	let key = format!("campaign:{id}:r1");

	// A launch from a revision the campaign is not at is refused.
	let (status, body) = launch(service, &id, 2, GRACE).await;
	assert_eq!(status, 409, "{body}");

	let (status, first) = launch(service, &id, 1, GRACE).await;
	assert_eq!(status, 200, "{first}");
	assert_eq!(first["send"]["duplicate"], false);
	assert_eq!(first["send"]["delivered"], 2);
	assert_eq!(first["send"]["state"], "complete");
	assert_eq!(first["campaign"]["state"], "launched");
	assert_eq!(first["campaign"]["send_key"], key.as_str());
	assert_eq!(first["campaign"]["launched_by"], GRACE);
	assert_eq!(first["campaign"]["revision"], 1);
	assert!(first["campaign"]["launched_at"].is_string(), "{first}");

	// Pressed again, by anyone: the same send, marked a duplicate.
	for by in [GRACE, ADA] {
		let (status, again) = launch(service, &id, 1, by).await;
		assert_eq!(status, 200, "{again}");
		assert_eq!(again["send"]["duplicate"], true);
		assert_eq!(again["send"]["delivered"], 2);
		assert_eq!(
			again["campaign"]["launched_by"], GRACE,
			"the first launch stands"
		);
		assert_eq!(
			again["campaign"]["launched_at"],
			first["campaign"]["launched_at"]
		);
	}

	assert_eq!(
		counted(
			&service.db,
			format!("SELECT count(*) FROM sends WHERE idempotency_key = '{key}'")
		)
		.await,
		1
	);
	assert_eq!(
		counted(
			&service.db,
			format!("SELECT count(*) FROM notifications WHERE send_key = '{key}'")
		)
		.await,
		2
	);
	assert_eq!(event_kinds(service, &id).await, ["created", "launched"]);

	// The send is an ordinary send: it is listed, and it has a report.
	let (sends, _) = every(service, "list_sends", json!({}), 10).await;
	assert!(field(&sends, "key").contains(&key.as_str()), "{sends:?}");
	let report = app(service, "send_report", json!({ "idempotency_key": key })).await;
	assert_eq!(report["spec"]["title"], "It ships");
	assert_eq!(report["inbox"]["rows"], 2);

	// Once retention has deleted the send's record, pressing launch again
	// must not make the send a second time.
	run(
		&service.db,
		format!("DELETE FROM notifications WHERE send_key = '{key}'"),
	)
	.await;
	run(
		&service.db,
		format!("DELETE FROM sends WHERE idempotency_key = '{key}'"),
	)
	.await;
	let (status, body) = launch(service, &id, 1, ADA).await;
	assert_eq!(status, 409, "{body}");
	assert!(body.to_string().contains("deleted"), "{body}");
	assert_eq!(
		counted(&service.db, "SELECT count(*) FROM sends".to_owned()).await,
		0,
		"no second send"
	);

	// What was launched can't be rewritten.
	let (status, body) = call(
		service,
		"update_campaign",
		json!({ "id": id, "revision": 1, "draft": { "name": "Changed" }, "by": ADA }),
		As::App,
	)
	.await;
	assert_eq!(status, 409, "{body}");
	assert!(body.contains("launched"), "{body}");
}

/// A launch checks everything a send checks, and that there is an
/// audience; an everyone audience needs the caller to say it is a
/// broadcast. A refused launch leaves a draft and no send.
pub async fn a_launch_checks_what_a_send_checks(service: &Service) {
	let registered = person(711);
	app(
		service,
		"set_person_attributes",
		json!({ "persons": [registered], "set": { "plan": "pro" }, "remove": [] }),
	)
	.await;

	for (draft, status, says) in [
		// No audience.
		(
			json!({ "name": "Nobody", "spec": { "category": "news", "title": "Hello" } }),
			400,
			"who this campaign is for",
		),
		// No title.
		(
			json!({
				"name": "Untitled",
				"spec": { "category": "news", "title": "" },
				"target": { "topic": "news.all" },
			}),
			400,
			"title",
		),
		// A condition that doesn't parse.
		(
			json!({
				"name": "Half a condition",
				"spec": { "category": "news", "title": "Hello" },
				"target": { "condition": "user.plan ==" },
			}),
			400,
			"",
		),
		// Everyone, without saying so.
		(
			json!({
				"name": "To all",
				"spec": { "category": "news", "title": "Hello" },
				"target": "everyone",
			}),
			403,
			"broadcast",
		),
	] {
		let saved = create(service, draft.clone(), ADA).await;
		let id = id_of(&saved).to_owned();
		let (answered, body) = launch(service, &id, 1, ADA).await;
		assert_eq!(answered, status, "{draft}: {body}");
		assert!(body.to_string().contains(says), "{draft}: {body}");

		let still = app(service, "campaign", json!({ "id": id })).await;
		assert_eq!(
			still["state"], "draft",
			"a refused launch leaves a draft: {still}"
		);
		assert!(still.get("send_key").is_none(), "{still}");
		assert_eq!(event_kinds(service, &id).await, ["created"]);
	}

	assert_eq!(
		counted(&service.db, "SELECT count(*) FROM sends".to_owned()).await,
		0,
		"no refused launch made a send"
	);

	// Everyone, said out loud.
	let all = create(
		service,
		json!({
			"name": "To all",
			"spec": { "category": "news", "title": "Hello" },
			"target": "everyone",
		}),
		ADA,
	)
	.await;
	let id = id_of(&all).to_owned();
	let (status, body) = call(
		service,
		"launch_campaign",
		json!({ "id": id, "revision": 1, "by": ADA, "broadcast": true }),
		As::App,
	)
	.await;
	assert_eq!(status, 200, "{body}");
	let launched: Value = serde_json::from_str(&body).unwrap();
	assert_eq!(launched["send"]["state"], "fanning_out");
	let (events, _) = every(service, "campaign_events", json!({ "id": id }), 10).await;
	assert_eq!(events[1]["kind"], "launched");
	assert!(
		events[1]["detail"].as_str().unwrap().contains("broadcast"),
		"{events:?}"
	);
}

/// A draft scheduled for a time that has passed is refused at launch —
/// `send` would treat it as now — and stays a draft; given a time ahead
/// it launches, and its rows wait for that time.
pub async fn a_past_schedule_is_refused_and_a_future_one_is_kept(service: &Service) {
	let people = [person(721), person(722)];
	let mut draft = ready("Tuesday's news", &people);
	draft["spec"]["visible_at"] = json!("2020-01-07T09:00:00Z");
	let saved = create(service, draft.clone(), ADA).await;
	let id = id_of(&saved).to_owned();

	let (status, body) = launch(service, &id, 1, ADA).await;
	assert_eq!(status, 400, "{body}");
	assert!(body.to_string().contains("has passed"), "{body}");
	assert_eq!(
		app(service, "campaign", json!({ "id": id })).await["state"],
		"draft"
	);
	assert_eq!(
		counted(&service.db, "SELECT count(*) FROM notifications".to_owned()).await,
		0,
		"nothing went out"
	);

	// A new time, an hour ahead.
	let later = hours_from_now(1);
	draft["spec"]["visible_at"] = json!(later);
	app(
		service,
		"update_campaign",
		json!({ "id": id, "revision": 1, "draft": draft, "by": ADA }),
	)
	.await;
	let (status, launched) = launch(service, &id, 2, ADA).await;
	assert_eq!(status, 200, "{launched}");
	assert_eq!(launched["send"]["delivered"], 2);

	// The rows are written and wait: nobody's inbox shows one yet.
	let key = launched["campaign"]["send_key"]
		.as_str()
		.unwrap()
		.to_owned();
	assert_eq!(key, format!("campaign:{id}:r2"));
	assert_eq!(
		counted(
			&service.db,
			format!("SELECT count(*) FROM notifications WHERE send_key = '{key}'")
		)
		.await,
		2
	);
	let inbox = as_person(
		service,
		people[0],
		"inbox",
		json!({ "page": { "limit": 10 } }),
	)
	.await;
	assert_eq!(inbox["items"], json!([]), "not due yet");
}

/// A scheduled campaign is cancelled before it is due, once, and answers
/// the same when asked again; a campaign people have already seen is
/// refused and stays launched; a draft has nothing to cancel.
#[expect(
	clippy::cognitive_complexity,
	reason = "three campaigns, each at a different point of its life"
)]
pub async fn a_cancel_stops_a_scheduled_campaign_and_not_a_seen_one(service: &Service) {
	let people = [person(731), person(732)];

	// Scheduled an hour ahead, then cancelled.
	let mut draft = ready("Later", &people);
	draft["spec"]["visible_at"] = json!(hours_from_now(1));
	let saved = create(service, draft, ADA).await;
	let id = id_of(&saved).to_owned();
	let (status, launched) = launch(service, &id, 1, ADA).await;
	assert_eq!(status, 200, "{launched}");
	let key = launched["campaign"]["send_key"]
		.as_str()
		.unwrap()
		.to_owned();

	let cancelled = app(service, "cancel_campaign", json!({ "id": id, "by": GRACE })).await;
	assert_eq!(cancelled["state"], "cancelled");
	assert_eq!(cancelled["cancelled_by"], GRACE);
	assert_eq!(cancelled["launched_by"], ADA);
	assert_eq!(
		counted(
			&service.db,
			format!(
				"SELECT count(*) FROM notifications WHERE send_key = '{key}' AND cancelled_at IS \
				 NOT NULL"
			)
		)
		.await,
		2,
		"both rows are stopped"
	);

	// Asked again, by someone else: the same answer, and one event.
	let again = app(service, "cancel_campaign", json!({ "id": id, "by": ADA })).await;
	assert_eq!(again, cancelled);
	assert_eq!(
		event_kinds(service, &id).await,
		["created", "launched", "cancelled"]
	);
	let (events, _) = every(service, "campaign_events", json!({ "id": id }), 10).await;
	assert_eq!(events[2]["by"], GRACE);
	assert_eq!(events[2]["detail"], "2 rows stopped");

	// A cancelled campaign is not launched again.
	let (status, body) = launch(service, &id, 1, ADA).await;
	assert_eq!(status, 409, "{body}");

	// Sent now, and so already in people's inboxes: past cancelling.
	let seen = create(service, ready("Now", &people), ADA).await;
	let seen_id = id_of(&seen).to_owned();
	let (status, body) = launch(service, &seen_id, 1, ADA).await;
	assert_eq!(status, 200, "{body}");
	let (status, body) = call(
		service,
		"cancel_campaign",
		json!({ "id": seen_id, "by": GRACE }),
		As::App,
	)
	.await;
	assert_eq!(status, 409, "{body}");
	let still = app(service, "campaign", json!({ "id": seen_id })).await;
	assert_eq!(still["state"], "launched", "{still}");
	assert!(still.get("cancelled_by").is_none(), "{still}");
	assert_eq!(
		event_kinds(service, &seen_id).await,
		["created", "launched"]
	);

	// A draft was never sent.
	let draft = create(service, ready("Unsent", &people), ADA).await;
	let (status, body) = call(
		service,
		"cancel_campaign",
		json!({ "id": id_of(&draft), "by": GRACE }),
		As::App,
	)
	.await;
	assert_eq!(status, 409, "{body}");
	assert!(body.contains("never launched"), "{body}");
}

/// A copy is a new draft at revision 1 with the message and the audience
/// and without the schedule, of a campaign or of a send anyone made; a
/// recipients send's copy has no audience, because the service keeps no
/// list of who a send named.
#[expect(
	clippy::cognitive_complexity,
	reason = "a copy of a campaign and of each kind of send"
)]
pub async fn a_duplicate_is_a_new_draft_without_the_schedule(service: &Service) {
	let people = [person(741), person(742)];
	let mut draft = ready("March newsletter", &people);
	draft["spec"]["visible_at"] = json!(hours_from_now(1));
	draft["spec"]["expires_at"] = json!(hours_from_now(48));
	draft["spec"]["label"] = json!("newsletter");
	draft["spec"]["options"] = json!({ "link": "/news/march" });
	draft["note"] = json!("Goes out on the first.");
	let original = create(service, draft, ADA).await;
	let id = id_of(&original).to_owned();
	let (status, body) = launch(service, &id, 1, ADA).await;
	assert_eq!(status, 200, "{body}");

	let copy = app(
		service,
		"duplicate_campaign",
		json!({ "id": id, "by": GRACE }),
	)
	.await;
	assert_ne!(copy["id"], original["id"]);
	assert_eq!(copy["state"], "draft");
	assert_eq!(copy["revision"], 1);
	assert_eq!(copy["created_by"], GRACE);
	assert_eq!(copy["draft"]["name"], "Copy of March newsletter");
	assert_eq!(copy["draft"]["note"], "Goes out on the first.");
	assert_eq!(copy["draft"]["spec"]["title"], "March newsletter");
	assert_eq!(copy["draft"]["spec"]["label"], "newsletter");
	assert_eq!(copy["draft"]["spec"]["options"]["link"], "/news/march");
	assert_eq!(copy["draft"]["target"], json!({ "recipients": people }));
	assert!(copy["draft"]["spec"]["visible_at"].is_null(), "{copy}");
	assert!(copy["draft"]["spec"]["expires_at"].is_null(), "{copy}");
	assert!(copy.get("send_key").is_none(), "{copy}");
	assert!(copy.get("launched_at").is_none(), "{copy}");

	let (events, _) = every(
		service,
		"campaign_events",
		json!({ "id": id_of(&copy) }),
		10,
	)
	.await;
	assert_eq!(events.len(), 1);
	assert_eq!(events[0]["kind"], "created");
	assert_eq!(events[0]["detail"], format!("a copy of campaign {id}"));

	// The original is as it was.
	assert_eq!(
		app(service, "campaign", json!({ "id": id })).await["state"],
		"launched"
	);

	// A send the app's server made, to a topic, scheduled.
	let mut scheduled = news("Server-made");
	scheduled["visible_at"] = json!(hours_from_now(2));
	send(
		service,
		"server-topic",
		scheduled,
		json!({ "topic": "news.all" }),
	)
	.await;
	let from_send = app(
		service,
		"duplicate_send",
		json!({ "idempotency_key": "server-topic", "by": ADA }),
	)
	.await;
	assert_eq!(from_send["state"], "draft");
	assert_eq!(from_send["draft"]["name"], "Copy of Server-made");
	assert_eq!(from_send["draft"]["spec"]["title"], "Server-made");
	assert_eq!(from_send["draft"]["target"], json!({ "topic": "news.all" }));
	assert!(
		from_send["draft"]["spec"]["visible_at"].is_null(),
		"{from_send}"
	);

	// A recipients send: the message, and no audience.
	send(
		service,
		"server-recipients",
		news("To a few"),
		json!({ "recipients": people }),
	)
	.await;
	let few = app(
		service,
		"duplicate_send",
		json!({ "idempotency_key": "server-recipients", "by": ADA }),
	)
	.await;
	assert_eq!(few["draft"]["spec"]["title"], "To a few");
	assert!(few["draft"].get("target").is_none(), "{few}");

	// An unknown campaign or send is not found.
	let (status, body) = call(
		service,
		"duplicate_campaign",
		json!({ "id": Uuid::nil(), "by": ADA }),
		As::App,
	)
	.await;
	assert_eq!(status, 404, "{body}");
	let (status, body) = call(
		service,
		"duplicate_send",
		json!({ "idempotency_key": "never-sent", "by": ADA }),
		As::App,
	)
	.await;
	assert_eq!(status, 404, "{body}");
}

/// A test pushes the draft's message to one person's device now, whatever
/// its schedule, and writes no inbox row and no delivery record; it
/// records an event that says how the push ended and does not name the
/// person. The campaign stays the draft it was.
#[expect(
	clippy::cognitive_complexity,
	reason = "one test followed to the provider, the tables, and the record"
)]
pub async fn a_test_writes_no_inbox_row_and_records_an_event(service: &Service) {
	let tester = person(751);
	let endpoint = enroll(service, tester, "7e57", json!({})).await;
	let mut draft = ready("Try me", &[person(752)]);
	draft["spec"]["visible_at"] = json!(hours_from_now(3));
	let saved = create(service, draft, ADA).await;
	let id = id_of(&saved).to_owned();

	let outcomes = app(
		service,
		"test_campaign",
		json!({ "id": id, "person": tester, "by": GRACE }),
	)
	.await;
	assert_eq!(
		outcomes,
		json!([{
			"endpoint": endpoint,
			"platform": "apns",
			"state": "accepted",
			"reason": null,
		}])
	);

	for table in ["notifications", "push_deliveries", "sends"] {
		assert_eq!(
			counted(&service.db, format!("SELECT count(*) FROM {table}")).await,
			0,
			"a test writes nothing to `{table}`"
		);
	}

	let (events, _) = every(service, "campaign_events", json!({ "id": id }), 10).await;
	assert_eq!(events.len(), 2, "{events:?}");
	assert_eq!(events[1]["kind"], "tested");
	assert_eq!(events[1]["by"], GRACE);
	assert_eq!(events[1]["revision"], 1);
	assert_eq!(events[1]["detail"], "apns accepted");
	let recorded = serde_json::to_string(&events).unwrap();
	assert!(
		!recorded.contains(&tester.to_string()) && !recorded.contains(&endpoint.to_string()),
		"an event names neither the person nor the device: {recorded}"
	);

	let after = app(service, "campaign", json!({ "id": id })).await;
	assert_eq!(after, saved, "a test changes nothing about the campaign");

	// A person with no device: nothing to push to, and that is recorded.
	let nobody = app(
		service,
		"test_campaign",
		json!({ "id": id, "person": person(753), "by": GRACE }),
	)
	.await;
	assert_eq!(nobody, json!([]));
	let (events, _) = every(service, "campaign_events", json!({ "id": id }), 10).await;
	assert_eq!(events[2]["detail"], "no device to push to");

	// An unknown campaign is not found, and a draft that can't be sent
	// can't be tested.
	let (status, body) = call(
		service,
		"test_campaign",
		json!({ "id": Uuid::nil(), "person": tester, "by": GRACE }),
		As::App,
	)
	.await;
	assert_eq!(status, 404, "{body}");
	let untitled = create(service, json!({ "name": "Untitled" }), ADA).await;
	let (status, body) = call(
		service,
		"test_campaign",
		json!({ "id": id_of(&untitled), "person": tester, "by": GRACE }),
		As::App,
	)
	.await;
	assert_eq!(status, 400, "{body}");
}

/// An archived campaign can be read and copied and nothing else; asked
/// again, archiving answers the same; a stale revision is refused.
pub async fn an_archived_campaign_is_put_away(service: &Service) {
	let people = [person(761)];
	let saved = create(service, ready("Old idea", &people), ADA).await;
	let id = id_of(&saved).to_owned();

	let (status, body) = call(
		service,
		"archive_campaign",
		json!({ "id": id, "revision": 7, "by": GRACE }),
		As::App,
	)
	.await;
	assert_eq!(status, 409, "{body}");

	let archived = app(
		service,
		"archive_campaign",
		json!({ "id": id, "revision": 1, "by": GRACE }),
	)
	.await;
	assert_eq!(archived["state"], "archived");
	assert_eq!(archived["updated_by"], GRACE);
	let again = app(
		service,
		"archive_campaign",
		json!({ "id": id, "revision": 1, "by": ADA }),
	)
	.await;
	assert_eq!(again, archived);
	assert_eq!(event_kinds(service, &id).await, ["created", "archived"]);

	for (operation, args) in [
		(
			"update_campaign",
			json!({ "id": id, "revision": 1, "draft": { "name": "Back" }, "by": ADA }),
		),
		(
			"launch_campaign",
			json!({ "id": id, "revision": 1, "by": ADA }),
		),
		("cancel_campaign", json!({ "id": id, "by": ADA })),
	] {
		let (status, body) = call(service, operation, args, As::App).await;
		assert_eq!(status, 409, "{operation}: {body}");
		assert!(body.contains("archived"), "{operation}: {body}");
	}

	let copy = app(
		service,
		"duplicate_campaign",
		json!({ "id": id, "by": ADA }),
	)
	.await;
	assert_eq!(copy["state"], "draft");

	let (archived, _) = every(
		service,
		"campaigns",
		json!({ "filter": { "state": "archived" } }),
		10,
	)
	.await;
	assert_eq!(field(&archived, "name"), ["Old idea"]);
}

/// Only the app's server reads or writes a campaign: a signed-in person
/// and an anonymous caller are refused on every operation.
pub async fn only_the_app_touches_campaigns(service: &Service) {
	let alice = person(771);
	let saved = create(service, ready("Private", &[alice]), ADA).await;
	let id = id_of(&saved).to_owned();
	send(
		service,
		"a-send",
		news("A send"),
		json!({ "topic": "news.all" }),
	)
	.await;

	for (operation, args) in [
		(
			"create_campaign",
			json!({ "draft": { "name": "Mine" }, "by": "me" }),
		),
		(
			"update_campaign",
			json!({ "id": id, "revision": 1, "draft": { "name": "Mine" }, "by": "me" }),
		),
		("campaign", json!({ "id": id })),
		("campaigns", json!({ "page": { "limit": 5 } })),
		("duplicate_campaign", json!({ "id": id, "by": "me" })),
		(
			"duplicate_send",
			json!({ "idempotency_key": "a-send", "by": "me" }),
		),
		(
			"archive_campaign",
			json!({ "id": id, "revision": 1, "by": "me" }),
		),
		(
			"launch_campaign",
			json!({ "id": id, "revision": 1, "by": "me" }),
		),
		("cancel_campaign", json!({ "id": id, "by": "me" })),
		(
			"test_campaign",
			json!({ "id": id, "person": alice, "by": "me" }),
		),
		(
			"campaign_events",
			json!({ "id": id, "page": { "limit": 5 } }),
		),
	] {
		for caller in [As::Nobody, As::Person(alice)] {
			let (status, body) = call(service, operation, args.clone(), caller).await;
			assert_eq!(status, 403, "{operation}: {body}");
		}
	}

	// Nothing a refused caller asked for happened.
	let after = app(service, "campaign", json!({ "id": id })).await;
	assert_eq!(after, saved);
	assert_eq!(
		counted(&service.db, "SELECT count(*) FROM campaigns".to_owned()).await,
		1
	);
	assert_eq!(event_kinds(service, &id).await, ["created"]);
}

/// What a caller can grow is capped and refused by name: the number of
/// drafts, the tests one campaign records, and the size of a page.
pub async fn campaigns_are_bounded(service: &Service) {
	let sql = Sql(service.db.dialect());
	let now = sql.timestamp(&chrono::Utc::now().to_rfc3339());
	let saved = create(service, ready("The one", &[person(781)]), ADA).await;
	let id = id_of(&saved).to_owned();
	let id_literal = sql.uuid(id.parse().unwrap());

	// 999 more drafts, written directly: the thousandth is the last.
	let rows: Vec<String> = (0..999)
		.map(|n| {
			format!(
				"({}, 1, 'draft', 'draft {n}', '{{\"category\":\"\",\"title\":\"\"}}', 'seed', \
				 'seed', {now}, {now})",
				sql.new_id()
			)
		})
		.collect();

	for chunk in rows.chunks(250) {
		run(
			&service.db,
			format!(
				"INSERT INTO campaigns (id, revision, state, name, spec, created_by, updated_by, \
				 created_at, updated_at) VALUES {}",
				chunk.join(", ")
			),
		)
		.await;
	}

	let (status, body) = call(
		service,
		"create_campaign",
		json!({ "draft": { "name": "One too many" }, "by": ADA }),
		As::App,
	)
	.await;
	assert_eq!(status, 400, "{body}");
	assert!(body.contains("1000 draft campaigns"), "{body}");
	let (status, body) = call(
		service,
		"duplicate_campaign",
		json!({ "id": id, "by": ADA }),
		As::App,
	)
	.await;
	assert_eq!(status, 400, "{body}");

	// A page of the list holds at most 50, however many are asked for,
	// and the whole list is every campaign once.
	let asked = page(service, "campaigns", json!({}), json!({ "limit": 500 })).await;
	assert_eq!(asked["items"].as_array().unwrap().len(), 50);
	assert_eq!(asked["has_more"], true);
	let (all, pages) = every(service, "campaigns", json!({}), 50).await;
	assert_eq!(all.len(), 1_000);
	assert_eq!(pages, 20);
	let mut ids = field(&all, "id");
	ids.sort_unstable();
	ids.dedup();
	assert_eq!(ids.len(), 1_000, "each campaign once");

	// A hundred recorded tests is as many as one campaign keeps.
	let events: Vec<String> = (0..100)
		.map(|_| {
			format!(
				"({}, {id_literal}, 'tested', 'seed', 1, {now})",
				sql.new_id()
			)
		})
		.collect();
	run(
		&service.db,
		format!(
			"INSERT INTO campaign_events (id, campaign, kind, by, revision, created_at) VALUES {}",
			events.join(", ")
		),
	)
	.await;
	let (status, body) = call(
		service,
		"test_campaign",
		json!({ "id": id, "person": person(781), "by": ADA }),
		As::App,
	)
	.await;
	assert_eq!(status, 400, "{body}");
	assert!(body.contains("100 tests"), "{body}");

	// A page of events holds at most 100.
	let asked = page(
		service,
		"campaign_events",
		json!({ "id": id }),
		json!({ "limit": 5_000 }),
	)
	.await;
	assert_eq!(asked["items"].as_array().unwrap().len(), 100);
	assert_eq!(asked["has_more"], true);
}

/// Eight launches of the same revision at once make one send: every one
/// of them succeeds, exactly one is not a duplicate, and one send, one
/// set of rows, and one launch event are stored.
pub async fn racing_launches_make_one_send(service: &Service) {
	let people: Vec<Uuid> = (791..796).map(person).collect();
	let saved = create(service, ready("All at once", &people), ADA).await;
	let id = id_of(&saved).to_owned();

	let answers = futures::future::join_all((0..8).map(|n| {
		let id = id.clone();

		async move { launch(service, &id, 1, &format!("presser-{n}")).await }
	}))
	.await;

	let mut made = 0;

	for (status, body) in &answers {
		assert_eq!(*status, 200, "{body}");
		assert_eq!(body["send"]["delivered"], 5, "{body}");
		assert_eq!(body["campaign"]["state"], "launched");

		if body["send"]["duplicate"] == false {
			made += 1;
		}
	}

	assert_eq!(made, 1, "exactly one launch made the send: {answers:?}");

	let key = format!("campaign:{id}:r1");
	assert_eq!(
		counted(
			&service.db,
			format!("SELECT count(*) FROM sends WHERE idempotency_key = '{key}'")
		)
		.await,
		1
	);
	assert_eq!(
		counted(
			&service.db,
			format!("SELECT count(*) FROM notifications WHERE send_key = '{key}'")
		)
		.await,
		5
	);
	assert_eq!(event_kinds(service, &id).await, ["created", "launched"]);

	// Everyone was told the same launcher.
	let by = answers[0].1["campaign"]["launched_by"].clone();
	assert!(
		answers
			.iter()
			.all(|(_, body)| body["campaign"]["launched_by"] == by),
		"{answers:?}"
	);
}

/// Eight saves made from the same revision at once: one is kept, seven
/// are refused as conflicts, and the campaign is one revision on, holding
/// the winner's draft whole.
pub async fn racing_saves_keep_one(service: &Service) {
	let saved = create(service, json!({ "name": "Shared draft" }), ADA).await;
	let id = id_of(&saved).to_owned();

	let answers = futures::future::join_all((0..8).map(|n| {
		let id = id.clone();

		async move {
			call(
				service,
				"update_campaign",
				json!({
					"id": id,
					"revision": 1,
					"draft": {
						"name": format!("Version {n}"),
						"spec": { "category": "news", "title": format!("Title {n}") },
						"note": format!("Note {n}"),
					},
					"by": format!("editor-{n}"),
				}),
				As::App,
			)
			.await
		}
	}))
	.await;

	let kept: Vec<&(u16, String)> = answers
		.iter()
		.filter(|(status, _)| *status == 200)
		.collect();
	let refused = answers.iter().filter(|(status, _)| *status == 409).count();
	assert_eq!(kept.len(), 1, "{answers:?}");
	assert_eq!(refused, 7, "{answers:?}");

	let winner: Value = serde_json::from_str(&kept[0].1).unwrap();
	let now = app(service, "campaign", json!({ "id": id })).await;
	assert_eq!(now, winner);
	assert_eq!(now["revision"], 2);
	let n = now["draft"]["name"]
		.as_str()
		.unwrap()
		.strip_prefix("Version ")
		.unwrap()
		.to_owned();
	assert_eq!(now["draft"]["spec"]["title"], format!("Title {n}"));
	assert_eq!(now["draft"]["note"], format!("Note {n}"));
	assert_eq!(now["updated_by"], format!("editor-{n}"));
}
