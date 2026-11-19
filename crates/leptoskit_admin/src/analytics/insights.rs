//! The insight builders: trends, funnels, and retention, each a page whose
//! address is its query, and the people behind any point of one.
//!
//! Every control is a link to the same page with one parameter changed.
//! The pages crate draws a builder and, in the browser, fetches the next
//! one's model in place, so the Workings drawer stays open and marks the
//! lines of code that changed.

use leptoskit::prelude::*;
pub(crate) use leptoskit_admin_pages::analytics::insights::*;
use leptoskit_admin_pages::model::Page;
use leptoskit_admin_pages::model::insight::Insight;
use leptoskit_admin_pages::model::insight::Saved;
use leptoskit_analytics::query::ActorPoint;
use leptoskit_analytics::query::ActorsQuery;
use leptoskit_analytics::query::Answer;
use leptoskit_analytics::query::DateRange;
use leptoskit_analytics::query::EventRow;
use leptoskit_analytics::query::EventsQuery;
use leptoskit_analytics::query::Interval;
use leptoskit_analytics::query::Query;
use leptoskit_analytics::query::QueryResult;
use leptoskit_analytics::service::query::InsightView;

use super::Connection;
use super::Context;
use super::Shown;
use super::format;
use super::params;
use super::params::Defaults;
use super::params::Params;
use super::stations;
use super::stations::Drawn;
use crate::BASE;
use crate::components;
use crate::components::Kind as ButtonKind;
use crate::components::Tone;
use crate::shell::Screen;
use crate::shell::Section;
use crate::style;

/// The latest event of `name`, for the Send station to weigh.
pub(crate) async fn sample(connection: &Connection, name: Option<String>) -> Option<EventRow> {
	let name = name?;
	let answer = connection
		.run(Query::Events(EventsQuery {
			range: DateRange::last_days(90),
			event: Some(name),
			filters: Vec::new(),
			columns: Vec::new(),
			limit: 1,
			cursor: None,
		}))
		.await
		.ok()?;

	match answer.result {
		QueryResult::Events(page) => page.events.into_iter().next(),
		_ => None,
	}
}

/// A builder page: the query its address asks for, what the service
/// answered, and its Workings. The pages crate draws it.
pub(crate) async fn page(cx: &Context<'_>, kind: Kind, saved: Option<InsightView>) -> Shown {
	if !super::has_events(cx.facts) {
		return super::listening(kind.title(), kind.section(), kind.shows()).into();
	}

	let definitions = super::definitions(cx.facts);
	let notes = super::notes(cx.preferences);
	let defaults = Defaults::from(&definitions);
	let (path, query) = match &saved {
		Some(saved) if !params::has_query(cx.params) => {
			(
				format!("{BASE}/analytics/insights/{}", saved.id),
				saved.query.clone(),
			)
		}
		Some(saved) => {
			(
				format!("{BASE}/analytics/insights/{}", saved.id),
				kind.query(cx.params, &defaults),
			)
		}
		None => {
			(
				format!("{BASE}/analytics/{}", kind.slug()),
				kind.query(cx.params, &defaults),
			)
		}
	};
	let here = params::href(&path, &params::of(&query));
	let (answer, sample) = tokio::join!(
		cx.connection.run(query.clone()),
		sample(cx.connection, stations::focus_event(&query)),
	);
	let name = saved
		.as_ref()
		.map_or_else(|| suggested_name(&query), |saved| saved.name.clone());
	let subtitle = match &saved {
		Some(saved) => {
			format!(
				"{} · saved by {} · updates live",
				kind.title(),
				saved.created_by
			)
		}
		None => format!("{} · new · updates live", kind.title()),
	};
	// The drawer explains the chart, so it needs how the rows were drawn.
	let (_, drawn, _) = answered(
		kind,
		&query,
		answer
			.as_ref()
			.map(|answer| &answer.result)
			.map_err(|failure| failure.0.as_str()),
		&here,
	);
	let focus = stations::focus_event(&query).unwrap_or_default();
	let source = format!(
		"{} → {}",
		if is_typed(&definitions, &focus) {
			format::struct_name(&focus)
		} else {
			focus.clone()
		},
		kind.title()
	);
	let drawer = stations::drawer(stations::Source {
		id: kind.slug(),
		title: format!("{name} · {} insight · live", kind.title()),
		query: &query,
		answer: answer.as_ref().ok(),
		definitions: &cx.facts.definitions,
		sample: sample.as_ref(),
		url: cx.connection.url(),
		drawn,
	});

	Page::Insight(Box::new(Insight {
		kind,
		path,
		address: cx.params.clone(),
		query,
		saved: saved.map(|saved| {
			Saved {
				id: saved.id.to_string(),
				created_by: saved.created_by,
			}
		}),
		name,
		subtitle,
		answer: answer
			.map(|answer| answer.result)
			.map_err(|failure| failure.0),
		definitions,
		notes,
		source,
		drawer,
	}))
	.into()
}

/// The people behind a point.
#[expect(clippy::too_many_lines, reason = "one page, top to bottom")]
pub(crate) async fn actors(connection: &Connection, params: &Params) -> Option<Screen> {
	let mut actors: ActorsQuery = serde_json::from_str(params.get("q")?).ok()?;
	actors.limit = actors.limit.clamp(1, ActorsQuery::MAX_LIMIT);
	let back = params
		.get("back")
		.filter(|back| back.starts_with(BASE))
		.cloned()
		.unwrap_or_else(|| format!("{BASE}/analytics/trends"));
	let section = Kind::of(&actors.query).map_or(Section::People, Kind::section);
	let what = match &actors.point {
		ActorPoint::Bucket {
			bucket,
			breakdown_value,
			..
		} => {
			format!(
				"People behind {}{}",
				format::bucket_label(bucket, false),
				group_name(breakdown_value.as_ref())
					.map_or_else(String::new, |group| format!(" · {group}"))
			)
		}
		ActorPoint::FunnelStep { step } => format!("People who reached step {step}"),
		ActorPoint::FunnelDropOff { after } => {
			format!("People who reached step {after} and stopped")
		}
		ActorPoint::RetentionCell {
			start_period,
			return_period,
		} => {
			let unit = match &actors.query {
				Query::Retention(retention) => {
					match retention.period {
						Interval::Hour => "Hour",
						Interval::Day => "Day",
						Interval::Week => "Week",
						Interval::Month => "Month",
					}
				}
				_ => "Period",
			};
			let back = match return_period {
				0 => format!("the same {}", unit.to_lowercase()),
				1 => format!("1 {} later", unit.to_lowercase()),
				later => format!("{later} {}s later", unit.to_lowercase()),
			};
			format!("{unit} {}'s cohort, back {back}", start_period + 1)
		}
	};
	let answer: Result<Answer, _> = connection
		.run(Query::Actors(Box::new(actors.clone())))
		.await;
	let mut list = View().class("flex-col").test_id("actors");
	let mut count = 0;
	let mut next = None;

	match &answer {
		Ok(Answer {
			result: QueryResult::Actors(page),
			..
		}) => {
			count = page.actors.len();
			next = page.next;

			for actor in &page.actors {
				let name = ["email", "name"]
					.iter()
					.find_map(|key| actor.properties.get(*key)?.as_str().map(str::to_owned))
					.or_else(|| actor.distinct_ids.first().cloned())
					.unwrap_or_else(|| actor.id.clone());
				list = list.child(
					Pressable(name.clone())
						.href(super::person_href(&actor.id))
						.class("cursor-pointer rounded-lg hover:bg-[var(--admin-sunken)]")
						.test_id("actor")
						.child(
							View()
								.class(
									"flex-row items-center gap-3 border-b \
									 border-[var(--admin-line)] px-3 py-2.5",
								)
								.child(Text(format::initials(&name)).class(
									"h-8 w-8 shrink-0 rounded-full bg-[var(--admin-sunken)] \
									 text-center text-xs font-semibold leading-8 \
									 text-[var(--admin-ink)]",
								))
								.child(
									View()
										.class("min-w-0 flex-1 flex-col")
										.child(Text(name).class(
											"truncate text-sm font-semibold \
											 text-[var(--admin-ink)]",
										))
										.child(
											Text(format!(
												"{} · {} id{}",
												if actor.is_identified {
													"identified"
												} else {
													"anonymous"
												},
												actor.distinct_ids.len(),
												if actor.distinct_ids.len() == 1 {
													""
												} else {
													"s"
												}
											))
											.class(style::LABEL),
										),
								)
								.child(Text("→").class("text-sm text-[var(--admin-ink-3)]")),
						),
				);
			}

			if page.actors.is_empty() {
				list = list.child(Text("Nobody here.").class(style::MUTED));
			}
		}
		Ok(_) => {
			list = list
				.child(Text("The service answered a different kind of query.").class(style::MUTED));
		}
		Err(failure) => {
			list = list.child(components::status_word(Tone::Bad, failure.0.clone()));
		}
	}

	let mut footer = View().class("flex-row gap-3");

	if let Some(offset) = next {
		let mut more = actors.clone();
		more.offset = offset;
		let mut next_params = params.clone();
		next_params.insert("q".into(), serde_json::to_string(&more).unwrap_or_default());
		footer = footer.child(components::link_button(
			ButtonKind::Secondary,
			"Next page",
			params::href(&format!("{BASE}/analytics/actors"), &next_params),
		));
	}

	let drawer = stations::drawer(stations::Source {
		id: "actors",
		title: format!("{what} · actors query"),
		query: &Query::Actors(Box::new(actors.clone())),
		answer: answer.as_ref().ok(),
		definitions: &[],
		sample: None,
		url: connection.url(),
		drawn: Drawn {
			sentence: format!(
				"{count} people came back on this page, newest identity first. Each is one \
				 person: every id they've used is merged into them."
			),
			rows: Vec::new(),
		},
	});
	let content = View()
		.class("flex-col gap-5")
		.child(components::link("← Back to the chart", back))
		.child(super::header(
			&what,
			"Actors: the people behind one point of a query",
			Vec::new(),
		))
		.child(
			components::card(format!(
				"{count} {}",
				if count == 1 { "person" } else { "people" }
			))
			.aside(if next.is_some() {
				"more on the next page"
			} else {
				""
			})
			.workings("actors", "Query::Actors")
			.child(list)
			.child(footer)
			.build(),
		);

	Some(Screen {
		title: what,
		section,
		content,
		drawers: vec![drawer],
		open: None,
	})
}
