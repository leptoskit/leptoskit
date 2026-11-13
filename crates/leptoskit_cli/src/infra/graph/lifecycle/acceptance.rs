//! Real operator-process acceptance using only `OpenTofu`'s built-in local
//! resource. Cloud provider planning is covered by the separate mocked
//! provider fixture; graph receipt substitution is covered by unit tests.

use super::*;

#[test]
#[ignore = "requires a verified OpenTofu executable on PATH; no cloud providers or credentials"]
fn real_local_saved_plan_inspection_and_apply() {
	environment().unwrap();
	let root = tempfile::tempdir().unwrap();
	let dir = root.path();
	fs::write(
		dir.join("main.tf.json"),
		serde_json::to_vec_pretty(&serde_json::json!({
			"terraform":{"required_version":">= 1.10, < 2"},
			"resource":{"terraform_data":{"fixture":{"input":"graph-provisioning-fixture"}}},
			"output":{"hosts":{"value":{"fixture":"${terraform_data.fixture.output}"}},
				"private_value":{"value":"fixture-private-output","sensitive":true}}
		}))
		.unwrap(),
	)
	.unwrap();
	let engine = Tofu;
	let identity = identity(dir, &engine).unwrap();
	engine
		.invoke(dir, &["init", "-input=false"], false)
		.unwrap();
	assert_eq!(workspace(dir, &engine).unwrap(), "default");

	engine.invoke(dir, &["validate"], false).unwrap();
	engine
		.invoke(
			dir,
			&["plan", "-input=false", "-lock=true", "-out=leptoskit.plan"],
			false,
		)
		.unwrap();
	protect(dir, &engine).unwrap();
	let plan_bytes = fs::read(dir.join(PLAN)).unwrap();
	assert_ne!(plan_bytes, [] as [u8; 0]);
	engine
		.invoke(
			dir,
			&["apply", "-input=false", "-lock=true", "leptoskit.plan"],
			false,
		)
		.unwrap();
	let public = engine
		.invoke(dir, &["output", "-json", "hosts"], true)
		.unwrap();
	assert_eq!(
		serde_json::from_slice::<Value>(&public).unwrap(),
		serde_json::json!({"fixture":"graph-provisioning-fixture"})
	);
	assert!(
		!String::from_utf8(public)
			.unwrap()
			.contains("fixture-private-output")
	);
	assert!(
		engine
			.invoke(
				dir,
				&["apply", "-input=false", "-lock=true", "leptoskit.plan"],
				true
			)
			.is_err(),
		"the real engine must reject the now-stale saved plan"
	);
	fs::write(dir.join(PLAN), b"not a saved plan").unwrap();
	assert!(protect(dir, &engine).is_err());
	println!(
		"Real local saved-plan acceptance: {} {}; no cloud resources",
		identity.version, identity.platform
	);
}
