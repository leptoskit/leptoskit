//! The web overview: visitors, pageviews, sessions, bounce rate, and
//! session length, with the tables beside them. Every site has it without
//! setup, because the web client sends `$pageview` by itself. The server
//! runs its queries; the pages crate draws it.

use leptoskit_admin_pages::model::Page;
use leptoskit_admin_pages::model::analytics::Web;
use leptoskit_analytics::query::Answer;
use leptoskit_analytics::query::Math;
use leptoskit_analytics::query::Query;
use leptoskit_analytics::query::QueryResult;
use leptoskit_analytics::query::TrendsQuery;
use leptoskit_analytics::query::WebOverviewResult;

use super::Context;
use super::Shown;
use super::format;
use super::insights;
use super::params;
use super::params::Params;
use super::stations;
use super::stations::Drawn;
use crate::BASE;
use crate::shell::Section;

const PATH: &str = "/analytics/overview";

fn path() -> String {
	format!("{BASE}{PATH}")
}

/// The daily trend of `$pageview` the tiles and chart draw, by `math`.
fn daily(params: &Params, math: Math) -> TrendsQuery {
	let web = params::web(params);
	TrendsQuery {
		range: web.range,
		interval: leptoskit_analytics::query::Interval::Day,
		series: vec!["$pageview".into()],
		filters: web.filters,
		math,
		breakdown: None,
		compare: web.compare,
		test_accounts: None,
	}
}

fn values(answer: &Result<Answer, super::Failure>) -> Option<Vec<f64>> {
	match answer {
		Ok(Answer {
			result: QueryResult::Trends(result),
			..
		}) => {
			result
				.series
				.first()
				.map(|series| series.buckets.iter().map(|bucket| bucket.value).collect())
		}
		_ => None,
	}
}

/// The overview page.
pub(crate) async fn page(cx: &Context<'_>) -> Shown {
	if !super::has_events(cx.facts) {
		return super::listening(
			"Web analytics",
			Section::Overview,
			"Visitors, pageviews, sessions, and bounce rate for your site, with the top pages, \
			 referrers, and devices beside them.",
		)
		.into();
	}

	let web = params::web(cx.params);
	let params = params::of(&Query::WebOverview(web.clone()));
	let here = params::href(&path(), &params);
	let web_query = Query::WebOverview(web.clone());
	let visitors_query = Query::Trends(daily(&params, Math::UniquePersons));
	let (overview, visitors, pageviews, sessions, sample) = tokio::join!(
		cx.connection.run(web_query.clone()),
		cx.connection.run(visitors_query.clone()),
		cx.connection
			.run(Query::Trends(daily(&params, Math::Total))),
		cx.connection
			.run(Query::Trends(daily(&params, Math::UniqueSessions))),
		insights::sample(cx.connection, Some("$pageview".to_owned())),
	);
	let overview_result: Option<&WebOverviewResult> = match &overview {
		Ok(Answer {
			result: QueryResult::WebOverview(result),
			..
		}) => Some(result),
		_ => None,
	};
	let Some(result) = overview_result else {
		let failure = match &overview {
			Err(failure) => failure.clone(),
			Ok(_) => super::Failure("The service answered a different kind of query.".into()),
		};
		return super::unreachable("Web analytics", Section::Overview, &failure).into();
	};
	let visitors_drawn = match &visitors {
		Ok(Answer {
			result: QueryResult::Trends(trend),
			..
		}) => insights::trend_view(&visitors_query, trend, &here, false, Some("Visitors")).drawn,
		Ok(_) | Err(_) => {
			Drawn {
				sentence: "Nothing was drawn.".into(),
				rows: Vec::new(),
			}
		}
	};
	let web_rows = {
		let mut table = vec![vec![
			"metric".to_owned(),
			"value".to_owned(),
			"previous".to_owned(),
		]];
		for (name, metric) in [
			("visitors", &result.visitors),
			("pageviews", &result.pageviews),
			("sessions", &result.sessions),
			("bounce_rate", &result.bounce_rate),
			("avg_session_s", &result.avg_session_duration_seconds),
		] {
			table.push(vec![
				name.to_owned(),
				format::number(metric.value),
				metric.previous.map_or_else(|| "—".into(), format::number),
			]);
		}
		stations::table(&table)
	};
	let drawers = vec![
		stations::drawer(stations::Source {
			id: "web",
			title: format!("Web overview · {} · live", params::range_label(&web.range)),
			query: &web_query,
			answer: overview.as_ref().ok(),
			definitions: &cx.facts.definitions,
			sample: sample.as_ref(),
			url: cx.connection.url(),
			drawn: Drawn {
				sentence: "One row per figure, then one per table row: the tiles show the value \
				           and its change against the period before; each table is a bar list, \
				           its bar the share of the largest row. Bounce rate and session length \
				           have no daily series yet, so their tiles have no sparkline."
					.to_owned(),
				rows: web_rows,
			},
		}),
		stations::drawer(stations::Source {
			id: "visitors",
			title: "Visitors over time · Trends insight · live".to_owned(),
			query: &visitors_query,
			answer: visitors.as_ref().ok(),
			definitions: &cx.facts.definitions,
			sample: sample.as_ref(),
			url: cx.connection.url(),
			drawn: visitors_drawn,
		}),
	];

	Page::Web(Box::new(Web {
		address: cx.params.clone(),
		query: web_query,
		result: result.clone(),
		visitors_query,
		pageviews: values(&pageviews),
		sessions: values(&sessions),
		visitors: visitors
			.map(|answer| answer.result)
			.map_err(|failure| failure.0),
		channels_note: cx.preferences.note("channels"),
		drawers,
	}))
	.into()
}
