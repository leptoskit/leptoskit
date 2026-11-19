//! The web overview: visitors, pageviews, sessions, bounce rate, and
//! session length, with the tables beside them. Its chips and table rows
//! change it in place, and it reads itself again as events arrive.

use leptoskit::prelude::*;
use leptoskit_analytics::query::FilterSource;
use leptoskit_analytics::query::Metric;
use leptoskit_analytics::query::NamedCount;
use leptoskit_analytics::query::Query;
use leptoskit_analytics::query::QueryResult;

use super::insight::Actions;
use crate::BASE;
use crate::analytics;
use crate::analytics::Stat;
use crate::analytics::draw;
use crate::analytics::format;
use crate::analytics::insights;
use crate::analytics::params;
use crate::analytics::params::Params;
use crate::components;
use crate::components::Kind as ButtonKind;
use crate::model::analytics::Web;
use crate::style;

/// The overview's address, without its query.
pub(crate) fn path() -> String {
	format!("{BASE}/analytics/overview")
}

/// The first series' daily values, when the trend came back.
fn values(answer: Result<&QueryResult, &String>) -> Option<Vec<f64>> {
	match answer {
		Ok(QueryResult::Trends(result)) => {
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

/// The filters in force, each with a way to drop it.
fn chips(params: &Params) -> Element {
	let web = params::web(params);
	let mut chips = View()
		.class("flex-row flex-wrap items-center gap-2")
		.test_id("filters");

	for days in [7_u32, 30, 90] {
		let on = web.range.from.is_none() && web.range.last_days == Some(days);
		chips = chips.child(analytics::chip(
			"",
			&format!("Last {days} days"),
			params::href(&path(), &params::with(params, "days", days.to_string())),
			on,
		));
	}

	chips = chips.child(analytics::chip(
		"Compare",
		if web.compare {
			"previous period"
		} else {
			"off"
		},
		params::href(
			&path(),
			&params::with(params, "compare", if web.compare { "0" } else { "1" }),
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
		chips = chips.child(analytics::chip(
			&filter.key,
			&format!(
				"{} ×",
				filter.value.as_ref().map(format::value).unwrap_or_default()
			),
			params::href(&path(), &params::with_list(params, "f", &rest)),
			true,
		));
	}

	chips
}

/// The overview page. With `actions`, Add to dashboard asks through the
/// confirm sheet.
#[expect(clippy::too_many_lines, reason = "one page, top to bottom")]
pub(crate) fn page(web: &Web, actions: Option<Actions>) -> Element {
	let params = params::of(&web.query);
	let here = params::href(&path(), &params);
	let result = &web.result;
	let range = match &web.query {
		Query::WebOverview(overview) => overview.range.clone(),
		_ => params::web(&params).range,
	};
	// What the deltas compare with: the same length of time just before.
	let versus = match range.last_days {
		Some(days) if range.from.is_none() => format!("previous {days} days"),
		_ => "period before".to_owned(),
	};
	// With no sessions at all in the previous period, every change would
	// read as a rise from nothing; say there's nothing to compare instead.
	let compared = result
		.sessions
		.previous
		.is_some_and(|previous| previous > 0.0);
	let delta = |metric: &Metric| compared.then(|| format::delta(metric.delta)).flatten();
	let points = |metric: &Metric| {
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
		.child(analytics::stat(Stat {
			label: "Visitors",
			value: format::count(result.visitors.value),
			delta: delta(&result.visitors),
			up_is_good: true,
			spark: values(web.visitors.as_ref()),
			drawer: "web",
			source: "$pageview → WebOverview",
			versus: versus.clone(),
		}))
		.child(analytics::stat(Stat {
			label: "Pageviews",
			value: format::count(result.pageviews.value),
			delta: delta(&result.pageviews),
			up_is_good: true,
			spark: web.pageviews.clone(),
			drawer: "web",
			source: "$pageview → WebOverview",
			versus: versus.clone(),
		}))
		.child(analytics::stat(Stat {
			label: "Sessions",
			value: format::count(result.sessions.value),
			delta: delta(&result.sessions),
			up_is_good: true,
			spark: web.sessions.clone(),
			drawer: "web",
			source: "sessions → WebOverview",
			versus: versus.clone(),
		}))
		.child(analytics::stat(Stat {
			label: "Bounce rate",
			value: format::share(result.bounce_rate.value),
			delta: points(&result.bounce_rate),
			up_is_good: false,
			spark: None,
			drawer: "web",
			source: "sessions.is_bounce",
			versus,
		}));

	let visitors_chart = match &web.visitors {
		Ok(QueryResult::Trends(trend)) => {
			insights::trend_view(&web.visitors_query, trend, &here, false, Some("Visitors")).chart
		}
		_ => Text("The daily visitors didn't come back.").class(style::MUTED),
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
		web.channels_note.then(|| {
			components::margin_note(
				"channels",
				"A channel is worked out once per session, from the referrer and utm_* tags.",
			)
		}),
	);
	let page_link = |page: &str| {
		let mut next: Params = web
			.address
			.iter()
			.filter(|(key, _)| matches!(key.as_str(), "days" | "from" | "to"))
			.map(|(key, value)| (key.clone(), value.clone()))
			.collect();
		next.insert("s0".into(), "$pageview".into());
		next.insert(
			"f0".into(),
			params::filter_value(FilterSource::Event, "$pathname", page),
		);
		params::href(&format!("{BASE}/analytics/trends"), &next)
	};
	let session_table = |title: &str, head: &str, key: &str, list: &[NamedCount]| {
		table_card(
			title,
			"",
			(head, "Sessions"),
			&rows(list, Some((key, &params)), None),
			&format!("sessions.{key}"),
			None,
		)
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
		.child(session_table(
			"Entry pages",
			"Page",
			"entry_pathname",
			&result.entry_pages,
		))
		.child(session_table(
			"Referrers",
			"Domain",
			"referrer_domain",
			&result.referrers,
		))
		.child(session_table(
			"UTM sources",
			"Source",
			"utm_source",
			&result.utm_sources,
		))
		.child(session_table(
			"UTM campaigns",
			"Campaign",
			"utm_campaign",
			&result.utm_campaigns,
		))
		.child(session_table(
			"Devices",
			"Device",
			"device_type",
			&result.devices,
		))
		.child(session_table(
			"Browsers",
			"Browser",
			"browser",
			&result.browsers,
		))
		.child(session_table(
			"Operating systems",
			"OS",
			"os",
			&result.operating_systems,
		));

	let mut add = Button("Add to dashboard")
		.class(ButtonKind::Secondary.class_name())
		.aria_description("Web overview")
		.test_id("add-to-dashboard");

	if let Some(actions) = actions {
		add = actions.add_web(add, params.clone());
	}

	let main = View()
		.class("grid grid-cols-3 gap-4 max-lg:grid-cols-1")
		.test_id("grid-main")
		.child(
			View()
				.class("col-span-2 flex-col max-lg:col-span-1")
				.child(over_time),
		)
		.child(channels);

	View()
		.class("flex-col gap-5")
		.test_id("live-refresh")
		.child(components::header(
			"Web analytics",
			&format!(
				"every site gets this without setup · sessions average {} · updates live",
				format::duration(result.avg_session_duration_seconds.value)
			),
			vec![add],
		))
		.child(chips(&params))
		.child(tiles)
		.child(main)
		.child(tables)
}
