//! The planner: what SQL a query runs, per dialect.
//!
//! [`plan`] turns a [`Query`] and a moment into a [`Plan`]: the named
//! statements to run, with every caller-controlled value bound as a
//! parameter, and the shape the runner folds their rows into. Nothing here
//! touches the database, which is what the SQL snapshots test against.
//!
//! Three boundaries keep untrusted text out of the SQL:
//!
//! - Property names go through [`checked_property`] and bind as parameters, so
//!   a name carrying SQL reads a property of that name, rather than running
//!   SQL.
//! - Session columns come from an allow-list.
//! - Event names, distinct ids, and values are always bound.

use std::fmt::Write as _;

use chrono::Datelike as _;
use chrono::Duration;
use chrono::NaiveDate;
use chrono::NaiveDateTime;
use chrono::TimeZone as _;
use chrono::Utc;
use leptoskit_db::Dialect;
use leptoskit_db::Timestamp;
use leptoskit_db::Value;

use super::dialect::DialectSql;
pub use super::dialect::PlanError;
use super::dialect::SESSION_COLUMNS;
use super::dialect::SessionColumn;
use super::dialect::checked_property;
use crate::query::ActorsQuery;
use crate::query::Breakdown;
use crate::query::DateRange;
use crate::query::EventColumn;
use crate::query::EventsQuery;
use crate::query::FilterSource;
use crate::query::FunnelsQuery;
use crate::query::Interval;
use crate::query::Math;
use crate::query::PropertyFilter;
use crate::query::Query;
use crate::query::RetentionQuery;
use crate::query::TestAccounts;
use crate::query::TrendsQuery;
use crate::query::WebQuery;

/// The most rows one statement reads, whatever the query asks: a query
/// that would read more is truncated, and its answer is a lower bound,
/// never wrong (§10's row limit).
pub const MAX_ROWS: u32 = 100_000;

/// The most rows an aggregation answers with, after grouping.
const MAX_GROUPED_ROWS: u32 = 20_000;

/// How long a query may run before the runner gives up on it (§10's
/// timeout).
pub const QUERY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// How a resolved range looks once the planner has it: absolute bounds in
/// UTC, with the fixed offset its wall-clock text stands for.
#[derive(Clone, Copy, Debug)]
pub struct Range {
	pub from: Timestamp,
	pub to: Timestamp,
	/// Seconds east of UTC the range's zone stands for.
	pub offset_seconds: i32,
}

impl Range {
	/// The range shifted back by its own length: the previous period.
	#[must_use]
	pub fn previous(&self) -> Self {
		let span = self.to - self.from;
		Self {
			from: self.from - span,
			to: self.to - span,
			offset_seconds: self.offset_seconds,
		}
	}

	/// The wall-clock text of an instant in the range's zone, the form
	/// bucket labels take.
	#[must_use]
	pub fn wall(&self, at: Timestamp) -> String {
		wall_text(at, self.offset_seconds)
	}

	/// The instant a bucket label stands for: wall clock in the range's
	/// zone, back to UTC.
	///
	/// # Errors
	///
	/// [`PlanError::Range`] when the label isn't the bucket text the
	/// engine itself writes.
	pub fn utc(&self, label: &str) -> Result<Timestamp, PlanError> {
		utc_of(label, self.offset_seconds)
	}

	/// The bucket starts the range holds, oldest first, aligned to
	/// `interval` in the range's zone: the buckets a result fills, so a
	/// chart has no holes. Capped at `cap`.
	#[must_use]
	pub fn buckets(&self, interval: Interval, cap: usize) -> Vec<String> {
		let mut starts = Vec::new();
		let mut wall = self.wall(self.from);
		align(&mut wall, interval);
		let to = self.wall(self.to);
		while wall < to && starts.len() < cap {
			starts.push(wall.clone());
			step(&mut wall, interval);
		}
		starts
	}
}

/// The wall-clock text of an instant, `offset_seconds` east of UTC.
#[must_use]
pub fn wall_text(at: Timestamp, offset_seconds: i32) -> String {
	(at + Duration::seconds(i64::from(offset_seconds)))
		.format("%Y-%m-%dT%H:%M:%S")
		.to_string()
}

/// The instant wall-clock text stands for, in a zone `offset_seconds` east
/// of UTC.
///
/// # Errors
///
/// [`PlanError::Range`] when the text isn't what [`wall_text`] writes.
pub fn utc_of(label: &str, offset_seconds: i32) -> Result<Timestamp, PlanError> {
	let wall = NaiveDateTime::parse_from_str(label, "%Y-%m-%dT%H:%M:%S")
		.map_err(|error| PlanError::Range(format!("{label:?} isn't a bucket start: {error}")))?;
	Ok(Utc.from_utc_datetime(&(wall - Duration::seconds(i64::from(offset_seconds)))))
}

/// Aligns wall-clock text down to its interval's start.
fn align(wall: &mut String, interval: Interval) {
	if interval == Interval::Hour {
		wall.replace_range(13.., "00:00");
		return;
	}
	wall.replace_range(10.., "T00:00:00");
	if interval == Interval::Week {
		let date = NaiveDate::parse_from_str(&wall[..10], "%Y-%m-%d").expect("aligned date");
		let monday = date - Duration::days(i64::from(date.weekday().num_days_from_monday()));
		*wall = format!("{}T00:00:00", monday.format("%Y-%m-%d"));
	}
}

/// Steps wall-clock text on to the next interval's start.
fn step(wall: &mut String, interval: Interval) {
	let date = NaiveDate::parse_from_str(&wall[..10], "%Y-%m-%d").expect("aligned date");
	let next = match interval {
		Interval::Hour => {
			let hour = wall[11..13].parse::<u32>().expect("aligned hour") + 1;
			if hour < 24 {
				wall.replace_range(11..13, &format!("{hour:02}"));
				return;
			}
			format!(
				"{}T00:00:00",
				date.succ_opt().expect("a next day").format("%Y-%m-%d")
			)
		}
		Interval::Day => {
			format!(
				"{}T00:00:00",
				date.succ_opt().expect("a next day").format("%Y-%m-%d")
			)
		}
		Interval::Week => {
			format!(
				"{}T00:00:00",
				(date + Duration::weeks(1)).format("%Y-%m-%d")
			)
		}
		Interval::Month => {
			let (year, month) = if date.month() == 12 {
				(date.year() + 1, 1)
			} else {
				(date.year(), date.month() + 1)
			};
			format!("{year:04}-{month:02}-01T00:00:00")
		}
	};
	*wall = next;
}

/// Reads a fixed offset from UTC: absent, `UTC`, or `Z` mean UTC;
/// `+HH:MM`, `-HH:MM`, and `+HHMM` are offsets. Named IANA zones move with
/// daylight saving, which a whole range can't follow at a fixed offset, so
/// they're refused for now rather than quietly bucketed wrong.
///
/// # Errors
///
/// [`PlanError::Unsupported`] for anything else.
pub fn parse_timezone(timezone: Option<&str>) -> Result<i32, PlanError> {
	let Some(label) = timezone.map(str::trim).filter(|label| !label.is_empty()) else {
		return Ok(0);
	};
	if matches!(label.to_ascii_lowercase().as_str(), "utc" | "z" | "gmt") {
		return Ok(0);
	}
	let (sign, rest) = match label.split_at(1) {
		("+", rest) => (1, rest),
		("-", rest) => (-1, rest),
		_ => {
			return Err(PlanError::Unsupported(format!(
				"timezone {label:?} isn't a fixed offset like \"+02:00\"; the engine buckets in \
				 UTC or a fixed offset for now, not named zones"
			)));
		}
	};
	let (hours, minutes) = match rest.split_once(':') {
		Some((hours, minutes)) => (hours, minutes),
		None if rest.len() == 4 => rest.split_at(2),
		None => ("", rest),
	};
	let (Ok(hours), Ok(minutes)) = (hours.parse::<i32>(), minutes.parse::<i32>()) else {
		return Err(PlanError::Unsupported(format!(
			"timezone {label:?} isn't a fixed offset"
		)));
	};
	if !(0..=14).contains(&hours) || !(0..60).contains(&minutes) {
		return Err(PlanError::Unsupported(format!(
			"timezone {label:?} isn't an offset UTC can hold"
		)));
	}
	Ok(sign * (hours * 3_600 + minutes * 60))
}

/// Parses an RFC 3339 bound.
///
/// # Errors
///
/// [`PlanError::Range`] when it doesn't parse.
pub fn parse_bound(text: &str, which: &str) -> Result<Timestamp, PlanError> {
	chrono::DateTime::parse_from_rfc3339(text)
		.map(Into::into)
		.map_err(|error| {
			PlanError::Range(format!("the range's {which} bound isn't RFC 3339: {error}"))
		})
}

/// Resolves a [`DateRange`] against `now`: absolute bounds, or the last
/// `last_days` days ending now, in a fixed-offset zone.
///
/// # Errors
///
/// [`PlanError::Range`] when bounds don't parse or don't hold.
pub fn resolve_range(range: &DateRange, now: Timestamp) -> Result<Range, PlanError> {
	let offset_seconds = parse_timezone(range.timezone.as_deref())?;
	let resolved = match (&range.from, &range.to) {
		(Some(from), to) => {
			Range {
				from: parse_bound(from, "from")?,
				to: to.as_deref().map_or(Ok(now), |to| parse_bound(to, "to"))?,
				offset_seconds,
			}
		}
		(None, to) => {
			let days = range.last_days.unwrap_or(7).max(1);
			let end = to.as_deref().map_or(Ok(now), |to| parse_bound(to, "to"))?;
			let helper = Range {
				from: end,
				to: end,
				offset_seconds,
			};
			let mut wall = helper.wall(end);
			align(&mut wall, Interval::Day);
			let from = helper.utc(&wall)? - Duration::days(i64::from(days - 1));
			Range {
				from,
				to: end,
				offset_seconds,
			}
		}
	};
	if resolved.from >= resolved.to {
		return Err(PlanError::Range(
			"the range's start isn't before its end".into(),
		));
	}
	Ok(resolved)
}

/// A planned statement: its purpose, its SQL, and its bound parameters.
#[derive(Clone, Debug, PartialEq)]
pub struct Statement {
	pub name: String,
	pub sql: String,
	pub args: Vec<Value>,
}

impl Statement {
	fn new(name: impl Into<String>, sql: String, args: Vec<Value>) -> Self {
		Self {
			name: name.into(),
			sql,
			args,
		}
	}
}

/// What a query runs, and what the runner folds from the rows.
#[derive(Clone, Debug, PartialEq)]
pub enum Plan {
	/// One statement per series: buckets, values, and, when broken down,
	/// the breakdown group. `starts` are the labels the result fills.
	Trends {
		current: Vec<Statement>,
		previous: Vec<Statement>,
		starts: Vec<String>,
		previous_starts: Vec<String>,
		math: Math,
	},
	/// Every event the funnel reads, one person at a time, in time order.
	/// A strict funnel needs the events between steps too, so it pulls
	/// everything the range holds.
	Funnels {
		pull: Statement,
		strict_pull: Option<Statement>,
		steps: Vec<String>,
		window_seconds: u64,
		order: crate::query::FunnelOrder,
	},
	/// The cohort statement carries each person's first-ever start and
	/// their start in the first period; the returns statement carries each
	/// person's return periods, as bucket labels.
	Retention {
		cohort: Statement,
		returns: Statement,
		period: Interval,
		kind: crate::query::RetentionKind,
		periods: u32,
		/// Every period the range holds, as labels: each starts a cohort.
		period_starts: Vec<String>,
	},
	/// The web overview's three reads: the sessions' totals, the
	/// `$pageview`s' top pages, and the sessions' breakdown tables.
	WebOverview {
		stats: Statement,
		previous_stats: Option<Statement>,
		pages: Statement,
		dimensions: Statement,
		table_limit: u32,
	},
	/// One page of the events list.
	Events { page: Statement },
	/// The people in one trend bucket, as resolved person ids.
	Actors(Statement),
}

/// Plans any query.
///
/// # Errors
///
/// [`PlanError`] naming what the query asks for that the engine can't
/// safely or sensibly do.
pub fn plan(query: &Query, now: Timestamp, dialect: Dialect) -> Result<Plan, PlanError> {
	let sql = DialectSql::new(dialect);
	match query {
		Query::Trends(trends) => trends_plan(&sql, trends, now),
		Query::Funnels(funnels) => funnels_plan(&sql, funnels, now),
		Query::Retention(retention) => retention_plan(&sql, retention, now),
		Query::WebOverview(web) => web_plan(&sql, web, now),
		Query::Events(events) => events_plan(&sql, events, now),
		Query::Actors(actors) => actors_plan(&sql, actors, now),
	}
}

/// The events table's person, resolved through `person_overrides` (§8): a
/// merge applies to past events at once, without rewriting them, and an
/// event that predates person processing finds its person through its
/// distinct id.
#[must_use]
pub fn person_of_events() -> &'static str {
	"COALESCE((SELECT \"po\".\"person_id\" FROM \"person_overrides\" \"po\" WHERE \
	 \"po\".\"distinct_id\" = \"events\".\"distinct_id\"), \"events\".\"person_id\", (SELECT \
	 \"pdi\".\"person_id\" FROM \"person_distinct_ids\" \"pdi\" WHERE \"pdi\".\"distinct_id\" = \
	 \"events\".\"distinct_id\"))"
}

/// The sessions table's person, through the same overrides. Sessions
/// queries read `FROM "sessions" "s"`, so the alias is `s`.
#[must_use]
pub fn person_of_sessions() -> String {
	person_of_events()
		.replace("\"events\".\"distinct_id\"", "\"s\".\"distinct_id\"")
		.replace("\"events\".\"person_id\"", "\"s\".\"person_id\"")
}

/// How a trend counts events.
fn aggregate(math: Math) -> String {
	match math {
		Math::Total => "count(*)".to_owned(),
		// A person without one still counts, under their distinct id, the
		// way anonymous events do; a merge never counts twice.
		Math::UniquePersons => {
			format!(
				"count(distinct coalesce({}, \"events\".\"distinct_id\"))",
				person_of_events()
			)
		}
		Math::UniqueSessions => "count(distinct \"events\".\"session_id\")".to_owned(),
	}
}

/// Builds the events table's `WHERE`: the range, one event's name,
/// property filters, and the test accounts. `args` grows in the order the
/// placeholders are numbered.
fn event_where(
	sql: &DialectSql,
	args: &mut Vec<Value>,
	range: &Range,
	filters: &[PropertyFilter],
	event: Option<&str>,
	test_accounts: Option<&TestAccounts>,
) -> Result<String, PlanError> {
	let mut parts = vec![format!(
		r#""events"."timestamp" >= {}"#,
		sql.placeholder(args.len() + 1)
	)];
	args.push(Value::Timestamp(range.from));
	parts.push(format!(
		r#""events"."timestamp" < {}"#,
		sql.placeholder(args.len() + 1)
	));
	args.push(Value::Timestamp(range.to));

	if let Some(event) = event {
		if event.is_empty() || event.len() > 200 {
			return Err(PlanError::Range(format!(
				"the event name is 1 to 200 bytes, not {}",
				event.len()
			)));
		}
		args.push(Value::Text(event.to_owned()));
		parts.push(format!(
			r#""events"."name" = {}"#,
			sql.placeholder(args.len())
		));
	}
	for filter in filters {
		parts.push(filter_condition(sql, args, filter)?);
	}
	if let Some(test_accounts) = test_accounts {
		let ids = match test_accounts {
			TestAccounts::Exclude(ids) | TestAccounts::Only(ids) => ids,
		};
		if ids.is_empty() {
			return Err(PlanError::Range(
				"the test-account filter names no distinct ids".into(),
			));
		}
		let placeholders = ids
			.iter()
			.map(|id| {
				args.push(Value::Text(id.clone()));
				sql.placeholder(args.len())
			})
			.collect::<Vec<_>>()
			.join(", ");
		let keyword = match test_accounts {
			TestAccounts::Exclude(_) => "NOT IN",
			TestAccounts::Only(_) => "IN",
		};
		parts.push(format!(
			r#""events"."distinct_id" {keyword} ({placeholders})"#
		));
	}
	Ok(parts.join(" AND "))
}

/// One filter's condition, on the event's properties, its person's, or its
/// session's columns.
fn filter_condition(
	sql: &DialectSql,
	args: &mut Vec<Value>,
	filter: &PropertyFilter,
) -> Result<String, PlanError> {
	let key = checked_property(&filter.key)?;
	match filter.source {
		FilterSource::Event => {
			property_condition(sql, args, r#""events"."properties""#, &key, filter)
		}
		FilterSource::Person => {
			let inner = property_condition(sql, args, r#""pp"."properties""#, &key, filter)?;
			Ok(format!(
				"{} IN (SELECT \"pp\".\"id\" FROM \"persons\" \"pp\" WHERE {inner})",
				person_of_events()
			))
		}
		FilterSource::Session => {
			let Some((column, kind)) = SESSION_COLUMNS.iter().find(|(name, _)| *name == filter.key)
			else {
				return Err(PlanError::Column(format!(
					"{:?} isn't a session column; the ones a filter may name are {}",
					filter.key,
					SESSION_COLUMNS
						.iter()
						.map(|(name, _)| *name)
						.collect::<Vec<_>>()
						.join(", ")
				)));
			};
			let mut condition_args = sql.condition_value(filter.operator, filter.value.as_ref())?;
			args.append(&mut condition_args);
			let inner = sql.session_condition(column, *kind, filter.operator, args.len())?;
			Ok(format!(
				r#""events"."session_id" IN (SELECT "s"."id" FROM "sessions" "s" WHERE {inner})"#
			))
		}
	}
}

/// A condition on one JSON column, binding the property's name, then the
/// value.
fn property_condition(
	sql: &DialectSql,
	args: &mut Vec<Value>,
	column: &str,
	key: &super::dialect::PropertyKey,
	filter: &PropertyFilter,
) -> Result<String, PlanError> {
	args.push(match sql.dialect {
		Dialect::Postgres => Value::Text(key.key.clone()),
		Dialect::Sqlite => Value::Text(key.path.clone()),
	});
	let name_at = args.len();
	let mut condition_args = sql.condition_value(filter.operator, filter.value.as_ref())?;
	args.append(&mut condition_args);
	sql.condition(filter.operator, column, name_at, args.len())
}

/// The breakdown expression: one property of the event, its person, or its
/// session, as text. The name binds; the value groups.
fn breakdown_expression(
	sql: &DialectSql,
	args: &mut Vec<Value>,
	breakdown: &Breakdown,
) -> Result<String, PlanError> {
	let key = checked_property(&breakdown.key)?;
	Ok(match breakdown.source {
		FilterSource::Event => property_text_expr(sql, args, r#""events"."properties""#, &key),
		FilterSource::Person => {
			let text = property_text_expr(sql, args, r#""pp"."properties""#, &key);
			format!(
				"(SELECT {text} FROM \"persons\" \"pp\" WHERE \"pp\".\"id\" = {})",
				person_of_events()
			)
		}
		FilterSource::Session => {
			let Some((column, kind)) = SESSION_COLUMNS
				.iter()
				.find(|(name, _)| *name == breakdown.key)
			else {
				return Err(PlanError::Column(format!(
					"{:?} isn't a session column",
					breakdown.key
				)));
			};
			if *kind != SessionColumn::Text {
				return Err(PlanError::Column(format!(
					"a breakdown groups text, and `{}` holds {}",
					breakdown.key,
					match kind {
						SessionColumn::Number => "a number",
						SessionColumn::Bool => "a boolean",
						SessionColumn::Text => "text",
					}
				)));
			}
			format!(
				"(SELECT \"sb\".\"{column}\" FROM \"sessions\" \"sb\" WHERE \"sb\".\"id\" = \
				 \"events\".\"session_id\")"
			)
		}
	})
}

/// One JSON property as text, with its name bound.
fn property_text_expr(
	sql: &DialectSql,
	args: &mut Vec<Value>,
	column: &str,
	key: &super::dialect::PropertyKey,
) -> String {
	args.push(match sql.dialect {
		Dialect::Postgres => Value::Text(key.key.clone()),
		Dialect::Sqlite => Value::Text(key.path.clone()),
	});
	sql.property_text(column, args.len())
}

/// The bound shift the range's zone takes, for [`DialectSql::bucket`]:
/// SQLite's strftime modifier wants a sign on the number, Postgres's
/// interval doesn't.
fn shift_arg(dialect: Dialect, range: &Range) -> Value {
	match dialect {
		Dialect::Sqlite => {
			let sign = if range.offset_seconds >= 0 { "+" } else { "-" };
			Value::Text(format!("{sign}{} seconds", range.offset_seconds.abs()))
		}
		Dialect::Postgres => Value::Text(format!("{} seconds", range.offset_seconds)),
	}
}

/// Plans a trends query: one statement per series, plus the previous
/// period's when compared.
fn trends_plan(sql: &DialectSql, query: &TrendsQuery, now: Timestamp) -> Result<Plan, PlanError> {
	if query.series.is_empty() {
		return Err(PlanError::Range(
			"a trends query names at least one series".into(),
		));
	}
	let range = resolve_range(&query.range, now)?;
	let mut current = Vec::new();
	let mut previous = Vec::new();
	for series in &query.series {
		if !query.compare {
			current.push(trends_statement(sql, query, series, &range, "trends")?);
			continue;
		}
		current.push(trends_statement(sql, query, series, &range, "trends")?);
		previous.push(trends_statement(
			sql,
			query,
			series,
			&range.previous(),
			"trends previous",
		)?);
	}
	Ok(Plan::Trends {
		current,
		previous,
		starts: range.buckets(query.interval, 1_000),
		previous_starts: if query.compare {
			range.previous().buckets(query.interval, 1_000)
		} else {
			Vec::new()
		},
		math: query.math,
	})
}

/// One series' statement, over `when`.
fn trends_statement(
	sql: &DialectSql,
	query: &TrendsQuery,
	series: &crate::query::Series,
	when: &Range,
	label: &str,
) -> Result<Statement, PlanError> {
	let mut args = Vec::new();
	// An empty series name counts every event.
	let event = if series.event.is_empty() {
		None
	} else {
		Some(series.event.as_str())
	};
	let where_clause = event_where(
		sql,
		&mut args,
		when,
		&query.filters,
		event,
		query.test_accounts.as_ref(),
	)?;
	args.push(shift_arg(sql.dialect, when));
	let bucket = sql.bucket(query.interval, args.len())?;
	let mut select = format!("SELECT {bucket} AS \"bucket\"");
	let mut group = "GROUP BY \"bucket\"".to_owned();
	if let Some(breakdown) = query.breakdown.as_ref() {
		let expression = breakdown_expression(sql, &mut args, breakdown)?;
		let _ = write!(select, ", {expression} AS \"group\"");
		group.push_str(", \"group\"");
	}
	let _ = write!(
		select,
		", {} AS \"value\" FROM \"events\" WHERE {where_clause} {group} ORDER BY \"bucket\" LIMIT \
		 {MAX_GROUPED_ROWS}",
		aggregate(query.math)
	);
	let who = if series.event.is_empty() {
		"every event"
	} else {
		&series.event
	};
	Ok(Statement::new(format!("{label}: {who}"), select, args))
}

/// Plans a funnel: one pull of its events, one person at a time, in time
/// order. A strict funnel needs the events between steps too, so it pulls
/// everything the range holds; the others pull only the steps' events.
fn funnels_plan(sql: &DialectSql, query: &FunnelsQuery, now: Timestamp) -> Result<Plan, PlanError> {
	if query.steps.len() < 2 {
		return Err(PlanError::Range("a funnel has at least two steps".into()));
	}
	if query.steps.len() > 8 {
		return Err(PlanError::Range("a funnel has at most eight steps".into()));
	}
	if query
		.steps
		.iter()
		.any(|step| step.is_empty() || step.len() > 200)
	{
		return Err(PlanError::Range(
			"a funnel's steps are event names of 1 to 200 bytes".into(),
		));
	}
	let mut distinct = query.steps.clone();
	distinct.sort();
	distinct.dedup();
	if distinct.len() != query.steps.len() {
		return Err(PlanError::Range(
			"a funnel's steps are distinct event names".into(),
		));
	}
	let range = resolve_range(&query.range, now)?;
	let window_seconds = query.window_seconds.unwrap_or(14 * 86_400);

	let mut pull_args = Vec::new();
	let where_clause = event_where(
		sql,
		&mut pull_args,
		&range,
		&query.filters,
		None,
		query.test_accounts.as_ref(),
	)?;
	let steps_at = pull_args.len() + 1;
	for step in &query.steps {
		pull_args.push(Value::Text(step.clone()));
	}
	let placeholders = (steps_at..steps_at + query.steps.len())
		.map(|n| sql.placeholder(n))
		.collect::<Vec<_>>()
		.join(", ");
	let breakdown = query
		.breakdown
		.as_ref()
		.map(|breakdown| breakdown_expression(sql, &mut pull_args, breakdown))
		.transpose()?
		.unwrap_or_else(|| "NULL".to_owned());
	// A funnel's people are whoever did the steps: a person resolved
	// through the overrides (§8), or, for events without one, the
	// distinct id itself, the way anonymous events count.
	let pull_sql = format!(
		"SELECT coalesce({0}, \"events\".\"distinct_id\") AS \"person\", \"events\".\"name\" AS \
		 \"name\", \"events\".\"timestamp\" AS \"at\", {breakdown} AS \"group\" FROM \"events\" \
		 WHERE \"events\".\"name\" IN ({placeholders}) AND {where_clause} ORDER BY \"person\", \
		 \"at\" LIMIT {MAX_ROWS}",
		person_of_events()
	);
	let mut strict_pull = None;
	if query.order == crate::query::FunnelOrder::Strict {
		let mut strict_args = Vec::new();
		let strict_where = event_where(
			sql,
			&mut strict_args,
			&range,
			&query.filters,
			None,
			query.test_accounts.as_ref(),
		)?;
		let strict_breakdown = query
			.breakdown
			.as_ref()
			.map(|breakdown| breakdown_expression(sql, &mut strict_args, breakdown))
			.transpose()?
			.unwrap_or_else(|| "NULL".to_owned());
		strict_pull = Some(Statement::new(
			"funnel strict pull",
			format!(
				"SELECT coalesce({0}, \"events\".\"distinct_id\") AS \"person\", \
				 \"events\".\"name\" AS \"name\", \"events\".\"timestamp\" AS \"at\", \
				 {strict_breakdown} AS \"group\" FROM \"events\" WHERE {strict_where} ORDER BY \
				 \"person\", \"at\" LIMIT {MAX_ROWS}",
				person_of_events()
			),
			strict_args,
		));
	}
	Ok(Plan::Funnels {
		pull: Statement::new("funnel pull", pull_sql, pull_args),
		strict_pull,
		steps: query.steps.clone(),
		window_seconds,
		order: query.order,
	})
}

/// Plans retention: the cohort's starts, and the returns' periods.
fn retention_plan(
	sql: &DialectSql,
	query: &RetentionQuery,
	now: Timestamp,
) -> Result<Plan, PlanError> {
	if !matches!(query.period, Interval::Day | Interval::Week) {
		return Err(PlanError::Range(
			"retention cohorts are day- or week-wide; hour and month aren't cohorts".into(),
		));
	}
	if query.start_event.is_empty() || query.return_event.is_empty() {
		return Err(PlanError::Range(
			"retention names a start and a return event".into(),
		));
	}
	let range = resolve_range(&query.range, now)?;
	// Every period the range holds starts a cohort; each cohort's returns
	// are counted in the periods after it, as far as the range reaches.
	let period_starts = range.buckets(query.period, 366);
	if period_starts.is_empty() {
		return Err(PlanError::Range(
			"the range holds less than one cohort period".into(),
		));
	}
	let periods = query
		.periods
		.unwrap_or_else(|| (period_starts.len() - 1).min(12) as u32)
		.clamp(1, 12);

	// The starts, one row per person and period. The statement reads the
	// start event's whole history up to the range's end, so first-time
	// retention can tell a person's first period ever from a later one.
	let mut cohort_args = Vec::new();
	let cohort_where = event_where(
		sql,
		&mut cohort_args,
		&Range {
			from: Timestamp::UNIX_EPOCH,
			to: range.to,
			offset_seconds: range.offset_seconds,
		},
		&query.filters,
		Some(&query.start_event),
		query.test_accounts.as_ref(),
	)?;
	cohort_args.push(shift_arg(sql.dialect, &range));
	let cohort_bucket = sql.bucket(query.period, cohort_args.len())?;
	// Retention counts whoever did the steps, the way funnels do: a
	// person resolved through the overrides, or the distinct id itself.
	let cohort_sql = format!(
		"SELECT coalesce({0}, \"events\".\"distinct_id\") AS \"person\", {cohort_bucket} AS \
		 \"period\" FROM \"events\" WHERE {cohort_where} GROUP BY \"person\", \"period\" ORDER BY \
		 \"person\", \"period\" LIMIT {MAX_ROWS}",
		person_of_events(),
	);
	let cohort = Statement::new("retention cohort", cohort_sql, cohort_args);

	let mut returns_args = Vec::new();
	let returns_where = event_where(
		sql,
		&mut returns_args,
		&range,
		&query.filters,
		Some(&query.return_event),
		query.test_accounts.as_ref(),
	)?;
	returns_args.push(shift_arg(sql.dialect, &range));
	let bucket = sql.bucket(query.period, returns_args.len())?;
	let returns_sql = format!(
		"SELECT coalesce({0}, \"events\".\"distinct_id\") AS \"person\", {bucket} AS \"period\" \
		 FROM \"events\" WHERE {returns_where} GROUP BY \"person\", \"period\" ORDER BY \
		 \"person\", \"period\" LIMIT {MAX_ROWS}",
		person_of_events()
	);
	let returns = Statement::new("retention returns", returns_sql, returns_args);

	Ok(Plan::Retention {
		cohort,
		returns,
		period: query.period,
		kind: query.kind,
		periods,
		period_starts,
	})
}

/// One web-overview stats read over `when`.
fn web_stats_statement(
	sql: &DialectSql,
	filters: &[PropertyFilter],
	when: &Range,
	label: &str,
) -> Result<Statement, PlanError> {
	let mut args = Vec::new();
	let where_clause = session_where(sql, &mut args, when, filters)?;
	let sql_text = format!(
		"SELECT count(*) AS \"sessions\", sum(\"s\".\"pageview_count\") AS \"pageviews\", \
		 count(distinct coalesce({person}, \"s\".\"distinct_id\")) AS \"visitors\", sum(CASE WHEN \
		 \"s\".\"is_bounce\" THEN 1 ELSE 0 END) AS \"bounces\", sum(\"s\".\"duration_ms\") AS \
		 \"duration\" FROM \"sessions\" \"s\" WHERE {where_clause}",
		person = person_of_sessions()
	);
	Ok(Statement::new(label.to_owned(), sql_text, args))
}

/// Plans the web overview: the sessions' totals, the top pages from
/// `$pageview`s, and the sessions' breakdown tables in one `UNION ALL`.
/// Every arm of the union reads the same `WHERE`, so they share one list
/// of parameters: SQLite's and Postgres's numbered placeholders may repeat,
/// and each number binds once.
fn web_plan(sql: &DialectSql, query: &WebQuery, now: Timestamp) -> Result<Plan, PlanError> {
	let range = resolve_range(&query.range, now)?;
	let table_limit = query.table_limit.max(1);
	let stats = web_stats_statement(sql, &query.filters, &range, "web overview")?;
	let previous_stats = if query.compare {
		Some(web_stats_statement(
			sql,
			&query.filters,
			&range.previous(),
			"web overview previous",
		)?)
	} else {
		None
	};

	let mut pages_args = Vec::new();
	let pages_where = event_where(
		sql,
		&mut pages_args,
		&range,
		&query.filters,
		Some("$pageview"),
		None,
	)?;
	let pathname = checked_property("$pathname")?;
	pages_args.push(match sql.dialect {
		Dialect::Postgres => Value::Text(pathname.key.clone()),
		Dialect::Sqlite => Value::Text(pathname.path.clone()),
	});
	let pages_sql = format!(
		"SELECT {} AS \"page\", count(*) AS \"count\" FROM \"events\" WHERE {pages_where} GROUP \
		 BY \"page\" ORDER BY \"count\" DESC, \"page\" LIMIT {table_limit}",
		sql.property_text(r#""events"."properties""#, pages_args.len())
	);
	let pages = Statement::new("web overview pages", pages_sql, pages_args);

	// The breakdown tables: one arm per session column, each the same
	// `WHERE`, so one list of parameters serves them all.
	let mut arm_args = Vec::new();
	let arm_where = session_where(sql, &mut arm_args, &range, &query.filters)?;
	let arms: Vec<String> = [
		"entry_pathname",
		"exit_pathname",
		"referrer_domain",
		"channel",
		"utm_source",
		"utm_campaign",
		"device_type",
		"browser",
		"os",
	]
	.iter()
	.map(|column| {
		format!(
			"SELECT '{column}' AS \"dimension\", \"s\".\"{column}\" AS \"value\", count(*) AS \
			 \"count\" FROM \"sessions\" \"s\" WHERE {arm_where} AND \"s\".\"{column}\" <> '' \
			 GROUP BY \"s\".\"{column}\""
		)
	})
	.collect();
	let dimensions_sql = format!(
		"SELECT \"dimension\", \"value\", \"count\" FROM ({}) AS \"tables\" ORDER BY \
		 \"dimension\", \"count\" DESC, \"value\" LIMIT {}",
		arms.join(" UNION ALL "),
		table_limit * u32::try_from(arms.len()).unwrap_or(9)
	);
	let dimensions = Statement::new("web overview tables", dimensions_sql, arm_args);

	Ok(Plan::WebOverview {
		stats,
		previous_stats,
		pages,
		dimensions,
		table_limit,
	})
}

/// The sessions table's `WHERE`: the range on `first_timestamp`, then the
/// query's filters — session filters on the row itself, event and person
/// filters through the session's events.
fn session_where(
	sql: &DialectSql,
	args: &mut Vec<Value>,
	range: &Range,
	filters: &[PropertyFilter],
) -> Result<String, PlanError> {
	let mut parts = vec![format!(
		r#""s"."first_timestamp" >= {}"#,
		sql.placeholder(args.len() + 1)
	)];
	args.push(Value::Timestamp(range.from));
	parts.push(format!(
		r#""s"."first_timestamp" < {}"#,
		sql.placeholder(args.len() + 1)
	));
	args.push(Value::Timestamp(range.to));
	for filter in filters {
		let key = checked_property(&filter.key)?;
		match filter.source {
			FilterSource::Session => {
				let Some((column, kind)) =
					SESSION_COLUMNS.iter().find(|(name, _)| *name == filter.key)
				else {
					return Err(PlanError::Column(format!(
						"{:?} isn't a session column",
						filter.key
					)));
				};
				let mut condition_args =
					sql.condition_value(filter.operator, filter.value.as_ref())?;
				args.append(&mut condition_args);
				parts.push(sql.session_condition(column, *kind, filter.operator, args.len())?);
			}
			FilterSource::Event => {
				let inner = property_condition(sql, args, r#""e"."properties""#, &key, filter)?;
				parts.push(format!(
					r#"EXISTS (SELECT 1 FROM "events" "e" WHERE "e"."session_id" = "sessions"."id" \
					   AND {inner})"#
				));
			}
			FilterSource::Person => {
				let inner = property_condition(sql, args, r#""pp"."properties""#, &key, filter)?;
				parts.push(format!(
					"{} IN (SELECT \"pp\".\"id\" FROM \"persons\" \"pp\" WHERE {inner})",
					person_of_sessions()
				));
			}
		}
	}
	Ok(parts.join(" AND "))
}

/// The events list's page, newest first, one statement.
fn events_plan(sql: &DialectSql, query: &EventsQuery, now: Timestamp) -> Result<Plan, PlanError> {
	let range = resolve_range(&query.range, now)?;
	let mut args = Vec::new();
	let mut parts = vec![format!(
		r#""events"."timestamp" >= {}"#,
		sql.placeholder(args.len() + 1)
	)];
	args.push(Value::Timestamp(range.from));
	parts.push(format!(
		r#""events"."timestamp" < {}"#,
		sql.placeholder(args.len() + 1)
	));
	args.push(Value::Timestamp(range.to));
	if let Some(event) = query.event.as_deref() {
		if event.is_empty() || event.len() > 200 {
			return Err(PlanError::Range(
				"the event's name is 1 to 200 bytes".into(),
			));
		}
		args.push(Value::Text(event.to_owned()));
		parts.push(format!(
			r#""events"."name" = {}"#,
			sql.placeholder(args.len())
		));
	}
	for filter in &query.filters {
		parts.push(filter_condition(sql, &mut args, filter)?);
	}
	if let Some(cursor) = query.cursor.as_ref() {
		let at = parse_bound(&cursor.timestamp, "cursor timestamp")?;
		args.push(Value::Timestamp(at));
		let stamp_at = args.len();
		if cursor.uuid.len() != 36 {
			return Err(PlanError::Range("the cursor's uuid isn't a UUID".into()));
		}
		args.push(Value::Text(cursor.uuid.clone()));
		parts.push(format!(
			"(\"events\".\"timestamp\", \"events\".\"uuid\") < ({}, {})",
			sql.placeholder(stamp_at),
			sql.placeholder(args.len()),
		));
	}
	let columns = if query.columns.is_empty() {
		EVENT_COLUMNS
	} else {
		query.columns.as_slice()
	};
	// The timestamp and the uuid always come first, for the order and the
	// cursor, whatever columns the page asks to show; the page's own
	// column list skips them, since they ride along anyway.
	let mut selected = String::from("\"events\".\"timestamp\", \"events\".\"uuid\"");
	for column in columns {
		if matches!(column, EventColumn::Uuid | EventColumn::Timestamp) {
			continue;
		}
		let _ = write!(selected, ", {}", column_expression(sql, *column));
	}
	let limit = query.limit.clamp(1, EventsQuery::MAX_LIMIT) + 1;
	let page_sql = format!(
		"SELECT {selected} FROM \"events\" WHERE {} ORDER BY \"events\".\"timestamp\" DESC, \
		 \"events\".\"uuid\" DESC LIMIT {limit}",
		parts.join(" AND ")
	);
	Ok(Plan::Events {
		page: Statement::new("events page", page_sql, args),
	})
}

/// Every column, in the answer's order.
pub const EVENT_COLUMNS: &[EventColumn] = &[
	EventColumn::Uuid,
	EventColumn::Name,
	EventColumn::DistinctId,
	EventColumn::PersonId,
	EventColumn::Timestamp,
	EventColumn::SessionId,
	EventColumn::Properties,
	EventColumn::Source,
];

/// One column's expression.
fn column_expression(_sql: &DialectSql, column: EventColumn) -> String {
	match column {
		EventColumn::Uuid => r#""events"."uuid""#.into(),
		EventColumn::Name => r#""events"."name""#.into(),
		EventColumn::DistinctId => r#""events"."distinct_id""#.into(),
		EventColumn::PersonId => person_of_events().to_owned(),
		EventColumn::Timestamp => r#""events"."timestamp""#.into(),
		EventColumn::SessionId => r#""events"."session_id""#.into(),
		EventColumn::Properties => r#""events"."properties""#.into(),
		EventColumn::Source => r#""events"."source""#.into(),
	}
}

/// Plans the actors of a trend bucket: the resolved persons whose events
/// fall in it. Funnel and retention points re-run their query's own plan
/// and compute their people from its rows, so only the bucket needs SQL of
/// its own.
fn actors_plan(sql: &DialectSql, query: &ActorsQuery, now: Timestamp) -> Result<Plan, PlanError> {
	let crate::query::ActorPoint::Bucket {
		series,
		bucket,
		breakdown_value,
	} = &query.point
	else {
		return match &query.query {
			Query::Funnels(funnels) => funnels_plan(sql, funnels, now),
			Query::Retention(retention) => retention_plan(sql, retention, now),
			_ => {
				Err(PlanError::Range(
					"the actors' point names a trends, funnels, or retention query".into(),
				))
			}
		};
	};
	let Query::Trends(trends) = &query.query else {
		return Err(PlanError::Range(
			"a trend bucket's actors need a trends query".into(),
		));
	};
	let Some(series) = trends
		.series
		.get(usize::try_from(*series).unwrap_or(usize::MAX))
	else {
		return Err(PlanError::Range(
			"the actors' series isn't one of the trend's".into(),
		));
	};
	// The bucket label must be one the range itself would write, so a
	// caller can't reach another time through it.
	let range = resolve_range(&trends.range, now)?;
	let label = range.wall(range.utc(bucket)?);
	let mut args = Vec::new();
	// An empty series name counts every event, as trends do.
	let event = if series.event.is_empty() {
		None
	} else {
		Some(series.event.as_str())
	};
	let where_clause = event_where(
		sql,
		&mut args,
		&range,
		&trends.filters,
		event,
		trends.test_accounts.as_ref(),
	)?;
	args.push(shift_arg(sql.dialect, &range));
	let bucket_expression = sql.bucket(trends.interval, args.len())?;
	args.push(Value::Text(label));
	let bucket_condition = format!("{bucket_expression} = {}", sql.placeholder(args.len()));
	// Every condition joins before the LIMIT, so the breakdown's bound
	// lands inside the read, not after it.
	let mut conditions = format!(
		"{where_clause} AND {bucket_condition} AND {0} IS NOT NULL",
		person_of_events()
	);
	if let Some(value) = breakdown_value {
		let Some(breakdown) = trends.breakdown.as_ref() else {
			return Err(PlanError::Range(
				"the actors name a breakdown group, and the trend doesn't break down".into(),
			));
		};
		let expression = breakdown_expression(sql, &mut args, breakdown)?;
		args.push(match sql.dialect {
			Dialect::Postgres => Value::Text(value.to_string()),
			Dialect::Sqlite => {
				match value {
					serde_json::Value::String(text) => Value::Text(text.clone()),
					other => Value::Text(other.to_string()),
				}
			}
		});
		let _ = write!(
			conditions,
			" AND {expression} = {}",
			sql.placeholder(args.len())
		);
	}
	let bucket_sql = format!(
		"SELECT DISTINCT {0} AS \"person\" FROM \"events\" WHERE {conditions} LIMIT {MAX_ROWS}",
		person_of_events()
	);
	Ok(Plan::Actors(Statement::new(
		"actors in bucket",
		bucket_sql,
		args,
	)))
}
