//! The insights engine (§10): the `run` operation turns a [`Query`] into
//! SQL per dialect, answers with series, steps, grids, and pages beside
//! [`QueryInfo`] — the SQL it ran and its timing, for the Workings drawer
//! — and re-runs as events arrive, because every query reads the writer's
//! one `ingest_ticks` row.
//!
//! The same module holds the saved insights and dashboards (§10), and the
//! `live_events` stream (§11, minimal) over the writer's tail ring.
//!
//! Every operation here is app-only, like the person reads: the admin app
//! reaches them through its app's server, which presents the service's
//! token and has already checked the acting person's roles (§18).

pub mod dialect;
pub mod plan;

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::time::Instant;

use leptoskit_db::Dialect;
use leptoskit_db::Read as _;
use leptoskit_db::Statement;
use leptoskit_db::Tracked;
use leptoskit_db::Value;
use leptoskit_rpc as rpc;
use leptoskit_rpc::server::Identity;
use plan::Plan;
use plan::Statement as Planned;
use serde::Deserialize;
use serde::Serialize;
use uuid::Uuid;

use crate::query::Actor;
use crate::query::ActorPoint as Point;
use crate::query::ActorsPage;
use crate::query::Answer;
use crate::query::EventRow;
use crate::query::EventsCursor;
use crate::query::EventsPage;
use crate::query::FunnelBreakdown;
use crate::query::FunnelOrder;
use crate::query::FunnelStepResult;
use crate::query::FunnelsResult;
use crate::query::Metric;
use crate::query::NamedCount;
use crate::query::QueryInfo;
use crate::query::QueryResult;
use crate::query::RetentionCohort;
use crate::query::RetentionKind;
use crate::query::RetentionResult;
use crate::query::StatementInfo;
use crate::query::TrendBucket;
use crate::query::TrendSeries;
use crate::query::TrendsResult;
use crate::query::WebOverviewResult;
use crate::service::tables;

/// A person key from a pulled row: the resolved person's UUID, or, for
/// events without one, the distinct id. Either way it sorts, so paging is
/// stable.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum PersonKey {
	Person(Uuid),
	Distinct(String),
}

impl PersonKey {
	/// From a pulled row's value.
	fn of(value: Option<&Value>) -> Option<Self> {
		match value? {
			Value::Uuid(uuid) => Some(Self::Person(*uuid)),
			Value::Bytes(bytes) => Uuid::from_slice(bytes).ok().map(Self::Person),
			Value::Text(text) => Some(Self::Distinct(text.clone())),
			_ => None,
		}
	}

	/// The person's UUID, when this is one: anonymous distinct ids have no
	/// person to show, so actors pages skip them.
	fn uuid(&self) -> Option<Uuid> {
		match self {
			Self::Person(uuid) => Some(*uuid),
			Self::Distinct(_) => None,
		}
	}
}

/// Runs a query, answering its result beside the SQL it ran and its
/// timing.
///
/// # Errors
///
/// [`rpc::Error`] when the caller isn't the app, the query can't be
/// planned, or a statement fails or takes over the timeout.
#[rpc::query(service = "analytics")]
pub async fn run(cx: &rpc::QueryCx, query: crate::query::Query) -> rpc::Result<Answer> {
	app_only(cx.identity())?;
	let db = cx.db();
	let dialect = db.dialect();

	// The one tick every insight reads, so the live layer re-runs this
	// query at most once per refresh window (§10 "Live, without
	// melting").
	read_tick(db, dialect).await?;

	let started = Instant::now();
	let planned =
		plan::plan(&query, leptoskit_db::now(), dialect).map_err(|error| plan_error(&error))?;
	let mut work = Work {
		info: QueryInfo {
			dialect: dialect_name(dialect),
			..QueryInfo::default()
		},
		rows_read: 0,
	};

	let result = match (&query, &planned) {
		(
			crate::query::Query::Trends(trends),
			Plan::Trends {
				current,
				previous,
				starts,
				previous_starts,
				math: _,
			},
		) => {
			let mut series = Vec::new();
			for (trend_series, statement) in trends.series.iter().zip(current) {
				let rows = run_statement(db, statement, &mut work).await?;
				series.extend(fold_series(
					&rows,
					starts,
					&trend_series.event,
					trends.breakdown.as_ref(),
				));
			}
			let mut previous_series = Vec::new();
			for (trend_series, statement) in trends.series.iter().zip(previous) {
				let rows = run_statement(db, statement, &mut work).await?;
				previous_series.extend(fold_series(
					&rows,
					previous_starts,
					&trend_series.event,
					trends.breakdown.as_ref(),
				));
			}
			QueryResult::Trends(TrendsResult {
				series,
				previous: previous_series,
			})
		}
		(
			crate::query::Query::Funnels(funnels),
			Plan::Funnels {
				pull,
				strict_pull,
				steps,
				window_seconds,
				order,
			},
		) => {
			let rows = run_statement(db, pull, &mut work).await?;
			let strict_rows = match strict_pull {
				Some(strict) => Some(run_statement(db, strict, &mut work).await?),
				None => None,
			};
			QueryResult::Funnels(fold_funnel(
				funnels,
				steps,
				*window_seconds,
				*order,
				&rows,
				strict_rows.as_deref(),
			))
		}
		(
			crate::query::Query::Retention(_),
			Plan::Retention {
				cohort,
				returns,
				period,
				kind,
				periods,
				period_starts,
			},
		) => {
			let cohort_rows = run_statement(db, cohort, &mut work).await?;
			let return_rows = run_statement(db, returns, &mut work).await?;
			QueryResult::Retention(fold_retention(
				*period,
				*kind,
				*periods,
				period_starts,
				&cohort_rows,
				&return_rows,
			))
		}
		(
			crate::query::Query::WebOverview(web),
			Plan::WebOverview {
				stats,
				previous_stats,
				pages,
				dimensions,
				table_limit,
			},
		) => {
			let stats_rows = run_statement(db, stats, &mut work).await?;
			let previous_rows = match previous_stats {
				Some(previous) => Some(run_statement(db, previous, &mut work).await?),
				None => None,
			};
			let pages_rows = run_statement(db, pages, &mut work).await?;
			let dimension_rows = run_statement(db, dimensions, &mut work).await?;
			QueryResult::WebOverview(Box::new(fold_web(
				web,
				*table_limit,
				&stats_rows,
				previous_rows.as_deref(),
				&pages_rows,
				&dimension_rows,
			)))
		}
		(crate::query::Query::Events(events), Plan::Events { page }) => {
			let rows = run_statement(db, page, &mut work).await?;
			QueryResult::Events(fold_events(events, &rows))
		}
		(crate::query::Query::Actors(actors), plan) => {
			let people = actors_people(db, actors, plan, &mut work).await?;
			let page = if people.is_empty() {
				ActorsPage::default()
			} else {
				persons_page(db, people, actors.limit, actors.offset).await?
			};
			QueryResult::Actors(page)
		}
		_ => {
			return Err(rpc::Error::server("the plan and the query disagree"));
		}
	};

	let info = work.finish(started);
	Ok(Answer { info, result })
}

/// What a run has read so far: each statement's SQL and timing, and how
/// many rows came back.
struct Work {
	info: QueryInfo,
	rows_read: u64,
}

impl Work {
	/// The run's cost, once every statement came back.
	fn finish(self, started: Instant) -> QueryInfo {
		QueryInfo {
			statements: self.info.statements,
			dialect: self.info.dialect,
			elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
			rows_scanned_hint: Some(self.rows_read),
		}
	}
}

/// One bound parameter, as the Workings drawer shows it.
fn arg_info(value: &Value) -> serde_json::Value {
	match value {
		Value::Null => serde_json::Value::Null,
		Value::Bool(bool) => serde_json::Value::Bool(*bool),
		Value::Int(int) => serde_json::Value::from(*int),
		Value::Float(float) => serde_json::Value::from(*float),
		Value::Text(text) => serde_json::Value::from(text.clone()),
		Value::Bytes(bytes) => serde_json::Value::from(format!("{bytes:?}")),
		Value::Uuid(uuid) => serde_json::Value::from(uuid.to_string()),
		Value::Timestamp(at) => serde_json::Value::from(super::persons::stamp(*at)),
		other => serde_json::Value::from(format!("{other:?}")),
	}
}

/// Runs one planned statement on the tracked connection, naming every
/// table its SQL may touch so the live layer can re-run the query.
async fn run_read(
	db: &Tracked<leptoskit_db::Db>,
	statement: &Planned,
) -> rpc::Result<(Vec<Vec<Value>>, u64)> {
	let read = Statement::new(statement.sql.clone(), statement.args.clone())
		.reads::<tables::Event>()
		.reads::<tables::Person>()
		.reads::<tables::PersonOverride>()
		.reads::<tables::PersonDistinctId>()
		.reads::<tables::Session>();
	let started = Instant::now();
	let rows = tokio::time::timeout(plan::QUERY_TIMEOUT, read.fetch(db))
		.await
		.map_err(|_| rpc::Error::server("the query took over its timeout"))?
		.map_err(|error| rpc::Error::server(error.to_string()))?;
	Ok((
		rows,
		u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
	))
}

/// A planned statement, run and recorded: what it read, and when it ran.
async fn run_statement(
	db: &Tracked<leptoskit_db::Db>,
	statement: &Planned,
	work: &mut Work,
) -> rpc::Result<Vec<Vec<Value>>> {
	let (rows, elapsed_ms) = run_read(db, statement).await?;
	work.info.statements.push(StatementInfo {
		name: statement.name.clone(),
		sql: statement.sql.clone(),
		args: statement.args.iter().map(arg_info).collect(),
	});
	work.rows_read += rows.len() as u64;
	let _ = elapsed_ms;
	Ok(rows)
}

/// The one `ingest_ticks` read every insight query makes, so a watched
/// insight re-runs when the writer bumps the tick.
async fn read_tick(db: &Tracked<leptoskit_db::Db>, dialect: Dialect) -> rpc::Result<()> {
	let statement = Statement::new(
		format!(
			"SELECT \"at\", \"events\" FROM \"ingest_ticks\" WHERE \"id\" = {}",
			placeholder(dialect, 1)
		),
		vec![Value::Int(1)],
	);
	let read = statement.reads::<tables::IngestTick>();
	tokio::time::timeout(plan::QUERY_TIMEOUT, read.fetch(db))
		.await
		.map_err(|_| rpc::Error::server("the query took over its timeout"))?
		.map_err(|error| rpc::Error::server(error.to_string()))?;
	Ok(())
}

/// The `n`th placeholder.
fn placeholder(dialect: Dialect, n: usize) -> String {
	match dialect {
		Dialect::Postgres => format!("${n}"),
		Dialect::Sqlite => format!("?{n}"),
	}
}

/// The dialect's name, as the answer carries it.
fn dialect_name(dialect: Dialect) -> String {
	match dialect {
		Dialect::Postgres => "postgres".to_owned(),
		Dialect::Sqlite => "sqlite".to_owned(),
	}
}

/// A plan error, as an rpc error.
fn plan_error(error: &plan::PlanError) -> rpc::Error {
	rpc::Error::invalid(error.to_string())
}

/// A `Value` that holds an instant, either as written or as SQLite's text.
fn timestamp_of(value: Option<&Value>) -> Option<leptoskit_db::Timestamp> {
	match value? {
		Value::Timestamp(at) => Some(*at),
		Value::Text(text) => super::persons::parse_stamp(text),
		_ => None,
	}
}

/// A `Value` that holds a UUID, as either dialect returns one.
fn uuid_of(value: Option<&Value>) -> Option<Uuid> {
	match value? {
		Value::Uuid(uuid) => Some(*uuid),
		Value::Bytes(bytes) => Uuid::from_slice(bytes).ok(),
		_ => None,
	}
}

/// A `Value` that holds text.
fn text_of(value: Option<&Value>) -> Option<String> {
	match value? {
		Value::Text(text) => Some(text.clone()),
		Value::Uuid(uuid) => Some(uuid.to_string()),
		Value::Bytes(bytes) => Some(format!("{bytes:?}")),
		_ => None,
	}
}

/// A `Value` that holds a number.
fn float_of(value: Option<&Value>) -> f64 {
	match value {
		Some(Value::Float(float)) => *float,
		Some(Value::Int(int)) => *int as f64,
		_ => 0.0,
	}
}

/// Folds one trend statement's rows into its series: one series when the
/// statement isn't broken down, one per top group plus `other` when it is.
/// Buckets fill from `starts`, so a chart has no holes.
fn fold_series(
	rows: &[Vec<Value>],
	starts: &[String],
	event: &str,
	breakdown: Option<&crate::query::Breakdown>,
) -> Vec<TrendSeries> {
	let mut values: BTreeMap<(String, String), f64> = BTreeMap::new();
	for row in rows {
		let Some(bucket) = text_of(row.first()) else {
			continue;
		};
		let group = text_of(row.get(1)).unwrap_or_default();
		let value = float_of(row.get(if breakdown.is_some() { 2 } else { 1 }));
		*values.entry((bucket, group)).or_default() += value;
	}
	let Some(breakdown) = breakdown else {
		let buckets = fill(
			values
				.into_iter()
				.map(|((bucket, _), value)| (bucket, value)),
			starts,
		);
		let total = buckets.iter().map(|bucket| bucket.value).sum();
		return vec![TrendSeries {
			event: event.to_owned(),
			breakdown_value: None,
			buckets,
			total,
		}];
	};
	// The top groups, by their totals, and the rest as `other`.
	let limit = usize::try_from(breakdown.limit.max(1)).unwrap_or(usize::MAX);
	let mut totals: BTreeMap<String, f64> = BTreeMap::new();
	for ((_, group), value) in &values {
		*totals.entry(group.clone()).or_default() += *value;
	}
	let mut ranked: Vec<(String, f64)> = totals.into_iter().collect();
	ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
	let top: BTreeSet<String> = ranked
		.iter()
		.take(limit)
		.map(|(group, _)| group.clone())
		.collect();
	let mut kept: BTreeMap<String, Vec<(String, f64)>> = BTreeMap::new();
	for ((bucket, group), value) in values {
		let group = if top.contains(&group) {
			group
		} else {
			"other".to_owned()
		};
		kept.entry(group).or_default().push((bucket, value));
	}
	// One series per kept group, ranked groups first, `other` last.
	let mut order: Vec<String> = ranked
		.iter()
		.take(limit)
		.map(|(group, _)| group.clone())
		.collect();
	if kept.contains_key("other") && ranked.len() > limit && !order.contains(&"other".to_owned()) {
		order.push("other".to_owned());
	}
	order
		.into_iter()
		.filter_map(|group| {
			let points = kept.get(&group)?;
			let buckets = fill(points.iter().cloned(), starts);
			let total = buckets.iter().map(|bucket| bucket.value).sum();
			Some(TrendSeries {
				event: event.to_owned(),
				breakdown_value: if group == "other" {
					None
				} else {
					Some(serde_json::Value::from(group))
				},
				buckets,
				total,
			})
		})
		.collect()
}

/// One series' buckets, oldest first, gaps zero.
fn fill(points: impl Iterator<Item = (String, f64)>, starts: &[String]) -> Vec<TrendBucket> {
	let by_bucket: BTreeMap<String, f64> = points.collect();
	starts
		.iter()
		.map(|start| {
			TrendBucket {
				start: start.clone(),
				value: by_bucket.get(start).copied().unwrap_or_default(),
			}
		})
		.collect()
}

/// What one person's events say about a funnel: which steps they reached,
/// when they entered, and their first step's breakdown group.
struct Walker {
	entry: leptoskit_db::Timestamp,
	reached: usize,
	group: Option<String>,
	finished_at: Option<leptoskit_db::Timestamp>,
}

/// Walks the funnel's pulled rows, one person at a time, in time order,
/// and answers each person's walk.
fn walk_funnel(
	steps: &[String],
	window_seconds: u64,
	order: FunnelOrder,
	rows: &[Vec<Value>],
	strict_rows: Option<&[Vec<Value>]>,
) -> BTreeMap<PersonKey, Walker> {
	let mut by_person: BTreeMap<PersonKey, Vec<(String, leptoskit_db::Timestamp, Option<String>)>> =
		BTreeMap::new();
	for row in rows {
		let Some(person) = PersonKey::of(row.first()) else {
			continue;
		};
		let Some(name) = text_of(row.get(1)) else {
			continue;
		};
		let Some(at) = timestamp_of(row.get(2)) else {
			continue;
		};
		let group = text_of(row.get(3));
		by_person.entry(person).or_default().push((name, at, group));
	}
	let source = if order == FunnelOrder::Strict {
		strict_rows.unwrap_or(rows)
	} else {
		rows
	};
	if order == FunnelOrder::Strict {
		let mut strict: BTreeMap<
			PersonKey,
			Vec<(String, leptoskit_db::Timestamp, Option<String>)>,
		> = BTreeMap::new();
		for row in source {
			let Some(person) = PersonKey::of(row.first()) else {
				continue;
			};
			let Some(name) = text_of(row.get(1)) else {
				continue;
			};
			let Some(at) = timestamp_of(row.get(2)) else {
				continue;
			};
			let group = text_of(row.get(3));
			strict.entry(person).or_default().push((name, at, group));
		}
		return strict
			.into_iter()
			.filter_map(|(person, events)| {
				walk_person(&person, events, steps, window_seconds, order)
					.map(|walker| (person, walker))
			})
			.collect();
	}
	by_person
		.into_iter()
		.filter_map(|(person, events)| {
			walk_person(&person, events, steps, window_seconds, order)
				.map(|walker| (person, walker))
		})
		.collect()
}

/// One person's walk: how far they got, and when.
fn walk_person(
	_person: &PersonKey,
	mut events: Vec<(String, leptoskit_db::Timestamp, Option<String>)>,
	steps: &[String],
	window_seconds: u64,
	order: FunnelOrder,
) -> Option<Walker> {
	events.sort_by_key(|a| a.1);
	let entry = events
		.iter()
		.find(|(name, ..)| name == &steps[0])
		.map(|(_, at, group)| (*at, group.clone()))?;
	let window = entry.0 + chrono::Duration::seconds(window_seconds as i64);
	let mut reached = 1usize;
	let mut last = entry.0;
	if order == FunnelOrder::Strict {
		// The next step must be this person's very next event.
		let entry_index = events
			.iter()
			.position(|(name, ..)| name == &steps[0])
			.unwrap_or(0);
		for (offset, step) in steps.iter().skip(1).enumerate() {
			let index = entry_index + 1 + offset;
			match events.get(index) {
				Some((name, at, _)) if name == step && *at <= window => {
					reached += 1;
					last = *at;
				}
				_ => break,
			}
		}
	} else if order == FunnelOrder::Unordered {
		let within: BTreeSet<&String> = events
			.iter()
			.filter(|(_, at, _)| *at >= entry.0 && *at <= window)
			.map(|(name, ..)| name)
			.collect();
		for step in steps.iter().skip(1) {
			if within.contains(step) {
				reached += 1;
			} else {
				break;
			}
		}
		if reached == steps.len() {
			last = events
				.iter()
				.filter(|(_, at, _)| *at >= entry.0 && *at <= window)
				.map(|(_, at, _)| *at)
				.max()
				.unwrap_or(last);
		}
	} else {
		for (offset, step) in steps.iter().skip(1).enumerate() {
			let index = 1 + offset;
			let found = events[index..]
				.iter()
				.find(|(name, at, _)| name == step && *at >= last && *at <= window)
				.map(|(_, at, _)| *at);
			match found {
				Some(at) => {
					reached += 1;
					last = at;
				}
				None => break,
			}
		}
	}
	Some(Walker {
		entry: entry.0,
		reached,
		group: entry.1,
		finished_at: (reached == steps.len()).then_some(last),
	})
}

/// Folds a funnel query's answer from its pulled rows.
fn fold_funnel(
	funnels: &crate::query::FunnelsQuery,
	steps: &[String],
	window_seconds: u64,
	order: FunnelOrder,
	rows: &[Vec<Value>],
	strict_rows: Option<&[Vec<Value>]>,
) -> FunnelsResult {
	let walks = walk_funnel(steps, window_seconds, order, rows, strict_rows);
	let entered = walks.len() as u64;
	let mut step_counts = vec![0u64; steps.len()];
	let mut seconds: Vec<f64> = Vec::new();
	for walker in walks.values() {
		for (index, count) in step_counts.iter_mut().enumerate() {
			if walker.reached > index {
				*count += 1;
			}
		}
		if let Some(finished) = walker.finished_at {
			seconds.push((finished - walker.entry).num_milliseconds() as f64 / 1_000.0);
		}
	}
	let rate = |count: u64| {
		if entered == 0 {
			0.0
		} else {
			count as f64 / entered as f64
		}
	};
	let steps_result: Vec<FunnelStepResult> = steps
		.iter()
		.enumerate()
		.map(|(index, name)| {
			let count = step_counts[index];
			let before = if index == 0 {
				entered
			} else {
				step_counts[index - 1]
			};
			FunnelStepResult {
				name: name.clone(),
				count,
				conversion_rate: rate(count),
				drop_off: before.saturating_sub(count),
			}
		})
		.collect();
	let mut breakdown = Vec::new();
	if let Some(breakdown_by) = funnels.breakdown.as_ref() {
		let limit = breakdown_by.limit.max(1);
		let mut groups: BTreeMap<String, Vec<&Walker>> = BTreeMap::new();
		for walker in walks.values() {
			let group = walker.group.clone().unwrap_or_default();
			groups.entry(group).or_default().push(walker);
		}
		let mut ranked: Vec<(String, usize)> = groups
			.iter()
			.map(|(group, walkers)| (group.clone(), walkers.len()))
			.collect();
		ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
		let top: BTreeSet<String> = ranked
			.iter()
			.take(usize::try_from(limit).unwrap_or(usize::MAX))
			.map(|(group, _)| group.clone())
			.collect();
		for (group, walkers) in &groups {
			if !top.contains(group) && ranked.len() > usize::try_from(limit).unwrap_or(usize::MAX) {
				continue;
			}
			let label = if group.is_empty()
				&& ranked.len() > usize::try_from(limit).unwrap_or(usize::MAX)
			{
				"other".to_owned()
			} else {
				group.clone()
			};
			let group_counts = fold_funnel_group(walkers.iter().copied(), steps.len());
			let group_entered = group_counts.first().copied().unwrap_or(0);
			let group_steps: Vec<FunnelStepResult> = steps
				.iter()
				.enumerate()
				.map(|(index, name)| {
					let count = group_counts[index];
					let before = if index == 0 {
						group_entered
					} else {
						group_counts[index - 1]
					};
					FunnelStepResult {
						name: name.clone(),
						count,
						conversion_rate: if group_entered == 0 {
							0.0
						} else {
							count as f64 / group_entered as f64
						},
						drop_off: before.saturating_sub(count),
					}
				})
				.collect();
			breakdown.push(FunnelBreakdown {
				value: serde_json::Value::from(label),
				steps: group_steps,
			});
		}
	}
	FunnelsResult {
		overall_conversion: rate(*step_counts.last().unwrap_or(&0)),
		median_seconds_to_convert: median(&mut seconds),
		steps: steps_result,
		breakdown,
	}
}

/// Per-step counts of one breakdown group's walkers.
fn fold_funnel_group<'a>(walkers: impl Iterator<Item = &'a Walker>, step_count: usize) -> Vec<u64> {
	let mut counts = vec![0u64; step_count];
	for walker in walkers {
		for (index, count) in counts.iter_mut().enumerate() {
			if walker.reached > index {
				*count += 1;
			}
		}
	}
	counts
}

/// The median of a list, taking the mean of the middle two on an even
/// count, and sorting the slice in place.
fn median(values: &mut [f64]) -> Option<f64> {
	if values.is_empty() {
		return None;
	}
	values.sort_by(f64::total_cmp);
	Some(if values.len() % 2 == 1 {
		values[values.len() / 2]
	} else {
		f64::midpoint(values[values.len() / 2 - 1], values[values.len() / 2])
	})
}

/// Each cohort's members, by period index: everyone who did the start
/// event in that period, or, for first-time retention, only those for whom
/// it was the first period they ever did it. The rows are the cohort
/// statement's `(person, period)` pairs, history included.
fn cohort_members(
	kind: RetentionKind,
	period_starts: &[String],
	cohort_rows: &[Vec<Value>],
) -> Vec<BTreeSet<PersonKey>> {
	let mut started: BTreeMap<PersonKey, BTreeSet<String>> = BTreeMap::new();
	for row in cohort_rows {
		if let (Some(person), Some(label)) = (PersonKey::of(row.first()), text_of(row.get(1))) {
			started.entry(person).or_default().insert(label);
		}
	}
	let mut members = vec![BTreeSet::new(); period_starts.len()];
	for (person, labels) in started {
		// Labels are wall-clock text, so they sort as time does.
		let joined: Vec<&String> = match kind {
			RetentionKind::FirstTime => labels.first().into_iter().collect(),
			RetentionKind::Recurring => labels.iter().collect(),
		};
		for label in joined {
			if let Some(index) = period_starts.iter().position(|start| start == label) {
				members[index].insert(person.clone());
			}
		}
	}
	members
}

/// The periods, by index, each person did the return event in.
fn return_periods(
	period_starts: &[String],
	return_rows: &[Vec<Value>],
) -> BTreeMap<PersonKey, BTreeSet<usize>> {
	let mut returned: BTreeMap<PersonKey, BTreeSet<usize>> = BTreeMap::new();
	for row in return_rows {
		if let (Some(person), Some(label)) = (PersonKey::of(row.first()), text_of(row.get(1)))
			&& let Some(index) = period_starts.iter().position(|start| *start == label)
		{
			returned.entry(person).or_default().insert(index);
		}
	}
	returned
}

/// Folds a retention query's answer from its cohort and returns rows: one
/// cohort per period the range holds, each followed for `periods` periods
/// or to the range's end, whichever comes first.
fn fold_retention(
	period: crate::query::Interval,
	kind: RetentionKind,
	periods: u32,
	period_starts: &[String],
	cohort_rows: &[Vec<Value>],
	return_rows: &[Vec<Value>],
) -> RetentionResult {
	let members = cohort_members(kind, period_starts, cohort_rows);
	let returned = return_periods(period_starts, return_rows);
	let follow = usize::try_from(periods)
		.unwrap_or(usize::MAX)
		.saturating_add(1);
	let cohorts = members
		.iter()
		.enumerate()
		.map(|(index, people)| {
			let length = follow.min(period_starts.len() - index);
			let mut returns = vec![0u64; length];
			for periods in people.iter().filter_map(|person| returned.get(person)) {
				for (offset, count) in returns.iter_mut().enumerate() {
					if periods.contains(&(index + offset)) {
						*count += 1;
					}
				}
			}
			RetentionCohort {
				start: period_starts[index].clone(),
				size: people.len() as u64,
				returns,
			}
		})
		.collect();
	RetentionResult {
		period,
		kind,
		cohorts,
	}
}

/// One dimension's setter: where its counts land in the answer.
type WebTable = (&'static str, fn(&mut WebOverviewResult, Vec<NamedCount>));

/// The web overview's dimensions, as the answer's tables name them.
const WEB_DIMENSIONS: &[WebTable] = &[
	("entry_pathname", |web, counts| web.entry_pages = counts),
	("exit_pathname", |web, counts| web.exit_pages = counts),
	("referrer_domain", |web, counts| web.referrers = counts),
	("channel", |web, counts| web.channels = counts),
	("utm_source", |web, counts| web.utm_sources = counts),
	("utm_campaign", |web, counts| web.utm_campaigns = counts),
	("device_type", |web, counts| web.devices = counts),
	("browser", |web, counts| web.browsers = counts),
	("os", |web, counts| web.operating_systems = counts),
];

/// Folds the web overview's answer from its sessions, pages, and tables.
fn fold_web(
	web: &crate::query::WebQuery,
	table_limit: u32,
	stats_rows: &[Vec<Value>],
	previous_rows: Option<&[Vec<Value>]>,
	pages_rows: &[Vec<Value>],
	dimension_rows: &[Vec<Value>],
) -> WebOverviewResult {
	let _ = web;
	let stats = stats_rows.first();
	let previous = previous_rows.and_then(|rows| rows.first());
	let metric = |index: usize, divide: f64| {
		let value = float_of(stats.and_then(|row| row.get(index))) / divide;
		let previous = previous
			.and_then(|row| row.get(index))
			.map(|value| float_of(Some(value)) / divide);
		Metric {
			value,
			previous,
			delta: delta(value, previous),
		}
	};
	let mut result = WebOverviewResult {
		visitors: metric(2, 1.0),
		pageviews: metric(1, 1.0),
		sessions: metric(0, 1.0),
		bounce_rate: metric(3, float_of(stats.and_then(|row| row.first())).max(1.0)),
		avg_session_duration_seconds: metric(
			4,
			float_of(stats.and_then(|row| row.first())).max(1.0) * 1_000.0,
		),
		..WebOverviewResult::default()
	};
	result.top_pages = counts_of(pages_rows, table_limit, |row| {
		NamedCount {
			name: text_of(row.first()).unwrap_or_default(),
			count: float_of(row.get(1)) as u64,
		}
	});
	for (dimension, setter) in WEB_DIMENSIONS {
		let counts: Vec<NamedCount> = dimension_rows
			.iter()
			.filter(|row| text_of(row.first()).as_deref() == Some(dimension))
			.take(usize::try_from(table_limit).unwrap_or(usize::MAX))
			.map(|row| {
				NamedCount {
					name: text_of(row.get(1)).unwrap_or_default(),
					count: float_of(row.get(2)) as u64,
				}
			})
			.collect();
		setter(&mut result, counts);
	}
	result
}

/// A value against its previous, as a relative change: none when there's
/// no previous, from zero when there was none before.
fn delta(value: f64, previous: Option<f64>) -> Option<f64> {
	previous.map(|previous| {
		if previous == 0.0 {
			if value == 0.0 { 0.0 } else { 1.0 }
		} else {
			(value - previous) / previous
		}
	})
}

/// A table's counts, already ordered by the SQL, cut to its limit.
fn counts_of(
	rows: &[Vec<Value>],
	limit: u32,
	count: impl Fn(&Vec<Value>) -> NamedCount,
) -> Vec<NamedCount> {
	rows.iter()
		.take(usize::try_from(limit).unwrap_or(usize::MAX))
		.map(count)
		.collect()
}

/// Folds one page of the events list. The rows carry the timestamp and
/// uuid first — the order and the cursor read them — then the columns the
/// query asked to show.
fn fold_events(query: &crate::query::EventsQuery, rows: &[Vec<Value>]) -> EventsPage {
	let limit = query.limit.clamp(1, crate::query::EventsQuery::MAX_LIMIT) as usize;
	let page_rows = rows.get(..limit).unwrap_or(rows);
	let columns = if query.columns.is_empty() {
		plan::EVENT_COLUMNS
	} else {
		query.columns.as_slice()
	};
	// The uuid and the timestamp ride first for the order and the cursor;
	// the page's own list of columns follows, without them.
	let shown: Vec<&crate::query::EventColumn> = columns
		.iter()
		.filter(|column| {
			!matches!(
				column,
				crate::query::EventColumn::Uuid | crate::query::EventColumn::Timestamp
			)
		})
		.collect();
	let events: Vec<EventRow> = page_rows
		.iter()
		.filter_map(|row| {
			let at = timestamp_of(row.first())?;
			let uuid = match row.get(1) {
				Some(Value::Uuid(id)) => id.to_string(),
				Some(Value::Bytes(bytes)) => {
					Uuid::from_slice(bytes)
						.map(|id| id.to_string())
						.unwrap_or_default()
				}
				Some(Value::Text(text)) => text.clone(),
				_ => String::new(),
			};
			let mut event = EventRow {
				uuid,
				name: String::new(),
				distinct_id: String::new(),
				person_id: None,
				timestamp: super::persons::stamp(at),
				session_id: None,
				properties: serde_json::Value::Null,
				source: None,
			};
			for (index, column) in shown.iter().enumerate() {
				let value = row.get(index + 2);
				match column {
					crate::query::EventColumn::Name => {
						event.name = text_of(value).unwrap_or_default();
					}
					crate::query::EventColumn::DistinctId => {
						event.distinct_id = text_of(value).unwrap_or_default();
					}
					crate::query::EventColumn::PersonId => event.person_id = text_of(value),
					crate::query::EventColumn::SessionId => {
						event.session_id = value
							.and_then(|value| uuid_of(Some(value)))
							.map(|uuid| uuid.to_string());
					}
					crate::query::EventColumn::Properties => {
						event.properties = match value {
							Some(Value::Text(text)) => {
								serde_json::from_str(text).unwrap_or_default()
							}
							Some(Value::Json(json)) => json.clone(),
							_ => serde_json::Value::Null,
						}
					}
					crate::query::EventColumn::Source => {
						event.source = match value {
							Some(Value::Int(int)) => Some(*int),
							_ => None,
						}
					}
					// The timestamp and the uuid came first.
					crate::query::EventColumn::Uuid | crate::query::EventColumn::Timestamp => {}
				}
			}
			Some(event)
		})
		.collect();
	let next = (rows.len() > limit && !page_rows.is_empty())
		.then(|| {
			// The cursor is the last row of this page, which the next page
			// continues after.
			let last = &page_rows[page_rows.len() - 1];
			Some(EventsCursor {
				timestamp: super::persons::stamp(timestamp_of(last.first())?),
				uuid: match last.get(1) {
					Some(Value::Uuid(id)) => id.to_string(),
					Some(Value::Bytes(bytes)) => Uuid::from_slice(bytes).ok()?.to_string(),
					Some(Value::Text(text)) => text.clone(),
					_ => return None,
				},
			})
		})
		.and_then(|inner| inner);
	EventsPage { events, next }
}

/// The people a point of another query names, running its plan for its
/// rows: a trend bucket's statement, or a funnel's or retention's walk.
async fn actors_people(
	db: &Tracked<leptoskit_db::Db>,
	actors: &crate::query::ActorsQuery,
	planned: &Plan,
	work: &mut Work,
) -> rpc::Result<Vec<Uuid>> {
	match (&actors.point, planned) {
		(Point::Bucket { .. }, Plan::Actors(statement)) => {
			let rows = run_statement(db, statement, work).await?;
			Ok(rows.iter().filter_map(|row| uuid_of(row.first())).collect())
		}
		(
			Point::FunnelStep { step } | Point::FunnelDropOff { after: step },
			Plan::Funnels {
				pull,
				strict_pull,
				steps,
				window_seconds,
				order,
			},
		) => {
			let rows = run_statement(db, pull, work).await?;
			let strict_rows = match strict_pull {
				Some(strict) => Some(run_statement(db, strict, work).await?),
				None => None,
			};
			let walks = walk_funnel(
				steps,
				*window_seconds,
				*order,
				&rows,
				strict_rows.as_deref(),
			);
			let keep = usize::try_from(*step).unwrap_or(usize::MAX);
			let drop_off = matches!(actors.point, Point::FunnelDropOff { .. });
			Ok(walks
				.into_iter()
				.filter(|(_, walker)| {
					if drop_off {
						walker.reached == keep
					} else {
						walker.reached >= keep
					}
				})
				.filter_map(|(person, _)| person.uuid())
				.collect())
		}
		(
			Point::RetentionCell {
				start_period,
				return_period,
			},
			Plan::Retention {
				cohort,
				returns,
				kind,
				period_starts,
				..
			},
		) => {
			let cohort_rows = run_statement(db, cohort, work).await?;
			let return_rows = run_statement(db, returns, work).await?;
			// The cell's people, as the retention fold counts them: the
			// cohort's members who did the return event that many periods
			// after their cohort's start.
			let start = usize::try_from(*start_period).unwrap_or(usize::MAX);
			let members = cohort_members(*kind, period_starts, &cohort_rows);
			let Some(people) = members.get(start) else {
				return Ok(Vec::new());
			};
			let target =
				start.saturating_add(usize::try_from(*return_period).unwrap_or(usize::MAX));
			let returned = return_periods(period_starts, &return_rows);
			Ok(people
				.iter()
				.filter(|person| {
					returned
						.get(*person)
						.is_some_and(|periods| periods.contains(&target))
				})
				.filter_map(PersonKey::uuid)
				.collect())
		}
		_ => Ok(Vec::new()),
	}
}

/// The persons behind the people, as one page: their rows and their
/// distinct ids, in the people's own order.
async fn persons_page(
	db: &Tracked<leptoskit_db::Db>,
	people: Vec<Uuid>,
	limit: u32,
	offset: u32,
) -> rpc::Result<ActorsPage> {
	let limit = usize::try_from(limit.clamp(1, crate::query::ActorsQuery::MAX_LIMIT)).unwrap_or(50);
	let offset = usize::try_from(offset).unwrap_or(0);
	let page: Vec<Uuid> = people
		.iter()
		.skip(offset)
		.take(limit + 1)
		.copied()
		.collect();
	if page.is_empty() {
		return Ok(ActorsPage::default());
	}
	let next = if page.len() > limit {
		Some(u32::try_from(offset + limit).unwrap_or(u32::MAX))
	} else {
		None
	};
	let page = &page[..limit.min(page.len())];
	let dialect = db.dialect();
	let mut rows = Vec::new();
	for chunk in page.chunks(100) {
		let placeholders = chunk
			.iter()
			.enumerate()
			.map(|(index, person)| {
				let _ = person;
				placeholder(dialect, index + 1)
			})
			.collect::<Vec<_>>()
			.join(", ");
		let args = chunk
			.iter()
			.map(|person| Value::Uuid(*person))
			.collect::<Vec<_>>();
		let read = Statement::new(
			format!(
				"SELECT \"id\", \"properties\", \"created_at\", \"is_identified\" FROM \
				 \"persons\" WHERE \"id\" IN ({placeholders})"
			),
			args,
		)
		.reads::<tables::Person>();
		let found = read
			.fetch(db)
			.await
			.map_err(|error| rpc::Error::server(error.to_string()))?;
		rows.extend(found);
	}
	let mut by_id: BTreeMap<Uuid, (String, serde_json::Value, bool, String)> = BTreeMap::new();
	for row in rows {
		let Some(id) = uuid_of(row.first()) else {
			continue;
		};
		let properties = match row.get(1) {
			Some(Value::Text(text)) => serde_json::from_str(text).unwrap_or_default(),
			_ => serde_json::Value::Null,
		};
		let created = timestamp_of(row.get(2)).map_or_else(String::new, super::persons::stamp);
		let identified = matches!(row.get(3), Some(Value::Bool(true) | Value::Int(1)));
		by_id.insert(id, (id.to_string(), properties, identified, created));
	}
	let mut distinct_ids: BTreeMap<Uuid, Vec<String>> = BTreeMap::new();
	for chunk in page.chunks(100) {
		let placeholders = chunk
			.iter()
			.enumerate()
			.map(|(index, _)| placeholder(dialect, index + 1))
			.collect::<Vec<_>>()
			.join(", ");
		let args = chunk
			.iter()
			.map(|person| Value::Uuid(*person))
			.collect::<Vec<_>>();
		let read = Statement::new(
			format!(
				"SELECT \"person_id\", \"distinct_id\" FROM \"person_distinct_ids\" WHERE \
				 \"person_id\" IN ({placeholders})"
			),
			args,
		)
		.reads::<tables::PersonDistinctId>();
		let found = read
			.fetch(db)
			.await
			.map_err(|error| rpc::Error::server(error.to_string()))?;
		for row in found {
			if let (Some(person), Some(id)) = (uuid_of(row.first()), text_of(row.get(1))) {
				distinct_ids.entry(person).or_default().push(id);
			}
		}
	}
	let actors = page
		.iter()
		.filter_map(|id| {
			let (uuid, properties, identified, created) = by_id.get(id)?;
			Some(Actor {
				id: uuid.clone(),
				distinct_ids: distinct_ids.get(id).cloned().unwrap_or_default(),
				properties: properties.clone(),
				is_identified: *identified,
				created_at: created.clone(),
			})
		})
		.collect();
	Ok(ActorsPage { actors, next })
}

/// Refuses anyone but the app, as the person reads do.
fn app_only(identity: &Identity) -> Result<(), rpc::Error> {
	if identity.caller.as_deref() != Some(rpc::server::APP_CALLER) {
		return Err(rpc::Error::forbidden(
			"only the app runs queries and manages insights: call this from its server, with the \
			 service's token",
		));
	}
	Ok(())
}

/// An insight, as the operations answer with.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct InsightView {
	pub id: Uuid,
	pub name: String,
	pub description: String,
	/// The query the engine runs.
	pub query: crate::query::Query,
	/// Who created it, as the app's server named the acting admin.
	pub created_by: String,
	pub created_at: String,
	pub updated_at: String,
}

/// The insight's row, as the view.
fn insight_view(row: tables::Insight) -> Option<InsightView> {
	Some(InsightView {
		id: row.id,
		name: row.name,
		description: row.description,
		query: serde_json::from_str(&row.query).ok()?,
		created_by: row.created_by,
		created_at: super::persons::stamp(row.created_at),
		updated_at: super::persons::stamp(row.updated_at),
	})
}

/// Saves an insight: a query, named, with who made it.
///
/// # Errors
///
/// [`rpc::Error`] when the caller isn't the app, the query JSON doesn't
/// parse, or the write fails.
#[rpc::mutation(service = "analytics")]
pub async fn create_insight(
	cx: &rpc::MutationCx,
	name: String,
	#[serde(default)] description: String,
	query: crate::query::Query,
	#[serde(default)] created_by: String,
) -> rpc::Result<InsightView> {
	app_only(cx.identity())?;
	check_name(&name)?;
	let now = leptoskit_db::now();
	let row = tables::NewInsight {
		id: Uuid::new_v4(),
		name,
		description,
		query: serde_json::to_string(&query)
			.map_err(|error| rpc::Error::server(error.to_string()))?,
		created_by,
		created_at: now,
		updated_at: now,
	}
	.insert(cx.tx())
	.await?;
	insight_view(row).ok_or_else(|| rpc::Error::server("the saved query doesn't parse"))
}

/// Updates an insight's name, description, or query.
///
/// # Errors
///
/// [`rpc::Error`] when there's no insight with that id, or the write
/// fails.
#[rpc::mutation(service = "analytics")]
pub async fn update_insight(
	cx: &rpc::MutationCx,
	id: Uuid,
	#[serde(default)] name: Option<String>,
	#[serde(default)] description: Option<String>,
	#[serde(default)] query: Option<crate::query::Query>,
) -> rpc::Result<InsightView> {
	app_only(cx.identity())?;
	if let Some(name) = name.as_deref() {
		check_name(name)?;
	}
	let row = tables::Insight::query()
		.filter(|insight| insight.id.eq(id))
		.first(cx.tx())
		.await?
		.ok_or_else(|| rpc::Error::invalid("no insight with that id"))?;
	let now = leptoskit_db::now();
	let row = tables::NewInsight {
		id: row.id,
		name: name.unwrap_or(row.name),
		description: description.unwrap_or(row.description),
		query: match query {
			Some(query) => {
				serde_json::to_string(&query)
					.map_err(|error| rpc::Error::server(error.to_string()))?
			}
			None => row.query,
		},
		created_by: row.created_by,
		created_at: row.created_at,
		updated_at: now,
	}
	.upsert(cx.tx(), |insight| insight.id)
	.await?;
	insight_view(row).ok_or_else(|| rpc::Error::server("the saved query doesn't parse"))
}

/// Deletes an insight. Dashboards that showed it keep the tile, pointing
/// at nothing.
///
/// # Errors
///
/// [`rpc::Error`] when the write fails.
#[rpc::mutation(service = "analytics")]
pub async fn delete_insight(cx: &rpc::MutationCx, id: Uuid) -> rpc::Result<()> {
	app_only(cx.identity())?;
	tables::Insight::query()
		.filter(|insight| insight.id.eq(id))
		.delete(cx.tx())
		.await?;
	Ok(())
}

/// Every insight, newest first.
///
/// # Errors
///
/// [`rpc::Error`] when the read fails.
#[rpc::query(service = "analytics")]
pub async fn insights(cx: &rpc::QueryCx) -> rpc::Result<Vec<InsightView>> {
	app_only(cx.identity())?;
	let rows = tables::Insight::query()
		.order_by(|insight| insight.created_at.desc())
		.all(cx.db())
		.await?;
	Ok(rows.into_iter().filter_map(insight_view).collect())
}

/// One insight.
///
/// # Errors
///
/// [`rpc::Error`] when the read fails.
#[rpc::query(service = "analytics")]
pub async fn insight(cx: &rpc::QueryCx, id: Uuid) -> rpc::Result<Option<InsightView>> {
	app_only(cx.identity())?;
	let row = tables::Insight::query()
		.filter(|insight| insight.id.eq(id))
		.first(cx.db())
		.await?;
	Ok(row.and_then(insight_view))
}

/// A dashboard, as the operations answer with.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct DashboardView {
	pub id: Uuid,
	pub name: String,
	/// The tiles, in placement order.
	pub tiles: Vec<tables::Tile>,
	pub created_at: String,
	pub updated_at: String,
}

/// The dashboard's row, as the view.
fn dashboard_view(row: tables::Dashboard) -> Option<DashboardView> {
	Some(DashboardView {
		id: row.id,
		name: row.name,
		tiles: serde_json::from_str(&row.tiles).ok()?,
		created_at: super::persons::stamp(row.created_at),
		updated_at: super::persons::stamp(row.updated_at),
	})
}

/// Saves a dashboard: named tiles over insights, placed on a grid.
///
/// # Errors
///
/// [`rpc::Error`] when the caller isn't the app or the write fails.
#[rpc::mutation(service = "analytics")]
pub async fn create_dashboard(
	cx: &rpc::MutationCx,
	name: String,
	#[serde(default)] tiles: Vec<tables::Tile>,
) -> rpc::Result<DashboardView> {
	app_only(cx.identity())?;
	check_name(&name)?;
	let now = leptoskit_db::now();
	let row = tables::NewDashboard {
		id: Uuid::new_v4(),
		name,
		tiles: serde_json::to_string(&tiles)
			.map_err(|error| rpc::Error::server(error.to_string()))?,
		created_at: now,
		updated_at: now,
	}
	.insert(cx.tx())
	.await?;
	dashboard_view(row).ok_or_else(|| rpc::Error::server("the saved tiles don't parse"))
}

/// Updates a dashboard's name or tiles.
///
/// # Errors
///
/// [`rpc::Error`] when there's no dashboard with that id, or the write
/// fails.
#[rpc::mutation(service = "analytics")]
pub async fn update_dashboard(
	cx: &rpc::MutationCx,
	id: Uuid,
	#[serde(default)] name: Option<String>,
	#[serde(default)] tiles: Option<Vec<tables::Tile>>,
) -> rpc::Result<DashboardView> {
	app_only(cx.identity())?;
	if let Some(name) = name.as_deref() {
		check_name(name)?;
	}
	let row = tables::Dashboard::query()
		.filter(|dashboard| dashboard.id.eq(id))
		.first(cx.tx())
		.await?
		.ok_or_else(|| rpc::Error::invalid("no dashboard with that id"))?;
	let now = leptoskit_db::now();
	let row = tables::NewDashboard {
		id: row.id,
		name: name.unwrap_or(row.name),
		tiles: match tiles {
			Some(tiles) => {
				serde_json::to_string(&tiles)
					.map_err(|error| rpc::Error::server(error.to_string()))?
			}
			None => row.tiles,
		},
		created_at: row.created_at,
		updated_at: now,
	}
	.upsert(cx.tx(), |dashboard| dashboard.id)
	.await?;
	dashboard_view(row).ok_or_else(|| rpc::Error::server("the saved tiles don't parse"))
}

/// Deletes a dashboard. The insights it showed stay.
///
/// # Errors
///
/// [`rpc::Error`] when the write fails.
#[rpc::mutation(service = "analytics")]
pub async fn delete_dashboard(cx: &rpc::MutationCx, id: Uuid) -> rpc::Result<()> {
	app_only(cx.identity())?;
	tables::Dashboard::query()
		.filter(|dashboard| dashboard.id.eq(id))
		.delete(cx.tx())
		.await?;
	Ok(())
}

/// Every dashboard, newest first.
///
/// # Errors
///
/// [`rpc::Error`] when the read fails.
#[rpc::query(service = "analytics")]
pub async fn dashboards(cx: &rpc::QueryCx) -> rpc::Result<Vec<DashboardView>> {
	app_only(cx.identity())?;
	let rows = tables::Dashboard::query()
		.order_by(|dashboard| dashboard.created_at.desc())
		.all(cx.db())
		.await?;
	Ok(rows.into_iter().filter_map(dashboard_view).collect())
}

/// One dashboard.
///
/// # Errors
///
/// [`rpc::Error`] when the read fails.
#[rpc::query(service = "analytics")]
pub async fn dashboard(cx: &rpc::QueryCx, id: Uuid) -> rpc::Result<Option<DashboardView>> {
	app_only(cx.identity())?;
	let row = tables::Dashboard::query()
		.filter(|dashboard| dashboard.id.eq(id))
		.first(cx.db())
		.await?;
	Ok(row.and_then(dashboard_view))
}

/// Refuses a name that isn't 1 to 200 bytes of anything.
///
/// # Errors
///
/// [`rpc::Error::invalid`] saying so.
fn check_name(name: &str) -> Result<(), rpc::Error> {
	if name.trim().is_empty() || name.len() > 200 {
		return Err(rpc::Error::invalid("a name is 1 to 200 bytes"));
	}
	Ok(())
}

/// The "Web overview" dashboard as data (§15): the queries its insights
/// hold, in tile order. An app that wants the dashboard saves these as
/// insights, then the dashboard itself — its own decision, never something
/// the service writes on its own.
#[must_use]
pub fn web_overview_insights() -> Vec<(&'static str, crate::query::Query)> {
	let last_7 = crate::query::DateRange::last_days(7);
	vec![
		(
			"Web overview",
			crate::query::Query::WebOverview(crate::query::WebQuery {
				range: last_7.clone(),
				compare: true,
				filters: Vec::new(),
				table_limit: crate::query::WebQuery::default_table_limit(),
			}),
		),
		(
			"Pageviews, last 7 days",
			crate::query::Query::Trends(crate::query::TrendsQuery {
				range: last_7.clone(),
				interval: crate::query::Interval::Day,
				series: vec!["$pageview".into()],
				filters: Vec::new(),
				math: crate::query::Math::Total,
				breakdown: None,
				compare: true,
				test_accounts: None,
			}),
		),
		(
			"Visitors, last 7 days",
			crate::query::Query::Trends(crate::query::TrendsQuery {
				range: last_7.clone(),
				interval: crate::query::Interval::Day,
				series: vec!["$pageview".into()],
				filters: Vec::new(),
				math: crate::query::Math::UniquePersons,
				breakdown: None,
				compare: true,
				test_accounts: None,
			}),
		),
		(
			"Signed up, last 7 days",
			crate::query::Query::Trends(crate::query::TrendsQuery {
				range: last_7,
				interval: crate::query::Interval::Day,
				series: vec!["signed_up".into()],
				filters: Vec::new(),
				math: crate::query::Math::UniquePersons,
				breakdown: None,
				compare: false,
				test_accounts: None,
			}),
		),
	]
}

/// The web overview's tiles over insights saved in
/// [`web_overview_insights`]'s order: one wide overview on top, three
/// charts under it.
#[must_use]
pub fn web_overview_tiles(insights: &[Uuid]) -> Vec<tables::Tile> {
	let place = |index: usize, x: u16, y: u16, w: u16, h: u16| {
		insights.get(index).map(|insight| {
			tables::Tile {
				insight: *insight,
				x,
				y,
				w,
				h,
			}
		})
	};
	[
		place(0, 0, 0, 4, 2),
		place(1, 0, 2, 2, 2),
		place(2, 2, 2, 1, 2),
		place(3, 3, 2, 1, 2),
	]
	.into_iter()
	.flatten()
	.collect()
}

/// One event as the live tail carries it: the event, with the matched
/// rate a second ago.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TailItem {
	#[serde(flatten)]
	pub event: crate::service::tail::TailEvent,
	/// Matching events in the last second, as far as the tail ring holds.
	pub rate: u64,
}

impl rpc::Resumable for TailItem {
	type Cursor = u64;

	fn cursor(&self) -> u64 {
		self.event.seq
	}
}

/// The live tail (§11, minimal): events as the writer stores them,
/// filtered server-side by event name and distinct id, each with the
/// matched rate. A reconnecting client resumes from its cursor; events
/// the ring no longer holds are gone, and the stream says where it
/// reached, so the page can say it may be missing some.
///
/// # Errors
///
/// [`rpc::Error`] when the caller isn't the app, or no writer is running.
#[rpc::stream(resumable, service = "analytics")]
pub async fn live_events(
	cx: &rpc::QueryCx,
	#[serde(default)] event: Option<String>,
	#[serde(default)] distinct_id: Option<String>,
	cursor: Option<u64>,
) -> rpc::Result<impl futures::Stream<Item = TailItem>> {
	use futures::StreamExt as _;
	app_only(cx.identity())?;
	let tail = cx.require::<super::Ingest>()?.tail();
	let cursor = cursor.unwrap_or_default();
	Ok(futures::stream::unfold(
		(tail, event, distinct_id, cursor),
		|(tail, event, distinct_id, mut cursor)| {
			async move {
				// Wake a few times a second, which a tail can read.
				tokio::time::sleep(std::time::Duration::from_millis(300)).await;
				let (events, latest, _missed) = tail.since(cursor);
				cursor = latest.max(cursor);
				let now = leptoskit_db::now();
				let rate = tail.rate(
					event.as_deref(),
					distinct_id.as_deref(),
					now,
					chrono::TimeDelta::seconds(1),
				);
				let items: Vec<TailItem> = events
					.into_iter()
					.filter(|tail_event| {
						event.as_deref().is_none_or(|name| tail_event.name == name)
							&& distinct_id
								.as_deref()
								.is_none_or(|id| tail_event.distinct_id == id)
					})
					.map(|tail_event| {
						TailItem {
							event: tail_event,
							rate,
						}
					})
					.collect();
				// An empty batch flattens to nothing and the poll continues.
				Some((
					futures::stream::iter(items),
					(tail, event, distinct_id, cursor),
				))
			}
		},
	)
	.flatten())
}

/// One event name the service has stored, as the catalogue knows it: what
/// the admin's series picker lists, and what its Workings drawer shows in
/// Emit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct EventDefinitionView {
	pub name: String,
	/// How many have been stored.
	pub volume: i64,
	pub first_seen: String,
	pub last_seen: String,
	/// The latest schema registered for it, when the app declares it as a
	/// Rust type (`#[derive(Event)]`): each property's name and type as
	/// spelled.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub fields: Option<Vec<(String, String)>>,
	/// The keys of the properties the latest few events carried, each with
	/// the JSON type its values had (`string`, `integer`, `number`,
	/// `boolean`, `array`, `object`), sorted by key. For an event without a
	/// type, it's what a derive could be written from.
	pub properties: Vec<(String, String)>,
}

/// How many recent events [`event_definitions`] reads per name for their
/// property keys.
const SAMPLED_EVENTS: i64 = 20;

/// The event catalogue: every event name stored, most stored first, with
/// its typed schema when it has one and the property keys recent events
/// carried.
///
/// # Errors
///
/// [`rpc::Error`] when the caller isn't the app, or a read fails.
#[rpc::query(service = "analytics")]
pub async fn event_definitions(cx: &rpc::QueryCx) -> rpc::Result<Vec<EventDefinitionView>> {
	app_only(cx.identity())?;
	let db = cx.db();
	let dialect = db.dialect();
	read_tick(db, dialect).await?;
	let definitions = tables::EventDefinition::query()
		.order_by(|definition| definition.volume.desc())
		.all(db)
		.await?;
	let schemas = tables::EventSchema::query()
		.order_by(|schema| schema.last_registered.asc())
		.all(db)
		.await?;
	let mut latest: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();

	for schema in schemas {
		if let Ok(fields) = serde_json::from_str::<Vec<(String, String)>>(&schema.fields) {
			// Ordered oldest first, so the last one written wins.
			latest.insert(schema.name, fields);
		}
	}

	let mut views = Vec::with_capacity(definitions.len());

	for definition in definitions {
		let statement = Statement::new(
			format!(
				"SELECT \"properties\" FROM \"events\" WHERE \"name\" = {} ORDER BY \"timestamp\" \
				 DESC LIMIT {SAMPLED_EVENTS}",
				placeholder(dialect, 1)
			),
			vec![Value::Text(definition.name.clone())],
		)
		.reads::<tables::Event>();
		let rows = tokio::time::timeout(plan::QUERY_TIMEOUT, statement.fetch(db))
			.await
			.map_err(|_| rpc::Error::server("the query took over its timeout"))?
			.map_err(|error| rpc::Error::server(error.to_string()))?;
		let mut properties: BTreeMap<String, String> = BTreeMap::new();

		for row in rows {
			let Some(Value::Text(text)) = row.first() else {
				continue;
			};
			let Ok(serde_json::Value::Object(object)) = serde_json::from_str(text) else {
				continue;
			};

			for (key, value) in object {
				properties.entry(key).or_insert_with(|| json_type(&value));
			}
		}

		views.push(EventDefinitionView {
			fields: latest.get(&definition.name).cloned(),
			name: definition.name,
			volume: definition.volume,
			first_seen: super::persons::stamp(definition.first_seen),
			last_seen: super::persons::stamp(definition.last_seen),
			properties: properties.into_iter().collect(),
		});
	}

	Ok(views)
}

/// A JSON value's type, as [`EventDefinitionView::properties`] names it.
fn json_type(value: &serde_json::Value) -> String {
	match value {
		serde_json::Value::Null => "null",
		serde_json::Value::Bool(_) => "boolean",
		serde_json::Value::Number(number) if number.is_i64() || number.is_u64() => "integer",
		serde_json::Value::Number(_) => "number",
		serde_json::Value::String(_) => "string",
		serde_json::Value::Array(_) => "array",
		serde_json::Value::Object(_) => "object",
	}
	.to_owned()
}
