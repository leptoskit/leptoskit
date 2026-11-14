//! The Infrastructure screens.

use leptoskit::prelude::*;
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

/// A project's tabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tab {
	Overview,
	Changes,
	Releases,
	Operations,
	Health,
	Backups,
	Config,
}

impl Tab {
	const ALL: [Self; 7] = [
		Self::Overview,
		Self::Changes,
		Self::Releases,
		Self::Operations,
		Self::Health,
		Self::Backups,
		Self::Config,
	];

	pub(crate) fn parse(value: &str) -> Option<Self> {
		Self::ALL.into_iter().find(|tab| tab.slug() == value)
	}

	pub(crate) fn slug(self) -> &'static str {
		match self {
			Self::Overview => "overview",
			Self::Changes => "changes",
			Self::Releases => "releases",
			Self::Operations => "operations",
			Self::Health => "health",
			Self::Backups => "backups",
			Self::Config => "config",
		}
	}

	fn label(self) -> &'static str {
		match self {
			Self::Overview => "Overview",
			Self::Changes => "Changes",
			Self::Releases => "Releases",
			Self::Operations => "Operations",
			Self::Health => "Health",
			Self::Backups => "Backups",
			Self::Config => "Config",
		}
	}
}

/// What a project page needs besides the project.
pub(crate) struct Context<'a> {
	pub now: u64,
	pub preferences: &'a Preferences,
	/// The last hour of fleet measurements, for Health.
	pub fleet: Option<Result<Fleet, String>>,
}

fn project_href(id: &str, tab: Tab) -> String {
	format!("{BASE}/infra/projects/{id}/{}", tab.slug())
}

fn operation_href(id: &str) -> String {
	format!("{BASE}/infra/operations/{id}")
}

fn page_header(title: &str, lead: Option<&str>) -> Element {
	let mut header = View()
		.class("flex-col gap-1")
		.child(Heading(1, title.to_owned()).class(style::PAGE_TITLE));

	if let Some(lead) = lead {
		header = header.child(Text(lead.to_owned()).class(style::SECONDARY));
	}

	header
}

/// Every registered project.
pub(crate) fn projects(states: &[ProjectState], now: u64) -> Screen {
	let mut content = View().class("flex-col gap-5").child(page_header(
		"Projects",
		Some("Every project the control service runs, and what each is serving."),
	));

	if states.is_empty() {
		content = content.child(components::empty_state(
			"No projects yet",
			"A project is a repository with a managed [infra] table in its leptoskit.toml. Add \
			 this to yours, then start the admin with --project pointing at the repository:",
			Some((
				"[infra]\nname = \"notes-production\"\ndomain = \"notes.example\"\nlocation = \
				 \"hel1\"\nserver_type = \"cx23\"\ninstances = 1",
				"TOML",
			)),
			"Plan changes appears on the project as soon as the admin can read the table.",
		));
	}

	let mut grid = View()
		.class("grid grid-cols-2 gap-4 max-md:grid-cols-1")
		.test_id("grid-main");

	for state in states {
		let id = &state.project.id;
		let serving = state.serving().map_or_else(
			|| "Nothing deployed yet".to_owned(),
			|operation| {
				format!(
					"Serving {} · {}",
					short(super::version(operation).unwrap_or_default()),
					ago(operation.updated, now)
				)
			},
		);
		let health = health_word(state);
		let last = state.operations.first().map_or_else(
			|| "No operations yet".to_owned(),
			|operation| {
				format!(
					"{}: {}",
					describe(&operation.intent.action),
					phase_word(operation.phase).1.to_lowercase()
				)
			},
		);

		grid = grid.child(
			Pressable(format!("Open {id}"))
				.href(project_href(id, Tab::Overview))
				.class("cursor-pointer rounded-xl hover:bg-[var(--admin-sunken)]")
				.test_id(format!("project:{id}"))
				.child(
					View()
						.class(style::CARD)
						.child(
							View()
								.class("flex-row flex-wrap items-center gap-2")
								.child(Heading(2, id.clone()).class(style::CARD_TITLE))
								.child(Text(state.project.environment.clone()).class(style::CHIP)),
						)
						.child(Text(serving).class(style::BODY))
						.child(health)
						.child(Text(last).class(style::MUTED)),
				),
		);
	}

	Screen {
		title: "Projects".to_owned(),
		section: Section::Projects,
		content: content.child(grid),
		drawers: Vec::new(),
		open: None,
	}
}

fn health_word(state: &ProjectState) -> Element {
	match state.health() {
		Some(operation) if operation.phase == Phase::Succeeded => {
			components::status_word(Tone::Good, "Healthy")
		}
		Some(operation) if operation.phase == Phase::Failed => {
			components::status_word(Tone::Bad, "Unhealthy")
		}
		Some(_) => components::status_word(Tone::Warn, "Check interrupted"),
		None => components::status_word(Tone::Info, "Not checked"),
	}
}

fn active_banner(state: &ProjectState, now: u64) -> Option<Element> {
	let operation = state.active()?;
	let (_, word) = phase_word(operation.phase);
	Some(
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
					ago(operation.created, now)
				))
				.class("flex-1 text-sm font-semibold text-[var(--admin-ink)]"),
			)
			.child(components::link("Follow it", operation_href(&operation.id))),
	)
}

/// A project screen.
pub(crate) fn project(state: &ProjectState, tab: Tab, context: &Context<'_>) -> Screen {
	let id = &state.project.id;
	let mut badges: [Option<String>; 7] = Default::default();

	if let Some(plan) = state.plan()
		&& !plan.consumed
		&& !plan.changes.is_empty()
	{
		badges[1] = Some(plan.changes.len().to_string());
	}

	if state.active().is_some() {
		badges[3] = Some("1 running".to_owned());
	}

	let items: Vec<(String, String, Option<String>)> = Tab::ALL
		.into_iter()
		.zip(badges)
		.map(|(tab, badge)| (tab.label().to_owned(), project_href(id, tab), badge))
		.collect();
	let selected = Tab::ALL.iter().position(|known| *known == tab).unwrap_or(0);

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

	page = page.children(active_banner(state, context.now));

	let (body, drawers, open) = match tab {
		Tab::Overview => overview(state, context),
		Tab::Changes => changes(state, context),
		Tab::Releases => releases(state, context.now),
		Tab::Operations => (operations(state, context.now), Vec::new(), None),
		Tab::Health => health_tab(state, context),
		Tab::Backups => backups(state, context.now),
		Tab::Config => config(state),
	};

	Screen {
		title: format!("{} · {id}", tab.label()),
		section: match tab {
			Tab::Health => Section::ProjectHealth(id.clone()),
			Tab::Operations => Section::ProjectOperations(id.clone()),
			_ => Section::Project(id.clone()),
		},
		content: page.children(body),
		drawers,
		open,
	}
}

type Body = (Vec<Element>, Vec<crate::workings::Drawer>, Option<String>);

fn overview(state: &ProjectState, context: &Context<'_>) -> Body {
	let id = &state.project.id;
	let now = context.now;
	let serving = state.serving();
	let release = serving.and_then(super::version);
	let commit = release.filter(|version| super::is_commit(version));

	let facts = View()
		.class("grid grid-cols-5 gap-4 max-lg:grid-cols-3 max-sm:grid-cols-2")
		.child(components::fact(
			"Release",
			Text(release.map_or_else(|| "Nothing yet".to_owned(), |v| short(v).to_owned()))
				.class("text-lg font-semibold text-[var(--admin-ink)]")
				.test_id("serving-release"),
		))
		.child(components::fact(
			"Commit",
			components::code(commit.map_or_else(|| "—".to_owned(), |c| c[..12].to_owned())),
		))
		.child(components::fact(
			"Deployed by",
			Text(
				serving
					.and_then(|operation| operation.actor.clone())
					.unwrap_or_else(|| "—".to_owned()),
			)
			.class(style::BODY)
			.class("truncate"),
		))
		.child(components::fact(
			"When",
			Text(serving.map_or_else(|| "—".to_owned(), |operation| ago(operation.updated, now)))
				.class(style::BODY),
		))
		.child(components::fact("Health", health_word(state)));

	let actions = View()
		.class("flex-row flex-wrap items-center gap-3")
		.child(components::link_button(
			Kind::Primary,
			"Deploy…",
			project_href(id, Tab::Releases),
		))
		.child(components::link_button(
			Kind::Secondary,
			"Roll back…",
			project_href(id, Tab::Releases),
		))
		.child(confirmable(
			Kind::Quiet,
			"Run health check",
			"action:status",
			"Runs `lk infra status`: it probes every host and changes nothing.",
		));

	let now_serving = components::card("Now serving")
		.workings("release", "release engine → hosts")
		.child(facts)
		.child(actions)
		.build();

	let mut timeline = View().class("flex-col").test_id("recent");

	if state.operations.is_empty() {
		timeline = timeline.child(
			Text("Nothing has run yet. Plan changes is the first step.").class(style::MUTED),
		);
	}

	for operation in state.operations.iter().take(5) {
		timeline = timeline.child(operation_row(operation, now));
	}

	let recent = components::card("Recent operations")
		.child(timeline)
		.child(components::link(
			"All operations",
			project_href(id, Tab::Operations),
		))
		.build();

	let budget = match state
		.config
		.as_ref()
		.ok()
		.and_then(|config| config.budget.as_ref())
	{
		Some(budget) => {
			components::card("Budget")
				.child(
					Text(format!(
						"{:.2} {} a month",
						budget.monthly_compute, budget.currency
					))
					.class(style::HERO_NUMBER)
					.class("text-2xl"),
				)
				.child(components::bar(0.0, Some(0.8)))
				.child(
					Text(
						"The limit [infra.budget] sets for compute and the primary IPv4. Plan \
						 prices the plan with live provider prices and refuses one over the \
						 limit. Excludes tax, traffic, storage, and domains.",
					)
					.class(style::MUTED),
				)
				.build()
		}
		None => {
			components::card("Budget")
				.child(
					Text("No budget set. Plan refuses changes over a budget once you set one:")
						.class(style::SECONDARY),
				)
				.child(components::paper_code(
					"[infra.budget]\ncurrency = \"EUR\"\nmonthly_compute = 20.0",
					"TOML",
				))
				.build()
		}
	};

	let topology = topology(state);

	let grid = View()
		.class("grid grid-cols-12 gap-4 max-lg:grid-cols-6")
		.test_id("grid-main")
		.child(
			View()
				.class("col-span-7 max-lg:col-span-6 flex-col")
				.child(recent),
		)
		.child(
			View()
				.class("col-span-5 max-lg:col-span-6 flex-col")
				.child(budget),
		)
		.child(
			View()
				.class("col-span-12 max-lg:col-span-6 flex-col")
				.child(topology),
		);

	let mut body = vec![now_serving, grid];

	if context.preferences.note("overview-tour") {
		body.insert(
			0,
			components::margin_note(
				"overview-tour",
				"This is what's serving. Changes is where a change starts, and Releases is how \
				 you undo one.",
			),
		);
	}

	(
		body,
		vec![
			drawers::release_drawer(state),
			drawers::status_drawer(state),
		],
		None,
	)
}

fn topology(state: &ProjectState) -> Element {
	let node = |title: String, detail: String| {
		View()
			.class(
				"min-w-[140px] flex-col gap-1 rounded-[10px] border \
				 border-[var(--admin-line-strong)] bg-[var(--admin-surface)] px-3 py-2",
			)
			.test_id("node")
			.child(Text(title).class("text-sm font-semibold text-[var(--admin-ink)]"))
			.child(Text(detail).class(style::LABEL))
	};
	let arrow = || {
		Text("→")
			.class("text-[var(--admin-ink-3)] max-sm:hidden")
			.aria_hidden(true)
	};

	let Ok(config) = &state.config else {
		return components::card("Topology")
			.child(Text("Drawn from [infra] once the admin can read it.").class(style::MUTED))
			.build();
	};

	let mut hosts = View().class("flex-col gap-2");

	for index in 0..config.instances.max(1) {
		hosts = hosts.child(node(
			format!("web[{index}]"),
			if index == 0 {
				format!(
					"{} · {} · primary, Postgres",
					config.server_type, config.location
				)
			} else {
				format!("{} · {} · replica", config.server_type, config.location)
			},
		));
	}

	let mut row = View()
		.class("flex-row flex-wrap items-center gap-3")
		.child(node(config.domain.clone(), "Cloudflare DNS".to_owned()))
		.child(arrow())
		.child(node("Tunnel".to_owned(), "no open ports".to_owned()))
		.child(arrow())
		.child(hosts);

	if config.backups.enabled {
		row = row.child(arrow()).child(node(
			"Backups".to_owned(),
			format!("encrypted · {}", config.backups.schedule),
		));
	}

	components::card("Topology")
		.aside(format!("from [infra] · {:?}", config.compute).to_lowercase())
		.child(View().class("overflow-scroll").child(row))
		.build()
}

fn operation_row(operation: &Operation, now: u64) -> Element {
	let (tone, word) = phase_word(operation.phase);
	Pressable(format!("{}: {word}", describe(&operation.intent.action)))
		.href(operation_href(&operation.id))
		.class("cursor-pointer rounded-lg hover:bg-[var(--admin-sunken)]")
		.test_id("operation-row")
		.child(
			View()
				.class(
					"flex-row flex-wrap items-center gap-x-3 gap-y-0.5 border-b \
					 border-[var(--admin-line)] px-1 py-2.5",
				)
				.child(components::status_word(tone, word).class("w-28 shrink-0"))
				.child(
					Text(describe(&operation.intent.action))
						.class("min-w-0 flex-1 truncate text-sm text-[var(--admin-ink)]"),
				)
				.child(
					Text(
						operation
							.actor
							.clone()
							.unwrap_or_else(|| "owner key".to_owned()),
					)
					.class(
						"max-w-[200px] truncate text-xs text-[var(--admin-ink-3)] max-sm:hidden",
					),
				)
				.child(
					Text(ago(operation.created, now))
						.class("w-24 text-end text-xs text-[var(--admin-ink-3)] tabular-nums"),
				),
		)
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

#[expect(
	clippy::too_many_lines,
	reason = "the plan card reads top to bottom as the page does: failure, summary, groups, \
	          blockers, actions"
)]
fn changes(state: &ProjectState, context: &Context<'_>) -> Body {
	let now = context.now;
	let plan = state.plan();
	let failed_after = state.last_plan().filter(|newest| {
		newest.phase != Phase::Succeeded
			&& !matches!(newest.phase, Phase::Queued | Phase::Running)
			&& plan
				.as_ref()
				.is_none_or(|plan| newest.created >= plan.operation.created)
	});

	let mut card = components::card("Plan").workings("plan", "[infra] → OpenTofu plan");

	if let Some(failed) = failed_after {
		card = card.child(
			View()
				.class("flex-col gap-2 rounded-[10px] border border-[var(--admin-bad)] px-3 py-2.5")
				.test_id("plan-failed")
				.child(components::status_word(
					Tone::Bad,
					format!("The last plan failed {}", ago(failed.updated, now)),
				))
				.child(Text(failed.message.clone()).class(style::SECONDARY))
				.child(
					components::prose(
						"Plans read the real provider accounts, so the machine running the worker \
						 needs their tokens. Run `lk infra plan` in the project to see the full \
						 output; the admin keeps raw tool output off the page.",
					)
					.class(style::MUTED),
				)
				.child(components::link(
					"See the operation",
					operation_href(&failed.id),
				)),
		);
	}

	match &plan {
		None => {
			card = card.child(
				Text(
					"Plan compares what [infra] declares with what exists, and lists the \
					 difference. It changes nothing; Apply runs exactly the plan you reviewed.",
				)
				.class(style::SECONDARY),
			);
		}
		Some(plan) if plan.consumed => {
			card = card.child(
				Text(format!(
					"The plan from {} was applied. Plan again to see what's changed since.",
					ago(plan.operation.updated, now)
				))
				.class(style::SECONDARY),
			);
		}
		Some(plan) if plan.changes.is_empty() => {
			card = card.child(components::status_word(
				Tone::Good,
				"No changes: what's running matches leptoskit.toml",
			));
		}
		Some(plan) => {
			let count = |verb: &str| {
				plan.changes
					.iter()
					.filter(|change| change.verb() == verb)
					.count()
			};
			let summary = format!(
				"{} to create, {} to update, {} to delete{}",
				count("create"),
				count("update"),
				count("delete"),
				match count("replace") {
					0 => String::new(),
					n => format!(", {n} to replace"),
				}
			);
			card = card.child(
				Text(summary)
					.class("text-lg font-semibold text-[var(--admin-ink)]")
					.test_id("plan-summary"),
			);
			card = card.child(
				Text(format!(
					"Planned {} by {}.",
					ago(plan.operation.updated, now),
					plan.operation
						.actor
						.clone()
						.unwrap_or_else(|| "the owner key".to_owned())
				))
				.class(style::MUTED),
			);

			let mut groups: Vec<(&str, Vec<&super::Change>)> = Vec::new();

			for change in &plan.changes {
				match groups.iter_mut().find(|(kind, _)| *kind == change.kind()) {
					Some((_, list)) => list.push(change),
					None => groups.push((change.kind(), vec![change])),
				}
			}

			let mut list = View()
				.class("flex-col gap-3 overflow-scroll")
				.test_id("diff");

			for (kind, changes) in groups {
				list = list.child(Text(kind.to_uppercase()).class(style::GROUP_LABEL));

				for change in changes {
					let mut row = View()
						.class(
							"flex-row items-center gap-3 rounded-lg border \
							 border-[var(--admin-line)] px-3 py-2 min-w-[420px]",
						)
						.child(
							Text(format!("{} {}", change.symbol(), change.verb()))
								.class("w-24 shrink-0 text-sm font-semibold")
								.class(change.tone().text()),
						)
						.child(
							Text(change.address.clone())
								.class(style::CODE)
								.class("min-w-0 flex-1 truncate"),
						);

					if change.persistent() {
						row = row.child(
							Text("🔒 keeps data").class("text-xs text-[var(--admin-ink-3)]"),
						);
					}

					list = list.child(
						Pressable(format!(
							"{} {}: show the [infra] key that causes it",
							change.verb(),
							change.address
						))
						.class("cursor-pointer rounded-lg hover:bg-[var(--admin-sunken)]")
						.test_id(format!("plan-row:{}", change.key()))
						.child(row),
					);

					if change.blocked() {
						list = list.child(
							Text(format!(
								"✕ persistent_resource_deletion_blocked: {} holds data that can't \
								 be made again, so Apply won't {} it. Put the [infra] value back, \
								 or move the data first and remove it by hand.",
								change.address,
								change.verb()
							))
							.class("text-sm text-[var(--admin-bad)]")
							.test_id("blocker"),
						);
					}
				}
			}

			card = card.child(list);
		}
	}

	let mut buttons = View().class("flex-row flex-wrap items-center gap-3");

	if let Some(plan) = plan
		.as_ref()
		.filter(|plan| !plan.consumed && !plan.changes.is_empty())
	{
		if plan.changes.iter().any(super::Change::blocked) {
			buttons = buttons.child(
				Text("Apply is off: this plan deletes something that keeps data.")
					.class("text-sm font-semibold text-[var(--admin-bad)]"),
			);
		} else {
			buttons = buttons.child(confirmable(
				Kind::Primary,
				&format!(
					"Apply {} change{}",
					plan.changes.len(),
					if plan.changes.len() == 1 { "" } else { "s" }
				),
				&format!("action:apply:{}", plan.operation.id),
				&format!(
					"Applies exactly this plan with `lk infra apply`. Apply refuses it if the \
					 configuration, provider lock, or state changed since it was made. {} \
					 change{} to real infrastructure.",
					plan.changes.len(),
					if plan.changes.len() == 1 { "" } else { "s" }
				),
			));
		}

		buttons = buttons.child(confirmable(
			Kind::Secondary,
			"Plan again",
			"action:plan",
			"Runs `lk infra plan`. Planning changes nothing.",
		));
	} else {
		buttons = buttons.child(confirmable(
			Kind::Primary,
			"Plan changes",
			"action:plan",
			"Runs `lk infra plan`. Planning changes nothing.",
		));
	}

	card = card.child(buttons);

	let mut body = Vec::new();

	if context.preferences.note("plan-review") {
		body.push(components::margin_note(
			"plan-review",
			"Plan never changes anything. Read the rows, press one to see the line in \
			 leptoskit.toml that causes it, then Apply runs exactly this plan.",
		));
	}

	body.push(card.build());

	// Workings opens here by default the first three times.
	let open = context
		.preferences
		.note("plan-workings")
		.then(|| "plan".to_owned());

	(body, vec![drawers::plan_drawer(state)], open)
}

fn releases(state: &ProjectState, now: u64) -> Body {
	let serving = state.serving().and_then(super::version).map(str::to_owned);

	let deploy = components::card("Deploy")
		.workings("release", "release engine → hosts")
		.child(
			Text(
				"Deploy an image tag, or a git commit the worker builds first. It waits for the \
				 health checks before moving traffic, and keeps the old browser assets so open \
				 tabs keep working.",
			)
			.class(style::SECONDARY),
		)
		.child(
			View()
				.class("flex-col gap-1.5")
				.child(Text("Image tag or git commit").class(style::LABEL))
				.child(
					TextInput()
						.placeholder("v42 or a 40-character commit")
						.aria_label("Image tag or git commit")
						.class(
							"rounded-lg border border-[var(--admin-line-strong)] \
							 bg-[var(--admin-surface)] px-3 py-2 text-sm text-[var(--admin-ink)] \
							 max-w-[420px]",
						)
						.test_id("version:deploy"),
				),
		)
		.child(
			View()
				.class("flex-row flex-wrap items-center gap-3")
				.child(confirmable(
					Kind::Primary,
					"Deploy",
					"action:deploy",
					"Runs `lk deploy --version <version>`: it starts the new image, waits for \
					 health checks, then moves traffic.",
				))
				.child(confirmable(
					Kind::Secondary,
					"First release",
					"action:first-release",
					"Runs `lk deploy --setup`: it creates the database and runs every migration \
					 before the first image starts. Use it once, for a new project.",
				))
				.child(confirmable(
					Kind::Secondary,
					"Deploy in maintenance",
					"action:maintenance",
					"Runs `lk deploy --maintenance`: visitors see the maintenance page and live \
					 connections are told to reconnect later.",
				)),
		)
		.build();

	let mut rollback = components::card("Roll back").child(
		Text(
			"Rolling back moves traffic to an older image. It doesn't reverse database changes, \
			 so it's for app problems, not data ones.",
		)
		.class(style::SECONDARY),
	);
	let earlier: Vec<&Operation> = state
		.releases()
		.into_iter()
		.filter(|operation| super::version(operation).map(str::to_owned) != serving)
		.take(5)
		.collect();

	if earlier.is_empty() {
		rollback = rollback.child(Text("No earlier release to roll back to.").class(style::MUTED));
	}

	for operation in earlier {
		let version = super::version(operation).unwrap_or_default();
		rollback = rollback.child(
			View()
				.class("flex-row flex-wrap items-center gap-3")
				.child(components::code(short(version).to_owned()).class("flex-1"))
				.child(Text(ago(operation.updated, now)).class(style::MUTED))
				.child(confirmable(
					Kind::Danger,
					&format!("Roll back to {}", short(version)),
					&format!("action:rollback:{version}"),
					&format!(
						"Moves traffic back to {}. Database changes made since stay as they are.",
						short(version)
					),
				)),
		);
	}

	let mut history = View().class("flex-col");

	for operation in state.releases() {
		history = history.child(operation_row(operation, now));
	}

	(
		vec![
			deploy,
			rollback.build(),
			components::card("History").child(history).build(),
		],
		vec![drawers::release_drawer(state)],
		None,
	)
}

fn operations(state: &ProjectState, now: u64) -> Vec<Element> {
	let mut list = View().class("flex-col").test_id("operations");

	if state.operations.is_empty() {
		list = list.child(Text("Nothing has run on this project yet.").class(style::MUTED));
	}

	for operation in &state.operations {
		list = list.child(operation_row(operation, now));
	}

	vec![
		components::card("Operations")
			.aside("newest first")
			.child(list)
			.build(),
	]
}

fn health_tab(state: &ProjectState, context: &Context<'_>) -> Body {
	let mut body = vec![
		components::card("Health check")
			.workings("status", "lk infra status")
			.child(health_word(state))
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
				"Runs `lk infra status`: it probes every host and changes nothing.",
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

	(body, vec![drawers::status_drawer(state)], None)
}

fn backups(state: &ProjectState, now: u64) -> Body {
	let mut card = components::card("Backups").workings("backup", "[infra.backups]");

	match &state.config {
		Ok(config) if config.backups.enabled => {
			let backups = &config.backups;
			card = card
				.child(
					View()
						.class("grid grid-cols-4 gap-4 max-sm:grid-cols-2")
						.child(components::fact(
							"Schedule",
							components::code(backups.schedule.clone()),
						))
						.child(components::fact(
							"Daily",
							Text(backups.daily.to_string()).class(style::BODY),
						))
						.child(components::fact(
							"Weekly",
							Text(backups.weekly.to_string()).class(style::BODY),
						))
						.child(components::fact(
							"Monthly",
							Text(backups.monthly.to_string()).class(style::BODY),
						)),
				)
				.child(
					Text(format!(
						"A backup older than {} hours fails the check.",
						backups.max_age_hours
					))
					.class(style::MUTED),
				);
		}
		Ok(_) => {
			card = card
				.child(
					Text("Backups are off for this project. Turn them on in [infra.backups]:")
						.class(style::SECONDARY),
				)
				.child(components::paper_code(
					"[infra.backups]\nenabled = true",
					"TOML",
				));
		}
		Err(_) => {
			card = card.child(Text("Shown once the admin can read [infra].").class(style::MUTED));
		}
	}

	card = card.child(match state.backup() {
		Some(operation) => {
			let (tone, word) = phase_word(operation.phase);
			components::status_word(
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
			components::status_word(
				Tone::Info,
				"Never verified: a backup you haven't restored is a hope",
			)
		}
	});

	card = card.child(confirmable(
		Kind::Primary,
		"Verify last backup",
		"action:backup",
		"Runs `lk infra backups verify`: it restores the latest backup into a scratch database \
		 and checks it. Nothing that's serving changes.",
	));

	(
		vec![card.build()],
		vec![drawers::backup_drawer(state)],
		None,
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

fn config(state: &ProjectState) -> Body {
	if state.declared.is_empty() {
		return (
			vec![components::empty_state(
				"No [infra] to show",
				"This project's leptoskit.toml has no managed [infra] table the admin can read.",
				None,
				"Add one, and this page shows it with each setting explained.",
			)],
			Vec::new(),
			None,
		);
	}

	let mut explained = View().class("flex-col gap-2").test_id("explained");

	for line in &state.declared {
		if let Some(key) = &line.key
			&& let Some(explanation) = explain(key)
		{
			explained = explained.child(
				View()
					.class("flex-row flex-wrap gap-x-3 gap-y-0.5")
					.child(components::code(key.clone()).class("w-48 shrink-0"))
					.child(
						Text(explanation)
							.class("min-w-0 flex-1")
							.class(style::SECONDARY),
					),
			);
		}
	}

	(
		vec![
			components::card("[infra]")
				.aside("leptoskit.toml, read-only")
				.child(components::code_block(&state.declared, "TOML"))
				.child(
					Text(
						"Edit leptoskit.toml and push, then plan from Changes. The admin never \
						 writes the file.",
					)
					.class(style::MUTED),
				)
				.build(),
			components::card("What each setting does")
				.child(explained)
				.build(),
		],
		Vec::new(),
		None,
	)
}

/// One operation and its timeline.
pub(crate) fn operation(state: &ProjectState, operation: &Operation, now: u64) -> Screen {
	let (tone, word) = phase_word(operation.phase);
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

	let header = View()
		.class("flex-col gap-2")
		.child(
			View()
				.class("flex-row flex-wrap items-center gap-2")
				.child(components::link(
					state.project.id.clone(),
					project_href(&state.project.id, Tab::Operations),
				))
				.child(Text("/").class(style::MUTED))
				.child(
					Text(operation.id.chars().take(8).collect::<String>()).class(style::SECONDARY),
				),
		)
		.child(Heading(1, describe(&operation.intent.action)).class(style::PAGE_TITLE))
		.child(
			View()
				.class("flex-row flex-wrap items-center gap-3")
				.child(components::status_word(tone, word).test_id("phase"))
				.child(
					Text(format!(
						"Started by {} · {} · {took}",
						operation
							.actor
							.clone()
							.unwrap_or_else(|| "the owner key".to_owned()),
						ago(operation.created, now)
					))
					.class(style::SECONDARY),
				),
		);

	let mut timeline = View().class("flex-col").test_id("timeline");

	for (index, event) in operation.events.iter().enumerate() {
		let (tone, word) = phase_word(event.phase);
		let last = index + 1 == operation.events.len();
		timeline = timeline.child(
			View()
				.class("relative flex-row gap-3 pb-4 pl-6")
				.test_id("event")
				.child(
					View()
						.class("absolute left-[5px] top-3 bottom-0 w-0.5 bg-[var(--admin-line)]")
						.class(if last { "hidden" } else { "" })
						.aria_hidden(true),
				)
				.child(
					View()
						.class(
							"absolute left-0 top-1 h-3 w-3 rounded-full border-2 \
							 border-[var(--admin-ink)]",
						)
						.class(if last && running {
							"bg-[var(--admin-ember)] animate-pulse"
						} else {
							"bg-[var(--admin-surface)]"
						})
						.aria_hidden(true),
				)
				.child(
					View()
						.class("min-w-0 flex-1 flex-col gap-0.5")
						.child(Text(event.message.clone()).class(style::BODY))
						.child(
							Text(format!("{word} · {}", ago(event.created, now)))
								.class("text-xs")
								.class(tone.text()),
						),
				),
		);
	}

	let mut card = components::card("Timeline")
		.workings("operation", "lk console status")
		.child(timeline);

	if running {
		card = card.child(
			Status("Following live. Closing this tab cancels nothing.")
				.class("text-sm text-[var(--admin-ink-2)]")
				.test_id("live"),
		);
	} else {
		card = card.child(
			Text(operation.message.clone())
				.class(style::SECONDARY)
				.test_id("result"),
		);
	}

	let changes = super::changes(operation);
	let mut body = vec![header, card.build()];

	if !changes.is_empty() {
		let mut list = View().class("flex-col gap-1.5");

		for change in &changes {
			list = list.child(
				View()
					.class("flex-row gap-3")
					.child(
						Text(format!("{} {}", change.symbol(), change.verb()))
							.class("w-24 shrink-0 text-sm font-semibold")
							.class(change.tone().text()),
					)
					.child(
						Text(change.address.clone())
							.class(style::CODE)
							.class("truncate"),
					),
			);
		}

		body.push(
			components::card("What the plan found")
				.child(list)
				.child(components::link(
					"Review it under Changes",
					project_href(&state.project.id, Tab::Changes),
				))
				.build(),
		);
	}

	Screen {
		title: describe(&operation.intent.action),
		section: Section::ProjectOperations(state.project.id.clone()),
		content: View().class("flex-col gap-5").children(body),
		drawers: vec![drawers::operation_drawer(state, operation)],
		open: None,
	}
}
