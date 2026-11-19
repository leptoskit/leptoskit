//! The Analytics area: the main site's analytics service, read through its
//! app-only operations.
//!
//! Every page reads real data. When the admin isn't connected to the
//! service, each page says how to connect it; when the service has nothing
//! yet, it shows the code that sends the first event and listens for it.

pub(crate) mod client;
pub(crate) mod code;
pub(crate) mod draw;
pub(crate) mod format;
pub(crate) mod insights;
pub(crate) mod library;
pub(crate) mod live;
pub(crate) mod params;
pub(crate) mod people;
pub(crate) mod routes;
pub(crate) mod stations;
pub(crate) mod tail;
pub(crate) mod web;

pub(crate) use client::Connection;
pub(crate) use client::Facts;
pub(crate) use client::Failure;
use leptoskit::prelude::*;
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

/// A pill that links somewhere: `Last 30 days`, `Compare previous period`.
pub(crate) fn chip(label: &str, value: &str, href: String, on: bool) -> Element {
	let mut content = View().class(style::CHIP).class("cursor-pointer");

	if !label.is_empty() {
		content = content.child(Text(label.to_owned()).class("text-[var(--admin-ink-2)]"));
	}

	content = content.child(Text(value.to_owned()).class("font-semibold text-[var(--admin-ink)]"));
	let mut link = Pressable(format!("{label} {value}").trim().to_owned())
		.href(href)
		.class("rounded-full hover:bg-[var(--admin-sunken)]")
		.child(content);

	if on {
		link = link.aria_current(leptoskit::native::Current::Page);
	}

	link
}

/// A row of links where one is chosen: `Day | Week`.
pub(crate) fn segmented(label: &str, options: &[(String, String, bool)]) -> Element {
	let mut row = View()
		.class(
			"flex-row self-start overflow-hidden rounded-lg border \
			 border-[var(--admin-line-strong)]",
		)
		.role(leptoskit::native::Role::Group)
		.aria_label(label.to_owned());

	for (name, href, on) in options {
		let mut link = Pressable(name.clone())
			.href(href.clone())
			.class("cursor-pointer px-3 py-1.5")
			.class(if *on {
				"bg-[var(--admin-ink)]"
			} else {
				"hover:bg-[var(--admin-sunken)]"
			})
			.child(Text(name.clone()).class(if *on {
				"text-sm font-semibold text-[var(--admin-paper)]"
			} else {
				"text-sm text-[var(--admin-ink-2)]"
			}));

		if *on {
			link = link.aria_current(leptoskit::native::Current::Page);
		}

		row = row.child(link);
	}

	View()
		.class("flex-col gap-1.5")
		.child(Text(label.to_owned()).class("text-sm font-semibold text-[var(--admin-ink-2)]"))
		.child(row)
}

/// A control that opens a list over the content beneath it: `+ Add
/// series`, `Breakdown`. The script opens one at a time and closes it on a
/// press anywhere else.
pub(crate) fn menu(label: &str, test_id: &str, items: Vec<Element>) -> Element {
	View()
		.class("relative flex-col")
		.child(
			Button(label.to_owned())
				.aria_expanded(false)
				.class(
					"self-start rounded-lg px-1 py-1 text-sm font-semibold \
					 text-[var(--admin-ink)] cursor-pointer hover:bg-[var(--admin-sunken)]",
				)
				.test_id("menu-toggle")
				.aria_description(test_id.to_owned()),
		)
		.child(
			View()
				.class(
					"absolute left-0 top-[100%] mt-1 max-h-72 w-72 flex-col gap-0.5 \
					 overflow-scroll rounded-xl border border-[var(--admin-line)] \
					 bg-[var(--admin-surface)] p-1.5",
				)
				.test_id("menu-panel")
				.children(items),
		)
}

/// One item in a [`menu`].
pub(crate) fn menu_item(label: &str, detail: Option<&str>, href: String) -> Element {
	let mut row = View()
		.class("flex-row items-center gap-2 px-2 py-1.5")
		.child(
			Text(label.to_owned()).class("min-w-0 flex-1 truncate text-sm text-[var(--admin-ink)]"),
		);

	if let Some(detail) = detail {
		row = row.child(Text(detail.to_owned()).class(style::LABEL));
	}

	Pressable(label.to_owned())
		.href(href)
		.class("cursor-pointer rounded-lg hover:bg-[var(--admin-sunken)]")
		.test_id("menu-item")
		.child(row)
}

/// The `struct` badge on an event with a Rust type.
pub(crate) fn struct_badge() -> Element {
	components::struct_badge()
}

/// A stat tile: label, hero number, a sparkline, and the change.
pub(crate) struct Stat<'a> {
	pub label: &'a str,
	pub value: String,
	pub delta: Option<String>,
	/// Whether a rise is good: bounce rate rising isn't.
	pub up_is_good: bool,
	pub spark: Option<Vec<f64>>,
	pub drawer: &'a str,
	pub source: &'a str,
	/// What a change is measured against, without its article: `previous
	/// 30 days`, `period before`.
	pub versus: String,
}

pub(crate) fn stat(stat: Stat<'_>) -> Element {
	let tone = match &stat.delta {
		Some(delta) if delta.starts_with('▲') == stat.up_is_good && !delta.starts_with('=') => {
			"text-[var(--admin-good)]"
		}
		Some(delta) if !delta.starts_with('=') => "text-[var(--admin-bad)]",
		_ => "text-[var(--admin-ink-3)]",
	};
	let mut card = components::card(stat.label)
		.workings(stat.drawer, stat.source)
		.child(
			Text(stat.value)
				.class(style::HERO_NUMBER)
				.test_id("stat-value"),
		);

	card = card.child(match stat.spark {
		Some(values) if values.len() > 1 => draw::sparkline(&values, "h-7 w-32"),
		_ => {
			Text("No daily series yet")
				.class("h-7 truncate text-xs leading-7 text-[var(--admin-ink-3)]")
		}
	});

	card.child(
		Text(match stat.delta {
			Some(delta) => format!("{delta} vs the {}", stat.versus),
			None => format!("Nothing in the {} to compare", stat.versus),
		})
		.class("text-[13px] tabular-nums")
		.class(tone),
	)
	.build()
	.test_id(format!(
		"stat:{}",
		stat.label.to_lowercase().replace(' ', "-")
	))
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
