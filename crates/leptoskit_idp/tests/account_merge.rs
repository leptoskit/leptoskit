//! An app with the provider can merge accounts and bring the other account's
//! data along: every provider table says how its rows move.

#![cfg(feature = "server")]

use leptoskit_auth::Account;
use leptoskit_db::Table;
use leptoskit_db::merge::MergeStrategy;
use leptoskit_idp as _;

fn account_data(dialect: leptoskit_db::Dialect) -> leptoskit_db::merge::AccountData {
	let schema = leptoskit_db::schema(dialect).unwrap();
	leptoskit_db::merge::account_data(&schema, &Account::table_name())
}

#[test]
fn every_provider_table_says_how_its_rows_move() {
	for dialect in [
		leptoskit_db::Dialect::Postgres,
		leptoskit_db::Dialect::Sqlite,
	] {
		let data = account_data(dialect);
		let problems = data
			.problems
			.iter()
			.map(ToString::to_string)
			.collect::<Vec<_>>();
		assert!(problems.is_empty(), "{dialect:?}: {problems:#?}");
		assert!(data.is_covered());
	}
}

#[test]
fn a_grant_for_an_app_the_survivor_already_granted_stays_behind() {
	let data = account_data(leptoskit_db::Dialect::Postgres);
	let grants = data
		.references
		.iter()
		.find(|reference| {
			reference.table.namespace.as_deref() == Some("idp")
				&& reference.table.name == "grants"
				&& reference.column == "account_id"
		})
		.expect("`idp.grants.account_id` points at the account");
	assert_eq!(grants.strategy, MergeStrategy::ReassignSkipConflicts);
}
