//! The visitor sign-in proofs, through the real site router (and, for
//! the admin, next-day, and secret-surface proofs, through real
//! generations).
//!
//! Nothing here forges a session: every account is created by the real
//! emailed-code flow, every admin sign-in goes through the real OIDC
//! redirects with the browser's cookies, and the mail a visitor reads is
//! the mail the real [`Mailer`](leptoskit_auth::Mailer) kept. The demo
//! inbox's bounds, TTL, and receipts get focused unit tests in
//! `src/inbox.rs`; this file proves the routes' contracts end to end.
//!
//! Two site instances on one file database stand in for two service
//! processes: they share the durable state and the keys, and their
//! in-memory inboxes are honestly separate.

#![cfg(unix)]

mod support;

use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Result;
use axum::Router;
use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use axum::http::header;
use demo_service::generation;
use demo_service::site;
use futures::future::join_all;
use leptoskit_auth::Account;
use leptoskit_auth::Keys;
use leptoskit_auth::RequestInfo;
use leptoskit_db::Db;
use serde_json::Value;
use serde_json::json;
use support::ADMIN_HOST;
use support::SITE_HOST;
use support::volume;
use tower::ServiceExt as _;

/// The site origin the HTTP proofs use: a loopback development name.
const SITE_ORIGIN: &str = "http://localhost";
/// The admin origin, a different loopback host.
const ADMIN_ORIGIN: &str = "http://127.0.0.1";
/// The site origin the HTTPS proof uses, so the `__Host-` cookie rules
/// apply.
const SECURE_SITE_ORIGIN: &str = "https://notes.example";
/// The admin origin that pairs with the secure site.
const SECURE_ADMIN_ORIGIN: &str = "https://admin.example";
/// The secure site's host, as its Host header spells it.
const SECURE_SITE_HOST: &str = "notes.example";
/// A fixed test key: the proofs care about the contract, not secrecy.
const TEST_SITE_KEY: [u8; 32] = [9; 32];

/// A browser: one cookie jar per origin it visits.
#[derive(Default)]
struct Browser {
	jars: HashMap<String, HashMap<String, String>>,
}

impl Browser {
	/// Sends a request as this browser, carrying its cookies for
	/// `origin`, and stores whatever `Set-Cookie` answers arrive.
	async fn send(
		&mut self,
		router: &Router,
		origin: &str,
		request: axum::http::request::Builder,
		body: Body,
	) -> axum::response::Response {
		let jar = self.jars.entry(origin.to_owned()).or_default();
		let cookies = jar
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
				jar.remove(name);
			} else {
				jar.insert(name.to_owned(), cookie.to_owned());
			}
		}

		response
	}

	/// A GET with this browser's cookies.
	async fn get(&mut self, router: &Router, origin: &str, path: &str) -> axum::response::Response {
		let request = Request::get(path).header(header::HOST, host_of(origin));
		self.send(router, origin, request, Body::empty()).await
	}

	/// A JSON POST with this browser's cookies and its Origin.
	async fn post_json(
		&mut self,
		router: &Router,
		origin: &str,
		path: &str,
		body: Value,
	) -> axum::response::Response {
		let request = Request::post(path)
			.header(header::HOST, host_of(origin))
			.header(header::ORIGIN, origin)
			.header(header::CONTENT_TYPE, "application/json");
		self.send(router, origin, request, Body::from(body.to_string()))
			.await
	}

	/// A form POST with this browser's cookies, an optional Origin, and
	/// any extra headers.
	async fn post_form(
		&mut self,
		router: &Router,
		host: &str,
		origin: Option<&str>,
		path: &str,
		headers: &[(&str, &str)],
		body: &str,
	) -> axum::response::Response {
		let mut request = Request::post(path)
			.header(header::HOST, host)
			.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");

		if let Some(value) = origin {
			request = request.header(header::ORIGIN, value);
		}
		for (name, value) in headers {
			request = request.header(*name, *value);
		}

		self.send(
			router,
			origin.unwrap_or(host),
			request,
			Body::from(body.to_owned()),
		)
		.await
	}

	/// The pass cookie this browser carries, if any.
	fn pass(&self, origin: &str) -> Option<String> {
		self.jars
			.get(origin)
			.and_then(|jar| jar.get("demo-pass").cloned())
	}
}

/// The Host header an origin's requests carry.
fn host_of(origin: &str) -> String {
	url::Url::parse(origin)
		.expect("a test origin")
		.host_str()
		.expect("a test host")
		.to_owned()
}

/// A body, as JSON.
async fn json_body(response: axum::response::Response) -> Value {
	let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
		.await
		.expect("a readable body");
	serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

/// A body, as lossy text.
async fn text_body(response: axum::response::Response) -> String {
	let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
		.await
		.expect("a readable body");
	String::from_utf8_lossy(&bytes).into_owned()
}

/// Where a redirect sends the browser.
fn location(response: &axum::response::Response) -> String {
	response.headers()[header::LOCATION]
		.to_str()
		.expect("a location header")
		.to_owned()
}

/// The site options one proof runs under: today's day, no IP header.
fn options() -> site::SiteOptions {
	site::SiteOptions::new(
		demo_service::clock::day_of(demo_service::clock::now_millis()),
		None,
	)
}

/// Mints a pass and persona for a browser through `router`.
async fn mint(browser: &mut Browser, router: &Router, persona: &str) -> axum::response::Response {
	let body = format!("persona={persona}");
	browser
		.post_form(
			router,
			SITE_HOST,
			Some(SITE_ORIGIN),
			"/demo/pass",
			&[],
			&body,
		)
		.await
}

/// Sends a code to `address` through the gated route.
async fn send_code(
	browser: &mut Browser,
	router: &Router,
	address: &str,
) -> axum::response::Response {
	browser
		.post_json(
			router,
			SITE_ORIGIN,
			"/auth/email/send",
			json!({ "email": address }),
		)
		.await
}

/// Reads the inbox and returns its JSON, proving `no-store` on the way.
async fn inbox(browser: &mut Browser, router: &Router) -> Value {
	let response = browser.get(router, SITE_ORIGIN, "/demo/inbox").await;
	assert_eq!(response.status(), StatusCode::OK, "the inbox answers");
	assert!(
		response
			.headers()
			.get(header::CACHE_CONTROL)
			.is_some_and(|value| value.to_str().expect("a header").contains("no-store")),
		"the inbox is never cached"
	);
	json_body(response).await
}

/// Reads the inbox as a second browser that carries only a given pass
/// cookie — the honest way to ask a reopened instance what it holds.
async fn inbox_with_pass(router: &Router, pass: &str) -> Value {
	let mut browser = Browser::default();
	browser
		.jars
		.entry(SITE_ORIGIN.to_owned())
		.or_default()
		.insert("demo-pass".to_owned(), pass.to_owned());
	inbox(&mut browser, router).await
}

/// The code a subject carries.
fn code_of(subject: &str) -> String {
	subject
		.rsplit(' ')
		.next()
		.expect("a subject with a code")
		.to_owned()
}

/// The inbox view of the browser's only unclaimed person, then a real
/// send → read → verify sign-in, returning the account id the session
/// names.
async fn sign_in_unclaimed(browser: &mut Browser, router: &Router) -> String {
	let view = inbox(browser, router).await;
	let person = view["people"]
		.as_array()
		.expect("people")
		.iter()
		.find(|person| person["signed_in"].as_bool() == Some(false))
		.expect("an unclaimed person to sign in as")
		.clone();
	let address = person["email"].as_str().expect("an email").to_owned();
	sign_in_address(browser, router, &address).await
}

/// The real send → read → verify sign-in for `address`, returning the
/// account id the session names.
async fn sign_in_address(browser: &mut Browser, router: &Router, address: &str) -> String {
	let sent = send_code(browser, router, address).await;
	assert_eq!(sent.status(), StatusCode::NO_CONTENT, "the code is sent");

	let view = inbox(browser, router).await;
	let message = view["messages"]
		.as_array()
		.expect("messages")
		.iter()
		.find(|message| message["to"].as_str() == Some(address))
		.expect("the code arrives");
	let code = code_of(message["subject"].as_str().expect("a subject"));

	let verified = browser
		.post_json(
			router,
			SITE_ORIGIN,
			"/auth/email/verify",
			json!({ "email": address, "code": code }),
		)
		.await;
	assert_eq!(
		verified.status(),
		StatusCode::OK,
		"the code signs in: {}",
		text_body(verified).await
	);

	let session = browser.get(router, SITE_ORIGIN, "/auth/session").await;
	let session = json_body(session).await;
	session["session"]["account_id"]
		.as_str()
		.expect("a signed-in account")
		.to_owned()
}

/// A site over a file database, with everything a proof needs to keep
/// asking questions about its durable state.
struct SiteHarness {
	db: Db,
	keys: Keys,
	day: String,
	/// The directory must outlive the databases; it is never read again.
	#[allow(dead_code, reason = "the directory must outlive the databases")]
	dir: PathBuf,
}

impl SiteHarness {
	/// A fresh harness: an empty directory and the keys one day's site
	/// runs on.
	async fn new() -> Self {
		let dir = volume();
		let day = demo_service::clock::day_of(demo_service::clock::now_millis());
		let keys = Keys::single(&generation::site_key_id(&day), TEST_SITE_KEY);
		let db = Db::connect(&format!("sqlite://{}", dir.join("site.sqlite3").display()))
			.await
			.expect("the site database opens");
		leptoskit_db::testing::create_tables(&db)
			.await
			.expect("the site's tables create");

		Self { db, keys, day, dir }
	}

	/// Builds the site's routes.
	async fn build(&self, options: &site::SiteOptions, admin_origin: &str) -> site::Site {
		site::build(
			&self.db,
			self.keys.clone(),
			SITE_ORIGIN,
			admin_origin,
			options,
		)
		.await
		.expect("the site builds")
	}

	/// A second, independently opened handle on the same file database,
	/// rebuilt as a second site instance.
	async fn second_instance(&self, options: &site::SiteOptions) -> Router {
		let db = Db::connect(&format!(
			"sqlite://{}",
			self.dir.join("site.sqlite3").display()
		))
		.await
		.expect("the second handle opens");
		site::rebuild(&db, self.keys.clone(), SITE_ORIGIN, options).expect("the site reopens")
	}

	/// A system handle for proofs that read what visitors must not.
	fn system(&self) -> Db {
		self.db.system("demo sign-in proof")
	}
}

/// Signs a site session in to the admin through the real OIDC flow, and
/// returns the redirect the admin's callback answered with.
async fn admin_flow(
	browser: &mut Browser,
	site: &Router,
	admin: &Router,
	admin_origin: &str,
) -> String {
	let started = browser
		.post_json(
			admin,
			admin_origin,
			"/auth/oauth/start",
			json!({ "provider": "main" }),
		)
		.await;
	assert_eq!(started.status(), StatusCode::OK, "the admin flow starts");
	let url = json_body(started).await["url"]
		.as_str()
		.expect("an authorize url")
		.to_owned();

	let back = browser.get(site, SITE_ORIGIN, &url).await;
	let back = location(&back);
	assert!(
		back.starts_with(&format!("{admin_origin}/auth/callback/main")),
		"{back}"
	);

	let finished = browser.get(admin, admin_origin, &back).await;
	if finished.status() == StatusCode::SEE_OTHER {
		location(&finished)
	} else {
		text_body(finished).await
	}
}

fn test_authorities() -> demo_service::config::Authorities {
	let authority = |host: &str| {
		demo_service::config::Authority::of_host_header(host).expect("a test authority")
	};
	demo_service::config::Authorities {
		site: authority(SITE_HOST),
		admin: authority(ADMIN_HOST),
		bind: authority("127.0.0.1:8080"),
	}
}

/// A generation for the process-level proofs: two authorities, one
/// temporary volume.
async fn a_generation(root: &Path, day: &str) -> Result<generation::Generation> {
	let config = demo_service::config::Config {
		bind: "127.0.0.1:0".parse().expect("a bind address"),
		volume: root.to_path_buf(),
		site_origin: SITE_ORIGIN.to_owned(),
		admin_origin: ADMIN_ORIGIN.to_owned(),
		authorities: test_authorities(),
		client_ip_header: None,
	};
	let terminated = demo_service::shutdown::init();
	let readiness = leptoskit_axum::Readiness::new();
	generation::open_or_build(
		&config,
		day,
		demo_service::clock::now_millis(),
		&terminated,
		&readiness,
	)
	.await
}

// ─── Minting: cookies, origins, reuse ─────────────────────────────────

/// The `__Host-` cookie rules on an HTTPS origin, the same-origin
/// refusal, Fetch Metadata never excusing a contradictory Origin, the
/// validated `return_to`, and the unknown-cookie states the inbox names.
#[tokio::test]
async fn a_mint_sets_host_cookies_and_refuses_foreign_origins() -> Result<()> {
	let harness = SiteHarness::new().await;
	let secure = site::SiteOptions::new(harness.day.clone(), None);
	let built = site::build(
		&harness.db,
		harness.keys.clone(),
		SECURE_SITE_ORIGIN,
		SECURE_ADMIN_ORIGIN,
		&secure,
	)
	.await
	.expect("the secure site builds");
	let mut browser = Browser::default();
	let body = "persona=ada";

	// No Origin and no Fetch Metadata: refused. The helper sends an
	// Origin only when asked, so `None` here means none.
	let refused = browser
		.post_form(
			&built.router,
			SECURE_SITE_HOST,
			None,
			"/demo/pass",
			&[],
			body,
		)
		.await;
	assert_eq!(refused.status(), StatusCode::FORBIDDEN);
	assert_eq!(json_body(refused).await["error"], "cross_origin");

	// A foreign Origin is refused whatever the metadata claims. The
	// session layer's same-site protection sits outside the gate, so the
	// refusal is Auth's own stable 403 — either way, the mint never
	// happens, and Fetch Metadata never excuses the Origin.
	let response = browser
		.post_form(
			&built.router,
			SECURE_SITE_HOST,
			Some("https://evil.example"),
			"/demo/pass",
			&[("sec-fetch-site", "same-origin")],
			body,
		)
		.await;
	assert_eq!(
		response.status(),
		StatusCode::FORBIDDEN,
		"a contradictory Origin"
	);
	assert_eq!(
		json_body(response).await["error"],
		"forbidden",
		"the same-site layer refuses it before any mint"
	);

	// Same-origin Fetch Metadata alone: accepted, because Origin is
	// absent.
	let request = Request::post("/demo/pass")
		.header(header::HOST, "notes.example")
		.header("sec-fetch-site", "same-origin")
		.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
	let response = browser
		.send(&built.router, SECURE_SITE_ORIGIN, request, Body::from(body))
		.await;
	assert_eq!(
		response.status(),
		StatusCode::SEE_OTHER,
		"metadata stands in"
	);

	let cookie = response
		.headers()
		.get(header::SET_COOKIE)
		.expect("a fresh mint sets the pass cookie")
		.to_str()
		.expect("a header")
		.to_owned();
	assert!(cookie.starts_with("__Host-demo-pass="), "{cookie}");
	assert!(cookie.contains("Path=/"), "{cookie}");
	assert!(cookie.contains("HttpOnly"), "{cookie}");
	assert!(cookie.contains("SameSite=Lax"), "{cookie}");
	assert!(cookie.contains("Secure"), "{cookie}");
	assert!(cookie.contains("Max-Age="), "{cookie}");
	assert!(!cookie.to_ascii_lowercase().contains("domain"), "{cookie}");

	let target = location(&response);
	assert!(target.starts_with("/sign-in?as=ada&return_to="), "{target}");

	// A validated return_to travels percent-encoded; a dangerous one
	// falls back to the app's own page.
	let evil = browser
		.post_form(
			&built.router,
			SECURE_SITE_HOST,
			Some(SECURE_SITE_ORIGIN),
			"/demo/pass",
			&[],
			"persona=ada&return_to=%2F%5Cevil.example",
		)
		.await;
	assert_eq!(
		location(&evil),
		"/sign-in?as=ada&return_to=%2Fnotes",
		"a backslash path falls back"
	);

	let legit = browser
		.post_form(
			&built.router,
			SECURE_SITE_HOST,
			Some(SECURE_SITE_ORIGIN),
			"/demo/pass",
			&[],
			"persona=grace&return_to=%2Foauth%2Fauthorize%3Fclient_id%3Dabc",
		)
		.await;
	assert_eq!(
		location(&legit),
		"/sign-in?as=grace&return_to=%2Foauth%2Fauthorize%3Fclient_id%3Dabc",
		"a legitimate OIDC target survives"
	);

	// An unknown persona is a bad request, not a mint.
	let unknown = browser
		.post_form(
			&built.router,
			SECURE_SITE_HOST,
			Some(SECURE_SITE_ORIGIN),
			"/demo/pass",
			&[],
			"persona=admin",
		)
		.await;
	assert_eq!(unknown.status(), StatusCode::BAD_REQUEST);
	Ok(())
}

/// Unusable pass cookies are absent and cleared, and an earlier day is
/// named as a new day for the front door.
#[tokio::test]
async fn unusable_pass_cookies_are_cleared_and_named() -> Result<()> {
	let harness = SiteHarness::new().await;
	let secure = site::SiteOptions::new(harness.day.clone(), None);
	let built = site::build(
		&harness.db,
		harness.keys.clone(),
		SECURE_SITE_ORIGIN,
		SECURE_ADMIN_ORIGIN,
		&secure,
	)
	.await
	.expect("the secure site builds");

	// An unknown token for today is absent, and cleared.
	let mut stranger = Browser::default();
	stranger
		.jars
		.entry(SECURE_SITE_ORIGIN.to_owned())
		.or_default()
		.insert(
			"__Host-demo-pass".to_owned(),
			format!("{}.{}", harness.day, "a".repeat(64)),
		);
	let unknown_pass = stranger
		.get(&built.router, SECURE_SITE_ORIGIN, "/demo/inbox")
		.await;
	assert_eq!(unknown_pass.status(), StatusCode::UNAUTHORIZED);
	let view = json_body(unknown_pass).await;
	assert_eq!(view["pass"], "absent", "{view}");
	assert!(
		stranger.pass(SECURE_SITE_ORIGIN).is_none(),
		"the unknown cookie is cleared"
	);

	// An earlier day is cleared, and named as a new day for the front
	// door.
	let mut yesterday = Browser::default();
	yesterday
		.jars
		.entry(SECURE_SITE_ORIGIN.to_owned())
		.or_default()
		.insert(
			"__Host-demo-pass".to_owned(),
			format!("2020-01-01.{}", "b".repeat(64)),
		);
	let old_day = yesterday
		.get(&built.router, SECURE_SITE_ORIGIN, "/demo/inbox")
		.await;
	assert_eq!(old_day.status(), StatusCode::UNAUTHORIZED);
	let view = json_body(old_day).await;
	assert_eq!(view["pass"], "new_day", "{view}");
	assert_eq!(view["day"], harness.day.as_str(), "the new day is named");
	assert!(
		yesterday.pass(SECURE_SITE_ORIGIN).is_none(),
		"the old-day cookie is cleared"
	);
	Ok(())
}

/// Two independent browser jars get distinct people and, after real
/// sign-ins, distinct accounts; one jar reuses its persona, adds the
/// other at most, and the roles follow the persona.
#[tokio::test]
async fn two_jars_get_distinct_people_and_accounts() -> Result<()> {
	let harness = SiteHarness::new().await;
	let built = harness.build(&options(), ADMIN_ORIGIN).await;
	let mut ada_browser = Browser::default();
	let mut grace_browser = Browser::default();

	let ada = mint(&mut ada_browser, &built.router, "ada").await;
	assert_eq!(ada.status(), StatusCode::SEE_OTHER);
	let ada_cookie = ada
		.headers()
		.get(header::SET_COOKIE)
		.expect("a cookie")
		.to_str()
		.expect("a header");
	assert!(ada_cookie.starts_with("demo-pass="), "{ada_cookie}");
	assert!(!ada_cookie.contains("Secure"), "plain HTTP is not Secure");
	assert!(!ada_cookie.to_ascii_lowercase().contains("domain"));

	let grace = mint(&mut grace_browser, &built.router, "grace").await;
	assert_eq!(grace.status(), StatusCode::SEE_OTHER);

	let ada_view = inbox(&mut ada_browser, &built.router).await;
	let grace_view = inbox(&mut grace_browser, &built.router).await;
	assert_eq!(ada_view["people"].as_array().expect("people").len(), 1);
	assert_eq!(grace_view["people"].as_array().expect("people").len(), 1);
	let ada_email = ada_view["people"][0]["email"].as_str().expect("email");
	let grace_email = grace_view["people"][0]["email"].as_str().expect("email");
	assert_ne!(ada_email, grace_email, "two jars, two people");
	assert!(ada_email.starts_with("ada-") && ada_email.ends_with("@demo.invalid"));
	assert!(grace_email.starts_with("grace-"));

	// The display tag is the address's random part, uppercased.
	let ada_tag = ada_view["people"][0]["tag"].as_str().expect("a tag");
	assert_eq!(ada_tag.len(), 4, "four characters");
	assert_eq!(ada_tag, ada_tag.to_ascii_uppercase());
	assert_eq!(
		ada_view["people"][0]["name"].as_str(),
		Some(format!("Ada #{ada_tag}").as_str())
	);

	// Minting the same persona again reuses the identity and sets no new
	// cookie.
	let again = mint(&mut ada_browser, &built.router, "ada").await;
	assert_eq!(again.status(), StatusCode::SEE_OTHER);
	assert!(
		again
			.headers()
			.get_all(header::SET_COOKIE)
			.iter()
			.next()
			.is_none(),
		"a reuse sets no cookie"
	);
	let again_view = inbox(&mut ada_browser, &built.router).await;
	assert_eq!(
		again_view["people"][0]["email"].as_str(),
		Some(ada_email),
		"the persona is reused, not re-minted"
	);

	// Switching adds the other persona on the same pass.
	let switched = mint(&mut ada_browser, &built.router, "grace").await;
	assert_eq!(switched.status(), StatusCode::SEE_OTHER);
	let switched_view = inbox(&mut ada_browser, &built.router).await;
	let people = switched_view["people"].as_array().expect("people");
	assert_eq!(people.len(), 2, "one pass, one Ada, one Grace");
	assert!(people.iter().any(|person| person["persona"] == "ada"));
	assert!(people.iter().any(|person| person["persona"] == "grace"));

	// Real sign-ins: two jars, two accounts.
	let ada_account = sign_in_unclaimed(&mut ada_browser, &built.router).await;
	let grace_account = sign_in_unclaimed(&mut grace_browser, &built.router).await;
	assert_ne!(ada_account, grace_account, "two jars, two accounts");

	// Ada carries the admin role; Grace does not.
	let ada_id: leptoskit_db::Id<Account> = ada_account.parse().expect("an account id");
	let grace_id: leptoskit_db::Id<Account> = grace_account.parse().expect("an account id");
	assert_eq!(
		built.auth().roles(ada_id).await.expect("roles"),
		["admin"],
		"Ada runs the place"
	);
	assert!(
		built
			.auth()
			.roles(grace_id)
			.await
			.expect("roles")
			.is_empty(),
		"Grace writes notes"
	);
	Ok(())
}

// ─── The gate: ownership, isolation, and no side effects ──────────────

/// A missing or foreign identity is refused before Auth writes a
/// challenge, mails, or counts a rate-limit hit; a query parameter
/// cannot change whose mail the inbox shows; bounded bodies only.
#[tokio::test]
async fn the_gate_refuses_foreign_identities_with_no_auth_effects() -> Result<()> {
	let harness = SiteHarness::new().await;
	let built = harness.build(&options(), ADMIN_ORIGIN).await;
	let system = harness.system();

	let mut ada = Browser::default();
	let mut grace = Browser::default();
	mint(&mut ada, &built.router, "ada").await;
	mint(&mut grace, &built.router, "grace").await;

	let ada_view = inbox(&mut ada, &built.router).await;
	let ada_address = ada_view["people"][0]["email"]
		.as_str()
		.expect("email")
		.to_owned();
	let grace_view = inbox(&mut grace, &built.router).await;
	let grace_address = grace_view["people"][0]["email"]
		.as_str()
		.expect("email")
		.to_owned();

	// No cookie at all: refused, and nothing anywhere moved.
	let mut stranger = Browser::default();
	let refused = send_code(&mut stranger, &built.router, &ada_address).await;
	assert_eq!(refused.status(), StatusCode::FORBIDDEN);
	assert_eq!(json_body(refused).await["error"], "not_your_identity");

	// Another pass's address: refused the same way, with no mail.
	let refused = send_code(&mut ada, &built.router, &grace_address).await;
	assert_eq!(refused.status(), StatusCode::FORBIDDEN);
	assert_eq!(json_body(refused).await["error"], "not_your_identity");

	// A verify for a foreign identity is refused before any code check.
	let refused = grace
		.post_json(
			&built.router,
			SITE_ORIGIN,
			"/auth/email/verify",
			json!({ "email": ada_address, "code": "00000000" }),
		)
		.await;
	assert_eq!(refused.status(), StatusCode::FORBIDDEN);
	assert_eq!(json_body(refused).await["error"], "not_your_identity");

	// None of those touched Auth's tables: no challenge, no rate limit.
	assert_eq!(
		leptoskit_auth::Challenge::query()
			.count(&system)
			.await
			.expect("challenges count"),
		0,
		"no challenge was written"
	);
	assert_eq!(
		leptoskit_auth::RateLimit::query()
			.count(&system)
			.await
			.expect("rate limits count"),
		0,
		"no rate limit was hit"
	);

	// A body past the bound is refused before anything is parsed.
	let oversized = ada
		.send(
			&built.router,
			SITE_ORIGIN,
			Request::post("/auth/email/send")
				.header(header::HOST, SITE_HOST)
				.header(header::ORIGIN, SITE_ORIGIN)
				.header(header::CONTENT_TYPE, "application/json"),
			Body::from(format!("{{\"email\":\"{}\"}}", "a".repeat(4096))),
		)
		.await;
	assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);

	// A body with no address is refused.
	let malformed = ada
		.post_json(&built.router, SITE_ORIGIN, "/auth/email/send", json!({}))
		.await;
	assert_eq!(malformed.status(), StatusCode::BAD_REQUEST);

	// The inbox answers no query: ownership is the cookie's alone.
	let with_query = grace
		.get(
			&built.router,
			SITE_ORIGIN,
			&format!("/demo/inbox?email={ada_address}"),
		)
		.await;
	assert_eq!(with_query.status(), StatusCode::OK);
	let queried = json_body(with_query).await;
	assert!(
		queried["messages"].as_array().expect("messages").is_empty(),
		"Grace's inbox stays empty, whatever the query asks"
	);
	assert_eq!(
		queried["people"].as_array().expect("people").len(),
		1,
		"only the pass's own people"
	);

	// Ada's own address goes through, exactly once, with one message.
	let sent = send_code(&mut ada, &built.router, &ada_address).await;
	assert_eq!(sent.status(), StatusCode::NO_CONTENT);
	let ada_after = inbox(&mut ada, &built.router).await;
	let messages = ada_after["messages"].as_array().expect("messages");
	assert_eq!(messages.len(), 1, "exactly one message");
	assert_eq!(messages[0]["to"].as_str(), Some(ada_address.as_str()));
	assert!(
		!messages[0]["receipt"]
			.as_str()
			.expect("a receipt")
			.is_empty(),
		"every message carries its receipt id"
	);
	assert!(
		!messages[0]["text"].as_str().expect("text").is_empty(),
		"the code's text arrives with it"
	);
	Ok(())
}

// ─── Reopen: shared state, unavailable receipts ───────────────────────

/// Fresh mail is never marked lost; a same-day reopen shares the passes,
/// people, and keys but loses the inbox memory, which reports
/// unavailable; a send through the reopened instance marks the older
/// one's view unavailable even while it retains older mail; and resend,
/// read, and verify work on the reopened instance.
#[tokio::test]
async fn a_reopen_reports_unavailable_receipts_and_still_signs_in() -> Result<()> {
	let harness = SiteHarness::new().await;
	let built = harness.build(&options(), ADMIN_ORIGIN).await;
	let mut ada = Browser::default();

	mint(&mut ada, &built.router, "ada").await;
	let view = inbox(&mut ada, &built.router).await;
	let address = view["people"][0]["email"]
		.as_str()
		.expect("email")
		.to_owned();

	// A fresh send is never lost: the mailer records the receipt before
	// Auth answers, and the inbox holds the matching message.
	let sent = send_code(&mut ada, &built.router, &address).await;
	assert_eq!(sent.status(), StatusCode::NO_CONTENT);
	let first = inbox(&mut ada, &built.router).await;
	assert!(
		first["lost"].as_array().expect("lost").is_empty(),
		"fresh mail is not lost: {first}"
	);
	assert_eq!(first["messages"].as_array().expect("messages").len(), 1);

	// Reopen the same durable state as a second instance: same passes,
	// same people, same keys, empty inbox memory.
	let pass = ada.pass(SITE_ORIGIN).expect("the pass cookie");
	let reopened = harness.second_instance(&options()).await;

	let reopened_view = inbox_with_pass(&reopened, &pass).await;
	assert_eq!(
		reopened_view["people"].as_array().expect("people").len(),
		1,
		"the pass and its people survive the reopen"
	);
	assert!(
		reopened_view["messages"]
			.as_array()
			.expect("messages")
			.is_empty(),
		"the second instance holds no mail"
	);
	let lost = reopened_view["lost"].as_array().expect("lost");
	assert_eq!(lost.len(), 1, "{reopened_view}");
	assert_eq!(lost[0]["persona"], "ada", "the unavailable mail is Ada's");
	assert!(
		lost[0]["sent_at"].as_str().is_some(),
		"the unavailable receipt names when it was sent"
	);

	// Sending through the reopened instance works — the durable person
	// is still there to claim the receipt.
	let resent = {
		let mut reopened_browser = Browser::default();
		reopened_browser
			.jars
			.entry(SITE_ORIGIN.to_owned())
			.or_default()
			.insert("demo-pass".to_owned(), pass.clone());
		send_code(&mut reopened_browser, &reopened, &address).await
	};
	assert_eq!(resent.status(), StatusCode::NO_CONTENT, "resend works");

	// The reopened instance now holds the new mail; the old one honestly
	// reports the newest receipt as unavailable, even while it still
	// retains its own older message.
	let reopened_view = inbox_with_pass(&reopened, &pass).await;
	assert_eq!(
		reopened_view["messages"]
			.as_array()
			.expect("messages")
			.len(),
		1,
		"the resend is readable on the instance that sent it"
	);
	assert_eq!(
		reopened_view["lost"].as_array().expect("lost").len(),
		0,
		"the resend's own receipt is not lost to its sender"
	);

	let old_view = inbox_with_pass(&built.router, &pass).await;
	assert_eq!(
		old_view["messages"].as_array().expect("messages").len(),
		1,
		"the old instance still holds its own older mail"
	);
	let lost = old_view["lost"].as_array().expect("lost");
	assert_eq!(lost.len(), 1, "{old_view}");
	assert_eq!(
		lost[0]["persona"], "ada",
		"the newest receipt is unavailable here"
	);

	// The code that is actually readable signs in — on the instance that
	// holds it.
	let code = code_of(
		reopened_view["messages"][0]["subject"]
			.as_str()
			.expect("subject"),
	);
	let mut reopened_browser = Browser::default();
	reopened_browser
		.jars
		.entry(SITE_ORIGIN.to_owned())
		.or_default()
		.insert("demo-pass".to_owned(), pass);
	let verified = reopened_browser
		.post_json(
			&reopened,
			SITE_ORIGIN,
			"/auth/email/verify",
			json!({ "email": address, "code": code }),
		)
		.await;
	assert_eq!(
		verified.status(),
		StatusCode::OK,
		"the reopened instance signs the visitor in: {}",
		text_body(verified).await
	);
	Ok(())
}

/// An address whose pre-minted person is gone creates no account, even
/// with a code that checks out: the sign-up hook claims pre-minted
/// people only.
#[tokio::test]
async fn an_unminted_address_creates_no_account() -> Result<()> {
	let harness = SiteHarness::new().await;
	let built = harness.build(&options(), ADMIN_ORIGIN).await;
	let system = harness.system();
	let mut ada = Browser::default();

	mint(&mut ada, &built.router, "ada").await;
	let view = inbox(&mut ada, &built.router).await;
	let address = view["people"][0]["email"]
		.as_str()
		.expect("email")
		.to_owned();

	// Send a real code, then remove the person it was minted for: the
	// address now has a live challenge and no pre-minted identity, which
	// is exactly what an unminted address looks like at the hook.
	let sent = send_code(&mut ada, &built.router, &address).await;
	assert_eq!(sent.status(), StatusCode::NO_CONTENT);
	let view = inbox(&mut ada, &built.router).await;
	let code = code_of(view["messages"][0]["subject"].as_str().expect("subject"));

	let deleted = demo_service::visitors::DemoPerson::query()
		.filter(|person| {
			person
				.email
				.eq(leptoskit_db::Email::parse(&address).expect("a minted address"))
		})
		.delete(&system)
		.await
		.expect("the person row deletes");
	assert_eq!(deleted, 1, "the pre-minted person is gone");

	let accounts_before = Account::query().count(&system).await.expect("accounts");
	let step = built
		.auth()
		.verify_email_code(&address, &code, &RequestInfo::default())
		.await;
	assert!(
		step.is_err(),
		"verification without a pre-minted person refuses"
	);
	let accounts_after = Account::query().count(&system).await.expect("accounts");
	assert_eq!(
		accounts_before, accounts_after,
		"no account was created for an unminted address"
	);
	Ok(())
}

// ─── Caps: atomic across two instances, honest when full ──────────────

/// The per-IP hourly cap and the day cap hold under concurrent mints
/// against two independently opened site instances on one file
/// database, existing passes keep signing in when the day is full, and
/// a configured IP header that is unusable refuses the request outright.
#[tokio::test]
async fn caps_hold_under_concurrency_across_two_instances() -> Result<()> {
	let harness = SiteHarness::new().await;
	let options = site::SiteOptions::new(harness.day.clone(), Some("x-forwarded-for".to_owned()))
		.with_test_caps(2, 6);
	let first = harness.build(&options, ADMIN_ORIGIN).await;
	let second = harness.second_instance(&options).await;

	// One visitor keeps a pass from before the caps fill.
	let mut keeper = Browser::default();
	let kept = mint(&mut keeper, &first.router, "grace").await;
	assert_eq!(kept.status(), StatusCode::SEE_OTHER);
	let keeper_view = inbox(&mut keeper, &first.router).await;
	let keeper_address = keeper_view["people"][0]["email"]
		.as_str()
		.expect("email")
		.to_owned();

	// Concurrent same-IP mints across both instances: the per-IP cap
	// holds exactly, with Retry-After on every refusal.
	let same_ip: Vec<(StatusCode, bool)> = join_all((0..8).map(|index| {
		let router = if index % 2 == 0 {
			first.router.clone()
		} else {
			second.clone()
		};
		async move {
			let mut browser = Browser::default();
			let response = browser
				.post_form(
					&router,
					SITE_HOST,
					Some(SITE_ORIGIN),
					"/demo/pass",
					&[("x-forwarded-for", "203.0.113.9")],
					"persona=ada",
				)
				.await;
			let retry_after = response
				.headers()
				.get(header::RETRY_AFTER)
				.is_some_and(|value| {
					value
						.to_str()
						.expect("a header")
						.parse::<u64>()
						.expect("a retry-after")
						>= 1
				});
			(response.status(), retry_after)
		}
	}))
	.await;
	let minted = same_ip
		.iter()
		.filter(|(status, _)| *status == StatusCode::SEE_OTHER)
		.count();
	assert_eq!(minted, 2, "the per-IP cap holds exactly");
	assert!(
		same_ip.iter().all(|(status, retry_after)| {
			*status == StatusCode::SEE_OTHER
				|| (*status == StatusCode::TOO_MANY_REQUESTS && *retry_after)
		}),
		"same-IP mints either pass or answer 429 with Retry-After: {same_ip:?}"
	);

	// Distinct IPs mint up to the day cap; everything after is 503 full.
	let distinct: Vec<StatusCode> = join_all((0..12).map(|index| {
		let router = if index % 2 == 0 {
			first.router.clone()
		} else {
			second.clone()
		};
		async move {
			let mut browser = Browser::default();
			let response = browser
				.post_form(
					&router,
					SITE_HOST,
					Some(SITE_ORIGIN),
					"/demo/pass",
					&[("x-forwarded-for", &format!("198.51.100.{index}"))],
					"persona=ada",
				)
				.await;
			response.status()
		}
	}))
	.await;
	assert!(
		distinct.iter().all(|status| {
			*status == StatusCode::SEE_OTHER || *status == StatusCode::SERVICE_UNAVAILABLE
		}),
		"distinct-IP mints either pass or answer 503: {distinct:?}"
	);

	let system = harness.system();
	let total = demo_service::visitors::DemoPass::query()
		.count(&system)
		.await
		.expect("passes count");
	assert_eq!(
		total, 6,
		"the day cap holds exactly across both instances (1 keeper + caps)"
	);

	// The full answer is a stable code.
	let full = {
		let mut browser = Browser::default();
		browser
			.post_form(
				&second,
				SITE_HOST,
				Some(SITE_ORIGIN),
				"/demo/pass",
				&[("x-forwarded-for", "198.51.100.99")],
				"persona=ada",
			)
			.await
	};
	assert_eq!(full.status(), StatusCode::SERVICE_UNAVAILABLE);
	assert_eq!(json_body(full).await["error"], "full");

	// The keeper's pass still signs in when the day is full.
	let session = sign_in_address(&mut keeper, &first.router, &keeper_address).await;
	assert!(!session.is_empty(), "an existing pass signs in when full");

	// A malformed IP header refuses the mint outright: never an
	// unlimited caller.
	for value in ["not-an-ip", "203.0.113.9, 198.51.100.7", ""] {
		let refused = {
			let mut browser = Browser::default();
			browser
				.post_form(
					&second,
					SITE_HOST,
					Some(SITE_ORIGIN),
					"/demo/pass",
					&[("x-forwarded-for", value)],
					"persona=ada",
				)
				.await
		};
		assert_eq!(
			refused.status(),
			StatusCode::BAD_REQUEST,
			"{value:?} is refused, not trusted"
		);
		assert_eq!(json_body(refused).await["error"], "bad_client_ip");
	}

	// A duplicated header is refused the same way.
	let duplicated = {
		let mut browser = Browser::default();
		let request = Request::post("/demo/pass")
			.header(header::HOST, SITE_HOST)
			.header(header::ORIGIN, SITE_ORIGIN)
			.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
			.header("x-forwarded-for", "203.0.113.9")
			.header("x-forwarded-for", "203.0.113.8");
		browser
			.send(&second, SITE_ORIGIN, request, Body::from("persona=ada"))
			.await
	};
	assert_eq!(duplicated.status(), StatusCode::BAD_REQUEST);
	Ok(())
}

/// With an edge header configured, the gated routes refuse unusable
/// values too, and a usable one flows through Auth's own builder.
#[tokio::test]
async fn a_configured_ip_header_gates_send_and_verify() -> Result<()> {
	let harness = SiteHarness::new().await;
	let options = site::SiteOptions::new(harness.day.clone(), Some("x-forwarded-for".to_owned()));
	let built = harness.build(&options, ADMIN_ORIGIN).await;
	let mut ada = Browser::default();

	mint(&mut ada, &built.router, "ada").await;
	let view = inbox(&mut ada, &built.router).await;
	let address = view["people"][0]["email"]
		.as_str()
		.expect("email")
		.to_owned();

	// Send without the header: the edge always sets it, so its absence
	// means no IP — the send itself is allowed, as locally it must be.
	let sent = {
		let mut browser = Browser::default();
		browser
			.jars
			.entry(SITE_ORIGIN.to_owned())
			.or_default()
			.extend(
				ada.pass(SITE_ORIGIN)
					.into_iter()
					.map(|value| ("demo-pass".to_owned(), value)),
			);
		let request = Request::post("/auth/email/send")
			.header(header::HOST, SITE_HOST)
			.header(header::ORIGIN, SITE_ORIGIN)
			.header(header::CONTENT_TYPE, "application/json");
		browser
			.send(
				&built.router,
				SITE_ORIGIN,
				request,
				Body::from(json!({ "email": address }).to_string()),
			)
			.await
	};
	assert_eq!(
		sent.status(),
		StatusCode::NO_CONTENT,
		"a header-less local request has no IP, and no IP is no refusal"
	);

	// A garbage header value is refused for send and verify alike.
	for path in ["/auth/email/send", "/auth/email/verify"] {
		let refused = {
			let mut browser = Browser::default();
			browser
				.jars
				.entry(SITE_ORIGIN.to_owned())
				.or_default()
				.extend(
					ada.pass(SITE_ORIGIN)
						.into_iter()
						.map(|value| ("demo-pass".to_owned(), value)),
				);
			let request = Request::post(path)
				.header(header::HOST, SITE_HOST)
				.header(header::ORIGIN, SITE_ORIGIN)
				.header(header::CONTENT_TYPE, "application/json")
				.header("x-forwarded-for", "garbage");
			browser
				.send(
					&built.router,
					SITE_ORIGIN,
					request,
					Body::from(json!({ "email": address }).to_string()),
				)
				.await
		};
		assert_eq!(
			refused.status(),
			StatusCode::BAD_REQUEST,
			"{path} refuses a non-IP header value"
		);
	}
	Ok(())
}

// ─── The admin: real OIDC, roles, and no secret surface ───────────────

/// Ada reaches the admin's overview through the real OIDC flow; Grace is
/// refused there and on its analytics and infra routes; the registered
/// client carries the `roles` scope; nothing the visitors did emitted an
/// analytics event; and no representative public response carries a
/// generation key, the analytics token, the control key, or the client
/// secret. Neither dev route exists.
#[tokio::test]
async fn the_oidc_flow_admits_ada_refuses_grace_and_leaks_nothing() -> Result<()> {
	let root = volume();
	let day = demo_service::clock::day_of(demo_service::clock::now_millis());
	let instance = a_generation(&root, &day).await?;

	// The registered client asks for `roles`: discovery offers it, and
	// Ada's sign-in later proves it delivered.
	let discovery = Browser::default()
		.get(
			&instance.site,
			SITE_ORIGIN,
			"/.well-known/openid-configuration",
		)
		.await;
	let discovery = json_body(discovery).await;
	assert!(
		discovery["scopes_supported"]
			.as_array()
			.expect("scopes")
			.iter()
			.any(|scope| scope == "roles"),
		"the roles scope is offered: {discovery}"
	);

	let events_before = site::count_events(&instance.loopback, &instance.secrets.analytics_token)
		.await
		.expect("the seeded count reads back");

	// Ada: mint, sign in to the site, then through the admin's real
	// flow to an overview.
	let mut ada = Browser::default();
	mint(&mut ada, &instance.site, "ada").await;
	sign_in_unclaimed(&mut ada, &instance.site).await;
	let finished = admin_flow(&mut ada, &instance.site, &instance.admin, ADMIN_ORIGIN).await;
	assert!(
		!finished.contains("auth_error"),
		"Ada's sign-in is not refused: {finished}"
	);

	let overview = ada
		.get(&instance.admin, ADMIN_ORIGIN, "/_leptoskit/admin/")
		.await;

	// The overview is a page, or a redirect deeper into the admin — the
	// one home the admin's own project overview gives. Never the
	// sign-in card.
	match overview.status() {
		StatusCode::OK => {}
		StatusCode::SEE_OTHER => {
			let target = overview
				.headers()
				.get(header::LOCATION)
				.expect("a redirect target")
				.to_str()
				.expect("a header");
			assert!(
				target.starts_with(leptoskit_admin::BASE) && !target.ends_with("/sign-in"),
				"Ada lands inside the admin: {target}"
			);
		}
		other => panic!("Ada sees the admin, not {other}"),
	}

	// Grace: minted, signed in to the site, and turned away by the
	// admin's gate — on the page and on its APIs.
	let mut grace = Browser::default();
	mint(&mut grace, &instance.site, "grace").await;
	sign_in_unclaimed(&mut grace, &instance.site).await;
	let finished = admin_flow(&mut grace, &instance.site, &instance.admin, ADMIN_ORIGIN).await;
	assert!(
		finished.contains("auth_error=not_allowed"),
		"Grace's refusal names the gate: {finished}"
	);
	let refused_page = grace
		.get(&instance.admin, ADMIN_ORIGIN, "/_leptoskit/admin/")
		.await;
	assert_eq!(
		refused_page
			.headers()
			.get(header::LOCATION)
			.map(|value| value.to_str().expect("h").to_owned())
			.as_deref(),
		Some(&*format!("{}/sign-in", leptoskit_admin::BASE)),
		"Grace is sent back to the admin's sign-in card"
	);
	// The analytics live API is a GET; the person-export API is a GET
	// too. Both are `api_gate`d, and refuse her without an admin session.
	let live = grace
		.get(
			&instance.admin,
			ADMIN_ORIGIN,
			"/_leptoskit/admin/api/analytics/live",
		)
		.await;
	assert!(
		matches!(
			live.status(),
			StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
		),
		"the analytics API refuses Grace: {}",
		live.status()
	);
	let export = grace
		.get(
			&instance.admin,
			ADMIN_ORIGIN,
			"/_leptoskit/admin/api/analytics/people/00000000-0000-0000-0000-000000000000/export",
		)
		.await;
	assert!(
		matches!(
			export.status(),
			StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
		),
		"the infra-facing export API refuses Grace: {}",
		export.status()
	);

	// Nothing the visitors did emitted an analytics event: the demo's
	// synthetic addresses never reach shared analytics, because this
	// task emits nothing at all.
	let events_after = site::count_events(&instance.loopback, &instance.secrets.analytics_token)
		.await
		.expect("the count reads back");
	assert_eq!(
		events_before, events_after,
		"no analytics was emitted by any visitor action"
	);

	// No dev routes exist on the site host.
	let dev = Browser::default()
		.post_json(&instance.site, SITE_ORIGIN, "/auth/dev/sign-in", json!({}))
		.await;
	assert_eq!(dev.status(), StatusCode::NOT_FOUND, "no dev sign-in");
	let mailbox = Browser::default()
		.get(&instance.site, SITE_ORIGIN, "/_leptoskit/mail")
		.await;
	assert_eq!(mailbox.status(), StatusCode::NOT_FOUND, "no dev mailbox");

	// No representative public response carries a secret.
	let secrets = [
		hex::encode(instance.secrets.site_key),
		hex::encode(instance.secrets.admin_key),
		instance.secrets.analytics_token.clone(),
		instance.secrets.control_key.clone(),
		instance.secrets.idp_client_secret.clone(),
	];

	let mut scanner = Browser::default();
	mint(&mut scanner, &instance.site, "ada").await;
	let home = scanner.get(&instance.site, SITE_ORIGIN, "/").await;
	let sign_in = scanner.get(&instance.site, SITE_ORIGIN, "/sign-in").await;
	let notes = scanner.get(&instance.site, SITE_ORIGIN, "/notes").await;
	let no_pass_inbox = scanner
		.get(&instance.site, SITE_ORIGIN, "/demo/inbox")
		.await;
	let bodies = [
		text_body(home).await,
		text_body(sign_in).await,
		text_body(notes).await,
		text_body(no_pass_inbox).await,
		finished,
	];
	for body in &bodies {
		for secret in &secrets {
			assert!(
				!body.contains(secret.as_str()),
				"a public response carries a secret"
			);
		}
	}
	Ok(())
}

/// A next day clears the old pass, and the old site and admin sessions
/// refuse against the new day's keys.
#[tokio::test]
async fn a_new_day_clears_the_pass_and_refuses_old_sessions() -> Result<()> {
	let root = volume();
	let day = demo_service::clock::day_of(demo_service::clock::now_millis());
	let first_day = a_generation(&root, &day).await?;

	// Ada signs in to the site, and through the real OIDC flow to the
	// admin, on day one.
	let mut ada = Browser::default();
	mint(&mut ada, &first_day.site, "ada").await;
	sign_in_unclaimed(&mut ada, &first_day.site).await;
	let finished = admin_flow(&mut ada, &first_day.site, &first_day.admin, ADMIN_ORIGIN).await;
	assert!(
		!finished.contains("auth_error"),
		"day one's admin sign-in works: {finished}"
	);
	let day_one = ada
		.get(&first_day.admin, ADMIN_ORIGIN, "/_leptoskit/admin/")
		.await;

	// Day one's overview is a page, or a redirect deeper into the admin.
	match day_one.status() {
		StatusCode::OK => {}
		StatusCode::SEE_OTHER => {
			let target = day_one
				.headers()
				.get(header::LOCATION)
				.expect("a redirect target")
				.to_str()
				.expect("a header");
			assert!(
				target.starts_with(leptoskit_admin::BASE) && !target.ends_with("/sign-in"),
				"day one's admin lands inside itself: {target}"
			);
		}
		other => panic!("day one's overview answers, not {other}"),
	}

	// Day two: a new generation, new keys.
	let tomorrow = support::tomorrow();
	let next_day = a_generation(&root, &tomorrow).await?;
	assert_ne!(first_day.day, next_day.day);

	// The old pass names an earlier day: cleared, and named as a new day.
	let inbox = ada.get(&next_day.site, SITE_ORIGIN, "/demo/inbox").await;
	assert_eq!(inbox.status(), StatusCode::UNAUTHORIZED);
	let view = json_body(inbox).await;
	assert_eq!(view["pass"], "new_day", "{view}");
	assert_eq!(view["error"], "no_pass");
	assert_eq!(
		view["day"].as_str(),
		Some(next_day.day.as_str()),
		"today is named"
	);
	assert!(
		ada.pass(SITE_ORIGIN).is_none(),
		"the old pass cookie is gone from the jar"
	);

	// The old site session refuses against the new day's keys: the
	// session route answers, but there is no session in it.
	let session = ada.get(&next_day.site, SITE_ORIGIN, "/auth/session").await;
	assert_eq!(session.status(), StatusCode::OK);
	assert!(
		json_body(session).await["session"].is_null(),
		"the old site session did not survive the day"
	);

	// The old admin session refuses too: no session, and the overview
	// sends the browser to the sign-in card.
	let admin = ada
		.get(&next_day.admin, ADMIN_ORIGIN, "/auth/session")
		.await;
	assert_eq!(admin.status(), StatusCode::OK);
	assert!(
		json_body(admin).await["session"].is_null(),
		"the admin session did not survive the day"
	);
	let page = ada
		.get(&next_day.admin, ADMIN_ORIGIN, "/_leptoskit/admin/")
		.await;
	assert_eq!(
		page.headers()
			.get(header::LOCATION)
			.map(|value| value.to_str().expect("h").to_owned()),
		Some(format!("{}/sign-in", leptoskit_admin::BASE)),
		"a signed-out admin sends the browser to its sign-in card"
	);

	// Dropping the generations ends this test's shared lifetime locks.
	drop(first_day);
	drop(next_day);
	Ok(())
}
