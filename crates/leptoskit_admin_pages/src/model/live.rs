//! Live: events as they arrive, a few seconds after they happen.

use serde::Deserialize;
use serde::Serialize;

use crate::workings::Drawer;

/// The Live page: the latest events when it was read, then each new one
/// from the admin's stream.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Live {
	/// The latest events, newest first.
	pub events: Vec<Arrival>,
	/// How the page reads them, for its Workings drawer.
	pub drawer: Drawer,
	/// Whether the note about Pause still shows.
	pub pause_note: bool,
}

/// One event, as the Live page lists it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Arrival {
	/// Its place in the service's log; zero for the events the page started
	/// with.
	#[serde(default)]
	pub seq: u64,
	pub name: String,
	pub distinct_id: String,
	/// When it happened, as the service wrote it.
	pub timestamp: String,
	pub properties: serde_json::Value,
	/// Whether the app declared its fields with `#[derive(Event)]`.
	pub typed: bool,
}
