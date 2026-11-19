//! What keeps the service's cost in step with what is live, on both
//! engines: the plans of the reads that grew with the tables, and the
//! pruning that keeps the tables from growing forever.
//!
//! **Plans.** Each check seeds a few thousand rows, asks the database how
//! it would run a statement (`EXPLAIN QUERY PLAN` on SQLite, `EXPLAIN` on
//! Postgres), and looks for the index the statement is meant to use. The
//! audience walk's statements are the service's own, taken from
//! `service::plans`. The others are written out here in the shape the
//! typed query builder gives them — the same conditions, order, and limit
//! — because the database crate renders those privately; a change to one
//! of those queries needs its mirror here changed with it.
//!
//! **Pruning.** Rows are seeded with plain SQL at chosen ages, and
//! `service::retention::prune` runs a whole sweep as of now, or as of a
//! day in the future instead of waiting for one.
//!
//! The Postgres halves start a private Postgres from local binaries and
//! skip without them, unless `LEPTOSKIT_REQUIRE_POSTGRES` is set, as
//! `tests/postgres.rs` does.

#![cfg(not(target_arch = "wasm32"))]

use chrono::DateTime;
use chrono::Duration;
use chrono::Utc;
use leptoskit_db::Db;
use leptoskit_db::Dialect;
use leptoskit_db::Read as _;
use leptoskit_db::Statement;
use leptoskit_db::Value;
use leptoskit_notifications::SERVICE;
use leptoskit_notifications::SendTarget;
use leptoskit_notifications::service::config::Retention;
use leptoskit_notifications::service::retention;
use leptoskit_notifications::service::retention::Pruned;
use leptoskit_rpc as rpc;
use serde_json::json;
use uuid::Uuid;

#[path = "support/sql.rs"]
mod sql;
use sql::Sql;

#[cfg(feature = "postgres")]
#[path = "support/postgres.rs"]
mod postgres;

const TOKEN: &str = "app-token";
const PREFIX: &str = "/_leptoskit/services/notifications";

async fn on_sqlite() -> Db {
	leptoskit_db::testing::sqlite_migrated(leptoskit_notifications::MIGRATIONS).await
}

async fn run(db: &Db, sql: impl Into<String>) -> Vec<Vec<Value>> {
	let sql = sql.into();

	db.fetch(Statement {
		sql: sql.clone(),
		args: Vec::new(),
	})
	.await
	.unwrap_or_else(|error| panic!("{error}\n{sql}"))
}

async fn count(db: &Db, sql: &str) -> i64 {
	match run(db, sql).await.first().and_then(|row| row.first()) {
		Some(Value::Int(count)) => *count,
		other => panic!("expected a count from {sql}, found {other:?}"),
	}
}

/// The text column of every row of `sql`, sorted.
async fn texts(db: &Db, sql: &str) -> Vec<String> {
	let mut texts: Vec<String> = run(db, sql)
		.await
		.into_iter()
		.map(|row| {
			match row.into_iter().next() {
				Some(Value::Text(text)) => text,
				other => panic!("expected text from {sql}, found {other:?}"),
			}
		})
		.collect();
	texts.sort();
	texts
}

/// Inserts `rows`, each a parenthesised list of values, a few hundred to
/// a statement.
async fn insert(db: &Db, table: &str, columns: &str, rows: &[String]) {
	for chunk in rows.chunks(250) {
		run(
			db,
			format!(
				"INSERT INTO {table} ({columns}) VALUES {}",
				chunk.join(", ")
			),
		)
		.await;
	}
}

/// A UUID v7-shaped id minted at `at`, the `n`th of its moment.
fn id_at(at: DateTime<Utc>, n: u128) -> Uuid {
	Uuid::from_u128(
		(u128::try_from(at.timestamp_millis()).expect("after 1970") << 80)
			| (0x7 << 76)
			| (0x8 << 60)
			| n,
	)
}

/// A person of this file's own.
fn person(n: u128) -> Uuid {
	Uuid::from_u128(0x5CA1_E000_0000_0000_0000_0000_0000_0000 | n)
}

/// How the dialect spells the `n`th placeholder.
fn placeholder(dialect: Dialect, n: usize) -> String {
	match dialect {
		Dialect::Sqlite => format!("?{n}"),
		Dialect::Postgres => format!("${n}"),
	}
}

/// `sql` with each `{}` replaced by the dialect's next placeholder.
fn numbered(dialect: Dialect, sql: &str) -> String {
	let mut out = String::new();

	for (index, part) in sql.split("{}").enumerate() {
		if index > 0 {
			out.push_str(&placeholder(dialect, index));
		}

		out.push_str(part);
	}

	out
}

/// How the database says it would run `statement`.
async fn explain(db: &Db, statement: &Statement) -> String {
	let prefix = match db.dialect() {
		Dialect::Sqlite => "EXPLAIN QUERY PLAN ",
		Dialect::Postgres => "EXPLAIN (COSTS OFF) ",
	};
	let rows = db
		.fetch(Statement {
			sql: format!("{prefix}{}", statement.sql),
			args: statement.args.clone(),
		})
		.await
		.unwrap_or_else(|error| panic!("{error}\n{}", statement.sql));

	rows.iter()
		.map(|row| {
			row.iter()
				.filter_map(|value| {
					match value {
						Value::Text(text) => Some(text.as_str()),
						_ => None,
					}
				})
				.collect::<Vec<_>>()
				.join(" ")
		})
		.collect::<Vec<_>>()
		.join("\n")
}

/// Panics unless `plan` reads through `index` and never sorts or
/// de-duplicates what it read.
fn assert_reads_in_index_order(what: &str, plan: &str, index: &str) {
	assert!(
		plan.contains(index),
		"{what} should read through {index}:\n{plan}"
	);

	for step in ["TEMP B-TREE", "Sort", "Unique", "HashAggregate"] {
		assert!(
			!plan.contains(step),
			"{what} should come out of {index} in order, with no `{step}` step:\n{plan}"
		);
	}
}

// ── Plans ─────────────────────────────────────────────────────────────

/// What the plan checks read: one send of 3,000 deliveries, all but a
/// hundred of them finished, among 300 small sends; one person with 2,000
/// notifications, most of them read, among 400 people; two overlapping topics
/// of 1,500 subscribers among 100 small ones.
struct Corpus {
	/// When the corpus's rows were minted.
	base: DateTime<Utc>,
}

impl Corpus {
	const BIG: &'static str = "big";

	/// The endpoint the big send's `n`th delivery went to.
	fn endpoint(n: u128) -> Uuid {
		Uuid::from_u128(0xE0D0_0000_0000_0000_0000_0000_0000_0000 | (n % 600))
	}

	/// The id of the big send's `n`th delivery.
	fn delivery(&self, n: u128) -> Uuid {
		id_at(
			self.base + Duration::milliseconds(i64::try_from(n).unwrap()),
			2 * n,
		)
	}

	async fn seed(db: &Db) -> Self {
		let sql = Sql(db.dialect());
		let base = Utc::now() - Duration::hours(2);
		let corpus = Self { base };
		let stamp = sql.timestamp(&base.to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
		let mut deliveries = Vec::new();

		for n in 0..3_000_u128 {
			// The big send's deliveries and the small sends' interleave in
			// id order, as sends that overlap in time do.
			// One in twenty of the later ones is still queued, so a page of
			// them is spread thin among finished rows.
			let (state, finished) = if n >= 1_000 && n % 20 == 0 {
				("enqueued", "NULL")
			} else {
				("accepted", stamp.as_str())
			};
			deliveries.push(format!(
				"({}, {}, '{}', {}, 1, {}, 'fcm', '{state}', 1, {finished}, {stamp})",
				sql.uuid(corpus.delivery(n)),
				sql.uuid(id_at(base, 10_000 + n)),
				Self::BIG,
				sql.uuid(Self::endpoint(n)),
				sql.uuid(person(n % 600)),
			));
			deliveries.push(format!(
				"({}, {}, 'small-{}', {}, 1, {}, 'apns', 'accepted', 1, {stamp}, {stamp})",
				sql.uuid(id_at(
					base + Duration::milliseconds(i64::try_from(n).unwrap()),
					2 * n + 1
				)),
				sql.uuid(id_at(base, 20_000 + n)),
				n % 300,
				sql.uuid(Self::endpoint(n + 7)),
				sql.uuid(person(n % 600)),
			));
		}

		insert(
			db,
			"push_deliveries",
			"id, notification, send_key, endpoint, generation, person, platform, state, attempts, \
			 finished_at, created_at",
			&deliveries,
		)
		.await;

		let mut notifications = Vec::new();

		for n in 0..6_000_u128 {
			// Person 0 holds a third of the rows, and has read all but a
			// hundred of theirs.
			let (owner, read) = if n < 2_000 {
				(person(0), if n % 20 == 0 { "NULL" } else { stamp.as_str() })
			} else {
				(person(1 + n % 400), "NULL")
			};
			notifications.push(format!(
				"({}, 'seen-{n}', {}, 'news', 't', '', '{{}}', {stamp}, {read}, {stamp})",
				sql.uuid(id_at(base, 30_000 + n)),
				sql.uuid(owner),
			));
		}

		insert(
			db,
			"notifications",
			"id, send_key, person, category, title, body, data, visible_at, read_at, created_at",
			&notifications,
		)
		.await;

		let mut subscriptions = Vec::new();
		let mut subscribe = |topic: String, who: u128| {
			subscriptions.push(format!(
				"({}, {}, '{topic}', {stamp})",
				sql.new_id(),
				sql.uuid(person(who))
			));
		};

		for n in 0..1_500_u128 {
			subscribe("alpha".to_owned(), n);
			subscribe("beta".to_owned(), 1_000 + n);
		}

		for n in 0..2_000_u128 {
			subscribe(format!("small-{}", n % 100), 5_000 + n);
		}

		insert(
			db,
			"topic_subscriptions",
			"id, person, topic, created_at",
			&subscriptions,
		)
		.await;
		run(db, "ANALYZE").await;

		corpus
	}
}

/// The statements whose plans matter, in the shape the service issues
/// them, each with the index it is meant to read through.
fn mirrored(dialect: Dialect, corpus: &Corpus) -> Vec<(&'static str, Statement, &'static str)> {
	let now = Value::Timestamp(Utc::now());
	let text = |text: &str| Value::Text(text.to_owned());
	let endpoint = Value::Uuid(Corpus::endpoint(5));
	let cursor = Value::Uuid(corpus.delivery(1_000));

	vec![
		(
			"a rotation's or a revoke's update of one endpoint's queued deliveries",
			Statement::new(
				numbered(
					dialect,
					"UPDATE \"push_deliveries\" SET \"generation\" = {} WHERE \"endpoint\" = {} \
					 AND \"state\" IN ({}, {})",
				),
				vec![
					Value::Int(2),
					endpoint.clone(),
					text("enqueued"),
					text("retrying"),
				],
			),
			"push_deliveries_endpoint_state_idx",
		),
		(
			"an invalidation's update of the endpoint's other queued deliveries",
			Statement::new(
				numbered(
					dialect,
					"UPDATE \"push_deliveries\" SET \"state\" = {} WHERE \"id\" <> {} AND \
					 \"endpoint\" = {} AND \"generation\" = {} AND \"state\" IN ({}, {})",
				),
				vec![
					text("skipped"),
					cursor.clone(),
					endpoint,
					Value::Int(1),
					text("enqueued"),
					text("retrying"),
				],
			),
			"push_deliveries_endpoint_state_idx",
		),
		(
			"a round's page of due deliveries in one state, after its cursor",
			Statement::new(
				numbered(
					dialect,
					"SELECT * FROM \"push_deliveries\" WHERE \"send_key\" = {} AND \"state\" = {} \
					 AND (\"next_attempt_at\" IS NULL OR \"next_attempt_at\" <= {}) AND \"id\" > \
					 {} ORDER BY \"id\" ASC LIMIT {}",
				),
				vec![
					text(Corpus::BIG),
					text("enqueued"),
					now.clone(),
					cursor,
					Value::Int(32),
				],
			),
			"push_deliveries_send_key_state_id_idx",
		),
		(
			"a round's first page of due deliveries in one state",
			Statement::new(
				numbered(
					dialect,
					"SELECT * FROM \"push_deliveries\" WHERE \"send_key\" = {} AND \"state\" = {} \
					 AND (\"next_attempt_at\" IS NULL OR \"next_attempt_at\" <= {}) ORDER BY \
					 \"id\" ASC LIMIT {}",
				),
				vec![
					text(Corpus::BIG),
					text("retrying"),
					now.clone(),
					Value::Int(32),
				],
			),
			"push_deliveries_send_key_state_id_idx",
		),
		(
			"the unread count",
			Statement::new(
				numbered(
					dialect,
					"SELECT count(*) FROM \"notifications\" WHERE \"person\" = {} AND \"kind\" = \
					 {} AND \"visible_at\" <= {} AND \"cancelled_at\" IS NULL AND (\"expires_at\" \
					 IS NULL OR \"expires_at\" > {}) AND \"read_at\" IS NULL AND \"dismissed_at\" \
					 IS NULL",
				),
				vec![
					Value::Uuid(person(0)),
					text("notification"),
					now.clone(),
					now,
				],
			),
			"notifications_person_kind_dismissed_at_read_at_idx",
		),
	]
}

/// Every mirrored statement reads through its index, and on SQLite the
/// reads that page in id order come out of theirs already ordered.
async fn the_reads_that_grew_use_their_indexes(db: &Db) {
	let corpus = Corpus::seed(db).await;

	for (what, statement, index) in mirrored(db.dialect(), &corpus) {
		let plan = explain(db, &statement).await;

		match db.dialect() {
			Dialect::Sqlite => assert_reads_in_index_order(what, &plan, index),
			// Postgres weighs costs: with only a hundred matching rows it
			// may fetch them all through the index and sort them, which is
			// as good. What matters is that it finds them by the index
			// rather than by walking the table or the primary key.
			Dialect::Postgres => {
				assert!(
					plan.contains(index),
					"{what} should read through {index}:\n{plan}"
				);
			}
		}
	}

	// The mirror of the unread count counts what the corpus says: the
	// hundred rows person 0 has not read.
	let (_, unread, _) = mirrored(db.dialect(), &corpus)
		.pop()
		.expect("the unread count is last");
	assert_eq!(
		db.fetch(unread).await.expect("a count")[0][0],
		Value::Int(100)
	);
}

/// An import reads two things for each of its entries, and each comes
/// from an index however many endpoints the service holds: who holds a
/// token, through the unique fingerprint, and how many live endpoints a
/// person has, through the person index. 6,000 endpoints of 600 people
/// stand in for the table.
async fn an_imports_reads_use_their_indexes(db: &Db) {
	let sql = Sql(db.dialect());
	let base = Utc::now() - Duration::days(1);
	let stamp = sql.timestamp(&base.to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
	let endpoints = (0..6_000_u128)
		.map(|n| {
			format!(
				"({}, {}, 'fcm', 'sealed', 1, '{n:064}', NULL, {stamp}, {stamp})",
				sql.uuid(id_at(base, 300_000 + n)),
				sql.uuid(person(n % 600)),
			)
		})
		.collect::<Vec<_>>();
	insert(
		db,
		"push_endpoints",
		"id, person, platform, credential, generation, fingerprint, invalidated_at, created_at, \
		 imported_at",
		&endpoints,
	)
	.await;
	run(db, "ANALYZE").await;

	for (what, statement, index, counted) in [
		(
			"who holds an imported token",
			Statement::new(
				numbered(
					db.dialect(),
					"SELECT count(*) FROM \"push_endpoints\" WHERE \"fingerprint\" = {}",
				),
				vec![Value::Text(format!("{:064}", 4_321))],
			),
			"push_endpoints_fingerprint_key",
			1,
		),
		(
			"how many live endpoints a person has",
			Statement::new(
				numbered(
					db.dialect(),
					"SELECT count(*) FROM \"push_endpoints\" WHERE \"person\" = {} AND \
					 \"invalidated_at\" IS NULL",
				),
				vec![Value::Uuid(person(321))],
			),
			"push_endpoints_person_idx",
			10,
		),
	] {
		let plan = explain(db, &statement).await;
		assert!(
			plan.contains(index),
			"{what} should read through {index}:\n{plan}"
		);
		assert_eq!(
			db.fetch(statement).await.expect("a count")[0][0],
			Value::Int(counted),
			"{what}"
		);
	}
}

/// A send's report counts the send's own rows, found through the
/// `(send_key, person)` unique index, and reads nobody else's: one send of
/// 300 rows among 1,170 small ones in a table of 12,000. The statement is
/// the service's own, from `service::plans`, and it counts what was
/// seeded.
async fn a_send_report_counts_only_the_sends_own_rows(db: &Db) {
	let sql = Sql(db.dialect());
	let base = Utc::now() - Duration::hours(2);
	let stamp = sql.timestamp(&base.to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
	let set = |holds: bool| if holds { stamp.as_str() } else { "NULL" };
	let mut notifications = Vec::new();

	for n in 0..12_000_u128 {
		// The campaign's rows arrive among everyone else's, as a fanout's
		// do: every fortieth row is one of its 300.
		let (key, who) = if n % 40 == 0 {
			("campaign".to_owned(), person(n / 40))
		} else {
			(format!("small-{}", n % 1_170), person(1_000 + n))
		};
		// Of the campaign's rows: half read, a third opened, a tenth
		// dismissed, none cancelled. Every other row is read and opened,
		// so a count that strayed outside the send would show.
		let mine = n % 40 == 0;
		let index = n / 40;
		notifications.push(format!(
			"({}, '{key}', {}, 'news', 't', '', '{{}}', {stamp}, {}, {}, {}, {stamp})",
			sql.uuid(id_at(base, 50_000 + n)),
			sql.uuid(who),
			set(!mine || index % 2 == 0),
			set(!mine || index % 3 == 0),
			set(mine && index % 10 == 0),
		));
	}

	insert(
		db,
		"notifications",
		"id, send_key, person, category, title, body, data, visible_at, read_at, opened_at, \
		 dismissed_at, created_at",
		&notifications,
	)
	.await;
	run(db, "ANALYZE").await;

	let statement = leptoskit_notifications::service::plans::send_counts(db.dialect(), "campaign");
	let plan = explain(db, &statement).await;

	assert!(
		plan.contains("notifications_send_key_person_key"),
		"a report should find its send's rows through the (send_key, person) index:\n{plan}"
	);

	for step in ["SCAN notifications", "Seq Scan"] {
		assert!(
			!plan.contains(step),
			"a report should not walk the table (`{step}`):\n{plan}"
		);
	}

	assert_eq!(
		db.fetch(statement).await.expect("the counts")[0],
		[300, 150, 100, 30, 0].map(Value::Int),
		"rows, read, opened, dismissed, cancelled"
	);
}

/// What a read answers: its rows.
type Rows = Vec<Vec<Value>>;

/// The reads the composer added, each through its index and each counting
/// or listing what was seeded: the page of the send list and a category's
/// latest send, a report's push breakdown and its reads by the hour, the
/// walks of distinct topics, attribute keys, attribute values, and
/// categories — one index seek per name, whatever the tables hold — and
/// the capped counts behind them. The catalogue's statements are the
/// service's own, from `service::plans`.
#[expect(
	clippy::too_many_lines,
	clippy::cognitive_complexity,
	reason = "one corpus, and every read the composer added against it"
)]
async fn the_composers_reads_use_their_indexes(db: &Db) {
	use leptoskit_notifications::push::PushPlatform;
	use leptoskit_notifications::service::plans;
	use leptoskit_notifications::service::plans::Names;

	let dialect = db.dialect();
	let sql = Sql(dialect);
	let corpus = Corpus::seed(db).await;
	let stamp = sql.timestamp(
		&corpus
			.base
			.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
	);
	let yes = match dialect {
		Dialect::Sqlite => "1",
		Dialect::Postgres => "true",
	};
	let no = match dialect {
		Dialect::Sqlite => "0",
		Dialect::Postgres => "false",
	};

	// 2,000 sends in the history across 40 categories, among 1,000 from
	// before it.
	let mut sends = Vec::new();

	for n in 0..3_000_u128 {
		let (history, category) = if n % 3 == 0 {
			("NULL".to_owned(), "NULL".to_owned())
		} else {
			(
				sql.uuid(id_at(
					corpus.base + Duration::milliseconds(i64::try_from(n).unwrap()),
					60_000 + n,
				)),
				format!("'cat-{:02}'", n % 40),
			)
		};
		sends.push(format!(
			"('sent-{n}', '{}', 1, 0, 1, 'complete', {history}, {category}, {stamp})",
			"0".repeat(64)
		));
	}

	insert(
		db,
		"sends",
		"idempotency_key, intent, delivered, skipped_opted_out, targeted, state, history_id, \
		 category, created_at",
		&sends,
	)
	.await;

	// 9,000 attributes: everyone has one of three plans, and one of 300
	// cities.
	let mut attributes = Vec::new();

	for n in 0..4_500_u128 {
		for (key, value) in [
			(
				"plan",
				["free", "pro", "team"][usize::try_from(n % 3).unwrap()].to_owned(),
			),
			("city", format!("city-{:03}", n % 300)),
		] {
			attributes.push(format!(
				"({}, {}, '{key}', '{value}', {stamp}, {stamp})",
				sql.new_id(),
				sql.uuid(person(n)),
			));
		}
	}

	insert(
		db,
		"person_attributes",
		"id, person, key, value, created_at, updated_at",
		&attributes,
	)
	.await;

	// 6,000 preferences over 60 categories, twenty of which no send has
	// used; a third of them opt-outs.
	let mut preferences = Vec::new();

	for n in 0..6_000_u128 {
		preferences.push(format!(
			"({}, {}, 'cat-{:02}', {}, {stamp})",
			sql.new_id(),
			sql.uuid(person(n / 60)),
			n % 60,
			if (n / 60) % 3 == 0 { yes } else { no },
		));
	}

	insert(
		db,
		"category_preferences",
		"id, person, category, opted_out, created_at",
		&preferences,
	)
	.await;

	// 6,000 endpoints: 2,000 people with two APNs devices each, 500 of
	// those people's devices gone, and 2,000 FCM devices.
	let mut endpoints = Vec::new();

	for n in 0..6_000_u128 {
		let (platform, who) = if n < 4_000 {
			("apns", n / 2)
		} else {
			("fcm", n)
		};
		endpoints.push(format!(
			"({}, {}, '{platform}', 'sealed', 1, '{n:064}', {}, {stamp})",
			sql.uuid(id_at(corpus.base, 70_000 + n)),
			sql.uuid(person(who)),
			if n < 1_000 { stamp.as_str() } else { "NULL" },
		));
	}

	insert(
		db,
		"push_endpoints",
		"id, person, platform, credential, generation, fingerprint, invalidated_at, created_at",
		&endpoints,
	)
	.await;

	// A hundred of the big send's notifications, read across three hours.
	let mut read = Vec::new();

	for n in 0..300_u128 {
		let at = corpus.base + Duration::minutes(i64::try_from(n % 3).unwrap() * 60 + 5);
		read.push(format!(
			"({}, '{}', {}, 'news', 't', '', '{{}}', {stamp}, {}, {stamp})",
			sql.uuid(id_at(corpus.base, 80_000 + n)),
			Corpus::BIG,
			sql.uuid(person(9_000 + n)),
			if n < 100 {
				sql.timestamp(&at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
			} else {
				"NULL".to_owned()
			},
		));
	}

	insert(
		db,
		"notifications",
		"id, send_key, person, category, title, body, data, visible_at, read_at, created_at",
		&read,
	)
	.await;
	run(db, "ANALYZE").await;

	let text = |text: &str| Value::Text(text.to_owned());
	let cases: Vec<(&str, Statement, &str, Option<Rows>)> = vec![
		(
			"a page of the send list",
			Statement::new(
				numbered(
					dialect,
					"SELECT idempotency_key FROM sends WHERE history_id IS NOT NULL ORDER BY \
					 history_id DESC LIMIT {}",
				),
				vec![Value::Int(51)],
			),
			"sends_history_id_idx",
			None,
		),
		(
			"a category's latest send",
			Statement::new(
				numbered(
					dialect,
					"SELECT idempotency_key FROM sends WHERE category = {} AND history_id IS NOT \
					 NULL ORDER BY history_id DESC LIMIT 1",
				),
				vec![text("cat-07")],
			),
			"sends_category_history_id_idx",
			Some(vec![vec![text("sent-2927")]]),
		),
		(
			// One of the small sends: the big one is half the table, which
			// Postgres rightly reads whole.
			"a report's push breakdown",
			plans::push_counts(dialect, "small-7"),
			"push_deliveries_send_key_",
			Some(vec![vec![
				text("apns"),
				text("accepted"),
				Value::Null,
				Value::Int(10),
			]]),
		),
		(
			"a report's reads by the hour",
			plans::reads_by_hour(dialect, Corpus::BIG),
			"notifications_send_key_person_key",
			None,
		),
		(
			"the walk of topics",
			plans::names(dialect, Names::Topics, Some("alpha"), None, 3),
			"topic_subscriptions_topic_person_idx",
			Some(vec![
				vec![text("beta")],
				vec![text("small-0")],
				vec![text("small-1")],
			]),
		),
		(
			"the walk of attribute keys",
			plans::names(dialect, Names::AttributeKeys, None, None, 51),
			"person_attributes_key_value_idx",
			Some(vec![vec![text("city")], vec![text("plan")]]),
		),
		(
			"the walk of one key's values",
			plans::names(
				dialect,
				Names::AttributeValues("city"),
				Some("city-297"),
				None,
				51,
			),
			"person_attributes_key_value_idx",
			Some(vec![vec![text("city-298")], vec![text("city-299")]]),
		),
		(
			"the walk of categories",
			plans::names(dialect, Names::Categories, Some("cat-37"), None, 4),
			"category_preferences_category_opted_out_idx",
			Some(vec![
				vec![text("cat-38")],
				vec![text("cat-39")],
				vec![text("cat-40")],
				vec![text("cat-41")],
			]),
		),
		(
			"the walk of categories, on the sends' side",
			plans::names(dialect, Names::Categories, None, None, 2),
			"sends_category_history_id_idx",
			Some(vec![vec![text("cat-00")], vec![text("cat-01")]]),
		),
		(
			"a topic's subscriber count",
			plans::topic_count(dialect, "small-7"),
			"topic_subscriptions_topic_person_idx",
			Some(vec![vec![Value::Int(20)]]),
		),
		(
			"a large topic's subscriber count, of half the table",
			plans::topic_count(dialect, "alpha"),
			"topic_subscriptions_topic_person_idx",
			Some(vec![vec![Value::Int(1_500)]]),
		),
		(
			"the count of people holding a key, of half the table",
			plans::attribute_count(dialect, "plan", None),
			"person_attributes_key_value_idx",
			Some(vec![vec![Value::Int(4_500)]]),
		),
		(
			"the count of people holding a value",
			plans::attribute_count(dialect, "city", Some("city-007")),
			"person_attributes_key_value_idx",
			Some(vec![vec![Value::Int(15)]]),
		),
		(
			"a category's opt-out count",
			plans::opted_out_count(dialect, "cat-05"),
			"category_preferences_category_opted_out_idx",
			None,
		),
		(
			"the people with a live endpoint on a platform, of half the table",
			plans::platform_people(dialect, PushPlatform::Apns),
			"push_endpoints_platform_invalidated_at_person_idx",
			Some(vec![vec![Value::Int(1_500)]]),
		),
	];

	for (what, statement, index, answer) in cases {
		let plan = explain(db, &statement).await;

		// A count of half a table is the one place Postgres may rightly
		// read the table instead: the count stops at its cap either way,
		// which the plan's `Limit` shows.
		let bounded = dialect == Dialect::Postgres && what.ends_with(", of half the table");

		if bounded {
			assert!(
				plan.contains("Limit"),
				"{what} should stop at its cap:\n{plan}"
			);
		} else {
			assert!(
				plan.contains(index),
				"{what} should read through {index}:\n{plan}"
			);
		}

		// None of the others walks a whole table to find its rows: on SQLite a
		// bare `SCAN t` is a walk of the table, and one `USING … INDEX`
		// is a read of the index.
		for line in plan.lines() {
			assert!(
				!(line.contains("SCAN")
					&& !line.contains("INDEX")
					&& !line.contains("walk")
					&& !line.contains("capped")
					&& !line.contains("nexts")
					&& !line.contains("SUBQUERY")
					&& !line.contains("CONSTANT"))
					&& (bounded || !line.contains("Seq Scan")),
				"{what} should not walk a table:\n{plan}"
			);
		}

		if let Some(answer) = answer {
			assert_eq!(
				db.fetch(statement).await.expect("the read runs"),
				answer,
				"{what}"
			);
		}
	}

	// The breakdown and the hours count what was seeded.
	let mut pushes = db
		.fetch(plans::push_counts(dialect, Corpus::BIG))
		.await
		.expect("the breakdown");
	pushes.sort_by_key(|row| format!("{row:?}"));
	assert_eq!(
		pushes,
		vec![
			vec![
				text("fcm"),
				text("accepted"),
				Value::Null,
				Value::Int(2_900)
			],
			vec![text("fcm"), text("enqueued"), Value::Null, Value::Int(100)],
		]
	);
	let hours = db
		.fetch(plans::reads_by_hour(dialect, Corpus::BIG))
		.await
		.expect("the hours");
	assert_eq!(hours.len(), 3, "{hours:?}");
	assert!(
		hours
			.iter()
			.all(|row| [Value::Int(33), Value::Int(34)].contains(&row[1])),
		"{hours:?}"
	);

	// A third of cat-05's hundred preferences are opt-outs.
	let opted = db
		.fetch(plans::opted_out_count(dialect, "cat-05"))
		.await
		.expect("the count");
	assert_eq!(opted, vec![vec![Value::Int(34)]]);
}

/// The campaign lists and counts read through their indexes: a page of
/// campaigns newest first, a page of one state, a campaign's events in
/// order, the count of drafts behind the cap, and the count of one
/// campaign's tests. 3,000 campaigns, a tenth of them drafts, each with
/// two events. The typed queries are mirrored here as SQL.
async fn the_campaign_lists_use_their_indexes(db: &Db) {
	let dialect = db.dialect();
	let sql = Sql(dialect);
	let base = Utc::now() - Duration::hours(2);
	let stamp = sql.timestamp(&base.to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
	let campaign = |n: u128| {
		id_at(
			base + Duration::milliseconds(i64::try_from(n).unwrap()),
			90_000 + n,
		)
	};
	let mut campaigns = Vec::new();
	let mut events = Vec::new();

	for n in 0..3_000_u128 {
		let state = match n % 10 {
			0 => "draft",
			1 => "archived",
			_ => "launched",
		};
		campaigns.push(format!(
			"({}, 1, '{state}', 'campaign {n}', '{{}}', 'seed', 'seed', {stamp}, {stamp})",
			sql.uuid(campaign(n)),
		));

		for (m, kind) in ["created", "tested"].into_iter().enumerate() {
			events.push(format!(
				"({}, {}, '{kind}', 'seed', 1, {stamp})",
				sql.uuid(id_at(
					base + Duration::milliseconds(i64::try_from(n).unwrap()),
					100_000 + 2 * n + u128::try_from(m).unwrap()
				)),
				sql.uuid(campaign(n)),
			));
		}
	}

	insert(
		db,
		"campaigns",
		"id, revision, state, name, spec, created_by, updated_by, created_at, updated_at",
		&campaigns,
	)
	.await;
	insert(
		db,
		"campaign_events",
		"id, campaign, kind, by, revision, created_at",
		&events,
	)
	.await;
	run(db, "ANALYZE").await;

	let text = |text: &str| Value::Text(text.to_owned());
	let one = Value::Uuid(campaign(1_234));
	let primary = match dialect {
		Dialect::Sqlite => "sqlite_autoindex_campaigns_1",
		Dialect::Postgres => "campaigns_pkey",
	};
	let cases = [
		(
			"a page of campaigns",
			Statement::new(
				numbered(
					dialect,
					"SELECT id FROM campaigns ORDER BY id DESC LIMIT {}",
				),
				vec![Value::Int(51)],
			),
			primary,
			51,
		),
		(
			"a page of the drafts",
			Statement::new(
				numbered(
					dialect,
					"SELECT id FROM campaigns WHERE state = {} ORDER BY id DESC LIMIT {}",
				),
				vec![text("draft"), Value::Int(51)],
			),
			"campaigns_state_id_idx",
			51,
		),
		(
			"the count of drafts",
			Statement::new(
				numbered(dialect, "SELECT count(*) FROM campaigns WHERE state = {}"),
				vec![text("draft")],
			),
			"campaigns_state_id_idx",
			1,
		),
		(
			"a campaign's events",
			Statement::new(
				numbered(
					dialect,
					"SELECT id FROM campaign_events WHERE campaign = {} ORDER BY id ASC LIMIT {}",
				),
				vec![one.clone(), Value::Int(101)],
			),
			"campaign_events_campaign_id_idx",
			2,
		),
		(
			"the count of a campaign's tests",
			Statement::new(
				numbered(
					dialect,
					"SELECT count(*) FROM campaign_events WHERE campaign = {} AND kind = {}",
				),
				vec![one, text("tested")],
			),
			"campaign_events_campaign_id_idx",
			1,
		),
	];

	for (what, statement, index, rows) in cases {
		let plan = explain(db, &statement).await;

		assert!(
			plan.contains(index),
			"{what} should read through {index}:\n{plan}"
		);

		for step in ["TEMP B-TREE", "Seq Scan", "Sort"] {
			assert!(
				!plan.contains(step),
				"{what} should come out of {index} with no `{step}` step:\n{plan}"
			);
		}

		assert_eq!(
			db.fetch(statement).await.expect("the read runs").len(),
			rows,
			"{what}"
		);
	}

	assert_eq!(
		count(db, "SELECT count(*) FROM campaigns WHERE state = 'draft'").await,
		300
	);
}

/// The recurrence reads go through their indexes: the job's read of what
/// is due, earliest first; a page of recurrences newest first; a page of
/// one state; and the count of the ones that have not ended, behind the
/// cap. 3,000 recurrences as a service that has run for years holds
/// them: a hundred active, each with a time to be due at, sixty paused,
/// and the rest ended. The typed queries are mirrored here as SQL.
/// On Postgres the reads of the active and the paused are found through
/// their index and sorted afterwards; see the loop below.
async fn the_recurrence_reads_use_their_indexes(db: &Db) {
	let dialect = db.dialect();
	let sql = Sql(dialect);
	let base = Utc::now() - Duration::hours(2);
	let at = |n: u128| base + Duration::seconds(i64::try_from(n).unwrap());
	let stamp =
		|at: DateTime<Utc>| sql.timestamp(&at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
	let mut recurrences = Vec::new();

	for n in 0..3_000_u128 {
		let (state, next_on, due_at) = if n % 30 == 0 {
			("active", "'2027-01-01'".to_owned(), stamp(at(n)))
		} else if n % 50 == 1 {
			("paused", "NULL".to_owned(), "NULL".to_owned())
		} else {
			("ended", "NULL".to_owned(), "NULL".to_owned())
		};
		recurrences.push(format!(
			"({}, '{state}', 'recurrence {n}', '{{}}', '{{}}', '{{}}', {next_on}, {due_at}, 0, 0, \
			 'seed', 'seed', {created}, {created})",
			sql.uuid(id_at(at(n), 120_000 + n)),
			created = stamp(base),
		));
	}

	insert(
		db,
		"recurrences",
		"id, state, name, spec, target, rule, next_on, due_at, sent, skipped, created_by, \
		 updated_by, created_at, updated_at",
		&recurrences,
	)
	.await;
	run(db, "ANALYZE").await;

	let text = |text: &str| Value::Text(text.to_owned());
	let primary = match dialect {
		Dialect::Sqlite => "sqlite_autoindex_recurrences_1",
		Dialect::Postgres => "recurrences_pkey",
	};
	// Half of the active ones are due.
	let now = Value::Timestamp(at(1_495));
	let cases = [
		(
			"the recurrences that are due",
			Statement::new(
				numbered(
					dialect,
					"SELECT id FROM recurrences WHERE state = {} AND due_at <= {} ORDER BY due_at \
					 ASC LIMIT {}",
				),
				vec![text("active"), now, Value::Int(200)],
			),
			"recurrences_state_due_at_idx",
			50,
			true,
		),
		(
			"a page of recurrences",
			Statement::new(
				numbered(
					dialect,
					"SELECT id FROM recurrences ORDER BY id DESC LIMIT {}",
				),
				vec![Value::Int(51)],
			),
			primary,
			51,
			false,
		),
		(
			"a page of the paused ones",
			Statement::new(
				numbered(
					dialect,
					"SELECT id FROM recurrences WHERE state = {} ORDER BY id DESC LIMIT {}",
				),
				vec![text("paused"), Value::Int(51)],
			),
			"recurrences_state_id_idx",
			51,
			true,
		),
		(
			"the count of the ones that have not ended",
			Statement::new(
				numbered(
					dialect,
					"SELECT count(*) FROM recurrences WHERE state IN ({}, {})",
				),
				vec![text("active"), text("paused")],
			),
			"recurrences_state_",
			1,
			false,
		),
	];

	for (what, statement, index, rows, few) in cases {
		let plan = explain(db, &statement).await;

		assert!(
			plan.contains(index),
			"{what} should read through {index}:\n{plan}"
		);

		for step in ["TEMP B-TREE", "Seq Scan", "Sort"] {
			// The active and the paused are at most a hundred together,
			// and for a set that small Postgres finds the rows through the
			// index's condition and sorts them itself rather than walk the
			// index in order. What it sorts is bounded by that cap, so the
			// sort is let be; SQLite reads them in order.
			if step == "Sort" && few && dialect == Dialect::Postgres {
				continue;
			}

			assert!(
				!plan.contains(step),
				"{what} should come out of {index} with no `{step}` step:\n{plan}"
			);
		}

		assert_eq!(
			db.fetch(statement).await.expect("the read runs").len(),
			rows,
			"{what}"
		);
	}

	assert_eq!(
		count(
			db,
			"SELECT count(*) FROM recurrences WHERE state IN ('active', 'paused')"
		)
		.await,
		160
	);
}

/// A walk over two topics reads each topic's subscribers on its own, in
/// person order straight from `(topic, person)`: one statement per topic,
/// none of them sorting or de-duplicating.
async fn a_walk_over_several_topics_reads_each_in_index_order(db: &Db) {
	Corpus::seed(db).await;

	let target = SendTarget::Condition("'alpha' in topics || 'beta' in topics".to_owned());
	let reads = leptoskit_notifications::service::plans::candidate_reads(
		db.dialect(),
		&target,
		Some(person(40)),
	)
	.expect("the condition walks its topics");

	assert_eq!(reads.len(), 2, "one read per topic: {reads:?}");

	for read in &reads {
		let plan = explain(db, read).await;

		assert_reads_in_index_order(
			"a topic's page of subscribers",
			&plan,
			"topic_subscriptions_topic_person_idx",
		);
	}

	// What the reads answer, merged, is the union in person order.
	let mut merged = std::collections::BTreeSet::new();

	for read in reads {
		for row in db.fetch(read).await.expect("the read runs") {
			merged.insert(format!("{:?}", row[0]));
		}
	}

	assert_eq!(merged.len(), 1_002, "501 of each topic, no one in both yet");
}

// ── Pruning ───────────────────────────────────────────────────────────

/// Seeds rows at chosen ages, counted back from `now`.
struct Seeds<'a> {
	db: &'a Db,
	sql: Sql,
	now: DateTime<Utc>,
	next: u128,
}

/// One notification to seed: when it was written, and what happened to it.
#[derive(Clone, Copy)]
struct Row {
	/// Days ago it was written, which is where its id sorts.
	written: i64,
	/// Days ago it became visible; negative is still to come.
	visible: i64,
	expired: Option<i64>,
	cancelled: Option<i64>,
	read: bool,
	data: bool,
}

impl Row {
	/// A plain notification written and visible `days` ago.
	fn seen(days: i64) -> Self {
		Self {
			written: days,
			visible: days,
			expired: None,
			cancelled: None,
			read: false,
			data: false,
		}
	}
}

impl<'a> Seeds<'a> {
	const DELIVERY_COLUMNS: &'static str = "id, notification, send_key, endpoint, generation, \
	                                        person, platform, state, reason, attempts, \
	                                        finished_at, created_at";
	const NOTIFICATION_COLUMNS: &'static str = "id, send_key, person, category, title, body, \
	                                            data, visible_at, expires_at, cancelled_at, \
	                                            read_at, created_at, kind";

	fn new(db: &'a Db) -> Self {
		Self {
			db,
			sql: Sql(db.dialect()),
			now: Utc::now(),
			next: 0,
		}
	}

	fn ago(&self, days: i64) -> DateTime<Utc> {
		self.now - Duration::days(days)
	}

	fn stamp(&self, at: DateTime<Utc>) -> String {
		self.sql
			.timestamp(&at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
	}

	fn stamp_or_null(&self, days: Option<i64>) -> String {
		days.map_or_else(|| "NULL".to_owned(), |days| self.stamp(self.ago(days)))
	}

	/// The values of one notification of the send `key`, and its id.
	fn notification_values(&mut self, key: &str, row: Row) -> (Uuid, String) {
		self.next += 1;
		let id = id_at(self.ago(row.written), self.next);
		let values = format!(
			"({}, '{key}', {}, 'news', 't', '', '{{}}', {}, {}, {}, {}, {}, '{}')",
			self.sql.uuid(id),
			self.sql.uuid(person(self.next)),
			self.stamp(self.ago(row.visible)),
			self.stamp_or_null(row.expired),
			self.stamp_or_null(row.cancelled),
			self.stamp_or_null(row.read.then_some(row.visible)),
			self.stamp(self.ago(row.written)),
			if row.data { "data" } else { "notification" },
		);

		(id, values)
	}

	/// One notification of the send `key`; `key` is also how the checks
	/// name it.
	async fn notification(&mut self, key: &str, row: Row) -> Uuid {
		let (id, values) = self.notification_values(key, row);
		insert(
			self.db,
			"notifications",
			Self::NOTIFICATION_COLUMNS,
			&[values],
		)
		.await;
		id
	}

	/// The values of one delivery of `notification`, written `written`
	/// days ago and finished `finished` days ago, or still in the queue.
	fn delivery_values(
		&mut self,
		reason: &str,
		key: &str,
		notification: Uuid,
		state: &str,
		written: i64,
		finished: Option<i64>,
	) -> String {
		self.next += 1;

		format!(
			"({}, {}, '{key}', {}, 1, {}, 'fcm', '{state}', '{reason}', 1, {}, {})",
			self.sql.uuid(id_at(self.ago(written), self.next)),
			self.sql.uuid(notification),
			self.sql.uuid(person(900_000 + self.next)),
			self.sql.uuid(person(self.next)),
			self.stamp_or_null(finished),
			self.stamp(self.ago(written)),
		)
	}

	/// One delivery; `label` is stored as its reason, which is how the
	/// checks name it.
	async fn delivery(
		&mut self,
		label: &str,
		key: &str,
		notification: Uuid,
		state: &str,
		written: i64,
		finished: Option<i64>,
	) {
		let values = self.delivery_values(label, key, notification, state, written, finished);
		insert(
			self.db,
			"push_deliveries",
			Self::DELIVERY_COLUMNS,
			&[values],
		)
		.await;
	}

	/// The record of the send `key`, made `made` days ago, in `state`.
	async fn send(&mut self, key: &str, made: i64, state: &str) {
		let values = format!(
			"('{key}', '{}', 0, 0, 0, '{state}', {})",
			"0".repeat(64),
			self.stamp(self.ago(made))
		);
		insert(
			self.db,
			"sends",
			"idempotency_key, intent, delivered, skipped_opted_out, targeted, state, created_at",
			&[values],
		)
		.await;
	}
}

async fn notifications_left(db: &Db) -> Vec<String> {
	texts(db, "SELECT send_key FROM notifications").await
}

async fn deliveries_left(db: &Db) -> Vec<String> {
	texts(db, "SELECT reason FROM push_deliveries").await
}

async fn sends_left(db: &Db) -> Vec<String> {
	texts(db, "SELECT idempotency_key FROM sends").await
}

/// The default retention deletes what ended a month ago, inbox rows past a
/// year, finished deliveries past a month, and send records past three
/// months that nothing names any more — and nothing the service still
/// works with, whatever its age. Then shorter settings take more, and
/// `off` takes nothing.
#[expect(
	clippy::cognitive_complexity,
	clippy::too_many_lines,
	reason = "one corpus, swept three ways; each sweep is judged against the same rows"
)]
async fn pruning_takes_what_is_over_and_nothing_else(db: &Db) {
	let mut seeds = Seeds::new(db);
	let now = seeds.now;

	// Notifications. Each is named by its send key.
	let expired_long = seeds
		.notification(
			"expired-long",
			Row {
				expired: Some(40),
				..Row::seen(50)
			},
		)
		.await;
	let expired_lately = seeds
		.notification(
			"expired-lately",
			Row {
				expired: Some(10),
				..Row::seen(50)
			},
		)
		.await;
	seeds
		.notification(
			"cancelled-long",
			Row {
				cancelled: Some(40),
				..Row::seen(50)
			},
		)
		.await;
	// Cancelled long ago, but scheduled for a time still to come: the job
	// that would have shown it is still waiting, under the send's key.
	seeds
		.notification(
			"cancelled-ahead",
			Row {
				visible: -5,
				cancelled: Some(40),
				..Row::seen(50)
			},
		)
		.await;
	seeds
		.notification(
			"silent-long",
			Row {
				data: true,
				..Row::seen(40)
			},
		)
		.await;
	// A data row ends a delivery window after its time: thirty days ago is
	// not yet thirty days past that.
	seeds
		.notification(
			"silent-lately",
			Row {
				data: true,
				..Row::seen(30)
			},
		)
		.await;
	seeds
		.notification(
			"old-read",
			Row {
				read: true,
				..Row::seen(400)
			},
		)
		.await;
	let last_quarter = seeds.notification("last-quarter", Row::seen(100)).await;
	// Written more than a year ago, for a time still to come.
	seeds
		.notification(
			"scheduled",
			Row {
				visible: -5,
				..Row::seen(400)
			},
		)
		.await;
	let still_queued = seeds
		.notification(
			"still-queued",
			Row {
				expired: Some(40),
				..Row::seen(50)
			},
		)
		.await;
	seeds
		.notification(
			"still-fanning",
			Row {
				expired: Some(40),
				..Row::seen(50)
			},
		)
		.await;

	// Deliveries. Each is named by its reason.
	seeds
		.delivery(
			"of-a-pruned-row",
			"expired-long",
			expired_long,
			"accepted",
			50,
			Some(49),
		)
		.await;
	seeds
		.delivery(
			"finished-long",
			"last-quarter",
			last_quarter,
			"accepted",
			100,
			Some(99),
		)
		.await;
	seeds
		.delivery(
			"finished-lately",
			"last-quarter",
			last_quarter,
			"accepted",
			100,
			Some(10),
		)
		.await;
	seeds
		.delivery(
			"rejected-long",
			"expired-lately",
			expired_lately,
			"rejected",
			50,
			Some(45),
		)
		.await;
	seeds
		.delivery(
			"in-the-queue",
			"still-queued",
			still_queued,
			"retrying",
			50,
			None,
		)
		.await;
	seeds
		.delivery(
			"orphan-in-the-queue",
			"delivery-only",
			Uuid::now_v7(),
			"enqueued",
			50,
			None,
		)
		.await;

	// Send records, all but one made four months ago.
	for (key, state) in [
		("expired-long", "complete"),
		("expired-lately", "complete"),
		("cancelled-long", "cancelled"),
		("cancelled-ahead", "cancelled"),
		("last-quarter", "complete"),
		("still-fanning", "fanning_out"),
		("delivery-only", "complete"),
		("reached-nobody", "complete"),
	] {
		seeds.send(key, 120, state).await;
	}

	seeds.send("reached-nobody-lately", 10, "complete").await;

	let everything = (
		notifications_left(db).await,
		deliveries_left(db).await,
		sends_left(db).await,
	);

	// `off` for everything prunes nothing.
	assert_eq!(
		retention::prune(db, &Retention::forever(), now)
			.await
			.expect("the sweep runs"),
		Pruned::default()
	);
	assert_eq!(
		(
			notifications_left(db).await,
			deliveries_left(db).await,
			sends_left(db).await
		),
		everything,
		"kept forever means nothing goes"
	);

	// The defaults.
	let pruned = retention::prune(db, &Retention::default(), now)
		.await
		.expect("the sweep runs");

	assert_eq!(
		notifications_left(db).await,
		[
			"cancelled-ahead",
			"expired-lately",
			"last-quarter",
			"scheduled",
			"silent-lately",
			"still-fanning",
			"still-queued",
		],
		"gone: expired-long, cancelled-long, silent-long, old-read"
	);
	assert_eq!(
		deliveries_left(db).await,
		["finished-lately", "in-the-queue", "orphan-in-the-queue"],
		"gone: of-a-pruned-row with its notification, finished-long and rejected-long by age"
	);
	assert_eq!(
		sends_left(db).await,
		[
			"cancelled-ahead",
			"delivery-only",
			"expired-lately",
			"last-quarter",
			"reached-nobody-lately",
			"still-fanning",
		],
		"gone: expired-long and cancelled-long once their rows went, and reached-nobody"
	);
	assert_eq!(
		pruned,
		Pruned {
			notifications: 4,
			deliveries: 3,
			sends: 3,
		}
	);

	// Again finds nothing.
	assert_eq!(
		retention::prune(db, &Retention::default(), now)
			.await
			.expect("the sweep runs"),
		Pruned::default(),
		"a second sweep has nothing to do"
	);

	// The operator's own, shorter settings take more — and still nothing
	// that is queued or fanning out.
	let short = Retention {
		inbox_days: Some(60),
		ended_days: Some(5),
		deliveries_days: Some(5),
		sends_days: Some(2),
	};
	let pruned = retention::prune(db, &short, now)
		.await
		.expect("the sweep runs");

	assert_eq!(
		notifications_left(db).await,
		[
			"cancelled-ahead",
			"scheduled",
			"still-fanning",
			"still-queued"
		],
		"gone: expired-lately, silent-lately, and last-quarter with its delivery"
	);
	assert_eq!(
		deliveries_left(db).await,
		["in-the-queue", "orphan-in-the-queue"]
	);
	assert_eq!(
		sends_left(db).await,
		["cancelled-ahead", "delivery-only", "still-fanning"],
		"a record goes once nothing names it; one a delivery still names stays"
	);
	assert_eq!(
		pruned,
		Pruned {
			notifications: 3,
			deliveries: 1,
			sends: 3,
		}
	);
}

/// More rows than one batch holds all go, each batch its own write, with
/// their deliveries and then their send's record.
async fn a_sweep_works_through_more_than_one_batch(db: &Db) {
	let mut seeds = Seeds::new(db);
	let now = seeds.now;
	let mut notifications = Vec::new();
	let mut deliveries = Vec::new();

	for _ in 0..1_203 {
		let (id, values) = seeds.notification_values(
			"bulk",
			Row {
				expired: Some(40),
				..Row::seen(50)
			},
		);
		notifications.push(values);
		deliveries.push(seeds.delivery_values("bulk", "bulk", id, "accepted", 50, Some(50)));
	}

	insert(
		db,
		"notifications",
		Seeds::NOTIFICATION_COLUMNS,
		&notifications,
	)
	.await;
	insert(db, "push_deliveries", Seeds::DELIVERY_COLUMNS, &deliveries).await;
	seeds.send("bulk", 120, "complete").await;
	let kept = seeds.notification("kept", Row::seen(3)).await;
	seeds
		.delivery("kept", "kept", kept, "accepted", 3, Some(3))
		.await;

	assert_eq!(
		retention::prune(db, &Retention::default(), now)
			.await
			.expect("the sweep runs"),
		Pruned {
			notifications: 1_203,
			deliveries: 1_203,
			sends: 1,
		}
	);
	assert_eq!(notifications_left(db).await, ["kept"]);
	assert_eq!(deliveries_left(db).await, ["kept"]);
	assert_eq!(sends_left(db).await, Vec::<String>::new());
}

/// The service over `db`, with no workers: the checks below run what they
/// need themselves.
async fn serve(db: &Db) -> std::net::SocketAddr {
	use rpc::server::Anonymous;
	use rpc::server::ServiceCallers;

	let callers = ServiceCallers::new(Anonymous)
		.caller(TOKEN, rpc::server::APP_CALLER)
		.forwarded_identity(SERVICE, TOKEN);
	let (router, live) = leptoskit_notifications::service::routes_with(db.clone(), None, callers);
	live.start().await.expect("live starts");
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let address = listener.local_addr().unwrap();
	tokio::spawn(async move {
		leptoskit_axum::axum::serve(listener, router).await.unwrap();
	});

	address
}

/// Sends `key` to one person, as the app. Answers the status, and for an
/// accepted send whether the service took it for a repeat.
async fn send(address: std::net::SocketAddr, key: &str, to: Uuid) -> (u16, Option<bool>) {
	let response = reqwest::Client::new()
		.post(format!("http://{address}{PREFIX}/rpc/send"))
		.bearer_auth(TOKEN)
		.json(&json!({
			"idempotency_key": key,
			"spec": { "category": "receipts", "title": "Your receipt" },
			"target": { "recipients": [to] },
		}))
		.send()
		.await
		.unwrap();
	let status = response.status().as_u16();
	let text = response.text().await.unwrap();
	let duplicate = serde_json::from_str::<serde_json::Value>(&text)
		.ok()
		.and_then(|body| body["duplicate"].as_bool());

	(status, duplicate)
}

/// A send's key answers the original for as long as its record stands —
/// at least `sends_days`, and as long as any notification it wrote is
/// still stored — so a retry inside that window never sends twice. Once
/// the record is pruned, nothing of the send is left, and the key is a new
/// send's.
#[expect(
	clippy::cognitive_complexity,
	reason = "one send followed through its whole window, a step at a time"
)]
async fn a_key_is_remembered_until_its_send_is_gone(db: &Db) {
	const NEW: (u16, Option<bool>) = (200, Some(false));
	const REPEAT: (u16, Option<bool>) = (200, Some(true));

	let address = serve(db).await;
	let jobs = rpc::testing::TestJobs::new(db).service(Some(SERVICE));
	let to = person(77);
	let now = Utc::now();
	let rows = "SELECT count(*) FROM notifications WHERE send_key = 'receipt-1'";

	assert_eq!(send(address, "receipt-1", to).await, NEW);
	assert_eq!(send(address, "receipt-1", to).await, REPEAT);
	assert_eq!(count(db, rows).await, 1);

	// A hundred days on, the record is older than `sends_days`, but its
	// notification is still in the inbox: the record stays, and a retry
	// still answers the original.
	let pruned = retention::prune(db, &Retention::default(), now + Duration::days(100))
		.await
		.expect("the sweep runs");
	assert_eq!(pruned, Pruned::default());
	assert_eq!(
		send(address, "receipt-1", to).await,
		REPEAT,
		"a retry inside the window is the same send"
	);
	assert_eq!(count(db, rows).await, 1, "and writes nothing");

	// Past the inbox's year the notification goes, and with nothing left
	// that names the key, so does the record.
	let pruned = retention::prune(db, &Retention::default(), now + Duration::days(400))
		.await
		.expect("the sweep runs");
	assert_eq!(
		pruned,
		Pruned {
			notifications: 1,
			deliveries: 0,
			sends: 1,
		}
	);
	assert_eq!(count(db, rows).await, 0);

	// The send's jobs are keyed by its key too. While the job table still
	// holds one, the key can't start a second send: the call is refused as
	// busy, and nothing is written.
	assert_eq!(
		send(address, "receipt-1", to).await,
		(409, None),
		"a key whose jobs are still held is refused, not sent again"
	);
	assert_eq!(count(db, rows).await, 0);

	// Its jobs run, and the job table forgets them, as it does a day after
	// they finish. Now the key is free: the same call is a new send, with
	// one row.
	jobs.run_jobs_until_idle().await;
	jobs.prune(std::time::Duration::ZERO).await;
	assert_eq!(
		send(address, "receipt-1", to).await,
		NEW,
		"past the window the key is a new send"
	);
	assert_eq!(count(db, rows).await, 1);
}

// ── SQLite ────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn sqlite_the_reads_that_grew_use_their_indexes() {
	the_reads_that_grew_use_their_indexes(&on_sqlite().await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn sqlite_an_imports_reads_use_their_indexes() {
	an_imports_reads_use_their_indexes(&on_sqlite().await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn sqlite_a_send_report_counts_only_the_sends_own_rows() {
	a_send_report_counts_only_the_sends_own_rows(&on_sqlite().await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn sqlite_the_composers_reads_use_their_indexes() {
	the_composers_reads_use_their_indexes(&on_sqlite().await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn sqlite_the_campaign_lists_use_their_indexes() {
	the_campaign_lists_use_their_indexes(&on_sqlite().await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn sqlite_the_recurrence_reads_use_their_indexes() {
	the_recurrence_reads_use_their_indexes(&on_sqlite().await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn sqlite_a_walk_over_several_topics_reads_each_in_index_order() {
	a_walk_over_several_topics_reads_each_in_index_order(&on_sqlite().await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn sqlite_pruning_takes_what_is_over_and_nothing_else() {
	pruning_takes_what_is_over_and_nothing_else(&on_sqlite().await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn sqlite_a_sweep_works_through_more_than_one_batch() {
	a_sweep_works_through_more_than_one_batch(&on_sqlite().await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn sqlite_a_key_is_remembered_until_its_send_is_gone() {
	a_key_is_remembered_until_its_send_is_gone(&on_sqlite().await).await;
}

/// The hourly job is on the service's schedule and prunes with the default
/// retention when its workers carry no other: an hour of the job clock
/// enqueues one tick, and running it deletes what ended a month ago.
#[tokio::test(flavor = "multi_thread")]
async fn sqlite_the_hourly_job_prunes_with_the_default_retention() {
	let db = on_sqlite().await;
	let mut seeds = Seeds::new(&db);
	seeds
		.notification(
			"expired-long",
			Row {
				expired: Some(40),
				..Row::seen(50)
			},
		)
		.await;
	seeds.notification("last-quarter", Row::seen(100)).await;

	let jobs = rpc::testing::TestJobs::new(&db).service(Some(SERVICE));
	assert_eq!(jobs.run_jobs_until_idle().await, 0, "nothing is due yet");

	jobs.advance(std::time::Duration::from_secs(3_600)).await;
	let ticks: Vec<_> = jobs
		.all()
		.await
		.into_iter()
		.filter(|job| job.name == "prune")
		.collect();
	assert_eq!(ticks.len(), 1, "one tick an hour: {ticks:?}");

	jobs.run_jobs_until_idle().await;

	assert_eq!(notifications_left(&db).await, ["last-quarter"]);
}

// ── Postgres ──────────────────────────────────────────────────────────

#[cfg(feature = "postgres")]
mod on_postgres {
	use super::*;
	use crate::postgres::Postgres;

	async fn connect(postgres: &Postgres) -> Db {
		let db = Db::connect(&postgres.database())
			.await
			.expect("the database opens");
		db.migrate(leptoskit_notifications::POSTGRES_MIGRATIONS)
			.await
			.expect("migrations apply");
		db
	}

	#[tokio::test(flavor = "multi_thread")]
	async fn postgres_an_imports_reads_use_their_indexes() {
		let Some(postgres) = Postgres::start() else {
			return;
		};

		an_imports_reads_use_their_indexes(&connect(&postgres).await).await;
	}

	#[tokio::test(flavor = "multi_thread")]
	async fn postgres_the_reads_that_grew_use_their_indexes() {
		let Some(postgres) = Postgres::start() else {
			return;
		};

		the_reads_that_grew_use_their_indexes(&connect(&postgres).await).await;
	}

	#[tokio::test(flavor = "multi_thread")]
	async fn postgres_a_send_report_counts_only_the_sends_own_rows() {
		let Some(postgres) = Postgres::start() else {
			return;
		};

		a_send_report_counts_only_the_sends_own_rows(&connect(&postgres).await).await;
	}

	#[tokio::test(flavor = "multi_thread")]
	async fn postgres_the_composers_reads_use_their_indexes() {
		let Some(postgres) = Postgres::start() else {
			return;
		};

		the_composers_reads_use_their_indexes(&connect(&postgres).await).await;
	}

	#[tokio::test(flavor = "multi_thread")]
	async fn postgres_the_campaign_lists_use_their_indexes() {
		let Some(postgres) = Postgres::start() else {
			return;
		};

		the_campaign_lists_use_their_indexes(&connect(&postgres).await).await;
	}

	#[tokio::test(flavor = "multi_thread")]
	async fn postgres_the_recurrence_reads_use_their_indexes() {
		let Some(postgres) = Postgres::start() else {
			return;
		};

		the_recurrence_reads_use_their_indexes(&connect(&postgres).await).await;
	}

	#[tokio::test(flavor = "multi_thread")]
	async fn postgres_a_walk_over_several_topics_reads_each_in_index_order() {
		let Some(postgres) = Postgres::start() else {
			return;
		};

		a_walk_over_several_topics_reads_each_in_index_order(&connect(&postgres).await).await;
	}

	#[tokio::test(flavor = "multi_thread")]
	async fn postgres_pruning_takes_what_is_over_and_nothing_else() {
		let Some(postgres) = Postgres::start() else {
			return;
		};

		pruning_takes_what_is_over_and_nothing_else(&connect(&postgres).await).await;
	}

	#[tokio::test(flavor = "multi_thread")]
	async fn postgres_a_sweep_works_through_more_than_one_batch() {
		let Some(postgres) = Postgres::start() else {
			return;
		};

		a_sweep_works_through_more_than_one_batch(&connect(&postgres).await).await;
	}

	#[tokio::test(flavor = "multi_thread")]
	async fn postgres_a_key_is_remembered_until_its_send_is_gone() {
		let Some(postgres) = Postgres::start() else {
			return;
		};

		a_key_is_remembered_until_its_send_is_gone(&connect(&postgres).await).await;
	}
}
