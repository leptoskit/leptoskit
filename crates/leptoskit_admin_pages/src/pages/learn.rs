//! The Learn hub and its lessons. The admin checks each lesson's outcome
//! against the project's real state and hands the result here; a lesson
//! can't be ticked by reading it.

use leptoskit::native::Role;
use leptoskit::prelude::*;

use crate::BASE;
use crate::components;
use crate::components::Tone;
use crate::model::Learn;
use crate::model::Lesson;
use crate::model::LessonRow;
use crate::style;

/// The tracks' titles.
const ANALYTICS: &str = "Analytics";
const INFRASTRUCTURE: &str = "Infrastructure";
const BUILD_TITLE: &str = "Build with leptoskit";

/// The Build track: chapters of the book, which the admin can't check.
const BUILD: &[(&str, &str)] = &[
	("Builder components", "guide/07-shared-components.html"),
	("Tailwind in leptoskit", "reference/css-coverage.html"),
	("Typed RPC and live queries", "guide/24-real-time.html"),
	(
		"Shipping to native with code push",
		"guide/05-code-push.html",
	),
];

/// Lessons done of `rows`, and lessons there are.
fn done(rows: &[LessonRow]) -> (usize, usize) {
	(
		rows.iter().filter(|row| row.evidence.is_some()).count(),
		rows.len(),
	)
}

/// The mark assembling: one piece per finished track.
fn assembling(analytics: bool, infrastructure: bool, build: bool) -> Element {
	let piece = |earned: bool, class: &str, place: &str| {
		View()
			.class("absolute rounded-[6px]")
			.class(place)
			.class(if earned {
				class
			} else {
				"border-2 border-[var(--admin-line-strong)]"
			})
	};

	View()
		.class("relative h-[112px] w-[112px] shrink-0")
		.role(Role::Img)
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

fn lesson_row(row: &LessonRow) -> Element {
	Pressable(format!(
		"{}: {}",
		row.title,
		if row.evidence.is_some() {
			"done"
		} else {
			"to do"
		}
	))
	.href(row.href.clone())
	.class("cursor-pointer rounded-lg hover:bg-[var(--admin-sunken)]")
	.test_id(format!("lesson:{}", row.slug))
	.child(
		View()
			.class(
				"flex-row items-center gap-3 rounded-lg border border-[var(--admin-line)] px-3 \
				 py-2.5",
			)
			.child(match &row.evidence {
				Some(_) => components::status_word(Tone::Good, "").class("w-5"),
				None => Text("○").class("w-5 text-sm text-[var(--admin-ink-3)]"),
			})
			.child(
				View()
					.class("min-w-0 flex-1 flex-col gap-0.5")
					.child(
						Text(row.title.clone())
							.class("text-sm font-semibold text-[var(--admin-ink)]"),
					)
					.child(
						Text(row.evidence.clone().unwrap_or_else(|| row.check.clone()))
							.class(style::MUTED)
							.class("text-xs"),
					),
			)
			.child(Text(format!("{} min", row.minutes)).class(style::LABEL)),
	)
}

fn track(rows: &[LessonRow]) -> Element {
	View()
		.class("flex-col gap-2")
		.children(rows.iter().map(lesson_row))
}

/// The hub: the mark, the lesson up next, and every lesson by track.
pub(crate) fn hub(learn: &Learn) -> Element {
	let (analytics_done, analytics_total) = done(&learn.analytics);
	let (infra_done, infra_total) = done(&learn.infrastructure);
	let (done, total) = (analytics_done + infra_done, analytics_total + infra_total);

	let mut analytics = track(&learn.analytics);

	if !learn.analytics_readable {
		analytics = analytics.child(
			Text(
				"These lessons check your real analytics, which the admin can't read: either no \
				 analytics service is connected, or it didn't answer. Analytics says which.",
			)
			.class(style::MUTED),
		);
	}

	let build = View()
		.class("flex-col gap-2")
		.children(BUILD.iter().map(|(title, path)| {
			components::link(*title, format!("https://leptoskit.dev/docs/{path}"))
		}));

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

	let up_next = match &learn.up_next {
		Some(next) => {
			components::card("Up next")
				.aside(format!("{} min · {}", next.minutes, next.track))
				.test_id("up-next")
				.child(
					Heading(2, next.title.clone())
						.class("text-2xl text-[var(--admin-ink)] font-[var(--admin-font-note)]"),
				)
				.child(
					Text(next.body.clone())
						.class(style::BODY)
						.class("text-base"),
				)
				.child(components::status_word(Tone::Info, next.check.clone()))
				.child(
					View()
						.class("flex-row flex-wrap items-center gap-4 pt-1")
						.child(components::raised(
							Pressable(next.label.clone())
								.href(next.target.clone())
								.class(style::BUTTON_PRIMARY)
								.child(Text(next.label.clone())),
						))
						.child(components::link("Read it here instead", next.href.clone())),
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

	View()
		.class("flex-col gap-5")
		.child(header)
		.child(hero)
		.child(
			View()
				.class("grid grid-cols-2 gap-4 max-lg:grid-cols-1")
				.test_id("grid-main")
				.child(
					components::card(ANALYTICS)
						.aside(format!("{analytics_done}/{analytics_total}"))
						.child(analytics)
						.build(),
				)
				.child(
					View()
						.class("flex-col gap-4")
						.child(
							components::card(INFRASTRUCTURE)
								.aside(format!("{infra_done}/{infra_total}"))
								.child(track(&learn.infrastructure))
								.build(),
						)
						.child(
							components::card(BUILD_TITLE)
								.aside("in the book; not tracked here")
								.child(build)
								.build(),
						),
				),
		)
}

/// One lesson: what it teaches, where to do it, and whether it holds.
pub(crate) fn lesson(lesson: &Lesson) -> Element {
	View()
		.class("flex-col gap-5 max-w-[720px]")
		.child(components::link("Your progress", format!("{BASE}/learn")))
		.child(Heading(1, lesson.title.clone()).class(style::PAGE_TITLE))
		.child(Text(format!("{} · {} min", lesson.track, lesson.minutes)).class(style::MUTED))
		.child(
			Text(lesson.body.clone())
				.class(style::BODY)
				.class("text-base"),
		)
		.child(
			components::card("Do it")
				.child(Text(format!("{}.", lesson.outcome)).class(style::SECONDARY))
				.child(components::link_button(
					components::Kind::Primary,
					"Open the screen",
					lesson.target.clone(),
				))
				.build(),
		)
		.child(
			components::card("Outcome")
				.test_id("outcome")
				.child(match &lesson.evidence {
					Some(evidence) => components::status_word(Tone::Good, evidence.clone()),
					None => {
						components::status_word(
							Tone::Info,
							"Not yet. This ticks itself once it's true of your data.",
						)
					}
				})
				.build(),
		)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::model::UpNext;

	fn row(slug: &str, evidence: Option<&str>) -> LessonRow {
		LessonRow {
			slug: slug.to_owned(),
			title: slug.to_owned(),
			minutes: 3,
			href: format!("{BASE}/learn/infrastructure/{slug}"),
			evidence: evidence.map(str::to_owned),
			check: "Check: it holds".to_owned(),
		}
	}

	#[test]
	fn every_class_the_learn_pages_use_compiles() {
		let next = UpNext {
			title: "Plan, review, apply".to_owned(),
			minutes: 5,
			track: INFRASTRUCTURE.to_owned(),
			body: "A plan compares.".to_owned(),
			check: "Check: you applied a plan".to_owned(),
			target: format!("{BASE}/infra/projects"),
			label: "Choose a project".to_owned(),
			href: format!("{BASE}/learn/infrastructure/plan-review-apply"),
		};

		for (up_next, readable) in [(Some(next), true), (None, false)] {
			let learn = Learn {
				analytics: vec![row("first-typed-event", None)],
				infrastructure: vec![row("what-infra-describes", Some("It does"))],
				analytics_readable: readable,
				up_next,
			};
			assert!(hub(&learn).validate().is_ok());
		}

		for evidence in [None, Some("You applied a plan just now".to_owned())] {
			let page = lesson(&Lesson {
				title: "Plan, review, apply".to_owned(),
				track: INFRASTRUCTURE.to_owned(),
				minutes: 5,
				body: "A plan compares.".to_owned(),
				outcome: "You applied a plan".to_owned(),
				target: format!("{BASE}/infra/projects"),
				evidence,
			});
			assert!(page.validate().is_ok());
		}
	}
}
