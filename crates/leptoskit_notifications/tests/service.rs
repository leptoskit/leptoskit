//! The service on a real port: who may call what (the app only sends;
//! a forwarded person reads only their own inbox), idempotency across a
//! restart — before and after a send's expiry — bounds, scheduled
//! visibility with cancel-before-due semantics (including a send with no
//! delivered rows), expiry, read and dismissal isolation between people,
//! preferences at send and at visibility, bounded pagination, concurrent
//! duplicate sends, and the migrations themselves. The live socket's own
//! tests are in `live.rs`.

#![cfg(not(target_arch = "wasm32"))]

use std::time::Duration;
use std::time::SystemTime;

use leptoskit_db::Db;
use leptoskit_db::Read as _;
use leptoskit_db::Statement;
use leptoskit_notifications::MAX_DATA_LEN;
use leptoskit_notifications::MAX_KEY_LEN;
use leptoskit_notifications::MAX_RECIPIENTS;
use leptoskit_notifications::MAX_SCHEDULE_SECS;
use leptoskit_notifications::MAX_TITLE_LEN;
use leptoskit_notifications::MIGRATIONS;
use leptoskit_notifications::SERVICE;
use leptoskit_notifications::SendResult;
use leptoskit_rpc as rpc;
use rpc::server::Anonymous;
use rpc::server::ServiceCallers;
use serde_json::Value;
use serde_json::json;
use uuid::Uuid;

const TOKEN: &str = "app-token";
const ORIGIN: &str = "http://app.test";
const PREFIX: &str = "/_leptoskit/services/notifications";

/// Two people the app knows, fixed so tests can name who they expect.
const ALICE: Uuid = Uuid::from_u128(0xA110_0000_0000_0000_0000_0000_0000_0001);
const BOB: Uuid = Uuid::from_u128(0xB0B0_0000_0000_0000_0000_0000_0000_0002);

struct Service {
	address: std::net::SocketAddr,
	db: Db,
	/// The service's job workers, stopped with the service so no test
	/// leaves one polling behind it.
	jobs: rpc::server::JobsHandle,
}

/// A fresh, migrated service on a free port, knowing `TOKEN` as the app and
/// forwarding identity as `lk` does for a first-party service. Its job
/// workers run on a test-fast poll, so a schedule's wake lands soon after
/// its time.
async fn start(db: Db) -> Service {
	let callers = ServiceCallers::new(Anonymous)
		.caller(TOKEN, rpc::server::APP_CALLER)
		.forwarded_identity(SERVICE, TOKEN);
	let (router, live) =
		leptoskit_notifications::service::routes_with(db.clone(), Some(ORIGIN), callers);
	live.start().await.expect("live starts");
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let address = listener.local_addr().unwrap();
	tokio::spawn(async move {
		leptoskit_axum::axum::serve(listener, router).await.unwrap();
	});
	let jobs = leptoskit_notifications::service::workers(db.clone())
		.poll(Duration::from_millis(50))
		.start();
	Service { address, db, jobs }
}

impl Drop for Service {
	fn drop(&mut self) {
		self.jobs.stop();
	}
}

async fn service() -> Service {
	let db = leptoskit_db::testing::sqlite_migrated(MIGRATIONS).await;
	start(db).await
}

/// The token the app's server would mint for `person`, as it does when a
/// signed-in browser or device asks for one.
fn signed_in(person: Uuid) -> String {
	signed_in_at(person, SystemTime::now())
}

/// The same, minted at `issued_at`, so tests can hold two devices' tokens
/// apart in time.
fn signed_in_at(person: Uuid, issued_at: SystemTime) -> String {
	rpc::server::IdentityKey::new(SERVICE, TOKEN)
		.expect("a key")
		.mint(person, issued_at, rpc::server::IDENTITY_LIFETIME)
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

/// `spec(category, title)` is the common send; named arguments add the rest.
fn spec(category: &str, title: &str) -> Value {
	json!({ "category": category, "title": title })
}

async fn send(service: &Service, key: &str, spec: Value, recipients: &[Uuid]) -> (u16, String) {
	post(
		service,
		"send",
		json!({
			"idempotency_key": key,
			"spec": spec,
			"target": { "recipients": recipients },
		}),
		Some(TOKEN),
	)
	.await
}

async fn inbox(service: &Service, token: &str, limit: u32, show: Value) -> (u16, Value) {
	let (status, body) = post(
		service,
		"inbox",
		json!({ "page": { "limit": limit }, "show": show }),
		Some(token),
	)
	.await;
	let parsed: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
	(status, parsed)
}

async fn unread(service: &Service, token: &str) -> (u16, u64) {
	let (status, body) = post(service, "unread_count", json!({}), Some(token)).await;
	(status, serde_json::from_str(&body).unwrap_or(0))
}

async fn first_item_id(service: &Service, token: &str) -> Option<Uuid> {
	let (status, page) = inbox(service, token, 1, json!("active")).await;
	assert_eq!(status, 200, "{page}");
	page["items"]
		.as_array()
		.and_then(|items| items.first())
		.and_then(|item| item["id"].as_str())
		.map(|id| Uuid::parse_str(id).expect("an id"))
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

#[tokio::test(flavor = "multi_thread")]
async fn only_the_app_sends_and_cancels() {
	let service = service().await;

	let (status, _) = post(
		&service,
		"send",
		json!({
			"idempotency_key": "anon",
			"spec": spec("news", "hello"),
			"target": { "recipients": [ALICE] },
		}),
		None,
	)
	.await;
	assert_eq!(status, 403, "an anonymous caller can't send");

	let (status, _) = post(
		&service,
		"send",
		json!({
			"idempotency_key": "wrong-token",
			"spec": spec("news", "hello"),
			"target": { "recipients": [ALICE] },
		}),
		Some("not-the-token"),
	)
	.await;
	assert_eq!(status, 403, "nor a caller with the wrong token");

	let (status, body) = send(&service, "app-1", spec("news", "hello"), &[ALICE]).await;
	assert_eq!(status, 200, "{body}");
	let sent: SendResult = serde_json::from_str(&body).unwrap();
	assert_eq!(
		(
			sent.delivered,
			sent.skipped_opted_out,
			sent.recipients,
			sent.duplicate
		),
		(1, 0, 1, false)
	);

	for (caller, name) in [
		(None, "anonymous"),
		(Some("not-the-token"), "a wrong token"),
	] {
		let (status, _) = post(
			&service,
			"cancel",
			json!({ "idempotency_key": "app-1" }),
			caller,
		)
		.await;
		assert_eq!(status, 403, "{name} can't cancel");
	}

	let (status, body) = post(
		&service,
		"cancel",
		json!({ "idempotency_key": "never-sent" }),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 404, "an unknown key cancels nothing: {body}");

	let stored = rows(&service, "SELECT count(*) FROM notifications").await;
	assert_eq!(text(&stored[0][0]), "1", "the refused calls stored nothing");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_forwarded_person_reads_only_their_own_inbox() {
	let service = service().await;

	let (status, body) = send(
		&service,
		"to-both",
		json!({
			"category": "news",
			"title": "It ships",
			"body": "Notifications are here.",
			"data": json!({ "screen": "inbox" }).to_string(),
		}),
		&[ALICE, BOB],
	)
	.await;
	assert_eq!(status, 200, "{body}");

	let alice = signed_in(ALICE);
	let bob = signed_in(BOB);

	for (status, body, name) in [
		(
			post(&service, "inbox", json!({ "page": { "limit": 10 } }), None)
				.await
				.0,
			String::new(),
			"an anonymous call",
		),
		(
			post(
				&service,
				"inbox",
				json!({ "page": { "limit": 10 } }),
				Some("garbage"),
			)
			.await
			.0,
			String::new(),
			"a made-up token",
		),
	] {
		assert_eq!(status, 403, "{name} reads nobody's inbox: {body}");
	}

	let (status, page) = inbox(&service, &alice, 10, json!("active")).await;
	assert_eq!(status, 200, "{page}");
	assert_eq!(page["items"].as_array().map(Vec::len), Some(1), "{page}");
	let item = &page["items"][0];
	assert_eq!(item["category"], "news");
	assert_eq!(item["title"], "It ships");
	assert_eq!(item["body"], "Notifications are here.");
	assert_eq!(
		item["data"],
		json!({ "screen": "inbox" }).to_string(),
		"data rides as its JSON object text"
	);
	assert!(item["read_at"].is_null());
	assert!(item["dismissed_at"].is_null());

	let (status, count) = unread(&service, &alice).await;
	assert_eq!(status, 200);
	assert_eq!(count, 1, "{count}");

	let (status, page) = inbox(&service, &bob, 10, json!("active")).await;
	assert_eq!(status, 200, "{page}");
	assert_eq!(
		page["items"].as_array().map(Vec::len),
		Some(1),
		"bob has his own copy"
	);
	assert_eq!(page["items"][0]["title"], "It ships");
	assert_ne!(
		page["items"][0]["id"], item["id"],
		"one send, one row per person, each with its own id"
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn forged_identity_gets_nowhere() {
	let service = service().await;
	let (status, _) = send(&service, "for-alice", spec("news", "hello"), &[ALICE]).await;
	assert_eq!(status, 200);

	let alice = signed_in(ALICE);
	let claims = alice
		.strip_prefix("lkid1.")
		.and_then(|rest| rest.split_once('.'))
		.expect("the token's shape");
	let forged = format!("lkid1.{}.{}", "eyJhdWQiOiJub3RpZmljYXRpb25zIn0", claims.1);
	let headers = [
		("Authorization", format!("Bearer {forged}")),
		("x-user", ALICE.to_string()),
		("x-forwarded-person", ALICE.to_string()),
		("x-leptoskit-person", ALICE.to_string()),
	];

	for (name, value) in headers {
		let status = reqwest::Client::new()
			.post(format!("http://{}{PREFIX}/rpc/inbox", service.address))
			.json(&json!({ "page": { "limit": 10 } }))
			.header(name, value)
			.send()
			.await
			.unwrap()
			.status()
			.as_u16();
		assert!(
			matches!(status, 401 | 403),
			"a forged `{name}` header reads nobody's inbox, got {status}"
		);
	}

	// A token minted for another service doesn't work here, even with the
	// same shared token it derives from.
	let for_another_service = rpc::server::IdentityKey::new("analytics", TOKEN)
		.expect("a key")
		.mint(ALICE, SystemTime::now(), rpc::server::IDENTITY_LIFETIME);
	let (status, _) = post(
		&service,
		"inbox",
		json!({ "page": { "limit": 10 } }),
		Some(&for_another_service),
	)
	.await;
	assert_eq!(status, 401, "a token for `analytics` isn't one of ours");
}

#[tokio::test(flavor = "multi_thread")]
async fn people_cannot_read_or_touch_each_others_notifications() {
	let service = service().await;
	let (status, _) = send(&service, "for-alice", spec("news", "hello"), &[ALICE]).await;
	assert_eq!(status, 200);

	let alice = signed_in(ALICE);
	let bob = signed_in(BOB);

	let (status, page) = inbox(&service, &bob, 10, json!("active")).await;
	assert_eq!(status, 200, "{page}");
	assert_eq!(
		page["items"].as_array().map(Vec::len),
		Some(0),
		"bob sees none of it"
	);

	let (_, count) = unread(&service, &bob).await;
	assert_eq!(count, 0, "bob's badge doesn't count alice's");

	let Some(id) = first_item_id(&service, &alice).await else {
		panic!("alice has the item");
	};

	let (status, _) = post(
		&service,
		"mark_read",
		json!({ "notification": id }),
		Some(&bob),
	)
	.await;
	assert_eq!(status, 404, "bob can't read alice's notification by id");

	let (status, _) = post(
		&service,
		"mark_dismissed",
		json!({ "notification": id }),
		Some(&bob),
	)
	.await;
	assert_eq!(status, 404, "nor dismiss it");

	let (status, count) = unread(&service, &alice).await;
	assert_eq!(status, 200);
	assert_eq!(count, 1, "the row is untouched");
}

#[tokio::test(flavor = "multi_thread")]
async fn inbox_state_is_shared_across_devices() {
	let service = service().await;
	let (status, _) = send(&service, "everywhere", spec("news", "hello"), &[ALICE]).await;
	assert_eq!(status, 200);

	let phone = signed_in(ALICE);
	let laptop = signed_in_at(ALICE, SystemTime::now() - Duration::from_secs(1));
	assert_ne!(phone, laptop, "two sign-ins, one person");

	let (_, count) = unread(&service, &phone).await;
	assert_eq!(count, 1, "the phone sees it");

	let Some(id) = first_item_id(&service, &phone).await else {
		panic!("the phone has the item");
	};
	let (status, _) = post(
		&service,
		"mark_read",
		json!({ "notification": id }),
		Some(&phone),
	)
	.await;
	assert_eq!(status, 200, "{status}");

	let (_, count) = unread(&service, &laptop).await;
	assert_eq!(count, 0, "the laptop sees the same read state");

	let (_, page) = inbox(&service, &laptop, 10, json!("unread")).await;
	assert_eq!(page["items"].as_array().map(Vec::len), Some(0), "{page}");
	let (_, page) = inbox(&service, &laptop, 10, json!("all")).await;
	assert_eq!(
		page["items"].as_array().map(Vec::len),
		Some(1),
		"still there, read"
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_retried_send_answers_once_across_a_restart() {
	let file = tempfile::Builder::new()
		.prefix("notifications-restart-")
		.suffix(".sqlite3")
		.tempfile()
		.expect("a database file");
	let url = format!("sqlite://{}", file.path().display());

	// The first process: connect, migrate, serve, send.
	let db = Db::connect(&url).await.expect("the database opens");
	let applied = db.migrate(MIGRATIONS).await.expect("migrations apply");
	assert_eq!(applied.len(), 3, "the migrations apply: {applied:?}");
	let first = start(db).await;
	let (status, body) = send(
		&first,
		"retried",
		json!({
			"category": "news",
			"title": "Once",
			"body": "Only one copy.",
		}),
		&[ALICE, BOB],
	)
	.await;
	assert_eq!(status, 200, "{body}");
	let sent: SendResult = serde_json::from_str(&body).unwrap();
	assert_eq!(sent.delivered, 2);

	// A second process over the same database: the send row survives it.
	let db = Db::connect(&url).await.expect("the database opens again");
	let restarted = start(db).await;

	let (status, body) = send(
		&restarted,
		"retried",
		json!({
			"category": "news",
			"title": "Once",
			"body": "Only one copy.",
		}),
		&[ALICE, BOB],
	)
	.await;
	assert_eq!(status, 200, "{body}");
	let again: SendResult = serde_json::from_str(&body).unwrap();
	assert!(again.duplicate, "the restart is recognised");
	assert_eq!(again.delivered, 2, "the stored counts come back");

	let stored = rows(&restarted, "SELECT count(*) FROM notifications").await;
	assert_eq!(text(&stored[0][0]), "2", "no second copy was written");
}

/// An accepted send answers its stored outcome forever: expiry is part of
/// the intent, not a reason to stop answering it. A retry after the
/// expiry — across a restart, on the same file — returns the first run's
/// counts; a changed intent still conflicts; a new send with a past
/// expiry is still refused — including one whose explicit `visible_at` is
/// also past, where an expiry after that but before now would otherwise
/// slip through and be born never-visible.
#[tokio::test(flavor = "multi_thread")]
async fn an_accepted_send_answers_after_its_expiry_across_a_restart() {
	let file = tempfile::Builder::new()
		.prefix("notifications-expired-retry-")
		.suffix(".sqlite3")
		.tempfile()
		.expect("a database file");
	let url = format!("sqlite://{}", file.path().display());

	let db = Db::connect(&url).await.expect("the database opens");
	db.migrate(MIGRATIONS).await.expect("migrations apply");
	let first = start(db).await;

	let expiring = json!({
		"category": "news",
		"title": "Gone by the time you read this",
		"expires_at": (chrono::Utc::now() + chrono::Duration::seconds(1)).to_rfc3339(),
	});
	let (status, body) = send(&first, "fleeting", expiring.clone(), &[ALICE, BOB]).await;
	assert_eq!(status, 200, "{body}");
	let sent: SendResult = serde_json::from_str(&body).unwrap();
	assert_eq!(sent.delivered, 2);

	// The second process, after the send has expired.
	let restarted = start(Db::connect(&url).await.expect("the database opens again")).await;
	tokio::time::sleep(Duration::from_millis(1_300)).await;

	let (status, body) = send(&restarted, "fleeting", expiring.clone(), &[ALICE, BOB]).await;
	assert_eq!(status, 200, "the stored outcome, not the clock: {body}");
	let again: SendResult = serde_json::from_str(&body).unwrap();
	assert!(again.duplicate, "{body}");
	assert_eq!(again.delivered, 2, "the first run's counts come back");

	let (status, body) = send(
		&restarted,
		"fleeting",
		json!({
			"category": "news",
			"title": "A different send entirely",
			"expires_at": (chrono::Utc::now() + chrono::Duration::seconds(1)).to_rfc3339(),
		}),
		&[ALICE, BOB],
	)
	.await;
	assert_eq!(status, 409, "a changed intent is still a conflict: {body}");

	let (status, body) = send(
		&restarted,
		"born-expired",
		json!({
			"category": "news",
			"title": "Too late",
			"expires_at": (chrono::Utc::now() - chrono::Duration::seconds(1)).to_rfc3339(),
		}),
		&[ALICE],
	)
	.await;
	assert_eq!(status, 400, "a new send can't be born expired: {body}");

	// The slip the simple check misses: an explicit past `visible_at`
	// with a past `expires_at` after it satisfies `expires >
	// visible_at`, but the notification would never be visible at all.
	let (status, body) = send(
		&restarted,
		"born-expired-scheduled",
		json!({
			"category": "news",
			"title": "Never seen",
			"visible_at": (chrono::Utc::now() - chrono::Duration::hours(2)).to_rfc3339(),
			"expires_at": (chrono::Utc::now() - chrono::Duration::hours(1)).to_rfc3339(),
		}),
		&[ALICE],
	)
	.await;
	assert_eq!(
		status, 400,
		"a send whose expiry is already past is refused: {body}"
	);

	// And an accepted send still answers from its store, not the clock:
	// the same key with the same intent replays after the refusals.
	let (status, body) = send(&restarted, "fleeting", expiring.clone(), &[ALICE, BOB]).await;
	assert_eq!(status, 200, "{body}");
	let replay: SendResult = serde_json::from_str(&body).unwrap();
	assert!(replay.duplicate, "{body}");
}

/// A send every recipient opted out of delivered nothing, and cancelling
/// it is still cancelling it: the send's own row is what makes the key
/// known, so cancelling answers 0 idempotently instead of not found.
#[tokio::test(flavor = "multi_thread")]
async fn a_send_with_no_delivered_rows_still_cancels() {
	let service = service().await;
	let bob = signed_in(BOB);

	let (status, body) = post(
		&service,
		"set_category_preference",
		json!({ "category": "marketing", "opted_out": true }),
		Some(&bob),
	)
	.await;
	assert_eq!(status, 200, "{body}");

	let (status, body) = send(&service, "nobody-sees", spec("marketing", "A sale"), &[BOB]).await;
	assert_eq!(status, 200, "{body}");
	let sent: SendResult = serde_json::from_str(&body).unwrap();
	assert_eq!(
		(sent.delivered, sent.skipped_opted_out),
		(0, 1),
		"bob opted out, so nothing was delivered"
	);

	let (status, body) = post(
		&service,
		"cancel",
		json!({ "idempotency_key": "nobody-sees" }),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "a known zero-fanout send cancels: {body}");
	assert_eq!(body, "0", "there were no rows to cancel");

	let (status, body) = post(
		&service,
		"cancel",
		json!({ "idempotency_key": "nobody-sees" }),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "idempotently: {body}");
	assert_eq!(body, "0");

	let (status, body) = post(
		&service,
		"cancel",
		json!({ "idempotency_key": "never-sent" }),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 404, "an unknown key is still not found: {body}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_same_key_with_a_different_intent_is_a_conflict() {
	let service = service().await;
	let (status, _) = send(&service, "one-send", spec("news", "First"), &[ALICE]).await;
	assert_eq!(status, 200);

	let (status, body) = send(&service, "one-send", spec("news", "Second"), &[ALICE]).await;
	assert_eq!(status, 409, "{body}");

	let (status, body) = send(&service, "one-send", spec("offers", "First"), &[ALICE]).await;
	assert_eq!(status, 409, "{body}");

	let (status, body) = send(&service, "one-send", spec("news", "First"), &[BOB]).await;
	assert_eq!(status, 409, "{body}");

	let stored = rows(&service, "SELECT title, person FROM notifications").await;
	assert_eq!(stored.len(), 1, "nothing conflicting was written");
	assert_eq!(text(&stored[0][0]), "First");
}

#[tokio::test(flavor = "multi_thread")]
async fn bounds_are_enforced() {
	let service = service().await;
	let far = chrono::Utc::now() + chrono::Duration::seconds(MAX_SCHEDULE_SECS + 60);

	let refusals: Vec<(&str, Value)> = vec![
		(
			"a blank key",
			json!({
				"idempotency_key": " ",
				"spec": spec("news", "hello"),
				"target": { "recipients": [ALICE] },
			}),
		),
		(
			"a key that is too long",
			json!({
				"idempotency_key": "k".repeat(MAX_KEY_LEN + 1),
				"spec": spec("news", "hello"),
				"target": { "recipients": [ALICE] },
			}),
		),
		(
			"a blank title",
			json!({
				"idempotency_key": "blank-title",
				"spec": spec("news", " "),
				"target": { "recipients": [ALICE] },
			}),
		),
		(
			"a title that is too long",
			json!({
				"idempotency_key": "long-title",
				"spec": json!({ "category": "news", "title": "x".repeat(MAX_TITLE_LEN + 1) }),
				"target": { "recipients": [ALICE] },
			}),
		),
		(
			"a body with a control character",
			json!({
				"idempotency_key": "control-body",
				"spec": json!({ "category": "news", "title": "hello", "body": "a\nb" }),
				"target": { "recipients": [ALICE] },
			}),
		),
		(
			"data that isn't JSON",
			json!({
				"idempotency_key": "bad-data",
				"spec": json!({ "category": "news", "title": "hello", "data": "{not json" }),
				"target": { "recipients": [ALICE] },
			}),
		),
		(
			"data that isn't an object",
			json!({
				"idempotency_key": "array-data",
				"spec": json!({ "category": "news", "title": "hello", "data": "[1]" }),
				"target": { "recipients": [ALICE] },
			}),
		),
		(
			"data that is too large",
			json!({
				"idempotency_key": "large-data",
				"spec": json!({
					"category": "news",
					"title": "hello",
					"data": json!({ "x": "x".repeat(MAX_DATA_LEN) }).to_string(),
				}),
				"target": { "recipients": [ALICE] },
			}),
		),
		(
			"no recipients",
			json!({
				"idempotency_key": "no-recipients",
				"spec": spec("news", "hello"),
				"target": { "recipients": [] },
			}),
		),
		(
			"too many recipients",
			json!({
				"idempotency_key": "too-many",
				"spec": spec("news", "hello"),
				"target": {
					"recipients": (0..=MAX_RECIPIENTS)
						.map(|n| Uuid::from_u128(u128::try_from(n).expect("a small index")))
						.collect::<Vec<_>>(),
				},
			}),
		),
		(
			"a schedule too far ahead",
			json!({
				"idempotency_key": "too-far",
				"spec": json!({ "category": "news", "title": "hello", "visible_at": far.to_rfc3339() }),
				"target": { "recipients": [ALICE] },
			}),
		),
		(
			"an expiry before visibility",
			json!({
				"idempotency_key": "born-expired",
				"spec": json!({
					"category": "news",
					"title": "hello",
					"visible_at": (chrono::Utc::now() + chrono::Duration::hours(2)).to_rfc3339(),
					"expires_at": (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339(),
				}),
				"target": { "recipients": [ALICE] },
			}),
		),
	];

	for (name, args) in refusals {
		let (status, body) = post(&service, "send", args, Some(TOKEN)).await;
		assert_eq!(status, 400, "{name} is refused: {body}");
	}

	let stored = rows(&service, "SELECT count(*) FROM notifications").await;
	assert_eq!(text(&stored[0][0]), "0", "nothing was stored");
}

#[tokio::test(flavor = "multi_thread")]
async fn scheduled_visibility_and_cancelling_before_it_is_due() {
	let service = service().await;
	let alice = signed_in(ALICE);
	let due = (chrono::Utc::now() + chrono::Duration::seconds(2)).to_rfc3339();

	let (status, body) = send(
		&service,
		"scheduled",
		json!({
			"category": "news",
			"title": "Later",
			"visible_at": due,
		}),
		&[ALICE],
	)
	.await;
	assert_eq!(status, 200, "{body}");

	let (_, page) = inbox(&service, &alice, 10, json!("active")).await;
	assert_eq!(
		page["items"].as_array().map(Vec::len),
		Some(0),
		"not due yet: {page}"
	);
	let (_, count) = unread(&service, &alice).await;
	assert_eq!(count, 0, "nor on the badge");

	let (status, body) = post(
		&service,
		"cancel",
		json!({ "idempotency_key": "scheduled" }),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "cancelling before it's due works: {body}");

	let (status, body) = post(
		&service,
		"cancel",
		json!({ "idempotency_key": "scheduled" }),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	assert_eq!(body, "0", "cancelling again answers zero");

	let cancelled = rows(
		&service,
		"SELECT count(*) FROM notifications WHERE send_key = 'scheduled' AND cancelled_at IS NOT \
		 NULL",
	)
	.await;
	assert_eq!(
		text(&cancelled[0][0]),
		"1",
		"the row records the cancellation"
	);

	// It never becomes visible, even past its due time.
	tokio::time::sleep(Duration::from_millis(2_300)).await;
	let (_, page) = inbox(&service, &alice, 10, json!("all")).await;
	assert_eq!(
		page["items"].as_array().map(Vec::len),
		Some(0),
		"cancelled stays invisible"
	);

	// Even past its due time, a cancelled send is a cancelled send.
	let (status, body) = post(
		&service,
		"cancel",
		json!({ "idempotency_key": "scheduled" }),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	assert_eq!(body, "0");

	// An already-visible send can't be cancelled.
	let (status, _) = send(&service, "visible-now", spec("news", "Now"), &[ALICE]).await;
	assert_eq!(status, 200);
	let (status, body) = post(
		&service,
		"cancel",
		json!({ "idempotency_key": "visible-now" }),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 409, "{body}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_scheduled_notification_becomes_visible_when_due() {
	let service = service().await;
	let alice = signed_in(ALICE);
	let due = (chrono::Utc::now() + chrono::Duration::seconds(1)).to_rfc3339();

	let (status, body) = send(
		&service,
		"due-soon",
		json!({
			"category": "news",
			"title": "Soon",
			"visible_at": due,
		}),
		&[ALICE],
	)
	.await;
	assert_eq!(status, 200, "{body}");

	let (_, page) = inbox(&service, &alice, 10, json!("active")).await;
	assert_eq!(
		page["items"].as_array().map(Vec::len),
		Some(0),
		"not due yet"
	);

	tokio::time::sleep(Duration::from_millis(1_300)).await;

	let (_, page) = inbox(&service, &alice, 10, json!("active")).await;
	assert_eq!(
		page["items"].as_array().map(Vec::len),
		Some(1),
		"due: {page}"
	);
	assert_eq!(page["items"][0]["title"], "Soon");
}

#[tokio::test(flavor = "multi_thread")]
async fn expired_notifications_leave_every_view_of_the_inbox() {
	let service = service().await;
	let alice = signed_in(ALICE);

	let (status, body) = send(
		&service,
		"fleeting",
		json!({
			"category": "news",
			"title": "Briefly",
			"expires_at": (chrono::Utc::now() + chrono::Duration::seconds(1)).to_rfc3339(),
		}),
		&[ALICE],
	)
	.await;
	assert_eq!(status, 200, "{body}");

	let (_, count) = unread(&service, &alice).await;
	assert_eq!(count, 1, "visible while it lasts");

	tokio::time::sleep(Duration::from_millis(1_300)).await;

	for show in ["active", "all", "unread"] {
		let (_, page) = inbox(&service, &alice, 10, json!(show)).await;
		assert_eq!(
			page["items"].as_array().map(Vec::len),
			Some(0),
			"{show} hides what expired: {page}"
		);
	}

	let (_, count) = unread(&service, &alice).await;
	assert_eq!(count, 0, "the badge counts none");
}

#[tokio::test(flavor = "multi_thread")]
async fn preferences_decide_at_send_and_at_visibility() {
	let service = service().await;
	let alice = signed_in(ALICE);
	let bob = signed_in(BOB);

	// Bob opts out before anything is sent.
	let (status, body) = post(
		&service,
		"set_category_preference",
		json!({ "category": "marketing", "opted_out": true }),
		Some(&bob),
	)
	.await;
	assert_eq!(status, 200, "{body}");

	let (status, body) = send(
		&service,
		"campaign",
		spec("marketing", "A sale"),
		&[ALICE, BOB],
	)
	.await;
	assert_eq!(status, 200, "{body}");
	let sent: SendResult = serde_json::from_str(&body).unwrap();
	assert_eq!(
		(sent.delivered, sent.skipped_opted_out),
		(1, 1),
		"the send skips bob"
	);

	let (_, page) = inbox(&service, &bob, 10, json!("active")).await;
	assert_eq!(
		page["items"].as_array().map(Vec::len),
		Some(0),
		"nothing reached bob"
	);

	// Alice opts out after her copy is stored: her inbox hides it.
	let (status, _) = post(
		&service,
		"set_category_preference",
		json!({ "category": "marketing", "opted_out": true }),
		Some(&alice),
	)
	.await;
	assert_eq!(status, 200);

	let (_, page) = inbox(&service, &alice, 10, json!("all")).await;
	assert_eq!(
		page["items"].as_array().map(Vec::len),
		Some(0),
		"hidden at visibility"
	);

	let (status, prefs) = post(&service, "category_preferences", json!({}), Some(&alice)).await;
	assert_eq!(status, 200, "{prefs}");
	let prefs: Vec<Value> = serde_json::from_str(&prefs).unwrap();
	assert_eq!(prefs.len(), 1);
	assert_eq!(prefs[0]["category"], "marketing");
	assert_eq!(prefs[0]["opted_out"], true);

	// Opting back in shows it again.
	let (status, _) = post(
		&service,
		"set_category_preference",
		json!({ "category": "marketing", "opted_out": false }),
		Some(&alice),
	)
	.await;
	assert_eq!(status, 200);

	let (_, page) = inbox(&service, &alice, 10, json!("active")).await;
	assert_eq!(
		page["items"].as_array().map(Vec::len),
		Some(1),
		"back in: {page}"
	);

	// Preferences are personal: alice's opt-out never touched bob's.
	let (status, prefs) = post(&service, "category_preferences", json!({}), Some(&bob)).await;
	let prefs: Vec<Value> = serde_json::from_str(&prefs).unwrap();
	assert_eq!(status, 200);
	assert_eq!(prefs.len(), 1);
	assert_eq!(prefs[0]["opted_out"], true);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_inbox_pages_are_bounded_and_follow_in_order() {
	let service = service().await;
	let alice = signed_in(ALICE);

	for n in 0..7 {
		let (status, body) = send(
			&service,
			&format!("page-{n}"),
			spec("news", &format!("Item {n}")),
			&[ALICE],
		)
		.await;
		assert_eq!(status, 200, "{body}");
	}

	let (status, first) = inbox(&service, &alice, 3, json!("active")).await;
	assert_eq!(status, 200, "{first}");
	let items = first["items"].as_array().expect("a page");
	assert_eq!(items.len(), 3);
	assert!(first["has_more"].as_bool().unwrap(), "{first}");
	let end = first["end"].as_str().expect("a cursor").to_owned();

	// Newest first.
	assert_eq!(items[0]["title"], "Item 6");

	let (status, second) = post(
		&service,
		"inbox",
		json!({ "page": { "limit": 3, "after": end }, "show": "active" }),
		Some(&alice),
	)
	.await;
	assert_eq!(status, 200, "{second}");
	let second: Value = serde_json::from_str(&second).unwrap();
	let items = second["items"].as_array().expect("a page");
	assert_eq!(items.len(), 3);
	assert!(second["has_more"].as_bool().unwrap());
	assert_eq!(items[0]["title"], "Item 3");

	let end = second["end"].as_str().expect("a cursor").to_owned();
	let (status, third) = post(
		&service,
		"inbox",
		json!({ "page": { "limit": 3, "after": end }, "show": "active" }),
		Some(&alice),
	)
	.await;
	assert_eq!(status, 200, "{third}");
	let third: Value = serde_json::from_str(&third).unwrap();
	assert_eq!(third["items"].as_array().map(Vec::len), Some(1));
	assert_eq!(third["has_more"], false, "{third}");

	// A page that asks for more than the bound gets the bound.
	let (status, over) = inbox(&service, &alice, 10_000, json!("active")).await;
	assert_eq!(status, 200);
	assert!(
		over["items"].as_array().map_or(99, Vec::len)
			<= usize::try_from(leptoskit_notifications::MAX_INBOX_PAGE).unwrap_or(usize::MAX),
		"a page reads at most 100"
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_sends_with_one_key_fan_out_once() {
	let service = service().await;

	let first = send(&service, "race", spec("news", "Raced"), &[ALICE, BOB]);
	let second = send(&service, "race", spec("news", "Raced"), &[ALICE, BOB]);
	let (a, b) = tokio::join!(first, second);

	assert_eq!(a.0, 200, "{}", a.1);
	assert_eq!(b.0, 200, "{}", b.1);

	let first: SendResult = serde_json::from_str(&a.1).unwrap();
	let second: SendResult = serde_json::from_str(&b.1).unwrap();

	let (fresh, duplicate) = if first.duplicate {
		(second, &first)
	} else {
		(first, &second)
	};

	assert!(!fresh.duplicate, "one of them did the work");
	assert!(duplicate.duplicate, "and the other answered from the store");
	assert_eq!(fresh.delivered, 2, "{fresh:?}");
	assert_eq!(
		duplicate.delivered, fresh.delivered,
		"the stored counts come back"
	);

	let stored = rows(
		&service,
		"SELECT count(*) FROM notifications WHERE send_key = 'race'",
	)
	.await;
	assert_eq!(text(&stored[0][0]), "2", "one row per person, once");

	let alice = signed_in(ALICE);
	let (_, count) = unread(&service, &alice).await;
	assert_eq!(count, 1, "no duplicate reached the badge");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_migrations_build_the_service_tables() {
	let service = service().await;

	let tables = rows(
		&service,
		"SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name",
	)
	.await;
	let names: Vec<String> = tables.iter().map(|row| text(&row[0])).collect();

	for expected in [
		"notifications",
		"sends",
		"category_preferences",
		// The packs the service's own runtime needs: mutations' receipts,
		// jobs, and inbox, and the live log its queries follow.
		"rpc_receipts",
		"rpc_jobs",
		"rpc_inbox",
		"live_changes",
		"live_commits",
	] {
		assert!(
			names.contains(&expected.to_owned()),
			"the migrations build `{expected}`: {names:?}"
		);
	}

	// The send's bookkeeping row is there, with its intent.
	let (status, _) = send(&service, "bookkeeping", spec("news", "Tracked"), &[ALICE]).await;
	assert_eq!(status, 200);

	let sends = rows(
		&service,
		"SELECT idempotency_key, delivered, skipped_opted_out, length(intent) FROM sends",
	)
	.await;
	assert_eq!(sends.len(), 1, "{sends:?}");
	assert_eq!(text(&sends[0][0]), "bookkeeping");
	assert_eq!(text(&sends[0][1]), "1");
	assert_eq!(text(&sends[0][2]), "0");
	assert_eq!(text(&sends[0][3]), "64", "the intent is a SHA-256");
}

fn text(value: &leptoskit_db::Value) -> String {
	match value {
		leptoskit_db::Value::Text(text) => text.clone(),
		leptoskit_db::Value::Int(int) => int.to_string(),
		other => format!("{other:?}"),
	}
}

/// The same send, inbox, and isolation behaviour on Postgres, as the
/// `postgres` feature's build runs it. Ignored like
/// `leptoskit_analytics`'s Postgres tests: run it against a private test
/// database with
/// `cargo test -p leptoskit_notifications --features postgres -- --ignored`,
/// with `LEPTOSKIT_NOTIFICATIONS_TEST_POSTGRES` set.
#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires LEPTOSKIT_NOTIFICATIONS_TEST_POSTGRES pointing to a private test database"]
async fn postgres_behaves_like_sqlite() {
	let url = std::env::var("LEPTOSKIT_NOTIFICATIONS_TEST_POSTGRES").expect("private Postgres URL");
	let db = Db::connect(&url).await.expect("the database opens");
	db.migrate(leptoskit_notifications::POSTGRES_MIGRATIONS)
		.await
		.expect("migrations apply");
	let service = start(db).await;

	let alice = signed_in(ALICE);

	let (status, body) = send(&service, "pg-1", spec("news", "On Postgres"), &[ALICE, BOB]).await;
	assert_eq!(status, 200, "{body}");

	let (status, count) = unread(&service, &alice).await;
	assert_eq!(status, 200);
	assert_eq!(count, 1);

	let (status, body) = send(&service, "pg-1", spec("news", "On Postgres"), &[ALICE, BOB]).await;
	assert_eq!(status, 200, "{body}");
	let again: SendResult = serde_json::from_str(&body).unwrap();
	assert!(again.duplicate, "{body}");

	// Bob can't touch alice's copy on Postgres either.
	let Some(id) = first_item_id(&service, &alice).await else {
		panic!("alice has the item");
	};
	let bob = signed_in(BOB);
	let (status, _) = post(
		&service,
		"mark_read",
		json!({ "notification": id }),
		Some(&bob),
	)
	.await;
	assert_eq!(status, 404);
}
