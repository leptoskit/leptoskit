//! The Infrastructure pages this crate builds: every project, and one
//! operation followed live while it runs.

use leptos::prelude::Effect;
use leptoskit::prelude::*;
use leptoskit::rpc::client::StreamHandle;
use leptoskit::rpc::client::StreamState;

use crate::BASE;
use crate::components;
use crate::model::Operation;
use crate::model::ProjectCard;
use crate::model::Step;
use crate::ops;
use crate::ops::Progress;
use crate::style;

/// A project's Overview.
fn overview_href(id: &str) -> String {
	format!("{BASE}/infra/projects/{id}/overview")
}

fn page_header(title: &str, lead: Option<&str>) -> Element {
	View()
		.class("flex-col gap-1")
		.child(Heading(1, title.to_owned()).class(style::PAGE_TITLE))
		.children(lead.map(|lead| Text(lead.to_owned()).class(style::SECONDARY)))
}

/// Every registered project, each a card that opens it.
pub(crate) fn projects(cards: &[ProjectCard]) -> Element {
	let mut content = View().class("flex-col gap-5").child(page_header(
		"Projects",
		Some("Every project the control service runs, and what each is serving."),
	));

	if cards.is_empty() {
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

	let grid = View()
		.class("grid grid-cols-2 gap-4 max-md:grid-cols-1")
		.test_id("grid-main")
		.children(cards.iter().map(|card| {
			Pressable(format!("Open {}", card.id))
				.href(overview_href(&card.id))
				.class("cursor-pointer rounded-xl hover:bg-[var(--admin-sunken)]")
				.test_id(format!("project:{}", card.id))
				.child(
					View()
						.class(style::CARD)
						.child(
							View()
								.class("flex-row flex-wrap items-center gap-2")
								.child(Heading(2, card.id.clone()).class(style::CARD_TITLE))
								.child(Text(card.environment.clone()).class(style::CHIP)),
						)
						.child(Text(card.serving.clone()).class(style::BODY))
						.child(components::status_word(
							card.health.0,
							card.health.1.clone(),
						))
						.child(Text(card.last.clone()).class(style::MUTED)),
				)
		}));

	content.child(grid)
}

/// Follows a running operation: its steps as they happen, and a fresh page
/// once it finishes, which shows how it ended.
#[derive(Clone, Copy)]
pub(crate) struct Follow {
	progress: StreamHandle<Progress>,
}

impl Follow {
	/// Opens the operation's progress stream, in the browser.
	pub(crate) fn open(id: &str) -> Self {
		#[cfg(feature = "hydrate")]
		leptoskit::rpc::client::configure(
			leptoskit::rpc::client::ClientConfig::new()
				.rpc_path(format!("{BASE}/rpc"))
				.live_path(format!("{BASE}/live")),
		);

		let id = id.to_owned();
		let progress = leptoskit::rpc::use_stream({
			let id = id.clone();
			move || ops::OperationProgress { id: id.clone() }
		});
		let page = format!("{BASE}/infra/operations/{id}");

		Effect::new(move |_| {
			let done = progress
				.items()
				.with(|items| items.iter().any(|item| *item == Progress::Done));

			if done {
				crate::go(&page);
			}
		});

		Self { progress }
	}

	/// The steps that arrived after `known`, which the page already shows.
	fn steps(&self, known: u64) -> Vec<Step> {
		self.progress.items().with(|items| {
			let mut steps: Vec<Step> = Vec::new();

			for item in items {
				if let Progress::Step {
					sequence,
					message,
					tone,
					word,
				} = item
				{
					let sequence = sequence.to_owned();
					let seen =
						sequence <= known || steps.iter().any(|step| step.sequence == sequence);

					if !seen {
						steps.push(Step {
							sequence,
							message: message.clone(),
							tone: tone.to_owned(),
							word: word.clone(),
							when: "just now".to_owned(),
						});
					}
				}
			}

			steps
		})
	}

	/// Whether the stream went before the operation finished.
	fn lost(&self) -> bool {
		self.progress
			.state()
			.with(|state| matches!(state, StreamState::Failed(_) | StreamState::Interrupted))
	}
}

/// One operation: its timeline, followed live while it runs, and what its
/// plan found.
pub(crate) fn operation(operation: &Operation, follow: Option<Follow>) -> Element {
	let header = View()
		.class("flex-col gap-2")
		.child(
			View()
				.class("flex-row flex-wrap items-center gap-2")
				.child(components::link(
					operation.project.clone(),
					format!("{BASE}/infra/projects/{}/operations", operation.project),
				))
				.child(Text("/").class(style::MUTED))
				.child(
					Text(operation.id.chars().take(8).collect::<String>()).class(style::SECONDARY),
				),
		)
		.child(Heading(1, operation.title.clone()).class(style::PAGE_TITLE))
		.child(
			View()
				.class("flex-row flex-wrap items-center gap-3")
				.child(
					components::status_word(operation.phase.0, operation.phase.1.clone())
						.test_id("phase"),
				)
				.child(Text(operation.started.clone()).class(style::SECONDARY)),
		);

	let known = operation
		.steps
		.iter()
		.map(|step| step.sequence)
		.max()
		.unwrap_or_default();
	let arrived = follow.map(|follow| follow.steps(known)).unwrap_or_default();
	let latest = arrived.last().map(|step| step.message.clone());
	let steps: Vec<&Step> = operation.steps.iter().chain(&arrived).collect();
	let count = steps.len();

	let timeline =
		View()
			.class("flex-col")
			.test_id("timeline")
			.children(steps.into_iter().enumerate().map(|(index, step)| {
				let last = index + 1 == count;
				View()
					.class("relative flex-row gap-3 pb-4 pl-6")
					.test_id("event")
					.child(
						View()
							.class(
								"absolute left-[5px] top-3 bottom-0 w-0.5 bg-[var(--admin-line)]",
							)
							.class(if last { "hidden" } else { "" })
							.aria_hidden(true),
					)
					.child(
						View()
							.class(
								"absolute left-0 top-1 h-3 w-3 rounded-full border-2 \
								 border-[var(--admin-ink)]",
							)
							.class(if last && operation.running {
								"bg-[var(--admin-ember)] animate-pulse"
							} else {
								"bg-[var(--admin-surface)]"
							})
							.aria_hidden(true),
					)
					.child(
						View()
							.class("min-w-0 flex-1 flex-col gap-0.5")
							.child(Text(step.message.clone()).class(style::BODY))
							.child(
								Text(format!("{} · {}", step.word, step.when))
									.class("text-xs")
									.class(step.tone.text()),
							),
					)
			}));

	let mut card = components::card("Timeline")
		.workings(operation.drawer.id.clone(), "lk console status")
		.child(timeline);

	if operation.running {
		let live = if follow.is_some_and(|follow| follow.lost()) {
			"Lost the live connection. Reload to catch up; the operation keeps running.".to_owned()
		} else if let Some(latest) = latest {
			format!("Following live: {latest}")
		} else {
			"Following live. Closing this tab cancels nothing.".to_owned()
		};
		card = card.child(
			Status(live)
				.class("text-sm text-[var(--admin-ink-2)]")
				.test_id("live"),
		);
	} else {
		card = card.child(
			Text(operation.result.clone())
				.class(style::SECONDARY)
				.test_id("result"),
		);
	}

	let mut body = View()
		.class("flex-col gap-5")
		.child(header)
		.child(card.build());

	if !operation.changes.is_empty() {
		let list = View()
			.class("flex-col gap-1.5")
			.children(operation.changes.iter().map(|change| {
				View()
					.class("flex-row gap-3")
					.child(
						Text(change.label.clone())
							.class("w-24 shrink-0 text-sm font-semibold")
							.class(change.tone.text()),
					)
					.child(
						Text(change.address.clone())
							.class(style::CODE)
							.class("truncate"),
					)
			}));

		body = body.child(
			components::card("What the plan found")
				.child(list)
				.child(components::link(
					"Review it under Changes",
					format!("{BASE}/infra/projects/{}/changes", operation.project),
				))
				.build(),
		);
	}

	body
}
