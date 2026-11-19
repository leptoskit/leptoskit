//! People: everyone your events came from, and one person whole: their
//! properties, every id merged into them, what they did, and their
//! sessions, with export and delete behind a confirmation.

use leptoskit_admin_pages::model::Page;
use leptoskit_admin_pages::model::people::Event;
use leptoskit_admin_pages::model::people::People;
use leptoskit_admin_pages::model::people::Person;
use leptoskit_admin_pages::model::people::PersonRow;
use leptoskit_admin_pages::model::people::Session;
use leptoskit_analytics::service::read::PersonSummary;
use uuid::Uuid;

use super::Context;
use super::Shown;
use super::format;
use super::params;
use crate::BASE;
use crate::components::Line;
use crate::shell::Section;
use crate::workings::Body;
use crate::workings::Drawer;
use crate::workings::Station;

const PAGE: u32 = 50;

/// A property key the list may filter by: letters, digits, `_`, `$`, `.`,
/// and `-`. The service puts the key into a JSON path, so nothing else
/// reaches it.
pub(crate) fn valid_property(key: &str) -> bool {
	!key.is_empty()
		&& key.len() <= 64
		&& key
			.bytes()
			.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$' | b'.' | b'-'))
}

fn row(person: &PersonSummary, now: &chrono::DateTime<chrono::Utc>) -> PersonRow {
	PersonRow {
		name: format::person_name(person),
		identified: person.is_identified,
		plan: person
			.properties
			.get("plan")
			.map(format::value)
			.unwrap_or_default(),
		since: format::instant(&person.created_at)
			.map(|time| format::ago(&time, now))
			.unwrap_or_default(),
		href: super::person_href(&person.id.to_string()),
	}
}

/// The people list, with a search and a property filter.
pub(crate) async fn list(cx: &Context<'_>) -> Shown {
	let typed = |key: &str| {
		cx.params
			.get(key)
			.map(|text| text.trim().to_owned())
			.unwrap_or_default()
	};
	let search = Some(typed("search")).filter(|search| !search.is_empty());
	let property = Some(typed("property")).filter(|property| valid_property(property));
	let value = Some(typed("value")).filter(|value| !value.is_empty());
	let offset: u32 = cx
		.params
		.get("offset")
		.and_then(|offset| offset.parse().ok())
		.unwrap_or(0);
	let now = chrono::Utc::now();
	let path = format!("{BASE}/analytics/people");
	let filter = property.zip(value);
	let people = cx
		.connection
		.persons(
			search.clone(),
			filter.as_ref().map(|(property, _)| property.clone()),
			filter.as_ref().map(|(_, value)| value.clone()),
			PAGE + 1,
			offset,
		)
		.await;
	let people = match people {
		Ok(people) => people,
		Err(failure) => return super::unreachable("People", Section::People, &failure).into(),
	};
	let more = people.len() > PAGE as usize;
	let page_at = |offset: u32| {
		params::href(
			&path,
			&params::with(cx.params, "offset", offset.to_string()),
		)
	};

	Page::People(People {
		search: typed("search"),
		property: typed("property"),
		value: typed("value"),
		filtered: search.is_some() || filter.is_some(),
		rows: people
			.iter()
			.take(PAGE as usize)
			.map(|person| row(person, &now))
			.collect(),
		previous: (offset > 0).then(|| page_at(offset.saturating_sub(PAGE))),
		next: more.then(|| page_at(offset + PAGE)),
		merge_note: cx.preferences.note("person-merge"),
	})
	.into()
}

/// The text that confirms deleting `person`: their name as the page shows it.
pub(crate) fn confirmation(person: &PersonSummary) -> String {
	format::person_name(person)
}

/// A session in one line: where it started, how much it saw, where it came
/// from, and what it ran on, leaving out what the session didn't record.
fn session_line(session: &leptoskit_analytics::service::read::SessionView) -> String {
	let mut parts = vec![
		if session.entry_pathname.is_empty() {
			"—".to_owned()
		} else {
			session.entry_pathname.clone()
		},
		format!(
			"{} pageview{}",
			session.pageview_count,
			if session.pageview_count == 1 { "" } else { "s" }
		),
		if session.channel.is_empty() {
			"Direct".to_owned()
		} else {
			session.channel.clone()
		},
	];
	let device = match (session.browser.is_empty(), session.os.is_empty()) {
		(false, false) => Some(format!("{} on {}", session.browser, session.os)),
		(false, true) => Some(session.browser.clone()),
		(true, false) => Some(session.os.clone()),
		(true, true) => None,
	};
	parts.extend(device);

	if session.is_bounce {
		parts.push("bounced".to_owned());
	}

	parts.join(" · ")
}

/// One person.
#[expect(clippy::too_many_lines, reason = "one page, top to bottom")]
pub(crate) async fn person(cx: &Context<'_>, id: Uuid) -> Option<Shown> {
	let (person, events, sessions) = tokio::join!(
		cx.connection.person(id),
		cx.connection.person_events(id, 50),
		cx.connection.person_sessions(id, 20),
	);
	let person = match person {
		Ok(Some(person)) => person,
		Ok(None) => return None,
		Err(failure) => {
			return Some(super::unreachable("Person", Section::People, &failure).into());
		}
	};
	let now = chrono::Utc::now();
	let name = format::person_name(&person.person);
	let properties = person
		.person
		.properties
		.as_object()
		.map(|map| {
			map.iter()
				.map(|(key, value)| (key.clone(), format::value(value)))
				.collect()
		})
		.unwrap_or_default();
	let typed = |name: &str| {
		cx.facts
			.definitions
			.iter()
			.any(|definition| definition.name == name && definition.fields.is_some())
	};
	let timeline = events
		.as_ref()
		.map_err(|failure| failure.0.clone())
		.map(|events| {
			events
				.iter()
				.map(|event| {
					let detail = event.properties.get("$pathname").map_or_else(
						|| {
							event
								.properties
								.as_object()
								.map(|map| {
									map.iter()
										.filter(|(key, _)| !key.starts_with('$'))
										.take(3)
										.map(|(key, value)| {
											format!("{key}: {}", format::value(value))
										})
										.collect::<Vec<_>>()
										.join(" · ")
								})
								.unwrap_or_default()
						},
						format::value,
					);
					Event {
						when: format::instant(&event.timestamp)
							.map(|at| format!("{} {}", format::date(&at), format::clock(&at)))
							.unwrap_or_default(),
						name: event.name.clone(),
						typed: typed(&event.name),
						detail,
						distinct_id: event.distinct_id.clone(),
					}
				})
				.collect()
		});
	let session_rows = sessions
		.as_ref()
		.map_err(|failure| failure.0.clone())
		.map(|sessions| {
			sessions
				.iter()
				.map(|session| {
					#[expect(clippy::cast_precision_loss, reason = "a duration for display")]
					let length = format::duration(session.duration_ms as f64 / 1000.0);
					Session {
						when: format::instant(&session.first_timestamp)
							.map(|at| format::ago(&at, &now))
							.unwrap_or_default(),
						length,
						line: session_line(session),
					}
				})
				.collect()
		});

	// The person row is created when the service first ingests one of their
	// events, which for imported or backdated events is later than the events
	// themselves; the earliest thing we can see is the honest answer.
	let event_times = events
		.iter()
		.flatten()
		.map(|event| event.timestamp.as_str());
	let session_times = sessions
		.iter()
		.flatten()
		.map(|session| session.first_timestamp.as_str());
	let since = std::iter::once(person.person.created_at.as_str())
		.chain(event_times)
		.chain(session_times)
		.filter_map(format::instant)
		.min()
		.map(|at| format::date(&at))
		.unwrap_or_default();
	let summary = format!(
		"{} · first seen {since} · {} id{}",
		if person.person.is_identified {
			"identified"
		} else {
			"anonymous"
		},
		person.distinct_ids.len(),
		if person.distinct_ids.len() == 1 {
			""
		} else {
			"s"
		}
	);
	let drawer = Drawer {
		id: "person".to_owned(),
		title: format!("{name} · person"),
		stations: vec![
			Station::new(
				"Store",
				"`persons`, `person_distinct_ids`",
				"A person is one row in `persons`; each id they've used is a row in \
				 `person_distinct_ids` pointing at it. Identify adds a row; a merge repoints the \
				 anonymous one.",
				Body::Code(
					person
						.distinct_ids
						.iter()
						.map(|distinct| Line::plain(format!("{distinct:<36} → {id}")))
						.collect(),
					"text".to_owned(),
				),
			),
			Station::new(
				"Ask",
				"the operations",
				"The page is four app-only operations on the analytics service, the same your \
				 server can call.",
				Body::Code(
					[
						format!(
							"let person = call_as_app(Person {{ id: {id:?}.parse()? }}).await?;"
						),
						"let events = call_as_app(PersonEvents { id, limit: 50, offset: 0 \
						 }).await?;"
							.to_string(),
						"let sessions = call_as_app(PersonSessions { id, limit: 20, offset: 0 \
						 }).await?;"
							.to_string(),
						"// Export and delete: ExportPerson { id }, DeletePerson { id }".to_owned(),
					]
					.into_iter()
					.map(Line::plain)
					.collect(),
					"Rust".to_owned(),
				),
			),
		],
	};

	Some(
		Page::Person(Box::new(Person {
			id: id.to_string(),
			name,
			summary,
			properties,
			ids_note: person.distinct_ids.len() > 1 && cx.preferences.note("person-ids"),
			distinct_ids: person.distinct_ids,
			events: timeline,
			sessions: session_rows,
			drawer,
		}))
		.into(),
	)
}
