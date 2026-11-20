//! The Workings drawers for infrastructure: each action walked forward
//! from the `leptoskit.toml` that declares it to the check that says it's
//! done.

use leptoskit_deploy_console::control::Operation;

use super::Change;
use super::ProjectState;
use crate::components::Line;
use crate::components::Mark;
use crate::components::Tone;
use crate::workings::Body;
use crate::workings::Drawer;
use crate::workings::Station;

/// Declare: the `[infra]` change since the last commit, or the tables as
/// they stand.
fn declare(state: &ProjectState, why: &str) -> Station {
	if !state.changed.is_empty() {
		return Station::new(
			"Declare",
			"leptoskit.toml, changes not yet committed",
			format!("{why} These lines changed since the last commit."),
			Body::Code(state.changed.clone(), "TOML".to_owned()),
		);
	}

	if state.declared.is_empty() {
		return Station::new(
			"Declare",
			"leptoskit.toml",
			why,
			Body::Missing(match &state.config {
				Err(reason) => format!("Can't read this project's [infra]: {reason}."),
				Ok(_) => "This project's [infra] lives somewhere the admin can't read.".to_owned(),
			}),
		);
	}

	Station::new(
		"Declare",
		"leptoskit.toml",
		format!("{why} Nothing in [infra] changed since the last commit."),
		Body::Code(state.declared.clone(), "TOML".to_owned()),
	)
}

/// Generate: the `main.tf.json` resources a plan touches.
fn generate(state: &ProjectState, changes: &[Change]) -> Station {
	let why = "`lk infra generate` turns [infra] into OpenTofu's own JSON, which you can read and \
	           review like any other file.";
	let name = state
		.config
		.as_ref()
		.map_or_else(|_| "<name>".to_owned(), |config| config.name.clone());
	let source = format!("infra/{name}/main.tf.json");

	let Some(generated) = &state.generated else {
		return Station::new(
			"Generate",
			source.clone(),
			why,
			Body::Missing(format!(
				"{source} isn't in this checkout yet. Plan writes it first, so it'll be here \
				 after the next plan."
			)),
		);
	};

	let Ok(json) = serde_json::from_str::<serde_json::Value>(generated) else {
		return Station::new(
			"Generate",
			source,
			why,
			Body::Missing("The generated file isn't valid JSON, so it can't be excerpted.".into()),
		);
	};

	let wanted: Vec<&str> = changes.iter().map(Change::resource_type).collect();
	let mut excerpt = serde_json::Map::new();

	if let Some(resources) = json["resource"].as_object() {
		for (kind, value) in resources {
			if wanted.is_empty() && excerpt.is_empty() || wanted.contains(&kind.as_str()) {
				excerpt.insert(kind.clone(), value.clone());
			}
		}
	}

	if excerpt.is_empty() {
		return Station::new(
			"Generate",
			source,
			why,
			Body::Missing("The generated file has no resources matching this change.".into()),
		);
	}

	let text = serde_json::to_string_pretty(&serde_json::json!({ "resource": excerpt }))
		.unwrap_or_default();
	let mut lines: Vec<Line> = text.lines().map(Line::plain).collect();

	if lines.len() > 40 {
		lines.truncate(40);
		lines.push(Line::plain(format!("  … the rest is in {source}")));
	}

	Station::new(
		"Generate",
		source,
		why,
		Body::Code(lines, "JSON".to_owned()),
	)
}

/// Plan: the resource diff, as `OpenTofu` reports it.
fn plan(state: &ProjectState) -> Station {
	let why = "A plan compares what's declared with what exists and lists the difference. It \
	           changes nothing.";

	match state.plan() {
		Some(plan) if !plan.changes.is_empty() => {
			let lines = plan
				.changes
				.iter()
				.map(|change| {
					Line::marked(
						format!(
							"{} {:<8} {}",
							match change.verb() {
								"create" => "+",
								"delete" => "-",
								"replace" => "-/+",
								_ => "~",
							},
							change.verb(),
							change.address
						),
						match change.verb() {
							"create" => Mark::Added,
							"delete" | "replace" => Mark::Removed,
							_ => Mark::Plain,
						},
					)
					.key(change.key())
				})
				.collect();
			Station::new(
				"Plan",
				format!("operation {}", short_id(&plan.operation.id)),
				why,
				Body::Code(lines, "text".to_owned()),
			)
		}
		Some(plan) => {
			Station::new(
				"Plan",
				format!("operation {}", short_id(&plan.operation.id)),
				why,
				Body::Checks(vec![(
					Tone::Good,
					"No changes: what's running matches leptoskit.toml.".to_owned(),
				)]),
			)
		}
		None => {
			Station::new(
				"Plan",
				"no plan yet",
				why,
				Body::Missing(match state.last_plan() {
					Some(failed) => {
						format!(
							"The last plan didn't finish: {} Plans need the provider tokens on \
							 the machine running the worker.",
							failed.message
						)
					}
					None => {
						"No plan yet. Plan changes runs `lk infra plan` as an operation.".to_owned()
					}
				}),
			)
		}
	}
}

fn events(operation: &Operation) -> Vec<Line> {
	operation
		.events
		.iter()
		.map(|event| {
			Line::plain(format!(
				"#{:<3} {:?}  {}",
				event.sequence, event.phase, event.message
			))
		})
		.collect()
}

/// Apply: the command the worker runs and what it streamed last time.
fn apply(command: &str, last: Option<&Operation>) -> Station {
	let mut lines = vec![
		Line::plain("# The worker runs, in a checkout of this project:".to_string()),
		Line::plain(command),
	];

	if let Some(operation) = last {
		lines.push(Line::plain(""));
		lines.push(Line::plain(format!(
			"# Events from operation {}:",
			short_id(&operation.id)
		)));
		lines.extend(events(operation));
	}

	Station::new(
		"Apply",
		"the control service's worker",
		"The control service runs one operation per project at a time, records every step, and \
		 keeps going if you close this tab.",
		Body::Code(lines, "shell".to_owned()),
	)
}

/// Check: what "done" means.
fn check(state: &ProjectState, extra: Vec<(Tone, String)>) -> Station {
	let mut checks = extra;

	match state.health() {
		Some(health) => {
			let (tone, word) = super::phase_word(health.phase);
			checks.push((
				tone,
				format!("Last health check: {word}. {}", health.message),
			));
		}
		None => checks.push((Tone::Info, "No health check has run yet.".to_owned())),
	}

	if let Ok(config) = &state.config {
		checks.push((
			Tone::Info,
			format!(
				"Backups must be younger than {} hours, as [infra.backups] max_age_hours says.",
				config.backups.max_age_hours
			),
		));
	}

	Station::new(
		"Check",
		"health checks and backups",
		"Done means the fleet answers its health checks, not that a command exited.",
		Body::Checks(checks),
	)
}

fn short_id(id: &str) -> &str {
	&id[..id.len().min(8)]
}

/// The plan card's drawer.
pub(crate) fn plan_drawer(state: &ProjectState) -> Drawer {
	let changes = state.plan().map(|plan| plan.changes).unwrap_or_default();
	Drawer {
		id: "plan".to_owned(),
		title: "Plan, review, apply".to_owned(),
		stations: vec![
			declare(state, "[infra] says what should exist."),
			generate(state, &changes),
			plan(state),
			apply("lk infra apply", state.last_apply()),
			check(
				state,
				vec![(
					Tone::Info,
					"Apply refuses a plan made against a different configuration, provider lock, \
					 or state."
						.to_owned(),
				)],
			),
		],
	}
}

/// The drawer for a release action: deploy, first release, maintenance,
/// or roll back.
pub(crate) fn release_drawer(state: &ProjectState) -> Drawer {
	let last = state.serving();
	Drawer {
		id: "release".to_owned(),
		title: "Releases".to_owned(),
		stations: vec![
			declare(state, "Releases run on the hosts [infra] declares."),
			Station::new(
				"Generate",
				"the release engine",
				"A deploy doesn't change infrastructure, so nothing new is generated.",
				Body::Checks(vec![(
					Tone::Info,
					"A git commit is built into an image first; a tag deploys as it is.".to_owned(),
				)]),
			),
			Station::new(
				"Plan",
				"the release engine",
				"Deploys and rollbacks have no plan to review: they move traffic between images.",
				Body::Checks(vec![
					(
						Tone::Info,
						"First release creates the database and runs every migration.".to_owned(),
					),
					(
						Tone::Info,
						"Maintenance tells live connections to come back later.".to_owned(),
					),
					(
						Tone::Warn,
						"Rolling back moves traffic to an older image. It doesn't reverse \
						 database changes."
							.to_owned(),
					),
				]),
			),
			apply("lk deploy --version <tag or commit>", last),
			check(state, Vec::new()),
		],
	}
}

/// The health check's drawer.
pub(crate) fn status_drawer(state: &ProjectState) -> Drawer {
	Drawer {
		id: "status".to_owned(),
		title: "Health check".to_owned(),
		stations: vec![
			declare(state, "The hosts it probes are the ones [infra] declares."),
			Station::new(
				"Generate",
				"nothing",
				"A health check reads; it generates nothing.",
				Body::Checks(Vec::new()),
			),
			Station::new(
				"Plan",
				"nothing",
				"A health check changes nothing, so there's no plan.",
				Body::Checks(Vec::new()),
			),
			apply("lk infra status", state.health()),
			check(state, Vec::new()),
		],
	}
}

/// The backup verification's drawer.
pub(crate) fn backup_drawer(state: &ProjectState) -> Drawer {
	Drawer {
		id: "backup".to_owned(),
		title: "Verify backup".to_owned(),
		stations: vec![
			declare(
				state,
				"[infra.backups] sets the schedule, the retention, and the maximum age.",
			),
			Station::new(
				"Generate",
				"the backup bucket",
				"Backups are encrypted before they leave the host.",
				Body::Checks(Vec::new()),
			),
			Station::new(
				"Plan",
				"nothing",
				"Verifying restores into a scratch database; it changes nothing that's serving.",
				Body::Checks(Vec::new()),
			),
			apply("lk infra backups verify", state.backup()),
			check(state, Vec::new()),
		],
	}
}

/// An operation's own drawer.
pub(crate) fn operation_drawer(state: &ProjectState, operation: &Operation) -> Drawer {
	let command = match &operation.intent.action {
		leptoskit_deploy_console::control::Action::Plan => "lk infra plan".to_owned(),
		leptoskit_deploy_console::control::Action::Apply { .. } => "lk infra apply".to_owned(),
		leptoskit_deploy_console::control::Action::Deploy {
			version,
			setup,
			maintenance,
		} => {
			format!(
				"lk deploy --version {version}{}{}",
				if *setup { " --setup" } else { "" },
				if *maintenance { " --maintenance" } else { "" }
			)
		}
		leptoskit_deploy_console::control::Action::Rollback { version } => {
			format!("lk deploy --rollback {version}")
		}
		leptoskit_deploy_console::control::Action::Status => "lk infra status".to_owned(),
		leptoskit_deploy_console::control::Action::Backup => "lk infra backups verify".to_owned(),
	};
	let changes = super::changes(operation);
	Drawer {
		id: "operation".to_owned(),
		title: super::describe(&operation.intent.action),
		stations: vec![
			declare(state, "Every operation acts on what [infra] declares."),
			generate(state, &changes),
			plan(state),
			apply(&command, Some(operation)),
			check(
				state,
				vec![(
					Tone::Info,
					format!(
						"Follow it from a terminal too: `lk console status` lists it as {}.",
						short_id(&operation.id)
					),
				)],
			),
		],
	}
}
