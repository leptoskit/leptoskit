//! The library: saved insights and dashboards, one list with two tabs.

use serde::Deserialize;
use serde::Serialize;

/// The library, on one of its tabs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Library {
	/// Whether the dashboards tab is the one showing.
	pub dashboards_tab: bool,
	/// How many insights are saved.
	pub insights: usize,
	/// How many dashboards there are.
	pub dashboards: usize,
	/// The showing tab's rows.
	pub items: Vec<Item>,
}

/// A saved insight or a dashboard, as a row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
	pub title: String,
	/// `Trends · by ada · updated 5 minutes ago`.
	pub detail: String,
	/// Its page.
	pub href: String,
}
