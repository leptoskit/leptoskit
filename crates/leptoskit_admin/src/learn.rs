//! The Learn hub: short lessons whose outcomes are checked against the
//! project's real state. A lesson can't be ticked by reading it.

use leptoskit_admin_pages::model;
use leptoskit_admin_pages::model::Learn;
use leptoskit_admin_pages::model::LessonRow;
use leptoskit_admin_pages::model::UpNext;
use leptoskit_analytics::query::Query;
use leptoskit_deploy_console::control::Action;
use leptoskit_deploy_console::control::Phase;

use crate::BASE;
use crate::access::Preferences;
use crate::analytics::Facts;
use crate::infra::ProjectState;

/// A track of lessons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Track {
	Analytics,
	Infrastructure,
}

impl Track {
	fn slug(self) -> &'static str {
		match self {
			Self::Analytics => "analytics",
			Self::Infrastructure => "infrastructure",
		}
	}

	fn title(self) -> &'static str {
		match self {
			Self::Analytics => "Analytics",
			Self::Infrastructure => "Infrastructure",
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
				"You deployed on {} and rolled back{} {}",
				deployed.0,
				if proof.simulated {
					" on the simulated cloud"
				} else {
					""
				},
				crate::infra::ago(rolled.1, now)
			))
		}
		"backups-restored" => {
			succeeded(&|action| matches!(action, Action::Backup)).map(|(project, at)| {
				format!(
					"{} {}",
					if proof.simulated {
						format!("A simulated backup verification completed for {project}")
					} else {
						format!("A backup of {project} restored and checked")
					},
					crate::infra::ago(at, now)
				)
			})
		}
		"reading-health" => {
			succeeded(&|action| matches!(action, Action::Status)).map(|(project, at)| {
				format!(
					"{project} passed a {}health check {}",
					if proof.simulated { "simulated " } else { "" },
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

/// The hub's rows for one track.
fn rows(track: Track, proof: &Proof<'_>) -> Vec<LessonRow> {
	LESSONS
		.iter()
		.filter(|lesson| lesson.track == track)
		.map(|lesson| {
			LessonRow {
				slug: lesson.slug.to_owned(),
				title: lesson.title.to_owned(),
				minutes: lesson.minutes,
				href: lesson_href(lesson),
				evidence: evidence(lesson, proof),
				check: check(lesson.outcome),
			}
		})
		.collect()
}

/// The hub: every lesson checked, and the first one that doesn't hold yet.
pub(crate) fn hub(proof: &Proof<'_>) -> Learn {
	let up_next = LESSONS
		.iter()
		.filter(|lesson| lesson.track != Track::Analytics || proof.facts.is_some())
		.find(|lesson| evidence(lesson, proof).is_none())
		.map(|lesson| {
			let (target, label) = lesson_target(lesson, proof.states);
			UpNext {
				title: lesson.title.to_owned(),
				minutes: lesson.minutes,
				track: lesson.track.title().to_owned(),
				body: lesson.body.to_owned(),
				check: check(lesson.outcome),
				target,
				label,
				href: lesson_href(lesson),
			}
		});

	Learn {
		analytics: rows(Track::Analytics, proof),
		infrastructure: rows(Track::Infrastructure, proof),
		analytics_readable: proof.facts.is_some(),
		up_next,
	}
}

/// One lesson, checked.
pub(crate) fn lesson(track: &str, slug: &str, proof: &Proof<'_>) -> Option<model::Lesson> {
	let lesson = LESSONS
		.iter()
		.find(|lesson| lesson.track.slug() == track && lesson.slug == slug)?;
	let (target, _) = lesson_target(lesson, proof.states);

	Some(model::Lesson {
		title: lesson.title.to_owned(),
		track: lesson.track.title().to_owned(),
		minutes: lesson.minutes,
		body: lesson.body.to_owned(),
		outcome: lesson.outcome.to_owned(),
		target,
		evidence: evidence(lesson, proof),
	})
}

/// `Check: you applied a plan`: the outcome with only its first letter
/// lowered, so Rust and SQL keep their capitals.
fn check(outcome: &str) -> String {
	let mut letters = outcome.chars();
	letters.next().map_or_else(String::new, |first| {
		format!("Check: {}{}", first.to_lowercase(), letters.as_str())
	})
}
