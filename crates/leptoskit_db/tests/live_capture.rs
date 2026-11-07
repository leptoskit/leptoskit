//! How live capture is chosen per dialect, and the columns a live table's
//! capture copies.

#![cfg(feature = "server")]

use leptoskit_db::Dialect;
use leptoskit_db::Id;
use leptoskit_db::LiveCapture;
use leptoskit_db::Table;
use leptoskit_db::table;
use leptoskit_db_schema::Schema;

/// A write everyone hears about, with a column its capture never copies.
#[table(name = "live_notes", live)]
pub struct LiveNote {
	pub id: Id<Self>,
	pub body: String,
	#[db(live = skip)]
	pub secret: String,
}

#[test]
fn a_live_table_captures_every_column_but_the_skipped_ones() {
	let live = LiveNote::LIVE;
	assert!(live);
	assert_eq!(LiveNote::LIVE_COLUMNS, ["id", "body"]);

	let schema = leptoskit_db::schema(Dialect::Postgres).unwrap();
	let note = schema
		.tables
		.iter()
		.find(|table| table.name.name == "live_notes")
		.unwrap();
	assert_eq!(
		note.columns
			.iter()
			.map(|column| column.name.as_str())
			.collect::<Vec<_>>(),
		["id", "body", "secret"]
	);
}

#[test]
fn replication_capture_is_refused_on_sqlite() {
	let schema = leptoskit_db::schema(Dialect::Sqlite).unwrap();
	let error = leptoskit_db::with_live_capture(schema, LiveCapture::Replication).unwrap_err();
	assert!(
		error
			.to_string()
			.contains("replication capture needs Postgres"),
		"{error}"
	);

	// A schema with no live tables has nothing to capture, so it keeps its
	// triggers (or lack of them) whatever the setting says.
	let empty = Schema::empty(Dialect::Sqlite);
	let kept = leptoskit_db::with_live_capture(empty, LiveCapture::Replication).unwrap();
	assert_eq!(kept.triggers.len(), 0);
}
