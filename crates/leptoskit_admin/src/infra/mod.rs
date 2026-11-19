//! The Infrastructure area: what's serving, what will change, and every
//! operation the control service ran, read from its journal and the
//! project's own files. Nothing here is made up: where the admin can't
//! know something (a plan's price, offline), it says so.

mod drawers;
mod pages;

use std::path::Path;

use leptoskit_config::infra::ManagedInfraConfig;
use leptoskit_deploy_console::control::Action;
use leptoskit_deploy_console::control::Operation;
use leptoskit_deploy_console::control::Phase;
use leptoskit_deploy_console::control::Project;
pub(crate) use pages::*;

use crate::components::Line;
use crate::components::Mark;
use crate::components::Tone;

/// One project, with everything its pages read.
pub(crate) struct ProjectState {
	pub project: Project,
	/// This project's operations, newest first.
	pub operations: Vec<Operation>,
	/// Its managed infrastructure, or why it couldn't be read.
	pub config: Result<ManagedInfraConfig, String>,
	/// The `[infra]` tables of its `leptoskit.toml`, as written.
	pub declared: Vec<Line>,
	/// Uncommitted changes to `[infra]`, as a diff.
	pub changed: Vec<Line>,
	/// `infra/<name>/main.tf.json`, when `lk infra generate` has written it.
	pub generated: Option<String>,
}

/// What a plan will do to one resource.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Change {
	pub address: String,
	pub actions: Vec<String>,
}

impl Change {
	/// `create`, `update`, `delete`, `replace`, or `read`.
	pub(crate) fn verb(&self) -> &'static str {
		let has = |action: &str| self.actions.iter().any(|known| known == action);

		if has("create") && has("delete") {
			"replace"
		} else if has("create") {
			"create"
		} else if has("delete") {
			"delete"
		} else if has("update") {
			"update"
		} else {
			"read"
		}
	}

	/// `+`, `~`, `-`, `±`.
	pub(crate) fn symbol(&self) -> &'static str {
		match self.verb() {
			"create" => "+",
			"delete" => "−",
			"replace" => "±",
			"update" => "~",
			_ => "•",
		}
	}

	pub(crate) fn tone(&self) -> Tone {
		match self.verb() {
			"create" => Tone::Good,
			"update" => Tone::Warn,
			"delete" | "replace" => Tone::Bad,
			_ => Tone::Info,
		}
	}

	/// The resource type: `hcloud_server` from `hcloud_server.web[1]`.
	pub(crate) fn resource_type(&self) -> &str {
		let mut parts = self.address.split('.');

		while let Some(part) = parts.next() {
			match part {
				"module" => {
					parts.next();
				}
				"data" => {}
				resource => return resource,
			}
		}

		&self.address
	}

	/// Which group of the diff list it goes in.
	pub(crate) fn kind(&self) -> &'static str {
		let kind = self.resource_type();

		if kind.contains("server") || kind.contains("droplet") || kind.contains("instance") {
			"Compute"
		} else if kind.contains("volume") || kind.contains("bucket") || kind.contains("b2_") {
			"Storage"
		} else if kind.starts_with("cloudflare")
			|| kind.contains("network")
			|| kind.contains("firewall")
			|| kind.contains("ip")
		{
			"Network and edge"
		} else {
			"Other"
		}
	}

	/// Primary hosts, volumes, buckets, and networks hold what can't be
	/// made again.
	pub(crate) fn persistent(&self) -> bool {
		let kind = self.resource_type();
		let primary_host = kind.contains("server")
			&& (self.address.ends_with("[0]") || !self.address.contains('['));
		primary_host
			|| kind.contains("volume")
			|| kind.contains("bucket")
			|| kind.contains("network")
			|| kind.contains("primary_ip")
	}

	/// Deleting or replacing something persistent is refused.
	pub(crate) fn blocked(&self) -> bool {
		self.persistent() && matches!(self.verb(), "delete" | "replace")
	}

	/// The `[infra]` key that causes this resource, for linked highlighting.
	pub(crate) fn key(&self) -> &'static str {
		let kind = self.resource_type();

		if kind.contains("server") || kind.contains("volume") {
			"instances"
		} else if kind.contains("b2") || kind.contains("bucket") {
			"backups"
		} else if kind.starts_with("cloudflare") {
			"domain"
		} else {
			"location"
		}
	}
}

/// A plan, and whether it's still the one to apply.
pub(crate) struct Plan<'a> {
	pub operation: &'a Operation,
	pub changes: Vec<Change>,
	/// A later apply used it.
	pub consumed: bool,
}

impl ProjectState {
	/// Reads a project's files beside its journal.
	pub(crate) fn load(project: Project, operations: &[Operation]) -> Self {
		let mut mine: Vec<Operation> = operations
			.iter()
			.filter(|operation| operation.project == project.id)
			.cloned()
			.collect();
		// The journal supplies newest first. Preserve that order within a
		// second; operation IDs are random, not a release sequence.
		mine.sort_by_key(|operation| std::cmp::Reverse(operation.created));

		let config = leptoskit_config::LeptoskitConfig::load_from(&project.root)
			.map_err(|error| error.to_string())
			.and_then(|config| {
				config
					.infra
					.and_then(leptoskit_config::infra::InfraConfig::into_managed)
					.ok_or_else(|| "leptoskit.toml has no managed [infra] table".to_owned())
			});
		let toml = std::fs::read_to_string(project.root.join("leptoskit.toml")).unwrap_or_default();
		let declared = infra_lines(&toml);
		let changed = infra_diff(&project.root);
		let generated = config.as_ref().ok().and_then(|config| {
			std::fs::read_to_string(
				project
					.root
					.join("infra")
					.join(&config.name)
					.join("main.tf.json"),
			)
			.ok()
		});

		Self {
			project,
			operations: mine,
			config,
			declared,
			changed,
			generated,
		}
	}

	fn latest(&self, matches: impl Fn(&Operation) -> bool) -> Option<&Operation> {
		self.operations.iter().find(|operation| matches(operation))
	}

	/// The operation running or waiting, if any.
	pub(crate) fn active(&self) -> Option<&Operation> {
		self.latest(|operation| matches!(operation.phase, Phase::Queued | Phase::Running))
	}

	/// The last release that reached the fleet: a deploy or a rollback.
	pub(crate) fn serving(&self) -> Option<&Operation> {
		self.latest(|operation| {
			operation.phase == Phase::Succeeded
				&& matches!(
					operation.intent.action,
					Action::Deploy { .. } | Action::Rollback { .. }
				)
		})
	}

	/// The last health check.
	pub(crate) fn health(&self) -> Option<&Operation> {
		self.latest(|operation| {
			matches!(operation.intent.action, Action::Status)
				&& !matches!(operation.phase, Phase::Queued | Phase::Running)
		})
	}

	/// The last backup verification.
	pub(crate) fn backup(&self) -> Option<&Operation> {
		self.latest(|operation| {
			matches!(operation.intent.action, Action::Backup)
				&& !matches!(operation.phase, Phase::Queued | Phase::Running)
		})
	}

	/// The newest plan, finished or not.
	pub(crate) fn last_plan(&self) -> Option<&Operation> {
		self.latest(|operation| matches!(operation.intent.action, Action::Plan))
	}

	/// The newest successful plan, and whether an apply has used it.
	pub(crate) fn plan(&self) -> Option<Plan<'_>> {
		let operation = self.latest(|operation| {
			matches!(operation.intent.action, Action::Plan) && operation.phase == Phase::Succeeded
		})?;
		let consumed = self.operations.iter().any(
			|later| matches!(&later.intent.action, Action::Apply { plan_id } if *plan_id == operation.id),
		);

		Some(Plan {
			operation,
			changes: changes(operation),
			consumed,
		})
	}

	/// The newest apply.
	pub(crate) fn last_apply(&self) -> Option<&Operation> {
		self.latest(|operation| matches!(operation.intent.action, Action::Apply { .. }))
	}

	/// Releases that reached the fleet, newest first, without repeats.
	pub(crate) fn releases(&self) -> Vec<&Operation> {
		let mut seen = std::collections::BTreeSet::new();
		self.operations
			.iter()
			.filter(|operation| {
				operation.phase == Phase::Succeeded
					&& matches!(
						operation.intent.action,
						Action::Deploy { .. } | Action::Rollback { .. }
					)
			})
			.filter(|operation| seen.insert(version(operation).unwrap_or_default().to_owned()))
			.collect()
	}
}

/// The resource changes a plan operation recorded.
pub(crate) fn changes(operation: &Operation) -> Vec<Change> {
	operation.result["changes"]
		.as_array()
		.into_iter()
		.flatten()
		.filter_map(|change| {
			Some(Change {
				address: change["address"].as_str()?.to_owned(),
				actions: change["actions"]
					.as_array()?
					.iter()
					.filter_map(|action| action.as_str().map(str::to_owned))
					.collect(),
			})
		})
		.collect()
}

/// The release an operation deployed or rolled back to.
pub(crate) fn version(operation: &Operation) -> Option<&str> {
	match &operation.intent.action {
		Action::Deploy { version, .. } | Action::Rollback { version } => Some(version),
		_ => None,
	}
}

/// Whether a version is a git commit rather than an image tag.
pub(crate) fn is_commit(version: &str) -> bool {
	matches!(version.len(), 40 | 64) && version.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// What an operation did, in a few words.
pub(crate) fn describe(action: &Action) -> String {
	match action {
		Action::Plan => "Plan changes".to_owned(),
		Action::Apply { .. } => "Apply plan".to_owned(),
		Action::Deploy {
			version,
			setup: true,
			..
		} => format!("First release of {}", short(version)),
		Action::Deploy {
			version,
			maintenance: true,
			..
		} => format!("Deploy {} in maintenance", short(version)),
		Action::Deploy { version, .. } => format!("Deploy {}", short(version)),
		Action::Rollback { version } => format!("Roll back to {}", short(version)),
		Action::Status => "Health check".to_owned(),
		Action::Backup => "Verify backup".to_owned(),
	}
}

/// A commit shortened to 12 characters; tags as they are.
pub(crate) fn short(version: &str) -> &str {
	if is_commit(version) {
		&version[..12]
	} else {
		version
	}
}

/// A phase's word and tone.
pub(crate) fn phase_word(phase: Phase) -> (Tone, &'static str) {
	match phase {
		Phase::Queued => (Tone::Info, "Queued"),
		Phase::Running => (Tone::Info, "Running"),
		Phase::Succeeded => (Tone::Good, "Succeeded"),
		Phase::Failed => (Tone::Bad, "Failed"),
		Phase::Interrupted => (Tone::Warn, "Interrupted"),
	}
}

/// `just now`, `4 min ago`, `3 h ago`, `2 days ago`.
pub(crate) fn ago(unix: u64, now: u64) -> String {
	let seconds = now.saturating_sub(unix);

	match seconds {
		0..60 => "just now".to_owned(),
		60..3600 => format!("{} min ago", seconds / 60),
		3600..86_400 => format!("{} h ago", seconds / 3600),
		_ => {
			let days = seconds / 86_400;
			format!("{days} day{} ago", if days == 1 { "" } else { "s" })
		}
	}
}

/// `2m 14s`.
pub(crate) fn duration(seconds: u64) -> String {
	match seconds {
		0..60 => format!("{seconds}s"),
		60..3600 => format!("{}m {}s", seconds / 60, seconds % 60),
		_ => format!("{}h {}m", seconds / 3600, seconds % 3600 / 60),
	}
}

/// The `[infra]` tables of a `leptoskit.toml`, each line keyed by the
/// setting it holds (`instances`, `backups.daily`).
pub(crate) fn infra_lines(toml: &str) -> Vec<Line> {
	let mut lines = Vec::new();
	let mut table: Option<String> = None;

	for raw in toml.lines() {
		let trimmed = raw.trim();

		if trimmed.starts_with('[') {
			let name = trimmed.trim_matches(|c| c == '[' || c == ']').trim();
			table = (name == "infra" || name.starts_with("infra.")).then(|| {
				name.trim_start_matches("infra")
					.trim_start_matches('.')
					.to_owned()
			});
		}

		let Some(prefix) = &table else {
			continue;
		};
		let mut line = Line::plain(raw);

		if let Some((key, _)) = trimmed.split_once('=')
			&& !trimmed.starts_with('#')
		{
			let key = key.trim();
			line = line.key(if prefix.is_empty() {
				key.to_owned()
			} else {
				format!("{prefix}.{key}")
			});
		}

		lines.push(line);
	}

	while lines.last().is_some_and(|line| line.text.trim().is_empty()) {
		lines.pop();
	}

	lines
}

/// Uncommitted changes to `leptoskit.toml`, as diff lines.
fn infra_diff(root: &Path) -> Vec<Line> {
	let diff = |base: Option<&str>| {
		let mut args = vec!["diff", "--no-color", "--no-ext-diff", "-U2"];
		args.extend(base);
		args.extend(["--", "leptoskit.toml"]);
		std::process::Command::new("git")
			.args(args)
			.current_dir(root)
			.stdin(std::process::Stdio::null())
			.stderr(std::process::Stdio::null())
			.output()
			.ok()
			.filter(|output| output.status.success())
	};

	// A repository with no commits yet has no HEAD; compare with what's
	// staged instead.
	let Some(output) = diff(Some("HEAD")).or_else(|| diff(None)) else {
		return Vec::new();
	};

	String::from_utf8_lossy(&output.stdout)
		.lines()
		.skip_while(|line| !line.starts_with("@@"))
		.filter(|line| !line.starts_with("@@"))
		.map(|line| {
			let (mark, text) = match line.split_at_checked(1) {
				Some(("+", rest)) => (Mark::Added, rest),
				Some(("-", rest)) => (Mark::Removed, rest),
				Some((_, rest)) => (Mark::Plain, rest),
				None => (Mark::Plain, ""),
			};
			let mut diff = Line::marked(
				format!(
					"{}{text}",
					match mark {
						Mark::Added => "+",
						Mark::Removed => "-",
						Mark::Plain => " ",
					}
				),
				mark,
			);

			if let Some((key, _)) = text.trim().split_once('=') {
				diff = diff.key(key.trim().to_owned());
			}

			diff
		})
		.collect()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn infra_lines_keep_only_infra_tables() {
		let lines = infra_lines(
			"[app]\nname = \"notes\"\n\n[infra]\nname = \"notes-production\"\ninstances = \
			 2\n\n[infra.backups]\ndaily = 7\n\n[db]\nurl = \"x\"\n",
		);
		let keys: Vec<_> = lines.iter().filter_map(|line| line.key.clone()).collect();
		assert_eq!(keys, ["name", "instances", "backups.daily"]);
		assert_eq!(lines[0].text, "[infra]");
	}

	#[test]
	fn changes_read_as_verbs() {
		let change = |address: &str, actions: &[&str]| {
			Change {
				address: address.to_owned(),
				actions: actions.iter().map(|action| (*action).to_owned()).collect(),
			}
		};
		assert_eq!(change("hcloud_server.web[1]", &["create"]).verb(), "create");
		assert_eq!(
			change("hcloud_server.web[0]", &["delete", "create"]).verb(),
			"replace"
		);
		assert!(change("hcloud_server.web[0]", &["delete", "create"]).blocked());
		assert!(!change("hcloud_server.web[1]", &["delete"]).blocked());
		assert_eq!(
			change("cloudflare_record.app", &["update"]).kind(),
			"Network and edge"
		);
		assert_eq!(
			change("module.app.hcloud_volume.data", &["create"]).kind(),
			"Storage"
		);
	}
}
