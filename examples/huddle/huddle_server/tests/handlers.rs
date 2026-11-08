//! Every handler the HTTP and live routes expose, driven the way a client
//! drives them: `POST /rpc/<name>` with a session cookie and the wire
//! frames over `/live`. The server runs in this process on SQLite, so the
//! tests cover the bodies and their refusals without anything outside.

use std::time::Duration;

use futures::SinkExt;
use futures::StreamExt;
use huddle_api::COLUMNS;
use huddle_server::Options;
use leptoskit_auth::Auth;
use leptoskit_auth::Keys;
use leptoskit_rpc::live::ClientFrame;
use leptoskit_rpc::live::ServerFrame;
use reqwest::Client;
use serde_json::Value;
use serde_json::json;
use tokio::net::TcpStream;
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

/// How long a job or live frame may take before the test gives up.
const WAIT: Duration = Duration::from_secs(20);

/// The installed services are one process-wide slot, so servers in one test
/// binary take turns. `cargo test` runs the tests in this file on threads of
/// one process; a server started while another test's jobs run would reach
/// the other's mailer and summarizer.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// A Huddle server on a free localhost port, with its job workers polling
/// quickly so a test never waits on the production cadence.
struct Server {
	huddle: Option<huddle_server::Huddle>,
	base: String,
	serving: tokio::task::JoinHandle<()>,
}

impl Server {
	async fn start(site_root: Option<std::path::PathBuf>) -> Self {
		let db = leptoskit_db::testing::sqlite_migrated(huddle_db::MIGRATIONS).await;
		let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
		let base = format!("http://{}", listener.local_addr().unwrap());
		let mut options = Options::new(&db);
		options.origin = Some(base.clone());
		options.site_root = site_root;
		options.mail_delay = Duration::from_millis(1);
		options.settlement.delay = Duration::from_millis(1);

		options.jobs = leptoskit_rpc::server::Jobs::new(db.clone())
			.poll(Duration::from_millis(20))
			.queue("mail", 2)
			.queue("settlement", 2);
		let auth = Auth::builder(&db, Keys::single("handlers", [11; 32])).origin(&base);
		let huddle = huddle_server::start(&db, auth, options).unwrap();
		let router = huddle.router.clone();
		let serving = tokio::spawn(async move {
			leptoskit::server::serve_listener(listener, router)
				.await
				.unwrap();
		});
		Self {
			huddle: Some(huddle),
			base,
			serving,
		}
	}

	async fn stop(mut self) {
		self.serving.abort();

		if let Some(huddle) = self.huddle.take() {
			huddle.shutdown().await;
		}
	}

	async fn sign_in(&self, email: &str) -> Person {
		Person::sign_in(&self.base, email).await
	}
}

/// Someone with a session cookie.
struct Person {
	http: Client,
	base: String,
	cookie: String,
}

impl Person {
	async fn sign_in(base: &str, email: &str) -> Self {
		let http = Client::new();
		let response = http
			.post(format!("{base}/auth/dev/sign-in"))
			.header("origin", base)
			.header("content-type", "application/json")
			.body(json!({ "email": email }).to_string())
			.send()
			.await
			.unwrap();
		assert!(response.status().is_success(), "{}", response.status());
		let cookie = response
			.headers()
			.get("set-cookie")
			.and_then(|value| value.to_str().ok())
			.and_then(|value| value.split(';').next())
			.expect("a session cookie")
			.to_owned();
		Self {
			http,
			base: base.to_owned(),
			cookie,
		}
	}

	async fn call(&self, name: &str, args: Value) -> Result<Value, (u16, String)> {
		let response = self
			.http
			.post(format!("{}/rpc/{name}", self.base))
			.header("origin", &self.base)
			.header("cookie", &self.cookie)
			.header("content-type", "application/json")
			.body(args.to_string())
			.send()
			.await
			.unwrap();
		let status = response.status().as_u16();
		let text = response.text().await.unwrap();

		if (200..300).contains(&status) {
			Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
		} else {
			Err((status, text))
		}
	}

	async fn ok(&self, name: &str, args: Value) -> Value {
		self.call(name, args)
			.await
			.unwrap_or_else(|(status, body)| panic!("{name}: {status}: {body}"))
	}

	async fn socket(&self) -> Socket {
		let url = format!("{}/live", self.base.replace("http://", "ws://"));
		let mut request = url.into_client_request().unwrap();
		request
			.headers_mut()
			.insert("cookie", self.cookie.parse().unwrap());
		let (stream, _) = connect_async(request).await.unwrap();
		let mut socket = Socket { stream };
		let ServerFrame::Welcome { .. } = socket.next().await else {
			panic!("welcome first");
		};
		socket
	}
}

/// One live connection.
struct Socket {
	stream: WebSocketStream<MaybeTlsStream<TcpStream>>,
}

impl Socket {
	async fn send(&mut self, frame: ClientFrame) {
		let text = serde_json::to_string(&frame).unwrap();
		self.stream.send(Message::Text(text.into())).await.unwrap();
	}

	async fn next(&mut self) -> ServerFrame {
		loop {
			let message = tokio::time::timeout(WAIT, self.stream.next())
				.await
				.expect("a frame arrives in time")
				.expect("the socket stays open")
				.unwrap();

			if let Message::Text(text) = message {
				return serde_json::from_str(&text).unwrap();
			}
		}
	}

	/// Reads frames until one matches `pick`.
	async fn frame<T>(&mut self, pick: impl Fn(&ServerFrame) -> Option<T>) -> T {
		loop {
			let frame = self.next().await;

			if let Some(found) = pick(&frame) {
				return found;
			}
		}
	}
}

/// `POST /rpc/<name>` without a session, as a stranger's page would.
async fn anonymous(base: &str, name: &str, body: &str) -> (u16, String) {
	let response = Client::new()
		.post(format!("{base}/rpc/{name}"))
		.header("origin", base)
		.header("content-type", "application/json")
		.body(body.to_owned())
		.send()
		.await
		.unwrap();
	let status = response.status().as_u16();
	(status, response.text().await.unwrap())
}

async fn general(person: &Person) -> Value {
	let channels = person.ok("channels", json!({})).await;
	channels
		.as_array()
		.unwrap()
		.iter()
		.find(|channel| channel["name"] == "general")
		.expect("the general channel")["id"]
		.clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn strangers_get_only_what_needs_no_session() {
	let _serial = SERIAL.lock().await;
	let server = Server::start(None).await;

	// A query that needs a person says so; one whose table policy asks about
	// the caller simply has nothing to return.
	let (status, body) = anonymous(&server.base, "channels", "{}").await;
	assert_eq!(status, 401, "{body}");
	let (status, body) = anonymous(
		&server.base,
		"messages",
		r#"{"channel":"0192f3c4-0000-7000-8000-000000000001","limit":10}"#,
	)
	.await;
	assert_eq!(status, 200, "{body}");
	assert_eq!(body, "[]");
	let (status, body) = anonymous(&server.base, "people", "{}").await;
	assert_eq!(status, 200, "{body}");
	assert_eq!(body, "[]");

	let (status, body) = anonymous(&server.base, "whoami", "{}").await;
	assert_eq!(status, 200, "{body}");
	assert_eq!(body, "null");

	// Unknown operations and malformed arguments are refused as such.
	let (status, _) = anonymous(&server.base, "no_such_operation", "{}").await;
	assert_eq!(status, 404);
	let (status, body) = anonymous(&server.base, "whoami", "not json").await;
	assert_eq!(status, 400, "{body}");

	// The mailbox renders with nothing in it, and no site root means no
	// files behind the API.
	let mail = Client::new()
		.get(format!("{}/_leptoskit/mail", server.base))
		.send()
		.await
		.unwrap();
	assert_eq!(mail.status(), 200);
	let page = mail.text().await.unwrap();
	assert!(page.contains("Huddle mail"), "{page}");
	assert!(page.contains("Nothing here left this server"), "{page}");
	let index = Client::new()
		.get(format!("{}/", server.base))
		.send()
		.await
		.unwrap();
	assert_eq!(index.status(), 404);

	server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[expect(clippy::too_many_lines, reason = "one story, told in order")]
async fn the_sidebar_history_members_and_read_markers_follow_the_rules() {
	let _serial = SERIAL.lock().await;
	let server = Server::start(None).await;
	let ada = server.sign_in("ada@huddle.test").await;
	let grace = server.sign_in("grace.hopper@huddle.test").await;

	let me = ada.ok("whoami", json!({})).await;
	assert_eq!(me["name"], "Ada");
	assert_eq!(me["is_admin"], true);
	let ada_id = me["id"].clone();
	let grace_id = grace.ok("whoami", json!({})).await["id"].clone();

	// A channel's name is normalized, and the creator is a member even when
	// it names only other people.
	let design = ada
		.ok(
			"create_channel",
			json!({ "name": "  #Design Team ", "private": true, "members": [ada_id] }),
		)
		.await;
	let design_id = design.clone();
	let cards = ada.ok("channels", json!({})).await;
	let card = cards
		.as_array()
		.unwrap()
		.iter()
		.find(|card| card["id"] == design)
		.expect("the new channel");
	assert_eq!(card["name"], "design-team");
	assert_eq!(card["private"], true);
	assert_eq!(card["owned"], true);

	// Grace can't see a private channel until she's added, twice is a
	// no-op, and removing her takes it away again.
	assert!(
		!grace
			.ok("channels", json!({}))
			.await
			.as_array()
			.unwrap()
			.iter()
			.any(|card| card["id"] == design_id)
	);
	ada.ok(
		"add_member",
		json!({ "channel": design_id, "user": grace_id }),
	)
	.await;
	ada.ok(
		"add_member",
		json!({ "channel": design_id, "user": grace_id }),
	)
	.await;
	assert!(
		grace
			.ok("channels", json!({}))
			.await
			.as_array()
			.unwrap()
			.iter()
			.any(|card| card["id"] == design_id)
	);
	let members = ada.ok("members", json!({ "channel": design_id })).await;
	assert_eq!(members.as_array().unwrap().len(), 2);
	ada.ok(
		"remove_member",
		json!({ "channel": design_id, "user": grace_id }),
	)
	.await;
	assert!(
		!grace
			.ok("channels", json!({}))
			.await
			.as_array()
			.unwrap()
			.iter()
			.any(|card| card["id"] == design_id)
	);

	assert_eq!(
		grace
			.ok("messages", json!({ "channel": design_id, "limit": 10 }))
			.await,
		json!([])
	);

	// A message is trimmed, a repeated key stores once, and an empty one is
	// refused. History pages backwards from a message.
	let general = general(&ada).await;
	let first = ada
		.ok(
			"send_message",
			json!({ "channel": general, "body": "  hello, Grace  ", "client_key": "ada-1" }),
		)
		.await;
	let again = ada
		.ok(
			"send_message",
			json!({ "channel": general, "body": "hello, Grace", "client_key": "ada-1" }),
		)
		.await;
	assert_eq!(first, again);
	assert!(
		ada.call(
			"send_message",
			json!({ "channel": general, "body": "   ", "client_key": "ada-2" })
		)
		.await
		.unwrap_err()
		.1
		.contains("a message needs some text")
	);
	let second = grace
		.ok(
			"send_message",
			json!({ "channel": general, "body": "on my way", "client_key": "grace-1" }),
		)
		.await;
	let third = grace
		.ok(
			"send_message",
			json!({ "channel": general, "body": "and the review", "client_key": "grace-2" }),
		)
		.await;
	let history = ada
		.ok("messages", json!({ "channel": general, "limit": 2 }))
		.await;

	assert_eq!(history.as_array().unwrap().len(), 2);
	assert_eq!(history[1]["id"], third);
	assert_eq!(history[1]["body"], "and the review");
	let older = ada
		.ok(
			"messages",
			json!({ "channel": general, "limit": 2, "before": second }),
		)
		.await;
	assert_eq!(older.as_array().unwrap().len(), 1);
	assert_eq!(older[0]["id"], first);
	// Settings outside the page size are clamped rather than refused.
	assert_eq!(
		ada.ok("messages", json!({ "channel": general, "limit": 0 }))
			.await
			.as_array()
			.unwrap()
			.len(),
		1
	);
	assert_eq!(
		ada.ok("messages", json!({ "channel": general, "limit": 1000 }))
			.await
			.as_array()
			.unwrap()
			.len(),
		3
	);

	// Reading the channel clears the unread count for its reader only.
	let before = ada.ok("channels", json!({})).await;
	let unread = before
		.as_array()
		.unwrap()
		.iter()
		.find(|card| card["id"] == general)
		.expect("general")["unread"]
		.clone();
	assert_eq!(unread, 2, "two messages from Grace");
	ada.ok("read_channel", json!({ "channel": general })).await;
	let after = ada.ok("channels", json!({})).await;
	assert_eq!(
		after
			.as_array()
			.unwrap()
			.iter()
			.find(|card| card["id"] == general)
			.expect("general")["unread"],
		0
	);

	server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_board_keeps_every_column_contiguous() {
	let _serial = SERIAL.lock().await;
	let server = Server::start(None).await;
	let ada = server.sign_in("ada@huddle.test").await;
	let grace = server.sign_in("grace.hopper@huddle.test").await;
	let general = general(&ada).await;

	assert!(
		ada.call("add_task", json!({ "channel": general, "title": " " }))
			.await
			.unwrap_err()
			.1
			.contains("a card needs a title")
	);
	let first = ada
		.ok(
			"add_task",
			json!({ "channel": general, "title": "First card" }),
		)
		.await;
	let second = grace
		.ok(
			"add_task",
			json!({ "channel": general, "title": "Second card" }),
		)
		.await;
	let third = ada
		.ok(
			"add_task",
			json!({ "channel": general, "title": "Third card" }),
		)
		.await;

	// Moving to the top of a column renumbers the whole column.
	ada.ok(
		"move_task",
		json!({ "task": third, "column": "todo", "index": 0 }),
	)
	.await;
	let board = ada.ok("board", json!({ "channel": general })).await;
	assert_eq!(board[0]["id"], third);
	assert_eq!(board[1]["id"], first);
	assert_eq!(board[2]["id"], second);

	for index in 0..3 {
		assert_eq!(board[index]["position"], index);
	}

	// Past the end lands last; a later move to the top takes the lead.
	ada.ok(
		"move_task",
		json!({ "task": first, "column": "doing", "index": 99 }),
	)
	.await;
	grace
		.ok(
			"move_task",
			json!({ "task": second, "column": "doing", "index": 0 }),
		)
		.await;
	// Repeating a move that already holds leaves the order alone.
	grace
		.ok(
			"move_task",
			json!({ "task": second, "column": "doing", "index": 0 }),
		)
		.await;
	let board = ada.ok("board", json!({ "channel": general })).await;
	let order: Vec<&Value> = board
		.as_array()
		.unwrap()
		.iter()
		.map(|card| &card["id"])
		.collect();
	assert_eq!(order, [&third, &second, &first]);
	assert_eq!(board[1]["column"], "doing");
	assert_eq!(board[1]["position"], 0);

	assert_eq!(board[2]["position"], 1);

	// Every column is reachable, and an unknown one is named in the refusal.
	ada.ok(
		"move_task",
		json!({ "task": first, "column": "done", "index": 0 }),
	)
	.await;
	let board = ada.ok("board", json!({ "channel": general })).await;
	assert_eq!(board[2]["column"], "done");
	assert_eq!(board[0]["column"], COLUMNS[0]);
	assert!(
		ada.call(
			"move_task",
			json!({ "task": first, "column": "unknown", "index": 0 })
		)
		.await
		.unwrap_err()
		.1
		.contains("there's no `unknown` column")
	);

	server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invitations_check_the_address_reach_the_mailbox_and_sign_people_up() {
	let _serial = SERIAL.lock().await;
	let server = Server::start(None).await;
	let ada = server.sign_in("ada@huddle.test").await;
	let general = general(&ada).await;

	assert!(
		ada.call("invite", json!({ "channel": general, "email": "nobody" }))
			.await
			.unwrap_err()
			.1
			.contains("that isn't an email address")
	);

	// A local part with markup in it proves the demo mailbox escapes what it
	// shows: it must render as text, never as an element.
	let marked = "ann<tag>@huddle.test";
	ada.ok("invite", json!({ "channel": general, "email": marked }))
		.await;

	for _ in 0..500 {
		let latest = ada.ok("invites", json!({ "channel": general })).await;

		if latest
			.as_array()
			.unwrap()
			.iter()
			.any(|invite| invite["email"] == marked && invite["status"] == "sent")
		{
			break;
		}
		tokio::time::sleep(Duration::from_millis(20)).await;
	}

	let latest = ada.ok("invites", json!({ "channel": general })).await;
	assert_eq!(latest[0]["status"], "sent", "{latest}");
	let mail = Client::new()
		.get(format!("{}/_leptoskit/mail", server.base))
		.send()
		.await
		.unwrap()
		.text()
		.await
		.unwrap();
	assert!(mail.contains("ann&lt;tag&gt;@huddle.test"), "{mail}");
	assert!(!mail.contains("ann<tag>@"), "{mail}");
	assert!(mail.contains("invited you to #general on Huddle"), "{mail}");

	// Inviting someone into a private channel means they join it when they
	// sign in, and their name comes from the address.
	let secret = ada
		.ok(
			"create_channel",
			json!({ "name": "secret", "private": true, "members": [] }),
		)
		.await;
	ada.ok(
		"invite",
		json!({ "channel": secret, "email": "new.person+team@huddle.test" }),
	)
	.await;
	let newcomer = server.sign_in("new.person+team@huddle.test").await;
	let who = newcomer.ok("whoami", json!({})).await;
	assert_eq!(who["name"], "New Person Team");
	assert_eq!(who["is_admin"], false);
	assert!(
		newcomer
			.ok("channels", json!({}))
			.await
			.as_array()
			.unwrap()
			.iter()
			.any(|card| card["id"] == secret),
		"the invited channel is waiting"
	);

	// An address that is only punctuation still gets a person, by the
	// fallback name.
	let punctuation = server.sign_in("---@huddle.test").await;
	assert_eq!(
		punctuation.ok("whoami", json!({})).await["name"],
		"New teammate"
	);

	server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tips_settle_and_only_a_real_attempt_may_be_retried() {
	let _serial = SERIAL.lock().await;
	let server = Server::start(None).await;
	let ada = server.sign_in("ada@huddle.test").await;
	let grace = server.sign_in("grace.hopper@huddle.test").await;
	let me = ada.ok("whoami", json!({})).await;
	let ada_id = me["id"].clone();
	let grace_id = grace.ok("whoami", json!({})).await["id"].clone();

	assert!(
		ada.call("send_tip", json!({ "to": ada_id, "amount": 5 }))
			.await
			.unwrap_err()
			.1
			.contains("tip someone else")
	);
	assert!(
		ada.call("send_tip", json!({ "to": grace_id, "amount": 0 }))
			.await
			.is_err(),
		"a tip needs at least one coin"
	);
	assert!(
		ada.call("send_tip", json!({ "to": grace_id, "amount": 1001 }))
			.await
			.is_err(),
		"a tip may carry at most a thousand"
	);

	let tip = ada
		.ok("send_tip", json!({ "to": grace_id, "amount": 5 }))
		.await;
	let tip_id = tip.clone();

	for _ in 0..500 {
		let tips = ada.ok("tips", json!({})).await;

		if tips[0]["status"] == "confirmed" {
			break;
		}

		tokio::time::sleep(Duration::from_millis(20)).await;
	}

	let tips = ada.ok("tips", json!({})).await;

	assert_eq!(tips[0]["status"], "confirmed", "{tips}");
	assert_eq!(tips[0]["from"], "Ada");
	assert_eq!(tips[0]["to"], "Grace Hopper");
	assert!(tips[0]["receipt"].is_string());

	// The receiver sees the same tip from the other side.
	let received = grace.ok("tips", json!({})).await;
	assert_eq!(received[0]["id"], tip_id);

	// A confirmed tip has nothing to retry; the call succeeds and does
	// nothing. Recording a status directly is only for the job.
	ada.ok("retry_tip", json!({ "tip": tip_id })).await;
	assert_eq!(ada.ok("tips", json!({})).await[0]["status"], "confirmed");
	let forged = ada
		.call(
			"set_tip_status",
			json!({ "tip": tip_id, "status": "failed", "receipt": null, "reason": null }),
		)
		.await;
	assert_eq!(forged.unwrap_err().0, 403);

	server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reminders_take_a_minute_to_a_day() {
	let _serial = SERIAL.lock().await;
	let server = Server::start(None).await;
	let ada = server.sign_in("ada@huddle.test").await;
	let general = general(&ada).await;

	for minutes in [0, 24 * 60 + 1] {
		assert!(
			ada.call(
				"remind_me",
				json!({ "channel": general, "minutes": minutes })
			)
			.await
			.unwrap_err()
			.1
			.contains("pick between a minute and a day")
		);
	}

	let job = ada
		.ok("remind_me", json!({ "channel": general, "minutes": 10 }))
		.await;
	assert!(job.as_str().is_some_and(|id| !id.is_empty()));

	server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn summaries_count_speakers_pin_and_truncate() {
	let _serial = SERIAL.lock().await;
	let server = Server::start(None).await;
	let ada = server.sign_in("ada@huddle.test").await;
	let grace = server.sign_in("grace.hopper@huddle.test").await;
	let general = general(&ada).await;

	// Nothing said yet has its own answer, pinned like any other.
	let empty = ada
		.ok("summarize_channel", json!({ "channel": general }))
		.await;
	let empty = empty.as_str().unwrap().to_owned();
	assert_eq!(empty, "Nothing has been said in #general yet.");
	let cards = ada.ok("channels", json!({})).await;
	assert_eq!(cards[0]["summary"], empty);

	ada.ok(
		"send_message",
		json!({ "channel": general, "body": "The launch checklist is ready", "client_key": "ada-s1" }),
	)
	.await;
	grace
		.ok(
			"send_message",
			json!({ "channel": general, "body": "Launch on Thursday, after the checklist review", "client_key": "grace-s1" }),
		)
		.await;
	let long = "Checklist approved, and the release notes are ready for the launch on Thursday \
	            morning, so ship it";
	ada.ok(
		"send_message",
		json!({ "channel": general, "body": long, "client_key": "ada-s2" }),
	)
	.await;
	let summary = ada
		.ok("summarize_channel", json!({ "channel": general }))
		.await;
	let summary = summary.as_str().unwrap().to_owned();
	assert!(summary.starts_with("3 messages from 2 people in #general. Ada said the most (2)."));
	assert!(summary.contains("Came up most:"), "{summary}");

	assert!(
		summary.contains("…"),
		"the latest message is cut at eighty characters: {summary}"
	);

	// A channel the caller can't see has no summary to make.
	let hidden = ada
		.call(
			"summarize_channel",
			json!({ "channel": "0192f3c4-0000-7000-8000-000000000099" }),
		)
		.await;
	assert_eq!(hidden.unwrap_err().0, 404);

	// Pinning is capped at the column's length.
	let pinned = "x".repeat(2_500);
	ada.ok(
		"pin_summary",
		json!({ "channel": general, "summary": pinned }),
	)
	.await;
	let cards = ada.ok("channels", json!({})).await;
	assert_eq!(cards[0]["summary"].as_str().unwrap().len(), 2_000);

	server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn people_and_sessions_cover_the_admin_paths() {
	let _serial = SERIAL.lock().await;
	let server = Server::start(None).await;
	let ada = server.sign_in("ada@huddle.test").await;
	let grace = server.sign_in("grace.hopper@huddle.test").await;
	let ada_id = ada.ok("whoami", json!({})).await["id"].clone();
	let grace_id = grace.ok("whoami", json!({})).await["id"].clone();

	let session = grace.ok("my_session", json!({})).await;
	assert_eq!(session["active"], true);
	let everyone = ada.ok("people", json!({})).await;
	assert_eq!(everyone.as_array().unwrap().len(), 2);
	assert_eq!(everyone[0]["name"], "Ada");

	// Only an admin records a sign-out, and only an admin signs someone out
	// everywhere.
	// A non-admin's write matches no row under the users policy, so it
	// changes nothing rather than failing loudly.
	grace.ok("mark_signed_out", json!({ "user": ada_id })).await;
	assert_eq!(ada.ok("my_session", json!({})).await["active"], true);
	assert_eq!(
		grace
			.call("sign_out_everywhere", json!({ "user": ada_id }))
			.await
			.unwrap_err()
			.0,
		403
	);
	ada.ok("sign_out_everywhere", json!({ "user": grace_id }))
		.await;
	assert_eq!(grace.ok("my_session", json!({})).await["active"], false);
	assert_eq!(grace.ok("whoami", json!({})).await, Value::Null);

	server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_live_room_refuses_people_who_are_not_in_the_channel() {
	let _serial = SERIAL.lock().await;
	let server = Server::start(None).await;
	let ada = server.sign_in("ada@huddle.test").await;
	let grace = server.sign_in("grace.hopper@huddle.test").await;
	let secret = ada
		.ok(
			"create_channel",
			json!({ "name": "secret", "private": true, "members": [] }),
		)
		.await;

	let args = json!({ "channel": secret });
	let room = serde_json::to_string(&args).unwrap();
	let mut socket = grace.socket().await;
	socket
		.send(ClientFrame::Join {
			channel: room,
			name: "room".into(),
			args,
			state: json!({ "user": grace.ok("whoami", json!({})).await["id"], "name": "Grace" }),
		})
		.await;
	let refused = socket
		.frame(|frame| {
			match frame {
				ServerFrame::ChannelError { err, .. } => Some(err.clone()),
				_ => None,
			}
		})
		.await;
	assert_eq!(refused.status, 403);
	assert_eq!(refused.message, "join the channel first");

	server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn static_files_serve_the_built_site_and_its_spa_routes() {
	let _serial = SERIAL.lock().await;
	let scratch = tempfile::tempdir().unwrap();
	std::fs::write(scratch.path().join("index.html"), "huddle home").unwrap();
	std::fs::create_dir_all(scratch.path().join("pkg")).unwrap();
	std::fs::write(scratch.path().join("pkg/app.js"), "glue").unwrap();
	let server = Server::start(Some(scratch.path().to_path_buf())).await;

	let home = Client::new()
		.get(format!("{}/", server.base))
		.send()
		.await
		.unwrap();
	assert_eq!(home.status(), 200);
	assert_eq!(home.text().await.unwrap(), "huddle home");
	let asset = Client::new()
		.get(format!("{}/pkg/app.js", server.base))
		.send()
		.await
		.unwrap();
	assert_eq!(asset.status(), 200);
	assert_eq!(asset.text().await.unwrap(), "glue");
	// A client route falls back to the page rather than a 404.
	let route = Client::new()
		.get(format!("{}/channels/general", server.base))
		.send()
		.await
		.unwrap();
	assert_eq!(route.status(), 200);

	assert_eq!(route.text().await.unwrap(), "huddle home");

	server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_projects_embedded_configuration_builds_sign_in() {
	let _serial = SERIAL.lock().await;
	let db = leptoskit_db::testing::sqlite_migrated(huddle_db::MIGRATIONS).await;
	let config = huddle_server::sign_in::config().expect("the embedded leptoskit.toml");
	assert_eq!(config.app.name, "huddle");
	assert!(config.auth.is_some());

	let builder = huddle_server::sign_in::builder(&db).expect("sign-in from the config");
	drop(builder);

	// Huddle names no Apple team or Android certificates, so the router has
	// neither association file; both requests fall through to a 404.
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let base = format!("http://{}", listener.local_addr().unwrap());
	let associations = huddle_server::sign_in::associations::<()>().expect("associations");
	let router = leptoskit::server::axum::Router::new().merge(associations);
	let serving = tokio::spawn(async move {
		leptoskit::server::serve_listener(listener, router)
			.await
			.unwrap();
	});
	let response = Client::new()
		.get(format!("{base}/.well-known/assetlinks.json"))
		.send()
		.await
		.unwrap();
	assert_eq!(response.status(), 404);
	serving.abort();

	// The debug descriptions name what a process runs without leaking keys.
	let options = Options::new(&db);
	let described = format!("{options:?}");
	assert!(described.contains("settlement"), "{described}");
	assert!(described.contains("mail"), "{described}");
	assert!(!described.contains("token"), "{described}");
}
