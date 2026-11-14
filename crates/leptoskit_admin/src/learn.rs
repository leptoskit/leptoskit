//! The Learn hub: short lessons whose outcomes are checked against the
//! project's real state. A lesson can't be ticked by reading it.

use leptoskit::prelude::*;
use leptoskit_deploy_console::control::Action;
use leptoskit_deploy_console::control::Phase;

use crate::BASE;
use crate::components;
use crate::components::Tone;
use crate::infra::ProjectState;
use crate::shell::Screen;
use crate::shell::Section;
use crate::style;

/// A track of lessons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Track {
	Analytics,
	Infrastructure,
	Build,
}

impl Track {
	fn slug(self) -> &'static str {
		match self {
			Self::Analytics => "analytics",
			Self::Infrastructure => "infrastructure",
			Self::Build => "build",
		}
	}

	fn title(self) -> &'static str {
		match self {
			Self::Analytics => "Analytics",
			Self::Infrastructure => "Infrastructure",
			Self::Build => "Build with leptoskit",
		}
	}
}

/// One lesson.
pub(crate) struct Lesson {
	pub track: Track,
	pub slug: &'static str,
	pub title: &'static str,
	pub minutes: u8,
	/// What it teaches, in a few sentences.
	pub body: &'static str,
	/// Where to do it, under the project.
	pub action: &'static str,
	/// The outcome it checks.
	pub outcome: &'static str,
}

const LESSONS: &[Lesson] = &[
	Lesson {
		track: Track::Infrastructure,
		slug: "what-infra-describes",
		title: "What [infra] describes",
		minutes: 3,
		body: "Your servers, their size and place, the tunnel in front of them, and the backups \
		       behind them are all one table in leptoskit.toml. Nothing about them lives only in \
		       a provider's console, so the file is the truth and every change starts there.",
		action: "config",
		outcome: "The admin reads a managed [infra] table in your project",
	},
	Lesson {
		track: Track::Infrastructure,
		slug: "plan-review-apply",
		title: "Plan, review, apply",
		minutes: 5,
		body: "A plan compares [infra] with what exists and lists the difference; it changes \
		       nothing. You read the rows, and Apply runs exactly that plan, refusing it if \
		       anything moved underneath. Deleting what keeps data is blocked outright.",
		action: "changes",
		outcome: "You applied a plan",
	},
	Lesson {
		track: Track::Infrastructure,
		slug: "releases-and-rollbacks",
		title: "Releases and rollbacks",
		minutes: 4,
		body: "A deploy starts the new image, waits for its health checks, then moves traffic. A \
		       rollback moves traffic back to an older image, and only that: database changes \
		       stay, which is why migrations add before they remove.",
		action: "releases",
		outcome: "You deployed a release and rolled one back",
	},
	Lesson {
		track: Track::Infrastructure,
		slug: "backups-restored",
		title: "Backups you've actually restored",
		minutes: 4,
		body: "A backup counts once it has been restored. Verify restores the latest one into a \
		       scratch database and checks it, without touching what's serving.",
		action: "backups",
		outcome: "A backup verification succeeded",
	},
	Lesson {
		track: Track::Infrastructure,
		slug: "reading-health",
		title: "Reading health",
		minutes: 5,
		body: "Health merges what every server reports: p95 latency, error rate, and each host's \
		       memory and disk. Release markers on the charts answer \"did the deploy do this?\" \
		       by looking.",
		action: "health",
		outcome: "A health check passed",
	},
];

const ANALYTICS: &[&str] = &[
	"Your first typed event",
	"Reading a trend",
	"Building a funnel",
	"Retention, honestly",
	"Who are these people?",
	"SQL when you need it",
];

const BUILD: &[(&str, &str)] = &[
	("Builder components", "guide/07-shared-components.html"),
	("Tailwind in leptoskit", "reference/css-coverage.html"),
	("Typed RPC and live queries", "guide/24-real-time.html"),
	(
		"Shipping to native with code push",
		"guide/05-code-push.html",
	),
];

/// Whether a lesson's outcome holds, and the evidence.
fn evidence(lesson: &Lesson, states: &[ProjectState], now: u64) -> Option<String> {
	let succeeded = |matches: &dyn Fn(&Action) -> bool| {
		states.iter().find_map(|state| {
			state
				.operations
				.iter()
				.find(|operation| {
					operation.phase == Phase::Succeeded && matches(&operation.intent.action)
				})
				.map(|operation| (state.project.id.clone(), operation.updated))
		})
	};

	match lesson.slug {
		"what-infra-describes" => {
			states
				.iter()
				.find(|state| state.config.is_ok())
				.map(|state| {
					format!(
						"{} declares its infrastructure in leptoskit.toml",
						state.project.id
					)
				})
		}
		"plan-review-apply" => {
			succeeded(&|action| matches!(action, Action::Apply { .. })).map(|(project, at)| {
				format!(
					"You applied a plan on {project} {}",
					crate::infra::ago(at, now)
				)
			})
		}
		"releases-and-rollbacks" => {
			let deployed = succeeded(&|action| matches!(action, Action::Deploy { .. }))?;
			let rolled = succeeded(&|action| matches!(action, Action::Rollback { .. }))?;
			Some(format!(
				"You deployed on {} and rolled back {}",
				deployed.0,
				crate::infra::ago(rolled.1, now)
			))
		}
		"backups-restored" => {
			succeeded(&|action| matches!(action, Action::Backup)).map(|(project, at)| {
				format!(
					"A backup of {project} restored and checked {}",
					crate::infra::ago(at, now)
				)
			})
		}
		"reading-health" => {
			succeeded(&|action| matches!(action, Action::Status)).map(|(project, at)| {
				format!(
					"{project} passed a health check {}",
					crate::infra::ago(at, now)
				)
			})
		}
		_ => None,
	}
}

/// Lessons finished, and lessons there are to check.
pub(crate) fn progress(states: &[ProjectState], now: u64) -> (usize, usize) {
	let done = LESSONS
		.iter()
		.filter(|lesson| evidence(lesson, states, now).is_some())
		.count();
	(done, LESSONS.len())
}

/// The mark assembling: one piece per finished track.
fn assembling(analytics: bool, infrastructure: bool, build: bool) -> Element {
	let piece = |earned: bool, class: &str, place: &str| {
		View()
			.class("absolute rounded-[6px]")
			.class(place)
			.class(if earned {
				class.to_owned()
			} else {
				"border-2 border-[var(--admin-line-strong)]".to_owned()
			})
	};

	View()
		.class("relative h-[112px] w-[112px] shrink-0")
		.role(leptoskit::native::Role::Img)
		.aria_label(format!(
			"Your mark: {} of 3 pieces",
			usize::from(analytics) + usize::from(infrastructure) + usize::from(build)
		))
		.test_id("progress-mark")
		.child(piece(
			analytics,
			"bg-[var(--admin-ember)]",
			"left-0 top-4 h-24 w-11",
		))
		.child(piece(
			infrastructure,
			"bg-[var(--admin-ember-warm)]",
			"left-14 top-[68px] h-11 w-11",
		))
		.child(piece(
			build,
			"bg-[var(--admin-mint)] -rotate-12",
			"left-[60px] top-0 h-11 w-11",
		))
}

/// Where a lesson is done: the screen under the only project, or the
/// project list when there are several.
fn lesson_target(lesson: &Lesson, states: &[ProjectState]) -> (String, String) {
	match states {
		[only] => {
			(
				format!(
					"{BASE}/infra/projects/{}/{}",
					only.project.id, lesson.action
				),
				format!("Continue in {}", only.project.id),
			)
		}
		_ => {
			(
				format!("{BASE}/infra/projects"),
				"Choose a project".to_owned(),
			)
		}
	}
}

fn lesson_href(lesson: &Lesson) -> String {
	format!("{BASE}/learn/{}/{}", lesson.track.slug(), lesson.slug)
}

/// The hub.
#[expect(
	clippy::too_many_lines,
	reason = "the hub in reading order: progress, up next, then each track"
)]
pub(crate) fn hub(states: &[ProjectState], now: u64) -> Screen {
	let (done, total) = progress(states, now);
	let infra_done = done == total;

	let mut infrastructure = View().class("flex-col gap-2");

	for lesson in LESSONS {
		let proof = evidence(lesson, states, now);
		infrastructure = infrastructure.child(
			Pressable(format!(
				"{}: {}",
				lesson.title,
				if proof.is_some() { "done" } else { "to do" }
			))
			.href(lesson_href(lesson))
			.class("cursor-pointer rounded-lg hover:bg-[var(--admin-sunken)]")
			.test_id(format!("lesson:{}", lesson.slug))
			.child(
				View()
					.class(
						"flex-row items-center gap-3 rounded-lg border border-[var(--admin-line)] \
						 px-3 py-2.5",
					)
					.child(match &proof {
						Some(_) => components::status_word(Tone::Good, "").class("w-5"),
						None => Text("○").class("w-5 text-sm text-[var(--admin-ink-3)]"),
					})
					.child(
						View()
							.class("min-w-0 flex-1 flex-col gap-0.5")
							.child(
								Text(lesson.title)
									.class("text-sm font-semibold text-[var(--admin-ink)]"),
							)
							.child(
								Text(proof.unwrap_or_else(|| {
									format!("Check: {}", lesson.outcome.to_lowercase())
								}))
								.class(style::MUTED)
								.class("text-xs"),
							),
					)
					.child(Text(format!("{} min", lesson.minutes)).class(style::LABEL)),
			),
		);
	}

	let mut analytics = View().class("flex-col gap-2");

	for title in ANALYTICS {
		analytics = analytics.child(
			View()
				.class(
					"flex-row items-center gap-3 rounded-lg border border-[var(--admin-line)] \
					 px-3 py-2.5",
				)
				.child(Text("○").class("w-5 text-sm text-[var(--admin-ink-3)]"))
				.child(Text(*title).class("min-w-0 flex-1 text-sm text-[var(--admin-ink-2)]"))
				.child(Text("coming with Analytics").class(style::LABEL)),
		);
	}

	let mut build = View().class("flex-col gap-2");

	for (title, path) in BUILD {
		build = build.child(components::link(
			*title,
			format!("https://leptoskit.dev/docs/{path}"),
		));
	}

	let left = total - done;
	let mark = View()
		.class("flex-col items-center justify-center gap-3 px-4 py-6")
		.child(
			View()
				.class("scale-150 py-8")
				.child(assembling(false, infra_done, false)),
		)
		.child(
			Text(match left {
				0 => "Infrastructure, done. A piece of the mark landed.".to_owned(),
				1 => "One lesson to go.".to_owned(),
				left => format!("{done} down. {left} to go."),
			})
			.class("text-xl text-center text-[var(--admin-ink)] font-[var(--admin-font-note)]"),
		)
		.child(
			Text(format!(
				"Finish a track and a piece of the mark lands. Infrastructure {done} of {total} · \
				 Analytics comes with its data."
			))
			.class(style::MUTED)
			.class("text-center")
			.test_id("progress"),
		);

	let next = LESSONS
		.iter()
		.find(|lesson| evidence(lesson, states, now).is_none());
	let up_next = match next {
		Some(lesson) => {
			let (target, label) = lesson_target(lesson, states);
			components::card("Up next")
				.aside(format!("{} min · {}", lesson.minutes, lesson.track.title()))
				.test_id("up-next")
				.child(
					Heading(2, lesson.title)
						.class("text-2xl text-[var(--admin-ink)] font-[var(--admin-font-note)]"),
				)
				.child(Text(lesson.body).class(style::BODY).class("text-base"))
				.child(components::status_word(
					Tone::Info,
					format!("Check: {}", lesson.outcome.to_lowercase()),
				))
				.child(
					View()
						.class("flex-row flex-wrap items-center gap-4 pt-1")
						.child(components::raised(
							Pressable(label.clone())
								.href(target)
								.class(style::BUTTON_PRIMARY)
								.child(Text(label)),
						))
						.child(components::link(
							"Read it here instead",
							lesson_href(lesson),
						)),
				)
				.child(
					Text("This lesson ticks itself once it's true of your projects.")
						.class(style::MUTED),
				)
				.build()
		}
		None => {
			components::card("All done")
				.test_id("up-next")
				.child(
					Text(
						"Every infrastructure lesson holds for your projects. Analytics lessons \
						 arrive with its data.",
					)
					.class(style::SECONDARY),
				)
				.build()
		}
	};

	let header = View()
		.class("flex-col gap-1")
		.child(Heading(1, "Learn").class(style::PAGE_TITLE))
		.child(
			Text("Short lessons that check your real project, not a quiz.").class(style::SECONDARY),
		);
	let hero = View()
		.class("grid grid-cols-[2fr_3fr] gap-4 max-lg:grid-cols-1")
		.child(View().class(style::CARD).child(mark))
		.child(up_next);

	let content = View()
		.class("flex-col gap-5")
		.child(header)
		.child(hero)
		.child(
			View()
				.class("grid grid-cols-2 gap-4 max-lg:grid-cols-1")
				.test_id("grid-main")
				.child(
					components::card(Track::Infrastructure.title())
						.aside(format!("{done}/{total}"))
						.child(infrastructure)
						.build(),
				)
				.child(
					View()
						.class("flex-col gap-4")
						.child(
							components::card(Track::Analytics.title())
								.aside("coming with Analytics")
								.child(analytics)
								.build(),
						)
						.child(
							components::card(Track::Build.title())
								.aside("in the book; not tracked here")
								.child(build)
								.build(),
						),
				),
		);

	Screen {
		title: "Your progress".to_owned(),
		section: Section::Learn,
		content,
		drawers: Vec::new(),
		open: None,
	}
}

/// One lesson.
pub(crate) fn lesson(track: &str, slug: &str, states: &[ProjectState], now: u64) -> Option<Screen> {
	let lesson = LESSONS
		.iter()
		.find(|lesson| lesson.track.slug() == track && lesson.slug == slug)?;
	let proof = evidence(lesson, states, now);
	let (target, _) = lesson_target(lesson, states);

	let content = View()
		.class("flex-col gap-5 max-w-[720px]")
		.child(components::link("Your progress", format!("{BASE}/learn")))
		.child(Heading(1, lesson.title).class(style::PAGE_TITLE))
		.child(
			Text(format!("{} · {} min", lesson.track.title(), lesson.minutes)).class(style::MUTED),
		)
		.child(Text(lesson.body).class(style::BODY).class("text-base"))
		.child(
			components::card("Do it")
				.child(Text(format!("{}.", lesson.outcome)).class(style::SECONDARY))
				.child(components::link_button(
					components::Kind::Primary,
					"Open the screen",
					target,
				))
				.build(),
		)
		.child(
			components::card("Outcome")
				.test_id("outcome")
				.child(match proof {
					Some(proof) => components::status_word(Tone::Good, proof),
					None => {
						components::status_word(
							Tone::Info,
							"Not yet. This ticks itself once it's true of your projects.",
						)
					}
				})
				.build(),
		);

	Some(Screen {
		title: lesson.title.to_owned(),
		section: Section::Learn,
		content,
		drawers: Vec::new(),
		open: None,
	})
}

const GLOSSARY: &[(&str, &str)] = &[
	(
		"Plan",
		"The list of changes OpenTofu would make so what exists matches [infra]. Making one \
		 changes nothing.",
	),
	(
		"Apply",
		"Running exactly one reviewed plan. It's refused if the configuration, provider lock, or \
		 state moved since.",
	),
	(
		"Replace",
		"Deleting a resource and creating a new one, because the change can't be made in place.",
	),
	(
		"Drift",
		"A difference between [infra] and what exists that nobody planned: someone changed \
		 something by hand.",
	),
	(
		"First release",
		"The first deploy of a project: it creates the database and runs every migration.",
	),
	(
		"Maintenance",
		"A deploy that shows visitors a maintenance page and asks live connections to come back \
		 later.",
	),
	(
		"Rollback",
		"Moving traffic back to an older image. Database changes stay.",
	),
	(
		"p95",
		"The time 95 of every 100 requests finish within. The slowest 5 are what people remember.",
	),
	(
		"Error rate",
		"The share of requests that ended in a server error (5xx).",
	),
	(
		"Operation",
		"One action the control service ran for a project, with every step recorded.",
	),
];

/// The glossary.
pub(crate) fn glossary() -> Screen {
	let mut list = View().class("flex-col gap-3");

	for (term, meaning) in GLOSSARY {
		list = list.child(
			View()
				.class("flex-col gap-0.5 border-b border-[var(--admin-line)] pb-3")
				.child(Text(*term).class("text-sm font-semibold text-[var(--admin-ink)]"))
				.child(Text(*meaning).class(style::SECONDARY)),
		);
	}

	Screen {
		title: "Glossary".to_owned(),
		section: Section::Glossary,
		content: View()
			.class("flex-col gap-5 max-w-[720px]")
			.child(Heading(1, "Glossary").class(style::PAGE_TITLE))
			.child(components::card("Infrastructure").child(list).build()),
		drawers: Vec::new(),
		open: None,
	}
}
