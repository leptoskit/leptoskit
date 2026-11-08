//! Online compatibility is stricter than a data-preserving schema diff.

use leptoskit_db_schema::*;

fn baseline() -> Schema {
	Schema::new(
		Dialect::Postgres,
		vec![Table::new(
			TableName::new("notes"),
			vec![
				Column::new("id", ColumnType::Int64),
				Column::new("body", ColumnType::TEXT),
			],
			vec!["id".into()],
		)],
	)
}

fn sql(before: &Schema, after: &Schema) -> String {
	let plan = diff(before, after, &mut RequireHints).unwrap();
	render(&plan, before, after).sql()
}

#[test]
fn nullable_expansion_is_exact_and_does_not_rewrite_its_sql() {
	let before = baseline();
	let mut after = before.clone();
	after.tables[0]
		.columns
		.push(Column::new("extra", ColumnType::TEXT).nullable());
	let generated = sql(&before, &after);
	verify_expansion(&before, &after, &generated).unwrap();

	for suffix in ["\n", "\nSELECT 1;", "\nDROP TABLE notes;"] {
		assert!(matches!(
			verify_expansion(&before, &after, &format!("{generated}{suffix}")),
			Err(ExpansionError::EditedSql)
		));
	}
}

#[test]
fn new_table_and_comments_are_supported() {
	let before = baseline();
	let mut after = before.clone();
	let mut table = Table::new(
		TableName::new("labels"),
		vec![Column::new("id", ColumnType::Int64)],
		vec!["id".into()],
	);
	table.comment = Some("New table".into());
	after.tables.push(table);
	after.tables[0].comment = Some("Existing notes".into());
	verify_expansion(&before, &after, &sql(&before, &after)).unwrap();
}

#[test]
fn data_preserving_defaults_constraints_and_indexes_are_not_online_evidence() {
	let before = baseline();
	let mut default = before.clone();
	default.tables[0].columns.push(
		Column::new("extra", ColumnType::TEXT)
			.nullable()
			.default(DefaultValue::Text("x".into())),
	);
	let mut index = before.clone();
	index.tables[0]
		.indexes
		.push(Index::unique(&TableName::new("notes"), &["body"]));
	let mut changed = before.clone();
	changed.tables[0].columns[1].nullable = true;
	let mut dropped = before.clone();

	dropped.tables[0].columns.pop();

	for after in [default, index, changed, dropped] {
		assert!(matches!(
			verify_expansion(&before, &after, &sql(&before, &after)),
			Err(ExpansionError::Unsupported(_))
		));
	}
}

#[test]
fn pack_membership_cannot_be_removed_or_downgraded_to_unknown() {
	let before = baseline().with_packs(vec![Pack::new("notes", vec![TableName::new("notes")])]);
	let after = baseline();
	assert!(verify_expansion(&before, &after, &sql(&before, &after)).is_err());
	let mut legacy = before.clone();
	legacy.format = 1;
	legacy.packs = None;
	assert!(verify_expansion(&legacy, &before, "").is_err());
	let mut future = before.clone();

	future.format += 1;
	assert!(verify_expansion(&before, &future, "").is_err());
}

#[test]
fn sqlite_is_not_implicitly_certified_by_postgres_rules() {
	let mut before = baseline();
	before.dialect = Dialect::Sqlite;
	assert!(matches!(
		verify_expansion(&before, &before, &sql(&before, &before)),
		Err(ExpansionError::Unsupported(_))
	));
}
