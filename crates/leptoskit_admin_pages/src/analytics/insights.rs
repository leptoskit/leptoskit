//! The insight builders: trends, funnels, and retention, each a page whose
//! address is its query, and the charts that draw what a query answered.
//!
//! Every control is a link to the same page with one parameter changed.

use leptoskit::native::Role;
use leptoskit::prelude::*;
use leptoskit_analytics::query::ActorPoint;
use leptoskit_analytics::query::ActorsQuery;
use leptoskit_analytics::query::FilterSource;
use leptoskit_analytics::query::FunnelOrder;
use leptoskit_analytics::query::FunnelsResult;
use leptoskit_analytics::query::Interval;
use leptoskit_analytics::query::Math;
use leptoskit_analytics::query::Query;
use leptoskit_analytics::query::RetentionKind;
use leptoskit_analytics::query::RetentionResult;
use leptoskit_analytics::query::TrendsResult;

use super::Definition;
use super::Drawn;
use super::draw;
use super::format;
use super::params;
use super::params::Defaults;
use super::params::Params;
use crate::BASE;
use crate::components;
use crate::components::Kind as ButtonKind;
use crate::model::Section;
use crate::style;

/// What a builder offers and teaches: the events the service has stored,
/// and the margin notes the person hasn't retired.
pub struct Builder<'a> {
	pub definitions: &'a [Definition],
	/// The ids of the notes that show.
	pub notes: &'a [String],
}

impl Builder<'_> {
	fn note(&self, id: &str, text: &str) -> Option<Element> {
		self.notes
			.iter()
			.any(|note| note == id)
			.then(|| components::margin_note(id, text.to_owned()))
	}
}

/// Which builder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
	Trends,
	Funnels,
	Retention,
}

impl Kind {
	pub fn parse(slug: &str) -> Option<Self> {
		Some(match slug {
			"trends" => Self::Trends,
			"funnels" => Self::Funnels,
			"retention" => Self::Retention,
			_ => return None,
		})
	}

	pub fn of(query: &Query) -> Option<Self> {
		Some(match query {
			Query::Trends(_) => Self::Trends,
			Query::Funnels(_) => Self::Funnels,
			Query::Retention(_) => Self::Retention,
			_ => return None,
		})
	}

	pub fn slug(self) -> &'static str {
		match self {
			Self::Trends => "trends",
			Self::Funnels => "funnels",
			Self::Retention => "retention",
		}
	}

	pub fn title(self) -> &'static str {
		match self {
			Self::Trends => "Trends",
			Self::Funnels => "Funnels",
			Self::Retention => "Retention",
		}
	}

	pub fn section(self) -> Section {
		match self {
			Self::Trends => Section::Trends,
			Self::Funnels => Section::Funnels,
			Self::Retention => Section::Retention,
		}
	}

	pub fn shows(self) -> &'static str {
		match self {
			Self::Trends => {
				"How often an event happens over time, split by any property you send with it, \
				 such as plan."
			}
			Self::Funnels => {
				"How many people go from one event to the next, such as SignedUp to \
				 CheckoutStarted, and where the rest stop."
			}
			Self::Retention => "Of the people who did something one week, how many came back.",
		}
	}

	/// The query an address asks for.
	pub fn query(self, params: &Params, defaults: &Defaults) -> Query {
		match self {
			Self::Trends => Query::Trends(params::trends(params, defaults)),
			Self::Funnels => Query::Funnels(params::funnels(params, defaults)),
			Self::Retention => Query::Retention(params::retention(params, defaults)),
		}
	}
}

fn definition<'a>(definitions: &'a [Definition], name: &str) -> Option<&'a Definition> {
	definitions
		.iter()
		.find(|definition| definition.name == name)
}

pub fn is_typed(definitions: &[Definition], name: &str) -> bool {
	definition(definitions, name).is_some_and(Definition::typed)
}

/// An event name as the builder shows it: mono, with its `struct` badge.
pub fn event_name(definitions: &[Definition], name: &str) -> Element {
	let mut row = View()
		.class("min-w-0 flex-row flex-wrap items-center gap-1.5")
		.child(
			Text(name.to_owned())
				.class(style::CODE)
				.class("font-semibold"),
		);

	if is_typed(definitions, name) {
		row = row.child(components::struct_badge());
	}

	row
}

/// The event catalogue as menu items, each a link `make` builds.
fn event_menu(
	label: &str,
	test_id: &str,
	definitions: &[Definition],
	skip: &[String],
	make: impl Fn(&str) -> String,
) -> Element {
	let items: Vec<Element> = definitions
		.iter()
		.filter(|definition| definition.volume > 0 && !skip.contains(&definition.name))
		.map(|definition| {
			let detail = format!(
				"{}{}",
				if definition.typed() { "struct · " } else { "" },
				format::thousands(definition.volume)
			);
			super::menu_item(&definition.name, Some(&detail), make(&definition.name))
		})
		.collect();
	super::menu(label, test_id, items)
}

fn letter(index: usize) -> char {
	char::from(b'A' + u8::try_from(index % 26).unwrap_or(0))
}

/// The slot's badge: the series letter on its colour.
fn badge(index: usize) -> Element {
	Text(letter(index).to_string())
		.class(
			"h-6 w-6 shrink-0 items-center justify-center rounded-md text-center text-xs \
			 font-semibold leading-6 text-white",
		)
		.class(draw::fill(index % 6 + 1))
}

/// The properties a breakdown can name for `event`.
pub fn breakdown_options(definitions: &[Definition], event: &str) -> Vec<(String, String)> {
	let mut options = Vec::new();

	if let Some(definition) = definition(definitions, event) {
		let keys: Vec<String> = match &definition.fields {
			Some(fields) => fields.iter().map(|(name, _)| name.clone()).collect(),
			None => {
				definition
					.properties
					.iter()
					.filter(|(_, ty)| ty == "string" || ty == "boolean" || ty == "integer")
					.map(|(key, _)| key.clone())
					.collect()
			}
		};

		for key in keys {
			options.push((key.clone(), format!("{key} (from {event})")));
		}
	}

	for (key, label) in [
		("channel", "Channel"),
		("browser", "Browser"),
		("os", "Operating system"),
		("device_type", "Device"),
		("referrer_domain", "Referrer"),
		("utm_source", "UTM source"),
		("entry_pathname", "Entry page"),
	] {
		options.push((format!("session:{key}"), format!("{label} (session)")));
	}

	options
}

fn breakdown_label(params: &Params, definitions: &[Definition], event: &str) -> String {
	params.get("by").map_or_else(
		|| "None".to_owned(),
		|by| {
			breakdown_options(definitions, event)
				.into_iter()
				.find(|(value, _)| value == by)
				.map_or_else(|| by.clone(), |(_, label)| label)
		},
	)
}

fn breakdown_menu(path: &str, params: &Params, definitions: &[Definition], event: &str) -> Element {
	let mut items = vec![super::menu_item(
		"None",
		None,
		params::href(path, &params::without(params, "by")),
	)];

	for (value, label) in breakdown_options(definitions, event) {
		items.push(super::menu_item(
			&label,
			None,
			params::href(path, &params::with(params, "by", value)),
		));
	}

	View()
		.class("flex-col gap-1.5")
		.child(Text("Breakdown").class("text-sm font-semibold text-[var(--admin-ink-2)]"))
		.child(super::menu(
			&format!("{} ▾", breakdown_label(params, definitions, event)),
			"breakdown",
			items,
		))
}

pub fn range_chips(path: &str, params: &Params) -> Element {
	let range = params::range(params);
	let mut row = View().class("flex-row flex-wrap items-center gap-2");

	for days in [7_u32, 14, 30, 90] {
		let on = range.from.is_none() && range.last_days == Some(days);
		let next = params::with(
			&params::without(&params::without(params, "from"), "to"),
			"days",
			days.to_string(),
		);
		row = row.child(super::chip(
			"",
			&format!("Last {days} days"),
			params::href(path, &next),
			on,
		));
	}

	if range.from.is_some() {
		row = row.child(super::chip(
			"",
			&params::range_label(&range),
			params::href(path, params),
			true,
		));
	}

	row
}

/// The page heading's actions: add to a dashboard, then save.
pub fn actions(kind: Kind, saved: Option<&str>, name: &str) -> Vec<Element> {
	vec![
		Button("Add to dashboard")
			.class(ButtonKind::Secondary.class_name())
			.aria_description(name.to_owned())
			.test_id("add-to-dashboard"),
		components::raised(
			Button(if saved.is_some() {
				"Save changes"
			} else {
				"Save"
			})
			.class(ButtonKind::Primary.class_name())
			.aria_description(format!(
				"{}|{}|{name}",
				kind.slug(),
				saved.unwrap_or_default()
			))
			.test_id("save-insight"),
		),
	]
}

/// The name a new insight starts with.
pub fn suggested_name(query: &Query) -> String {
	match query {
		Query::Trends(trends) => {
			let names: Vec<&str> = trends
				.series
				.iter()
				.map(|series| series.event.as_str())
				.collect();
			format!(
				"{}, {}",
				names.join(" vs "),
				params::range_phrase(&trends.range)
			)
		}
		Query::Funnels(funnels) => {
			format!(
				"{} → {}",
				funnels.steps.first().map_or("", String::as_str),
				funnels.steps.last().map_or("", String::as_str)
			)
		}
		Query::Retention(retention) => {
			format!(
				"{} → {} retention",
				retention.start_event, retention.return_event
			)
		}
		_ => "Web overview".to_owned(),
	}
}

/// The people behind a point, as a link to the Actors page.
pub fn actors_href(query: &Query, point: ActorPoint, back: &str) -> String {
	let actors = ActorsQuery {
		query: query.clone(),
		point,
		limit: ActorsQuery::default_limit(),
		offset: 0,
	};
	let mut params = Params::new();
	params.insert(
		"q".into(),
		serde_json::to_string(&actors).unwrap_or_default(),
	);
	params.insert("back".into(), back.to_owned());
	params::href(&format!("{BASE}/analytics/actors"), &params)
}

/// The value a breakdown group is named by.
pub fn group_name(value: Option<&serde_json::Value>) -> Option<String> {
	value.map(|value| {
		let text = format::value(value);
		if text.is_empty() || text == "—" {
			"(none)".to_owned()
		} else {
			text
		}
	})
}

fn math_label(math: Math) -> &'static str {
	match math {
		Math::Total => "Total count",
		Math::UniquePersons => "Unique people",
		Math::UniqueSessions => "Unique sessions",
	}
}

/// What a trend answered, drawn: the chart, its table, and its drawer's
/// Draw station.
pub struct TrendView {
	pub chart: Element,
	pub table: Element,
	pub drawn: Drawn,
}

/// Draws a trend's answer.
#[expect(
	clippy::too_many_lines,
	reason = "the chart, the table, and the Draw rows from one answer"
)]
pub fn trend_view(
	query: &Query,
	result: &TrendsResult,
	here: &str,
	wide: bool,
	label: Option<&str>,
) -> TrendView {
	let Query::Trends(trends) = query else {
		return TrendView {
			chart: View(),
			table: View(),
			drawn: Drawn {
				sentence: String::new(),
				rows: Vec::new(),
			},
		};
	};
	let hourly = trends.interval == Interval::Hour;
	let labels: Vec<String> = result
		.series
		.first()
		.map(|series| {
			series
				.buckets
				.iter()
				.map(|bucket| format::bucket_label(&bucket.start, hourly))
				.collect()
		})
		.unwrap_or_default();
	let name_of = |series: &leptoskit_analytics::query::TrendSeries| {
		group_name(series.breakdown_value.as_ref())
			.or_else(|| label.map(str::to_owned))
			.unwrap_or_else(|| series.event.clone())
	};
	let lines: Vec<draw::Line> = result
		.series
		.iter()
		.enumerate()
		.map(|(index, series)| {
			let previous = result
				.previous
				.iter()
				.find(|previous| {
					previous.event == series.event
						&& previous.breakdown_value == series.breakdown_value
				})
				.map(|previous| previous.buckets.iter().map(|bucket| bucket.value).collect());
			draw::Line {
				name: name_of(series),
				slot: index % 6 + 1,
				values: series.buckets.iter().map(|bucket| bucket.value).collect(),
				previous,
			}
		})
		.collect();
	let first = result.series.first();
	let series_index = first
		.and_then(|series| {
			trends
				.series
				.iter()
				.position(|wanted| wanted.event == series.event)
		})
		.unwrap_or(0);
	let targets: Vec<Option<String>> = first.map_or_else(Vec::new, |series| {
		series
			.buckets
			.iter()
			.map(|bucket| {
				Some(actors_href(
					query,
					ActorPoint::Bucket {
						series: u32::try_from(series_index).unwrap_or(0),
						bucket: bucket.start.clone(),
						breakdown_value: series.breakdown_value.clone(),
					},
					here,
				))
			})
			.collect()
	});
	let partial = trends.range.to.is_none();
	let chart = draw::trend(
		&draw::Trend {
			lines,
			labels: labels.clone(),
			targets,
			partial,
			format: format::number,
			aspect: if wide { "aspect-[4/1]" } else { "aspect-[5/2]" },
		},
		&format!("{} over time", suggested_name(query)),
	);

	// The table under the chart: each line's total and its change.
	let mut table = View()
		.class("flex-col")
		.role(Role::Table)
		.aria_label("Each line's total")
		.test_id("trend-table")
		.child(
			View()
				.class("flex-row gap-3 rounded-lg bg-[var(--admin-sunken)] px-3 py-2")
				.child(Text("Series").class("min-w-0 flex-1").class(style::LABEL))
				.child(Text("Total").class("w-24 text-end").class(style::LABEL))
				.child(
					Text("vs previous")
						.class("w-24 text-end")
						.class(style::LABEL),
				)
				.child(
					Text("People")
						.class("w-20 text-end max-sm:hidden")
						.class(style::LABEL),
				),
		);

	for (index, series) in result.series.iter().enumerate() {
		let previous = result
			.previous
			.iter()
			.find(|previous| {
				previous.event == series.event && previous.breakdown_value == series.breakdown_value
			})
			.map(|previous| previous.total);
		let change = previous
			.and_then(|previous| (previous > 0.0).then(|| (series.total - previous) / previous));
		let last = series.buckets.last().map(|bucket| {
			actors_href(
				query,
				ActorPoint::Bucket {
					series: u32::try_from(
						trends
							.series
							.iter()
							.position(|wanted| wanted.event == series.event)
							.unwrap_or(0),
					)
					.unwrap_or(0),
					bucket: bucket.start.clone(),
					breakdown_value: series.breakdown_value.clone(),
				},
				here,
			)
		});
		table = table.child(
			View()
				.class("flex-row items-center gap-3 border-b border-[var(--admin-line)] px-3 py-2")
				.child(
					View()
						.class("min-w-0 flex-1 flex-row items-center gap-2")
						.child(
							View()
								.class("h-0.5 w-4 shrink-0 rounded-full")
								.class(draw::fill(index % 6 + 1)),
						)
						.child(
							Text(name_of(series)).class("truncate text-sm text-[var(--admin-ink)]"),
						),
				)
				.child(Text(format::number(series.total)).class(
					"w-24 text-end text-sm font-semibold tabular-nums text-[var(--admin-ink)]",
				))
				.child(
					Text(format::delta(change).unwrap_or_else(|| "—".to_owned()))
						.class("w-24 text-end text-sm tabular-nums text-[var(--admin-ink-2)]"),
				)
				.child(match last {
					Some(href) => {
						Pressable(format!("People in the last bucket of {}", name_of(series)))
							.href(href)
							.class("w-20 cursor-pointer text-end max-sm:hidden")
							.child(
								Text("Latest →")
									.class("text-sm font-semibold text-[var(--admin-mint-text)]"),
							)
					}
					None => View().class("w-20 max-sm:hidden"),
				}),
		);
	}

	// The Draw station's rows: the last few buckets of each line.
	let unit = match trends.interval {
		Interval::Hour => "hour",
		Interval::Day => "day",
		Interval::Week => "week",
		Interval::Month => "month",
	};
	let mut rows = vec![{
		let mut header = vec![unit.to_owned()];
		header.extend(result.series.iter().take(4).map(|series| {
			let name = name_of(series);
			name.chars().take(18).collect::<String>()
		}));
		header
	}];
	let buckets = first.map_or(0, |series| series.buckets.len());

	for bucket in buckets.saturating_sub(4)..buckets {
		let mut row = vec![
			first
				.and_then(|series| series.buckets.get(bucket))
				.map(|bucket| {
					bucket
						.start
						.get(..if hourly { 16 } else { 10 })
						.unwrap_or(&bucket.start)
						.to_owned()
				})
				.unwrap_or_default(),
		];
		row.extend(result.series.iter().take(4).map(|series| {
			series
				.buckets
				.get(bucket)
				.map(|bucket| format::number(bucket.value))
				.unwrap_or_default()
		}));
		rows.push(row);
	}

	let sentence = format!(
		"{} row{} per line came back, one per {}; {} line{} drawn{}.{} The page re-reads it when \
		 new events arrive.",
		buckets,
		if buckets == 1 { "" } else { "s" },
		unit,
		result.series.len(),
		if result.series.len() == 1 { "" } else { "s" },
		// The chart leaves out a previous period with nothing in it.
		if result
			.previous
			.iter()
			.any(|series| series.buckets.iter().any(|bucket| bucket.value != 0.0))
		{
			", the previous period dashed beside each"
		} else if trends.compare {
			"; the previous period had nothing in it, so there's no dashed line"
		} else {
			""
		},
		if partial {
			" The last one is still filling, so it's drawn dashed."
		} else {
			""
		}
	);
	TrendView {
		chart,
		table,
		drawn: Drawn {
			sentence,
			rows: super::table(&rows),
		},
	}
}

/// What a funnel answered, drawn.
pub fn funnel_view(
	query: &Query,
	result: &FunnelsResult,
	here: &str,
	height: &'static str,
) -> (Element, Drawn) {
	let steps: Vec<draw::Step> = result
		.steps
		.iter()
		.enumerate()
		.map(|(index, step)| {
			let before = index
				.checked_sub(1)
				.and_then(|before| result.steps.get(before));
			draw::Step {
				name: step.name.clone(),
				count: step.count,
				#[expect(clippy::cast_precision_loss, reason = "a share for display")]
				conversion: before.map(|before| {
					if before.count == 0 {
						0.0
					} else {
						step.count as f64 / before.count as f64
					}
				}),
				reached: (step.count > 0).then(|| {
					actors_href(
						query,
						ActorPoint::FunnelStep {
							step: u32::try_from(index + 1).unwrap_or(1),
						},
						here,
					)
				}),
				dropped: before.filter(|before| before.count > step.count).map(|_| {
					actors_href(
						query,
						ActorPoint::FunnelDropOff {
							after: u32::try_from(index).unwrap_or(1),
						},
						here,
					)
				}),
			}
		})
		.collect();
	let mut rows = vec![vec![
		"step".to_owned(),
		"people".to_owned(),
		"of first".to_owned(),
		"dropped".to_owned(),
	]];

	for (index, step) in result.steps.iter().enumerate() {
		rows.push(vec![
			format!("{}. {}", index + 1, step.name),
			format::thousands(step.count),
			format::share(step.conversion_rate),
			format::thousands(step.drop_off),
		]);
	}

	(
		draw::funnel(&steps, height),
		Drawn {
			sentence: format!(
				"One row per step: how many people reached it within the window. Each column's \
				 height is that count; the hatched part is who reached the step before and \
				 stopped. Overall, {} converted{}.",
				format::share(result.overall_conversion),
				result
					.median_seconds_to_convert
					.map_or_else(String::new, |seconds| {
						format!(", taking {} at the median", format::duration(seconds))
					})
			),
			rows: super::table(&rows),
		},
	)
}

/// What a retention query answered, drawn.
pub fn retention_view(query: &Query, result: &RetentionResult, here: &str) -> (Element, Drawn) {
	let label = match result.period {
		Interval::Day => "Day ",
		_ => "Week ",
	};
	let cohorts: Vec<draw::Cohort> = result
		.cohorts
		.iter()
		.enumerate()
		.map(|(cohort_index, cohort)| {
			draw::Cohort {
				label: format::bucket_label(&cohort.start, false),
				size: cohort.size,
				returns: cohort.returns.clone(),
				targets: cohort
					.returns
					.iter()
					.enumerate()
					.map(|(period, returned)| {
						(*returned > 0).then(|| {
							actors_href(
								query,
								ActorPoint::RetentionCell {
									start_period: u32::try_from(cohort_index).unwrap_or(0),
									return_period: u32::try_from(period).unwrap_or(0),
								},
								here,
							)
						})
					})
					.collect(),
			}
		})
		.collect();
	let periods = result
		.cohorts
		.iter()
		.map(|cohort| cohort.returns.len())
		.max()
		.unwrap_or(0);
	// The mean of each period over the cohorts old enough to have it,
	// weighted by size.
	#[expect(clippy::cast_precision_loss, reason = "shares for display")]
	let mean: Vec<f64> = (0..periods)
		.map(|period| {
			let (returned, size) = result
				.cohorts
				.iter()
				.filter(|cohort| cohort.returns.len() > period)
				.fold((0_u64, 0_u64), |(returned, size), cohort| {
					(returned + cohort.returns[period], size + cohort.size)
				});
			if size == 0 {
				0.0
			} else {
				returned as f64 / size as f64
			}
		})
		.collect();
	let mut rows = vec![{
		let mut header = vec!["cohort".to_owned(), "size".to_owned()];
		header.extend(
			(0..periods.min(5)).map(|period| format!("{}{period}", label.trim()).to_lowercase()),
		);
		header
	}];

	for cohort in result.cohorts.iter().take(4) {
		let mut row = vec![
			cohort.start.get(..10).unwrap_or(&cohort.start).to_owned(),
			format::thousands(cohort.size),
		];
		row.extend(
			cohort
				.returns
				.iter()
				.take(5)
				.map(|returned| format::thousands(*returned)),
		);
		rows.push(row);
	}

	(
		draw::retention(&cohorts, label, &mean),
		Drawn {
			sentence: format!(
				"{} cohorts came back, each a row of returns per {}. A cell is that period's \
				 returns over the cohort's size, shaded in one hue so darker means more stayed; \
				 the top row is the mean, weighted by each cohort's size.",
				result.cohorts.len(),
				label.trim().to_lowercase()
			),
			rows: super::table(&rows),
		},
	)
}

/// The trends builder's controls.
pub fn trends_builder(path: &str, params: &Params, query: &Query, cx: &Builder<'_>) -> Element {
	let Query::Trends(trends) = query else {
		return View();
	};
	let definitions = cx.definitions;
	let names: Vec<String> = trends
		.series
		.iter()
		.map(|series| series.event.clone())
		.collect();
	let mut series = View().class("flex-col gap-2").test_id("series");

	for (index, name) in names.iter().enumerate() {
		let mut row = View()
			.class("flex-row items-center gap-3 rounded-lg bg-[var(--admin-sunken)] px-3 py-2.5")
			.test_id(format!("series:{index}"))
			.child(badge(index))
			.child(
				View()
					.class("min-w-0 flex-1 flex-col gap-0.5")
					.child(event_name(definitions, name))
					.child(Text(math_label(trends.math)).class(style::LABEL)),
			);

		if names.len() > 1 {
			let rest: Vec<String> = names
				.iter()
				.enumerate()
				.filter(|(other, _)| *other != index)
				.map(|(_, name)| name.clone())
				.collect();
			row = row.child(
				Pressable(format!("Remove {name}"))
					.href(params::href(path, &params::with_list(params, "s", &rest)))
					.class("cursor-pointer rounded-md px-1.5 hover:bg-[var(--admin-line)]")
					.child(Text("×").class("text-base text-[var(--admin-ink-3)]")),
			);
		}

		series = series.child(row);
	}

	if names.len() < params::MAX_SERIES {
		series = series.child(event_menu(
			"+ Add series",
			"add-series",
			definitions,
			&names,
			|name| {
				let mut next = names.clone();
				next.push(name.to_owned());
				params::href(path, &params::with_list(params, "s", &next))
			},
		));
	}

	let first = names.first().cloned().unwrap_or_default();
	let math = super::segmented(
		"Count",
		&[
			(
				"Total".to_owned(),
				params::href(path, &params::with(params, "math", "total")),
				trends.math == Math::Total,
			),
			(
				"People".to_owned(),
				params::href(path, &params::with(params, "math", "people")),
				trends.math == Math::UniquePersons,
			),
			(
				"Sessions".to_owned(),
				params::href(path, &params::with(params, "math", "sessions")),
				trends.math == Math::UniqueSessions,
			),
		],
	);
	let interval = super::segmented(
		"Interval",
		&[
			(
				"Hour".to_owned(),
				params::href(path, &params::with(params, "interval", "hour")),
				trends.interval == Interval::Hour,
			),
			(
				"Day".to_owned(),
				params::href(path, &params::with(params, "interval", "day")),
				trends.interval == Interval::Day,
			),
			(
				"Week".to_owned(),
				params::href(path, &params::with(params, "interval", "week")),
				trends.interval == Interval::Week,
			),
		],
	);
	let compare = super::segmented(
		"Compare",
		&[
			(
				"Previous period".to_owned(),
				params::href(path, &params::with(params, "compare", "1")),
				trends.compare,
			),
			(
				"Off".to_owned(),
				params::href(path, &params::with(params, "compare", "0")),
				!trends.compare,
			),
		],
	);
	let mut card = View().class(style::CARD).test_id("builder").child(
		View()
			.class("grid grid-cols-2 gap-5 max-md:grid-cols-1")
			.child(
				View()
					.class("flex-col gap-1.5")
					.child(Text("Series").class("text-sm font-semibold text-[var(--admin-ink-2)]"))
					.child(series),
			)
			.child(breakdown_menu(path, params, definitions, &first)),
	);

	if let Some(note) = cx.note(
		"breakdown",
		"Breakdowns split each line by a property. Try one your struct sends, like plan: it comes \
		 straight from its fields.",
	) {
		card = card.child(note);
	}

	card.child(
		View()
			.class("flex-row flex-wrap gap-6")
			.child(math)
			.child(interval)
			.child(compare),
	)
}

fn steps_editor(path: &str, params: &Params, steps: &[String], cx: &Builder<'_>) -> Element {
	let definitions = cx.definitions;
	let mut list = View().class("flex-col gap-2").test_id("steps");

	for (index, step) in steps.iter().enumerate() {
		let mut row = View()
			.class("flex-row items-center gap-3 rounded-lg bg-[var(--admin-sunken)] px-3 py-2")
			.test_id(format!("step:{index}"))
			.child(Text((index + 1).to_string()).class(
				"h-6 w-6 shrink-0 rounded-md border border-[var(--admin-line-strong)] text-center \
				 text-xs font-semibold leading-6 text-[var(--admin-ink)]",
			))
			.child(
				View()
					.class("min-w-0 flex-1")
					.child(event_name(definitions, step)),
			);

		if steps.len() > 2 {
			let rest: Vec<String> = steps
				.iter()
				.enumerate()
				.filter(|(other, _)| *other != index)
				.map(|(_, name)| name.clone())
				.collect();
			row = row.child(
				Pressable(format!("Remove step {}", index + 1))
					.href(params::href(path, &params::with_list(params, "s", &rest)))
					.class("cursor-pointer rounded-md px-1.5 hover:bg-[var(--admin-line)]")
					.child(Text("×").class("text-base text-[var(--admin-ink-3)]")),
			);
		}

		list = list.child(row);
	}

	if steps.len() < params::MAX_STEPS {
		list = list.child(event_menu(
			"+ Add step",
			"add-step",
			definitions,
			&[],
			|name| {
				let mut next = steps.to_vec();
				next.push(name.to_owned());
				params::href(path, &params::with_list(params, "s", &next))
			},
		));
	}

	list
}

pub fn funnels_builder(path: &str, params: &Params, query: &Query, cx: &Builder<'_>) -> Element {
	let Query::Funnels(funnels) = query else {
		return View();
	};
	let window_days = funnels.window_seconds.unwrap_or(14 * 86_400) / 86_400;
	let window = super::segmented(
		"Converted within",
		&[1_u64, 7, 14, 30]
			.iter()
			.map(|days| {
				(
					format!("{days} day{}", if *days == 1 { "" } else { "s" }),
					params::href(path, &params::with(params, "window", days.to_string())),
					window_days == *days,
				)
			})
			.collect::<Vec<_>>(),
	);
	let order = super::segmented(
		"Order",
		&[
			(
				"In order".to_owned(),
				params::href(path, &params::with(params, "order", "ordered")),
				funnels.order == FunnelOrder::Ordered,
			),
			(
				"Strictly".to_owned(),
				params::href(path, &params::with(params, "order", "strict")),
				funnels.order == FunnelOrder::Strict,
			),
			(
				"Any order".to_owned(),
				params::href(path, &params::with(params, "order", "any")),
				funnels.order == FunnelOrder::Unordered,
			),
		],
	);
	let mut card = View().class(style::CARD).test_id("builder").child(
		View()
			.class("grid grid-cols-2 gap-5 max-md:grid-cols-1")
			.child(
				View()
					.class("flex-col gap-1.5")
					.child(Text("Steps").class("text-sm font-semibold text-[var(--admin-ink-2)]"))
					.child(steps_editor(path, params, &funnels.steps, cx)),
			)
			.child(
				View()
					.class("flex-col gap-5")
					.child(window)
					.child(order)
					.child(breakdown_menu(
						path,
						params,
						cx.definitions,
						funnels.steps.first().map_or("", String::as_str),
					)),
			),
	);

	if let Some(note) = cx.note(
		"funnel-drop-off",
		"Press the hatched part of a column to see exactly who stopped there.",
	) {
		card = card.child(note);
	}

	card
}

pub fn retention_builder(path: &str, params: &Params, query: &Query, cx: &Builder<'_>) -> Element {
	let Query::Retention(retention) = query else {
		return View();
	};
	let definitions = cx.definitions;
	let pick = |label: &str, key: &str, current: &str| {
		View()
			.class("flex-col gap-1.5")
			.child(Text(label.to_owned()).class("text-sm font-semibold text-[var(--admin-ink-2)]"))
			.child(
				View()
					.class(
						"flex-row items-center gap-3 rounded-lg bg-[var(--admin-sunken)] px-3 py-2",
					)
					.child(event_name(definitions, current)),
			)
			.child(event_menu(
				"Change ▾",
				key,
				definitions,
				&[current.to_owned()],
				|name| params::href(path, &params::with(params, key, name)),
			))
	};
	let kind = super::segmented(
		"Counts",
		&[
			(
				"First time".to_owned(),
				params::href(path, &params::with(params, "kind", "first")),
				retention.kind == RetentionKind::FirstTime,
			),
			(
				"Every time".to_owned(),
				params::href(path, &params::with(params, "kind", "recurring")),
				retention.kind == RetentionKind::Recurring,
			),
		],
	);
	let period = super::segmented(
		"Period",
		&[
			(
				"Day".to_owned(),
				params::href(path, &params::with(params, "period", "day")),
				retention.period == Interval::Day,
			),
			(
				"Week".to_owned(),
				params::href(path, &params::with(params, "period", "week")),
				retention.period != Interval::Day,
			),
		],
	);
	let mut card = View()
		.class(style::CARD)
		.test_id("builder")
		.child(
			View()
				.class("grid grid-cols-2 gap-5 max-md:grid-cols-1")
				.child(pick("People who did", "start", &retention.start_event))
				.child(pick(
					"and came back to do",
					"return",
					&retention.return_event,
				)),
		)
		.child(
			View()
				.class("flex-row flex-wrap gap-6")
				.child(kind)
				.child(period),
		);

	if let Some(note) = cx.note(
		"retention-cells",
		"Each row is the people who started in that period. Press a cell to see who came back.",
	) {
		card = card.child(note);
	}

	card
}

/// The kind of a property value's source in a filter: what an overview row
/// filters by.
pub fn session_filter(key: &str, value: &str) -> String {
	params::filter_value(FilterSource::Session, key, value)
}
