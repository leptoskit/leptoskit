//! An insight's query as the page's address holds it, and back.
//!
//! Every builder control is a link to the same page with one parameter
//! changed, so a query is always a deep link, the back button undoes a
//! change, and the page works without its script. The script only fetches
//! the new page in place, so the Workings drawer can wash the lines that
//! changed.
//!
//! - `days=30`, or `from=2026-11-02&to=2026-11-09` (UTC dates, `to` exclusive)
//! - series and steps as `s0`, `s1`, … (event names)
//! - `math=total|people|sessions`, `interval=hour|day|week|month`,
//!   `compare=1|0`
//! - `by=plan` (an event property), `by=person:plan`, `by=session:browser`
//! - funnels: `window` (days), `order=ordered|strict|any`
//! - retention: `start`, `return`, `kind=first|recurring`, `period=day|week`,
//!   `periods`
//! - filters as `f0=session:channel=Direct`, …

use std::collections::BTreeMap;

use leptoskit_analytics::query::Breakdown;
use leptoskit_analytics::query::DateRange;
use leptoskit_analytics::query::FilterSource;
use leptoskit_analytics::query::FunnelOrder;
use leptoskit_analytics::query::FunnelsQuery;
use leptoskit_analytics::query::Interval;
use leptoskit_analytics::query::Math;
use leptoskit_analytics::query::Operator;
use leptoskit_analytics::query::PropertyFilter;
use leptoskit_analytics::query::Query;
use leptoskit_analytics::query::RetentionKind;
use leptoskit_analytics::query::RetentionQuery;
use leptoskit_analytics::query::TrendsQuery;
use leptoskit_analytics::query::WebQuery;
use leptoskit_analytics::service::query::EventDefinitionView;

use super::format::encode;

/// The page's query parameters.
pub(crate) type Params = BTreeMap<String, String>;

/// The most series a trend holds, and steps a funnel.
pub(crate) const MAX_SERIES: usize = 6;
pub(crate) const MAX_STEPS: usize = 8;

/// What a new insight starts from, read from the event catalogue.
#[derive(Clone, Debug, Default)]
pub(crate) struct Defaults {
	pub trend: String,
	pub steps: Vec<String>,
	pub start: String,
	pub back: String,
}

impl Defaults {
	/// The app's own events, most stored first, stand in for what it cares
	/// about: a checkout's steps are stored less and less often, so the
	/// most stored few make a sensible first funnel.
	pub(crate) fn from(definitions: &[EventDefinitionView]) -> Self {
		let own: Vec<&str> = definitions
			.iter()
			.filter(|definition| !definition.name.starts_with('$'))
			.map(|definition| definition.name.as_str())
			.collect();
		let first = own
			.iter()
			.find(|name| {
				definitions
					.iter()
					.any(|definition| definition.name == **name && definition.fields.is_some())
			})
			.or(own.first())
			.map_or("$pageview", |name| name);
		let mut steps: Vec<String> = own
			.iter()
			.filter(|name| name.contains("checkout") || name.contains("payment"))
			.map(|name| (*name).to_owned())
			.take(4)
			.collect();

		if steps.len() < 2 {
			steps = own.iter().take(3).map(|name| (*name).to_owned()).collect();
		}

		if steps.len() < 2 {
			steps = vec!["$pageview".to_owned(), first.to_owned()];
		}

		let start = own
			.iter()
			.find(|name| name.contains("sign"))
			.or(own.first())
			.map_or("$pageview", |name| name)
			.to_owned();
		let back = own
			.iter()
			.find(|name| **name != start && !name.contains("checkout"))
			.map_or("$pageview", |name| name)
			.to_owned();
		Self {
			trend: first.to_owned(),
			steps,
			start,
			back,
		}
	}
}

/// The range a page asks for: `from`/`to` dates, else the last `days`.
pub(crate) fn range(params: &Params) -> DateRange {
	let date = |key: &str| {
		params.get(key).and_then(|value| {
			chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
				.ok()
				.map(|date| format!("{date}T00:00:00Z"))
		})
	};

	match (date("from"), date("to")) {
		(Some(from), Some(to)) if from < to => DateRange::between(from, to),
		_ => {
			DateRange::last_days(
				params
					.get("days")
					.and_then(|days| days.parse().ok())
					.filter(|days| (1..=366).contains(days))
					.unwrap_or(30),
			)
		}
	}
}

/// How the range reads: `Last 30 days`, `2 Nov – 9 Nov 2026`.
pub(crate) fn range_label(range: &DateRange) -> String {
	match (&range.from, &range.to) {
		(Some(from), Some(to)) => {
			// `to` is exclusive: a range to 2 November ends on the 1st.
			let day = |text: &str, back: u64| {
				chrono::NaiveDate::parse_from_str(text.get(..10).unwrap_or(text), "%Y-%m-%d")
					.ok()
					.and_then(|date| date.checked_sub_days(chrono::Days::new(back)))
					.map_or_else(|| text.to_owned(), |date| date.format("%-d %b").to_string())
			};
			format!("{} – {}", day(from, 0), day(to, 1))
		}
		_ => format!("Last {} days", range.last_days.unwrap_or(30)),
	}
}

/// The range inside a sentence: `last 30 days`, `5 Oct – 1 Nov`.
pub(crate) fn range_phrase(range: &DateRange) -> String {
	let label = range_label(range);

	match label.strip_prefix("Last") {
		Some(rest) => format!("last{rest}"),
		None => label,
	}
}

fn put_range(params: &mut Params, range: &DateRange) {
	match (&range.from, &range.to) {
		(Some(from), Some(to)) => {
			params.insert("from".into(), from.get(..10).unwrap_or(from).to_owned());
			params.insert("to".into(), to.get(..10).unwrap_or(to).to_owned());
		}
		_ => {
			params.insert("days".into(), range.last_days.unwrap_or(30).to_string());
		}
	}
}

/// The numbered list `s0`, `s1`, … in order.
fn numbered(params: &Params, prefix: &str, most: usize) -> Vec<String> {
	(0..most)
		.filter_map(|index| params.get(&format!("{prefix}{index}")))
		.map(|value| value.trim().to_owned())
		.filter(|value| !value.is_empty())
		.collect()
}

fn source_of(prefix: &str) -> Option<FilterSource> {
	match prefix {
		"event" => Some(FilterSource::Event),
		"person" => Some(FilterSource::Person),
		"session" => Some(FilterSource::Session),
		_ => None,
	}
}

fn source_name(source: FilterSource) -> &'static str {
	match source {
		FilterSource::Event => "event",
		FilterSource::Person => "person",
		FilterSource::Session => "session",
	}
}

/// `by=plan`, `by=session:browser`.
fn breakdown(params: &Params) -> Option<Breakdown> {
	let by = params.get("by")?.trim();

	if by.is_empty() {
		return None;
	}

	let (source, key) = match by.split_once(':') {
		Some((prefix, key)) if source_of(prefix).is_some() => (source_of(prefix)?, key),
		_ => (FilterSource::Event, by),
	};
	Some(Breakdown {
		source,
		key: key.to_owned(),
		limit: 6,
	})
}

/// How a breakdown is written in the address.
pub(crate) fn breakdown_param(breakdown: &Breakdown) -> String {
	match breakdown.source {
		FilterSource::Event => breakdown.key.clone(),
		source => format!("{}:{}", source_name(source), breakdown.key),
	}
}

/// `f0=session:channel=Direct`.
fn filters(params: &Params) -> Vec<PropertyFilter> {
	numbered(params, "f", 4)
		.iter()
		.filter_map(|filter| {
			let (target, value) = filter.split_once('=')?;
			let (source, key) = target.split_once(':')?;
			Some(PropertyFilter {
				source: source_of(source)?,
				key: key.to_owned(),
				operator: Operator::Equals,
				value: Some(serde_json::Value::String(value.to_owned())),
			})
		})
		.collect()
}

fn put_filters(params: &mut Params, filters: &[PropertyFilter]) {
	for (index, filter) in filters.iter().enumerate() {
		let value = filter
			.value
			.as_ref()
			.map(super::format::value)
			.unwrap_or_default();
		params.insert(
			format!("f{index}"),
			format!("{}:{}={value}", source_name(filter.source), filter.key),
		);
	}
}

fn compare(params: &Params, default: bool) -> bool {
	params
		.get("compare")
		.map_or(default, |value| value == "1" || value == "true")
}

pub(crate) fn trends(params: &Params, defaults: &Defaults) -> TrendsQuery {
	let mut series = numbered(params, "s", MAX_SERIES);

	if series.is_empty() {
		series.push(defaults.trend.clone());
	}

	TrendsQuery {
		range: range(params),
		interval: match params.get("interval").map(String::as_str) {
			Some("hour") => Interval::Hour,
			Some("week") => Interval::Week,
			Some("month") => Interval::Month,
			_ => Interval::Day,
		},
		series: series.into_iter().map(Into::into).collect(),
		filters: filters(params),
		math: match params.get("math").map(String::as_str) {
			Some("people") => Math::UniquePersons,
			Some("sessions") => Math::UniqueSessions,
			_ => Math::Total,
		},
		breakdown: breakdown(params),
		compare: compare(params, true),
		test_accounts: None,
	}
}

pub(crate) fn funnels(params: &Params, defaults: &Defaults) -> FunnelsQuery {
	let mut steps = numbered(params, "s", MAX_STEPS);

	if steps.len() < 2 {
		steps.clone_from(&defaults.steps);
	}

	FunnelsQuery {
		range: range(params),
		steps,
		window_seconds: Some(
			params
				.get("window")
				.and_then(|days| days.parse::<u64>().ok())
				.filter(|days| (1..=90).contains(days))
				.unwrap_or(14)
				* 86_400,
		),
		order: match params.get("order").map(String::as_str) {
			Some("strict") => FunnelOrder::Strict,
			Some("any") => FunnelOrder::Unordered,
			_ => FunnelOrder::Ordered,
		},
		filters: filters(params),
		breakdown: breakdown(params),
		test_accounts: None,
	}
}

pub(crate) fn retention(params: &Params, defaults: &Defaults) -> RetentionQuery {
	let pick = |key: &str, default: &str| {
		params
			.get(key)
			.map(|value| value.trim().to_owned())
			.filter(|value| !value.is_empty())
			.unwrap_or_else(|| default.to_owned())
	};
	RetentionQuery {
		range: range(params),
		start_event: pick("start", &defaults.start),
		return_event: pick("return", &defaults.back),
		kind: match params.get("kind").map(String::as_str) {
			Some("recurring") => RetentionKind::Recurring,
			_ => RetentionKind::FirstTime,
		},
		period: match params.get("period").map(String::as_str) {
			Some("day") => Interval::Day,
			_ => Interval::Week,
		},
		periods: params
			.get("periods")
			.and_then(|periods| periods.parse().ok())
			.filter(|periods| (1..=12).contains(periods)),
		filters: filters(params),
		test_accounts: None,
	}
}

pub(crate) fn web(params: &Params) -> WebQuery {
	WebQuery {
		range: range(params),
		compare: compare(params, true),
		filters: filters(params),
		table_limit: 6,
	}
}

/// Whether the address names a query at all, rather than leaving it to a
/// saved insight or the defaults.
pub(crate) fn has_query(params: &Params) -> bool {
	params.keys().any(|key| {
		key.starts_with('s')
			|| matches!(
				key.as_str(),
				"math"
					| "interval"
					| "by"
					| "compare"
					| "window"
					| "order"
					| "start"
					| "return"
					| "kind"
					| "period"
					| "periods"
					| "days"
					| "from"
					| "to"
			)
	})
}

/// The parameters that hold `query`.
pub(crate) fn of(query: &Query) -> Params {
	let mut params = Params::new();

	match query {
		Query::Trends(trends) => {
			put_range(&mut params, &trends.range);
			for (index, series) in trends.series.iter().enumerate() {
				params.insert(format!("s{index}"), series.event.clone());
			}
			params.insert(
				"math".into(),
				match trends.math {
					Math::Total => "total",
					Math::UniquePersons => "people",
					Math::UniqueSessions => "sessions",
				}
				.into(),
			);
			params.insert(
				"interval".into(),
				match trends.interval {
					Interval::Hour => "hour",
					Interval::Day => "day",
					Interval::Week => "week",
					Interval::Month => "month",
				}
				.into(),
			);
			params.insert("compare".into(), u8::from(trends.compare).to_string());
			if let Some(breakdown) = &trends.breakdown {
				params.insert("by".into(), breakdown_param(breakdown));
			}
			put_filters(&mut params, &trends.filters);
		}
		Query::Funnels(funnels) => {
			put_range(&mut params, &funnels.range);
			for (index, step) in funnels.steps.iter().enumerate() {
				params.insert(format!("s{index}"), step.clone());
			}
			params.insert(
				"window".into(),
				(funnels.window_seconds.unwrap_or(14 * 86_400) / 86_400).to_string(),
			);
			params.insert(
				"order".into(),
				match funnels.order {
					FunnelOrder::Ordered => "ordered",
					FunnelOrder::Strict => "strict",
					FunnelOrder::Unordered => "any",
				}
				.into(),
			);
			if let Some(breakdown) = &funnels.breakdown {
				params.insert("by".into(), breakdown_param(breakdown));
			}
			put_filters(&mut params, &funnels.filters);
		}
		Query::Retention(retention) => {
			put_range(&mut params, &retention.range);
			params.insert("start".into(), retention.start_event.clone());
			params.insert("return".into(), retention.return_event.clone());
			params.insert(
				"kind".into(),
				match retention.kind {
					RetentionKind::FirstTime => "first",
					RetentionKind::Recurring => "recurring",
				}
				.into(),
			);
			params.insert(
				"period".into(),
				if retention.period == Interval::Day {
					"day"
				} else {
					"week"
				}
				.into(),
			);
			if let Some(periods) = retention.periods {
				params.insert("periods".into(), periods.to_string());
			}
			put_filters(&mut params, &retention.filters);
		}
		Query::WebOverview(web) => {
			put_range(&mut params, &web.range);
			params.insert("compare".into(), u8::from(web.compare).to_string());
			put_filters(&mut params, &web.filters);
		}
		Query::Events(_) | Query::Actors(_) => {}
	}

	params
}

/// `path?key=value&…`.
pub(crate) fn href(path: &str, params: &Params) -> String {
	if params.is_empty() {
		return path.to_owned();
	}

	let query: Vec<String> = params
		.iter()
		.map(|(key, value)| format!("{}={}", encode(key), encode(value)))
		.collect();
	format!("{path}?{}", query.join("&"))
}

/// `params` with `key` set to `value`.
pub(crate) fn with(params: &Params, key: &str, value: impl Into<String>) -> Params {
	let mut params = params.clone();
	params.insert(key.to_owned(), value.into());
	params
}

/// `params` without `key`.
pub(crate) fn without(params: &Params, key: &str) -> Params {
	let mut params = params.clone();
	params.remove(key);
	params
}

/// A numbered list replaced: `s0`, `s1`, … become `items`.
pub(crate) fn with_list(params: &Params, prefix: &str, items: &[String]) -> Params {
	let mut params = params.clone();
	params.retain(|key, _| {
		!(key.starts_with(prefix) && key[prefix.len()..].chars().all(|c| c.is_ascii_digit()))
	});

	for (index, item) in items.iter().enumerate() {
		params.insert(format!("{prefix}{index}"), item.clone());
	}

	params
}

/// A filter's parameter value, for a bar list row that filters the page.
pub(crate) fn filter_value(source: FilterSource, key: &str, value: &str) -> String {
	format!("{}:{key}={value}", source_name(source))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_query_survives_the_address() {
		let defaults = Defaults {
			trend: "signed_up".into(),
			steps: vec!["a".into(), "b".into()],
			start: "a".into(),
			back: "b".into(),
		};
		let mut params = Params::new();
		params.insert("s0".into(), "checkout_started".into());
		params.insert("s1".into(), "checkout_completed".into());
		params.insert("by".into(), "session:browser".into());
		params.insert("math".into(), "people".into());
		params.insert("from".into(), "2026-11-02".into());
		params.insert("to".into(), "2026-11-09".into());
		params.insert("f0".into(), "event:plan=pro".into());
		let query = Query::Trends(trends(&params, &defaults));
		assert_eq!(Query::Trends(trends(&of(&query), &defaults)), query);

		let funnel = Query::Funnels(funnels(&Params::new(), &defaults));
		assert_eq!(Query::Funnels(funnels(&of(&funnel), &defaults)), funnel);

		let back = Query::Retention(retention(&Params::new(), &defaults));
		assert_eq!(Query::Retention(retention(&of(&back), &defaults)), back);
	}

	#[test]
	fn lists_are_replaced_whole() {
		let mut params = Params::new();
		params.insert("s0".into(), "a".into());
		params.insert("s1".into(), "b".into());
		params.insert("start".into(), "x".into());
		let replaced = with_list(&params, "s", &["c".to_owned()]);
		assert_eq!(replaced.get("s0").map(String::as_str), Some("c"));
		assert!(!replaced.contains_key("s1"));
		assert!(replaced.contains_key("start"), "only numbered keys go");
	}
}
