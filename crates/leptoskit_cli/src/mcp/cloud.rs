//! `LeptosKit` Cloud and database MCP tools: gives AI agents end-to-end
//! visibility and safe deployment controls (`cloud_estimate`, `cloud_deploy`,
//! `cloud_status`, `cloud_logs`, `cloud_secrets_list`, `db_check`).
//!
//! `cloud_secrets_list` returns secret names and version metadata only —
//! secret values are never exposed over MCP.

use serde_json::Value;
use serde_json::json;

use crate::cloud::CloudArgs;
use crate::cloud::CloudCommand;
use crate::cloud::ship::SecretCommand;

/// Tool names routed to this module by the MCP dispatcher.
pub(super) const NAMES: &[&str] = &[
	"cloud_estimate",
	"cloud_deploy",
	"cloud_status",
	"cloud_logs",
	"cloud_secrets_list",
	"db_check",
];

/// Tool schemas advertised in `tools/list`.
pub(super) fn tools() -> Vec<Value> {
	vec![
		json!({
			"name": "cloud_estimate",
			"description": "Validates `[cloud]`, `[db]`, and `[services]` in `leptoskit.toml` and \
				returns the resolved release plan and (when signed in and linked) live monthly \
				cost estimate from LeptosKit Cloud without deploying.",
			"inputSchema": {
				"type": "object",
				"properties": {
					"environment": {"type": "string", "description": "Environment name (defaults to production)."},
					"org": {"type": "string"},
					"project": {"type": "string"},
				},
				"additionalProperties": false,
			},
		}),
		json!({
			"name": "cloud_deploy",
			"description": "Packs, uploads, and deploys the current LeptosKit project to LeptosKit \
				Cloud. Pass `detach: true` to return immediately once queued instead of blocking \
				until rollout completes.",
			"inputSchema": {
				"type": "object",
				"properties": {
					"environment": {"type": "string", "description": "Environment name (defaults to production)."},
					"detach": {"type": "boolean", "description": "Return once queued without waiting for rollout."},
					"org": {"type": "string"},
					"project": {"type": "string"},
				},
				"additionalProperties": false,
			},
		}),
		json!({
			"name": "cloud_status",
			"description": "Returns the latest observed process readiness, replica counts, \
				restarts, and live HTTPS hostname for a LeptosKit Cloud environment.",
			"inputSchema": {
				"type": "object",
				"properties": {
					"environment": {"type": "string", "description": "Environment name (defaults to production)."},
					"org": {"type": "string"},
					"project": {"type": "string"},
				},
				"additionalProperties": false,
			},
		}),
		json!({
			"name": "cloud_logs",
			"description": "Fetches recent runtime log lines from a LeptosKit Cloud environment.",
			"inputSchema": {
				"type": "object",
				"properties": {
					"environment": {"type": "string"},
					"process": {"type": "string", "description": "Filter to one process (e.g. `app`)."},
					"since_minutes": {"type": "integer", "minimum": 1},
					"contains": {"type": "string"},
					"limit": {"type": "integer", "minimum": 1, "maximum": 1000},
					"org": {"type": "string"},
					"project": {"type": "string"},
				},
				"additionalProperties": false,
			},
		}),
		json!({
			"name": "cloud_secrets_list",
			"description": "Lists the names and version metadata of secrets configured in a \
				LeptosKit Cloud environment. Never returns secret values.",
			"inputSchema": {
				"type": "object",
				"properties": {
					"environment": {"type": "string"},
					"org": {"type": "string"},
					"project": {"type": "string"},
				},
				"additionalProperties": false,
			},
		}),
		json!({
			"name": "db_check",
			"description": "Checks that `#[db::table]` schema definitions match committed \
				migrations and database invariants (equivalent to `lk db check`). Read-only.",
			"inputSchema": {
				"type": "object",
				"properties": {
					"service": {"type": "string", "description": "Optional `[services.<name>]` database to check."},
				},
				"additionalProperties": false,
			},
		}),
	]
}

fn str_arg(arguments: &Value, key: &str) -> Option<String> {
	arguments
		.get(key)
		.and_then(Value::as_str)
		.map(str::trim)
		.filter(|s| !s.is_empty())
		.map(str::to_owned)
}

fn u32_arg(arguments: &Value, key: &str) -> Option<u32> {
	arguments
		.get(key)
		.and_then(Value::as_u64)
		.and_then(|v| u32::try_from(v).ok())
}

/// The arguments of one cloud command for a tool call. There is no `url`: an
/// argument comes from the model or the client, and `Context::load` treats
/// `CloudArgs.url` as a cloud the person named, which `LEPTOSKIT_CLOUD_TOKEN`
/// is then sent to. A person names a cloud with `--url` or
/// `LEPTOSKIT_CLOUD_URL`.
fn cloud_args(arguments: &Value, command: CloudCommand) -> CloudArgs {
	CloudArgs {
		url: None,
		json: true,
		org: str_arg(arguments, "org"),
		project: str_arg(arguments, "project"),
		environment: str_arg(arguments, "environment"),
		command,
	}
}

fn ok_json(value: &Value) -> Value {
	let text = serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string());
	json!({
		"content": [{"type": "text", "text": text}],
		"isError": false,
	})
}

fn err_text(message: &str) -> Value {
	json!({
		"content": [{"type": "text", "text": message}],
		"isError": true,
	})
}

/// An error as the model reads it. The guard on `LEPTOSKIT_CLOUD_TOKEN` tells
/// a person to pass `--url` or set `LEPTOSKIT_CLOUD_URL`; a tool call has no
/// `--url`, so only the variable is named.
fn tool_error(error: &anyhow::Error) -> String {
	error.to_string().replace(
		"Pass --url or set LEPTOSKIT_CLOUD_URL",
		"Set LEPTOSKIT_CLOUD_URL",
	)
}

/// Executes one cloud or database MCP tool call.
pub(super) fn call(name: &str, arguments: &Value) -> Value {
	match name {
		"cloud_estimate" => {
			let args = cloud_args(arguments, CloudCommand::Estimate);
			match crate::cloud::execute_json(&args) {
				Ok(value) => ok_json(&value),
				Err(error) => err_text(&tool_error(&error)),
			}
		}
		"cloud_deploy" => {
			let detach = arguments
				.get("detach")
				.and_then(Value::as_bool)
				.unwrap_or(false);
			let args = cloud_args(arguments, CloudCommand::Deploy { detach });
			match crate::cloud::execute_json(&args) {
				Ok(value) => ok_json(&value),
				Err(error) => err_text(&tool_error(&error)),
			}
		}
		"cloud_status" => {
			let args = cloud_args(arguments, CloudCommand::Status);
			match crate::cloud::execute_json(&args) {
				Ok(value) => ok_json(&value),
				Err(error) => err_text(&tool_error(&error)),
			}
		}
		"cloud_logs" => {
			let args = cloud_args(
				arguments,
				CloudCommand::Logs {
					follow: false,
					process: str_arg(arguments, "process"),
					since_minutes: u32_arg(arguments, "since_minutes"),
					contains: str_arg(arguments, "contains"),
					limit: u32_arg(arguments, "limit"),
				},
			);
			match crate::cloud::execute_json(&args) {
				Ok(value) => ok_json(&value),
				Err(error) => err_text(&tool_error(&error)),
			}
		}
		"cloud_secrets_list" => {
			let args = cloud_args(
				arguments,
				CloudCommand::Secret {
					command: SecretCommand::List,
				},
			);
			match crate::cloud::execute_json(&args) {
				Ok(value) => ok_json(&value),
				Err(error) => err_text(&tool_error(&error)),
			}
		}
		"db_check" => {
			let service = str_arg(arguments, "service");
			match crate::db::run(
				crate::db::DbCommand::Check {
					live: false,
					url: None,
				},
				service.as_deref(),
			) {
				Ok(()) => ok_json(&json!({ "ok": true })),
				Err(error) => err_text(&error.to_string()),
			}
		}
		other => err_text(&format!("unknown cloud MCP tool: {other}")),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn tool_schemas_match_routed_names_and_never_expose_secret_values() {
		let schemas = tools();
		let names: Vec<&str> = schemas
			.iter()
			.map(|tool| tool["name"].as_str().unwrap())
			.collect();
		assert_eq!(names, NAMES);

		let secrets_tool = schemas
			.iter()
			.find(|tool| tool["name"] == "cloud_secrets_list")
			.expect("cloud_secrets_list schema");
		assert!(
			secrets_tool["description"]
				.as_str()
				.unwrap()
				.contains("Never returns secret values")
		);
	}

	#[test]
	fn cloud_tools_offer_no_url_argument() {
		for tool in tools() {
			let name = tool["name"].as_str().unwrap();
			if name != "db_check" {
				assert!(
					tool["inputSchema"]["properties"].get("url").is_none(),
					"{name}"
				);
			}
		}
	}

	/// The tool call itself, in the child process `cloud::tests` starts for
	/// its `cloud_tools_…` tests: `call` is this module's, and
	/// `LEPTOSKIT_CLOUD_TOKEN` belongs to the environment of a whole process.
	#[test]
	fn subprocess_helper() {
		let (Ok(request), Ok(written)) = (
			std::env::var("LEPTOSKIT_TEST_TOOL_CALL"),
			std::env::var("LEPTOSKIT_TEST_TOOL_ANSWER"),
		) else {
			return;
		};
		let request: Value = serde_json::from_str(&request).unwrap();
		let answer = call(request["name"].as_str().unwrap(), &request["arguments"]);
		std::fs::write(written, answer.to_string()).unwrap();
	}
}
