//! A real generated expansion shared by release evidence acceptance tests.

use leptoskit_db::Migration;
use leptoskit_db::MigrationContract;
use leptoskit_db::MigrationWindow;
use leptoskit_db::checksum;
use leptoskit_db_schema::Column;
use leptoskit_db_schema::ColumnType;
use leptoskit_db_schema::Dialect;
use leptoskit_db_schema::RequireHints;
use leptoskit_db_schema::Schema;
use leptoskit_db_schema::Table;
use leptoskit_db_schema::TableName;
use serde_json::Value;
use serde_json::json;

pub(super) fn evidence() -> (Value, String) {
	let baseline = Schema::new(
		Dialect::Postgres,
		vec![Table::new(
			TableName::new("notes"),
			vec![Column::new("id", ColumnType::Int64)],
			vec!["id".into()],
		)],
	);
	let mut expanded = baseline.clone();
	expanded.tables[0]
		.columns
		.push(Column::new("body", ColumnType::TEXT).nullable());
	let sql = |before: &Schema, after: &Schema| {
		let plan = leptoskit_db_schema::diff(before, after, &mut RequireHints).unwrap();
		leptoskit_db_schema::render(&plan, before, after).sql()
	};
	let migrations = [
		Migration::new("initial", sql(&Schema::empty(Dialect::Postgres), &baseline)),
		Migration::new("expanded", sql(&baseline, &expanded)),
	];
	let window = MigrationWindow::new(
		&migrations[..1],
		&baseline.to_json(),
		[(migrations[1].clone(), expanded.to_json())],
	)
	.unwrap()
	.to_json();
	let contract = MigrationContract::new(&migrations, Some(&expanded.to_json())).unwrap();
	let fingerprint = super::history_fingerprint(
		contract
			.migration_checksums()
			.map(|(name, sha)| (name.into(), sha.into()))
			.collect(),
	)
	.unwrap();

	let contract = contract.to_json();
	(
		json!({"format_version": 1, "window_sha256": checksum(&window), "window": window,
		"contract_sha256": checksum(&contract), "contract": contract}),
		fingerprint,
	)
}
