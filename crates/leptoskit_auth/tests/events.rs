//! The signed-in person's security events, against both databases: newest
//! first, a page at a time, only their own, and only with a fresh session,
//! through `Auth::security_events` and `GET /auth/events`.

#![cfg(all(feature = "axum", feature = "postgres", feature = "sqlite"))]

#[path = "../../leptoskit_db/tests/support/postgres.rs"]
mod postgres;
#[path = "support/totp.rs"]
mod totp;

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use axum::http::header;
use chrono::TimeDelta;
use http_body_util::BodyExt;
use leptoskit_auth::Account;
use leptoskit_auth::Auth;
use leptoskit_auth::AuthError;
use leptoskit_auth::ErrorResponse;
use leptoskit_auth::Event;
use leptoskit_auth::EventKind;
use leptoskit_auth::Keys;
use leptoskit_auth::Mailbox;
use leptoskit_auth::RequestInfo;
use leptoskit_auth::SecurityEvents;
use leptoskit_auth::SignIn;
use leptoskit_auth::SignedIn;
use leptoskit_db::Connection;
use leptoskit_db::Db;
use leptoskit_db::Dialect;
use leptoskit_db::Id;
use leptoskit_db::Statement;
use leptoskit_db::Value;
use postgres::PostgresCluster;
use totp::totp;
use tower::ServiceExt;

const ORIGIN: &str = "https://notes.example";
const FIREFOX: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 14.5) Firefox/131.0";

/// Both databases, each with every table, including the pack's.
async fn databases(cluster: Option<&PostgresCluster>) -> Vec<Db> {
	let mut databases = vec![Db::connect("sqlite::memory:").await.unwrap()];

	if let Some(cluster) = cluster {
		databases.push(Db::connect(&cluster.database()).await.unwrap());
	}

	for db in &databases {
		leptoskit_db::testing::create_tables(db).await.unwrap();
	}

	databases
}

fn request() -> RequestInfo {
	RequestInfo {
		ip: Some("203.0.113.9".parse().unwrap()),
		user_agent: Some(FIREFOX.into()),
		..RequestInfo::default()
	}
}

fn auth(db: &Db, mailbox: &Mailbox) -> Auth {
	Auth::builder(db, Keys::single("test", [9; 32]))
		.app_name("Notes")
		.origin(ORIGIN)
		.mailer(mailbox.clone())
		.build()
		.unwrap()
}

async fn sign_in(auth: &Auth, mailbox: &Mailbox, address: &str) -> SignIn {
	auth.send_email_code(address, &request()).await.unwrap();
	let mail = mailbox.latest_to(address).unwrap();
	let code = mail.subject.rsplit(' ').next().unwrap().to_owned();
	auth.verify_email_code(address, &code, &request())
		.await
		.unwrap()
		.signed_in()
		.expect("no second factor")
}

/// Every event of `account`, newest first, as the database holds them.
async fn trail(db: &Db, account: Id<Account>) -> Vec<(EventKind, leptoskit_db::Timestamp)> {
	Event::query()
		.filter(|event| event.account_id.eq(account))
		.order_by(|event| event.created_at.desc())
		.order_by(|event| event.id.desc())
		.all(&db.system("test: read events"))
		.await
		.unwrap()
		.into_iter()
		.map(|event| (event.kind, event.created_at))
		.collect()
}

/// Moves the session's sign-in and second factor `minutes` into the past,
/// as time passing would. The pack's writers are private, so this goes
/// around them.
async fn age(db: &Db, me: &SignedIn, minutes: i64) -> SignedIn {
	let then = leptoskit_db::now() - TimeDelta::minutes(minutes);
	let system = db.system("test: age a session");
	let (table, first, second) = match system.dialect() {
		Dialect::Postgres => ("auth.sessions", "$1", "$2"),
		Dialect::Sqlite => ("auth_sessions", "?1", "?2"),
	};
	for (column, only) in [
		("authenticated_at", ""),
		("mfa_at", " AND mfa_at IS NOT NULL"),
	] {
		system
			.execute(Statement {
				sql: format!("UPDATE {table} SET {column} = {first} WHERE id = {second}{only}"),
				args: vec![Value::Timestamp(then), Value::Uuid(me.session.uuid())],
			})
			.await
			.unwrap();
	}
	SignedIn {
		authenticated_at: then,
		mfa_at: me.mfa_at.map(|_| then),
		..me.clone()
	}
}

/// Every page, following each `next`.
async fn every_page(auth: &Auth, me: &SignedIn, limit: u32) -> Vec<SecurityEvents> {
	let mut pages = vec![auth.security_events(me, None, Some(limit)).await.unwrap()];
	while let Some(next) = pages.last().and_then(|page| page.next.clone()) {
		pages.push(
			auth.security_events(me, Some(&next), Some(limit))
				.await
				.unwrap(),
		);
	}
	pages
}

#[tokio::test]
async fn people_page_through_their_own_events_newest_first() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let mailbox = Mailbox::new();
		let auth = auth(&db, &mailbox);
		let mut ada = sign_in(&auth, &mailbox, "ada@example.com").await;
		for _ in 0..3 {
			ada = sign_in(&auth, &mailbox, "ada@example.com").await;
		}
		let bob = sign_in(&auth, &mailbox, "bob@example.com").await;
		let ada = ada.signed_in();
		let bob = bob.signed_in();

		// Three at a time, every event of Ada's, in the database's order.
		let expected = trail(&db, ada.account).await;
		assert!(expected.len() > 6, "{expected:?}");
		let pages = every_page(&auth, &ada, 3).await;
		assert_eq!(pages.len(), expected.len().div_ceil(3));
		assert!(
			pages[..pages.len() - 1]
				.iter()
				.all(|page| page.events.len() == 3)
		);
		let seen: Vec<_> = pages
			.iter()
			.flat_map(|page| &page.events)
			.map(|event| (event.kind, event.at))
			.collect();
		assert_eq!(seen, expected);
		assert!(seen.windows(2).all(|pair| pair[0].1 >= pair[1].1));

		// Each says where it came from.
		let newest = &pages[0].events[0];
		assert_eq!(newest.kind, EventKind::SignedIn);
		assert_eq!(newest.device.as_deref(), Some("Firefox on macOS"));
		assert_eq!(newest.ip.as_deref(), Some("203.0.113.9"));

		// Bob sees only his own, even with a cursor from Ada's pages.
		let bobs = trail(&db, bob.account).await;
		let theirs = every_page(&auth, &bob, 100).await;
		assert_eq!(theirs.len(), 1);
		assert_eq!(
			theirs[0]
				.events
				.iter()
				.map(|event| (event.kind, event.at))
				.collect::<Vec<_>>(),
			bobs
		);
		let cursor = pages[0].next.clone().unwrap();
		let borrowed = auth
			.security_events(&bob, Some(&cursor), None)
			.await
			.unwrap();
		assert!(
			borrowed
				.events
				.iter()
				.all(|event| bobs.contains(&(event.kind, event.at)))
		);

		// The default page is 50, and no page is bigger than 100.
		assert_eq!(
			auth.security_events(&ada, None, None)
				.await
				.unwrap()
				.events
				.len(),
			expected.len().min(50)
		);
		assert!(
			auth.security_events(&ada, None, Some(1_000))
				.await
				.unwrap()
				.next
				.is_none()
		);

		// A cursor it didn't write is refused.
		assert!(matches!(
			auth.security_events(&ada, Some("not a cursor"), None).await,
			Err(AuthError::NotFound)
		));
	}
}

#[tokio::test]
async fn the_trail_needs_a_fresh_session() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let mailbox = Mailbox::new();
		let auth = auth(&db, &mailbox);
		let ada = sign_in(&auth, &mailbox, "ada@example.com")
			.await
			.signed_in();

		// Signed in long ago, and no second factor: sign in again.
		let stale = age(&db, &ada, 20).await;
		assert!(matches!(
			auth.security_events(&stale, None, None).await,
			Err(AuthError::Reauthenticate)
		));

		// With an authenticator, step up.
		let grace = sign_in(&auth, &mailbox, "grace@example.com")
			.await
			.signed_in();
		let setup = auth.begin_totp(&grace).await.unwrap();
		let confirmed = auth
			.confirm_totp(&grace, &totp(&setup.secret, 0), &request())
			.await
			.unwrap();
		let grace = confirmed.sign_in.signed_in();
		assert!(auth.security_events(&grace, None, None).await.is_ok());
		let stale = age(&db, &grace, 20).await;
		assert!(matches!(
			auth.security_events(&stale, None, None).await,
			Err(AuthError::StepUpRequired)
		));
	}
}

#[tokio::test]
async fn the_route_lists_the_signed_in_persons_events() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let mailbox = Mailbox::new();
		let auth = auth(&db, &mailbox);
		let router: Router = auth.router().layer(auth.layer());
		let send = async |path: &str, cookie: Option<&str>| {
			let mut builder = Request::get(path).header(header::USER_AGENT, FIREFOX);
			if let Some(cookie) = cookie {
				builder = builder.header(header::COOKIE, cookie);
			}
			router
				.clone()
				.oneshot(builder.body(Body::empty()).unwrap())
				.await
				.unwrap()
		};
		let body = async |response: axum::response::Response| {
			response.into_body().collect().await.unwrap().to_bytes()
		};

		let signed_in = sign_in(&auth, &mailbox, "ada@example.com").await;
		let cookie = format!("__Host-session={}", signed_in.token.expose());
		let ada = signed_in.signed_in();

		let response = send("/auth/events?limit=1", Some(&cookie)).await;
		assert_eq!(response.status(), StatusCode::OK);
		let page: SecurityEvents = serde_json::from_slice(&body(response).await).unwrap();
		assert_eq!(page.events.len(), 1);
		let next = page.next.unwrap();
		let response = send(&format!("/auth/events?before={next}"), Some(&cookie)).await;
		let older: SecurityEvents = serde_json::from_slice(&body(response).await).unwrap();
		assert_eq!(1 + older.events.len(), trail(&db, ada.account).await.len());
		assert!(
			older
				.events
				.iter()
				.all(|event| event.at <= page.events[0].at)
		);

		// Signed out, or not fresh enough, it says so.
		let response = send("/auth/events", None).await;
		assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
		age(&db, &ada, 20).await;
		let response = send("/auth/events", Some(&cookie)).await;
		assert_eq!(response.status(), StatusCode::FORBIDDEN);
		let error: ErrorResponse = serde_json::from_slice(&body(response).await).unwrap();
		assert_eq!(error.error, "reauthenticate");
	}
}
