//! People: everyone the events came from, and one person whole.

use serde::Deserialize;
use serde::Serialize;

use crate::workings::Drawer;

/// The people list, with what it was searched and filtered by.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct People {
	/// The search, the property, and the value, as they were typed.
	pub search: String,
	pub property: String,
	pub value: String,
	/// Whether a search or a filter narrowed the list.
	pub filtered: bool,
	/// This page of people, newest first.
	pub rows: Vec<PersonRow>,
	/// The page before this one, and the page after.
	pub previous: Option<String>,
	pub next: Option<String>,
	/// Whether the margin note about merged ids shows.
	pub merge_note: bool,
}

/// A person as a row in the list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonRow {
	/// Their name, as the admin shows it: an email, a name, or an id.
	pub name: String,
	pub identified: bool,
	/// Their `plan` property, when they have one.
	pub plan: String,
	/// `3 days ago`.
	pub since: String,
	/// Their page.
	pub href: String,
}

/// One person: their properties, ids, events and sessions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Person {
	pub id: String,
	pub name: String,
	/// `identified · first seen Oct 3 · 2 ids`.
	pub summary: String,
	/// Each property and its value, as text.
	pub properties: Vec<(String, String)>,
	/// Every id merged into them.
	pub distinct_ids: Vec<String>,
	/// What they did, newest first, or why the admin couldn't read it.
	pub events: Result<Vec<Event>, String>,
	/// Their latest sessions, or why the admin couldn't read them.
	pub sessions: Result<Vec<Session>, String>,
	/// Whether the margin note about their merged ids shows.
	pub ids_note: bool,
	/// The Workings of their ids: the store and the operations.
	pub drawer: Drawer,
}

impl Person {
	/// What confirms deleting them: their name as the page shows it.
	#[must_use]
	pub fn confirmation(&self) -> &str {
		&self.name
	}

	/// Where their export downloads from.
	#[must_use]
	pub fn export_href(&self) -> String {
		format!("{}/api/analytics/people/{}/export", crate::BASE, self.id)
	}
}

/// One event on a person's timeline.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
	/// `Oct 3 14:05`.
	pub when: String,
	pub name: String,
	/// Whether a Rust type sends it.
	pub typed: bool,
	/// Its page, or its first few properties.
	pub detail: String,
	/// The id it came in under.
	pub distinct_id: String,
}

/// One of a person's sessions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
	/// `3 days ago`.
	pub when: String,
	/// `4m 10s`.
	pub length: String,
	/// `/notes · 3 pageviews · Direct · Chrome on macOS`.
	pub line: String,
}
