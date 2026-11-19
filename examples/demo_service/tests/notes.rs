//! The private notes proofs, through the real site router and the real
//! analytics service.
//!
//! Every account is created by the real emailed-code flow, every note goes
//! through `POST /demo/notes`, and every event is read back through the
//! analytics service's own `run` query: nothing here inserts a row, forges
//! a session, or stands in for the service on the happy path. The failure
//! paths do use a local router that refuses, rate-limits, or stalls, to
//! isolate what the site does when delivery cannot be confirmed; that
//! proves the saved-note-and-unconfirmed contract, not delivery.
//!
//! The cookie-jar browser and sign-in helpers here are a small copy of the
//! ones in `signin.rs`, kept separate so the sign-in fixtures stay as they
//! were reviewed.

#![cfg(unix)]

mod support;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

use anyhow::Result;
use axum::Router;
use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use axum::http::header;
use demo_service::activity::Analytics;
use demo_service::generation;
use demo_service::notes::DemoNote;
use demo_service::site;
use futures::future::join_all;
use leptoskit_analytics::query::DateRange;
use leptoskit_analytics::query::EventRow;
use leptoskit_analytics::query::EventsQuery;
use leptoskit_analytics::query::Query;
use leptoskit_analytics::query::QueryResult;
use leptoskit_auth::Keys;
use leptoskit_db::Db;
use serde_json::Value;
use serde_json::json;
use support::SITE_HOST;
use support::volume;
use tower::ServiceExt as _;

const SITE_ORIGIN: &str = "http://localhost";
const ADMIN_ORIGIN: &str = "http://127.0.0.1";
const TEST_SITE_KEY: [u8; 32] = [7; 32];
const TOKEN: &str = "notes-proof-analytics-token";

// ─── A cookie-jar browser and the real sign-in ───────────────────────

/// One browser: one cookie jar for the site's origin.
#[derive(Clone, Default)]
struct Browser {
	jar: HashMap<String, String>,
}

impl Browser {
	async fn send(
		&mut self,
		router: &Router,
		request: axum::http::request::Builder,
		body: Body,
	) -> axum::response::Response {
		let cookies = self
			.jar
			.iter()
			.map(|(name, value)| format!("{name}={value}"))
			.collect::<Vec<_>>()
			.join("; ");
		let request = if cookies.is_empty() {
			request
		} else {
			request.header(header::COOKIE, cookies)
		};
		let response = router
			.clone()
			.oneshot(request.body(body).expect("a request body"))
			.await
			.expect("the router answers");

		for value in response.headers().get_all(header::SET_COOKIE) {
			let value = value.to_str().expect("a cookie header");
			let (pair, attributes) = value.split_once(';').unwrap_or((value, ""));
			let (name, cookie) = pair.split_once('=').expect("a cookie pair");

			if cookie.is_empty() || attributes.contains("Max-Age=0") {
				self.jar.remove(name);
			} else {
				self.jar.insert(name.to_owned(), cookie.to_owned());
			}
		}

		response
	}

	async fn get(&mut self, router: &Router, path: &str) -> axum::response::Response {
		self.send(
			router,
			Request::get(path).header(header::HOST, SITE_HOST),
			Body::empty(),
		)
		.await
	}

	/// A JSON POST as a same-origin page would send it.
	async fn post_json(
		&mut self,
		router: &Router,
		path: &str,
		body: Value,
	) -> axum::response::Response {
		self.post_raw(
			router,
			path,
			Some(SITE_ORIGIN),
			"application/json",
			body.to_string(),
		)
		.await
	}

	/// A POST with every header chosen by the caller.
	async fn post_raw(
		&mut self,
		router: &Router,
		path: &str,
		origin: Option<&str>,
		content_type: &str,
		body: String,
	) -> axum::response::Response {
		let mut request = Request::post(path)
			.header(header::HOST, SITE_HOST)
			.header(header::CONTENT_TYPE, content_type);

		if let Some(origin) = origin {
			request = request.header(header::ORIGIN, origin);
		}

		self.send(router, request, Body::from(body)).await
	}

	/// `POST /demo/notes` with this text, the way the page sends it.
	async fn note(&mut self, router: &Router, text: &str) -> axum::response::Response {
		self.post_json(router, "/demo/notes", json!({ "text": text }))
			.await
	}

	async fn list(&mut self, router: &Router) -> Value {
		let response = self.get(router, "/demo/notes").await;
		assert_eq!(response.status(), StatusCode::OK, "the notes list answers");
		assert_no_store(&response);
		json_body(response).await
	}
}

async fn json_body(response: axum::response::Response) -> Value {
	let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
		.await
		.expect("a readable body");
	serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

fn assert_no_store(response: &axum::response::Response) {
	assert!(
		response
			.headers()
			.get(header::CACHE_CONTROL)
			.is_some_and(|value| value.to_str().expect("a header").contains("no-store")),
		"{} is never cached",
		response.status()
	);
}

/// What a refusal answered: its status and stable code, after proving it
/// is never cached.
async fn refusal(response: axum::response::Response) -> (StatusCode, String) {
	assert_no_store(&response);
	let status = response.status();
	let body = json_body(response).await;
	(
		status,
		body["error"].as_str().unwrap_or_default().to_owned(),
	)
}

/// Mints a persona's pass and signs it in with a real emailed code,
/// returning the address the code went to.
async fn sign_in(browser: &mut Browser, router: &Router, persona: &str) -> String {
	let minted = browser
		.post_raw(
			router,
			"/demo/pass",
			Some(SITE_ORIGIN),
			"application/x-www-form-urlencoded",
			format!("persona={persona}"),
		)
		.await;
	assert_eq!(minted.status(), StatusCode::SEE_OTHER, "the pass is minted");

	let view = json_body(browser.get(router, "/demo/inbox").await).await;
	let address = view["people"]
		.as_array()
		.expect("people")
		.iter()
		.find(|person| person["persona"] == persona)
		.expect("the persona's person")["email"]
		.as_str()
		.expect("an email")
		.to_owned();

	let sent = browser
		.post_json(router, "/auth/email/send", json!({ "email": address }))
		.await;
	assert_eq!(sent.status(), StatusCode::NO_CONTENT, "the code is sent");

	let view = json_body(browser.get(router, "/demo/inbox").await).await;
	let subject = view["messages"]
		.as_array()
		.expect("messages")
		.iter()
		.find(|message| message["to"] == address.as_str())
		.expect("the code arrives")["subject"]
		.as_str()
		.expect("a subject")
		.to_owned();
	let code = subject.rsplit(' ').next().expect("a code").to_owned();

	let verified = browser
		.post_json(
			router,
			"/auth/email/verify",
			json!({ "email": address, "code": code }),
		)
		.await;
	assert_eq!(verified.status(), StatusCode::OK, "the code signs in");
	address
}

// ─── The analytics service, asked through its own routes ─────────────

/// Puts the typed events in the service's catalogue through its own
/// `register_events` route, as the generation's seed does.
async fn register_events(router: &Router) {
	let arguments = leptoskit_analytics::service::RegisterEvents {
		schemas: admin_demo::analytics::schemas(),
	};
	let response = router
		.clone()
		.oneshot(
			Request::post(format!(
				"{}/rpc/register_events",
				leptoskit_rpc::service_prefix(leptoskit_analytics::SERVICE)
			))
			.header(header::ACCEPT, "application/json")
			.header(header::CONTENT_TYPE, "application/json")
			.header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
			.body(Body::from(
				serde_json::to_vec(&arguments).expect("the schemas serialize"),
			))
			.expect("a request"),
		)
		.await
		.expect("the service answers");
	assert!(response.status().is_success(), "the events register");
}

/// One of the service's RPC routes, called as the app.
async fn rpc(router: &Router, token: &str, name: &str, arguments: &Value) -> Value {
	let response = router
		.clone()
		.oneshot(
			Request::post(format!(
				"{}/rpc/{name}",
				leptoskit_rpc::service_prefix(leptoskit_analytics::SERVICE)
			))
			.header(header::ACCEPT, "application/json")
			.header(header::CONTENT_TYPE, "application/json")
			.header(header::AUTHORIZATION, format!("Bearer {token}"))
			.body(Body::from(arguments.to_string()))
			.expect("a request"),
		)
		.await
		.expect("the service answers");
	let status = response.status();
	let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
		.await
		.expect("a readable body");
	assert!(
		status.is_success(),
		"{name} answered {status}: {}",
		String::from_utf8_lossy(&bytes)
	);
	serde_json::from_slice(&bytes).expect("a JSON answer")
}

/// Every `note_created` event the service stores, through its own `run`
/// query, newest first.
async fn note_events(router: &Router, token: &str) -> Vec<EventRow> {
	let query = Query::Events(EventsQuery {
		range: DateRange::last_days(2),
		event: Some("note_created".to_owned()),
		filters: Vec::new(),
		columns: Vec::new(),
		limit: EventsQuery::MAX_LIMIT,
		cursor: None,
	});
	let answer = rpc(router, token, "run", &json!({ "query": query })).await;
	let answer: leptoskit_analytics::query::Answer =
		serde_json::from_value(answer).expect("a query answer");
	let QueryResult::Events(page) = answer.result else {
		panic!("the events query answers with events");
	};
	page.events
}

// ─── A site over a file database ─────────────────────────────────────

/// The `note_created` events a visitor caused, not the seeded month's: a
/// visitor's actor is `v-` and a hash.
async fn visitor_events(router: &Router, token: &str) -> Vec<EventRow> {
	note_events(router, token)
		.await
		.into_iter()
		.filter(|event| event.distinct_id.starts_with("v-"))
		.collect()
}

/// A site, its analytics service, and the second handle on the same file
/// database that stands in for a second service process.
struct Harness {
	dir: PathBuf,
	db: Db,
	keys: Keys,
	options: site::SiteOptions,
	/// The real analytics service's routes.
	analytics: Router,
	site: Router,
}

impl Harness {
	/// A site whose notes send their events to the real service.
	async fn real() -> Self {
		Self::with(|service| Analytics::new(service, TOKEN)).await
	}

	/// A site whose notes send their events to `delivery`, built from the
	/// real service's router.
	async fn with(delivery: impl FnOnce(Router) -> Analytics) -> Self {
		let dir = volume();
		let day = demo_service::clock::day_of(demo_service::clock::now_millis());
		let keys = Keys::single(&generation::site_key_id(&day), TEST_SITE_KEY);
		let db = Db::connect(&format!("sqlite://{}", dir.join("site.sqlite3").display()))
			.await
			.expect("the site database opens");
		leptoskit_db::testing::create_tables(&db)
			.await
			.expect("the site's tables create");

		let analytics = admin_demo::analytics::start(&dir, SITE_ORIGIN, TOKEN)
			.await
			.expect("the analytics service starts");
		register_events(&analytics).await;

		let options = site::SiteOptions::new(day, None);
		let built = site::build(
			&db,
			keys.clone(),
			SITE_ORIGIN,
			ADMIN_ORIGIN,
			&options,
			delivery(analytics.clone()),
		)
		.await
		.expect("the site builds");

		Self {
			dir,
			db,
			keys,
			options,
			analytics,
			site: built.router,
		}
	}

	/// A second site over its own handle on the same database file, with the
	/// same keys and the same analytics service.
	async fn second(&self) -> Router {
		let db = Db::connect(&format!(
			"sqlite://{}",
			self.dir.join("site.sqlite3").display()
		))
		.await
		.expect("the second handle opens");
		site::rebuild(
			&db,
			self.keys.clone(),
			SITE_ORIGIN,
			ADMIN_ORIGIN,
			&self.options,
			Analytics::new(self.analytics.clone(), TOKEN),
		)
		.expect("the second site opens")
	}

	/// Every note row, whoever owns it: what a visitor can never read.
	async fn stored(&self) -> Vec<DemoNote> {
		DemoNote::query()
			.all(&self.db.system("notes proof: read every note"))
			.await
			.expect("the notes read")
	}

	async fn events(&self) -> Vec<EventRow> {
		note_events(&self.analytics, TOKEN).await
	}
}

// ─── Ownership ───────────────────────────────────────────────────────

/// Two signed-in browsers each write and read only their own notes, the
/// newest first, with no cache and an analytics actor of their own.
#[tokio::test]
async fn each_account_reads_and_writes_only_its_own_notes() -> Result<()> {
	let harness = Harness::real().await;
	let (mut ada, mut grace) = (Browser::default(), Browser::default());
	sign_in(&mut ada, &harness.site, "ada").await;
	sign_in(&mut grace, &harness.site, "grace").await;

	assert_eq!(ada.list(&harness.site).await["notes"], json!([]));

	let first = ada.note(&harness.site, "ada's first").await;
	assert_eq!(first.status(), StatusCode::CREATED);
	assert_no_store(&first);
	let first = json_body(first).await;
	assert_eq!(first["analytics"], "confirmed", "{first}");
	assert_eq!(first["words"], 2);
	assert_eq!(first["note"]["text"], "ada's first");

	let second = json_body(ada.note(&harness.site, "ada's second").await).await;
	assert_eq!(second["actor"], first["actor"], "one person, one actor");

	let secret = "grace's private thought";
	let graces = json_body(grace.note(&harness.site, secret).await).await;
	assert_ne!(graces["actor"], first["actor"], "two people, two actors");

	let adas = ada.list(&harness.site).await;
	let texts: Vec<&str> = adas["notes"]
		.as_array()
		.expect("notes")
		.iter()
		.map(|note| note["text"].as_str().expect("text"))
		.collect();
	assert_eq!(texts, ["ada's second", "ada's first"], "newest first");
	assert_eq!(adas["limit"], 20);
	assert_eq!(adas["max_chars"], 280);
	assert_eq!(adas["actor"], first["actor"]);
	assert!(
		!adas.to_string().contains(secret),
		"Ada never receives Grace's text"
	);

	let hers = grace.list(&harness.site).await;
	assert_eq!(hers["notes"].as_array().expect("notes").len(), 1);
	assert_eq!(hers["notes"][0]["text"], secret);

	// The rows really are two accounts' and the policy keeps them apart
	// even for a query that asks for everything.
	let rows = harness.stored().await;
	assert_eq!(rows.len(), 3);
	assert_eq!(
		rows.iter()
			.map(|row| row.account_id)
			.collect::<std::collections::HashSet<_>>()
			.len(),
		2
	);

	// A body cannot choose whose note it is.
	let forged = ada
		.post_json(
			&harness.site,
			"/demo/notes",
			json!({ "text": "x", "account_id": rows[0].account_id.to_string() }),
		)
		.await;
	assert_eq!(
		refusal(forged).await,
		(StatusCode::BAD_REQUEST, "bad_request".into())
	);
	assert_eq!(harness.stored().await.len(), 3);
	Ok(())
}

/// No visit is a `401` before anything about the body is read: a malformed
/// or oversized body from an unauthenticated caller is not a `400` or
/// `413`, and a pass that is not signed in, or a session that is not the
/// pass's, is refused the same way, with no note or event.
#[tokio::test]
async fn a_real_visit_is_required_before_the_body_is_read() -> Result<()> {
	let harness = Harness::real().await;
	let (mut ada, mut grace) = (Browser::default(), Browser::default());
	sign_in(&mut ada, &harness.site, "ada").await;
	sign_in(&mut grace, &harness.site, "grace").await;

	let oversized = format!("{{\"text\":\"{}\"}}", "a".repeat(10_000));
	for body in ["not json at all", "", "{", oversized.as_str()] {
		let mut stranger = Browser::default();
		let response = stranger
			.post_raw(
				&harness.site,
				"/demo/notes",
				Some(SITE_ORIGIN),
				"application/json",
				body.to_owned(),
			)
			.await;
		assert_eq!(
			refusal(response).await,
			(StatusCode::UNAUTHORIZED, "no_visit".into()),
			"an unauthenticated {}-byte body is refused first",
			body.len()
		);
	}

	let mut stranger = Browser::default();
	let listing = stranger.get(&harness.site, "/demo/notes").await;
	assert_eq!(refusal(listing).await.0, StatusCode::UNAUTHORIZED);

	// A pass alone is not a session.
	let mut pass_only = Browser::default();
	pass_only
		.post_raw(
			&harness.site,
			"/demo/pass",
			Some(SITE_ORIGIN),
			"application/x-www-form-urlencoded",
			"persona=ada".to_owned(),
		)
		.await;
	let response = pass_only.note(&harness.site, "no session").await;
	assert_no_store(&response);
	assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
	assert_eq!(json_body(response).await["pass"], "signed_out");

	// Ada's pass with Grace's session: neither the pass nor the session
	// alone is enough, so a mismatch is refused.
	let mut mismatched = ada.clone();
	for (name, value) in &grace.jar {
		if name != "demo-pass" {
			mismatched.jar.insert(name.clone(), value.clone());
		}
	}
	assert_eq!(
		mismatched.jar.get("demo-pass"),
		ada.jar.get("demo-pass"),
		"the pass is Ada's"
	);
	let response = mismatched.note(&harness.site, "wrong session").await;
	assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
	let listing = mismatched.get(&harness.site, "/demo/notes").await;
	assert_eq!(listing.status(), StatusCode::UNAUTHORIZED);

	assert!(harness.stored().await.is_empty(), "nothing was saved");
	assert!(harness.events().await.is_empty(), "nothing was sent");
	Ok(())
}

/// A cross-origin, duplicate-origin, or origin-less post is refused, and
/// never saved.
#[tokio::test]
async fn only_a_same_origin_post_is_accepted() -> Result<()> {
	let harness = Harness::real().await;
	let mut ada = Browser::default();
	sign_in(&mut ada, &harness.site, "ada").await;

	let body = json!({ "text": "from elsewhere" }).to_string();
	for origin in [
		Some("http://evil.example"),
		Some("http://localhost:81"),
		None,
	] {
		let response = ada
			.post_raw(
				&harness.site,
				"/demo/notes",
				origin,
				"application/json",
				body.clone(),
			)
			.await;
		// The session layer's same-site check answers some of these before
		// the route does; either way the post is refused.
		assert_eq!(
			response.status(),
			StatusCode::FORBIDDEN,
			"origin {origin:?}"
		);
	}

	// Two Origin headers are one too many, even when one is right.
	let request = Request::post("/demo/notes")
		.header(header::HOST, SITE_HOST)
		.header(header::CONTENT_TYPE, "application/json")
		.header(header::ORIGIN, SITE_ORIGIN)
		.header(header::ORIGIN, "http://evil.example");
	let response = ada
		.send(&harness.site, request, Body::from(body.clone()))
		.await;
	assert_eq!(
		response.status(),
		StatusCode::FORBIDDEN,
		"two Origin headers"
	);

	assert!(harness.stored().await.is_empty(), "nothing was saved");
	assert!(harness.events().await.is_empty(), "nothing was sent");

	// The same request from the site's own origin works.
	assert_eq!(
		ada.note(&harness.site, "from here").await.status(),
		StatusCode::CREATED
	);
	Ok(())
}

// ─── Validation ──────────────────────────────────────────────────────

/// Every refusal has a stable code and no note or event; the limits count
/// Unicode scalar values after trimming, and the body and type are
/// bounded.
#[tokio::test]
async fn invalid_notes_are_refused_with_no_side_effects() -> Result<()> {
	let harness = Harness::real().await;
	let mut ada = Browser::default();
	sign_in(&mut ada, &harness.site, "ada").await;

	let long = "a".repeat(281);
	let too_many_scalars = "é".repeat(281);
	let cases: Vec<(&str, StatusCode, &str, String, &str)> = vec![
		(
			"empty",
			StatusCode::UNPROCESSABLE_ENTITY,
			"empty_note",
			json!({ "text": "" }).to_string(),
			"application/json",
		),
		(
			"only whitespace",
			StatusCode::UNPROCESSABLE_ENTITY,
			"empty_note",
			json!({ "text": " \n\t " }).to_string(),
			"application/json",
		),
		(
			"281 characters",
			StatusCode::UNPROCESSABLE_ENTITY,
			"note_too_long",
			json!({ "text": long }).to_string(),
			"application/json",
		),
		(
			"281 two-byte scalars",
			StatusCode::UNPROCESSABLE_ENTITY,
			"note_too_long",
			json!({ "text": too_many_scalars }).to_string(),
			"application/json",
		),
		(
			"an unknown field",
			StatusCode::BAD_REQUEST,
			"bad_request",
			json!({ "text": "x", "shared": true }).to_string(),
			"application/json",
		),
		(
			"no text",
			StatusCode::BAD_REQUEST,
			"bad_request",
			"{}".to_owned(),
			"application/json",
		),
		(
			"a number for text",
			StatusCode::BAD_REQUEST,
			"bad_request",
			json!({ "text": 5 }).to_string(),
			"application/json",
		),
		(
			"malformed JSON",
			StatusCode::BAD_REQUEST,
			"bad_request",
			r#"{"text":"#.to_owned(),
			"application/json",
		),
		(
			"a form body",
			StatusCode::UNSUPPORTED_MEDIA_TYPE,
			"unsupported_media_type",
			"text=hello".to_owned(),
			"application/x-www-form-urlencoded",
		),
		(
			"plain text",
			StatusCode::UNSUPPORTED_MEDIA_TYPE,
			"unsupported_media_type",
			json!({ "text": "x" }).to_string(),
			"text/plain",
		),
		(
			"a body over 4 KiB",
			StatusCode::PAYLOAD_TOO_LARGE,
			"too_large",
			format!("{{\"text\":\"{}\"}}", "a".repeat(4096)),
			"application/json",
		),
	];

	for (name, status, code, body, content_type) in cases {
		let response = ada
			.post_raw(
				&harness.site,
				"/demo/notes",
				Some(SITE_ORIGIN),
				content_type,
				body,
			)
			.await;
		assert_eq!(refusal(response).await, (status, code.into()), "{name}");
	}

	assert_eq!(ada.list(&harness.site).await["notes"], json!([]));
	assert!(harness.stored().await.is_empty(), "no refusal saved a note");
	assert!(
		harness.events().await.is_empty(),
		"no refusal sent an event"
	);

	// The boundary is in scalar values, not bytes: 280 two-byte and
	// four-byte characters fit, and surrounding whitespace is trimmed.
	for (text, expected) in [
		("é".repeat(280), "é".repeat(280)),
		("😀".repeat(280), "😀".repeat(280)),
		("  padded  ".to_owned(), "padded".to_owned()),
		(
			"two\nlines  of text".to_owned(),
			"two\nlines  of text".to_owned(),
		),
	] {
		let response = ada.note(&harness.site, &text).await;
		assert_eq!(response.status(), StatusCode::CREATED);
		assert_eq!(json_body(response).await["note"]["text"], expected.as_str());
	}
	let events = harness.events().await;
	assert_eq!(events.len(), 4);
	let words: Vec<u64> = events
		.iter()
		.map(|event| event.properties["words"].as_u64().expect("words"))
		.collect();
	assert!(
		words.contains(&1),
		"the one-word notes count one: {words:?}"
	);
	assert!(
		words.contains(&4),
		"'two\\nlines  of text' is four words: {words:?}"
	);
	Ok(())
}

/// The notebook holds twenty notes. Two service handles on one file
/// database race to write thirty: exactly twenty persist, the rest are
/// told the notebook is full, and each saved note sent exactly one event.
#[tokio::test]
async fn the_cap_holds_across_two_handles_racing() -> Result<()> {
	let harness = Harness::real().await;
	let second = harness.second().await;
	let mut ada = Browser::default();
	sign_in(&mut ada, &harness.site, "ada").await;

	let answers = join_all((0..30).map(|index| {
		let mut browser = ada.clone();
		let router = if index % 2 == 0 {
			harness.site.clone()
		} else {
			second.clone()
		};
		async move {
			let response = browser.note(&router, &format!("note {index}")).await;
			let status = response.status();
			(status, json_body(response).await)
		}
	}))
	.await;

	let created = answers
		.iter()
		.filter(|(status, _)| *status == StatusCode::CREATED)
		.count();
	let full: Vec<_> = answers
		.iter()
		.filter(|(status, body)| *status == StatusCode::CONFLICT && body["error"] == "notes_full")
		.collect();
	assert_eq!(created, 20, "exactly twenty are saved: {answers:?}");
	assert_eq!(full.len(), 10, "the rest are told the notebook is full");

	assert_eq!(
		harness.stored().await.len(),
		20,
		"exactly twenty rows persist"
	);
	assert_eq!(
		ada.list(&harness.site).await["notes"]
			.as_array()
			.expect("n")
			.len(),
		20
	);
	assert_eq!(
		harness.events().await.len(),
		20,
		"one event per saved note, none for a refused one"
	);

	// A full notebook stays full, and reading it sends nothing.
	let again = ada.note(&harness.site, "one more").await;
	assert_eq!(
		refusal(again).await,
		(StatusCode::CONFLICT, "notes_full".into())
	);
	assert_eq!(harness.events().await.len(), 20);
	Ok(())
}

// ─── The event ───────────────────────────────────────────────────────

/// A saved note is one real typed `NoteCreated` event in the real
/// service, from a stable pseudonymous actor, with the word count and the
/// persona and nothing a visitor typed or an address revealed; reading
/// notes sends none.
#[tokio::test]
async fn a_saved_note_is_one_real_typed_event_with_no_private_data() -> Result<()> {
	let harness = Harness::real().await;
	let mut ada = Browser::default();
	let address = sign_in(&mut ada, &harness.site, "ada").await;

	let text = "buy oat milk, then call grandma";
	let saved = json_body(ada.note(&harness.site, text).await).await;
	assert_eq!(saved["analytics"], "confirmed", "{saved}");
	let actor = saved["actor"].as_str().expect("an actor").to_owned();
	assert!(actor.starts_with("v-") && actor.len() == 66, "{actor}");

	for _ in 0..3 {
		ada.list(&harness.site).await;
	}
	let events = harness.events().await;
	assert_eq!(events.len(), 1, "exactly one event, and reads add none");
	let event = &events[0];
	assert_eq!(event.name, "note_created");
	assert_eq!(event.distinct_id, actor);
	assert_eq!(event.properties["words"], 6);
	assert_eq!(event.properties["shared"], false);
	assert_eq!(event.properties["persona"], "ada");
	assert_eq!(
		event.properties.as_object().expect("properties").len(),
		3,
		"only the fields and the persona: {}",
		event.properties
	);

	// The catalogue knows it as a typed event with its fields.
	let definitions = rpc(&harness.analytics, TOKEN, "event_definitions", &json!({})).await;
	let note = definitions
		.as_array()
		.expect("definitions")
		.iter()
		.find(|definition| definition["name"] == "note_created")
		.expect("note_created is catalogued");
	assert!(
		note["fields"].is_object() || note["fields"].is_array(),
		"typed: {note}"
	);

	// Nothing private reaches the service: not the text, the address, the
	// pass, a loopback address, or a user agent, in events or persons.
	let people = rpc(
		&harness.analytics,
		TOKEN,
		"persons",
		&json!({ "limit": 50 }),
	)
	.await;
	let everything = format!("{} {people}", serde_json::to_string(&events)?);
	for secret in [
		text,
		"oat milk",
		address.as_str(),
		"demo.invalid",
		"127.0.0.1",
		"curl",
	] {
		assert!(
			!everything.contains(secret),
			"the service never sees {secret:?}"
		);
	}

	// No profile or history is invented for an actor: the event alone
	// arrives, and no person row exists to carry anything.
	assert_eq!(people, json!([]), "no person was created");
	Ok(())
}

/// When the service refuses, rate-limits, or does not answer in time, the
/// note is still saved, the answer says the event is unconfirmed, the
/// service is asked exactly once, and nothing retries.
#[tokio::test]
async fn an_unconfirmed_event_still_saves_the_note_and_is_never_retried() -> Result<()> {
	#[derive(Clone)]
	struct Fake {
		calls: Arc<AtomicUsize>,
	}

	/// Returns a router that answers every request with `status` and
	/// `body` after `delay`, counting the requests it was sent.
	fn stand_in(status: StatusCode, body: &'static str, delay: Duration) -> (Fake, Analytics) {
		let fake = Fake {
			calls: Arc::new(AtomicUsize::new(0)),
		};
		let counted = fake.calls.clone();
		let router = Router::new().fallback(move || {
			let counted = counted.clone();
			async move {
				counted.fetch_add(1, Ordering::SeqCst);
				tokio::time::sleep(delay).await;
				(status, [(header::CONTENT_TYPE, "application/json")], body)
			}
		});
		(fake, Analytics::new(router, "a-stand-in-token"))
	}

	let cases = [
		(
			"the service refuses",
			StatusCode::SERVICE_UNAVAILABLE,
			r#"{"error":"down"}"#,
			Duration::ZERO,
		),
		(
			"the receipt shows a rate limit",
			StatusCode::OK,
			r#"{"stored":0,"duplicates":0,"rate_limited":1,"refused":[]}"#,
			Duration::ZERO,
		),
		(
			"the receipt shows a refused entry",
			StatusCode::OK,
			r#"{"stored":0,"duplicates":0,"rate_limited":0,"refused":[{"index":0,"reason":"unknown_schema"}]}"#,
			Duration::ZERO,
		),
		(
			"the receipt accounts for nothing",
			StatusCode::OK,
			r#"{"stored":0,"duplicates":0,"rate_limited":0,"refused":[]}"#,
			Duration::ZERO,
		),
		(
			"the answer is not a receipt",
			StatusCode::OK,
			"<html>hello</html>",
			Duration::ZERO,
		),
		(
			"the service is too slow",
			StatusCode::OK,
			r#"{"stored":1,"duplicates":0,"rate_limited":0,"refused":[]}"#,
			Duration::from_secs(8),
		),
	];

	for (name, status, body, delay) in cases {
		let (fake, analytics) = stand_in(status, body, delay);
		let harness = Harness::with(|_| analytics).await;
		let mut ada = Browser::default();
		sign_in(&mut ada, &harness.site, "ada").await;

		let started = Instant::now();
		let response = ada.note(&harness.site, "still saved").await;
		let took = started.elapsed();
		assert_eq!(response.status(), StatusCode::CREATED, "{name}");
		let answer = json_body(response).await;
		assert_eq!(answer["analytics"], "unconfirmed", "{name}: {answer}");
		assert_eq!(answer["note"]["text"], "still saved", "{name}");
		assert!(
			!answer.to_string().contains("a-stand-in-token"),
			"{name}: the token never shows"
		);
		assert!(
			took < Duration::from_secs(5),
			"{name}: the wait is bounded, not {took:?}"
		);
		if delay > Duration::ZERO {
			assert!(
				took >= Duration::from_millis(1900),
				"{name}: it waited {took:?}"
			);
		}

		assert_eq!(
			ada.list(&harness.site).await["notes"][0]["text"],
			"still saved",
			"{name}: the note is in the notebook"
		);
		tokio::time::sleep(Duration::from_millis(1500)).await;
		assert_eq!(
			fake.calls.load(Ordering::SeqCst),
			1,
			"{name}: asked once, never retried"
		);
	}
	Ok(())
}

// ─── Reopening, and a new day ────────────────────────────────────────

/// A same-day reopen keeps the notes and sessions and sends no event
/// again; the next day starts with an empty notebook, and yesterday's
/// session cannot read yesterday's notes.
#[tokio::test]
async fn a_reopen_keeps_the_notes_and_a_new_day_starts_empty() -> Result<()> {
	let root = volume();
	let config = |origin: &str| {
		let authority = |host: &str| {
			demo_service::config::Authority::of_host_header(host).expect("an authority")
		};
		demo_service::config::Config {
			bind: "127.0.0.1:0".parse().expect("a bind address"),
			volume: root.clone(),
			site_origin: origin.to_owned(),
			admin_origin: ADMIN_ORIGIN.to_owned(),
			authorities: demo_service::config::Authorities {
				site: authority(SITE_HOST),
				admin: authority("127.0.0.1"),
				bind: authority("127.0.0.1:8080"),
			},
			client_ip_header: None,
		}
	};
	let open = async |day: &str| {
		generation::open_or_build(
			&config(SITE_ORIGIN),
			day,
			demo_service::clock::now_millis(),
			&demo_service::shutdown::init(),
			&leptoskit_axum::Readiness::new(),
		)
		.await
		.expect("the generation opens")
	};

	let today = support::today();
	let first = open(&today).await;
	let mut ada = Browser::default();
	sign_in(&mut ada, &first.site, "ada").await;

	let saved = json_body(ada.note(&first.site, "kept across a restart").await).await;
	assert_eq!(
		saved["analytics"], "confirmed",
		"a fresh generation registers the typed event: {saved}"
	);
	let token = first.secrets.analytics_token.clone();
	let before = visitor_events(&first.loopback, &token).await;
	assert_eq!(before.len(), 1);
	drop(first);

	// The same day, a new process image: same notes, same session, and
	// the reopen sends no second event.
	let reopened = open(&today).await;
	let kept = ada.list(&reopened.site).await;
	assert_eq!(kept["notes"][0]["text"], "kept across a restart");
	assert_eq!(
		visitor_events(&reopened.loopback, &token).await.len(),
		1,
		"a reopen and a read send no event"
	);
	let more = json_body(ada.note(&reopened.site, "and one more").await).await;
	assert_eq!(
		more["analytics"], "confirmed",
		"the reopened service answers"
	);
	assert_eq!(
		more["actor"], saved["actor"],
		"the actor is stable across the reopen"
	);
	assert_eq!(visitor_events(&reopened.loopback, &token).await.len(), 2);
	drop(reopened);

	// Tomorrow: yesterday's pass is a new day, its session is gone, and a
	// fresh visitor starts with an empty notebook.
	let tomorrow = support::tomorrow();
	let next = open(&tomorrow).await;
	let old = ada.get(&next.site, "/demo/notes").await;
	assert_no_store(&old);
	assert_eq!(old.status(), StatusCode::UNAUTHORIZED);
	let old = json_body(old).await;
	assert_eq!(old["pass"], "new_day", "{old}");
	assert_eq!(old["day"], tomorrow.as_str());
	let refused = ada.note(&next.site, "from yesterday").await;
	assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);

	let mut fresh = Browser::default();
	sign_in(&mut fresh, &next.site, "ada").await;
	assert_eq!(fresh.list(&next.site).await["notes"], json!([]));
	assert!(
		visitor_events(&next.loopback, &next.secrets.analytics_token)
			.await
			.is_empty(),
		"the new day's service has no note events"
	);
	drop(next);
	Ok(())
}
