//! The insights engine end to end: seeded events, persons, and sessions
//! through the real writer, then exact numbers from the `run` operation
//! and the saved insights and dashboards, on SQLite. The same seeds run on
//! Postgres as the ignored test at the bottom.

#![cfg(not(target_arch = "wasm32"))]

use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;

use leptoskit_analytics::query::*;
use leptoskit_analytics::service::Ingest;
use leptoskit_analytics::service::IngestConfig;
use leptoskit_analytics::service::JsonBatch;
use leptoskit_analytics::service::JsonEvent;
use leptoskit_analytics::service::Source;
use leptoskit_analytics::service::query;
use leptoskit_db::Db;
use leptoskit_db::Read as _;
use leptoskit_db::Statement;
use leptoskit_rpc as rpc;
use leptoskit_rpc::Operation as _;
use rpc::server::APP_CALLER;
use serde_json::json;
use uuid::Uuid;

/// The seeds' time base: the week of Monday 9 November 2026, UTC.
const WEEK: u32 = 9;

/// An instant `days` into the week, at `hour`:00 UTC.
fn at(days: u32, hour: u32) -> String {
	format!("2026-11-{:02}T{:02}:00:00Z", WEEK + days, hour)
}

/// The next UUID v7-shaped id, so every seed event differs. The writer
/// deduplicates on it, and reads its time bits, which the explicit
/// `timestamp` overrides.
fn next_uuid() -> Uuid {
	static NEXT: AtomicU32 = AtomicU32::new(1);
	let seed = NEXT.fetch_add(1, Ordering::Relaxed);
	let mut bytes = [0u8; 16];
	bytes[..4].copy_from_slice(&seed.to_be_bytes());
	bytes[6] = 0x70;
	bytes[8] = 0x80;
	Uuid::from_bytes(bytes)
}

/// One event, with its exact time.
fn event(name: &str, distinct_id: &str, when: &str, properties: serde_json::Value) -> JsonEvent {
	JsonEvent {
		event: name.to_owned(),
		distinct_id: distinct_id.to_owned(),
		uuid: Some(next_uuid()),
		timestamp: Some(when.to_owned()),
		session_id: None,
		properties: properties.as_object().cloned().unwrap_or_default(),
	}
}

/// Identifies an anonymous id, so its events resolve to a person.
fn identify(distinct_id: &str, anon_distinct_id: &str, when: &str) -> JsonEvent {
	event(
		"$identify",
		distinct_id,
		when,
		json!({"$anon_distinct_id": anon_distinct_id}),
	)
}

/// A pageview in a session, with the page and the visit's origin. The
/// URL's path is what follows the seed app's host.
fn pageview(distinct_id: &str, session: Uuid, when: &str, url: &str, referrer: &str) -> JsonEvent {
	let mut event = event(
		"$pageview",
		distinct_id,
		when,
		json!({
			"$current_url": url,
			"$pathname": url.split("app.example").nth(1).unwrap_or("/"),
			"$referrer": referrer,
			"utm_source": if referrer.contains("google") { "google" } else { "" },
			"utm_medium": if referrer.contains("google") { "organic" } else { "" },
			"utm_campaign": "launch",
			"$browser": "Chrome",
			"$os": "Mac OS X",
			"$device_type": "Desktop",
		}),
	);
	event.session_id = Some(session);
	event
}

struct Harness {
	db: Db,
	ingest: Ingest,
}

impl Harness {
	/// A migrated SQLite database with a writer on it.
	async fn start() -> Self {
		Self::with_config(IngestConfig::default()).await
	}

	async fn with_config(config: IngestConfig) -> Self {
		let db =
			leptoskit_db::testing::sqlite_migrated(leptoskit_analytics::service::MIGRATIONS).await;
		let ingest = Ingest::start(db.clone(), config);
		Self { db, ingest }
	}

	/// Sends events through the writer and waits for the commit.
	async fn store(&self, events: Vec<JsonEvent>) {
		let received_at = leptoskit_db::now();
		let accepted = leptoskit_analytics::service::accept::json_batch(
			JsonBatch {
				sent_at: None,
				batch: events,
			},
			received_at,
			Source::Server,
		);
		assert!(
			accepted.refused.is_empty(),
			"the seeds are all valid: {:?}",
			accepted.refused
		);
		self.ingest
			.submit(accepted, received_at)
			.await
			.expect("the batch stores");
	}

	/// Runs an operation as the app, in process.
	async fn as_app<O: rpc::Operation>(&self, op: O) -> rpc::Result<O::Output> {
		let identity = rpc::server::Identity::anonymous().with_caller(APP_CALLER.to_owned());
		rpc::server::call_as(&self.db, identity, None, op).await
	}

	/// The tick row, as stored.
	async fn tick(&self) -> Option<(String, i64)> {
		let rows = self
			.db
			.fetch(Statement {
				sql: "SELECT \"at\", \"events\" FROM \"ingest_ticks\" WHERE \"id\" = 1".into(),
				args: Vec::new(),
			})
			.await
			.expect("the tick reads");
		rows.first().and_then(|row| {
			Some((
				match row.first()? {
					leptoskit_db::Value::Text(text) => text.clone(),
					_ => String::new(),
				},
				match row.get(1)? {
					leptoskit_db::Value::Int(events) => *events,
					_ => 0,
				},
			))
		})
	}
}

fn trends_query(range: DateRange, compare: bool, breakdown: Option<Breakdown>) -> Query {
	Query::Trends(TrendsQuery {
		range,
		interval: Interval::Day,
		series: vec!["signed_up".into()],
		filters: Vec::new(),
		math: Math::Total,
		breakdown,
		compare,
		test_accounts: None,
	})
}

async fn run_trends(harness: &Harness, query: Query) -> TrendsResult {
	match harness
		.as_app(query::Run { query })
		.await
		.expect("the trend runs")
		.result
	{
		QueryResult::Trends(trends) => trends,
		other => panic!("expected trends, got {}", other.kind()),
	}
}

fn bucket_values(series: &TrendSeries) -> Vec<f64> {
	series.buckets.iter().map(|bucket| bucket.value).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_trend_counts_its_buckets_fills_gaps_and_compares() {
	let harness = Harness::start().await;
	harness
		.store(vec![
			event("signed_up", "ada", &at(0, 9), json!({"plan": "pro"})),
			event("signed_up", "grace", &at(0, 10), json!({"plan": "pro"})),
			event("signed_up", "linus", &at(1, 11), json!({"plan": "free"})),
			event("signed_up", "grace", &at(1, 12), json!({"plan": "free"})),
		])
		.await;

	let trends = run_trends(
		&harness,
		trends_query(DateRange::between(at(0, 0), at(4, 0)), true, None),
	)
	.await;
	assert_eq!(trends.series.len(), 1);
	let series = &trends.series[0];
	assert_eq!(series.event, "signed_up");
	// Four day buckets, the last two empty and filled with zero.
	assert_eq!(bucket_values(series), [2.0, 2.0, 0.0, 0.0]);
	assert_eq!(series.buckets[0].start, "2026-11-09T00:00:00");
	assert_eq!(series.total, 4.0);
	// The previous period, four days earlier, held nothing.
	assert_eq!(trends.previous.len(), 1);
	assert!(
		trends.previous[0]
			.buckets
			.iter()
			.all(|bucket| bucket.value == 0.0)
	);

	// The same range broken down by plan: pro first, free second, each
	// with its own buckets.
	let trends = run_trends(
		&harness,
		trends_query(
			DateRange::between(at(0, 0), at(4, 0)),
			false,
			Some(Breakdown::event("plan")),
		),
	)
	.await;
	let plans: Vec<(Option<&serde_json::Value>, Vec<f64>)> = trends
		.series
		.iter()
		.map(|series| (series.breakdown_value.as_ref(), bucket_values(series)))
		.collect();
	// Ties rank alphabetically, so `free` before `pro` at two each.
	assert_eq!(
		plans,
		vec![
			(Some(&json!("free")), vec![0.0, 2.0, 0.0, 0.0]),
			(Some(&json!("pro")), vec![2.0, 0.0, 0.0, 0.0]),
		]
	);

	// Unique persons: grace signed up twice on day one and counts once.
	let query = Query::Trends(TrendsQuery {
		range: DateRange::between(at(0, 0), at(2, 0)),
		interval: Interval::Day,
		series: vec!["signed_up".into()],
		filters: Vec::new(),
		math: Math::UniquePersons,
		breakdown: None,
		compare: false,
		test_accounts: None,
	});
	let trends = run_trends(&harness, query).await;
	assert_eq!(bucket_values(&trends.series[0]), [2.0, 2.0]);

	// A test account's events drop out.
	let query = Query::Trends(TrendsQuery {
		range: DateRange::between(at(0, 0), at(2, 0)),
		interval: Interval::Day,
		series: vec!["signed_up".into()],
		filters: Vec::new(),
		math: Math::Total,
		breakdown: None,
		compare: false,
		test_accounts: Some(TestAccounts::Exclude(vec!["grace".into()])),
	});
	let trends = run_trends(&harness, query).await;
	assert_eq!(trends.series[0].total, 2.0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_funnel_counts_order_windows_and_time_to_convert() {
	let harness = Harness::start().await;
	// Kay identifies herself, so her person can appear on the actors page.
	harness.store(vec![identify("kay", "kay", &at(0, 8))]).await;
	let day = |n: u32, hour: u32| at(n, hour);
	harness
		.store(vec![
			// Ada walks the whole funnel within the window.
			event("$pageview", "ada", &day(0, 9), json!({})),
			event("signed_up", "ada", &day(1, 9), json!({})),
			event("checkout", "ada", &day(2, 9), json!({})),
			// Grace never signs up.
			event("$pageview", "grace", &day(0, 9), json!({})),
			event("checkout", "grace", &day(1, 9), json!({})),
			// Linus signs up outside the window.
			event("$pageview", "linus", &day(0, 9), json!({})),
			event("signed_up", "linus", &day(3, 10), json!({})),
			// Kay walks the steps, but checks out before signing up: the
			// ordered funnel stops at two, the unordered one finishes.
			event("$pageview", "kay", &day(0, 9), json!({})),
			event("checkout", "kay", &day(1, 9), json!({})),
			event("signed_up", "kay", &day(1, 12), json!({})),
		])
		.await;

	let funnel = |order: FunnelOrder| {
		Query::Funnels(FunnelsQuery {
			range: DateRange::between(at(0, 0), at(4, 0)),
			steps: vec!["$pageview".into(), "signed_up".into(), "checkout".into()],
			window_seconds: Some(2 * 86_400),
			order,
			filters: Vec::new(),
			breakdown: None,
			test_accounts: None,
		})
	};
	let fold = |answer: Answer| {
		match answer.result {
			QueryResult::Funnels(funnels) => funnels,
			other => panic!("expected funnels, got {}", other.kind()),
		}
	};
	let ordered = fold(
		harness
			.as_app(query::Run {
				query: funnel(FunnelOrder::Ordered),
			})
			.await
			.unwrap(),
	);
	let counts: Vec<u64> = ordered.steps.iter().map(|step| step.count).collect();
	assert_eq!(counts, [4, 2, 1], "ada and kay to two; only ada converts");
	assert_eq!(ordered.steps[0].name, "$pageview");
	assert_eq!(ordered.steps[2].drop_off, 1);
	assert_eq!(ordered.overall_conversion, 0.25);
	assert_eq!(ordered.median_seconds_to_convert, Some(2.0 * 86_400.0));

	let unordered = fold(
		harness
			.as_app(query::Run {
				query: funnel(FunnelOrder::Unordered),
			})
			.await
			.unwrap(),
	);
	let counts: Vec<u64> = unordered.steps.iter().map(|step| step.count).collect();
	assert_eq!(
		counts,
		[4, 2, 2],
		"kay's out-of-order steps count unordered"
	);

	// Strict: the next step must be the person's very next event. Ada's
	// walk is; kay's isn't, and linus's third step is outside the window.
	let strict = fold(
		harness
			.as_app(query::Run {
				query: funnel(FunnelOrder::Strict),
			})
			.await
			.unwrap(),
	);
	let counts: Vec<u64> = strict.steps.iter().map(|step| step.count).collect();
	assert_eq!(counts, [4, 1, 1]);

	// The actors of the drop-off after the second step are those who
	// signed up but didn't check out: kay, ordered.
	let answer = harness
		.as_app(query::Run {
			query: Query::Actors(Box::new(ActorsQuery {
				query: funnel(FunnelOrder::Ordered),
				point: ActorPoint::FunnelDropOff { after: 2 },
				limit: 50,
				offset: 0,
			})),
		})
		.await
		.unwrap();
	let QueryResult::Actors(page) = answer.result else {
		panic!("expected actors");
	};
	let names: Vec<String> = page
		.actors
		.iter()
		.flat_map(|actor| actor.distinct_ids.iter())
		.cloned()
		.collect();
	assert_eq!(names, ["kay"], "kay reached two and stopped there");
}

#[tokio::test(flavor = "multi_thread")]
async fn retention_builds_the_cohort_grid() {
	let harness = Harness::start().await;
	// Ada and Grace identify themselves, so the cell's actors page names
	// them.
	harness
		.store(vec![
			identify("ada", "ada", &at(0, 8)),
			identify("grace", "grace", &at(0, 8)),
		])
		.await;
	// Ada, Grace, and Linus start on day zero. Ada and Grace ping back on
	// day one; Ada pings on day two as well.
	harness
		.store(vec![
			event("signed_up", "ada", &at(0, 9), json!({})),
			event("signed_up", "grace", &at(0, 10), json!({})),
			event("signed_up", "linus", &at(0, 11), json!({})),
			event("opened", "ada", &at(0, 12), json!({})),
			event("opened", "ada", &at(1, 9), json!({})),
			event("opened", "grace", &at(1, 10), json!({})),
			event("opened", "ada", &at(2, 9), json!({})),
		])
		.await;
	let retention = |kind: RetentionKind| {
		Query::Retention(RetentionQuery {
			range: DateRange::between(at(0, 0), at(4, 0)),
			start_event: "signed_up".into(),
			return_event: "opened".into(),
			kind,
			period: Interval::Day,
			periods: Some(3),
			filters: Vec::new(),
			test_accounts: None,
		})
	};
	let fold = |answer: Answer| {
		match answer.result {
			QueryResult::Retention(retention) => retention,
			other => panic!("expected retention, got {}", other.kind()),
		}
	};
	let first_time = fold(
		harness
			.as_app(query::Run {
				query: retention(RetentionKind::FirstTime),
			})
			.await
			.unwrap(),
	);
	assert_eq!(first_time.period, Interval::Day);
	assert_eq!(first_time.cohorts.len(), 1);
	let cohort = &first_time.cohorts[0];
	assert_eq!(cohort.start, "2026-11-09T00:00:00");
	assert_eq!(cohort.size, 3);
	// Day zero itself: Ada pinged the same day. Day one: Ada and Grace.
	// Day two: Ada alone. Day three, the last the query asked for: none.
	assert_eq!(cohort.returns, [1, 2, 1, 0]);

	let recurring = fold(
		harness
			.as_app(query::Run {
				query: retention(RetentionKind::Recurring),
			})
			.await
			.unwrap(),
	);
	assert_eq!(recurring.cohorts[0].size, 3);

	// The people of the day-one cell are Ada and Grace.
	let answer = harness
		.as_app(query::Run {
			query: Query::Actors(Box::new(ActorsQuery {
				query: retention(RetentionKind::FirstTime),
				point: ActorPoint::RetentionCell {
					start_period: 0,
					return_period: 1,
				},
				limit: 50,
				offset: 0,
			})),
		})
		.await
		.unwrap();
	let QueryResult::Actors(page) = answer.result else {
		panic!("expected actors");
	};
	let names: Vec<String> = page
		.actors
		.iter()
		.flat_map(|actor| actor.distinct_ids.iter())
		.cloned()
		.collect();
	assert_eq!(names.len(), 2);
	assert!(names.contains(&"ada".to_owned()));
	assert!(names.contains(&"grace".to_owned()));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_web_overview_counts_sessions_with_deltas_and_tables() {
	let harness = Harness::start().await;
	let first = Uuid::from_bytes([7u8; 16]);
	let second = Uuid::from_bytes([8u8; 16]);
	let mut events = vec![
		pageview("ada", first, &at(0, 9), "https://app.example/", ""),
		pageview("ada", first, &at(0, 11), "https://app.example/pricing", ""),
		pageview(
			"grace",
			second,
			&at(0, 10),
			"https://app.example/",
			"https://www.google.com/",
		),
	];
	// The day before held one bouncing session, which the compared range
	// counts as the previous period's whole.
	let earlier = Uuid::from_bytes([9u8; 16]);
	let previous_day = at(0, 9).replace("2026-11-09", "2026-11-08");
	events.push(pageview(
		"linus",
		earlier,
		&previous_day,
		"https://app.example/",
		"",
	));
	harness.store(events).await;

	let answer = harness
		.as_app(query::Run {
			query: Query::WebOverview(WebQuery {
				range: DateRange::between(at(0, 0), at(1, 0)),
				compare: true,
				filters: Vec::new(),
				table_limit: 5,
			}),
		})
		.await
		.unwrap();
	let QueryResult::WebOverview(web) = answer.result else {
		panic!("expected the web overview");
	};
	assert_eq!(web.sessions.value, 2.0);
	assert_eq!(web.pageviews.value, 3.0);
	assert_eq!(web.visitors.value, 2.0);
	assert_eq!(web.bounce_rate.value, 0.5, "grace's session bounced");
	assert_eq!(
		web.avg_session_duration_seconds.value, 3_600.0,
		"ada's two hours and grace's nothing average an hour"
	);
	assert_eq!(web.sessions.previous, Some(1.0));
	assert_eq!(web.sessions.delta, Some(1.0));

	let names = |counts: &[NamedCount]| {
		counts
			.iter()
			.map(|count| count.name.clone())
			.collect::<Vec<_>>()
	};
	assert_eq!(names(&web.top_pages), ["/", "/pricing"]);
	assert_eq!(names(&web.entry_pages), ["/"]);
	assert_eq!(names(&web.referrers), ["google.com"]);
	// One visit each, so ties rank alphabetically.
	assert_eq!(names(&web.channels), ["Direct", "Organic Search"]);
	assert_eq!(names(&web.utm_sources), ["google"]);
	assert_eq!(names(&web.browsers), ["Chrome"]);
	assert_eq!(names(&web.operating_systems), ["Mac OS X"]);

	// A session filter narrows everything: grace's session only, and her
	// tables with it.
	let answer = harness
		.as_app(query::Run {
			query: Query::WebOverview(WebQuery {
				range: DateRange::between(at(0, 0), at(1, 0)),
				compare: false,
				filters: vec![PropertyFilter::session(
					"utm_source",
					Operator::Equals,
					json!("google"),
				)],
				table_limit: 5,
			}),
		})
		.await
		.unwrap();
	let QueryResult::WebOverview(web) = answer.result else {
		panic!("expected the web overview");
	};
	assert_eq!(web.sessions.value, 1.0);
	assert_eq!(names(&web.referrers), ["google.com"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_events_list_paginates_on_a_cursor() {
	let harness = Harness::start().await;
	let mut events = Vec::new();
	for index in 0..5 {
		events.push(event(
			"note",
			"ada",
			&at(0, 9 + index),
			json!({ "n": index }),
		));
	}
	harness.store(events).await;
	let page = |cursor: Option<EventsCursor>| {
		Query::Events(EventsQuery {
			range: DateRange::between(at(0, 0), at(2, 0)),
			event: Some("note".into()),
			filters: Vec::new(),
			columns: vec![EventColumn::Name, EventColumn::Properties],
			limit: 2,
			cursor,
		})
	};
	let mut seen = Vec::new();
	let mut cursor = None;
	for _ in 0..4 {
		let answer = harness
			.as_app(query::Run {
				query: page(cursor.take()),
			})
			.await
			.unwrap();
		let QueryResult::Events(page) = answer.result else {
			panic!("expected events");
		};
		assert_eq!(page.events.len(), 2.min(5 - seen.len()));
		for row in &page.events {
			assert!(
				row.person_id.is_none(),
				"the columns asked don't include it"
			);
			seen.push(row.properties["n"].as_u64().expect("the index"));
		}
		cursor = page.next;
		if cursor.is_none() {
			break;
		}
	}
	assert_eq!(seen, [4, 3, 2, 1, 0], "newest first, three pages");
}

#[tokio::test(flavor = "multi_thread")]
async fn actors_serve_a_trend_bucket() {
	let harness = Harness::start().await;
	harness
		.store(vec![
			event("signed_up", "ada", &at(0, 9), json!({})),
			event("signed_up", "grace", &at(1, 9), json!({})),
			event("opened", "ada", &at(1, 10), json!({})),
		])
		.await;
	let trends = trends_query(DateRange::between(at(0, 0), at(2, 0)), false, None);
	let bucket = |trends: Query| {
		Query::Actors(Box::new(ActorsQuery {
			query: trends,
			point: ActorPoint::Bucket {
				series: 0,
				bucket: "2026-11-09T00:00:00".into(),
				breakdown_value: None,
			},
			limit: 50,
			offset: 0,
		}))
	};
	let answer = harness
		.as_app(query::Run {
			query: bucket(trends_query(
				DateRange::between(at(0, 0), at(2, 0)),
				false,
				None,
			)),
		})
		.await
		.unwrap();
	let QueryResult::Actors(page) = answer.result else {
		panic!("expected actors");
	};
	assert_eq!(
		page.actors.len(),
		0,
		"anonymous sign-ups have no person yet"
	);

	// With an identified person behind the id, the bucket names her.
	harness.store(vec![identify("ada", "ada", &at(0, 8))]).await;
	let answer = harness
		.as_app(query::Run {
			query: bucket(trends_query(
				DateRange::between(at(0, 0), at(2, 0)),
				false,
				None,
			)),
		})
		.await
		.unwrap();
	let QueryResult::Actors(page) = answer.result else {
		panic!("expected actors");
	};
	assert_eq!(page.actors.len(), 1, "ada's person is in the bucket");
	let names: Vec<String> = page.actors[0].distinct_ids.iter().cloned().collect();
	assert!(names.contains(&"ada".to_owned()), "{names:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_property_filter_cannot_carry_sql() {
	let harness = Harness::start().await;
	harness
		.store(vec![
			event("note", "ada", &at(0, 9), json!({"plan": "pro"})),
			event("note", "grace", &at(0, 9), json!({"plan": "free"})),
		])
		.await;
	let notes = |filter: PropertyFilter| {
		Query::Events(EventsQuery {
			range: DateRange::between(at(0, 0), at(1, 0)),
			event: None,
			filters: vec![filter],
			columns: Vec::new(),
			limit: 50,
			cursor: None,
		})
	};
	// A property name carrying SQL reads a property of that name, which
	// nothing has: the name is a bound parameter, never SQL.
	let answer = harness
		.as_app(query::Run {
			query: notes(PropertyFilter {
				source: FilterSource::Event,
				key: "plan') OR 1=1 --".into(),
				operator: Operator::Equals,
				value: Some(json!("anything")),
			}),
		})
		.await
		.unwrap();
	let QueryResult::Events(page) = answer.result else {
		panic!("expected events");
	};
	assert!(page.events.is_empty());

	// The value side is bound too: a value with SQL matches nothing.
	let answer = harness
		.as_app(query::Run {
			query: notes(PropertyFilter::event(
				"plan",
				Operator::Equals,
				json!("pro' OR '1'='1"),
			)),
		})
		.await
		.unwrap();
	let QueryResult::Events(page) = answer.result else {
		panic!("expected events");
	};
	assert_eq!(page.events.len(), 0);

	// And the honest filter still matches exactly one.
	let answer = harness
		.as_app(query::Run {
			query: notes(PropertyFilter::event(
				"plan",
				Operator::Equals,
				json!("pro"),
			)),
		})
		.await
		.unwrap();
	let QueryResult::Events(page) = answer.result else {
		panic!("expected events");
	};
	assert_eq!(page.events.len(), 1);
	assert_eq!(page.events[0].distinct_id, "ada");
}

#[tokio::test(flavor = "multi_thread")]
async fn insights_and_dashboards_round_trip() {
	let harness = Harness::start().await;
	let query = trends_query(DateRange::last_days(7), true, None);
	let saved = harness
		.as_app(query::CreateInsight {
			name: "Sign-ups, daily".into(),
			description: "Who joined this week".into(),
			query: query.clone(),
			created_by: "ada@example.com".into(),
		})
		.await
		.expect("the insight saves");
	assert_eq!(saved.name, "Sign-ups, daily");
	assert_eq!(saved.created_by, "ada@example.com");
	assert_eq!(saved.query, query);

	let listed = harness.as_app(query::Insights {}).await.unwrap();
	assert_eq!(listed.len(), 1);

	let updated = harness
		.as_app(query::UpdateInsight {
			id: saved.id,
			name: Some("Sign-ups".into()),
			description: None,
			query: None,
		})
		.await
		.expect("the insight updates");
	assert_eq!(updated.name, "Sign-ups");
	assert_eq!(updated.description, "Who joined this week");

	let dashboard = harness
		.as_app(query::CreateDashboard {
			name: "Web overview".into(),
			tiles: query::web_overview_tiles(&[saved.id]),
		})
		.await
		.expect("the dashboard saves");
	assert_eq!(dashboard.tiles.len(), 1);
	assert_eq!(dashboard.tiles[0].insight, saved.id);

	let listed = harness.as_app(query::Dashboards {}).await.unwrap();
	assert_eq!(listed.len(), 1);

	harness
		.as_app(query::DeleteDashboard { id: dashboard.id })
		.await
		.expect("the dashboard deletes");
	assert!(
		harness
			.as_app(query::Dashboards {})
			.await
			.unwrap()
			.is_empty()
	);

	harness
		.as_app(query::DeleteInsight { id: saved.id })
		.await
		.expect("the insight deletes");
	assert!(harness.as_app(query::Insights {}).await.unwrap().is_empty());

	// The default web overview is data an app can save as-is.
	let web_overview = query::web_overview_insights();
	assert_eq!(web_overview.len(), 4);
	assert_eq!(
		web_overview[0].0, "Web overview",
		"the first insight is the overview itself"
	);
	let tiles = query::web_overview_tiles(&[Uuid::nil(), Uuid::nil(), Uuid::nil(), Uuid::nil()]);
	assert_eq!(tiles.len(), 4, "one wide overview over three charts");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_writer_bumps_the_tick_once_per_window() {
	let harness = Harness::with_config(IngestConfig {
		refresh_seconds: 1,
		..IngestConfig::default()
	})
	.await;
	harness
		.store(vec![event("note", "ada", &at(0, 9), json!({}))])
		.await;
	let first = harness
		.tick()
		.await
		.expect("the first batch bumps the tick");
	assert_eq!(first.1, 1);

	// A batch inside the window doesn't bump again.
	harness
		.store(vec![event("note", "ada", &at(0, 10), json!({}))])
		.await;
	assert_eq!(
		harness.tick().await,
		Some(first.clone()),
		"no bump inside the window"
	);

	// After the window, the tick moves and carries the events since.
	tokio::time::sleep(std::time::Duration::from_millis(1_200)).await;
	harness
		.store(vec![event("note", "ada", &at(0, 11), json!({}))])
		.await;
	let second = harness.tick().await.expect("the window passed");
	assert_ne!(second.0, first.0, "the tick moved");
	assert_eq!(second.1, 1, "the tick carries this window's events");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_reads_the_tick_so_watchers_rerun() {
	let harness = Harness::start().await;
	harness
		.store(vec![event("signed_up", "ada", &at(0, 9), json!({}))])
		.await;
	let identity = rpc::server::Identity::anonymous().with_caller(APP_CALLER.to_owned());
	let cx = rpc::QueryCx::new(harness.db.clone(), identity);
	let query = trends_query(DateRange::between(at(0, 0), at(1, 0)), false, None);
	query::Run { query }
		.run(&cx)
		.await
		.expect("the trend runs in process");
	let reads = cx.into_reads();
	let tables: Vec<String> = reads.iter().map(|read| read.table.to_string()).collect();
	assert!(
		tables.iter().any(|table| table == "ingest_ticks"),
		"the run reads the tick, so the live layer re-runs it: {tables:?}"
	);
	assert!(
		tables.iter().any(|table| table == "events"),
		"the run names the tables its SQL touches: {tables:?}"
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn only_the_app_runs_queries() {
	let harness = Harness::start().await;
	let identity = rpc::server::Identity::anonymous();
	let cx = rpc::QueryCx::new(harness.db.clone(), identity);
	let error = query::Run {
		query: trends_query(DateRange::last_days(1), false, None),
	}
	.run(&cx)
	.await
	.expect_err("an anonymous caller is refused");
	assert!(error.to_string().contains("only the app"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_tail_holds_what_the_writer_stored() {
	let harness = Harness::start().await;
	harness
		.store(vec![
			event("note", "ada", &at(0, 9), json!({ "n": 1 })),
			event("note", "grace", &at(0, 10), json!({ "n": 2 })),
		])
		.await;
	let (events, latest, missed) = harness.ingest.tail().since(0);
	assert_eq!(events.len(), 2);
	assert_eq!(latest, 2);
	assert!(!missed);
	assert_eq!(events[0].name, "note");
	assert_eq!(events[0].properties["n"], 1);
	assert!(events[0].timestamp.starts_with("2026-11-09T09:00"));
}

#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires LEPTOSKIT_ANALYTICS_TEST_POSTGRES pointing to a private test database"]
async fn postgres_runs_the_same_queries() {
	use leptoskit_analytics::service::POSTGRES_MIGRATIONS;

	let url = std::env::var("LEPTOSKIT_ANALYTICS_TEST_POSTGRES").expect("private Postgres URL");
	let db = Db::connect(&url).await.unwrap();
	db.migrate(POSTGRES_MIGRATIONS).await.unwrap();
	let harness = Harness {
		db: db.clone(),
		ingest: Ingest::start(db, IngestConfig::default()),
	};
	harness
		.store(vec![
			event("signed_up", "pg-ada", &at(0, 9), json!({"plan": "pro"})),
			event("signed_up", "pg-grace", &at(1, 9), json!({"plan": "free"})),
		])
		.await;
	let trends = run_trends(
		&harness,
		trends_query(DateRange::between(at(0, 0), at(2, 0)), true, None),
	)
	.await;
	assert_eq!(bucket_values(&trends.series[0]), [1.0, 1.0, 0.0]);

	// The overview reads sessions the writer upserted, and the pages come
	// from the events' JSON.
	let session = Uuid::from_bytes([11u8; 16]);
	harness
		.store(vec![
			pageview("pg-ada", session, &at(0, 9), "https://app.example/", ""),
			pageview(
				"pg-ada",
				session,
				&at(0, 10),
				"https://app.example/pricing",
				"",
			),
		])
		.await;
	let answer = harness
		.as_app(query::Run {
			query: Query::WebOverview(WebQuery {
				range: DateRange::between(at(0, 0), at(1, 0)),
				compare: false,
				filters: Vec::new(),
				table_limit: 5,
			}),
		})
		.await
		.unwrap();
	let QueryResult::WebOverview(web) = answer.result else {
		panic!("expected the web overview");
	};
	assert_eq!(web.sessions.value, 1.0);
	assert_eq!(web.pageviews.value, 2.0);
	let names: Vec<String> = web
		.top_pages
		.iter()
		.map(|count| count.name.clone())
		.collect();
	assert_eq!(names, ["/", "/pricing"]);
}
