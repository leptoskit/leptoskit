use leptoskit_db_schema::Column;
use leptoskit_db_schema::ColumnType;
use leptoskit_db_schema::Dialect;
use leptoskit_db_schema::RequireHints;
use leptoskit_db_schema::Table;
use leptoskit_db_schema::TableName;

use super::*;

pub(super) fn fixture() -> (Vec<Migration>, Vec<Schema>, MigrationWindow) {
	let baseline = Schema::new(
		Dialect::Postgres,
		vec![Table::new(
			TableName::new("notes"),
			vec![Column::new("id", ColumnType::Int64)],
			vec!["id".into()],
		)],
	);
	let mut snapshots = vec![baseline];

	for column in ["body", "extra"] {
		let mut next = snapshots.last().unwrap().clone();
		next.tables[0]
			.columns
			.push(Column::new(column, ColumnType::TEXT).nullable());
		snapshots.push(next);
	}

	let mut previous = Schema::empty(Dialect::Postgres);
	let migrations: Vec<_> = snapshots
		.iter()
		.enumerate()
		.map(|(index, next)| {
			let plan = leptoskit_db_schema::diff(&previous, next, &mut RequireHints).unwrap();
			let sql = leptoskit_db_schema::render(&plan, &previous, next).sql();
			previous = next.clone();
			Migration::new(format!("migration_{index}"), sql)
		})
		.collect();

	let window = MigrationWindow::new(
		&migrations[..1],
		&snapshots[0].to_json(),
		migrations[1..]
			.iter()
			.cloned()
			.zip(snapshots[1..].iter().map(Schema::to_json)),
	)
	.unwrap();
	(migrations, snapshots, window)
}

fn applied(migrations: &[Migration]) -> Vec<(String, String)> {
	migrations
		.iter()
		.map(|m| (m.name.to_string(), m.checksum.to_string()))
		.collect()
}

#[test]
fn every_build_checks_its_own_checkpoint_and_history() {
	let (migrations, schemas, window) = fixture();
	let window = MigrationWindow::from_json(&window.to_json()).unwrap();

	for length in 1..=3 {
		let checked = window
			.checked(&migrations[..length], &schemas[length - 1].to_json())
			.unwrap();

		for installed in 1..=3 {
			let mut history = applied(&migrations[..installed]);
			history.reverse(); // Database row ordering is irrelevant.
			assert_eq!(
				checked.pending(&history, &migrations[..length]).unwrap(),
				&migrations[installed.min(length)..length]
			);
		}
	}

	assert!(
		window
			.check_build(&migrations[..1], &schemas[2].to_json())
			.is_err()
	);
	assert!(
		window
			.check_build(&migrations, &schemas[0].to_json())
			.is_err()
	);
}

#[test]
fn unknown_missing_duplicate_and_modified_history_are_refused() {
	let (migrations, schemas, window) = fixture();
	let checked = window
		.checked(&migrations[..1], &schemas[0].to_json())
		.unwrap();
	let mut wrong_checksum = applied(&migrations);
	wrong_checksum[1].1 = "0".repeat(64);
	let mut outside = applied(&migrations);
	outside.push(("not_in_window".into(), "0".repeat(64)));
	let mut duplicate = applied(&migrations[..1]);
	duplicate.extend(duplicate.clone());

	let missing_middle = vec![
		applied(&migrations)[0].clone(),
		applied(&migrations)[2].clone(),
	];

	for history in [vec![], wrong_checksum, outside, duplicate, missing_middle] {
		assert!(matches!(
			checked.pending(&history, &migrations[..1]),
			Err(MigrationWindowError::HistoryMismatch)
		));
	}
}

#[test]
fn sql_substitution_and_consolidated_history_are_rejected() {
	let (mut migrations, schemas, window) = fixture();
	migrations[0].sql.to_mut().push_str("\nSELECT 1;");
	assert!(
		window
			.check_build(&migrations[..1], &schemas[0].to_json())
			.is_err()
	);
	migrations[0].checksum = checksum(&migrations[0].sql).into();
	assert!(matches!(
		window.check_build(&migrations[..1], &schemas[0].to_json()),
		Err(MigrationWindowError::BuildMismatch)
	));
	migrations[0].replaces.push("previous".into());
	assert!(
		MigrationWindow::new(
			&migrations[..1],
			&schemas[0].to_json(),
			[(migrations[1].clone(), schemas[1].to_json())]
		)
		.is_err()
	);
}

#[test]
fn stored_documents_are_revalidated_without_trusting_flags() {
	let (_, _, window) = fixture();
	let original: serde_json::Value = serde_json::from_str(&window.to_json()).unwrap();
	let mut variants = Vec::new();
	let mut changed = original.clone();
	changed["expansions"][0]["sql"] = "SELECT 1".into();
	variants.push(changed);
	let mut changed = original.clone();
	changed["format_version"] = 2.into();

	variants.push(changed);
	let mut changed = original.clone();
	changed["algorithm"] = "trusted".into();
	variants.push(changed);
	let mut changed = original.clone();
	changed["verified"] = true.into();
	variants.push(changed);
	let mut changed = original.clone();
	changed["expansions"][0]["name"] = original["baseline"][0]["name"].clone();
	variants.push(changed);
	let mut changed = original;
	changed["expansions"] = serde_json::json!([]);
	variants.push(changed);

	for document in variants {
		assert!(MigrationWindow::from_json(&document.to_string()).is_err());
	}

	assert!(MigrationWindow::from_json(&" ".repeat(MAX_BYTES + 1)).is_err());
}

#[test]
fn an_independent_pin_rejects_replacement_even_when_both_documents_are_valid() {
	let (_, _, window) = fixture();
	let json = window.to_json();
	let pin = checksum(&json);
	MigrationWindow::from_json_pinned(&json, &pin).unwrap();
	let mut replacement: serde_json::Value = serde_json::from_str(&json).unwrap();
	replacement["baseline"][0]["sha256"] = "0".repeat(64).into();
	let replacement = replacement.to_string();
	// Structural validity is not authority to replace the operator's input.
	MigrationWindow::from_json(&replacement).unwrap();

	assert!(matches!(
		MigrationWindow::from_json_pinned(&replacement, &pin),
		Err(MigrationWindowError::DigestMismatch)
	));
	assert!(MigrationWindow::from_json_pinned(&format!("{json}\n"), &pin).is_err());
}
