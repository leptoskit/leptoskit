//! The library: saved insights and dashboards, one list with two tabs, and
//! a dashboard's grid of insight cards that re-read as events arrive.

use leptoskit::prelude::*;
use leptoskit_analytics::query::Answer;
use leptoskit_analytics::query::Query;
use leptoskit_analytics::query::QueryResult;
use leptoskit_analytics::service::query::DashboardView;
use leptoskit_analytics::service::query::InsightView;
use uuid::Uuid;

use super::Context;
use super::Failure;
use super::format;
use super::insights;
use super::insights::Kind;
use crate::BASE;
use crate::components;
use crate::components::Kind as ButtonKind;
use crate::components::Tone;
use crate::shell::Screen;
use crate::shell::Section;
use crate::style;

pub(crate) fn insight_href(id: Uuid) -> String {
	format!("{BASE}/analytics/insights/{id}")
}

pub(crate) fn dashboard_href(id: Uuid) -> String {
	format!("{BASE}/analytics/dashboards/{id}")
}

fn kind_label(query: &Query) -> &'static str {
	match query {
		Query::Trends(_) => "Trends",
		Query::Funnels(_) => "Funnel",
		Query::Retention(_) => "Retention",
		Query::WebOverview(_) => "Web overview",
		Query::Events(_) => "Events",
		Query::Actors(_) => "Actors",
	}
}

fn when(text: &str) -> String {
	let now = chrono::Utc::now();
	format::instant(text).map_or_else(String::new, |at| format::ago(&at, &now))
}

fn item(title: &str, detail: String, href: String, test_id: &str) -> Element {
	Pressable(title.to_owned())
		.href(href)
		.class("cursor-pointer rounded-lg hover:bg-[var(--admin-sunken)]")
		.test_id(test_id.to_owned())
		.child(
			View()
				.class(
					"flex-row items-center gap-3 border-b border-[var(--admin-line)] px-3 py-2.5",
				)
				.child(
					View()
						.class("min-w-0 flex-1 flex-col gap-0.5")
						.child(
							Text(title.to_owned())
								.class("truncate text-sm font-semibold text-[var(--admin-ink)]"),
						)
						.child(Text(detail).class(style::LABEL)),
				)
				.child(Text("→").class("text-sm text-[var(--admin-ink-3)]")),
		)
}

/// The library, on its insights or dashboards tab.
pub(crate) async fn page(cx: &Context<'_>) -> Screen {
	let dashboards_tab = cx.params.get("tab").is_some_and(|tab| tab == "dashboards");
	let (insights, dashboards) = tokio::join!(cx.connection.insights(), cx.connection.dashboards());
	let (insights, dashboards) = match (insights, dashboards) {
		(Ok(insights), Ok(dashboards)) => (insights, dashboards),
		(Err(failure), _) | (_, Err(failure)) => {
			return super::unreachable("Library", Section::Library, &failure);
		}
	};
	let tabs = components::tabs(
		&[
			(
				"Insights".to_owned(),
				format!("{BASE}/analytics/library"),
				Some(insights.len().to_string()),
			),
			(
				"Dashboards".to_owned(),
				format!("{BASE}/analytics/library?tab=dashboards"),
				Some(dashboards.len().to_string()),
			),
		],
		usize::from(dashboards_tab),
	);
	let mut list = View().class("flex-col").test_id("library");

	if dashboards_tab {
		for dashboard in &dashboards {
			list = list.child(item(
				&dashboard.name,
				format!(
					"{} card{} · updated {}",
					dashboard.tiles.len(),
					if dashboard.tiles.len() == 1 { "" } else { "s" },
					when(&dashboard.updated_at)
				),
				dashboard_href(dashboard.id),
				"library-dashboard",
			));
		}

		if dashboards.is_empty() {
			list = list.child(
				Text("No dashboards yet. Open any insight and press Add to dashboard.")
					.class(style::MUTED)
					.class("px-3 py-2"),
			);
		}
	} else {
		for insight in &insights {
			list = list.child(item(
				&insight.name,
				format!(
					"{} · by {} · updated {}",
					kind_label(&insight.query),
					insight.created_by,
					when(&insight.updated_at)
				),
				insight_href(insight.id),
				"library-insight",
			));
		}

		if insights.is_empty() {
			list = list.child(
				Text(
					"Nothing saved yet. Build a trend, a funnel, or a retention grid, then press \
					 Save.",
				)
				.class(style::MUTED)
				.class("px-3 py-2"),
			);
		}
	}

	let actions = vec![
		components::link_button(
			ButtonKind::Secondary,
			"New trend",
			format!("{BASE}/analytics/trends"),
		),
		components::link_button(
			ButtonKind::Secondary,
			"New funnel",
			format!("{BASE}/analytics/funnels"),
		),
	];
	let content = View()
		.class("flex-col gap-5")
		.child(super::header(
			"Library",
			"saved insights and the dashboards they sit on",
			actions,
		))
		.child(tabs)
		.child(View().class(style::CARD).child(list));

	Screen {
		title: "Library".to_owned(),
		section: Section::Library,
		content,
		drawers: Vec::new(),
		open: None,
	}
}

/// One dashboard card: its insight, run and drawn small.
fn tile(insight: &InsightView, answer: &Result<Answer, Failure>, wide: bool) -> Element {
	let here = insight_href(insight.id);
	let body = match answer {
		Ok(answer) => {
			match &answer.result {
				QueryResult::Trends(result) => {
					insights::trend_view(&insight.query, result, &here, wide, None).chart
				}
				QueryResult::Funnels(result) => {
					insights::funnel_view(&insight.query, result, &here, "h-40").0
				}
				QueryResult::Retention(result) => {
					View()
						.class("max-h-80 overflow-scroll")
						.child(insights::retention_view(&insight.query, result, &here).0)
				}
				QueryResult::WebOverview(result) => {
					let figure = |label: &str, value: String| {
						View()
							.class("flex-col gap-0.5")
							.child(Text(label.to_owned()).class(style::LABEL))
							.child(Text(value).class(
								"text-2xl font-semibold tabular-nums text-[var(--admin-ink)]",
							))
					};
					View()
						.class("grid grid-cols-4 gap-3 max-sm:grid-cols-2")
						.child(figure("Visitors", format::count(result.visitors.value)))
						.child(figure("Pageviews", format::count(result.pageviews.value)))
						.child(figure("Sessions", format::count(result.sessions.value)))
						.child(figure(
							"Bounce rate",
							format::share(result.bounce_rate.value),
						))
				}
				_ => Text("This kind of insight has no card yet.").class(style::MUTED),
			}
		}
		Err(failure) => components::status_word(Tone::Bad, failure.0.clone()),
	};
	let mut card = components::card(insight.name.clone())
		.aside(kind_label(&insight.query))
		.test_id("dashboard-tile")
		.child(body)
		.child(components::link("Open →", here))
		.build();

	if wide {
		card = View()
			.class("col-span-2 flex-col max-lg:col-span-1")
			.child(card);
	}

	card
}

/// A dashboard.
pub(crate) async fn dashboard(cx: &Context<'_>, id: Uuid) -> Option<Screen> {
	let dashboard: DashboardView = match cx.connection.dashboard(id).await {
		Ok(Some(dashboard)) => dashboard,
		Ok(None) => return None,
		Err(failure) => return Some(super::unreachable("Dashboard", Section::Library, &failure)),
	};
	let mut tiles = dashboard.tiles.clone();
	tiles.sort_by_key(|tile| (tile.y, tile.x));
	let saved =
		futures::future::join_all(tiles.iter().map(|tile| cx.connection.insight(tile.insight)))
			.await;
	let pairs: Vec<(InsightView, u16)> = saved
		.into_iter()
		.zip(&tiles)
		.filter_map(|(insight, tile)| Some((insight.ok().flatten()?, tile.w)))
		.collect();
	let answers = futures::future::join_all(
		pairs
			.iter()
			.map(|(insight, _)| cx.connection.run(insight.query.clone())),
	)
	.await;
	let mut grid = View()
		.class("grid grid-cols-2 gap-4 max-lg:grid-cols-1")
		.test_id("grid-tiles");

	for ((insight, width), answer) in pairs.iter().zip(&answers) {
		let wide = *width >= 12
			|| matches!(insight.query, Query::WebOverview(_))
			|| Kind::of(&insight.query).is_none();
		grid = grid.child(tile(insight, answer, wide));
	}

	if pairs.is_empty() {
		grid = grid.child(
			Text("No cards yet. Open any insight and press Add to dashboard.").class(style::MUTED),
		);
	}

	let content = View()
		.class("flex-col gap-5")
		.test_id("live-refresh")
		.child(components::link(
			"← Library",
			format!("{BASE}/analytics/library?tab=dashboards"),
		))
		.child(super::header(
			&dashboard.name,
			&format!(
				"dashboard · {} card{} · updates live",
				pairs.len(),
				if pairs.len() == 1 { "" } else { "s" }
			),
			Vec::new(),
		))
		.child(grid);

	Some(Screen {
		title: dashboard.name.clone(),
		section: Section::Library,
		content,
		drawers: Vec::new(),
		open: None,
	})
}
