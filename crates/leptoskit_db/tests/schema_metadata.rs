//! Table metadata that only the declaration paths reach: sequence columns,
//! and the schema they build.

#![cfg(feature = "server")]

use leptoskit_db::Dialect;
use leptoskit_db::Id;
use leptoskit_db::Table;
use leptoskit_db::table;
use leptoskit_db_schema::ColumnType;

/// Rows numbered by the database as they arrive.
#[table(name = "counters")]
pub struct Counter {
	pub id: Id<Self>,
	#[db(sequence)]
	pub position: i64,
}

#[test]
fn a_sequence_column_is_a_column_type_of_its_own() {
	let definition = Counter::definition();
	let position = definition
		.columns
		.iter()
		.find(|column| column.name == "position")
		.unwrap();

	assert_eq!(position.ty, ColumnType::Sequence);
	assert_eq!(Counter::COLUMNS, ["id", "position"]);

	let schema = leptoskit_db::schema(Dialect::Postgres).unwrap();
	let counter = schema
		.tables
		.iter()
		.find(|table| table.name.name == "counters")
		.unwrap();
	assert_eq!(
		counter
			.columns
			.iter()
			.find(|column| column.name == "position")
			.unwrap()
			.ty,
		ColumnType::Sequence
	);
}

#[test]
#[should_panic(expected = "`sequence` marks an i64 column")]
fn a_sequence_column_must_be_an_i64() {
	use leptoskit_db::__private::TableBuilder;

	let mut builder = TableBuilder::new(None, "counters", None, None);
	builder.column::<i32>("position", None, None, None);
	builder.sequence("position");
}
