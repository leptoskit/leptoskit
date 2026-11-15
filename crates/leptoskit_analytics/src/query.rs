//! The insights queries (§10 of the design): what an insight stores, and
//! what the service's query engine answers.
//!
//! These types compile without the `service` feature — they are serde and
//! JSON schema only — so an app, and the admin that drives it, holds and
//! edits queries without compiling the engine. The engine itself is
//! `leptoskit_analytics::service::query`, behind `service`.
//!
//! ```
//! use leptoskit_analytics::query::*;
//!
//! let query = Query::Trends(TrendsQuery {
//! 	range: DateRange::last_days(7),
//! 	interval: Interval::Day,
//! 	series: vec!["signed_up".into()],
//! 	filters: Vec::new(),
//! 	math: Math::UniquePersons,
//! 	breakdown: None,
//! 	compare: true,
//! 	test_accounts: None,
//! });
//! let json = serde_json::to_string(&query).unwrap();
//! assert_eq!(serde_json::from_str::<Query>(&json).unwrap(), query);
//! ```

use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

/// Every insight is one of these, stored as JSON in the `insights` table
/// and answered by the engine's `run` operation. The variants here are the
/// ones step 4 builds; paths, stickiness, lifecycle, and the SQL subset
/// follow later.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Query {
	/// Events counted per interval, with math, breakdown, and compare.
	Trends(TrendsQuery),
	/// Ordered steps and the people who walk through them.
	Funnels(FunnelsQuery),
	/// A start event and a return event, as a cohort grid.
	Retention(RetentionQuery),
	/// The web dashboard, over sessions (§14, in part).
	WebOverview(WebQuery),
	/// The events list, filtered and paginated.
	Events(EventsQuery),
	/// The people behind a point of another query.
	Actors(Box<ActorsQuery>),
}

/// When a query looks: absolute bounds, or the last few days, in the
/// project's time zone.
///
/// The zone is a fixed offset from UTC (`"UTC"`, `"Z"`, `"+02:00"`,
/// `"-07:30"`), which is what day and week buckets need to start at local
/// midnight. Named IANA zones move with daylight saving, which a fixed
/// offset can't say for a whole range, so the engine refuses them for now.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DateRange {
	/// The lower bound, inclusive, as RFC 3339. Set for an absolute range.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub from: Option<String>,
	/// The upper bound, exclusive, as RFC 3339. Set for an absolute range.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub to: Option<String>,
	/// The last `last_days` days, today included. Used when `from` is
	/// absent.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub last_days: Option<u32>,
	/// A fixed offset from UTC, or `"UTC"`. Absent means UTC.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub timezone: Option<String>,
}

impl DateRange {
	/// The last `days` days, today included, in UTC.
	#[must_use]
	pub fn last_days(days: u32) -> Self {
		Self {
			last_days: Some(days.max(1)),
			..Self::default()
		}
	}

	/// An absolute range, bounds as RFC 3339; `to` is exclusive.
	#[must_use]
	pub fn between(from: impl Into<String>, to: impl Into<String>) -> Self {
		Self {
			from: Some(from.into()),
			to: Some(to.into()),
			..Self::default()
		}
	}

	/// In a fixed-offset time zone, such as `"+02:00"`.
	#[must_use]
	pub fn in_timezone(mut self, timezone: impl Into<String>) -> Self {
		self.timezone = Some(timezone.into());
		self
	}
}

/// How a trend's values group in time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Interval {
	/// Each hour.
	Hour,
	/// Each day, midnight to midnight in the range's zone.
	#[default]
	Day,
	/// Each week, Monday to Sunday.
	Week,
	/// Each calendar month.
	Month,
}

/// What a filter matches against: the event's own properties, its
/// person's, or its session's columns.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FilterSource {
	/// A key of the event's `properties` JSON.
	#[default]
	Event,
	/// A key of the event's person's `properties` JSON.
	Person,
	/// A column of the event's session.
	Session,
}

/// How a property's value is compared.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Operator {
	/// Equal, comparing as text when either side is text.
	#[default]
	Equals,
	/// Not equal.
	NotEquals,
	/// The property's text contains the value, case-insensitively.
	Contains,
	/// The property's text matches the value as a regular expression.
	Regex,
	/// Greater than, comparing as numbers.
	GreaterThan,
	/// Less than, comparing as numbers.
	LessThan,
	/// The property is present and not null.
	IsSet,
	/// The property is absent or null.
	IsNotSet,
}

/// One condition: a property of an event, a person, or a session, compared
/// with an operator. A query's filters are `ANDed`.
///
/// Values are always sent to the database as bound parameters, and the
/// property's name is checked before any SQL is built, so neither can
/// carry SQL in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PropertyFilter {
	/// What holds the property.
	#[serde(default)]
	pub source: FilterSource,
	/// The property's name, or the session column's name.
	pub key: String,
	/// How it's compared.
	#[serde(default)]
	pub operator: Operator,
	/// What to compare against. `IsSet` and `IsNotSet` take none.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub value: Option<serde_json::Value>,
}

impl PropertyFilter {
	/// A condition on the event's own property `key`.
	#[must_use]
	pub fn event(key: impl Into<String>, operator: Operator, value: serde_json::Value) -> Self {
		Self {
			source: FilterSource::Event,
			key: key.into(),
			operator,
			value: Some(value),
		}
	}

	/// A condition on a session column, such as `browser`.
	#[must_use]
	pub fn session(key: impl Into<String>, operator: Operator, value: serde_json::Value) -> Self {
		Self {
			source: FilterSource::Session,
			key: key.into(),
			operator,
			value: Some(value),
		}
	}
}

/// How a trend counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Math {
	/// How many events.
	#[default]
	Total,
	/// How many distinct people did it, a person resolved through
	/// `person_overrides` (§8). At day interval this is DAU, at week
	/// interval WAU, at month interval MAU.
	UniquePersons,
	/// How many distinct sessions it happened in.
	UniqueSessions,
}

/// Groups a series into the top values of one property, everything past
/// the limit summed as `"other"`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Breakdown {
	/// What holds the property.
	#[serde(default)]
	pub source: FilterSource,
	/// The property's name, or the session column's name.
	pub key: String,
	/// The most values shown, most frequent first. The rest sum into
	/// `"other"`.
	#[serde(default = "Breakdown::default_limit")]
	pub limit: u32,
}

impl Breakdown {
	/// An event property, with the default limit.
	#[must_use]
	pub fn event(key: impl Into<String>) -> Self {
		Self {
			source: FilterSource::Event,
			key: key.into(),
			limit: Self::default_limit(),
		}
	}

	/// [`Self::limit`] when none is given.
	#[must_use]
	pub const fn default_limit() -> u32 {
		10
	}
}

/// The test-account filter hook: which distinct ids a query drops or
/// keeps. What an app marks as its test accounts is the app's own
/// decision, so the list travels with the query rather than living in the
/// service.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TestAccounts {
	/// Drop events recorded under these distinct ids.
	Exclude(Vec<String>),
	/// Keep only events recorded under these distinct ids.
	Only(Vec<String>),
}

/// A series: one event's count over time. An empty name counts every
/// event.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Series {
	/// The event's name, or empty for every event.
	pub event: String,
}

impl From<&str> for Series {
	fn from(event: &str) -> Self {
		Self {
			event: event.to_owned(),
		}
	}
}

impl From<String> for Series {
	fn from(event: String) -> Self {
		Self { event }
	}
}

/// How events distribute over time: one series per event, each counted by
/// the query's math, optionally grouped by one property, optionally beside
/// the previous period of the same length.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TrendsQuery {
	/// When it looks.
	pub range: DateRange,
	/// How values group in time.
	#[serde(default)]
	pub interval: Interval,
	/// One series per event. At least one.
	pub series: Vec<Series>,
	/// Conditions every series' events must match.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub filters: Vec<PropertyFilter>,
	/// How each series counts.
	#[serde(default)]
	pub math: Math,
	/// One series per top value of one property, when set.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub breakdown: Option<Breakdown>,
	/// Also answer the previous period of the same length, bucket for
	/// bucket.
	#[serde(default, skip_serializing_if = "std::ops::Not::not")]
	pub compare: bool,
	/// Which test accounts to drop or keep.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub test_accounts: Option<TestAccounts>,
}

/// How each step must follow the one before.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FunnelOrder {
	/// The steps happen in order, other events allowed between them.
	#[default]
	Ordered,
	/// The next step must be the person's very next event.
	Strict,
	/// The steps happen in any order, within the window.
	Unordered,
}

/// How many people walk a sequence of events, and where they stop.
///
/// A person enters when their first event of step 1 falls in the range.
/// Each further step must happen within `window_seconds` of that entry,
/// following `order`. A person is counted once per step.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FunnelsQuery {
	/// When it looks: the range bounds step 1.
	pub range: DateRange,
	/// The steps, in order. Two to eight.
	pub steps: Vec<String>,
	/// How long a person has from entering to finish, in seconds. Default:
	/// fourteen days.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub window_seconds: Option<u64>,
	/// How each step must follow the one before.
	#[serde(default)]
	pub order: FunnelOrder,
	/// Conditions on the events the funnel reads.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub filters: Vec<PropertyFilter>,
	/// Group people by one property's value on their first step
	/// (first-touch attribution), when set.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub breakdown: Option<Breakdown>,
	/// Which test accounts to drop or keep.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub test_accounts: Option<TestAccounts>,
}

/// Whether a cohort is everyone whose first-ever start event falls in its
/// period, or everyone who started there at all.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RetentionKind {
	/// People whose first start event ever falls in the cohort's period:
	/// each person joins one cohort at most.
	#[default]
	FirstTime,
	/// Everyone who started in the cohort's period, whenever they first
	/// did: a person joins every cohort they started in.
	Recurring,
}

/// Who comes back: a cohort grid with a row for each period the range
/// holds, of the people who did the start event in it, and how many of
/// them did the return event in each later period, up to the range's end.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RetentionQuery {
	/// When it looks: the range bounds the cohorts.
	pub range: DateRange,
	/// The event that puts a person in a cohort.
	pub start_event: String,
	/// The event that counts as coming back.
	pub return_event: String,
	/// First-time or recurring cohorts. Named `retention_kind` on the wire,
	/// because [`Query`]'s own tag is `kind`.
	#[serde(default, rename = "retention_kind")]
	pub kind: RetentionKind,
	/// How wide a cohort and each return period are: day or week.
	#[serde(default)]
	pub period: Interval,
	/// How many return periods to follow each cohort into, at most: a
	/// later cohort stops at the range's end. Default: as many as the range
	/// holds after its first period, up to twelve.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub periods: Option<u32>,
	/// Conditions on the events retention reads, both events'.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub filters: Vec<PropertyFilter>,
	/// Which test accounts to drop or keep.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub test_accounts: Option<TestAccounts>,
}

/// The web dashboard (§14, in part): visitors, pageviews, sessions,
/// bounce, and duration, each beside the previous period when compared,
/// with the top pages, entry and exit pages, referrers, channels, UTM
/// parameters, and devices.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WebQuery {
	/// When it looks.
	pub range: DateRange,
	/// Also answer the previous period of the same length.
	#[serde(default, skip_serializing_if = "std::ops::Not::not")]
	pub compare: bool,
	/// Conditions on what counts: filters on sessions, and on `$pageview`
	/// events for the top pages.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub filters: Vec<PropertyFilter>,
	/// How many entries each breakdown table shows.
	#[serde(default = "WebQuery::default_table_limit")]
	pub table_limit: u32,
}

impl WebQuery {
	/// [`Self::table_limit`] when none is given.
	#[must_use]
	pub const fn default_table_limit() -> u32 {
		10
	}
}

/// Which columns the events list answers with. The default is all.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EventColumn {
	/// The event's id.
	Uuid,
	/// The event's name.
	Name,
	/// The id it was recorded under.
	DistinctId,
	/// The person it belongs to, resolved through `person_overrides` (§8).
	PersonId,
	/// When it happened.
	Timestamp,
	/// The session it happened in.
	SessionId,
	/// The event's properties, as a JSON object.
	Properties,
	/// Which kind of client sent it.
	Source,
}

/// Where a page of events continues: the last row of the page before it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EventsCursor {
	/// The last row's `timestamp`, as RFC 3339.
	pub timestamp: String,
	/// The last row's `uuid`.
	pub uuid: String,
}

/// The events list (§10): what happened, filtered, newest first,
/// paginated by cursor.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EventsQuery {
	/// When it looks.
	pub range: DateRange,
	/// One event's name, or every event when absent.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub event: Option<String>,
	/// Conditions the events must match.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub filters: Vec<PropertyFilter>,
	/// The columns to answer with. Empty means all.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub columns: Vec<EventColumn>,
	/// The most events on the page, 1 to [`Self::MAX_LIMIT`].
	#[serde(default = "EventsQuery::default_limit")]
	pub limit: u32,
	/// The page after the one before, whose last row this names.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub cursor: Option<EventsCursor>,
}

impl EventsQuery {
	/// The most events one page answers with.
	pub const MAX_LIMIT: u32 = 200;

	/// [`Self::limit`] when none is given.
	#[must_use]
	pub const fn default_limit() -> u32 {
		50
	}
}

/// Which point of another query the actors answer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "point", rename_all = "snake_case")]
pub enum ActorPoint {
	/// The people behind one bucket of a trend. `series` indexes the
	/// trend's series; `breakdown_value` names the breakdown group when
	/// the trend broke down.
	Bucket {
		series: u32,
		bucket: String,
		#[serde(default, skip_serializing_if = "Option::is_none")]
		breakdown_value: Option<serde_json::Value>,
	},
	/// The people who reached a funnel's step, 1-based.
	FunnelStep { step: u32 },
	/// The people who reached a funnel's step, 1-based, but not the next.
	FunnelDropOff { after: u32 },
	/// The cohort that started in period `start_period` (0-based) and
	/// returned in `return_period`, 0-based from the cohort's own start.
	RetentionCell {
		start_period: u32,
		return_period: u32,
	},
}

/// The people behind a point of another query (§10): a trend bucket, a
/// funnel step or its drop-off, or a retention cell.
///
/// `query` is the query the point belongs to, sent back whole so the
/// caller doesn't have to rebuild it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ActorsQuery {
	/// The query the point belongs to: trends, funnels, or retention.
	pub query: Query,
	/// Which point of it.
	pub point: ActorPoint,
	/// The most actors on the page, 1 to [`Self::MAX_LIMIT`].
	#[serde(default = "ActorsQuery::default_limit")]
	pub limit: u32,
	/// How many actors to skip, for the next pages.
	#[serde(default)]
	pub offset: u32,
}

impl ActorsQuery {
	/// The most actors one page answers with.
	pub const MAX_LIMIT: u32 = 200;

	/// [`Self::limit`] when none is given.
	#[must_use]
	pub const fn default_limit() -> u32 {
		50
	}
}

/// The SQL a query ran, with its timing: what the Workings drawer shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StatementInfo {
	/// What the statement was for, such as `"trends: signed_up"`.
	pub name: String,
	/// The SQL, as the database received it.
	pub sql: String,
	/// The bound parameters, in order, as JSON.
	pub args: Vec<serde_json::Value>,
}

/// What a query's run cost, beside its answer: the SQL it ran, how long it
/// took, and how many rows it read, as far as the database says.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct QueryInfo {
	/// Each statement, in the order it ran.
	pub statements: Vec<StatementInfo>,
	/// The database's kind: `sqlite` or `postgres`.
	#[serde(default)]
	pub dialect: String,
	/// The whole run, in milliseconds.
	pub elapsed_ms: u64,
	/// The rows the statements returned, added up: a lower bound on what
	/// the database read, which it doesn't report.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub rows_scanned_hint: Option<u64>,
}

/// One trend's values over time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TrendSeries {
	/// The event the series counts, or empty for every event.
	pub event: String,
	/// The breakdown group the series counts, when the query broke down.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub breakdown_value: Option<serde_json::Value>,
	/// Each interval's value, oldest first, gaps as zero. Each `start` is
	/// wall-clock text in the range's zone, without an offset.
	pub buckets: Vec<TrendBucket>,
	/// The range's total, the buckets' sum.
	pub total: f64,
}

/// One interval's value.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TrendBucket {
	/// The bucket's start, as `YYYY-MM-DDTHH:MM:SS` wall-clock text in the
	/// range's zone.
	pub start: String,
	pub value: f64,
}

/// A trend query's answer: the period's series, and, when compared, the
/// previous period's, in the same order.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TrendsResult {
	pub series: Vec<TrendSeries>,
	/// The previous period's series, when the query compared. Each series
	/// matches `series` by position.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub previous: Vec<TrendSeries>,
}

/// One funnel step's numbers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FunnelStepResult {
	/// The step's event name.
	pub name: String,
	/// How many people reached it.
	pub count: u64,
	/// `count` over the people who entered, 0 to 1.
	pub conversion_rate: f64,
	/// How many people who could have reached it didn't: the step before's
	/// count minus this one's.
	pub drop_off: u64,
}

/// One breakdown group's funnel, with the group's value from the person's
/// or event's first step (first-touch attribution).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FunnelBreakdown {
	pub value: serde_json::Value,
	pub steps: Vec<FunnelStepResult>,
}

/// A funnel query's answer.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FunnelsResult {
	/// Each step, in order, over everyone who entered.
	pub steps: Vec<FunnelStepResult>,
	/// The share of people who entered and finished, 0 to 1.
	pub overall_conversion: f64,
	/// The median time from the first step to the last, over the people
	/// who finished, in seconds.
	pub median_seconds_to_convert: Option<f64>,
	/// One funnel per breakdown group, when the query broke down.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub breakdown: Vec<FunnelBreakdown>,
}

/// One row of the retention grid: a cohort, and how many of it returned
/// in each later period.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RetentionCohort {
	/// The cohort period's start, as wall-clock text in the range's zone.
	pub start: String,
	/// How many people started here.
	pub size: u64,
	/// How many returned in the cohort's 0th, 1st, … later period. Index 0
	/// is the start period itself; a later cohort's list is shorter, ending
	/// where the range does.
	pub returns: Vec<u64>,
}

/// A retention query's answer.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RetentionResult {
	/// How wide a cohort is, as the query asked.
	pub period: Interval,
	pub kind: RetentionKind,
	/// One cohort per period the range holds, oldest first, so a
	/// [`ActorPoint::RetentionCell`]'s `start_period` indexes it. A period
	/// nobody started in has a cohort of size 0.
	pub cohorts: Vec<RetentionCohort>,
}

/// One number beside its previous period.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Metric {
	pub value: f64,
	/// The previous period's value, when the query compared.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub previous: Option<f64>,
	/// `value` against `previous`, as a relative change: positive up,
	/// negative down, from zero when there was none before.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub delta: Option<f64>,
}

/// A count of one thing, for a breakdown table.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NamedCount {
	/// The page, the referrer, the channel, the device: what was counted.
	pub name: String,
	pub count: u64,
}

/// The web overview's answer (§14, in part).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WebOverviewResult {
	pub visitors: Metric,
	pub pageviews: Metric,
	pub sessions: Metric,
	/// The share of sessions that bounced, 0 to 1.
	pub bounce_rate: Metric,
	/// The mean session, in seconds.
	pub avg_session_duration_seconds: Metric,
	pub top_pages: Vec<NamedCount>,
	pub entry_pages: Vec<NamedCount>,
	pub exit_pages: Vec<NamedCount>,
	pub referrers: Vec<NamedCount>,
	pub channels: Vec<NamedCount>,
	pub utm_sources: Vec<NamedCount>,
	pub utm_campaigns: Vec<NamedCount>,
	pub devices: Vec<NamedCount>,
	pub browsers: Vec<NamedCount>,
	pub operating_systems: Vec<NamedCount>,
}

/// One stored event, as the events list answers with.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EventRow {
	pub uuid: String,
	pub name: String,
	pub distinct_id: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub person_id: Option<String>,
	pub timestamp: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub session_id: Option<String>,
	/// The event's own properties, as a JSON object.
	pub properties: serde_json::Value,
	/// Which kind of client sent it, as `Source`'s number holds.
	pub source: Option<i64>,
}

/// A page of the events list, and where the next one continues.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EventsPage {
	pub events: Vec<EventRow>,
	/// The cursor for the next page, absent when this page was the last.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub next: Option<EventsCursor>,
}

/// One person, as the actors page answers with.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Actor {
	/// The person's id.
	pub id: String,
	/// Every distinct id events were recorded under for them.
	pub distinct_ids: Vec<String>,
	/// The person's properties, as a JSON object.
	pub properties: serde_json::Value,
	/// Whether the app identified this person with `$identify`.
	pub is_identified: bool,
	/// When the person was created, as RFC 3339.
	pub created_at: String,
}

/// A page of actors.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ActorsPage {
	pub actors: Vec<Actor>,
	/// The offset to ask for next, absent when this page was the last.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub next: Option<u32>,
}

/// A query's answer: the result beside [`QueryInfo`], which the Workings
/// drawer shows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Answer {
	pub info: QueryInfo,
	#[serde(flatten)]
	pub result: QueryResult,
}

/// What the engine answered, as the query kind that was run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum QueryResult {
	Trends(TrendsResult),
	Funnels(FunnelsResult),
	Retention(RetentionResult),
	/// Boxed: it holds every table, and the enum stays small beside it.
	WebOverview(Box<WebOverviewResult>),
	Events(EventsPage),
	Actors(ActorsPage),
}

impl QueryResult {
	/// The kind's name, as the JSON tag holds it.
	#[must_use]
	pub fn kind(&self) -> &'static str {
		match self {
			Self::Trends(_) => "trends",
			Self::Funnels(_) => "funnels",
			Self::Retention(_) => "retention",
			Self::WebOverview(_) => "web_overview",
			Self::Events(_) => "events",
			Self::Actors(_) => "actors",
		}
	}
}
