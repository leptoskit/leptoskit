//! Schema pack names leptoskit reserves: an app can't call a pack `live` or
//! `audit`, whose tables and triggers only leptoskit writes.

#![cfg(feature = "server")]

use leptoskit_db::Dialect;
use leptoskit_db::Id;
use leptoskit_db::table;
use leptoskit_db_schema::SchemaError;

/// A table an app wrongly claims for leptoskit's own live pack.
#[table(name = "reserved_rows", pack = "live")]
pub struct ReservedRow {
	pub id: Id<Self>,
}

#[test]
fn the_builtin_pack_names_are_reserved() {
	let Err(SchemaError::Invalid(problems)) = leptoskit_db::schema(Dialect::Postgres) else {
		panic!("a table in the `live` pack should be refused");
	};
	let message = problems
		.iter()
		.map(ToString::to_string)
		.find(|message| message.contains("reserved"))
		.unwrap_or_else(|| panic!("no reservation problem in {problems:#?}"));

	assert!(
		message.contains("reserved_rows") && message.contains("`live` and `audit`"),
		"{message}"
	);
}
