//! Live: events as they arrive, a few seconds after they happen.
//!
//! The page starts with the latest events, read with an `Events` query,
//! then the script follows the admin's own stream of the service's
//! `live_events` tail and adds each one at the top.

use leptoskit::native::Role;
use leptoskit::prelude::*;
use leptoskit_analytics::query::Answer;
use leptoskit_analytics::query::DateRange;
use leptoskit_analytics::query::EventsQuery;
use leptoskit_analytics::query::Query;
use leptoskit_analytics::query::QueryResult;

use super::Context;
use super::format;
use super::stations;
use super::stations::Drawn;
use crate::components;
use crate::components::Tone;
use crate::shell::Screen;
use crate::shell::Section;
use crate::style;

/// The most events the page holds.
const KEEP: u32 = 50;

/// One event's row: time, name, who, and its properties under a toggle.
pub(crate) fn row(
	name: &str,
	distinct_id: &str,
	timestamp: &str,
	properties: &serde_json::Value,
	typed: bool,
) -> Element {
	let at = format::instant(timestamp)
		.map(|at| format::clock(&at) + &format!(":{:02}", chrono::Timelike::second(&at)))
		.unwrap_or_default();
	let detail = properties
		.get("$pathname")
		.map(format::value)
		.unwrap_or_default();
	// Every row carries the badge, hidden unless the event is typed, so the
	// script sets it from each new event rather than from the row it copies.
	let title = View()
		.class("min-w-0 flex-1 flex-row items-center gap-1.5")
		.child(
			Text(name.to_owned())
				.class(style::CODE)
				.class("truncate font-semibold"),
		)
		.child(
			super::struct_badge()
				.aria_hidden(!typed)
				.test_id("struct-badge"),
		);

	View()
		.class("flex-col border-b border-[var(--admin-line)]")
		.test_id("live-event")
		.aria_description(name.to_owned())
		.child(
			Pressable(format!("{name} from {distinct_id} at {at}"))
				.aria_expanded(false)
				.class("cursor-pointer hover:bg-[var(--admin-sunken)]")
				.test_id("how-toggle")
				.child(
					View()
						.class("flex-row items-center gap-3 px-3 py-2")
						.child(
							Text(at).class(
								"w-20 shrink-0 text-xs tabular-nums text-[var(--admin-ink-3)]",
							),
						)
						.child(title)
						.child(
							Text(detail).class(
								"w-40 truncate text-xs text-[var(--admin-ink-2)] max-md:hidden",
							),
						)
						.child(Text(distinct_id.to_owned()).class(
							"w-44 truncate text-end text-xs text-[var(--admin-ink-3)] \
							 max-sm:hidden",
						)),
				),
		)
		.child(
			View()
				.class("px-3 pb-3")
				.test_id("how-panel")
				.child(components::paper_code(
					&serde_json::to_string_pretty(properties).unwrap_or_default(),
					"JSON",
				)),
		)
}

/// The Live page.
#[expect(clippy::too_many_lines, reason = "one page, top to bottom")]
pub(crate) async fn page(cx: &Context<'_>) -> Screen {
	let hub = cx.connection.live();
	let pulse = hub.pulse();
	let query = Query::Events(EventsQuery {
		range: DateRange::last_days(1),
		event: None,
		filters: Vec::new(),
		columns: Vec::new(),
		limit: KEEP,
		cursor: None,
	});
	let answer = cx.connection.run(query.clone()).await;
	let typed = |name: &str| {
		cx.facts
			.definitions
			.iter()
			.any(|definition| definition.name == name && definition.fields.is_some())
	};
	let mut list = View()
		.class("flex-col")
		.role(Role::Log)
		.aria_label("Events as they arrive")
		.test_id("live-events");
	let mut count = 0;
	let mut sample = None;

	match &answer {
		Ok(Answer {
			result: QueryResult::Events(page),
			..
		}) => {
			count = page.events.len();
			sample = page.events.first();

			for event in &page.events {
				list = list.child(row(
					&event.name,
					&event.distinct_id,
					&event.timestamp,
					&event.properties,
					typed(&event.name),
				));
			}
		}
		Ok(_) => {}
		Err(failure) => return super::unreachable("Live", Section::Live, failure),
	}

	let status = match &pulse.status {
		super::live::Status::Live => components::status_word(Tone::Good, "Listening"),
		super::live::Status::Connecting => components::status_word(Tone::Info, "Connecting"),
		super::live::Status::Disconnected(reason) => {
			components::status_word(Tone::Warn, format!("Reconnecting: {reason}"))
		}
	};
	let controls = View()
		.class("flex-row flex-wrap items-center gap-3")
		.child(
			TextInput()
				.placeholder("Filter by event name")
				.aria_label("Filter by event name")
				.class(
					"min-w-0 flex-1 rounded-lg border border-[var(--admin-line-strong)] \
					 bg-[var(--admin-surface)] px-3 py-2 text-sm text-[var(--admin-ink)]",
				)
				.test_id("live-filter"),
		)
		.child(
			Button("Pause")
				.aria_pressed(false)
				.class(components::Kind::Secondary.class_name())
				.test_id("live-pause"),
		);
	let rate = View()
		.class("flex-row flex-wrap items-center gap-4")
		.child(
			View()
				.class("flex-col")
				.child(
					Text(format!("{:.1}/s", pulse.per_second))
						.class(style::HERO_NUMBER)
						.test_id("live-rate"),
				)
				.child(Text("events per second, last 10 s").class(style::LABEL)),
		)
		.child(
			View()
				.class("flex-col")
				.child(
					Text(format::thousands(pulse.per_minute))
						.class("text-2xl font-semibold tabular-nums text-[var(--admin-ink)]")
						.test_id("live-minute"),
				)
				.child(Text("in the last minute").class(style::LABEL)),
		)
		.child(
			#[expect(clippy::cast_precision_loss, reason = "counts for drawing")]
			super::draw::sparkline(
				&pulse
					.seconds
					.iter()
					.map(|count| *count as f64)
					.collect::<Vec<_>>(),
				"h-10 w-48",
			)
			.test_id("live-spark"),
		)
		.child(View().class("grow"))
		.child(status.test_id("live-status"));
	let drawer = stations::drawer(stations::Source {
		id: "live",
		title: "Live events · tail".to_owned(),
		query: &query,
		answer: answer.as_ref().ok(),
		definitions: &cx.facts.definitions,
		sample,
		url: cx.connection.url(),
		drawn: Drawn {
			sentence: format!(
				"The page started with the latest {count} events from an Events query; after \
				 that, each new one comes from the service's live_events stream, which the \
				 admin's server follows once and shares with every open page. The rate counts \
				 arrivals per second over the last minute."
			),
			rows: Vec::new(),
		},
	});
	let mut card = components::card("Events")
		.aside("newest first")
		.workings("live", "live_events")
		.child(controls);

	if let Some(note) = super::note(
		cx.preferences,
		"live-pause",
		"Pause holds the list still while you read; new events wait and come in when you resume.",
	) {
		card = card.child(note);
	}

	let content = View()
		.class("flex-col gap-5")
		.test_id("live-page")
		.child(super::header(
			"Live",
			"events as they arrive, a few seconds after they happen",
			Vec::new(),
		))
		.child(View().class(style::CARD).child(rate))
		.child(card.child(list).build());

	Screen {
		title: "Live".to_owned(),
		section: Section::Live,
		content,
		drawers: vec![drawer],
		open: None,
	}
}
