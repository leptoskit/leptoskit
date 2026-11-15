//! The web overview: visitors, pageviews, sessions, bounce rate, and
//! session length, with the tables beside them. Every site has it without
//! setup, because the web client sends `$pageview` by itself.

use leptoskit::prelude::*;
use leptoskit_analytics::query::Answer;
use leptoskit_analytics::query::Math;
use leptoskit_analytics::query::NamedCount;
use leptoskit_analytics::query::Query;
use leptoskit_analytics::query::QueryResult;
use leptoskit_analytics::query::TrendsQuery;
use leptoskit_analytics::query::WebOverviewResult;

use super::Context;
use super::draw;
use super::format;
use super::insights;
use super::params;
use super::params::Params;
use super::stations;
use super::stations::Drawn;
use crate::BASE;
use crate::components;
use crate::components::Kind as ButtonKind;
use crate::shell::Screen;
use crate::shell::Section;
use crate::style;

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

fn rows(
	list: &[NamedCount],
	filter: Option<(&str, &Params)>,
	link: Option<&dyn Fn(&str) -> String>,
) -> Vec<draw::Bar> {
	list.iter()
		.map(|row| {
			#[expect(clippy::cast_precision_loss, reason = "a count for drawing")]
			let value = row.count as f64;
			draw::Bar {
				label: row.name.clone(),
				value,
				href: match (filter, link) {
					(Some((key, params)), _) => {
						let index = (0..4)
							.find(|index| !params.contains_key(&format!("f{index}")))
							.unwrap_or(3);
						Some(params::href(
							&path(),
							&params::with(
								params,
								&format!("f{index}"),
								insights::session_filter(key, &row.name),
							),
						))
					}
					(None, Some(link)) => Some(link(&row.name)),
					(None, None) => None,
				},
			}
		})
		.collect()
}

fn table_card(
	title: &str,
	aside: &str,
	head: (&str, &str),
	list: &[draw::Bar],
	source: &str,
	note: Option<Element>,
) -> Element {
	let mut card = components::card(title)
		.aside(aside)
		.workings("web", source)
		.child(draw::bar_list(head, list, "Nothing yet in this range."));

	if let Some(note) = note {
		card = card.child(note);
	}

	card.build()
}

/// The overview page.
#[expect(clippy::too_many_lines, reason = "one page, top to bottom")]
pub(crate) async fn page(cx: &Context<'_>) -> Screen {
	if !super::has_events(cx.facts) {
		return super::listening(
			"Web analytics",
			Section::Overview,
			"Visitors, pageviews, sessions, and bounce rate for your site, with the top pages, \
			 referrers, and devices beside them.",
		);
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
		return super::unreachable("Web analytics", Section::Overview, &failure);
	};
	// What the deltas compare with: the same length of time just before.
	let versus = match web.range.last_days {
		Some(days) if web.range.from.is_none() => format!("previous {days} days"),
		_ => "period before".to_owned(),
	};
	// With no sessions at all in the previous period, every change would
	// read as a rise from nothing; say there's nothing to compare instead.
	let compared = result
		.sessions
		.previous
		.is_some_and(|previous| previous > 0.0);
	let delta = |metric: &leptoskit_analytics::query::Metric| {
		compared.then(|| format::delta(metric.delta)).flatten()
	};
	let points = |metric: &leptoskit_analytics::query::Metric| {
		match (metric.previous, metric.delta) {
			(Some(previous), Some(_)) if compared => {
				let change = (metric.value - previous) * 100.0;
				Some(if change.abs() < 0.05 {
					"= no change".to_owned()
				} else if change > 0.0 {
					format!("▲ {change:.1} pts")
				} else {
					format!("▼ {:.1} pts", -change)
				})
			}
			_ => None,
		}
	};

	let tiles = View()
		.class("grid grid-cols-4 gap-4 max-lg:grid-cols-2 max-sm:grid-cols-1")
		.test_id("grid-tiles")
		.child(super::stat(super::Stat {
			label: "Visitors",
			value: format::count(result.visitors.value),
			delta: delta(&result.visitors),
			up_is_good: true,
			spark: values(&visitors),
			drawer: "web",
			source: "$pageview → WebOverview",
			versus: versus.clone(),
		}))
		.child(super::stat(super::Stat {
			label: "Pageviews",
			value: format::count(result.pageviews.value),
			delta: delta(&result.pageviews),
			up_is_good: true,
			spark: values(&pageviews),
			drawer: "web",
			source: "$pageview → WebOverview",
			versus: versus.clone(),
		}))
		.child(super::stat(super::Stat {
			label: "Sessions",
			value: format::count(result.sessions.value),
			delta: delta(&result.sessions),
			up_is_good: true,
			spark: values(&sessions),
			drawer: "web",
			source: "sessions → WebOverview",
			versus: versus.clone(),
		}))
		.child(super::stat(super::Stat {
			label: "Bounce rate",
			value: format::share(result.bounce_rate.value),
			delta: points(&result.bounce_rate),
			up_is_good: false,
			spark: None,
			drawer: "web",
			source: "sessions.is_bounce",
			versus,
		}));

	let (visitors_chart, visitors_drawn) = match &visitors {
		Ok(Answer {
			result: QueryResult::Trends(trend),
			..
		}) => {
			let view = insights::trend_view(&visitors_query, trend, &here, false, Some("Visitors"));
			(view.chart, view.drawn)
		}
		Ok(_) | Err(_) => {
			(
				Text("The daily visitors didn't come back.").class(style::MUTED),
				Drawn {
					sentence: "Nothing was drawn.".into(),
					rows: Vec::new(),
				},
			)
		}
	};
	let over_time = components::card("Visitors over time")
		.aside("daily")
		.workings("visitors", "$pageview → Trends")
		.test_id("visitors-card")
		.child(visitors_chart)
		.build();

	let channels = table_card(
		"Channels",
		"",
		("Channel", "Sessions"),
		&rows(&result.channels, Some(("channel", &params)), None),
		"sessions.channel",
		super::note(
			cx.preferences,
			"channels",
			"A channel is worked out once per session, from the referrer and utm_* tags.",
		),
	);
	let page_link = |page: &str| {
		let mut next: Params = cx
			.params
			.iter()
			.filter(|(key, _)| matches!(key.as_str(), "days" | "from" | "to"))
			.map(|(key, value)| (key.clone(), value.clone()))
			.collect();
		next.insert("s0".into(), "$pageview".into());
		next.insert(
			"f0".into(),
			params::filter_value(
				leptoskit_analytics::query::FilterSource::Event,
				"$pathname",
				page,
			),
		);
		params::href(&format!("{BASE}/analytics/trends"), &next)
	};
	let tables = View()
		.class("grid grid-cols-3 gap-4 max-lg:grid-cols-2 max-md:grid-cols-1")
		.test_id("grid-tiles")
		.child(table_card(
			"Top pages",
			"by pageviews",
			("Page", "Views"),
			&rows(&result.top_pages, None, Some(&page_link)),
			"$pageview.$pathname",
			None,
		))
		.child(table_card(
			"Entry pages",
			"",
			("Page", "Sessions"),
			&rows(&result.entry_pages, Some(("entry_pathname", &params)), None),
			"sessions.entry_pathname",
			None,
		))
		.child(table_card(
			"Referrers",
			"",
			("Domain", "Sessions"),
			&rows(&result.referrers, Some(("referrer_domain", &params)), None),
			"sessions.referrer_domain",
			None,
		))
		.child(table_card(
			"UTM sources",
			"",
			("Source", "Sessions"),
			&rows(&result.utm_sources, Some(("utm_source", &params)), None),
			"sessions.utm_source",
			None,
		))
		.child(table_card(
			"UTM campaigns",
			"",
			("Campaign", "Sessions"),
			&rows(&result.utm_campaigns, Some(("utm_campaign", &params)), None),
			"sessions.utm_campaign",
			None,
		))
		.child(table_card(
			"Devices",
			"",
			("Device", "Sessions"),
			&rows(&result.devices, Some(("device_type", &params)), None),
			"sessions.device_type",
			None,
		))
		.child(table_card(
			"Browsers",
			"",
			("Browser", "Sessions"),
			&rows(&result.browsers, Some(("browser", &params)), None),
			"sessions.browser",
			None,
		))
		.child(table_card(
			"Operating systems",
			"",
			("OS", "Sessions"),
			&rows(&result.operating_systems, Some(("os", &params)), None),
			"sessions.os",
			None,
		));

	// The filters in force, each with a way to drop it.
	let mut chips = View()
		.class("flex-row flex-wrap items-center gap-2")
		.test_id("filters");

	for days in [7_u32, 30, 90] {
		let on = web.range.from.is_none() && web.range.last_days == Some(days);
		chips = chips.child(super::chip(
			"",
			&format!("Last {days} days"),
			params::href(&path(), &params::with(&params, "days", days.to_string())),
			on,
		));
	}

	chips = chips.child(super::chip(
		"Compare",
		if web.compare {
			"previous period"
		} else {
			"off"
		},
		params::href(
			&path(),
			&params::with(&params, "compare", if web.compare { "0" } else { "1" }),
		),
		web.compare,
	));

	for (index, filter) in web.filters.iter().enumerate() {
		let rest: Vec<String> = web
			.filters
			.iter()
			.enumerate()
			.filter(|(other, _)| *other != index)
			.map(|(_, filter)| {
				params::filter_value(
					filter.source,
					&filter.key,
					&filter.value.as_ref().map(format::value).unwrap_or_default(),
				)
			})
			.collect();
		chips = chips.child(super::chip(
			&filter.key,
			&format!(
				"{} ×",
				filter.value.as_ref().map(format::value).unwrap_or_default()
			),
			params::href(&path(), &params::with_list(&params, "f", &rest)),
			true,
		));
	}

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

	let main = View()
		.class("grid grid-cols-3 gap-4 max-lg:grid-cols-1")
		.test_id("grid-main")
		.child(
			View()
				.class("col-span-2 flex-col max-lg:col-span-1")
				.child(over_time),
		)
		.child(channels);
	let content = View()
		.class("flex-col gap-5")
		.test_id("live-refresh")
		.child(super::header(
			"Web analytics",
			&format!(
				"every site gets this without setup · sessions average {} · updates live",
				format::duration(result.avg_session_duration_seconds.value)
			),
			vec![
				Button("Add to dashboard")
					.class(ButtonKind::Secondary.class_name())
					.aria_description("Web overview")
					.test_id("add-to-dashboard"),
			],
		))
		.child(chips)
		.child(tiles)
		.child(main)
		.child(tables);

	Screen {
		title: "Web analytics".to_owned(),
		section: Section::Overview,
		content,
		drawers,
		open: None,
	}
}
