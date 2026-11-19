//! The library: saved insights and dashboards, one list with two tabs, and
//! a dashboard: each card's insight run, for the pages crate to draw.

use leptoskit_admin_pages::model::Page;
use leptoskit_admin_pages::model::analytics::Card;
use leptoskit_admin_pages::model::analytics::Dashboard;
use leptoskit_admin_pages::model::library::Item;
use leptoskit_admin_pages::model::library::Library;
use leptoskit_analytics::query::Answer;
use leptoskit_analytics::query::Query;
use leptoskit_analytics::service::query::DashboardView;
use leptoskit_analytics::service::query::InsightView;
use uuid::Uuid;

use super::Context;
use super::Shown;
use super::format;
use super::insights::Kind;
use crate::BASE;
use crate::shell::Section;

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

/// The library, on its insights or dashboards tab.
pub(crate) async fn page(cx: &Context<'_>) -> Shown {
	let dashboards_tab = cx.params.get("tab").is_some_and(|tab| tab == "dashboards");
	let (insights, dashboards) = tokio::join!(cx.connection.insights(), cx.connection.dashboards());
	let (insights, dashboards) = match (insights, dashboards) {
		(Ok(insights), Ok(dashboards)) => (insights, dashboards),
		(Err(failure), _) | (_, Err(failure)) => {
			return super::unreachable("Library", Section::Library, &failure).into();
		}
	};
	let items = if dashboards_tab {
		dashboards
			.iter()
			.map(|dashboard| {
				Item {
					title: dashboard.name.clone(),
					detail: format!(
						"{} card{} · updated {}",
						dashboard.tiles.len(),
						if dashboard.tiles.len() == 1 { "" } else { "s" },
						when(&dashboard.updated_at)
					),
					href: dashboard_href(dashboard.id),
				}
			})
			.collect()
	} else {
		insights
			.iter()
			.map(|insight| {
				Item {
					title: insight.name.clone(),
					detail: format!(
						"{} · by {} · updated {}",
						kind_label(&insight.query),
						insight.created_by,
						when(&insight.updated_at)
					),
					href: insight_href(insight.id),
				}
			})
			.collect()
	};

	Page::Library(Library {
		dashboards_tab,
		insights: insights.len(),
		dashboards: dashboards.len(),
		items,
	})
	.into()
}

/// A dashboard: each card's insight, run.
pub(crate) async fn dashboard(cx: &Context<'_>, id: Uuid) -> Option<Shown> {
	let dashboard: DashboardView = match cx.connection.dashboard(id).await {
		Ok(Some(dashboard)) => dashboard,
		Ok(None) => return None,
		Err(failure) => {
			return Some(super::unreachable("Dashboard", Section::Library, &failure).into());
		}
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
	let cards = pairs
		.into_iter()
		.zip(answers)
		.map(|((insight, width), answer)| {
			Card {
				wide: width >= 12
					|| matches!(insight.query, Query::WebOverview(_))
					|| Kind::of(&insight.query).is_none(),
				name: insight.name,
				kind: kind_label(&insight.query).to_owned(),
				href: insight_href(insight.id),
				answer: answer
					.map(|answer: Answer| answer.result)
					.map_err(|failure| failure.0),
				query: insight.query,
			}
		})
		.collect();

	Some(
		Page::Dashboard(Box::new(Dashboard {
			name: dashboard.name,
			cards,
		}))
		.into(),
	)
}
