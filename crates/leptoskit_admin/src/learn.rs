//! The Learn hub: short lessons whose outcomes are checked against the
//! project's real state. A lesson can't be ticked by reading it.

use leptoskit::prelude::*;
use leptoskit_analytics::query::Query;
use leptoskit_deploy_console::control::Action;
use leptoskit_deploy_console::control::Phase;

use crate::BASE;
use crate::access::Preferences;
use crate::analytics::Facts;
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

/// What lessons are checked against: the projects, what the analytics
/// service holds, and what the person has done in the admin.
pub(crate) struct Proof<'a> {
	pub states: &'a [ProjectState],
	pub now: u64,
	/// `None` when the admin isn't connected to analytics, or it didn't
	/// answer.
	pub facts: Option<&'a Facts>,
	pub preferences: &'a Preferences,
	/// Whether infrastructure actions ran against a stand-in cloud, so
	/// evidence doesn't claim real infrastructure changed.
	pub simulated: bool,
}

/// One lesson.
pub(crate) struct Lesson {
	pub track: Track,
	pub slug: &'static str,
	pub title: &'static str,
	pub minutes: u8,
	/// What it teaches, in a few sentences.
	pub body: &'static str,
	/// Where to do it: a tab under the project, or for analytics a path
	/// under the admin.
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
	Lesson {
		track: Track::Analytics,
		slug: "first-typed-event",
		title: "Your first typed event",
		minutes: 4,
		body: "An event is a Rust struct with #[derive(Event)]: its fields are the properties, so \
		       a misspelled one fails when it compiles instead of becoming a new property in a \
		       chart. Typed events also send no property names, so each is a fraction of the \
		       bytes. Declare one, send it from your app, and watch it arrive.",
		action: "/analytics/live",
		outcome: "An event with a Rust type arrived",
	},
	Lesson {
		track: Track::Analytics,
		slug: "reading-a-trend",
		title: "Reading a trend",
		minutes: 4,
		body: "A trend counts an event per day, or per hour or week. Total counts every event; \
		       People counts each person once a day. A breakdown splits the line by a property, \
		       and the dashed line is the period before, so a rise reads as a rise. Press any day \
		       to see who's behind it. Save one to keep it.",
		action: "/analytics/trends",
		outcome: "You saved a trend",
	},
	Lesson {
		track: Track::Analytics,
		slug: "building-a-funnel",
		title: "Building a funnel",
		minutes: 5,
		body: "A funnel follows each person through steps in order, within a window, and counts \
		       who reached each one. The hatched part of a column is who reached the step before \
		       and stopped; pressing it lists them by name. Three steps is where a funnel starts \
		       telling you where people leave.",
		action: "/analytics/funnels",
		outcome: "You saved a funnel of three steps or more",
	},
	Lesson {
		track: Track::Analytics,
		slug: "retention-honestly",
		title: "Retention, honestly",
		minutes: 4,
		body: "Retention groups people by when they first did something, then asks how many came \
		       back in each later week. Period 0 is always everyone, so it's grey; read across a \
		       row to see one cohort fade, and down a column to see whether newer cohorts stay \
		       longer.",
		action: "/analytics/retention",
		outcome: "You saved a retention grid",
	},
	Lesson {
		track: Track::Analytics,
		slug: "who-are-these-people",
		title: "Who are these people?",
		minutes: 3,
		body: "Before someone signs in, their events carry an anonymous id. web::identify after \
		       sign-in joins that id to their account's, so one person holds everything they did, \
		       from the first visit. A person page shows every id merged into them.",
		action: "/analytics/people",
		outcome: "Someone's anonymous visits were merged into their account",
	},
	Lesson {
		track: Track::Analytics,
		slug: "sql-when-you-need-it",
		title: "SQL when you need it",
		minutes: 3,
		body: "Every chart is one value of the typed Query enum, and the service renders it to \
		       SQL for your database. Open any chart's Workings and read the Ask station's SQL \
		       tab: that's exactly what ran, with its bound values, so you can take it to a \
		       console when a chart isn't enough.",
		action: "/analytics/trends#workings=trends",
		outcome: "You read a query's SQL in its Workings",
	},
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

/// Whether an analytics lesson's outcome holds, and the evidence.
fn analytics_evidence(lesson: &Lesson, facts: &Facts, preferences: &Preferences) -> Option<String> {
	let saved = |matches: &dyn Fn(&Query) -> bool| {
		facts
			.insights
			.iter()
			.find(|insight| matches(&insight.query))
			.map(|insight| format!("You saved “{}”", insight.name))
	};

	match lesson.slug {
		"first-typed-event" => {
			facts
				.definitions
				.iter()
				.find(|definition| definition.fields.is_some() && definition.volume > 0)
				.map(|definition| {
					format!(
						"{} arrived {} times, typed",
						crate::analytics::format::struct_name(&definition.name),
						crate::analytics::format::thousands(
							u64::try_from(definition.volume).unwrap_or(0)
						)
					)
				})
		}
		"reading-a-trend" => saved(&|query| matches!(query, Query::Trends(_))),
		"building-a-funnel" => {
			saved(&|query| matches!(query, Query::Funnels(funnels) if funnels.steps.len() >= 3))
		}
		"retention-honestly" => saved(&|query| matches!(query, Query::Retention(_))),
		"who-are-these-people" => {
			facts
				.merged_person
				.as_ref()
				.map(|(_, name)| format!("{name}'s anonymous visits and account are one person"))
		}
		"sql-when-you-need-it" => {
			preferences
				.notes
				.get("ask-sql")
				.filter(|note| note.uses > 0)
				.map(|_| "You read a query's SQL in its Workings".to_owned())
		}
		_ => None,
	}
}

/// Whether a lesson's outcome holds, and the evidence.
fn evidence(lesson: &Lesson, proof: &Proof<'_>) -> Option<String> {
	let (states, now) = (proof.states, proof.now);

	if lesson.track == Track::Analytics {
		return analytics_evidence(lesson, proof.facts?, proof.preferences);
	}

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
					"You applied a plan on {project}{} {}",
					if proof.simulated {
						" on the simulated cloud"
					} else {
						""
					},
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
pub(crate) fn progress(proof: &Proof<'_>) -> (usize, usize) {
	let done = LESSONS
		.iter()
		.filter(|lesson| evidence(lesson, proof).is_some())
		.count();
	(done, LESSONS.len())
}

/// Lessons finished in one track, and lessons in it.
fn track_progress(track: Track, proof: &Proof<'_>) -> (usize, usize) {
	let lessons: Vec<&Lesson> = LESSONS
		.iter()
		.filter(|lesson| lesson.track == track)
		.collect();
	let done = lessons
		.iter()
		.filter(|lesson| evidence(lesson, proof).is_some())
		.count();
	(done, lessons.len())
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
	if lesson.track == Track::Analytics {
		return (format!("{BASE}{}", lesson.action), "Open it".to_owned());
	}

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

fn lesson_row(lesson: &Lesson, proof: &Proof<'_>) -> Element {
	let evidence = evidence(lesson, proof);
	Pressable(format!(
		"{}: {}",
		lesson.title,
		if evidence.is_some() { "done" } else { "to do" }
	))
	.href(lesson_href(lesson))
	.class("cursor-pointer rounded-lg hover:bg-[var(--admin-sunken)]")
	.test_id(format!("lesson:{}", lesson.slug))
	.child(
		View()
			.class(
				"flex-row items-center gap-3 rounded-lg border border-[var(--admin-line)] px-3 \
				 py-2.5",
			)
			.child(match &evidence {
				Some(_) => components::status_word(Tone::Good, "").class("w-5"),
				None => Text("○").class("w-5 text-sm text-[var(--admin-ink-3)]"),
			})
			.child(
				View()
					.class("min-w-0 flex-1 flex-col gap-0.5")
					.child(
						Text(lesson.title).class("text-sm font-semibold text-[var(--admin-ink)]"),
					)
					.child(
						Text(evidence.unwrap_or_else(|| check(lesson.outcome)))
							.class(style::MUTED)
							.class("text-xs"),
					),
			)
			.child(Text(format!("{} min", lesson.minutes)).class(style::LABEL)),
	)
}

/// The hub.
pub(crate) fn hub(proof: &Proof<'_>) -> Screen {
	let (done, total) = progress(proof);
	let (infra_done, infra_total) = track_progress(Track::Infrastructure, proof);
	let (analytics_done, analytics_total) = track_progress(Track::Analytics, proof);

	let mut infrastructure = View().class("flex-col gap-2");
	let mut analytics = View().class("flex-col gap-2");

	for lesson in LESSONS {
		match lesson.track {
			Track::Infrastructure => {
				infrastructure = infrastructure.child(lesson_row(lesson, proof));
			}
			Track::Analytics => analytics = analytics.child(lesson_row(lesson, proof)),
			Track::Build => {}
		}
	}

	if proof.facts.is_none() {
		analytics = analytics.child(
			Text(
				"These lessons check your real analytics, which the admin can't read: either no \
				 analytics service is connected, or it didn't answer. Analytics says which.",
			)
			.class(style::MUTED),
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
		.child(View().class("scale-150 py-8").child(assembling(
			analytics_done == analytics_total,
			infra_done == infra_total,
			false,
		)))
		.child(
			Text(match left {
				0 => "Every lesson, done. The mark is yours.".to_owned(),
				1 => "One lesson to go.".to_owned(),
				left => format!("{done} down. {left} to go."),
			})
			.class("text-xl text-center text-[var(--admin-ink)] font-[var(--admin-font-note)]"),
		)
		.child(
			Text(format!(
				"Finish a track and a piece of the mark lands. Analytics {analytics_done} of \
				 {analytics_total} · Infrastructure {infra_done} of {infra_total}"
			))
			.class(style::MUTED)
			.class("text-center")
			.test_id("progress"),
		);

	let next = LESSONS
		.iter()
		.filter(|lesson| lesson.track != Track::Analytics || proof.facts.is_some())
		.find(|lesson| evidence(lesson, proof).is_none());
	let up_next = match next {
		Some(lesson) => {
			let (target, label) = lesson_target(lesson, proof.states);
			components::card("Up next")
				.aside(format!("{} min · {}", lesson.minutes, lesson.track.title()))
				.test_id("up-next")
				.child(
					Heading(2, lesson.title)
						.class("text-2xl text-[var(--admin-ink)] font-[var(--admin-font-note)]"),
				)
				.child(Text(lesson.body).class(style::BODY).class("text-base"))
				.child(components::status_word(Tone::Info, check(lesson.outcome)))
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
					Text("This lesson ticks itself once it's true of your data.")
						.class(style::MUTED),
				)
				.build()
		}
		None => {
			components::card("All done")
				.test_id("up-next")
				.child(
					Text("Every lesson the admin can check holds for your projects and your data.")
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
					components::card(Track::Analytics.title())
						.aside(format!("{analytics_done}/{analytics_total}"))
						.child(analytics)
						.build(),
				)
				.child(
					View()
						.class("flex-col gap-4")
						.child(
							components::card(Track::Infrastructure.title())
								.aside(format!("{infra_done}/{infra_total}"))
								.child(infrastructure)
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
pub(crate) fn lesson(track: &str, slug: &str, proof: &Proof<'_>) -> Option<Screen> {
	let lesson = LESSONS
		.iter()
		.find(|lesson| lesson.track.slug() == track && lesson.slug == slug)?;
	let evidence = evidence(lesson, proof);
	let (target, _) = lesson_target(lesson, proof.states);

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
				.child(match evidence {
					Some(evidence) => components::status_word(Tone::Good, evidence),
					None => {
						components::status_word(
							Tone::Info,
							"Not yet. This ticks itself once it's true of your data.",
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

const ANALYTICS_GLOSSARY: &[(&str, &str)] = &[
	(
		"Distinct id",
		"The id an event is sent under: an anonymous one before sign-in, the account's after. One \
		 person can have several.",
	),
	(
		"Session",
		"A run of events from one browser with no gap longer than 30 minutes.",
	),
	(
		"Bounce",
		"A session with one pageview and under 10 seconds. Bounce rate is the share of sessions \
		 that bounced.",
	),
	(
		"Channel",
		"Where a session came from, worked out once from its referrer and utm_* tags: Direct, \
		 Organic search, Referral, Social, Email, Paid search.",
	),
	(
		"Typed event",
		"An event declared as a Rust struct with #[derive(Event)]. It sends values in field \
		 order, without property names.",
	),
	(
		"Cohort",
		"The people who did something first in the same period: a row of the retention grid.",
	),
	(
		"Actors",
		"The people behind one number: a day of a trend, a funnel step, a retention cell.",
	),
];

fn terms(entries: &[(&'static str, &'static str)]) -> Element {
	let mut list = View().class("flex-col gap-3");

	for (term, meaning) in entries {
		list = list.child(
			View()
				.class("flex-col gap-0.5 border-b border-[var(--admin-line)] pb-3")
				.child(Text(*term).class("text-sm font-semibold text-[var(--admin-ink)]"))
				.child(Text(*meaning).class(style::SECONDARY)),
		);
	}

	list
}

/// The glossary.
pub(crate) fn glossary() -> Screen {
	Screen {
		title: "Glossary".to_owned(),
		section: Section::Glossary,
		content: View()
			.class("flex-col gap-5 max-w-[720px]")
			.child(Heading(1, "Glossary").class(style::PAGE_TITLE))
			.child(
				components::card("Analytics")
					.child(terms(ANALYTICS_GLOSSARY))
					.build(),
			)
			.child(
				components::card("Infrastructure")
					.child(terms(GLOSSARY))
					.build(),
			),
		drawers: Vec::new(),
		open: None,
	}
}

/// `Check: you applied a plan`: the outcome with only its first letter
/// lowered, so Rust and SQL keep their capitals.
fn check(outcome: &str) -> String {
	let mut letters = outcome.chars();
	letters.next().map_or_else(String::new, |first| {
		format!("Check: {}{}", first.to_lowercase(), letters.as_str())
	})
}
