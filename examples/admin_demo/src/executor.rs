//! A worker that rehearses operations without a cloud provider.
//!
//! Plans are real as far as they can be without credentials: the executor
//! runs the same generator `lk infra generate` does, then compares the
//! resources with the ones it last "applied". Everything after that is a
//! simulation, and each operation says so in its first step.

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::Context as _;
use anyhow::Result;
use leptoskit_config::LeptoskitConfig;
use leptoskit_deploy_console::control::Action;
use leptoskit_deploy_console::control::Executor;
use leptoskit_deploy_console::control::Operation;
use leptoskit_deploy_console::control::Project;
use leptoskit_deploy_console::control::RunResult;
use leptoskit_deploy_console::control::Service;
use serde_json::Value;
use serde_json::json;

const NOTICE: &str = "Demo executor: simulated, no cloud provider is contacted.";

pub struct Simulated {
	service: Service,
	/// The `leptoskit.toml` the simulated cloud last applied.
	applied: Arc<Mutex<String>>,
}

impl Simulated {
	pub fn new(service: Service, applied: &str) -> Self {
		Self {
			service,
			applied: Arc::new(Mutex::new(applied.to_owned())),
		}
	}
}

impl Executor for Simulated {
	fn run(
		&self,
		project: Project,
		operation: Operation,
	) -> Pin<Box<dyn Future<Output = RunResult> + Send>> {
		let service = self.service.clone();
		let applied = self.applied.clone();
		Box::pin(async move {
			let step = |message: &str| {
				let _ = service.progress(&operation.id, message);
			};
			step(NOTICE);

			match rehearse(&project.root, &operation.intent.action, &applied, step).await {
				Ok(result) => result,
				Err(error) => {
					RunResult {
						succeeded: false,
						message: format!("The demo couldn't rehearse this: {error:#}"),
						details: json!({ "code": "demo_failed" }),
					}
				}
			}
		})
	}
}

async fn pause() {
	tokio::time::sleep(Duration::from_millis(1500)).await;
}

async fn rehearse(
	root: &Path,
	action: &Action,
	applied: &Mutex<String>,
	step: impl Fn(&str),
) -> Result<RunResult> {
	let working = std::fs::read_to_string(root.join("leptoskit.toml"))?;
	let ok = |message: &str, details: Value| {
		Ok(RunResult {
			succeeded: true,
			message: message.to_owned(),
			details,
		})
	};

	match action {
		Action::Plan => {
			step("Generating infra/notes-production/main.tf.json from leptoskit.toml");
			let wanted = generate(root, root)?;
			pause().await;
			let scratch = root.join(".git/admin-demo-applied");
			std::fs::create_dir_all(&scratch)?;
			std::fs::write(scratch.join("leptoskit.toml"), applied_text(applied))?;
			let existing = generate(&scratch, &scratch)?;
			step("Comparing with what the simulated cloud has");
			pause().await;
			let changes = compare(&existing, &wanted);
			step(&match changes.len() {
				0 => "Nothing would change".to_owned(),
				1 => "1 resource would change".to_owned(),
				count => format!("{count} resources would change"),
			});
			ok(
				"Completed. The plan was compared with the simulated cloud.",
				json!({ "code": "completed", "changes": changes }),
			)
		}
		Action::Apply { .. } => {
			for message in [
				"Locking the simulated state",
				"Applying the reviewed plan",
				"Waiting for the new host to answer /health",
			] {
				step(message);
				pause().await;
			}

			*applied
				.lock()
				.map_err(|_| anyhow::anyhow!("the applied state is poisoned"))? = working;
			ok(
				"Completed. The simulated cloud matches leptoskit.toml.",
				json!({ "code": "completed" }),
			)
		}
		Action::Deploy {
			version,
			setup,
			maintenance,
		} => {
			if *setup {
				step("Creating the database and running every migration");
				pause().await;
			}

			if *maintenance {
				step("Telling live connections to come back later");
				pause().await;
			}

			for message in [
				format!("Building the image for {version}"),
				format!("Starting {version} on web-0"),
				"Waiting for web-0 to answer /health".to_owned(),
			] {
				step(&message);
				pause().await;
			}

			// `bad` stands in for a release that fails its health check.
			if version == "bad" {
				return Ok(RunResult {
					succeeded: false,
					message: "web-0 didn't answer /health with the new image, so traffic stayed \
					          on the previous release."
						.into(),
					details: json!({ "code": "health_check_failed" }),
				});
			}

			step(&format!("Moving traffic to {version}"));
			pause().await;
			ok(
				&format!("Completed. {version} is serving."),
				json!({ "code": "completed" }),
			)
		}
		Action::Rollback { version } => {
			step(&format!("Moving traffic back to {version}"));
			pause().await;
			ok(
				&format!("Completed. {version} is serving again."),
				json!({ "code": "completed" }),
			)
		}
		Action::Status => {
			let hosts = hosts(&applied_text(applied));
			step(&format!("Probing {hosts} hosts"));
			pause().await;
			ok(
				&format!("{hosts} of {hosts} hosts answered /health (simulated)."),
				json!({ "code": "completed" }),
			)
		}
		Action::Backup => {
			for message in [
				"Fetching the newest backup from notes-backups",
				"Restoring it into a scratch database",
			] {
				step(message);
				pause().await;
			}

			ok(
				"The newest backup restored cleanly (simulated).",
				json!({ "code": "completed" }),
			)
		}
	}
}

fn applied_text(applied: &Mutex<String>) -> String {
	applied.lock().map(|text| text.clone()).unwrap_or_default()
}

fn hosts(config: &str) -> usize {
	config
		.lines()
		.find_map(|line| line.trim().strip_prefix("instances = "))
		.and_then(|count| count.trim().parse().ok())
		.unwrap_or(1)
}

/// Runs the real generator on `config_root`'s `leptoskit.toml`, writing
/// under `out`, and returns the resources.
fn generate(config_root: &Path, out: &Path) -> Result<Value> {
	let config = LeptoskitConfig::load_from(config_root)?;
	let infra = config
		.managed_infra()
		.context("the demo project needs a managed [infra]")?;
	let dir = out.join("infra").join(&infra.name);
	leptoskit_cli::infra::generate(&dir, infra, true)?;
	let text = std::fs::read_to_string(dir.join("main.tf.json"))?;
	Ok(serde_json::from_str::<Value>(&text)?["resource"].clone())
}

/// What a plan would report: each resource instance created, deleted, or
/// updated between the two generated files.
fn compare(existing: &Value, wanted: &Value) -> Vec<Value> {
	let instances = |resources: &Value| {
		let mut found = std::collections::BTreeMap::new();

		for (kind, named) in resources.as_object().into_iter().flatten() {
			for (name, body) in named.as_object().into_iter().flatten() {
				let mut body = body.clone();
				let count = body.get("count").cloned();
				let each = body.get("for_each").cloned();

				if let Some(fields) = body.as_object_mut() {
					fields.remove("count");
					fields.remove("for_each");
				}

				// Addresses as OpenTofu writes them: `web[0]` for `count`,
				// `replica["1"]` for `for_each`.
				let addresses: Vec<String> = match (count, each) {
					(Some(count), _) => {
						(0..count.as_u64().unwrap_or(1))
							.map(|index| format!("{kind}.{name}[{index}]"))
							.collect()
					}
					(None, Some(Value::Object(keys))) => {
						keys.keys()
							.map(|key| format!("{kind}.{name}[\"{key}\"]"))
							.collect()
					}
					_ => vec![format!("{kind}.{name}")],
				};

				for address in addresses {
					found.insert(address, body.clone());
				}
			}
		}

		found
	};
	let before = instances(existing);
	let after = instances(wanted);
	let mut changes = Vec::new();

	for (address, body) in &after {
		match before.get(address) {
			None => changes.push(json!({ "address": address, "actions": ["create"] })),
			Some(old) if old != body => {
				changes.push(json!({ "address": address, "actions": ["update"] }));
			}
			Some(_) => {}
		}
	}

	for address in before
		.keys()
		.filter(|address| !after.contains_key(*address))
	{
		changes.push(json!({ "address": address, "actions": ["delete"] }));
	}

	changes
}
