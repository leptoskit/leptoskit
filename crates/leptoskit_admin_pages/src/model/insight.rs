//! A builder page: trends, funnels or retention, new or saved, with what
//! its query answered.

use leptoskit_analytics::query::Query;
use leptoskit_analytics::query::QueryResult;
use serde::Deserialize;
use serde::Serialize;

use crate::analytics::Definition;
use crate::analytics::insights::Kind;
use crate::analytics::params;
use crate::analytics::params::Params;
use crate::workings::Drawer;

/// A builder, as the server ran it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Insight {
	pub kind: Kind,
	/// The page's address without its query: the builder's, or a saved
	/// insight's.
	pub path: String,
	/// What the address asked for, as it came. Save and Add to dashboard
	/// send it back, so an unchanged saved insight stays itself.
	pub address: Params,
	/// The query the page ran.
	pub query: Query,
	/// The saved insight it shows, when it is one.
	pub saved: Option<Saved>,
	pub name: String,
	/// `Trends · saved by ada@example.com · updates live`.
	pub subtitle: String,
	/// What the service answered, or why it didn't.
	pub answer: Result<QueryResult, String>,
	/// The events the builder offers.
	pub definitions: Vec<Definition>,
	/// The margin notes that still show.
	pub notes: Vec<String>,
	/// What the chart's Workings tab says it reads: `NoteCreated → Trends`.
	pub source: String,
	/// The chart's Workings.
	pub drawer: Drawer,
}

/// A saved insight.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Saved {
	pub id: String,
	/// Who saved it.
	pub created_by: String,
}

impl Insight {
	/// The address of what the page shows, with its whole query.
	#[must_use]
	pub fn here(&self) -> String {
		params::href(&self.path, &params::of(&self.query))
	}
}
