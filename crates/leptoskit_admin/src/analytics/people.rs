//! People: everyone your events came from, and one person whole: their
//! properties, every id merged into them, what they did, and their
//! sessions, with export and delete behind a confirmation.

use leptoskit::native::Role;
use leptoskit::prelude::*;
use leptoskit_analytics::service::read::PersonSummary;
use uuid::Uuid;

use super::Context;
use super::format;
use super::params;
use crate::BASE;
use crate::components;
use crate::components::Kind as ButtonKind;
use crate::components::Line;
use crate::components::Tone;
use crate::shell::Screen;
use crate::shell::Section;
use crate::style;
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

fn avatar(name: &str, size: &str) -> Element {
	Text(format::initials(name))
		.class(
			"shrink-0 rounded-full bg-[var(--admin-sunken)] text-center font-semibold \
			 text-[var(--admin-ink)]",
		)
		.class(size)
		.aria_hidden(true)
}

fn row(person: &PersonSummary, now: &chrono::DateTime<chrono::Utc>) -> Element {
	let name = format::person_name(person);
	let plan = person
		.properties
		.get("plan")
		.map(format::value)
		.unwrap_or_default();
	let since = format::instant(&person.created_at)
		.map(|time| format::ago(&time, now))
		.unwrap_or_default();
	Pressable(name.clone())
		.href(super::person_href(&person.id.to_string()))
		.class("cursor-pointer rounded-lg hover:bg-[var(--admin-sunken)]")
		.test_id("person-row")
		.child(
			View()
				.class(
					"flex-row items-center gap-3 border-b border-[var(--admin-line)] px-3 py-2.5",
				)
				.child(avatar(&name, "h-8 w-8 text-xs leading-8"))
				.child(
					View()
						.class("min-w-0 flex-1 flex-col")
						.child(
							Text(name)
								.class("truncate text-sm font-semibold text-[var(--admin-ink)]"),
						)
						.child(
							Text(if person.is_identified {
								"identified"
							} else {
								"anonymous"
							})
							.class(style::LABEL),
						),
				)
				.child(Text(plan).class("w-20 text-sm text-[var(--admin-ink-2)] max-sm:hidden"))
				.child(
					Text(format!("first seen {since}"))
						.class("w-36 text-end text-xs text-[var(--admin-ink-3)] max-md:hidden"),
				),
		)
}

/// The people list, with a search and a property filter.
pub(crate) async fn list(cx: &Context<'_>) -> Screen {
	let search = cx
		.params
		.get("search")
		.map(|search| search.trim().to_owned())
		.filter(|search| !search.is_empty());
	let property = cx
		.params
		.get("property")
		.map(|property| property.trim().to_owned())
		.filter(|property| valid_property(property));
	let value = cx
		.params
		.get("value")
		.map(|value| value.trim().to_owned())
		.filter(|value| !value.is_empty());
	let offset: u32 = cx
		.params
		.get("offset")
		.and_then(|offset| offset.parse().ok())
		.unwrap_or(0);
	let now = chrono::Utc::now();
	let path = format!("{BASE}/analytics/people");
	let filter = property.clone().zip(value.clone());
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
		Err(failure) => return super::unreachable("People", Section::People, &failure),
	};
	let more = people.len() > PAGE as usize;
	let mut list = View().class("flex-col").role(Role::List).test_id("people");

	for person in people.iter().take(PAGE as usize) {
		list = list.child(row(person, &now));
	}

	if people.is_empty() {
		list = list.child(
			Text(if search.is_some() || filter.is_some() {
				"Nobody matches."
			} else {
				"Nobody yet. A person appears once your app identifies someone, with web::identify \
				 after they sign in."
			})
			.class(style::MUTED)
			.class("px-3 py-2"),
		);
	}

	let field = |label: &str, test_id: &str, key: &str, placeholder: &str, width: &str| {
		View()
			.class("flex-col gap-1.5")
			.class(width)
			.child(Text(label.to_owned()).class("text-sm font-semibold text-[var(--admin-ink-2)]"))
			.child(
				TextInput()
					.value(cx.params.get(key).cloned().unwrap_or_default())
					.placeholder(placeholder.to_owned())
					.aria_label(label.to_owned())
					.class(
						"rounded-lg border border-[var(--admin-line-strong)] \
						 bg-[var(--admin-surface)] px-3 py-2 text-sm text-[var(--admin-ink)]",
					)
					.test_id(test_id.to_owned()),
			)
	};
	let filters = View()
		.class("flex-row flex-wrap items-end gap-3")
		.test_id("people-filters")
		.child(field(
			"Search",
			"people-search",
			"search",
			"an email, an id, a name…",
			"min-w-0 flex-1",
		))
		.child(field(
			"Property",
			"people-property",
			"property",
			"plan",
			"w-36",
		))
		.child(field("equals", "people-value", "value", "pro", "w-36"))
		.child(
			Button("Filter")
				.class(ButtonKind::Secondary.class_name())
				.test_id("people-apply"),
		);
	let mut pages = View().class("flex-row gap-3");

	if offset > 0 {
		pages = pages.child(components::link_button(
			ButtonKind::Secondary,
			"Previous",
			params::href(
				&path,
				&params::with(cx.params, "offset", offset.saturating_sub(PAGE).to_string()),
			),
		));
	}

	if more {
		pages = pages.child(components::link_button(
			ButtonKind::Secondary,
			"Next",
			params::href(
				&path,
				&params::with(cx.params, "offset", (offset + PAGE).to_string()),
			),
		));
	}

	let mut card = components::card("People")
		.aside(if search.is_some() || filter.is_some() {
			"filtered"
		} else {
			"newest first"
		})
		.child(filters);

	if let Some(note) = super::note(
		cx.preferences,
		"person-merge",
		"One person can have many ids: the anonymous one from before they signed in, then their \
		 account's. Identify merges them, so their history stays whole.",
	) {
		card = card.child(note);
	}

	let content = View()
		.class("flex-col gap-5")
		.child(super::header(
			"People",
			"everyone your events came from, newest first",
			Vec::new(),
		))
		.child(card.child(list).child(pages).build());

	Screen {
		title: "People".to_owned(),
		section: Section::People,
		content,
		drawers: Vec::new(),
		open: None,
	}
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
pub(crate) async fn person(cx: &Context<'_>, id: Uuid) -> Option<Screen> {
	let (person, events, sessions) = tokio::join!(
		cx.connection.person(id),
		cx.connection.person_events(id, 50),
		cx.connection.person_sessions(id, 20),
	);
	let person = match person {
		Ok(Some(person)) => person,
		Ok(None) => return None,
		Err(failure) => return Some(super::unreachable("Person", Section::People, &failure)),
	};
	let now = chrono::Utc::now();
	let name = format::person_name(&person.person);
	let mut properties = View().class("flex-col").test_id("person-properties");

	if let Some(map) = person.person.properties.as_object() {
		for (key, value) in map {
			properties = properties.child(
				View()
					.class("flex-row gap-3 border-b border-[var(--admin-line)] py-1.5")
					.child(
						Text(key.clone())
							.class("w-32 shrink-0 truncate")
							.class(style::CODE),
					)
					.child(
						Text(format::value(value))
							.class("min-w-0 flex-1 truncate text-sm text-[var(--admin-ink)]"),
					),
			);
		}

		if map.is_empty() {
			properties = properties.child(Text("No properties set.").class(style::MUTED));
		}
	}

	let mut ids = View().class("flex-col gap-1.5").test_id("distinct-ids");

	for distinct in &person.distinct_ids {
		ids = ids.child(Text(distinct.clone()).class(style::CODE).class("truncate"));
	}

	let mut timeline = View().class("flex-col").test_id("person-timeline");
	let typed = |name: &str| {
		cx.facts
			.definitions
			.iter()
			.any(|definition| definition.name == name && definition.fields.is_some())
	};

	match &events {
		Ok(events) => {
			for event in events {
				let at = format::instant(&event.timestamp);
				let detail = event.properties.get("$pathname").map_or_else(
					|| {
						event
							.properties
							.as_object()
							.map(|map| {
								map.iter()
									.filter(|(key, _)| !key.starts_with('$'))
									.take(3)
									.map(|(key, value)| format!("{key}: {}", format::value(value)))
									.collect::<Vec<_>>()
									.join(" · ")
							})
							.unwrap_or_default()
					},
					format::value,
				);
				timeline = timeline.child(
					View()
						.class(
							"flex-row items-start gap-3 border-b border-[var(--admin-line)] py-2",
						)
						.test_id("timeline-event")
						.child(
							Text(
								at.map(|at| {
									format!("{} {}", format::date(&at), format::clock(&at))
								})
								.unwrap_or_default(),
							)
							.class("w-32 shrink-0 text-xs tabular-nums text-[var(--admin-ink-3)]"),
						)
						.child(
							View()
								.class("min-w-0 flex-1 flex-col")
								.child(name_row(&event.name, typed(&event.name)))
								.child(
									Text(detail)
										.class("truncate text-xs text-[var(--admin-ink-2)]"),
								),
						)
						.child(Text(event.distinct_id.clone()).class(
							"w-40 truncate text-end text-xs text-[var(--admin-ink-3)] \
							 max-md:hidden",
						)),
				);
			}

			if events.is_empty() {
				timeline = timeline.child(Text("No events.").class(style::MUTED));
			}
		}
		Err(failure) => {
			timeline = timeline.child(components::status_word(Tone::Bad, failure.0.clone()));
		}
	}

	let mut session_list = View().class("flex-col").test_id("sessions");

	match &sessions {
		Ok(sessions) => {
			for session in sessions {
				let start = format::instant(&session.first_timestamp);
				#[expect(clippy::cast_precision_loss, reason = "a duration for display")]
				let length = format::duration(session.duration_ms as f64 / 1000.0);
				session_list = session_list.child(
					View()
						.class("flex-col gap-0.5 border-b border-[var(--admin-line)] py-2")
						.child(
							View()
								.class("flex-row items-center justify-between gap-2")
								.child(
									Text(
										start.map(|at| format::ago(&at, &now)).unwrap_or_default(),
									)
									.class("text-sm font-semibold text-[var(--admin-ink)]"),
								)
								.child(
									Text(length)
										.class("text-xs tabular-nums text-[var(--admin-ink-2)]"),
								),
						)
						.child(
							Text(session_line(session)).class("text-xs text-[var(--admin-ink-3)]"),
						),
				);
			}

			if sessions.is_empty() {
				session_list = session_list.child(Text("No sessions.").class(style::MUTED));
			}
		}
		Err(failure) => {
			session_list =
				session_list.child(components::status_word(Tone::Bad, failure.0.clone()));
		}
	}

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
	let actions = vec![
		Button("Export")
			.class(ButtonKind::Secondary.class_name())
			.aria_description(id.to_string())
			.test_id("person-export"),
		Button("Delete")
			.class(ButtonKind::Danger.class_name())
			.aria_description(format!("{id}|{}", confirmation(&person.person)))
			.test_id("person-delete"),
	];
	let profile = View()
		.class("flex-row flex-wrap items-center gap-4")
		.child(avatar(&name, "h-14 w-14 text-lg leading-[56px]"))
		.child(
			View()
				.class("min-w-[14rem] flex-1 flex-col gap-1")
				.child(
					Heading(1, name.clone())
						.class(style::PAGE_TITLE)
						.class("truncate"),
				)
				.child(
					Text(format!(
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
					))
					.class(style::SECONDARY),
				),
		)
		.child(View().class("flex-row gap-2").children(actions));
	let mut ids_card = components::card("Distinct ids")
		.aside("merged into one person")
		.workings("person", "persons ⨝ person_distinct_ids")
		.child(ids);

	if person.distinct_ids.len() > 1
		&& let Some(note) = super::note(
			cx.preferences,
			"person-ids",
			"These ids were one person all along: identify joined the anonymous one to the \
			 account when they signed in.",
		) {
		ids_card = ids_card.child(note);
	}

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
	let content = View()
		.class("flex-col gap-5")
		.child(components::link(
			"← People",
			format!("{BASE}/analytics/people"),
		))
		.child(profile)
		.child(
			View()
				.class("grid grid-cols-3 gap-4 max-lg:grid-cols-1")
				.test_id("grid-main")
				.child(
					View()
						.class("col-span-2 flex-col gap-4 max-lg:col-span-1")
						.child(
							components::card("Events")
								.aside("newest first")
								.child(timeline)
								.build(),
						),
				)
				.child(
					View()
						.class("flex-col gap-4")
						.child(components::card("Properties").child(properties).build())
						.child(ids_card.build())
						.child(
							components::card("Sessions")
								.aside("latest 20")
								.child(session_list)
								.build(),
						),
				),
		);

	Some(Screen {
		title: name,
		section: Section::People,
		content,
		drawers: vec![drawer],
		open: None,
	})
}

/// An event's name, with the `struct` badge when a Rust type sends it.
fn name_row(name: &str, typed: bool) -> Element {
	let row = View().class("flex-row items-center gap-1.5").child(
		Text(name.to_owned())
			.class(style::CODE)
			.class("font-semibold"),
	);

	if typed {
		row.child(super::struct_badge())
	} else {
		row
	}
}
