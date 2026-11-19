//! The Analytics area: the main site's analytics service, read through its
//! app-only operations.
//!
//! Every page reads real data. When the admin isn't connected to the
//! service, each page says how to connect it; when the service has nothing
//! yet, it shows the code that sends the first event and listens for it.

pub(crate) mod client;
pub(crate) mod code;
pub(crate) mod format;
pub(crate) mod insights;
pub(crate) mod library;
pub(crate) mod live;
pub(crate) mod people;
pub(crate) mod routes;
pub(crate) mod stations;
pub(crate) mod tail;
pub(crate) mod web;

pub(crate) use client::Connection;
pub(crate) use client::Facts;
pub(crate) use client::Failure;
use leptoskit::prelude::*;
pub(crate) use leptoskit_admin_pages::analytics::Stat;
pub(crate) use leptoskit_admin_pages::analytics::chip;
pub(crate) use leptoskit_admin_pages::analytics::draw;
pub(crate) use leptoskit_admin_pages::analytics::params;
pub(crate) use leptoskit_admin_pages::analytics::stat;
use params::Params;

use crate::BASE;
use crate::access::Preferences;
use crate::components;
use crate::components::Tone;
use crate::shell::Screen;
use crate::shell::Section;
use crate::style;

/// What an analytics page shows: a screen the admin draws, or a page the
/// pages crate draws and the browser picks up.
pub(crate) enum Shown {
	Screen(Box<Screen>),
	Page(Box<leptoskit_admin_pages::model::Page>),
}

impl From<Screen> for Shown {
	fn from(screen: Screen) -> Self {
		Self::Screen(Box::new(screen))
	}
}

impl From<leptoskit_admin_pages::model::Page> for Shown {
	fn from(page: leptoskit_admin_pages::model::Page) -> Self {
		Self::Page(Box::new(page))
	}
}

/// What every analytics page reads from.
pub(crate) struct Context<'a> {
	pub connection: &'a Connection,
	pub params: &'a Params,
	pub preferences: &'a Preferences,
	pub facts: &'a Facts,
}

/// The Rust that sends a typed event, from the analytics guide.
const FIRST_EVENT: &str = "use leptoskit_analytics::Event;
use leptoskit_analytics::client::web;

#[derive(Event)]
struct SignedUp {
    plan: String,
    seats: u32,
}

web::init(web::Options::default());
web::capture_event(SignedUp {
    plan: \"pro\".into(),
    seats: 3,
});";

/// How the admin connects to the service.
const CONNECT: &str = r"# Wherever the admin runs: the main site's analytics service,
# at an address the admin can reach, and the token its server uses.
export LEPTOSKIT_SERVICE_ANALYTICS_URL=http://127.0.0.1:4800
export LEPTOSKIT_SERVICE_ANALYTICS_TOKEN=…   # the same token as the site's server
lk admin serve --main-site https://notes.example";

/// The page heading: title, a line under it, and actions on the right.
pub(crate) fn header(title: &str, subtitle: &str, actions: Vec<Element>) -> Element {
	components::header(title, subtitle, actions)
}

/// A note in the margin, if the person hasn't retired it.
pub(crate) fn note(preferences: &Preferences, id: &str, text: &str) -> Option<Element> {
	preferences
		.note(id)
		.then(|| components::margin_note(id, text.to_owned()))
}

/// The screen for when the admin has no analytics service to read.
pub(crate) fn not_configured(title: &str, section: Section) -> Screen {
	let content = View()
		.class("flex-col gap-5")
		.test_id("analytics-not-configured")
		.child(Heading(1, title.to_owned()).class(style::PAGE_TITLE))
		.child(components::empty_state(
			"Connect the admin to your analytics",
			"The admin is its own app, so it reads your site's analytics the way your site's \
			 server does: from the analytics service, signed with the service's token. It hasn't \
			 been given either yet. Set both where the admin runs, then reload:",
			Some((CONNECT, "shell")),
			"The token is the app's: with it the admin can read every event, so keep it in the \
			 admin's environment, never in a page. AdminBuilder::analytics does the same from \
			 Rust.",
		))
		.child(
			components::card("Then, from your app")
				.aside("once the service runs")
				.child(
					Text(
						"Your app sends events with a Rust type for each, so a misspelled \
						 property fails when it compiles rather than in a chart:",
					)
					.class(style::SECONDARY),
				)
				.child(components::paper_code(FIRST_EVENT, "Rust"))
				.build(),
		);

	Screen {
		title: title.to_owned(),
		section,
		content,
		drawers: Vec::new(),
		open: None,
	}
}

/// The screen for when the service didn't answer.
pub(crate) fn unreachable(title: &str, section: Section, failure: &Failure) -> Screen {
	let content = View()
		.class("flex-col gap-5")
		.test_id("analytics-unreachable")
		.child(Heading(1, title.to_owned()).class(style::PAGE_TITLE))
		.child(
			components::card("The analytics service didn't answer")
				.child(components::status_word(Tone::Bad, "Not connected"))
				.child(Text(failure.0.clone()).class(style::BODY))
				.child(
					Text(
						"Nothing on this page is guessed, so it shows nothing until the service \
						 answers. Reload once it's back.",
					)
					.class(style::MUTED),
				)
				.build(),
		);

	Screen {
		title: title.to_owned(),
		section,
		content,
		drawers: Vec::new(),
		open: None,
	}
}

/// The listening empty state: no events yet, the code that sends one, and
/// a checklist the page ticks when the first one arrives.
pub(crate) fn listening(title: &str, section: Section, shows: &str) -> Screen {
	let check = |done: bool, text: &str| {
		View()
			.class("flex-row items-center gap-2")
			.child(if done {
				components::status_word(Tone::Good, "")
			} else {
				View()
					.class("h-2.5 w-2.5 rounded-full bg-[var(--admin-mint)] animate-pulse")
					.aria_hidden(true)
			})
			.child(Text(text.to_owned()).class(style::BODY))
	};
	let content = View()
		.class("flex-col gap-5")
		.test_id("live-refresh")
		.child(Heading(1, title.to_owned()).class(style::PAGE_TITLE))
		.child(components::empty_state(
			"Waiting for your first event",
			&format!(
				"{shows} Your app sends events with a Rust type for each, so a misspelled \
				 property fails when it compiles rather than in a chart. Add leptoskit_analytics \
				 with the web and derive features, then send one:"
			),
			Some((FIRST_EVENT, "Rust")),
			"This page listens: it draws in by itself when the first event arrives.",
		))
		.child(
			components::card("Listening")
				.test_id("checklist")
				.child(check(true, "The admin reaches your analytics service"))
				.child(check(false, "Capture an event … waiting"))
				.build(),
		);

	Screen {
		title: title.to_owned(),
		section,
		content,
		drawers: Vec::new(),
		open: None,
	}
}

/// The event catalogue, as the builders offer it.
pub(crate) fn definitions(facts: &Facts) -> Vec<leptoskit_admin_pages::analytics::Definition> {
	facts
		.definitions
		.iter()
		.map(|definition| {
			leptoskit_admin_pages::analytics::Definition {
				name: definition.name.clone(),
				volume: u64::try_from(definition.volume).unwrap_or(0),
				fields: definition.fields.clone(),
				properties: definition.properties.clone(),
			}
		})
		.collect()
}

/// The builders' margin notes the person hasn't retired.
pub(crate) fn notes(preferences: &Preferences) -> Vec<String> {
	["breakdown", "funnel-drop-off", "retention-cells"]
		.into_iter()
		.filter(|id| preferences.note(id))
		.map(str::to_owned)
		.collect()
}

/// Where a person's page is.
pub(crate) fn person_href(id: &str) -> String {
	format!("{BASE}/analytics/people/{id}")
}

/// Whether the service has seen any event at all.
pub(crate) fn has_events(facts: &Facts) -> bool {
	facts
		.definitions
		.iter()
		.any(|definition| definition.volume > 0)
}
