//! A project's tabs: Overview, Changes, Releases, Operations, Backups and
//! Config, and the actions they offer, each asked through the confirm
//! sheet before it runs.

use leptos::prelude::GetValue as _;
use leptos::prelude::StoredValue;
use leptoskit::native::FocusRequest;
use leptoskit::native::FontWeight;
use leptoskit::native::TextSpan;
use leptoskit::prelude::*;

use crate::BASE;
use crate::components;
use crate::components::Kind;
use crate::components::Tone;
use crate::confirm::Ask;
use crate::confirm::Confirm;
use crate::model::project::Action;
use crate::model::project::Apply;
use crate::model::project::Backups;
use crate::model::project::Body;
use crate::model::project::Changes;
use crate::model::project::Config;
use crate::model::project::Overview;
use crate::model::project::Plan;
use crate::model::project::Project;
use crate::model::project::Releases;
use crate::model::project::Request;
use crate::model::project::Row;
use crate::model::project::Schedule;
use crate::model::project::Tab;
use crate::model::project::Topology;
use crate::style;
use crate::workings::Workings;

fn project_href(id: &str, tab: Tab) -> String {
	format!("{BASE}/infra/projects/{id}/{}", tab.slug())
}

/// What a project page's actions change while it runs in the browser.
#[derive(Clone, Copy)]
pub(crate) struct Actions {
	project: StoredValue<String>,
	/// The version typed under Deploy.
	version: RwSignal<String>,
	/// Takes focus to the version field.
	version_field: FocusRequest,
	/// The test id of the action on its way to the server.
	busy: RwSignal<Option<String>>,
	confirm: Confirm,
	notice: RwSignal<String>,
}

impl Actions {
	/// The actions of `project`'s page, asking through `confirm` and
	/// saying what went wrong in `notice`.
	pub(crate) fn new(project: &str, confirm: Confirm, notice: RwSignal<String>) -> Self {
		Self {
			project: StoredValue::new(project.to_owned()),
			version: RwSignal::new(String::new()),
			version_field: FocusRequest::new(),
			busy: RwSignal::new(None),
			confirm,
			notice,
		}
	}

	/// Asks first, then sends `action`.
	fn run(self, action: &Action, from: FocusRequest) {
		if self.busy.get_untracked().is_some() {
			return;
		}

		if action.request.typed_version() && self.version.with_untracked(|v| v.trim().is_empty()) {
			self.notice
				.set("Type an image tag or a git commit first.".to_owned());
			self.version_field.request();
			return;
		}

		let request = action.request.clone();
		let ask = Ask::new(
			action.label.trim_end_matches('…').trim(),
			action.explain.clone(),
		);
		self.confirm
			.ask(ask, Some(from), move |_| self.send(request.clone()));
	}

	/// Sends `request` to the admin, and opens the operation it starts.
	fn send(self, request: Request) {
		self.busy.set(Some(request.test_id()));
		let version = self.version.get_untracked().trim().to_owned();
		let body = request.body(&version, &crate::idempotency_key());
		let path = format!("{BASE}/api/projects/{}/actions", self.project.get_value());

		crate::spawn(async move {
			let answer = crate::post_json(&path, &body).await;
			let href = answer
				.as_ref()
				.filter(|(ok, _)| *ok)
				.and_then(|(_, data)| data["href"].as_str().map(str::to_owned));

			if let Some(href) = href {
				if matches!(request, Request::Plan | Request::Apply { .. }) {
					crate::shell::preference(r#"{"kind":"use_note","note":"plan-review"}"#);
				}
				crate::go(&href);
				return;
			}

			self.busy.set(None);
			self.notice.set(
				answer
					.and_then(|(_, data)| data["message"].as_str().map(str::to_owned))
					.unwrap_or_else(|| {
						"The admin couldn't reach the server. Nothing was sent.".to_owned()
					}),
			);
		});
	}
}

/// A button that asks first, then sends its request.
fn action_button(actions: Option<Actions>, action: &Action) -> Element {
	let id = action.request.test_id();
	let busy = actions.is_some_and(|actions| {
		actions
			.busy
			.with(|busy| busy.as_deref() == Some(id.as_str()))
	});
	let label = if busy {
		format!("{}…", action.label.trim_end_matches('…'))
	} else {
		action.label.clone()
	};
	let mut button = Button(label)
		.class(action.kind.class_name())
		.test_id(id)
		.aria_description(action.explain.clone());

	if let Some(actions) = actions {
		let here = FocusRequest::new();
		let action = action.clone();
		button = button
			.aria_busy(busy)
			.focus_on(here)
			.on_press(move || actions.run(&action, here));
	}

	if action.kind == Kind::Primary {
		components::raised(button)
	} else {
		button
	}
}

fn operation_row(row: &Row) -> Element {
	Pressable(format!("{}: {}", row.action, row.word))
		.href(row.href.clone())
		.class("cursor-pointer rounded-lg hover:bg-[var(--admin-sunken)]")
		.test_id("operation-row")
		.child(
			View()
				.class(
					"flex-row flex-wrap items-center gap-x-3 gap-y-0.5 border-b \
					 border-[var(--admin-line)] px-1 py-2.5",
				)
				.child(components::status_word(row.tone, row.word.clone()).class("w-28 shrink-0"))
				.child(
					Text(row.action.clone())
						.class("min-w-0 flex-1 truncate text-sm text-[var(--admin-ink)]"),
				)
				.child(Text(row.actor.clone()).class(
					"max-w-[200px] truncate text-xs text-[var(--admin-ink-3)] max-sm:hidden",
				))
				.child(
					Text(row.when.clone())
						.class("w-24 text-end text-xs text-[var(--admin-ink-3)] tabular-nums"),
				),
		)
}

/// Says plainly that this admin's infrastructure is a stand-in, on the
/// screens where a change starts.
fn simulated_notice() -> Element {
	View()
		.class("rounded-xl border border-[var(--admin-line-strong)] px-4 py-2.5")
		.test_id("simulated-notice")
		.child(
			Text("")
				.spans([
					TextSpan::new("Simulated: ").weight(FontWeight::Semibold),
					TextSpan::new(
						"plans and applies run against a stand-in cloud; no provider is contacted.",
					),
				])
				.class(style::SECONDARY),
		)
}

/// One tab of one project.
pub(crate) fn page(project: &Project, actions: Option<Actions>) -> Element {
	let id = &project.id;
	let items: Vec<(String, String, Option<String>)> = Tab::ALL
		.into_iter()
		.zip(
			project
				.badges
				.iter()
				.cloned()
				.chain(std::iter::repeat(None)),
		)
		.map(|(tab, badge)| (tab.label().to_owned(), project_href(id, tab), badge))
		.collect();
	let selected = Tab::ALL
		.iter()
		.position(|known| *known == project.tab)
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
				.child(Text(project.environment.clone()).class(style::CHIP)),
		)
		.child(components::tabs(&items, selected));

	if let Some(reason) = &project.unreadable {
		page = page.child(
			Status(format!(
				"The admin can't read this project's [infra]: {reason}. Actions still go to the \
				 control service, which reads the file itself."
			))
			.class("text-sm text-[var(--admin-warn)]"),
		);
	}

	if let Some(active) = &project.active {
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
					Text(active.text.clone())
						.class("flex-1 text-sm font-semibold text-[var(--admin-ink)]"),
				)
				.child(components::link("Follow it", active.href.clone())),
		);
	}

	if project.simulated && matches!(project.tab, Tab::Overview | Tab::Changes) {
		page = page.child(simulated_notice());
	}

	page.children(match &project.body {
		Body::Overview(overview) => self::overview(id, overview, actions),
		Body::Changes(changes) => self::changes(changes, actions),
		Body::Releases(releases) => self::releases(releases, actions),
		Body::Operations(rows) => operations(rows),
		Body::Backups(backups) => self::backups(backups, actions),
		Body::Config(config) => self::config(config),
	})
}

fn overview(id: &str, overview: &Overview, actions: Option<Actions>) -> Vec<Element> {
	let facts = View()
		.class("grid grid-cols-5 gap-4 max-lg:grid-cols-3 max-sm:grid-cols-2")
		.child(components::fact(
			"Release",
			Text(
				overview
					.release
					.clone()
					.unwrap_or_else(|| "Nothing yet".to_owned()),
			)
			.class("text-lg font-semibold text-[var(--admin-ink)]")
			.test_id("serving-release"),
		))
		.child(components::fact(
			"Commit",
			components::code(overview.commit.clone().unwrap_or_else(|| "—".to_owned())),
		))
		.child(components::fact(
			"Deployed by",
			Text(overview.deployed_by.clone())
				.class(style::BODY)
				.class("truncate"),
		))
		.child(components::fact(
			"When",
			Text(overview.when.clone()).class(style::BODY),
		))
		.child(components::fact(
			"Health",
			components::status_word(overview.health.0, overview.health.1.clone()),
		));

	let buttons = View()
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
		.child(action_button(actions, &overview.status));

	let now_serving = components::card("Now serving")
		.workings("release", "release engine → hosts")
		.child(facts)
		.child(buttons)
		.build();

	let mut timeline = View().class("flex-col").test_id("recent");

	if overview.recent.is_empty() {
		timeline = timeline.child(
			Text("Nothing has run yet. Plan changes is the first step.").class(style::MUTED),
		);
	}

	timeline = timeline.children(overview.recent.iter().map(operation_row));

	let recent = components::card("Recent operations")
		.child(timeline)
		.child(components::link(
			"All operations",
			project_href(id, Tab::Operations),
		))
		.build();

	let budget = match &overview.budget {
		Some(budget) => {
			components::card("Budget")
				.child(
					Text(budget.clone())
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
				.child(topology(overview.topology.as_ref())),
		);

	let mut body = Vec::new();

	if overview.tour {
		body.push(components::margin_note(
			"overview-tour",
			"This is what's serving. Changes is where a change starts, and Releases is how you \
			 undo one.",
		));
	}

	body.push(now_serving);
	body.push(grid);
	body
}

fn topology(topology: Option<&Topology>) -> Element {
	let node = |title: &str, detail: &str| {
		View()
			.class(
				"min-w-[140px] flex-col gap-1 rounded-[10px] border \
				 border-[var(--admin-line-strong)] bg-[var(--admin-surface)] px-3 py-2",
			)
			.test_id("node")
			.child(Text(title.to_owned()).class("text-sm font-semibold text-[var(--admin-ink)]"))
			.child(Text(detail.to_owned()).class(style::LABEL))
	};
	let arrow = || {
		Text("→")
			.class("text-[var(--admin-ink-3)] max-sm:hidden")
			.aria_hidden(true)
	};

	let Some(topology) = topology else {
		return components::card("Topology")
			.child(Text("Drawn from [infra] once the admin can read it.").class(style::MUTED))
			.build();
	};

	let hosts = View().class("flex-col gap-2").children(
		topology
			.hosts
			.iter()
			.map(|(title, detail)| node(title, detail)),
	);

	let mut row = View()
		.class("flex-row flex-wrap items-center gap-3")
		.child(node(&topology.domain, "Cloudflare DNS"))
		.child(arrow())
		.child(node("Tunnel", "no open ports"))
		.child(arrow())
		.child(hosts);

	if let Some(backups) = &topology.backups {
		row = row.child(arrow()).child(node("Backups", backups));
	}

	components::card("Topology")
		.aside(topology.aside.clone())
		.child(View().class("overflow-scroll").child(row))
		.build()
}

fn changes(changes: &Changes, actions: Option<Actions>) -> Vec<Element> {
	let workings = Workings::current();
	let mut card = components::card("Plan").workings("plan", "[infra] → OpenTofu plan");

	if let Some(failed) = &changes.failed {
		card = card.child(
			View()
				.class("flex-col gap-2 rounded-[10px] border border-[var(--admin-bad)] px-3 py-2.5")
				.test_id("plan-failed")
				.child(components::status_word(Tone::Bad, failed.when.clone()))
				.child(Text(failed.message.clone()).class(style::SECONDARY))
				.child(
					components::prose(
						"Plans read the real provider accounts, so the machine running the worker \
						 needs their tokens. Run `lk infra plan` in the project to see the full \
						 output; the admin keeps raw tool output off the page.",
					)
					.class(style::MUTED),
				)
				.child(components::link("See the operation", failed.href.clone())),
		);
	}

	match &changes.plan {
		Plan::None => {
			card = card.child(
				Text(
					"Plan compares what [infra] declares with what exists, and lists the \
					 difference. It changes nothing; Apply runs exactly the plan you reviewed.",
				)
				.class(style::SECONDARY),
			);
		}
		Plan::Applied(text) => {
			card = card.child(Text(text.clone()).class(style::SECONDARY));
		}
		Plan::Clean => {
			card = card.child(components::status_word(
				Tone::Good,
				"No changes: what's running matches leptoskit.toml",
			));
		}
		Plan::Changes {
			summary,
			planned,
			groups,
		} => {
			card = card
				.child(
					Text(summary.clone())
						.class("text-lg font-semibold text-[var(--admin-ink)]")
						.test_id("plan-summary"),
				)
				.child(Text(planned.clone()).class(style::MUTED));

			let mut list = View()
				.class("flex-col gap-3 overflow-scroll")
				.test_id("diff");

			for group in groups {
				list = list.child(Text(group.kind.to_uppercase()).class(style::GROUP_LABEL));

				for change in &group.rows {
					let mut row = View()
						.class(
							"flex-row items-center gap-3 rounded-lg border \
							 border-[var(--admin-line)] px-3 py-2 min-w-[420px]",
						)
						.child(
							Text(change.label.clone())
								.class("w-24 shrink-0 text-sm font-semibold")
								.class(change.tone.text()),
						)
						.child(
							Text(change.address.clone())
								.class(style::CODE)
								.class("min-w-0 flex-1 truncate"),
						);

					if change.persistent {
						row = row.child(
							Text("🔒 keeps data").class("text-xs text-[var(--admin-ink-3)]"),
						);
					}

					let mut pressable = Pressable(format!(
						"{} {}: show the [infra] key that causes it",
						change.verb, change.address
					))
					.class("cursor-pointer rounded-lg hover:bg-[var(--admin-sunken)]")
					.test_id(format!("plan-row:{}", change.key));

					if let Some(workings) = workings {
						let here = FocusRequest::new();
						let key = change.key.clone();

						if workings.is_linked(&change.key) {
							pressable = pressable.class(style::LINKED);
						}

						pressable = pressable
							.focus_on(here)
							.on_press(move || workings.link(&key, Some(here)));
					}

					list = list.child(pressable.child(row));

					if let Some(blocker) = &change.blocker {
						list = list.child(
							Text(blocker.clone())
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

	match &changes.apply {
		Apply::None => {}
		Apply::Blocked => {
			buttons = buttons.child(
				Text("Apply is off: this plan deletes something that keeps data.")
					.class("text-sm font-semibold text-[var(--admin-bad)]"),
			);
		}
		Apply::Ready(apply) => buttons = buttons.child(action_button(actions, apply)),
	}

	card = card.child(buttons.child(action_button(actions, &changes.replan)));

	let mut body = Vec::new();

	if changes.review_note {
		body.push(components::margin_note(
			"plan-review",
			"Plan never changes anything. Read the rows, press one to see the line in \
			 leptoskit.toml that causes it, then Apply runs exactly this plan.",
		));
	}

	body.push(card.build());
	body
}

fn releases(releases: &Releases, actions: Option<Actions>) -> Vec<Element> {
	let mut field = TextInput()
		.placeholder("v42 or a 40-character commit")
		.aria_label("Image tag or git commit")
		.class(
			"rounded-lg border border-[var(--admin-line-strong)] bg-[var(--admin-surface)] px-3 \
			 py-2 text-sm text-[var(--admin-ink)] max-w-[420px]",
		)
		.test_id("version:deploy");

	if let Some(actions) = actions {
		field = field
			.value(actions.version.get())
			.focus_on(actions.version_field)
			.on_change(move |text| actions.version.set(text.to_owned()));
	}

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
				.child(field),
		)
		.child(
			View()
				.class("flex-row flex-wrap items-center gap-3")
				.children(
					releases
						.deploy
						.iter()
						.map(|action| action_button(actions, action)),
				),
		)
		.build();

	let mut rollback = components::card("Roll back").child(
		Text(
			"Rolling back moves traffic to an older image. It doesn't reverse database changes, \
			 so it's for app problems, not data ones.",
		)
		.class(style::SECONDARY),
	);

	if releases.rollbacks.is_empty() {
		rollback = rollback.child(Text("No earlier release to roll back to.").class(style::MUTED));
	}

	for earlier in &releases.rollbacks {
		rollback = rollback.child(
			View()
				.class("flex-row flex-wrap items-center gap-3")
				.child(components::code(earlier.version.clone()).class("flex-1"))
				.child(Text(earlier.when.clone()).class(style::MUTED))
				.child(action_button(actions, &earlier.action)),
		);
	}

	vec![
		deploy,
		rollback.build(),
		components::card("History")
			.child(
				View()
					.class("flex-col")
					.children(releases.history.iter().map(operation_row)),
			)
			.build(),
	]
}

fn operations(rows: &[Row]) -> Vec<Element> {
	let mut list = View().class("flex-col").test_id("operations");

	if rows.is_empty() {
		list = list.child(Text("Nothing has run on this project yet.").class(style::MUTED));
	}

	vec![
		components::card("Operations")
			.aside("newest first")
			.child(list.children(rows.iter().map(operation_row)))
			.build(),
	]
}

fn backups(backups: &Backups, actions: Option<Actions>) -> Vec<Element> {
	let mut card = components::card("Backups").workings("backup", "[infra.backups]");

	match &backups.schedule {
		Schedule::On {
			schedule,
			daily,
			weekly,
			monthly,
			max_age_hours,
		} => {
			card = card
				.child(
					View()
						.class("grid grid-cols-4 gap-4 max-sm:grid-cols-2")
						.child(components::fact(
							"Schedule",
							components::code(schedule.clone()),
						))
						.child(components::fact(
							"Daily",
							Text(daily.to_string()).class(style::BODY),
						))
						.child(components::fact(
							"Weekly",
							Text(weekly.to_string()).class(style::BODY),
						))
						.child(components::fact(
							"Monthly",
							Text(monthly.to_string()).class(style::BODY),
						)),
				)
				.child(
					Text(format!(
						"A backup older than {max_age_hours} hours fails the check."
					))
					.class(style::MUTED),
				);
		}
		Schedule::Off => {
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
		Schedule::Unknown => {
			card = card.child(Text("Shown once the admin can read [infra].").class(style::MUTED));
		}
	}

	vec![
		card.child(components::status_word(
			backups.last.0,
			backups.last.1.clone(),
		))
		.child(action_button(actions, &backups.verify))
		.build(),
	]
}

fn config(config: &Config) -> Vec<Element> {
	if config.declared.is_empty() {
		return vec![components::empty_state(
			"No [infra] to show",
			"This project's leptoskit.toml has no managed [infra] table the admin can read.",
			None,
			"Add one, and this page shows it with each setting explained.",
		)];
	}

	let explained = View()
		.class("flex-col gap-2")
		.test_id("explained")
		.children(config.explained.iter().map(|(key, explanation)| {
			View()
				.class("flex-row flex-wrap gap-x-3 gap-y-0.5")
				.child(components::code(key.clone()).class("w-48 shrink-0"))
				.child(
					Text(explanation.clone())
						.class("min-w-0 flex-1")
						.class(style::SECONDARY),
				)
		}));

	vec![
		components::card("[infra]")
			.aside("leptoskit.toml, read-only")
			.child(components::code_block(&config.declared, "TOML"))
			.child(
				Text(
					"Edit leptoskit.toml and push, then plan from Changes. The admin never writes \
					 the file.",
				)
				.class(style::MUTED),
			)
			.build(),
		components::card("What each setting does")
			.child(explained)
			.build(),
	]
}
