use super::*;

const SPEC: &str = include_str!(concat!(
	env!("CARGO_MANIFEST_DIR"),
	"/tests/fixtures/fns_spec.json"
));

fn baseline(specification: &str) -> Baseline {
	Baseline {
		sha256: digest(specification.as_bytes()),
		specification: specification.into(),
	}
}

fn contract() -> Contract {
	Contract::exported(SPEC.as_bytes().to_vec(), &[baseline(SPEC)]).unwrap()
}

#[test]
fn exact_contracts_are_retained_and_recomputed() {
	let contract = contract();
	assert!(contract.checked());
	contract.require(&[digest(SPEC.as_bytes())]).unwrap();
	let stored = serde_json::to_vec(&contract).unwrap();
	let decoded: Contract = serde_json::from_slice(&stored).unwrap();
	decoded.validate().unwrap();
	decoded.require(&[digest(SPEC.as_bytes())]).unwrap();
}

#[test]
fn changing_either_specification_cannot_reuse_its_digest() {
	let original = serde_json::to_value(contract()).unwrap();
	for pointer in [
		"/compatibility/checked/specification",
		"/compatibility/checked/baselines/0/specification",
	] {
		let mut value = original.clone();
		*value.pointer_mut(pointer).unwrap() = serde_json::json!(format!("{SPEC} "));
		let changed: Contract = serde_json::from_value(value).unwrap();
		assert!(changed.validate().is_err(), "{pointer}");
	}
}

#[test]
fn changing_the_candidate_and_rehashing_it_still_checks_the_contract() {
	let mut candidate: serde_json::Value = serde_json::from_str(SPEC).unwrap();
	candidate["functions"][0]["params"][0]["schema"] = serde_json::json!({"type": "integer"});
	let bytes = serde_json::to_vec(&candidate).unwrap();
	assert!(Contract::exported(bytes, &[baseline(SPEC)]).is_err());
}

#[test]
fn a_publisher_selected_empty_baseline_cannot_replace_operator_policy() {
	let empty = ApiSpec::empty("empty", "1").to_json().unwrap();
	let candidate = Contract::exported(SPEC.as_bytes().to_vec(), &[baseline(&empty)]).unwrap();
	candidate.validate().unwrap();
	assert!(candidate.require(&[digest(SPEC.as_bytes())]).is_err());
}

#[test]
fn every_supported_client_baseline_must_be_checked() {
	let mut second: serde_json::Value = serde_json::from_str(SPEC).unwrap();
	second["title"] = serde_json::json!("another published version");
	let second = serde_json::to_string(&second).unwrap();
	let baselines = [baseline(SPEC), baseline(&second)];
	let candidate = Contract::exported(SPEC.as_bytes().to_vec(), &baselines).unwrap();
	candidate
		.require(
			&baselines
				.iter()
				.map(|item| item.sha256.clone())
				.collect::<Vec<_>>(),
		)
		.unwrap();
	assert!(contract().require(&[digest(second.as_bytes())]).is_err());
}

#[test]
fn legacy_and_no_rpc_evidence_do_not_satisfy_a_required_window() {
	let legacy: Contract = serde_json::from_value(serde_json::json!({"state":"exported", "sha256":digest(SPEC.as_bytes()),"compatibility":"not_checked"})).unwrap();
	legacy.validate().unwrap();
	assert!(!legacy.checked());
	assert!(legacy.require(&[digest(SPEC.as_bytes())]).is_err());
	let absent = Contract::NotApplicable {
		declaration: "operator_declares_no_rpc".into(),
	};
	assert!(absent.require(&[digest(SPEC.as_bytes())]).is_err());
}

#[test]
fn missing_duplicate_and_unknown_algorithm_evidence_is_refused() {
	let original = serde_json::to_value(contract()).unwrap();
	for replacement in [
		serde_json::json!([]),
		serde_json::json!([baseline(SPEC), baseline(SPEC)]),
	] {
		let mut value = original.clone();
		value["compatibility"]["checked"]["baselines"] = replacement;
		let changed: Contract = serde_json::from_value(value).unwrap();
		assert!(changed.validate().is_err());
	}
	let mut value = original;
	value["compatibility"]["checked"]["algorithm"] = serde_json::json!("unrecognized");
	let changed: Contract = serde_json::from_value(value).unwrap();
	assert!(changed.validate().is_err());
}

#[test]
fn baseline_files_require_pins_and_are_frozen_after_reading() {
	let dir = tempfile::tempdir().unwrap();
	let path = dir.path().join("rpc.json");
	fs::write(&path, SPEC).unwrap();
	let pin = digest(SPEC.as_bytes());
	assert!(baselines(std::slice::from_ref(&path), &[]).is_err());
	assert!(baselines(std::slice::from_ref(&path), &["f".repeat(64)]).is_err());
	let selected = baselines(std::slice::from_ref(&path), std::slice::from_ref(&pin)).unwrap();
	fs::write(path, "replaced after validation").unwrap();
	Contract::exported(SPEC.as_bytes().to_vec(), &selected)
		.unwrap()
		.require(&[pin])
		.unwrap();
}
