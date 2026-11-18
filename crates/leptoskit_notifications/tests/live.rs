//! The live socket end to end: a schedule's turning points reach an open
//! subscription with no write but the wake. A scheduled item appears when
//! it is due, an expiring one leaves, an opt-out hides what an open inbox
//! already shows, a read on one device reaches another's screen, one
//! person's wake never touches another's, and the wake jobs survive a
//! restart, drain once, and don't spin.

#![cfg(not(target_arch = "wasm32"))]

use std::collections::HashMap;
use std::time::Duration;

use futures::SinkExt as _;
use futures::StreamExt as _;
use leptoskit_db::Db;
use leptoskit_db::Read as _;
use leptoskit_db::Statement;
use leptoskit_notifications::MIGRATIONS;
use leptoskit_notifications::SERVICE;
use leptoskit_rpc as rpc;
use leptoskit_rpc::live::ClientFrame;
use leptoskit_rpc::live::ServerFrame;
use leptoskit_rpc::live::Update;
use leptoskit_rpc::server::Anonymous;
use leptoskit_rpc::server::ServiceCallers;
use serde_json::Value;
use serde_json::json;
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

const TOKEN: &str = "app-token";
const ORIGIN: &str = "http://app.test";
const PREFIX: &str = "/_leptoskit/services/notifications";

/// Two people the app knows, fixed so tests can name who they expect.
const ALICE: Uuid = Uuid::from_u128(0xA110_0000_0000_0000_0000_0000_0000_0001);
const BOB: Uuid = Uuid::from_u128(0xB0B0_0000_0000_0000_0000_0000_0000_0002);

/// How long a value may take to change: the wake runs within the workers'
/// poll of its time, but a loaded machine can be slower than that.
const ARRIVES: Duration = Duration::from_secs(15);

struct Service {
	address: std::net::SocketAddr,
	db: Db,
	/// The service's job workers, stopped with the service so no test
	/// leaves one polling behind it.
	jobs: Option<rpc::server::JobsHandle>,
}

/// A fresh, migrated service on a free port, knowing `TOKEN` as the app and
/// forwarding identity as `lk` does, with its job workers on a test-fast
/// poll so a wake lands soon after its time.
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
	Service {
		address,
		db,
		jobs: Some(jobs),
	}
}

impl Service {
	/// Stops the workers and waits for them to finish, so nothing of this
	/// service runs a job again.
	async fn stop_jobs(&mut self) {
		if let Some(jobs) = self.jobs.take() {
			jobs.shutdown().await;
		}
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
	start(leptoskit_db::testing::sqlite_migrated(MIGRATIONS).await).await
}

/// The token the app's server would mint for `person`, as it does when a
/// signed-in browser or device asks for one.
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

/// Sends a spec as the app, panicking unless it lands.
async fn send(service: &Service, key: &str, spec: Value, recipients: &[Uuid]) {
	let (status, body) = post(
		service,
		"send",
		json!({
			"idempotency_key": key,
			"spec": spec,
			"target": { "recipients": recipients },
		}),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "{body}");
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

/// One person's open connection: authenticated, holding the current value
/// and a change counter for each of its subscriptions, so a test can wait
/// for whichever subscription it cares to see change next.
struct Screen {
	socket: WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
	next_id: u32,
	values: HashMap<u32, Value>,
	changes: HashMap<u32, u64>,
}

/// Connects as `person` and authenticates, as a native app does.
async fn screen(service: &Service, person: Uuid) -> Screen {
	let (mut socket, _) = connect_async(format!("ws://{}{PREFIX}/live", service.address))
		.await
		.unwrap();

	assert!(
		matches!(next(&mut socket).await, ServerFrame::Welcome { .. }),
		"welcome first"
	);

	send_frame(
		&mut socket,
		ClientFrame::Auth {
			token: signed_in(person),
		},
	)
	.await;

	match next(&mut socket).await {
		ServerFrame::AuthStatus { refused: false, .. } => {}
		other => panic!("the token authenticates: {other:?}"),
	}

	Screen {
		socket,
		next_id: 0,
		values: HashMap::new(),
		changes: HashMap::new(),
	}
}

impl Screen {
	/// Subscribes to `query`, keeping its first value.
	async fn subscribe(&mut self, query: &str, args: Value) -> Value {
		let id = self.next_id + 1;
		self.next_id = id;
		send_frame(
			&mut self.socket,
			ClientFrame::Sub {
				id,
				query: query.to_owned(),
				args,
			},
		)
		.await;

		let value = match next(&mut self.socket).await {
			ServerFrame::Done {
				request,
				ok: Some(value),
				err: None,
				..
			} if request == id => value,
			other => panic!("the subscription's first value: {other:?}"),
		};
		self.values.insert(id, value.clone());
		self.changes.insert(id, 0);

		value
	}

	/// The subscription `id`'s current value.
	fn value(&self, id: u32) -> Value {
		self.values[&id].clone()
	}

	/// How many transitions the subscription `id` has had, first value
	/// excluded: a marker to wait from.
	fn changes(&self, id: u32) -> u64 {
		self.changes[&id]
	}

	/// The subscription `id`'s value once it changes after `from` changes,
	/// patches applied, waiting as long as a wake may take. One transition
	/// can carry several subscriptions' changes; each is counted wherever
	/// the test was waiting.
	async fn next_change(&mut self, id: u32, from: u64) -> Value {
		let value = loop {
			if self.changes(id) > from {
				break self.value(id);
			}

			match next(&mut self.socket).await {
				ServerFrame::Transition(transition) => self.apply(&transition.updates),
				other => panic!("a transition for subscription {id}: {other:?}"),
			}
		};
		self.values.insert(id, value.clone());

		value
	}

	/// Applies each update and counts every subscription it changed.
	fn apply(&mut self, updates: &[leptoskit_rpc::live::SubscriptionUpdate]) {
		for update in updates {
			let Some(mut current) = self.values.get(&update.id).cloned() else {
				continue;
			};
			let value = match &update.update {
				Update::Ok(value) => value.clone(),
				Update::Patch(patch) => patched(&current, patch),
				Update::Err(error) => {
					panic!("subscription {} failed: {error:?}", update.id)
				}
			};

			if value != current {
				*self.changes.entry(update.id).or_default() += 1;
			}

			current = value;
			self.values.insert(update.id, current);
		}
	}

	/// Asserts no transition reaches subscription `id` for `within`.
	async fn hears_nothing_for(&mut self, id: u32, within: Duration) {
		let heard = tokio::time::timeout(within, async {
			loop {
				let frame = next(&mut self.socket).await;

				if let ServerFrame::Transition(transition) = &frame
					&& transition.updates.iter().any(|update| update.id == id)
				{
					return frame;
				}
			}
		})
		.await;

		if let Ok(frame) = heard {
			panic!("subscription {id} heard {frame:?} where nothing should reach it");
		}
	}
}

/// `patch` applied to the page `current`: removals by row id, upserts in
/// order at their places, the object's other fields set whole.
fn patched(current: &Value, patch: &leptoskit_rpc::live::Patch) -> Value {
	let mut page = current.clone();
	let field = patch.field.as_deref().unwrap_or("items");
	let mut rows = page
		.get(field)
		.and_then(Value::as_array)
		.cloned()
		.unwrap_or_default();

	if !patch.remove.is_empty() {
		rows.retain(|row| {
			row.get("id")
				.and_then(Value::as_str)
				.is_none_or(|id| !patch.remove.iter().any(|removed| removed == id))
		});
	}

	for upsert in &patch.upsert {
		// An upsert replaces the row with its id wherever it is, and
		// only inserts at its place when no such row is there.
		let id = upsert.value.get("id").and_then(Value::as_str);

		if let Some(id) = id
			&& let Some(existing) = rows
				.iter()
				.position(|row| row.get("id").and_then(Value::as_str) == Some(id))
		{
			rows[existing] = upsert.value.clone();
		} else {
			let at = upsert.at.min(rows.len());
			rows.insert(at, upsert.value.clone());
		}
	}

	if let Some(object) = page.as_object_mut() {
		object.insert(field.to_owned(), Value::Array(rows));

		for (name, value) in &patch.set {
			object.insert(name.clone(), value.clone());
		}
	}

	page
}

async fn send_frame(
	socket: &mut WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
	frame: ClientFrame,
) {
	let text = serde_json::to_string(&frame).unwrap();
	socket.send(Message::Text(text.into())).await.unwrap();
}

async fn next(socket: &mut WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>) -> ServerFrame {
	loop {
		let message = tokio::time::timeout(Duration::from_secs(20), socket.next())
			.await
			.expect("a frame arrives")
			.unwrap()
			.unwrap();

		match message {
			Message::Text(text) => return serde_json::from_str(&text).unwrap(),
			Message::Ping(_) | Message::Pong(_) => {}
			other => panic!("unexpected frame: {other:?}"),
		}
	}
}

/// `seconds` from now, as RFC 3339 for a spec.
fn in_seconds(seconds: i64) -> String {
	(chrono::Utc::now() + chrono::Duration::seconds(seconds)).to_rfc3339()
}

/// An inbox page's items.
fn items(page: &Value) -> Vec<Value> {
	page["items"].as_array().cloned().unwrap_or_default()
}

/// A topic send's fanout reaches an open inbox the way a recipients send
/// does: the batch's insert is the only write, and the subscription
/// re-runs with the new row.
#[tokio::test(flavor = "multi_thread")]
async fn a_topic_send_reaches_an_open_inbox_of_a_subscriber() {
	let service = service().await;
	// Bob's connection opens first, so alice's binding can keep the
	// helper's name.
	let mut bob = screen(&service, BOB).await;
	let mut screen = screen(&service, ALICE).await;

	let page = screen
		.subscribe(
			"inbox",
			json!({ "page": { "limit": 10 }, "show": "active" }),
		)
		.await;
	assert_eq!(items(&page).len(), 0, "{page}");

	// The app subscribes alice to the topic, then sends to it.
	let (status, body) = post(
		&service,
		"subscribe_to_topic",
		json!({ "topic": "news.live", "persons": [ALICE, BOB] }),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "{body}");

	let inbox_seen = screen.changes(1);
	let (status, body) = post(
		&service,
		"send",
		json!({
			"idempotency_key": "live-topic",
			"spec": { "category": "news", "title": "By topic" },
			"target": { "topic": "news.live" },
		}),
		Some(TOKEN),
	)
	.await;
	assert_eq!(status, 200, "{body}");

	let arrived = screen.next_change(1, inbox_seen).await;
	assert_eq!(items(&arrived).len(), 1, "{arrived}");
	assert_eq!(arrived["items"][0]["title"], "By topic");

	// His topics list is live too: an open `my_topics` page follows the
	// subscription that preceded the send.
	let topics = bob
		.subscribe("my_topics", json!({ "page": { "limit": 10 } }))
		.await;
	assert_eq!(
		topics["items"],
		json!(["news.live"]),
		"the app's batch subscription is there: {topics}"
	);

	let topics_seen = bob.changes(1);
	let (status, body) = post(
		&service,
		"unsubscribe",
		json!({ "topic": "news.live" }),
		Some(&signed_in(BOB)),
	)
	.await;
	assert_eq!(status, 200, "{body}");

	let gone = bob.next_change(1, topics_seen).await;
	assert_eq!(gone["items"], json!([]), "his own unsubscribe reaches it");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_scheduled_send_appears_on_an_open_inbox_without_any_other_write() {
	let service = service().await;
	let mut screen = screen(&service, ALICE).await;

	let inbox = screen
		.subscribe(
			"inbox",
			json!({ "page": { "limit": 10 }, "show": "active" }),
		)
		.await;
	assert_eq!(items(&inbox).len(), 0, "{inbox}");

	let badge = screen.subscribe("unread_count", json!({})).await;
	assert_eq!(badge, json!(0), "{badge}");

	// The only writes from here: the send's own rows, and the wake job at
	// its due time. No re-fetch, no second send.
	let inbox_seen = screen.changes(1);
	let badge_seen = screen.changes(2);
	send(
		&service,
		"due-later",
		json!({
			"category": "news",
			"title": "Later",
			"visible_at": in_seconds(2),
		}),
		&[ALICE],
	)
	.await;

	let arrived = screen.next_change(1, inbox_seen).await;
	assert_eq!(items(&arrived).len(), 1, "{arrived}");
	assert_eq!(arrived["items"][0]["title"], "Later");

	let badge = screen.next_change(2, badge_seen).await;
	assert_eq!(badge, json!(1), "the badge counts it: {badge}");

	// The wake did land: one row for alice, from the job that carried it.
	let wakes = rows(&service, "SELECT count(*) FROM wakes").await;
	assert_eq!(text(&wakes[0][0]), "1", "one wake row, one person");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_expired_item_leaves_open_subscriptions_when_its_time_passes() {
	let service = service().await;
	let mut screen = screen(&service, ALICE).await;

	let inbox = screen
		.subscribe(
			"inbox",
			json!({ "page": { "limit": 10 }, "show": "active" }),
		)
		.await;
	assert_eq!(items(&inbox).len(), 0, "{inbox}");
	let badge = screen.subscribe("unread_count", json!({})).await;
	assert_eq!(badge, json!(0), "{badge}");

	let inbox_seen = screen.changes(1);
	let badge_seen = screen.changes(2);
	send(
		&service,
		"fleeting",
		json!({
			"category": "news",
			"title": "Briefly",
			"expires_at": in_seconds(2),
		}),
		&[ALICE],
	)
	.await;

	// It arrives while it lasts, then leaves when its time is up: both on
	// the open subscriptions, without any other write.
	let arrived = screen.next_change(1, inbox_seen).await;
	assert_eq!(items(&arrived).len(), 1, "{arrived}");
	let badge = screen.next_change(2, badge_seen).await;
	assert_eq!(badge, json!(1), "{badge}");

	let gone_seen = screen.changes(1);
	let badge_gone_seen = screen.changes(2);
	let gone = screen.next_change(1, gone_seen).await;
	assert_eq!(items(&gone).len(), 0, "expired: {gone}");
	let badge = screen.next_change(2, badge_gone_seen).await;
	assert_eq!(badge, json!(0), "the badge counts none: {badge}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_opt_out_updates_an_already_open_inbox_and_badge() {
	let service = service().await;
	let mut screen = screen(&service, ALICE).await;

	let inbox = screen
		.subscribe(
			"inbox",
			json!({ "page": { "limit": 10 }, "show": "active" }),
		)
		.await;
	assert_eq!(items(&inbox).len(), 0, "{inbox}");
	let badge = screen.subscribe("unread_count", json!({})).await;
	assert_eq!(badge, json!(0), "{badge}");

	let inbox_seen = screen.changes(1);
	let badge_seen = screen.changes(2);
	send(
		&service,
		"campaign",
		json!({ "category": "marketing", "title": "A sale" }),
		&[ALICE],
	)
	.await;

	let arrived = screen.next_change(1, inbox_seen).await;
	assert_eq!(items(&arrived).len(), 1, "{arrived}");
	let badge = screen.next_change(2, badge_seen).await;
	assert_eq!(badge, json!(1), "{badge}");

	// Opting out is a write to the preference table, and nothing else:
	// the open page and badge follow it.
	let (status, body) = post(
		&service,
		"set_category_preference",
		json!({ "category": "marketing", "opted_out": true }),
		Some(&signed_in(ALICE)),
	)
	.await;
	assert_eq!(status, 200, "{body}");

	let inbox_seen = screen.changes(1);
	let badge_seen = screen.changes(2);
	let hidden = screen.next_change(1, inbox_seen).await;
	assert_eq!(items(&hidden).len(), 0, "hidden at visibility: {hidden}");
	let badge = screen.next_change(2, badge_seen).await;
	assert_eq!(badge, json!(0), "{badge}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_read_on_one_device_reaches_another_devices_screen() {
	let service = service().await;
	let mut screen = screen(&service, ALICE).await;

	let inbox = screen
		.subscribe(
			"inbox",
			json!({ "page": { "limit": 10 }, "show": "active" }),
		)
		.await;
	assert_eq!(items(&inbox).len(), 0, "{inbox}");
	let badge = screen.subscribe("unread_count", json!({})).await;
	assert_eq!(badge, json!(0), "{badge}");

	let inbox_seen = screen.changes(1);
	let badge_seen = screen.changes(2);
	send(
		&service,
		"everywhere",
		json!({ "category": "news", "title": "Hello" }),
		&[ALICE],
	)
	.await;

	// The send arrives on the open screen…
	let arrived = screen.next_change(1, inbox_seen).await;
	assert_eq!(items(&arrived).len(), 1, "{arrived}");
	let badge = screen.next_change(2, badge_seen).await;
	assert_eq!(badge, json!(1), "{badge}");

	// …and the other device reads it over HTTP, with its own token.
	let id = items(&screen.value(1))[0]["id"].clone();
	let (status, body) = post(
		&service,
		"mark_read",
		json!({ "notification": id }),
		Some(&signed_in(ALICE)),
	)
	.await;
	assert_eq!(status, 200, "{body}");

	let inbox_seen = screen.changes(1);
	let badge_seen = screen.changes(2);
	let badge = screen.next_change(2, badge_seen).await;
	assert_eq!(
		badge,
		json!(0),
		"the badge drops on the open screen: {badge}"
	);

	let read = screen.next_change(1, inbox_seen).await;
	assert_eq!(
		items(&read).len(),
		1,
		"still in the inbox, now read: {read}"
	);
	assert!(
		read["items"][0]["read_at"].as_str().is_some(),
		"the read state crossed devices: {read}"
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn one_persons_wake_never_touches_another_persons_screen() {
	let service = service().await;
	let mut screen = screen(&service, ALICE).await;

	let inbox = screen
		.subscribe(
			"inbox",
			json!({ "page": { "limit": 10 }, "show": "active" }),
		)
		.await;
	assert_eq!(items(&inbox).len(), 0, "{inbox}");

	// A scheduled send to bob only: his wake must not re-run alice's
	// subscriptions. Past his due time and the wake's slack, she has heard
	// nothing.
	send(
		&service,
		"for-bob",
		json!({
			"category": "news",
			"title": "Bob's news",
			"visible_at": in_seconds(1),
		}),
		&[BOB],
	)
	.await;
	screen.hears_nothing_for(1, Duration::from_secs(3)).await;

	// Her own schedule does reach her, through the same mechanism.
	let seen = screen.changes(1);
	send(
		&service,
		"for-alice",
		json!({
			"category": "news",
			"title": "Alice's news",
			"visible_at": in_seconds(1),
		}),
		&[ALICE],
	)
	.await;
	let arrived = tokio::time::timeout(ARRIVES, screen.next_change(1, seen))
		.await
		.expect("her own wake reaches her");
	assert_eq!(items(&arrived).len(), 1, "{arrived}");
	assert_eq!(arrived["items"][0]["title"], "Alice's news");
}

#[tokio::test(flavor = "multi_thread")]
async fn wake_jobs_survive_a_restart_and_drain_once_without_spinning() {
	let file = tempfile::Builder::new()
		.prefix("notifications-wake-restart-")
		.suffix(".sqlite3")
		.tempfile()
		.expect("a database file");
	let url = format!("sqlite://{}", file.path().display());

	// The first process: a send due after it is gone.
	let db = Db::connect(&url).await.expect("the database opens");
	db.migrate(MIGRATIONS).await.expect("migrations apply");
	let mut first = start(db).await;
	send(
		&first,
		"across-restart",
		json!({
			"category": "news",
			"title": "Waited",
			"visible_at": in_seconds(1),
		}),
		&[ALICE],
	)
	.await;

	// Its workers stop before the wake is due, so the wake's time passes
	// while nobody is running it.
	first.stop_jobs().await;

	let second = start(Db::connect(&url).await.expect("the database opens again")).await;

	// The restarted workers claim the overdue wake, run it once, and leave
	// it done: one wake row, no pending or spinning job. The worker records
	// the job done in a write of its own after the wake row commits, so
	// wait for that record, not for the row.
	let drained = async {
		loop {
			let jobs = rows(
				&second,
				"SELECT count(*) FROM rpc_jobs WHERE state = 'done'",
			)
			.await;

			if text(&jobs[0][0]) != "0" {
				break;
			}

			tokio::time::sleep(Duration::from_millis(100)).await;
		}
	};
	tokio::time::timeout(ARRIVES, drained)
		.await
		.expect("the overdue wake runs after the restart");

	let jobs = rows(
		&second,
		"SELECT state, count(*) FROM rpc_jobs GROUP BY state",
	)
	.await;
	assert_eq!(jobs.len(), 1, "one wake job, run once: {jobs:?}");
	assert_eq!(text(&jobs[0][0]), "done", "it finished: {jobs:?}");
	assert_eq!(text(&jobs[0][1]), "1", "{jobs:?}");

	// The notification it carried is there to read.
	let (status, page) = post(
		&second,
		"inbox",
		json!({ "page": { "limit": 10 }, "show": "active" }),
		Some(&signed_in(ALICE)),
	)
	.await;
	assert_eq!(status, 200, "{page}");
	let page: Value = serde_json::from_str(&page).unwrap();
	assert_eq!(items(&page).len(), 1, "due and visible: {page}");
	assert_eq!(page["items"][0]["title"], "Waited");

	// And no busy loop: the wake row is written once and stays.
	let written = rows(&second, "SELECT woken_at FROM wakes").await;
	tokio::time::sleep(Duration::from_secs(2)).await;
	let again = rows(&second, "SELECT woken_at FROM wakes").await;
	assert_eq!(
		text(&written[0][0]),
		text(&again[0][0]),
		"the wake writes once and stops"
	);
}
