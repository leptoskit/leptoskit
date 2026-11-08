use leptoskit_db::Migration;
use leptoskit_db::MigrationWindow;
use leptoskit_db_schema::Column;
use leptoskit_db_schema::ColumnType;
use leptoskit_db_schema::Dialect;
use leptoskit_db_schema::RequireHints;
use leptoskit_db_schema::Schema;
use leptoskit_db_schema::Table;
use leptoskit_db_schema::TableName;

fn main() {
	let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
	let baseline = Schema::new(
		Dialect::Postgres,
		vec![Table::new(
			TableName::new("notes"),
			vec![
				Column::new("id", ColumnType::Int64),
				Column::new("body", ColumnType::TEXT),
			],
			vec!["id".into()],
		)],
	);
	let mut expanded = baseline.clone();
	expanded.tables[0]
		.columns
		.push(Column::new("extra", ColumnType::TEXT).nullable());
	let sql = |before: &Schema, after: &Schema| {
		let plan = leptoskit_db_schema::diff(before, after, &mut RequireHints).unwrap();
		leptoskit_db_schema::render(&plan, before, after).sql()
	};
	let migrations = [
		Migration::new("initial", sql(&Schema::empty(Dialect::Postgres), &baseline)),
		Migration::new("expanded", sql(&baseline, &expanded)),
	];
	for (folder, count) in [("old", 1), ("new", 2)] {
		let directory = root.join(folder);
		std::fs::create_dir_all(&directory).unwrap();
		std::fs::write(
			directory.join("registry.txt"),
			if count == 1 {
				"initial\n"
			} else {
				"initial\nexpanded\n"
			},
		)
		.unwrap();
		for (migration, schema) in migrations[..count].iter().zip([&baseline, &expanded]) {
			let step = directory.join(migration.name.as_ref());
			std::fs::create_dir_all(&step).unwrap();
			std::fs::write(step.join("migration.sql"), migration.sql.as_ref()).unwrap();
			std::fs::write(step.join("schema.json"), schema.to_json()).unwrap();
		}
	}
	let window = MigrationWindow::new(
		&migrations[..1],
		&baseline.to_json(),
		[(migrations[1].clone(), expanded.to_json())],
	)
	.unwrap()
	.to_json();
	std::fs::write(root.join("window.sha256"), leptoskit_db::checksum(&window)).unwrap();
	std::fs::write(root.join("window.json"), window).unwrap();
}
