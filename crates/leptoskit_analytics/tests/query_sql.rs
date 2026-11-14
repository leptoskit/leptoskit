//! The SQL each insights query builds, per dialect, snapshot-tested so a
//! change to the planner is reviewed in the open. The planner is a pure
//! function: no database takes part.

#![cfg(not(target_arch = "wasm32"))]

use insta::assert_snapshot;
use leptoskit_analytics::query::*;
use leptoskit_analytics::service::query::plan;
use leptoskit_db::Dialect;

fn now() -> leptoskit_db::Timestamp {
	chrono::DateTime::parse_from_rfc3339("2026-11-14T12:00:00Z")
		.expect("the moment")
		.into()
}

/// Plans a query and prints each statement's SQL, one snapshot per query.
fn sql_of(query: &Query, dialect: Dialect) -> String {
	let planned = plan::plan(query, now(), dialect).expect("the query plans");
	let mut printed = String::new();
	let mut print = |name: &str, sql: &str| {
		printed.push_str("-- ");
		printed.push_str(name);
		printed.push('\n');
		printed.push_str(sql);
		printed.push_str("\n\n");
	};
	match &planned {
		plan::Plan::Trends {
			current, previous, ..
		} => {
			for planned in current {
				print(&planned.name, &planned.sql);
			}
			for planned in previous {
				print(&planned.name, &planned.sql);
			}
		}
		plan::Plan::Funnels {
			pull, strict_pull, ..
		} => {
			print(&pull.name, &pull.sql);
			if let Some(strict) = strict_pull {
				print(&strict.name, &strict.sql);
			}
		}
		plan::Plan::Retention {
			cohort, returns, ..
		} => {
			print(&cohort.name, &cohort.sql);
			print(&returns.name, &returns.sql);
		}
		plan::Plan::WebOverview {
			stats,
			previous_stats,
			pages,
			dimensions,
			..
		} => {
			print(&stats.name, &stats.sql);
			if let Some(previous) = previous_stats {
				print(&previous.name, &previous.sql);
			}
			print(&pages.name, &pages.sql);
			print(&dimensions.name, &dimensions.sql);
		}
		plan::Plan::Events { page } => print(&page.name, &page.sql),
		plan::Plan::Actors(actors) => print(&actors.name, &actors.sql),
	}
	printed
}

fn trends(breakdown: bool, compare: bool, timezone: Option<&str>) -> Query {
	Query::Trends(TrendsQuery {
		range: DateRange::last_days(3).in_timezone(timezone.unwrap_or("UTC")),
		interval: Interval::Day,
		series: vec!["signed_up".into(), "".into()],
		filters: vec![
			PropertyFilter::event("plan", Operator::Equals, serde_json::json!("pro")),
			PropertyFilter {
				source: FilterSource::Person,
				key: "email".into(),
				operator: Operator::IsSet,
				value: None,
			},
			PropertyFilter::session("browser", Operator::Contains, serde_json::json!("Chrome")),
		],
		math: Math::UniquePersons,
		breakdown: breakdown.then(|| Breakdown::event("plan")),
		compare,
		test_accounts: Some(TestAccounts::Exclude(vec!["bot-1".into()])),
	})
}

#[test]
fn trends_sql_on_sqlite() {
	assert_snapshot!(sql_of(&trends(false, false, None), Dialect::Sqlite));
}

#[test]
fn trends_sql_on_postgres() {
	assert_snapshot!(sql_of(&trends(false, false, None), Dialect::Postgres));
}

#[test]
fn a_broken_down_compared_trend_in_a_timezone_on_sqlite() {
	assert_snapshot!(sql_of(&trends(true, true, Some("+02:00")), Dialect::Sqlite));
}

#[test]
fn a_broken_down_compared_trend_in_a_timezone_on_postgres() {
	assert_snapshot!(sql_of(
		&trends(true, true, Some("+02:00")),
		Dialect::Postgres
	));
}

fn funnels(order: FunnelOrder) -> Query {
	Query::Funnels(FunnelsQuery {
		range: DateRange::last_days(7),
		steps: vec!["$pageview".into(), "signed_up".into(), "checkout".into()],
		window_seconds: Some(3 * 86_400),
		order,
		filters: vec![PropertyFilter {
			source: FilterSource::Person,
			key: "plan".into(),
			operator: Operator::NotEquals,
			value: Some(serde_json::json!("free")),
		}],
		breakdown: Some(Breakdown {
			source: FilterSource::Event,
			key: "source".into(),
			limit: 5,
		}),
		test_accounts: None,
	})
}

#[test]
fn an_ordered_funnel_on_sqlite() {
	assert_snapshot!(sql_of(&funnels(FunnelOrder::Ordered), Dialect::Sqlite));
}

#[test]
fn a_strict_funnel_on_postgres() {
	assert_snapshot!(sql_of(&funnels(FunnelOrder::Strict), Dialect::Postgres));
}

fn retention() -> Query {
	Query::Retention(RetentionQuery {
		range: DateRange::last_days(7),
		start_event: "signed_up".into(),
		return_event: "opened".into(),
		kind: RetentionKind::FirstTime,
		period: Interval::Week,
		periods: Some(3),
		filters: Vec::new(),
		test_accounts: None,
	})
}

#[test]
fn retention_sql_on_sqlite() {
	assert_snapshot!(sql_of(&retention(), Dialect::Sqlite));
}

#[test]
fn retention_sql_on_postgres() {
	assert_snapshot!(sql_of(&retention(), Dialect::Postgres));
}

fn web(compare: bool) -> Query {
	Query::WebOverview(WebQuery {
		range: DateRange::last_days(7),
		compare,
		filters: vec![PropertyFilter::session(
			"device_type",
			Operator::Equals,
			serde_json::json!("Desktop"),
		)],
		table_limit: 5,
	})
}

#[test]
fn the_web_overview_on_sqlite() {
	assert_snapshot!(sql_of(&web(true), Dialect::Sqlite));
}

#[test]
fn the_web_overview_on_postgres() {
	assert_snapshot!(sql_of(&web(false), Dialect::Postgres));
}

#[test]
fn the_events_list_on_sqlite() {
	let query = Query::Events(EventsQuery {
		range: DateRange::between("2026-11-01T00:00:00Z", "2026-11-14T00:00:00Z"),
		event: Some("$pageview".into()),
		filters: vec![PropertyFilter::event(
			"$browser",
			Operator::Equals,
			serde_json::json!("Firefox"),
		)],
		columns: vec![EventColumn::Name, EventColumn::DistinctId],
		limit: 50,
		cursor: Some(EventsCursor {
			timestamp: "2026-11-10T09:00:00Z".into(),
			uuid: "0199c0de-0000-7000-8000-000000000001".into(),
		}),
	});
	assert_snapshot!(sql_of(&query, Dialect::Sqlite));
}

#[test]
fn the_events_list_on_postgres() {
	let query = Query::Events(EventsQuery {
		range: DateRange::last_days(7),
		event: None,
		filters: Vec::new(),
		columns: Vec::new(),
		limit: 50,
		cursor: None,
	});
	assert_snapshot!(sql_of(&query, Dialect::Postgres));
}

#[test]
fn actors_sql_on_sqlite() {
	let query = Query::Actors(Box::new(ActorsQuery {
		query: trends(true, false, None),
		point: ActorPoint::Bucket {
			series: 0,
			bucket: "2026-11-12T00:00:00".into(),
			breakdown_value: Some(serde_json::json!("pro")),
		},
		limit: 50,
		offset: 0,
	}));
	assert_snapshot!(sql_of(&query, Dialect::Sqlite));
}

#[test]
fn actors_sql_on_postgres() {
	let query = Query::Actors(Box::new(ActorsQuery {
		query: retention(),
		point: ActorPoint::RetentionCell {
			start_period: 0,
			return_period: 1,
		},
		limit: 50,
		offset: 0,
	}));
	assert_snapshot!(sql_of(&query, Dialect::Postgres));
}

#[test]
fn a_property_name_carrying_sql_reads_a_property_of_that_name() {
	let query = Query::Events(EventsQuery {
		range: DateRange::last_days(1),
		event: None,
		filters: vec![PropertyFilter {
			source: FilterSource::Event,
			key: "x') OR 1=1 --".into(),
			operator: Operator::Equals,
			value: Some(serde_json::json!("injected")),
		}],
		columns: Vec::new(),
		limit: 50,
		cursor: None,
	});
	let planned = plan::plan(&query, now(), Dialect::Sqlite).expect("the name binds");
	let plan::Plan::Events { page } = &planned else {
		panic!("the plan is an events page");
	};
	assert_snapshot!(page.sql.clone());
	// Every value is bound: the SQL carries the property name nowhere.
	assert!(!page.sql.contains("OR 1=1"));
}

#[test]
fn the_planner_refuses_what_it_cannot_answer() {
	let refused = |query: Query| {
		plan::plan(&query, now(), Dialect::Sqlite)
			.err()
			.map(|error| error.to_string())
	};
	assert_eq!(
		refused(Query::Trends(TrendsQuery {
			range: DateRange::last_days(1),
			interval: Interval::Day,
			series: Vec::new(),
			filters: Vec::new(),
			math: Math::Total,
			breakdown: None,
			compare: false,
			test_accounts: None,
		})),
		Some("the range doesn't hold together: a trends query names at least one series".into())
	);
	assert_eq!(
		refused(Query::Funnels(FunnelsQuery {
			range: DateRange::last_days(1),
			steps: vec!["one".into()],
			window_seconds: None,
			order: FunnelOrder::Ordered,
			filters: Vec::new(),
			breakdown: None,
			test_accounts: None,
		})),
		Some("the range doesn't hold together: a funnel has at least two steps".into())
	);
	assert_eq!(
		refused(Query::Trends(TrendsQuery {
			range: DateRange {
				from: None,
				to: None,
				last_days: Some(1),
				timezone: Some("Europe/Berlin".into()),
			},
			interval: Interval::Day,
			series: vec!["x".into()],
			filters: Vec::new(),
			math: Math::Total,
			breakdown: None,
			compare: false,
			test_accounts: None,
		}))
		.map(|error| error.starts_with("not supported: timezone")),
		Some(true)
	);
	assert_eq!(
		refused(Query::Trends(TrendsQuery {
			range: DateRange::last_days(1),
			interval: Interval::Day,
			series: vec!["x".into()],
			filters: vec![PropertyFilter {
				source: FilterSource::Event,
				key: String::new(),
				operator: Operator::Equals,
				value: Some(serde_json::json!("")),
			}],
			math: Math::Total,
			breakdown: None,
			compare: false,
			test_accounts: None,
		})),
		Some("the property can't be read: a property's name can't be empty".into())
	);
	assert_eq!(
		refused(Query::Trends(TrendsQuery {
			range: DateRange::last_days(1),
			interval: Interval::Day,
			series: vec!["x".into()],
			filters: vec![PropertyFilter {
				source: FilterSource::Session,
				key: "person_id; DROP TABLE sessions".into(),
				operator: Operator::Equals,
				value: Some(serde_json::json!("")),
			}],
			math: Math::Total,
			breakdown: None,
			compare: false,
			test_accounts: None,
		}))
		.map(|error| error.starts_with("the session column can't be filtered")),
		Some(true)
	);
}
