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
pub(crate) use leptoskit_admin_pages::analytics::params;
use leptoskit_admin_pages::model::Page;
use leptoskit_admin_pages::model::Section;
use leptoskit_admin_pages::model::analytics::Notice;
use leptoskit_admin_pages::model::analytics::Why;
use params::Params;

use crate::BASE;
use crate::access::Preferences;

/// What an analytics page shows: a page the pages crate draws and the
/// browser picks up.
pub(crate) struct Shown(Box<Page>);

impl From<Page> for Shown {
	fn from(page: Page) -> Self {
		Self(Box::new(page))
	}
}

/// What every analytics page reads from.
pub(crate) struct Context<'a> {
	pub connection: &'a Connection,
	pub params: &'a Params,
	pub preferences: &'a Preferences,
	pub facts: &'a Facts,
}

/// The page for when the admin has no analytics service to read.
pub(crate) fn not_configured(title: &str, section: Section) -> Page {
	notice(title, section, Why::NotConfigured)
}

/// The page for when the service didn't answer.
pub(crate) fn unreachable(title: &str, section: Section, failure: &Failure) -> Page {
	notice(title, section, Why::Unreachable(failure.0.clone()))
}

/// The listening empty state: no events yet, the code that sends one, and
/// a checklist; the page reads itself again when the first one arrives.
pub(crate) fn listening(title: &str, section: Section, shows: &str) -> Page {
	notice(
		title,
		section,
		Why::Listening {
			shows: shows.to_owned(),
		},
	)
}

fn notice(title: &str, section: Section, why: Why) -> Page {
	Page::Notice(Box::new(Notice {
		title: title.to_owned(),
		section,
		why,
	}))
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
