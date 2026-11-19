//! The glossary: the words the admin uses, in two cards.

use leptoskit::prelude::*;

use crate::components;
use crate::style;

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

/// The glossary's content.
pub(crate) fn content() -> Element {
	View()
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
		)
}
