//! The headless tools: `headless_run`, `headless_tree`, and
//! `headless_snapshot` load the built guest `.wasm` with no device or
//! window, run a script, and report, the same as `leptoskit test headless`.

use std::path::Path;

use base64::Engine;
use leptoskit_testing::headless::HeadlessApp;
use leptoskit_testing::headless::Script;
use leptoskit_testing::headless::SnapshotFormat;
use serde_json::Value;
use serde_json::json;

/// Every headless tool's name.
pub(super) const NAMES: &[&str] = &["headless_run", "headless_tree", "headless_snapshot"];

/// The arguments every tool takes: the guest, and a script to run first.
fn guest_properties() -> Value {
	json!({
		"wasm": {
			"type": "string",
			"description": "The guest module, relative to the working directory. \
				`leptoskit native bundle` writes dist/app.wasm, the default.",
		},
		"script": {
			"type": "object",
			"description": "A headless script: optional viewport {width, height} and dark, \
				then steps. Each step has an action — press, type, submit, toggle, slide, \
				scroll, resize, dark, wait, wait_for, expect, expect_absent, snapshot, or \
				tree — and selects its widget by exactly one of text, label, test_id, or \
				role with name.",
		},
	})
}

pub(super) fn tools() -> Vec<Value> {
	let mut snapshot = guest_properties();
	snapshot["format"] = json!({"type": "string", "enum": ["png", "svg"], "default": "png"});
	snapshot["scale"] = json!({"type": "number", "default": 2});
	let mut tree = guest_properties();
	tree["json"] = json!({"type": "boolean", "default": false});

	vec![
		json!({
			"name": "headless_run",
			"description": "Runs a script against the built guest .wasm with no device or \
				window: presses, typing, scrolls, waits, and assertions. Steps stop at the \
				first failure; the reply reports each step and the final widget tree.",
			"inputSchema": {
				"type": "object",
				"properties": guest_properties(),
				"required": ["script"],
				"additionalProperties": false,
			},
		}),
		json!({
			"name": "headless_tree",
			"description": "Dumps the built guest's committed widget tree, after an optional \
				script: one line per widget with its kind, accessible name, role, key props, \
				and frame (x,y width x height), or the same as JSON.",
			"inputSchema": {
				"type": "object",
				"properties": tree,
				"additionalProperties": false,
			},
		}),
		json!({
			"name": "headless_snapshot",
			"description": "Draws the built guest's laid-out tree, after an optional script, \
				without a GPU: PNG image content with text as bars, or SVG text with real \
				text. Native controls draw as outlines; it shows layout, not platform pixels.",
			"inputSchema": {
				"type": "object",
				"properties": snapshot,
				"additionalProperties": false,
			},
		}),
	]
}

/// Runs one headless tool, from the working directory.
pub(super) fn call(name: &str, arguments: &Value) -> Value {
	match run(name, arguments) {
		Ok(result) => result,
		Err(error) => {
			json!({
				"content": [{"type": "text", "text": error}],
				"isError": true,
			})
		}
	}
}

fn run(name: &str, arguments: &Value) -> Result<Value, String> {
	let wasm = arguments
		.get("wasm")
		.and_then(Value::as_str)
		.unwrap_or("dist/app.wasm");
	let wasm = std::fs::read(Path::new(wasm)).map_err(|error| {
		format!("reading {wasm}: {error}; build the guest with `leptoskit native bundle`")
	})?;
	let script: Script = match arguments.get("script") {
		Some(script) => {
			serde_json::from_value(script.clone()).map_err(|error| error.to_string())?
		}
		None => Script::default(),
	};

	if name == "headless_run" {
		let report = HeadlessApp::run_script(&wasm, &script).map_err(|error| error.to_string())?;
		return Ok(json!({
			"content": [{"type": "text", "text": report.summary()}],
			"isError": !report.passed,
		}));
	}

	let (width, height) = script.start_viewport();
	let mut app = HeadlessApp::start(&wasm, width, height).map_err(|error| error.to_string())?;

	if script.dark {
		app.set_dark_mode(true).map_err(|error| error.to_string())?;
	}

	let report = app.run_steps(&script.steps);

	if !report.passed {
		return Ok(json!({
			"content": [{"type": "text", "text": report.summary()}],
			"isError": true,
		}));
	}

	match name {
		"headless_tree" => {
			let text = if arguments.get("json").and_then(Value::as_bool) == Some(true) {
				serde_json::to_string_pretty(&app.tree_json()).expect("trees serialize")
			} else {
				app.tree()
			};
			Ok(json!({"content": [{"type": "text", "text": text}]}))
		}
		"headless_snapshot" => {
			let scale = arguments
				.get("scale")
				.and_then(Value::as_f64)
				.unwrap_or(2.0) as f32;
			let format = match arguments.get("format").and_then(Value::as_str) {
				None | Some("png") => SnapshotFormat::Png { scale },
				Some("svg") => SnapshotFormat::Svg,
				Some(other) => return Err(format!("unknown snapshot format {other:?}")),
			};
			let snapshot = app.snapshot(format).map_err(|error| error.to_string())?;

			Ok(match format {
				SnapshotFormat::Svg => {
					json!({"content": [{
						"type": "text",
						"text": String::from_utf8_lossy(snapshot.bytes()),
					}]})
				}
				SnapshotFormat::Png { .. } => {
					json!({"content": [{
						"type": "image",
						"data": base64::engine::general_purpose::STANDARD.encode(snapshot.bytes()),
						"mimeType": snapshot.mime_type(),
					}]})
				}
			})
		}
		other => Err(format!("unknown tool {other:?}")),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_missing_guest_is_a_tool_error() {
		let result = call("headless_tree", &json!({"wasm": "/nonexistent/app.wasm"}));
		assert_eq!(result["isError"], true);
		assert!(
			result["content"][0]["text"]
				.as_str()
				.unwrap()
				.contains("leptoskit native bundle")
		);
	}

	#[test]
	fn every_tool_declares_its_schema() {
		let tools = tools();
		let names: Vec<&str> = tools
			.iter()
			.map(|tool| tool["name"].as_str().unwrap())
			.collect();
		assert_eq!(names, NAMES);
		assert_eq!(tools[0]["inputSchema"]["required"], json!(["script"]));
		assert_eq!(
			tools[2]["inputSchema"]["properties"]["format"]["enum"],
			json!(["png", "svg"])
		);
	}
}
