use serde_json::json;

use super::*;

fn plans() -> (Value, Value) {
	let old = json!({"format_version":1,"kind":"graph-static-release","version":"v1",
		"installer_sha256":"a".repeat(64),"transport_sha256":"b".repeat(64),
		"manifest":"c".repeat(64),"trust":"d".repeat(64),
		"inputs":{"configuration":"e".repeat(64),"files":{"main.tf.json":"f".repeat(64)},"outputs":{}},
		"hosts":{"web":{"root":"/var/lib/leptoskit/site","trust":{"host_key":"pinned-key"},"units":{"frontends.site":{}},"edge":{"routes":[]}}},
		"evidence":{"authorization":"operator-directed"}});
	let mut current = old.clone();
	current["kind"] = json!("graph-release");
	current["installer_sha256"] = json!("1".repeat(64));
	current["transport_sha256"] = json!("2".repeat(64));
	current["hosts"]["web"]["executables"] = json!({});
	current["hosts"]["web"]["linux_formats"] = json!({});
	(current, old)
}

#[test]
fn historical_context_is_preserved_exactly_while_worker_is_upgraded() {
	let (current, old) = plans();
	assert_ne!(
		super::super::hash(&serde_json::to_vec(&current).unwrap()),
		super::super::hash(&serde_json::to_vec(&old).unwrap())
	);
	let selected = select(current, &old).unwrap();
	assert_eq!(selected, old);
	assert!(selected["hosts"]["web"].get("executables").is_none());
}

#[test]
fn recovery_cannot_change_artifact_version_selection_host_or_provider_intent() {
	for field in [
		"version",
		"manifest",
		"trust",
		"configuration",
		"files",
		"provider",
		"host",
		"units",
		"edge",
		"extra",
	] {
		let (mut current, old) = plans();
		match field {
			"version" | "manifest" | "trust" => current[field] = json!("changed"),
			"configuration" | "files" => current["inputs"][field] = json!("changed"),
			"provider" => current["inputs"]["outputs"] = json!({"ip":"changed"}),
			"host" => current["hosts"]["web"]["trust"] = json!({"host_key":"changed"}),
			"units" | "edge" => current["hosts"]["web"][field] = json!({"changed":true}),
			"extra" => current["new_field"] = json!(true),
			_ => unreachable!(),
		}
		assert!(select(current, &old).is_err(), "{field}");
	}
}

#[test]
fn legacy_static_recovery_never_authorizes_new_executables_or_unknown_formats() {
	let (mut current, old) = plans();
	current["hosts"]["web"]["executables"] = json!({"frontends.site":{"backend":"new"}});
	assert!(select(current, &old).is_err());
	for field in [
		"format_version",
		"kind",
		"installer_sha256",
		"transport_sha256",
	] {
		let (current, mut old) = plans();
		old[field] = json!("unknown");
		assert!(select(current, &old).is_err());
	}
}

#[test]
fn modern_plan_recovery_keeps_its_original_worker_identity() {
	let (current, _) = plans();
	let mut historical = current.clone();
	historical["installer_sha256"] = json!("f".repeat(64));
	assert_eq!(select(current, &historical).unwrap(), historical);
}

#[test]
fn historical_plan_bytes_require_an_independent_pin() {
	let temp = tempfile::tempdir().unwrap();
	let path = temp.path().join("release.json");
	let (_, old) = plans();
	let bytes = serde_json::to_vec_pretty(&old).unwrap();
	std::fs::write(&path, &bytes).unwrap();
	let mut args = super::super::Args {
		bundle: temp.path().join("bundle"),
		sha256: "a".repeat(64),
		trust: temp.path().join("trust.json"),
		trust_sha256: "b".repeat(64),
		caddy: Some(temp.path().join("caddy")),
		caddy_sha256: Some("c".repeat(64)),
		version: "v1".into(),
		unit: vec!["frontends.site".into()],
		plan: path.clone(),
		policy: "operator".into(),
		recover_plan_sha256: Some(super::super::hash(&bytes)),
	};
	assert_eq!(load(&args).unwrap(), Some(old));
	std::fs::write(&path, b"{}").unwrap();
	assert!(load(&args).is_err());
	args.recover_plan_sha256 = Some("invalid".into());
	assert!(load(&args).is_err());
	args.recover_plan_sha256 = None;
	assert!(load(&args).unwrap().is_none());
}
