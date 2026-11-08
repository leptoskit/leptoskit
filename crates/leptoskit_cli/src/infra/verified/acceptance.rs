//! Real confined engine export controls, explicitly opted into by the operator.

use super::*;

#[test]
#[ignore = "requires the owned acceptance engine and successfully packaged descriptor"]
fn real_confined_exports_reject_missing_failed_and_nonregular_outputs() {
	let directory = std::env::var("LEPTOSKIT_WINDOW_ACCEPTANCE_EVIDENCE").unwrap();
	let descriptor: Value =
		serde_json::from_slice(&fs::read(Path::new(&directory).join("descriptor.json")).unwrap())
			.unwrap();
	let engine = Engine::Podman;
	let exported = export_document(
		engine,
		descriptor["engine_image_id"].as_str().unwrap(),
		"LEPTOSKIT_DB_CONTRACT_OUT",
		"database.json",
		"migration contract",
		8 * 1024 * 1024,
	)
	.unwrap();
	assert_eq!(
		exported,
		descriptor["schema_window"]["contract"]
			.as_str()
			.unwrap()
			.as_bytes()
	);
	let scripts = [
		("exit 0", "did not export"),
		("exit 9", "export failed"),
		(
			"ln -s /etc/passwd \"$LEPTOSKIT_DB_CONTRACT_OUT\"",
			"invalid migration contract export file",
		),
		(
			"head -c 2048 /dev/zero > \"$LEPTOSKIT_DB_CONTRACT_OUT\"",
			"invalid migration contract export file",
		),
	];
	for (index, (script, expected)) in scripts.iter().enumerate() {
		let context = tempfile::tempdir().unwrap();
		fs::write(
			context.path().join("backend"),
			format!("#!/bin/sh\n{script}\n"),
		)
		.unwrap();
		fs::write(
			context.path().join("Dockerfile"),
			format!(
				"FROM {}\nCOPY --chmod=755 backend /app/backend\n",
				descriptor["runtime_image"].as_str().unwrap()
			),
		)
		.unwrap();
		let tag = format!("localhost/a05-export-control:{index}");
		output(
			engine,
			&[
				"build",
				"--network=none",
				"-t",
				&tag,
				context.path().to_str().unwrap(),
			],
		)
		.unwrap();
		let result = export_document(
			engine,
			&tag,
			"LEPTOSKIT_DB_CONTRACT_OUT",
			"database.json",
			"migration contract",
			1024,
		);
		output(engine, &["rmi", &tag]).unwrap();
		assert!(
			result.unwrap_err().to_string().contains(expected),
			"{script}"
		);
	}
}

#[test]
#[ignore = "requires the owned acceptance engine and successfully packaged descriptor"]
fn real_exported_schema_and_history_substitutions_fail_source_binding() {
	let directory = std::env::var("LEPTOSKIT_WINDOW_ACCEPTANCE_EVIDENCE").unwrap();
	let descriptor: Value =
		serde_json::from_slice(&fs::read(Path::new(&directory).join("descriptor.json")).unwrap())
			.unwrap();
	let expected = leptoskit_db::MigrationContract::from_json(
		descriptor["schema_window"]["contract"].as_str().unwrap(),
	)
	.unwrap();
	let window = leptoskit_db::MigrationWindow::from_json(
		descriptor["schema_window"]["window"].as_str().unwrap(),
	)
	.unwrap();
	for field in ["schema", "history"] {
		let mut changed: Value = serde_json::from_str(&expected.to_json()).unwrap();
		if field == "schema" {
			changed["schema"]["tables"][0]["comment"] = json!("another source checkpoint");
		} else {
			changed["migrations"][0]["sha256"] = json!("0".repeat(64));
		}
		let json = serde_json::to_string(&changed).unwrap();
		let other_source = leptoskit_db::MigrationContract::from_json(&json).unwrap();
		let context = tempfile::tempdir().unwrap();
		fs::write(context.path().join("contract.json"), &json).unwrap();
		fs::write(
			context.path().join("backend"),
			"#!/bin/sh\ncp /contract.json \"$LEPTOSKIT_DB_CONTRACT_OUT\"\n",
		)
		.unwrap();
		fs::write(
			context.path().join("Dockerfile"),
			format!(
				"FROM {}\nCOPY --chmod=755 backend /app/backend\nCOPY --chmod=444 contract.json \
				 /contract.json\n",
				descriptor["runtime_image"].as_str().unwrap()
			),
		)
		.unwrap();
		let tag = format!("localhost/a05-export-substitution:{field}");
		output(
			Engine::Podman,
			&[
				"build",
				"--network=none",
				"-t",
				&tag,
				context.path().to_str().unwrap(),
			],
		)
		.unwrap();
		let exported = export_document(
			Engine::Podman,
			&tag,
			"LEPTOSKIT_DB_CONTRACT_OUT",
			"database.json",
			"migration contract",
			8 * 1024 * 1024,
		)
		.unwrap();
		output(Engine::Podman, &["rmi", &tag]).unwrap();
		assert!(
			schema_window::check_exported_contract(
				&expected,
				&window,
				std::str::from_utf8(&exported).unwrap()
			)
			.is_err()
		);
		assert!(
			schema_window::check_exported_contract(&other_source, &window, &expected.to_json())
				.is_err()
		);
	}
}
