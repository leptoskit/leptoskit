//! The Analytics area, before its pages land: every screen says what it
//! will show, and the Rust that sends the first event. No numbers are made
//! up.

use leptoskit::prelude::*;

use crate::components;
use crate::shell::Screen;
use crate::shell::Section;
use crate::style;

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

/// One Analytics screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Page {
	Overview,
	Live,
	Trends,
	Funnels,
	Retention,
	Library,
	People,
}

impl Page {
	pub(crate) fn parse(slug: &str, kind: Option<&str>) -> Option<Self> {
		Some(match (slug, kind) {
			("overview", _) => Self::Overview,
			("live", _) => Self::Live,
			("insights", Some("funnels")) => Self::Funnels,
			("insights", Some("retention")) => Self::Retention,
			("insights", _) => Self::Trends,
			("library", _) => Self::Library,
			("people", _) => Self::People,
			_ => return None,
		})
	}

	fn title(self) -> &'static str {
		match self {
			Self::Overview => "Overview",
			Self::Live => "Live",
			Self::Trends => "Trends",
			Self::Funnels => "Funnels",
			Self::Retention => "Retention",
			Self::Library => "Library",
			Self::People => "People",
		}
	}

	fn section(self) -> Section {
		match self {
			Self::Overview => Section::Overview,
			Self::Live => Section::Live,
			Self::Trends => Section::Trends,
			Self::Funnels => Section::Funnels,
			Self::Retention => Section::Retention,
			Self::Library => Section::Library,
			Self::People => Section::People,
		}
	}

	/// What the screen shows, once there's data.
	fn shows(self) -> &'static str {
		match self {
			Self::Overview => {
				"Visitors, pageviews, sessions, and bounce rate for your site, with the top pages, \
				 referrers, and devices beside them."
			}
			Self::Live => "Events as they arrive, a few seconds after they happen.",
			Self::Trends => {
				"How often an event happens over time, split by any property you send with it, \
				 such as plan."
			}
			Self::Funnels => {
				"How many people go from one event to the next, such as SignedUp to \
				 CheckoutStarted, and where the rest stop."
			}
			Self::Retention => {
				"Of the people who did something one week, how many came back the next."
			}
			Self::Library => "The trends, funnels, and dashboards you've saved, to come back to.",
			Self::People => "Each person your events came from, and everything they did, in order.",
		}
	}
}

/// An Analytics screen: a teaching empty state, honestly.
pub(crate) fn screen(page: Page) -> Screen {
	let content = View()
		.class("flex-col gap-5")
		.child(Heading(1, page.title()).class(style::PAGE_TITLE))
		.child(components::empty_state(
			"Waiting for your first event",
			&format!(
				"{} Your app sends events with a Rust type for each, so a misspelled property \
				 fails when it compiles rather than in a chart. Add leptoskit_analytics with the \
				 web and derive features, then send one:",
				page.shows()
			),
			Some((FIRST_EVENT, "Rust")),
			"This page lands with the analytics service; until then the admin shows nothing \
			 rather than guessing. The guide has the rest: Product analytics.",
		))
		.child(
			components::card("While you wait")
				.child(
					Text(
						"Infrastructure already works: plan a change, deploy a release, and read \
						 your servers' health.",
					)
					.class(style::SECONDARY),
				)
				.child(components::link(
					"Go to Infrastructure",
					format!("{}/infra/projects", crate::BASE),
				))
				.build(),
		);

	Screen {
		title: page.title().to_owned(),
		section: page.section(),
		content,
		drawers: Vec::new(),
		open: None,
	}
}
