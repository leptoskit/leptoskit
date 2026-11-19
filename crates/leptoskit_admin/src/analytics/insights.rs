//! The insight builders: trends, funnels, and retention, each a page whose
//! address is its query, and the people behind any point of one.
//!
//! Every control is a link to the same page with one parameter changed.
//! The script fetches it in place so the Workings drawer stays open and
//! washes the lines of code that changed.

use leptoskit::prelude::*;
pub(crate) use leptoskit_admin_pages::analytics::insights::*;
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

/// A builder page: the controls, the chart, and its Workings.
#[expect(clippy::too_many_lines, reason = "one page, top to bottom")]
pub(crate) async fn page(cx: &Context<'_>, kind: Kind, saved: Option<InsightView>) -> Screen {
	if !super::has_events(cx.facts) {
		return super::listening(kind.title(), kind.section(), kind.shows());
	}

	let definitions = super::definitions(cx.facts);
	let notes = super::notes(cx.preferences);
	let builder = Builder {
		definitions: &definitions,
		notes: &notes,
	};
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
	let params = params::of(&query);
	let here = params::href(&path, &params);
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

	let builder = match kind {
		Kind::Trends => trends_builder(&path, &params, &query, &builder),
		Kind::Funnels => funnels_builder(&path, &params, &query, &builder),
		Kind::Retention => retention_builder(&path, &params, &query, &builder),
	};
	let drawer_id = kind.slug();
	let (body, drawn, aside) = match &answer {
		Ok(answer) => {
			match (&answer.result, kind) {
				(QueryResult::Trends(result), Kind::Trends) => {
					let view = trend_view(&query, result, &here, false, None);
					let interval = match &query {
						Query::Trends(trends) => {
							match trends.interval {
								Interval::Hour => "hourly",
								Interval::Week => "weekly",
								Interval::Month => "monthly",
								Interval::Day => "daily",
							}
						}
						_ => "",
					};
					(
						View()
							.class("flex-col gap-4")
							.child(view.chart)
							.child(view.table),
						view.drawn,
						interval.to_owned(),
					)
				}
				(QueryResult::Funnels(result), Kind::Funnels) => {
					let (chart, drawn) = funnel_view(&query, result, &here, "h-64");
					let first = result.steps.first();
					let last = result.steps.last();
					let within = match &query {
						Query::Funnels(funnel) => {
							funnel.window_seconds.map_or_else(String::new, |seconds| {
								match seconds {
									86_400 => " within a day".to_owned(),
									seconds if seconds % 86_400 == 0 => {
										format!(" within {} days", seconds / 86_400)
									}
									#[expect(
										clippy::cast_precision_loss,
										reason = "a window in seconds, for display"
									)]
									seconds => format!(" within {}", format::duration(seconds as f64)),
								}
							})
						}
						_ => String::new(),
					};
					let caption = format!(
						"of people who did {} went on to {}{within}",
						first.map_or("the first step", |step| step.name.as_str()),
						last.map_or("the last step", |step| step.name.as_str()),
					);
					let summary = format!(
						"{} of {} people{}",
						format::thousands(last.map_or(0, |step| step.count)),
						format::thousands(first.map_or(0, |step| step.count)),
						result
							.median_seconds_to_convert
							.map_or_else(String::new, |seconds| {
								format!(" · median {} to convert", format::duration(seconds))
							})
					);
					(
						View()
							.class("flex-col gap-4")
							.child(
								View()
									.class("flex-row flex-wrap items-end gap-x-6 gap-y-1")
									.test_id("funnel-summary")
									.child(
										View()
											.class("min-w-[16rem] flex-1 flex-col")
											.child(
												Text(format::share(result.overall_conversion))
													.class(style::HERO_NUMBER),
											)
											.child(Text(caption).class(style::SECONDARY)),
									)
									.child(
										Text(summary).class(style::SECONDARY).class("tabular-nums"),
									),
							)
							.child(
								View()
									.class("overflow-scroll")
									.child(View().class("min-w-[520px]").child(chart)),
							),
						drawn,
						"people".to_owned(),
					)
				}
				(QueryResult::Retention(result), Kind::Retention) => {
					let (grid, drawn) = retention_view(&query, result, &here);
					(grid, drawn, "% of each cohort".to_owned())
				}
				_ => {
					(
						components::status_word(
							Tone::Bad,
							"The service answered a different kind of query.",
						),
						Drawn {
							sentence: String::new(),
							rows: Vec::new(),
						},
						String::new(),
					)
				}
			}
		}
		Err(failure) => {
			(
				View()
					.class("flex-col gap-2")
					.test_id("query-error")
					.child(components::status_word(Tone::Bad, "This query didn't run"))
					.child(Text(failure.0.clone()).class(style::BODY)),
				Drawn {
					sentence: "Nothing was drawn: the query didn't run.".to_owned(),
					rows: Vec::new(),
				},
				String::new(),
			)
		}
	};
	let chart_title = match &query {
		Query::Trends(trends) => {
			trends
				.series
				.iter()
				.map(|series| series.event.as_str())
				.collect::<Vec<_>>()
				.join(" vs ")
		}
		Query::Funnels(funnels) => format!("{} steps", funnels.steps.len()),
		Query::Retention(retention) => {
			format!("{} → {}", retention.start_event, retention.return_event)
		}
		_ => String::new(),
	};
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
	let chart = components::card(chart_title)
		.aside(aside)
		.workings(drawer_id, source)
		.test_id("chart-card")
		.child(body)
		.build();
	let drawer = stations::drawer(stations::Source {
		id: drawer_id,
		title: format!("{name} · {} insight · live", kind.title()),
		query: &query,
		answer: answer.as_ref().ok(),
		definitions: &cx.facts.definitions,
		sample: sample.as_ref(),
		url: cx.connection.url(),
		drawn,
	});

	let content = View()
		.class("flex-col gap-5")
		.test_id("live-refresh")
		.child(super::header(
			&name,
			&subtitle,
			actions(
				kind,
				saved.as_ref().map(|saved| saved.id.to_string()).as_deref(),
				&name,
			),
		))
		.child(range_chips(&path, &params))
		.child(builder)
		.child(chart);

	Screen {
		title: name,
		section: kind.section(),
		content,
		drawers: vec![drawer],
		open: None,
	}
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
