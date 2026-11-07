//! A previous published contract must remain usable by installed clients.

use leptoskit_rpc_codegen::ApiChannel;
use leptoskit_rpc_codegen::ApiFunction;
use leptoskit_rpc_codegen::ApiSpec;
use serde_json::json;

fn function() -> ApiFunction {
	serde_json::from_value(json!({
		"name": "notes", "kind": "query", "module_path": "app", "struct_name": "Notes",
		"path": "/api/notes", "method": "POST", "input": "Json", "output": "Json",
		"error_content_type": "text/plain", "params": [{"name": "limit", "required": false, "schema": {"type": "integer"}}],
		"output_schema": {"type": "string"}, "internal": false, "deprecated": false
	})).unwrap()
}

fn spec() -> ApiSpec {
	let mut spec = ApiSpec::empty("Notes", "1.0.0");
	spec.functions.push(function());
	spec
}

fn channel() -> ApiChannel {
	serde_json::from_value(json!({
		"name": "chat", "module_path": "app", "struct_name": "Chat", "fields": ["room", "topic"],
		"args_schema": {"type": "object"}, "message_schema": {"type": "string"},
		"presence_schema": {"type": "string"}
	}))
	.unwrap()
}

#[test]
fn existing_arguments_cannot_change_type_or_become_required_silently() {
	let before = spec();
	let mut after = before.clone();
	after.functions[0].params[0].schema = json!({"type": "string"});
	assert!(
		before
			.breaking_changes(&after)
			.iter()
			.any(|line| line.contains("argument `limit` schema"))
	);
	after = before.clone();
	after.functions[0].params[0].required = true;
	assert_eq!(
		before.breaking_changes(&after),
		["`notes` now requires its `limit` argument"]
	);
}

#[test]
fn references_follow_changed_definitions_and_terminate_on_cycles() {
	let mut before = spec();
	before.functions[0].output_schema = json!({"$ref": "#/components/schemas/Node"});
	before.schemas.insert(
		"Node".into(),
		json!({"type": "object", "properties": {
			"next": {"$ref": "#/components/schemas/Node"},
			"value": {"$ref": "#/$defs/Value"}
		}}),
	);
	before
		.schemas
		.insert("Value".into(), json!({"type": "string"}));
	assert_eq!(before.breaking_changes(&before), Vec::<String>::new());
	let mut after = before.clone();
	after
		.schemas
		.insert("Value".into(), json!({"type": "integer"}));
	assert!(
		before
			.breaking_changes(&after)
			.iter()
			.any(|line| line.contains("result schema"))
	);
	after = before.clone();
	after
		.schemas
		.insert("Unrelated".into(), json!({"type": "integer"}));
	assert_eq!(before.breaking_changes(&after), Vec::<String>::new());
}

#[test]
fn publication_keeps_conditional_and_pattern_schema_references() {
	let mut before = spec();
	before.functions[0].output_schema = json!({
		"type": "object", "if": {"properties": {"kind": {"const": "note"}}},
		"then": {"$ref": "#/components/schemas/Conditional"},
		"else": {"not": {"$ref": "#/components/schemas/Negated"}},
		"patternProperties": {"^extra": {"$ref": "#/components/schemas/Pattern"}},
		"dependentSchemas": {"kind": {"$ref": "#/components/schemas/Dependent"}},
		"unevaluatedProperties": {"$ref": "#/components/schemas/Other"}
	});
	for name in ["Conditional", "Negated", "Pattern", "Dependent", "Other"] {
		before
			.schemas
			.insert(name.into(), json!({"type": "string"}));
	}
	let before = before.public();
	assert_eq!(before.schemas.len(), 5);
	for name in before.schemas.keys() {
		let mut after = before.clone();
		after
			.schemas
			.insert(name.clone(), json!({"type": "integer"}));
		assert!(
			!before.breaking_changes(&after.public()).is_empty(),
			"changed {name}"
		);
	}
}

#[test]
fn application_data_with_reference_keys_is_not_a_schema_reference() {
	let mut before = spec();
	before.functions[0].output_schema = json!({"type": "object", "examples": [{"$ref": "https://example.invalid/data"}], "default": {"$ref": "data"}, "const": {"$ref": "literal"}});
	assert_eq!(before.breaking_changes(&before), Vec::<String>::new());
}

#[test]
fn unresolved_and_dynamic_references_cannot_claim_compatibility() {
	for schema in [
		json!({"$ref": "https://example.invalid/schema"}),
		json!({"$ref": "#/components/schemas/Missing"}),
		json!({"$dynamicRef": "#node"}),
		json!({"$recursiveRef": "#"}),
		json!({"$id": "https://example.invalid/schema", "type": "object"}),
	] {
		let mut before = spec();
		before.functions[0].output_schema = schema;
		assert_ne!(before.breaking_changes(&before), Vec::<String>::new());
	}
}

#[test]
fn channel_removal_key_order_payloads_and_live_path_are_checked() {
	let mut before = spec();
	before.channels.push(channel());
	let mut after = before.clone();
	after.channels.clear();
	assert_eq!(before.breaking_changes(&after), ["removed channel `chat`"]);
	after = before.clone();
	after.channels[0].fields.reverse();
	assert_eq!(
		before.breaking_changes(&after),
		["channel `chat` changed its join field order"]
	);
	for part in 0..3 {
		after = before.clone();
		let value = match part {
			0 => &mut after.channels[0].args_schema,
			1 => &mut after.channels[0].message_schema,
			_ => &mut after.channels[0].presence_schema,
		};
		*value = json!({"type": "integer"});
		assert_eq!(before.breaking_changes(&after).len(), 1);
	}
	after = before.clone();
	after.live_path = "/v2/live".into();
	assert_eq!(
		before.breaking_changes(&after),
		["live endpoint moved from /live to /v2/live"]
	);
}

#[test]
fn errors_and_duplicate_names_cannot_hide_contract_changes() {
	let before = spec();
	let mut after = before.clone();
	after.functions[0].error_content_type = "application/json".into();
	assert_eq!(
		before.breaking_changes(&after),
		["`notes` changed its error encoding"]
	);
	after = before.clone();
	after.functions.push(function());
	assert!(
		before
			.breaking_changes(&after)
			.iter()
			.any(|line| line.contains("repeats function"))
	);
	after = before.clone();
	let mut duplicate_route = function();
	duplicate_route.name = "different_name".into();
	after.functions.push(duplicate_route);
	assert!(
		before
			.breaking_changes(&after)
			.iter()
			.any(|line| line.contains("repeats route"))
	);
	after = before.clone();
	let repeated = after.functions[0].params[0].clone();
	after.functions[0].params.push(repeated);
	assert!(
		before
			.breaking_changes(&after)
			.iter()
			.any(|line| line.contains("repeats `notes` argument"))
	);
	after = before.clone();
	after.channels = vec![channel(), channel()];
	assert!(
		before
			.breaking_changes(&after)
			.iter()
			.any(|line| line.contains("repeats channel"))
	);
}

#[test]
fn new_operations_optional_arguments_and_function_docs_are_allowed() {
	let before = spec();
	let mut after = before.clone();
	let mut added = function();
	added.name = "other".into();
	added.path = "/api/other".into();
	after.functions.push(added);
	let mut param = after.functions[0].params[0].clone();
	param.name = "offset".into();
	after.functions[0].params.push(param);
	after.functions[0].docs = "Updated documentation".into();
	assert_eq!(before.breaking_changes(&after), Vec::<String>::new());
}

#[test]
fn escaped_component_names_and_nested_pointers_keep_their_definitions() {
	let mut before = spec();
	before.functions[0].output_schema =
		json!({"$ref": "#/components/schemas/A~1B/properties/value"});
	before.schemas.insert(
		"A/B".into(),
		json!({"type": "object", "properties": {"value": {"type": "string"}}}),
	);
	let before = before.public();
	assert!(before.schemas.contains_key("A/B"));
	assert_eq!(before.breaking_changes(&before), Vec::<String>::new());
	let mut after = before.clone();
	after.schemas["A/B"]["properties"]["value"] = json!({"type": "integer"});
	assert_ne!(before.breaking_changes(&after), Vec::<String>::new());
}

#[test]
fn legacy_tuples_dependencies_and_content_schemas_follow_references() {
	for schema in [
		json!({"items": [{"$ref": "#/$defs/Value"}]}),
		json!({"dependencies": {"key": {"$ref": "#/$defs/Value"}, "other": ["key"]}}),
		json!({"contentSchema": {"$ref": "#/$defs/Value"}}),
	] {
		let mut before = spec();
		before.functions[0].output_schema = schema;
		before
			.schemas
			.insert("Value".into(), json!({"type": "string"}));
		let before = before.public();
		assert!(before.schemas.contains_key("Value"));
		assert_eq!(before.breaking_changes(&before), Vec::<String>::new());
		let mut after = before.clone();
		after.schemas["Value"] = json!({"type": "integer"});
		assert_ne!(
			before.breaking_changes(&after.public()),
			Vec::<String>::new()
		);
	}
}
