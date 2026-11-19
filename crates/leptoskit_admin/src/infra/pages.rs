//! The Infrastructure screens.

use leptoskit::prelude::*;
use leptoskit_admin_pages::model;
use leptoskit_admin_pages::model::project;
/// A project's tabs.
pub(crate) use leptoskit_admin_pages::model::project::Tab;
use leptoskit_deploy_console::control::Fleet;
use leptoskit_deploy_console::control::Operation;
use leptoskit_deploy_console::control::Phase;

use super::ProjectState;
use super::ago;
use super::describe;
use super::drawers;
use super::duration;
use super::health;
use super::phase_word;
use super::short;
use crate::BASE;
use crate::access::Preferences;
use crate::components;
use crate::components::Kind;
use crate::components::Tone;
use crate::shell::Screen;
use crate::shell::Section;
use crate::style;

/// What a project page needs besides the project.
pub(crate) struct Context<'a> {
	pub now: u64,
	pub preferences: &'a Preferences,
	/// The last hour of fleet measurements, for Health.
	pub fleet: Option<Result<Fleet, String>>,
	/// Whether plans and applies run against a stand-in cloud.
	pub simulated: bool,
	/// Whether a link opened a drawer, so none opens by default.
	pub linked: bool,
}

fn project_href(id: &str, tab: Tab) -> String {
	format!("{BASE}/infra/projects/{id}/{}", tab.slug())
}

fn operation_href(id: &str) -> String {
	format!("{BASE}/infra/operations/{id}")
}

/// Every registered project, as the Projects page shows it.
pub(crate) fn projects(states: &[ProjectState], now: u64) -> Vec<model::ProjectCard> {
	states
		.iter()
		.map(|state| {
			model::ProjectCard {
				id: state.project.id.clone(),
				environment: state.project.environment.clone(),
				serving: state.serving().map_or_else(
					|| "Nothing deployed yet".to_owned(),
					|operation| {
						format!(
							"Serving {} · {}",
							short(super::version(operation).unwrap_or_default()),
							ago(operation.updated, now)
						)
					},
				),
				health: health(state),
				last: state.operations.first().map_or_else(
					|| "No operations yet".to_owned(),
					|operation| {
						format!(
							"{}: {}",
							describe(&operation.intent.action),
							phase_word(operation.phase).1.to_lowercase()
						)
					},
				),
			}
		})
		.collect()
}

/// A project's last health check, in a word.
fn health(state: &ProjectState) -> (Tone, String) {
	let (tone, word) = match state.health() {
		Some(operation) if operation.phase == Phase::Succeeded => (Tone::Good, "Healthy"),
		Some(operation) if operation.phase == Phase::Failed => (Tone::Bad, "Unhealthy"),
		Some(_) => (Tone::Warn, "Check interrupted"),
		None => (Tone::Info, "Not checked"),
	};
	(tone, word.to_owned())
}

/// One tab of one project, for the pages crate to draw. Health isn't one:
/// [`health_screen`] draws it.
pub(crate) fn project(state: &ProjectState, tab: Tab, context: &Context<'_>) -> project::Project {
	let id = &state.project.id;
	let (body, drawers) = match tab {
		Tab::Overview => overview(state, context),
		Tab::Changes => changes(state, context),
		Tab::Releases => releases(state, context.now, context.simulated),
		Tab::Operations | Tab::Health => {
			(
				project::Body::Operations(
					state
						.operations
						.iter()
						.map(|operation| row(operation, context.now))
						.collect(),
				),
				Vec::new(),
			)
		}
		Tab::Backups => backups(state, context.now, context.simulated),
		Tab::Config => config(state),
	};

	project::Project {
		id: id.clone(),
		title: format!("{} · {id}", tab.label()),
		environment: state.project.environment.clone(),
		tab,
		badges: badges(state),
		unreadable: state.config.as_ref().err().cloned(),
		active: state.active().map(|operation| {
			project::Active {
				text: format!(
					"{}: {} · started {}",
					phase_word(operation.phase).1,
					describe(&operation.intent.action),
					ago(operation.created, context.now)
				),
				href: operation_href(&operation.id),
			}
		}),
		simulated: context.simulated,
		body,
		drawers,
	}
}

/// The tab row's badges: the changes planned, and what's running.
fn badges(state: &ProjectState) -> Vec<Option<String>> {
	let mut badges = vec![None; Tab::ALL.len()];

	if let Some(plan) = state.plan()
		&& !plan.consumed
		&& !plan.changes.is_empty()
	{
		badges[1] = Some(plan.changes.len().to_string());
	}

	if state.active().is_some() {
		badges[3] = Some("1 running".to_owned());
	}

	badges
}

fn row(operation: &Operation, now: u64) -> project::Row {
	let (tone, word) = phase_word(operation.phase);
	project::Row {
		tone,
		word: word.to_owned(),
		action: describe(&operation.intent.action),
		actor: operation
			.actor
			.clone()
			.unwrap_or_else(|| "owner key".to_owned()),
		when: ago(operation.created, now),
		href: operation_href(&operation.id),
	}
}

fn action(
	kind: Kind,
	label: impl Into<String>,
	request: project::Request,
	explain: impl Into<String>,
) -> project::Action {
	project::Action {
		kind,
		label: label.into(),
		request,
		explain: explain.into(),
	}
}

/// What a tab shows, and its drawers.
type TabBody = (project::Body, Vec<leptoskit_admin_pages::workings::Drawer>);

fn overview(state: &ProjectState, context: &Context<'_>) -> TabBody {
	let now = context.now;
	let serving = state.serving();
	let release = serving.and_then(super::version);
	let commit = release.filter(|version| super::is_commit(version));
	let budget = state
		.config
		.as_ref()
		.ok()
		.and_then(|config| config.budget.as_ref())
		.map(|budget| format!("{:.2} {} a month", budget.monthly_compute, budget.currency));

	let overview = project::Overview {
		release: release.map(|version| short(version).to_owned()),
		commit: commit.map(|commit| commit[..12].to_owned()),
		deployed_by: serving
			.and_then(|operation| operation.actor.clone())
			.unwrap_or_else(|| "—".to_owned()),
		when: serving.map_or_else(|| "—".to_owned(), |operation| ago(operation.updated, now)),
		health: health(state),
		status: action(
			Kind::Quiet,
			"Run health check",
			project::Request::Status,
			status_explanation(context.simulated),
		),
		recent: state
			.operations
			.iter()
			.take(5)
			.map(|operation| row(operation, now))
			.collect(),
		budget,
		topology: topology(state),
		tour: context.preferences.note("overview-tour"),
	};

	(
		project::Body::Overview(Box::new(overview)),
		vec![
			drawers::release_drawer(state),
			drawers::status_drawer(state),
		],
	)
}

fn topology(state: &ProjectState) -> Option<project::Topology> {
	let config = state.config.as_ref().ok()?;

	Some(project::Topology {
		domain: config.domain.clone(),
		hosts: (0..config.instances.max(1))
			.map(|index| {
				(
					format!("web[{index}]"),
					if index == 0 {
						format!(
							"{} · {} · primary, Postgres",
							config.server_type, config.location
						)
					} else {
						format!("{} · {} · replica", config.server_type, config.location)
					},
				)
			})
			.collect(),
		backups: config
			.backups
			.enabled
			.then(|| format!("encrypted · {}", config.backups.schedule)),
		aside: format!("from [infra] · {:?}", config.compute).to_lowercase(),
	})
}

/// What the plan confirmation says before `lk infra plan` runs.
fn plan_explanation(simulated: bool) -> &'static str {
	if simulated {
		"Simulated: compares the configuration with a stand-in cloud. Planning changes nothing, \
		 and no provider is contacted."
	} else {
		"Runs `lk infra plan`. Planning changes nothing."
	}
}

/// What the apply confirmation says before exactly this plan runs.
fn apply_explanation(changes: usize, simulated: bool) -> String {
	let plural = if changes == 1 { "" } else { "s" };

	if simulated {
		format!(
			"Simulated: applies exactly this plan to a stand-in cloud. {changes} change{plural} \
			 to the stand-in; no provider is contacted and no real infrastructure changes."
		)
	} else {
		format!(
			"Applies exactly this plan with `lk infra apply`. Apply refuses it if the \
			 configuration, provider lock, or state changed since it was made. {changes} \
			 change{plural} to real infrastructure."
		)
	}
}

fn status_explanation(simulated: bool) -> &'static str {
	if simulated {
		"Simulated: returns a sample health result; no provider is contacted and no real host is \
		 probed."
	} else {
		"Runs `lk infra status`: it probes every host and changes nothing."
	}
}

/// A button that asks first: the script shows the confirm sheet with
/// `explain` before sending the action.
fn confirmable(kind: Kind, label: &str, action: &str, explain: &str) -> Element {
	let button = Button(label)
		.class(kind.class_name())
		.test_id(action.to_owned())
		.aria_description(explain);

	if kind == Kind::Primary {
		components::raised(button)
	} else {
		button
	}
}

fn changes(state: &ProjectState, context: &Context<'_>) -> TabBody {
	let now = context.now;
	let plan = state.plan();
	let failed = state
		.last_plan()
		.filter(|newest| {
			newest.phase != Phase::Succeeded
				&& !matches!(newest.phase, Phase::Queued | Phase::Running)
				&& plan
					.as_ref()
					.is_none_or(|plan| newest.created >= plan.operation.created)
		})
		.map(|failed| {
			project::Failed {
				when: format!("The last plan failed {}", ago(failed.updated, now)),
				message: failed.message.clone(),
				href: operation_href(&failed.id),
			}
		});

	let shown = match &plan {
		None => project::Plan::None,
		Some(plan) if plan.consumed => {
			project::Plan::Applied(format!(
				"The plan from {} was applied. Plan again to see what's changed since.",
				ago(plan.operation.updated, now)
			))
		}
		Some(plan) if plan.changes.is_empty() => project::Plan::Clean,
		Some(plan) => {
			let count = |verb: &str| {
				plan.changes
					.iter()
					.filter(|change| change.verb() == verb)
					.count()
			};
			let mut groups: Vec<project::Group> = Vec::new();

			for change in &plan.changes {
				let row = project::PlanRow {
					key: change.key().to_owned(),
					label: format!("{} {}", change.symbol(), change.verb()),
					verb: change.verb().to_owned(),
					tone: change.tone(),
					address: change.address.clone(),
					persistent: change.persistent(),
					blocker: change.blocked().then(|| {
						format!(
							"✕ persistent_resource_deletion_blocked: {} holds data that can't be \
							 made again, so Apply won't {} it. Put the [infra] value back, or \
							 move the data first and remove it by hand.",
							change.address,
							change.verb()
						)
					}),
				};

				match groups.iter_mut().find(|group| group.kind == change.kind()) {
					Some(group) => group.rows.push(row),
					None => {
						groups.push(project::Group {
							kind: change.kind().to_owned(),
							rows: vec![row],
						});
					}
				}
			}

			project::Plan::Changes {
				summary: format!(
					"{} to create, {} to update, {} to delete{}",
					count("create"),
					count("update"),
					count("delete"),
					match count("replace") {
						0 => String::new(),
						n => format!(", {n} to replace"),
					}
				),
				planned: format!(
					"Planned {} by {}.",
					ago(plan.operation.updated, now),
					plan.operation
						.actor
						.clone()
						.unwrap_or_else(|| "the owner key".to_owned())
				),
				groups,
			}
		}
	};

	let pending = plan
		.as_ref()
		.filter(|plan| !plan.consumed && !plan.changes.is_empty());
	let apply = match pending {
		None => project::Apply::None,
		Some(plan) if plan.changes.iter().any(super::Change::blocked) => project::Apply::Blocked,
		Some(plan) => {
			let count = plan.changes.len();
			project::Apply::Ready(action(
				Kind::Primary,
				format!("Apply {count} change{}", if count == 1 { "" } else { "s" }),
				project::Request::Apply {
					plan_id: plan.operation.id.clone(),
				},
				apply_explanation(count, context.simulated),
			))
		}
	};
	let replan = if pending.is_some() {
		action(
			Kind::Secondary,
			"Plan again",
			project::Request::Plan,
			plan_explanation(context.simulated),
		)
	} else {
		action(
			Kind::Primary,
			"Plan changes",
			project::Request::Plan,
			plan_explanation(context.simulated),
		)
	};

	let changes = project::Changes {
		failed,
		plan: shown,
		apply,
		replan,
		review_note: context.preferences.note("plan-review"),
		// Workings opens here by default the first three times.
		workings_by_default: !context.linked && context.preferences.note("plan-workings"),
	};

	(
		project::Body::Changes(Box::new(changes)),
		vec![drawers::plan_drawer(state)],
	)
}

fn releases(state: &ProjectState, now: u64, simulated: bool) -> TabBody {
	let serving = state.serving().and_then(super::version).map(str::to_owned);
	let deploy = vec![
		action(
			Kind::Primary,
			"Deploy",
			project::Request::Deploy,
			if simulated {
				"Simulated: records a release on the stand-in cloud; no provider is contacted, no \
				 image is built, and no real traffic moves."
			} else {
				"Runs `lk deploy --version <version>`: it starts the new image, waits for health \
				 checks, then moves traffic."
			},
		),
		action(
			Kind::Secondary,
			"First release",
			project::Request::FirstRelease,
			if simulated {
				"Simulated: records a first release on the stand-in cloud; no provider is \
				 contacted, no database is created, and no migration runs."
			} else {
				"Runs `lk deploy --setup`: it creates the database and runs every migration before \
				 the first image starts. Use it once, for a new project."
			},
		),
		action(
			Kind::Secondary,
			"Deploy in maintenance",
			project::Request::Maintenance,
			if simulated {
				"Simulated: records a maintenance release on the stand-in cloud; no provider is \
				 contacted and visitors' connections stay open."
			} else {
				"Runs `lk deploy --maintenance`: visitors see the maintenance page and live \
				 connections are told to reconnect later."
			},
		),
	];
	let rollbacks = state
		.releases()
		.into_iter()
		.filter(|operation| super::version(operation).map(str::to_owned) != serving)
		.take(5)
		.map(|operation| {
			let version = super::version(operation).unwrap_or_default();
			project::Rollback {
				version: short(version).to_owned(),
				when: ago(operation.updated, now),
				action: action(
					Kind::Danger,
					format!("Roll back to {}", short(version)),
					project::Request::Rollback {
						version: version.to_owned(),
					},
					if simulated {
						format!(
							"Simulated: records a rollback to {} on the stand-in cloud; no \
							 provider is contacted and no real traffic moves.",
							short(version)
						)
					} else {
						format!(
							"Moves traffic back to {}. Database changes made since stay as they \
							 are.",
							short(version)
						)
					},
				),
			}
		})
		.collect();

	(
		project::Body::Releases(project::Releases {
			deploy,
			rollbacks,
			history: state
				.releases()
				.into_iter()
				.map(|operation| row(operation, now))
				.collect(),
		}),
		vec![drawers::release_drawer(state)],
	)
}

/// The Health tab, which the admin still draws itself: its charts move
/// with the analytics pages.
pub(crate) fn health_screen(state: &ProjectState, context: &Context<'_>) -> Screen {
	let id = &state.project.id;
	let badges = badges(state);
	let items: Vec<(String, String, Option<String>)> = Tab::ALL
		.into_iter()
		.zip(badges)
		.map(|(tab, badge)| (tab.label().to_owned(), project_href(id, tab), badge))
		.collect();
	let selected = Tab::ALL
		.iter()
		.position(|known| *known == Tab::Health)
		.unwrap_or(0);

	let mut page = View()
		.class("flex-col gap-5")
		.child(
			View()
				.class("flex-row flex-wrap items-center gap-2")
				.child(components::link(
					"Projects",
					format!("{BASE}/infra/projects"),
				))
				.child(Text("/").class(style::MUTED))
				.child(Text(id.clone()).class(style::SECONDARY)),
		)
		.child(
			View()
				.class("flex-row flex-wrap items-center gap-3")
				.child(Heading(1, id.clone()).class(style::PAGE_TITLE))
				.child(Text(state.project.environment.clone()).class(style::CHIP)),
		)
		.child(components::tabs(&items, selected));

	if let Err(reason) = &state.config {
		page = page.child(
			Status(format!(
				"The admin can't read this project's [infra]: {reason}. Actions still go to the \
				 control service, which reads the file itself."
			))
			.class("text-sm text-[var(--admin-warn)]"),
		);
	}

	if let Some(operation) = state.active() {
		let (_, word) = phase_word(operation.phase);
		page = page.child(
			View()
				.class(
					"flex-row flex-wrap items-center gap-3 rounded-xl border \
					 border-[var(--admin-line-strong)] bg-[var(--admin-sunken)] px-4 py-3",
				)
				.test_id("active")
				.child(
					View()
						.class("h-2.5 w-2.5 rounded-full bg-[var(--admin-ember)] animate-pulse")
						.aria_hidden(true),
				)
				.child(
					Text(format!(
						"{word}: {} · started {}",
						describe(&operation.intent.action),
						ago(operation.created, context.now)
					))
					.class("flex-1 text-sm font-semibold text-[var(--admin-ink)]"),
				)
				.child(components::link("Follow it", operation_href(&operation.id))),
		);
	}

	let (tone, word) = health(state);
	let mut body = vec![
		components::card("Health check")
			.workings("status", "lk infra status")
			.child(components::status_word(tone, word))
			.child(
				Text(state.health().map_or_else(
					|| "No health check has run yet.".to_owned(),
					|operation| {
						format!(
							"{} ({})",
							operation.message,
							ago(operation.updated, context.now)
						)
					},
				))
				.class(style::SECONDARY),
			)
			.child(confirmable(
				Kind::Secondary,
				"Run health check",
				"action:status",
				status_explanation(context.simulated),
			))
			.build(),
	];

	match &context.fleet {
		Some(Ok(fleet)) => body.extend(health::section(fleet, 500)),
		Some(Err(error)) => {
			body.push(
				Status(format!(
					"The control service couldn't read measurements: {error}"
				))
				.class("text-sm text-[var(--admin-bad)]"),
			);
		}
		None => {}
	}

	Screen {
		title: format!("{} · {id}", Tab::Health.label()),
		section: Section::ProjectHealth(id.clone()),
		content: page.children(body),
		drawers: vec![drawers::status_drawer(state)],
		open: None,
	}
}

fn backups(state: &ProjectState, now: u64, simulated: bool) -> TabBody {
	let schedule = match &state.config {
		Ok(config) if config.backups.enabled => {
			let backups = &config.backups;
			project::Schedule::On {
				schedule: backups.schedule.clone(),
				daily: backups.daily.into(),
				weekly: backups.weekly.into(),
				monthly: backups.monthly.into(),
				max_age_hours: backups.max_age_hours.into(),
			}
		}
		Ok(_) => project::Schedule::Off,
		Err(_) => project::Schedule::Unknown,
	};
	let last = match state.backup() {
		Some(operation) => {
			let (tone, word) = phase_word(operation.phase);
			(
				tone,
				format!(
					"Last verification {} {}, took {}",
					word.to_lowercase(),
					ago(operation.updated, now),
					duration(operation.updated.saturating_sub(operation.created))
				),
			)
		}
		None => {
			(
				Tone::Info,
				"Never verified: a backup you haven't restored is a hope".to_owned(),
			)
		}
	};

	(
		project::Body::Backups(project::Backups {
			schedule,
			last,
			verify: action(
				Kind::Primary,
				"Verify last backup",
				project::Request::Backup,
				if simulated {
					"Simulated: returns a sample backup verification; no provider is contacted and \
					 no backup is restored or checked."
				} else {
					"Runs `lk infra backups verify`: it restores the latest backup into a scratch \
					 database and checks it. Nothing that's serving changes."
				},
			),
		}),
		vec![drawers::backup_drawer(state)],
	)
}

/// What each `[infra]` key means, in one line.
fn explain(key: &str) -> Option<&'static str> {
	Some(match key {
		"name" => "Prefix for every resource. Use a different one per environment.",
		"domain" => "The public hostname, on your Cloudflare zone.",
		"location" => "The provider's data centre, such as hel1.",
		"server_type" => "The provider's size for each host, such as cx23.",
		"instances" => "How many hosts. The first keeps Postgres; the rest are replicas.",
		"compute" => "Which provider runs the hosts.",
		"backups.enabled" => "Whether encrypted backups run.",
		"backups.schedule" => "When backups run.",
		"backups.daily" | "backups.weekly" | "backups.monthly" => {
			"How many backups of that age to keep."
		}
		"backups.max_age_hours" => "How old the newest backup may be before checks fail.",
		"budget.monthly_compute" => {
			"The most a plan may cost a month for compute; plan refuses more."
		}
		"budget.currency" => "The budget's currency.",
		_ => return None,
	})
}

fn config(state: &ProjectState) -> TabBody {
	let explained = state
		.declared
		.iter()
		.filter_map(|line| {
			let key = line.key.as_ref()?;
			Some((key.clone(), explain(key)?.to_owned()))
		})
		.collect();

	(
		project::Body::Config(project::Config {
			declared: state.declared.clone(),
			explained,
		}),
		Vec::new(),
	)
}

/// One operation, as its page shows it.
pub(crate) fn operation(state: &ProjectState, operation: &Operation, now: u64) -> model::Operation {
	let running = matches!(operation.phase, Phase::Queued | Phase::Running);
	let took = if running {
		format!(
			"running for {}",
			duration(now.saturating_sub(operation.created))
		)
	} else {
		format!(
			"took {}",
			duration(operation.updated.saturating_sub(operation.created))
		)
	};
	let (tone, word) = phase_word(operation.phase);

	model::Operation {
		id: operation.id.clone(),
		project: state.project.id.clone(),
		title: describe(&operation.intent.action),
		phase: (tone, word.to_owned()),
		started: format!(
			"Started by {} · {} · {took}",
			operation
				.actor
				.clone()
				.unwrap_or_else(|| "the owner key".to_owned()),
			ago(operation.created, now)
		),
		running,
		steps: operation
			.events
			.iter()
			.map(|event| {
				let (tone, word) = phase_word(event.phase);
				model::Step {
					sequence: event.sequence,
					message: event.message.clone(),
					tone,
					word: word.to_owned(),
					when: ago(event.created, now),
				}
			})
			.collect(),
		result: operation.message.clone(),
		changes: super::changes(operation)
			.iter()
			.map(|change| {
				model::ChangeRow {
					label: format!("{} {}", change.symbol(), change.verb()),
					tone: change.tone(),
					address: change.address.clone(),
				}
			})
			.collect(),
		drawer: drawers::operation_drawer(state, operation),
	}
}
