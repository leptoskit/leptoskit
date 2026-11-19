//! People: everyone the events came from, and one person whole, with export
//! and delete behind the confirm sheet.

use std::fmt::Write as _;

use leptos::prelude::GetValue as _;
use leptos::prelude::StoredValue;
use leptoskit::native::FocusRequest;
use leptoskit::native::Role;
use leptoskit::prelude::*;

use crate::BASE;
use crate::components;
use crate::components::Kind;
use crate::components::Tone;
use crate::confirm::Ask;
use crate::confirm::Confirm;
use crate::model::people::People;
use crate::model::people::Person;
use crate::model::people::PersonRow;
use crate::style;

/// What the list's search fields hold, and the search they make.
#[derive(Clone, Copy)]
pub(crate) struct Search {
	text: RwSignal<String>,
	property: RwSignal<String>,
	value: RwSignal<String>,
}

impl Search {
	/// The fields, holding what the list was searched by.
	pub(crate) fn new(people: &People) -> Self {
		Self {
			text: RwSignal::new(people.search.clone()),
			property: RwSignal::new(people.property.clone()),
			value: RwSignal::new(people.value.clone()),
		}
	}

	/// The list's address for what the fields hold now.
	fn href(self) -> String {
		let query: Vec<String> = [
			("search", self.text),
			("property", self.property),
			("value", self.value),
		]
		.into_iter()
		.filter_map(|(key, field)| {
			let value = field.get_untracked().trim().to_owned();
			(!value.is_empty()).then(|| format!("{key}={}", encode(&value)))
		})
		.collect();

		if query.is_empty() {
			format!("{BASE}/analytics/people")
		} else {
			format!("{BASE}/analytics/people?{}", query.join("&"))
		}
	}

	fn run(self) {
		crate::go(&self.href());
	}
}

/// `value` as one part of a query string.
fn encode(value: &str) -> String {
	let mut encoded = String::with_capacity(value.len());

	for byte in value.bytes() {
		if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~' | b'$') {
			encoded.push(char::from(byte));
		} else {
			let _ = write!(encoded, "%{byte:02X}");
		}
	}

	encoded
}

fn row(person: &PersonRow) -> Element {
	Pressable(person.name.clone())
		.href(person.href.clone())
		.class("cursor-pointer rounded-lg hover:bg-[var(--admin-sunken)]")
		.test_id("person-row")
		.child(
			View()
				.class(
					"flex-row items-center gap-3 border-b border-[var(--admin-line)] px-3 py-2.5",
				)
				.child(components::avatar(
					&person.name,
					"h-8 w-8 text-xs leading-8",
				))
				.child(
					View()
						.class("min-w-0 flex-1 flex-col")
						.child(
							Text(person.name.clone())
								.class("truncate text-sm font-semibold text-[var(--admin-ink)]"),
						)
						.child(
							Text(if person.identified {
								"identified"
							} else {
								"anonymous"
							})
							.class(style::LABEL),
						),
				)
				.child(
					Text(person.plan.clone())
						.class("w-20 text-sm text-[var(--admin-ink-2)] max-sm:hidden"),
				)
				.child(
					Text(format!("first seen {}", person.since))
						.class("w-36 text-end text-xs text-[var(--admin-ink-3)] max-md:hidden"),
				),
		)
}

/// The people list, with a search and a property filter.
pub(crate) fn list(people: &People, search: Option<Search>) -> Element {
	let mut list = View()
		.class("flex-col")
		.role(Role::List)
		.test_id("people")
		.children(people.rows.iter().map(row));

	if people.rows.is_empty() {
		list = list.child(
			Text(if people.filtered {
				"Nobody matches."
			} else {
				"Nobody yet. A person appears once your app identifies someone, with web::identify \
				 after they sign in."
			})
			.class(style::MUTED)
			.class("px-3 py-2"),
		);
	}

	let field = |label: &str,
	             test_id: &str,
	             typed: &str,
	             bound: Option<RwSignal<String>>,
	             placeholder: &str,
	             width: &str| {
		let mut input = TextInput()
			.value(bound.map_or_else(|| typed.to_owned(), |bound| bound.get()))
			.placeholder(placeholder.to_owned())
			.aria_label(label.to_owned())
			.class(
				"rounded-lg border border-[var(--admin-line-strong)] bg-[var(--admin-surface)] \
				 px-3 py-2 text-sm text-[var(--admin-ink)]",
			)
			.test_id(test_id.to_owned());

		if let (Some(bound), Some(search)) = (bound, search) {
			input = input.on_change(move |text| bound.set(text.to_owned())).on(
				NativeEventSlot::Submit,
				move |_| {
					search.run();
					EventOutcome::Handled
				},
			);
		}

		View()
			.class("flex-col gap-1.5")
			.class(width)
			.child(Text(label.to_owned()).class("text-sm font-semibold text-[var(--admin-ink-2)]"))
			.child(input)
	};
	let mut apply = Button("Filter")
		.class(Kind::Secondary.class_name())
		.test_id("people-apply");

	if let Some(search) = search {
		apply = apply.on_press(move || search.run());
	}

	let filters = View()
		.class("flex-row flex-wrap items-end gap-3")
		.test_id("people-filters")
		.child(field(
			"Search",
			"people-search",
			&people.search,
			search.map(|search| search.text),
			"an email, an id, a name…",
			"min-w-0 flex-1",
		))
		.child(field(
			"Property",
			"people-property",
			&people.property,
			search.map(|search| search.property),
			"plan",
			"w-36",
		))
		.child(field(
			"equals",
			"people-value",
			&people.value,
			search.map(|search| search.value),
			"pro",
			"w-36",
		))
		.child(apply);
	let pages = View()
		.class("flex-row gap-3")
		.children(
			people
				.previous
				.as_ref()
				.map(|href| components::link_button(Kind::Secondary, "Previous", href.clone())),
		)
		.children(
			people
				.next
				.as_ref()
				.map(|href| components::link_button(Kind::Secondary, "Next", href.clone())),
		);
	let mut card = components::card("People")
		.aside(if people.filtered {
			"filtered"
		} else {
			"newest first"
		})
		.child(filters);

	if people.merge_note {
		card = card.child(components::margin_note(
			"person-merge",
			"One person can have many ids: the anonymous one from before they signed in, then \
			 their account's. Identify merges them, so their history stays whole.",
		));
	}

	View()
		.class("flex-col gap-5")
		.child(components::header(
			"People",
			"everyone your events came from, newest first",
			Vec::new(),
		))
		.child(card.child(list).child(pages).build())
}

/// What a person's page can do: export them, and delete them.
#[derive(Clone, Copy)]
pub(crate) struct Actions {
	id: StoredValue<String>,
	name: StoredValue<String>,
	export: FocusRequest,
	delete: FocusRequest,
	confirm: Confirm,
	notice: RwSignal<String>,
}

impl Actions {
	pub(crate) fn new(person: &Person, confirm: Confirm, notice: RwSignal<String>) -> Self {
		Self {
			id: StoredValue::new(person.id.clone()),
			name: StoredValue::new(person.confirmation().to_owned()),
			export: FocusRequest::new(),
			delete: FocusRequest::new(),
			confirm,
			notice,
		}
	}

	/// Asks, then downloads everything stored about them.
	fn export(self) {
		let id = self.id.get_value();
		let ask = Ask::new(
			"Export",
			"Downloads everything stored about this person as JSON: their properties, ids, \
			 events, and sessions. Handle it as personal data.",
		)
		.download(
			format!("{BASE}/api/analytics/people/{id}/export"),
			format!("person-{id}.json"),
		);
		self.confirm.ask(ask, Some(self.export), |_| ());
	}

	/// Asks for their name, then deletes them and goes back to the list.
	fn delete(self) {
		let name = self.name.get_value();
		let ask = Ask::new(
			"Delete",
			format!(
				"Deletes this person, every id merged into them, and all their events and \
				 sessions, for good. Type `{name}` to confirm."
			),
		)
		.typed("Type their name to confirm", name)
		.danger();
		self.confirm.ask(ask, Some(self.delete), move |typed| {
			self.send_delete(&typed);
		});
	}

	fn send_delete(self, typed: &str) {
		let path = format!("{BASE}/api/analytics/people/{}/delete", self.id.get_value());
		let body = serde_json::json!({ "confirm": typed }).to_string();

		crate::spawn(async move {
			let answer = crate::post_json(&path, &body).await;
			let href = answer
				.as_ref()
				.filter(|(ok, _)| *ok)
				.and_then(|(_, data)| data["href"].as_str().map(str::to_owned));

			if let Some(href) = href {
				crate::go(&href);
				return;
			}

			self.notice.set(
				answer
					.and_then(|(_, data)| data["message"].as_str().map(str::to_owned))
					.unwrap_or_else(|| {
						"The admin couldn't delete them. Nothing was deleted.".to_owned()
					}),
			);
		});
	}
}

/// An event's name, with the `struct` badge when a Rust type sends it.
fn name_row(name: &str, typed: bool) -> Element {
	View()
		.class("flex-row items-center gap-1.5")
		.child(
			Text(name.to_owned())
				.class(style::CODE)
				.class("font-semibold"),
		)
		.children(typed.then(components::struct_badge))
}

fn timeline(person: &Person) -> Element {
	let mut timeline = View().class("flex-col").test_id("person-timeline");

	match &person.events {
		Ok(events) => {
			for event in events {
				timeline = timeline.child(
					View()
						.class(
							"flex-row items-start gap-3 border-b border-[var(--admin-line)] py-2",
						)
						.test_id("timeline-event")
						.child(
							Text(event.when.clone()).class(
								"w-32 shrink-0 text-xs tabular-nums text-[var(--admin-ink-3)]",
							),
						)
						.child(
							View()
								.class("min-w-0 flex-1 flex-col")
								.child(name_row(&event.name, event.typed))
								.child(
									Text(event.detail.clone())
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
			timeline = timeline.child(components::status_word(Tone::Bad, failure.clone()));
		}
	}

	timeline
}

fn sessions(person: &Person) -> Element {
	let mut list = View().class("flex-col").test_id("sessions");

	match &person.sessions {
		Ok(sessions) => {
			for session in sessions {
				list = list.child(
					View()
						.class("flex-col gap-0.5 border-b border-[var(--admin-line)] py-2")
						.child(
							View()
								.class("flex-row items-center justify-between gap-2")
								.child(
									Text(session.when.clone())
										.class("text-sm font-semibold text-[var(--admin-ink)]"),
								)
								.child(
									Text(session.length.clone())
										.class("text-xs tabular-nums text-[var(--admin-ink-2)]"),
								),
						)
						.child(
							Text(session.line.clone()).class("text-xs text-[var(--admin-ink-3)]"),
						),
				);
			}

			if sessions.is_empty() {
				list = list.child(Text("No sessions.").class(style::MUTED));
			}
		}
		Err(failure) => {
			list = list.child(components::status_word(Tone::Bad, failure.clone()));
		}
	}

	list
}

/// One person.
pub(crate) fn person(person: &Person, actions: Option<Actions>) -> Element {
	let mut properties = View()
		.class("flex-col")
		.test_id("person-properties")
		.children(person.properties.iter().map(|(key, value)| {
			View()
				.class("flex-row gap-3 border-b border-[var(--admin-line)] py-1.5")
				.child(
					Text(key.clone())
						.class("w-32 shrink-0 truncate")
						.class(style::CODE),
				)
				.child(
					Text(value.clone())
						.class("min-w-0 flex-1 truncate text-sm text-[var(--admin-ink)]"),
				)
		}));

	if person.properties.is_empty() {
		properties = properties.child(Text("No properties set.").class(style::MUTED));
	}

	let ids = View()
		.class("flex-col gap-1.5")
		.test_id("distinct-ids")
		.children(
			person
				.distinct_ids
				.iter()
				.map(|distinct| Text(distinct.clone()).class(style::CODE).class("truncate")),
		);
	let mut export = Button("Export")
		.class(Kind::Secondary.class_name())
		.aria_description("Downloads everything stored about this person as JSON.")
		.test_id("person-export");
	let mut delete = Button("Delete")
		.class(Kind::Danger.class_name())
		.aria_description("Deletes this person for good, once you type their name.")
		.test_id("person-delete");

	if let Some(actions) = actions {
		export = export
			.focus_on(actions.export)
			.on_press(move || actions.export());
		delete = delete
			.focus_on(actions.delete)
			.on_press(move || actions.delete());
	}

	let profile = View()
		.class("flex-row flex-wrap items-center gap-4")
		.child(components::avatar(
			&person.name,
			"h-14 w-14 text-lg leading-[56px]",
		))
		.child(
			View()
				.class("min-w-[14rem] flex-1 flex-col gap-1")
				.child(
					Heading(1, person.name.clone())
						.class(style::PAGE_TITLE)
						.class("truncate"),
				)
				.child(Text(person.summary.clone()).class(style::SECONDARY)),
		)
		.child(View().class("flex-row gap-2").child(export).child(delete));
	let mut ids_card = components::card("Distinct ids")
		.aside("merged into one person")
		.workings("person", "persons ⨝ person_distinct_ids")
		.child(ids);

	if person.ids_note {
		ids_card = ids_card.child(components::margin_note(
			"person-ids",
			"These ids were one person all along: identify joined the anonymous one to the \
			 account when they signed in.",
		));
	}

	View()
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
								.child(timeline(person))
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
								.child(sessions(person))
								.build(),
						),
				),
		)
}
