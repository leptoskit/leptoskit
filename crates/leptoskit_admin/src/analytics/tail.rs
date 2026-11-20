//! Live: events as they arrive, a few seconds after they happen.
//!
//! The page starts with the latest events, read with an `Events` query;
//! then the pages' Wasm follows the admin's own stream of the service's
//! `live_events` tail and adds each one at the top.

use leptoskit_admin_pages::model::Page;
use leptoskit_admin_pages::model::Section;
use leptoskit_admin_pages::model::live::Arrival;
use leptoskit_admin_pages::model::live::Live;
use leptoskit_analytics::query::Answer;
use leptoskit_analytics::query::DateRange;
use leptoskit_analytics::query::EventsQuery;
use leptoskit_analytics::query::Query;
use leptoskit_analytics::query::QueryResult;

use super::Context;
use super::Shown;
use super::stations;
use super::stations::Drawn;

/// The most events the page holds.
const KEEP: u32 = 50;

/// The Live page.
pub(crate) async fn page(cx: &Context<'_>) -> Shown {
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
	let mut events = Vec::new();
	let mut sample = None;

	match &answer {
		Ok(Answer {
			result: QueryResult::Events(page),
			..
		}) => {
			sample = page.events.first();
			events = page
				.events
				.iter()
				.map(|event| {
					Arrival {
						seq: 0,
						name: event.name.clone(),
						distinct_id: event.distinct_id.clone(),
						timestamp: event.timestamp.clone(),
						properties: event.properties.clone(),
						typed: typed(&event.name),
					}
				})
				.collect();
		}
		Ok(_) => {}
		Err(failure) => return super::unreachable("Live", Section::Live, failure).into(),
	}

	let count = events.len();
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

	Page::Live(Box::new(Live {
		events,
		drawer,
		pause_note: cx.preferences.note("live-pause"),
	}))
	.into()
}
