//! Exercise the public command with captured contracts and no server build.

use super::*;

fn compare(config: &str, update: impl FnOnce(&mut serde_json::Value)) -> Output {
	let scratch = Scratch::new("compatibility");
	project(&scratch.path, config);
	let mut current: serde_json::Value = serde_json::from_str(SPEC).unwrap();
	update(&mut current);
	std::fs::write(
		scratch.path.join("current.json"),
		serde_json::to_vec(&current).unwrap(),
	)
	.unwrap();
	leptoskit(
		&scratch.path,
		&[
			"rpc",
			"check",
			"--against",
			"spec.json",
			"--from",
			"current.json",
		],
	)
}

#[test]
fn unchanged_published_contract_passes_without_building_a_server() {
	let result = compare("", |_| {});
	assert!(result.status.success(), "{}", stderr(&result));
	assert!(stdout(&result).contains("no breaking changes"));
}

#[test]
fn request_type_changes_fail_the_command() {
	let result = compare("", |current| {
		current["functions"][0]["params"][0]["schema"] = serde_json::json!({"type": "integer"});
	});
	assert!(!result.status.success());
	assert!(
		stdout(&result).contains("argument `title` schema"),
		"{}",
		stdout(&result)
	);
}

#[test]
fn unchanged_reference_text_does_not_hide_changed_results() {
	let result = compare("", |current| {
		current["schemas"]["Todo"] = serde_json::json!({"type": "string"});
	});
	assert!(!result.status.success());
	assert!(
		stdout(&result).contains("result schema"),
		"{}",
		stdout(&result)
	);
}

#[test]
fn current_exclusions_cannot_rewrite_the_previous_contract() {
	let result = compare("[rpc]\nexclude = [\"get_todo\"]\n", |_| {});
	assert!(!result.status.success());
	assert!(
		stdout(&result).contains("removed `get_todo`"),
		"{}",
		stdout(&result)
	);
}
