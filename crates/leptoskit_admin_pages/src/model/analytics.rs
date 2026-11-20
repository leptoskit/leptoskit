//! The analytics pages that aren't a builder: the web overview, a
//! dashboard, the people behind one point of a chart, and what a page says
//! when it has nothing to read.

use leptoskit_analytics::query::Query;
use leptoskit_analytics::query::QueryResult;
use leptoskit_analytics::query::WebOverviewResult;
use serde::Deserialize;
use serde::Serialize;

use super::Section;
use crate::analytics::params::Params;
use crate::workings::Drawer;

/// A dashboard: its saved insights, each run and drawn small.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Dashboard {
	pub name: String,
	pub cards: Vec<Card>,
}

/// One dashboard card.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Card {
	/// The saved insight's name.
	pub name: String,
	/// `Trends`, `Funnel`, …
	pub kind: String,
	/// The saved insight's page.
	pub href: String,
	pub query: Query,
	/// What the service answered, or why it didn't.
	pub answer: Result<QueryResult, String>,
	/// Whether it spans both columns.
	pub wide: bool,
}

/// The people behind one point of a chart.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Actors {
	/// `People behind Mar 3 · pro`.
	pub title: String,
	/// The chart the point is on.
	pub back: String,
	pub section: Section,
	/// The people on this page, or why there are none.
	pub people: Result<Vec<Actor>, String>,
	/// The next page's address, when there are more.
	pub next: Option<String>,
	/// The actors query's Workings.
	pub drawer: Drawer,
}

/// One person behind a point.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Actor {
	pub name: String,
	/// `identified · 2 ids`.
	pub summary: String,
	pub href: String,
}

/// What an analytics page says instead of its data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notice {
	/// The page's title.
	pub title: String,
	pub section: Section,
	pub why: Why,
}

/// Why a page shows a [`Notice`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Why {
	/// The admin has no analytics service to read.
	NotConfigured,
	/// The service didn't answer, and what went wrong.
	Unreachable(String),
	/// The service has no events yet; the page listens for the first.
	/// `shows` says what the page will show.
	Listening { shows: String },
}

/// The web overview: the service's figures and tables for the site, with
/// the daily series the tiles and chart draw.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Web {
	/// What the address asked for, as it came.
	pub address: Params,
	/// The `WebOverview` query the page ran.
	pub query: Query,
	pub result: WebOverviewResult,
	/// The daily visitors' trend, and what it answered.
	pub visitors_query: Query,
	pub visitors: Result<QueryResult, String>,
	/// Daily pageviews and sessions, for the tiles' sparklines.
	pub pageviews: Option<Vec<f64>>,
	pub sessions: Option<Vec<f64>>,
	/// Whether the note about channels still shows.
	pub channels_note: bool,
	/// The figures' and the chart's Workings.
	pub drawers: Vec<Drawer>,
}
