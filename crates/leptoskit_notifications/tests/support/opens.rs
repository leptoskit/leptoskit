//! Checks of open tracking, a send's report, and a send's label, which
//! both databases run against a service with no push configured and no
//! workers: a recipients send is whole the moment it is made, and nothing
//! moves unless the check moves it.
//!
//! The rows are read back with plain SQL, so a check says what the
//! database holds and not what the service would answer about it.

use std::time::Duration;

use leptoskit_db::Db;
use leptoskit_db::Read as _;
use leptoskit_db::Statement;
use leptoskit_notifications::SERVICE;
use leptoskit_notifications::service::config::Retention;
use leptoskit_notifications::service::retention;
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

fn person(n: u128) -> Uuid {
	Uuid::from_u128(0x09E7_0000_0000_0000_0000_0000_0000_0000 | n)
}

/// Serves the notifications routes over `db`, already migrated, and
/// answers where.
pub async fn serve(db: &Db) -> std::net::SocketAddr {
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

	address
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

/// Calls `operation` as `caller`.
async fn call(
	address: std::net::SocketAddr,
	operation: &str,
	args: Value,
	caller: As,
) -> (u16, String) {
	let request = reqwest::Client::new()
		.post(format!("http://{address}{PREFIX}/rpc/{operation}"))
		.json(&args)
		.timeout(Duration::from_secs(60));
	let request = match caller {
		As::App => request.bearer_auth(TOKEN),
		As::Person(person) => request.bearer_auth(signed_in(person)),
		As::Nobody => request,
	};
	let response = request.send().await.unwrap();

	(response.status().as_u16(), response.text().await.unwrap())
}

/// Calls `operation` and answers its JSON, which must be a success.
async fn ok(address: std::net::SocketAddr, operation: &str, args: Value, caller: As) -> Value {
	let (status, body) = call(address, operation, args, caller).await;
	assert_eq!(status, 200, "{operation}: {body}");
	serde_json::from_str(&body).unwrap()
}

/// Sends `spec` to `people` as `key`.
async fn send(address: std::net::SocketAddr, key: &str, spec: Value, people: &[Uuid]) -> Value {
	ok(
		address,
		"send",
		json!({
			"idempotency_key": key,
			"spec": spec,
			"target": { "recipients": people },
		}),
		As::App,
	)
	.await
}

fn news() -> Value {
	json!({ "category": "news", "title": "Read me" })
}

/// The id of the one row `key` wrote for `who`.
async fn row_id(db: &Db, key: &str, who: Uuid) -> Uuid {
	let rows = db
		.fetch(Statement::new(
			format!(
				"SELECT id FROM notifications WHERE send_key = '{key}' AND person = {}",
				Sql(db.dialect()).uuid(who)
			),
			Vec::new(),
		))
		.await
		.expect("a read");

	match rows.first().and_then(|row| row.first()) {
		Some(leptoskit_db::Value::Uuid(id)) => *id,
		Some(leptoskit_db::Value::Bytes(bytes)) => Uuid::from_slice(bytes).expect("an id"),
		other => panic!("no row of {key} for {who}: {other:?}"),
	}
}

/// A row's `opened_at` and `read_at`, as the database holds them.
async fn opened_and_read(db: &Db, id: Uuid) -> (leptoskit_db::Value, leptoskit_db::Value) {
	let mut rows = db
		.fetch(Statement::new(
			format!(
				"SELECT opened_at, read_at FROM notifications WHERE id = {}",
				Sql(db.dialect()).uuid(id)
			),
			Vec::new(),
		))
		.await
		.expect("a read");
	let mut row = rows.remove(0).into_iter();

	(row.next().unwrap(), row.next().unwrap())
}

/// The send's report.
async fn report(address: std::net::SocketAddr, key: &str) -> Value {
	ok(
		address,
		"send_report",
		json!({ "idempotency_key": key }),
		As::App,
	)
	.await
}

/// The first open is recorded and marks the row read in the same write;
/// opening again changes nothing and still succeeds. A row read before it
/// was opened keeps the time it was read.
#[expect(
	clippy::cognitive_complexity,
	reason = "two rows followed through each order of reading and opening"
)]
pub async fn the_first_open_is_recorded_and_marks_the_row_read(
	address: std::net::SocketAddr,
	db: &Db,
) {
	use leptoskit_db::Value::Null;

	let (alice, bob) = (person(1), person(2));
	send(address, "open-1", news(), &[alice, bob]).await;
	let hers = row_id(db, "open-1", alice).await;
	let his = row_id(db, "open-1", bob).await;

	assert_eq!(
		opened_and_read(db, hers).await,
		(Null, Null),
		"a new row is unopened and unread"
	);

	let answer = ok(
		address,
		"mark_opened",
		json!({ "notification": hers }),
		As::Person(alice),
	)
	.await;
	assert_eq!(answer, Value::Null);

	let (opened, read) = opened_and_read(db, hers).await;
	assert_ne!(opened, Null, "the open is recorded");
	assert_eq!(read, opened, "opening marks it read, at the same moment");

	// The inbox says the same on every device.
	let inbox = ok(
		address,
		"inbox",
		json!({ "page": { "limit": 10 } }),
		As::Person(alice),
	)
	.await;
	assert!(inbox["items"][0]["read_at"].is_string(), "{inbox}");
	let unread = ok(address, "unread_count", json!({}), As::Person(alice)).await;
	assert_eq!(unread, json!(0));

	// Opening it again, later: the first open stands.
	tokio::time::sleep(Duration::from_millis(20)).await;
	ok(
		address,
		"mark_opened",
		json!({ "notification": hers }),
		As::Person(alice),
	)
	.await;
	assert_eq!(
		opened_and_read(db, hers).await,
		(opened.clone(), opened),
		"a second open changes nothing"
	);

	// Bob read his in the inbox first, and opens the push afterwards: the
	// time he read it stays.
	ok(
		address,
		"mark_read",
		json!({ "notification": his }),
		As::Person(bob),
	)
	.await;
	let (unopened, read_first) = opened_and_read(db, his).await;
	assert_eq!(unopened, Null, "reading is not opening");
	assert_ne!(read_first, Null);

	tokio::time::sleep(Duration::from_millis(20)).await;
	ok(
		address,
		"mark_opened",
		json!({ "notification": his }),
		As::Person(bob),
	)
	.await;
	let (opened_later, read_after) = opened_and_read(db, his).await;
	assert_ne!(opened_later, Null);
	assert_eq!(read_after, read_first, "the earlier read time is kept");
	assert_ne!(opened_later, read_after, "the open has its own time");
}

/// A person opens only their own rows: another person's id, an unknown id,
/// and a caller who is not a signed-in person are answered exactly as
/// `mark_read` answers them, and nothing is recorded.
pub async fn only_the_owner_opens_a_row(address: std::net::SocketAddr, db: &Db) {
	let (alice, bob) = (person(11), person(12));
	send(address, "open-2", news(), &[alice]).await;
	let hers = row_id(db, "open-2", alice).await;

	for (who, id) in [
		(As::Person(bob), hers),
		(As::Person(alice), Uuid::now_v7()),
		(As::App, hers),
		(As::Nobody, hers),
	] {
		let opened = call(address, "mark_opened", json!({ "notification": id }), who).await;
		// A fresh row for the read, so the comparison never depends on what
		// an earlier iteration left.
		let read = call(address, "mark_read", json!({ "notification": id }), who).await;

		assert_eq!(opened, read, "the same answer as mark_read");
		assert!(
			matches!(opened.0, 403 | 404),
			"refused, not an error: {opened:?}"
		);
	}

	let (opened, read) = opened_and_read(db, hers).await;
	assert_eq!(
		(opened, read),
		(leptoskit_db::Value::Null, leptoskit_db::Value::Null),
		"nobody else's call recorded anything"
	);
}

/// A silent data message has no open: its row is not in anyone's inbox, so
/// opening it is answered like an id that is not there, and no open is
/// stored or counted.
pub async fn a_data_message_has_no_open(address: std::net::SocketAddr, db: &Db) {
	let alice = person(21);
	send(
		address,
		"open-3",
		json!({ "category": "sync", "title": "Sync", "options": { "kind": "data" } }),
		&[alice],
	)
	.await;
	let row = row_id(db, "open-3", alice).await;

	let (status, body) = call(
		address,
		"mark_opened",
		json!({ "notification": row }),
		As::Person(alice),
	)
	.await;
	assert_eq!(status, 404, "{body}");
	assert_eq!(
		opened_and_read(db, row).await,
		(leptoskit_db::Value::Null, leptoskit_db::Value::Null)
	);

	let report = report(address, "open-3").await;
	assert_eq!(
		report["inbox"],
		json!({ "rows": 1, "read": 0, "opened": 0, "dismissed": 0, "cancelled": 0 }),
		"{report}"
	);
}

/// A send's report counts its own rows: how many it wrote, and how many of
/// them were read, opened, dismissed, and cancelled — beside the progress
/// `send_status` answers. Only the app reads it.
#[expect(
	clippy::cognitive_complexity,
	reason = "one send followed from fresh, through what people did, to its report"
)]
pub async fn a_report_counts_what_became_of_a_sends_rows(address: std::net::SocketAddr, db: &Db) {
	let people: Vec<Uuid> = (31..=36).map(person).collect();

	// The sixth opted out, so the send skips them.
	ok(
		address,
		"set_category_preference",
		json!({ "category": "news", "opted_out": true }),
		As::Person(people[5]),
	)
	.await;
	send(address, "report-1", news(), &people).await;
	// Another send to the same people, which the report must not count.
	send(address, "report-other", news(), &people).await;

	let fresh = report(address, "report-1").await;
	assert_eq!(
		fresh["inbox"],
		json!({ "rows": 5, "read": 0, "opened": 0, "dismissed": 0, "cancelled": 0 }),
		"{fresh}"
	);

	let mut ids = Vec::new();
	for who in &people[..5] {
		ids.push(row_id(db, "report-1", *who).await);
	}

	// Two open theirs (one of them twice), one reads theirs in the inbox,
	// one dismisses theirs, one does nothing.
	for (operation, index) in [
		("mark_opened", 0),
		("mark_opened", 0),
		("mark_opened", 1),
		("mark_read", 2),
		("mark_dismissed", 3),
	] {
		ok(
			address,
			operation,
			json!({ "notification": ids[index] }),
			As::Person(people[index]),
		)
		.await;
	}

	// And everything of the other send is opened.
	for who in &people[..5] {
		let id = row_id(db, "report-other", *who).await;
		ok(
			address,
			"mark_opened",
			json!({ "notification": id }),
			As::Person(*who),
		)
		.await;
	}

	let report = report(address, "report-1").await;
	assert_eq!(
		report["inbox"],
		// Opening and dismissing both mark a row read.
		json!({ "rows": 5, "read": 4, "opened": 2, "dismissed": 1, "cancelled": 0 }),
		"{report}"
	);
	assert_eq!(
		report["progress"],
		ok(
			address,
			"send_status",
			json!({ "idempotency_key": "report-1" }),
			As::App
		)
		.await,
		"the report carries what send_status answers"
	);
	assert_eq!(
		report["progress"],
		json!({ "state": "complete", "targeted": 6, "delivered": 5, "skipped_opted_out": 1 })
	);
	assert!(report["created_at"].is_string(), "{report}");
	assert!(report.get("label").is_none(), "no label was set: {report}");

	// A scheduled send cancelled before its time: every row is cancelled.
	let later = (chrono::Utc::now() + chrono::Duration::hours(2)).to_rfc3339();
	send(
		address,
		"report-later",
		json!({ "category": "news", "title": "Later", "visible_at": later }),
		&people[..3],
	)
	.await;
	ok(
		address,
		"cancel",
		json!({ "idempotency_key": "report-later" }),
		As::App,
	)
	.await;
	let cancelled = self::report(address, "report-later").await;
	assert_eq!(
		cancelled["inbox"],
		json!({ "rows": 3, "read": 0, "opened": 0, "dismissed": 0, "cancelled": 3 }),
		"{cancelled}"
	);

	// Only the app reads a report, and only of a send that exists.
	for (who, key, expected) in [
		(As::Person(people[0]), "report-1", 403),
		(As::Nobody, "report-1", 403),
		(As::App, "never-sent", 404),
		(As::App, "", 400),
	] {
		let (status, body) = call(
			address,
			"send_report",
			json!({ "idempotency_key": key }),
			who,
		)
		.await;
		assert_eq!(status, expected, "{key}: {body}");
	}
}

/// The inbox counts are of the rows the service still stores: once
/// retention has pruned a send's rows they count nothing, while the
/// progress — kept on the send's own record — still says what the send
/// did. Once the record itself is pruned the report is gone with it.
pub async fn a_reports_counts_shrink_when_rows_are_pruned(address: std::net::SocketAddr, db: &Db) {
	let people: Vec<Uuid> = (41..=43).map(person).collect();
	send(address, "pruned-1", news(), &people).await;
	let id = row_id(db, "pruned-1", people[0]).await;
	ok(
		address,
		"mark_opened",
		json!({ "notification": id }),
		As::Person(people[0]),
	)
	.await;

	let before = report(address, "pruned-1").await;
	assert_eq!(before["inbox"]["rows"], 3);
	assert_eq!(before["inbox"]["opened"], 1);

	// A sweep as of two months from now, keeping an inbox for a month.
	let retention = Retention {
		inbox_days: Some(30),
		..Retention::default()
	};
	let pruned = retention::prune(
		db,
		&retention,
		leptoskit_db::now() + chrono::Duration::days(60),
	)
	.await
	.expect("the sweep runs");
	assert_eq!(pruned.notifications, 3, "{pruned:?}");
	assert_eq!(pruned.sends, 0, "the record outlives its rows: {pruned:?}");

	let after = report(address, "pruned-1").await;
	assert_eq!(
		after["inbox"],
		json!({ "rows": 0, "read": 0, "opened": 0, "dismissed": 0, "cancelled": 0 }),
		"{after}"
	);
	assert_eq!(
		after["progress"], before["progress"],
		"what the send did is still known"
	);
	assert_eq!(after["progress"]["delivered"], 3);

	// Past the record's own retention, the send is unknown again.
	retention::prune(
		db,
		&retention,
		leptoskit_db::now() + chrono::Duration::days(120),
	)
	.await
	.expect("the sweep runs");
	let (status, body) = call(
		address,
		"send_report",
		json!({ "idempotency_key": "pruned-1" }),
		As::App,
	)
	.await;
	assert_eq!(status, 404, "{body}");
}

/// A send may carry a short label. It is checked, stored with the send —
/// a recipients send and a fanning-out one alike — answered in the report,
/// and part of what the idempotency key stands for.
pub async fn a_label_is_checked_stored_and_part_of_the_intent(
	address: std::net::SocketAddr,
	_db: &Db,
) {
	let alice = person(51);
	let labelled = |label: &str| json!({ "category": "news", "title": "Sale", "label": label });

	// A send without a label is, byte for byte, the send it always was.
	let unlabelled = serde_json::to_value(leptoskit_notifications::SendSpec::default()).unwrap();
	assert!(unlabelled.get("label").is_none(), "{unlabelled}");

	for bad in ["", "two words", "né", &"x".repeat(65), "semi;colon"] {
		let (status, body) = call(
			address,
			"send",
			json!({
				"idempotency_key": "label-bad",
				"spec": labelled(bad),
				"target": { "recipients": [alice] },
			}),
			As::App,
		)
		.await;
		assert_eq!(status, 400, "{bad:?}: {body}");
		assert!(body.contains("label"), "{bad:?}: {body}");
	}

	let first = send(address, "label-1", labelled("spring-sale_2026.a"), &[alice]).await;
	assert_eq!(first["duplicate"], false);
	assert_eq!(
		report(address, "label-1").await["label"],
		"spring-sale_2026.a"
	);

	// The same key and label is the same send; another label is not, and
	// neither is no label.
	let again = send(address, "label-1", labelled("spring-sale_2026.a"), &[alice]).await;
	assert_eq!(again["duplicate"], true);

	for other in [labelled("summer-sale"), news_titled("Sale")] {
		let (status, body) = call(
			address,
			"send",
			json!({
				"idempotency_key": "label-1",
				"spec": other,
				"target": { "recipients": [alice] },
			}),
			As::App,
		)
		.await;
		assert_eq!(status, 409, "{body}");
	}

	// A topic send keeps its label too, before its fanout has run a step.
	ok(
		address,
		"send",
		json!({
			"idempotency_key": "label-topic",
			"spec": labelled("digest"),
			"target": { "topic": "weekly" },
		}),
		As::App,
	)
	.await;
	let topic = report(address, "label-topic").await;
	assert_eq!(topic["label"], "digest");
	assert_eq!(topic["progress"]["state"], "fanning_out");
	assert_eq!(topic["inbox"]["rows"], 0);
}

fn news_titled(title: &str) -> Value {
	json!({ "category": "news", "title": title })
}

/// Rows and sends written before the migration are unopened and unlabelled
/// after it, and are counted like any others. `before` is every migration
/// up to the one that adds open tracking, and `all` is every migration.
pub async fn rows_from_before_the_migration_are_unopened(
	db: &Db,
	before: &[leptoskit_db::Migration],
	all: &[leptoskit_db::Migration],
) {
	let sql = Sql(db.dialect());
	let alice = person(61);
	let id = Uuid::now_v7();
	let now = sql.timestamp(&chrono::Utc::now().to_rfc3339());

	db.migrate(before).await.expect("the earlier migrations");

	for statement in [
		format!(
			"INSERT INTO sends (idempotency_key, intent, delivered, skipped_opted_out, targeted, \
			 state, created_at) VALUES ('old-1', '{}', 1, 0, 1, 'complete', {now})",
			"0".repeat(64)
		),
		format!(
			"INSERT INTO notifications (id, send_key, person, category, title, body, data, \
			 visible_at, read_at, created_at, kind) VALUES ({}, 'old-1', {}, 'news', 'Old', '', \
			 '{{}}', {now}, {now}, {now}, 'notification')",
			sql.uuid(id),
			sql.uuid(alice)
		),
	] {
		db.fetch(Statement::new(statement.clone(), Vec::new()))
			.await
			.unwrap_or_else(|error| panic!("{error}\n{statement}"));
	}

	let applied = db.migrate(all).await.expect("the rest apply over rows");
	assert_eq!(applied.len(), all.len() - before.len(), "{applied:?}");

	let address = serve(db).await;
	let old = report(address, "old-1").await;
	assert_eq!(
		old["inbox"],
		json!({ "rows": 1, "read": 1, "opened": 0, "dismissed": 0, "cancelled": 0 }),
		"{old}"
	);
	assert!(old.get("label").is_none(), "{old}");

	ok(
		address,
		"mark_opened",
		json!({ "notification": id }),
		As::Person(alice),
	)
	.await;
	assert_eq!(report(address, "old-1").await["inbox"]["opened"], 1);
}
