//! A query as code: the typed `Query` value as Rust you can paste into your
//! own server, the SQL it ran, and the HTTP call that runs it. What the Ask
//! station's three tabs show.

use std::fmt::Write as _;

use leptoskit_analytics::query::Breakdown;
use leptoskit_analytics::query::DateRange;
use leptoskit_analytics::query::FilterSource;
use leptoskit_analytics::query::FunnelOrder;
use leptoskit_analytics::query::Interval;
use leptoskit_analytics::query::Math;
use leptoskit_analytics::query::Operator;
use leptoskit_analytics::query::PropertyFilter;
use leptoskit_analytics::query::Query;
use leptoskit_analytics::query::QueryInfo;
use leptoskit_analytics::query::RetentionKind;

fn string(text: &str) -> String {
	serde_json::to_string(text).unwrap_or_else(|_| format!("{text:?}"))
}

fn range(range: &DateRange) -> String {
	let mut text = match (&range.from, &range.to) {
		(Some(from), Some(to)) => format!("DateRange::between({}, {})", string(from), string(to)),
		_ => format!("DateRange::last_days({})", range.last_days.unwrap_or(30)),
	};

	if let Some(zone) = &range.timezone {
		let _ = write!(text, ".in_timezone({})", string(zone));
	}

	text
}

fn interval(interval: Interval) -> &'static str {
	match interval {
		Interval::Hour => "Interval::Hour",
		Interval::Day => "Interval::Day",
		Interval::Week => "Interval::Week",
		Interval::Month => "Interval::Month",
	}
}

fn source(source: FilterSource) -> &'static str {
	match source {
		FilterSource::Event => "FilterSource::Event",
		FilterSource::Person => "FilterSource::Person",
		FilterSource::Session => "FilterSource::Session",
	}
}

fn operator(operator: Operator) -> &'static str {
	match operator {
		Operator::Equals => "Operator::Equals",
		Operator::NotEquals => "Operator::NotEquals",
		Operator::Contains => "Operator::Contains",
		Operator::Regex => "Operator::Regex",
		Operator::GreaterThan => "Operator::GreaterThan",
		Operator::LessThan => "Operator::LessThan",
		Operator::IsSet => "Operator::IsSet",
		Operator::IsNotSet => "Operator::IsNotSet",
	}
}

fn breakdown(breakdown: Option<&Breakdown>) -> String {
	match breakdown {
		None => "None".to_owned(),
		Some(breakdown) if breakdown.source == FilterSource::Event => {
			format!(
				"Some(Breakdown {{ limit: {}, ..Breakdown::event({}) }})",
				breakdown.limit,
				string(&breakdown.key)
			)
		}
		Some(breakdown) => {
			format!(
				"Some(Breakdown {{ source: {}, key: {}.into(), limit: {} }})",
				source(breakdown.source),
				string(&breakdown.key),
				breakdown.limit
			)
		}
	}
}

fn filters(lines: &mut Vec<String>, filters: &[PropertyFilter]) {
	if filters.is_empty() {
		lines.push("    filters: Vec::new(),".into());
		return;
	}

	lines.push("    filters: vec![".into());

	for filter in filters {
		let value = filter.value.as_ref().map_or_else(
			|| "None".to_owned(),
			|value| format!("Some(serde_json::json!({value}))"),
		);
		lines.push(format!(
			"        PropertyFilter {{ source: {}, key: {}.into(), operator: {}, value: {value} \
			 }},",
			source(filter.source),
			string(&filter.key),
			operator(filter.operator),
		));
	}

	lines.push("    ],".into());
}

fn names(lines: &mut Vec<String>, field: &str, names: &[String]) {
	lines.push(format!("    {field}: vec!["));

	for name in names {
		lines.push(format!("        {}.into(),", string(name)));
	}

	lines.push("    ],".into());
}

/// `query` as Rust: the `use`, then `let query = …;`.
pub(crate) fn rust(query: &Query) -> Vec<String> {
	let mut lines = vec![
		"use leptoskit_analytics::query::*;".to_owned(),
		String::new(),
	];

	match query {
		Query::Trends(trends) => {
			lines.push("let query = Query::Trends(TrendsQuery {".into());
			lines.push(format!("    range: {},", range(&trends.range)));
			lines.push(format!("    interval: {},", interval(trends.interval)));
			names(
				&mut lines,
				"series",
				&trends
					.series
					.iter()
					.map(|series| series.event.clone())
					.collect::<Vec<_>>(),
			);
			filters(&mut lines, &trends.filters);
			lines.push(format!(
				"    math: {},",
				match trends.math {
					Math::Total => "Math::Total",
					Math::UniquePersons => "Math::UniquePersons",
					Math::UniqueSessions => "Math::UniqueSessions",
				}
			));
			lines.push(format!(
				"    breakdown: {},",
				breakdown(trends.breakdown.as_ref())
			));
			lines.push(format!("    compare: {},", trends.compare));
			lines.push("    test_accounts: None,".into());
		}
		Query::Funnels(funnels) => {
			lines.push("let query = Query::Funnels(FunnelsQuery {".into());
			lines.push(format!("    range: {},", range(&funnels.range)));
			names(&mut lines, "steps", &funnels.steps);
			lines.push(format!(
				"    window_seconds: {},",
				funnels
					.window_seconds
					.map_or_else(|| "None".to_owned(), |seconds| format!("Some({seconds})"))
			));
			lines.push(format!(
				"    order: {},",
				match funnels.order {
					FunnelOrder::Ordered => "FunnelOrder::Ordered",
					FunnelOrder::Strict => "FunnelOrder::Strict",
					FunnelOrder::Unordered => "FunnelOrder::Unordered",
				}
			));
			filters(&mut lines, &funnels.filters);
			lines.push(format!(
				"    breakdown: {},",
				breakdown(funnels.breakdown.as_ref())
			));
			lines.push("    test_accounts: None,".into());
		}
		Query::Retention(retention) => {
			lines.push("let query = Query::Retention(RetentionQuery {".into());
			lines.push(format!("    range: {},", range(&retention.range)));
			lines.push(format!(
				"    start_event: {}.into(),",
				string(&retention.start_event)
			));
			lines.push(format!(
				"    return_event: {}.into(),",
				string(&retention.return_event)
			));
			lines.push(format!(
				"    kind: {},",
				match retention.kind {
					RetentionKind::FirstTime => "RetentionKind::FirstTime",
					RetentionKind::Recurring => "RetentionKind::Recurring",
				}
			));
			lines.push(format!("    period: {},", interval(retention.period)));
			lines.push(format!(
				"    periods: {},",
				retention
					.periods
					.map_or_else(|| "None".to_owned(), |periods| format!("Some({periods})"))
			));
			filters(&mut lines, &retention.filters);
			lines.push("    test_accounts: None,".into());
		}
		Query::WebOverview(web) => {
			lines.push("let query = Query::WebOverview(WebQuery {".into());
			lines.push(format!("    range: {},", range(&web.range)));
			lines.push(format!("    compare: {},", web.compare));
			filters(&mut lines, &web.filters);
			lines.push(format!("    table_limit: {},", web.table_limit));
		}
		Query::Events(events) => {
			lines.push("let query = Query::Events(EventsQuery {".into());
			lines.push(format!("    range: {},", range(&events.range)));
			lines.push(format!(
				"    event: {},",
				events.event.as_deref().map_or_else(
					|| "None".to_owned(),
					|event| format!("Some({}.into())", string(event))
				)
			));
			filters(&mut lines, &events.filters);
			lines.push("    columns: Vec::new(),".into());
			lines.push(format!("    limit: {},", events.limit));
			lines.push("    cursor: None,".into());
		}
		Query::Actors(actors) => {
			lines
				.push("// The people behind one point of the query above, sent back whole:".into());
			lines.push("let query = Query::Actors(Box::new(ActorsQuery {".into());
			lines.push("    query,".into());
			lines.push(format!(
				"    point: serde_json::from_value(serde_json::json!({}))?,",
				serde_json::to_string(&actors.point).unwrap_or_default()
			));
			lines.push(format!("    limit: {},", actors.limit));
			lines.push(format!("    offset: {},", actors.offset));
			lines.push("}));".into());
			return lines;
		}
	}

	lines.push("});".into());
	lines.push(String::new());
	lines.push("// From the app's server, as the app:".into());
	lines.push("let answer = rpc::client::call_as_app(Run { query }).await?;".into());
	lines
}

/// Breaks a statement before its clauses, so it reads down the page.
fn clauses(sql: &str) -> Vec<String> {
	const BREAKS: [&str; 11] = [
		" FROM ",
		" LEFT JOIN ",
		" INNER JOIN ",
		" JOIN ",
		" WHERE ",
		" GROUP BY ",
		" HAVING ",
		" ORDER BY ",
		" LIMIT ",
		" UNION ALL ",
		" WITH ",
	];
	let mut text = sql.to_owned();

	for clause in BREAKS {
		text = text.replace(clause, &format!("\n{}", clause.trim_start()));
	}

	text = text.replace(" AND ", "\n  AND ");
	text.lines()
		.map(str::trim_end)
		.filter(|line| !line.is_empty())
		.map(str::to_owned)
		.collect()
}

/// The SQL the run sent, statement by statement, with its bound values.
pub(crate) fn sql(info: &QueryInfo) -> Vec<String> {
	let mut lines = Vec::new();

	for (index, statement) in info.statements.iter().enumerate() {
		if index > 0 {
			lines.push(String::new());
		}

		lines.push(format!("-- {} ({})", statement.name, info.dialect));
		lines.extend(clauses(&statement.sql));

		if !statement.args.is_empty() {
			let args: Vec<String> = statement.args.iter().map(ToString::to_string).collect();
			lines.push(format!("-- bound: {}", args.join(", ")));
		}
	}

	if lines.is_empty() {
		lines.push("-- this answer ran no SQL".into());
	}

	lines
}

/// The same query over HTTP, as `curl`, signed as the app.
pub(crate) fn http(query: &Query, url: &str) -> Vec<String> {
	let body =
		serde_json::to_string_pretty(&serde_json::json!({ "query": query })).unwrap_or_default();
	let mut lines = vec![
		format!(
			"curl -X POST {url}{}/rpc/run \\",
			leptoskit_rpc::service_prefix(leptoskit_analytics::SERVICE)
		),
		"  -H \"authorization: Bearer $LEPTOSKIT_SERVICE_ANALYTICS_TOKEN\" \\".to_owned(),
		"  -H \"content-type: application/json\" \\".to_owned(),
		"  -d @- <<'JSON'".to_owned(),
	];
	lines.extend(body.lines().map(str::to_owned));
	lines.push("JSON".to_owned());
	lines
}

#[cfg(test)]
mod tests {
	use leptoskit_analytics::query::TrendsQuery;

	use super::*;

	#[test]
	fn the_rust_names_every_field() {
		let query = Query::Trends(TrendsQuery {
			range: DateRange::last_days(30),
			interval: Interval::Day,
			series: vec!["checkout_started".into()],
			filters: vec![PropertyFilter::event(
				"plan",
				Operator::Equals,
				serde_json::json!("pro"),
			)],
			math: Math::UniquePersons,
			breakdown: Some(Breakdown::event("plan")),
			compare: true,
			test_accounts: None,
		});
		let rust = rust(&query).join("\n");
		assert!(rust.contains("range: DateRange::last_days(30),"), "{rust}");
		assert!(rust.contains("\"checkout_started\".into(),"), "{rust}");
		assert!(rust.contains("math: Math::UniquePersons,"), "{rust}");
		assert!(
			rust.contains(
				"breakdown: Some(Breakdown { limit: 10, ..Breakdown::event(\"plan\") }),"
			),
			"{rust}"
		);
		assert!(
			rust.contains("value: Some(serde_json::json!(\"pro\"))"),
			"{rust}"
		);
	}

	#[test]
	fn sql_reads_down_the_page() {
		assert_eq!(
			clauses("SELECT a FROM events WHERE x = ?1 AND y = ?2 GROUP BY a"),
			[
				"SELECT a",
				"FROM events",
				"WHERE x = ?1",
				"  AND y = ?2",
				"GROUP BY a"
			]
		);
	}
}
