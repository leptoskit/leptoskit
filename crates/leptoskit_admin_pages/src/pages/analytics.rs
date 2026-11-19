//! The analytics pages that aren't a builder: a dashboard, the people behind
//! one point of a chart, and what a page says when it has nothing to read.

use leptoskit::prelude::*;
use leptoskit_analytics::query::QueryResult;

use crate::BASE;
use crate::analytics::format;
use crate::analytics::insights;
use crate::components;
use crate::components::Kind;
use crate::components::Tone;
use crate::model::analytics::Actor;
use crate::model::analytics::Actors;
use crate::model::analytics::Card;
use crate::model::analytics::Dashboard;
use crate::model::analytics::Notice;
use crate::model::analytics::Why;
use crate::style;

/// The code that sends a first event.
const FIRST_EVENT: &str = "use leptoskit_analytics::Event;
use leptoskit_analytics::client::web;

#[derive(Event)]
struct SignedUp {
    plan: String,
    seats: u32,
}

web::init(web::Options::default());
web::capture_event(SignedUp {
    plan: \"pro\".into(),
    seats: 3,
});";

/// What connects the admin to the analytics service.
const CONNECT: &str = r"# Wherever the admin runs: the main site's analytics service,
# at an address the admin can reach, and the token its server uses.
export LEPTOSKIT_SERVICE_ANALYTICS_URL=http://127.0.0.1:4800
export LEPTOSKIT_SERVICE_ANALYTICS_TOKEN=…   # the same token as the site's server
lk admin serve --main-site https://notes.example";

/// One dashboard card: its insight, drawn small.
fn card(card: &Card) -> Element {
	let body = match &card.answer {
		Ok(QueryResult::Trends(result)) => {
			insights::trend_view(&card.query, result, &card.href, card.wide, None).chart
		}
		Ok(QueryResult::Funnels(result)) => {
			insights::funnel_view(&card.query, result, &card.href, "h-40").0
		}
		Ok(QueryResult::Retention(result)) => {
			View()
				.class("max-h-80 overflow-scroll")
				.child(insights::retention_view(&card.query, result, &card.href).0)
		}
		Ok(QueryResult::WebOverview(result)) => {
			let figure = |label: &str, value: String| {
				View()
					.class("flex-col gap-0.5")
					.child(Text(label.to_owned()).class(style::LABEL))
					.child(
						Text(value)
							.class("text-2xl font-semibold tabular-nums text-[var(--admin-ink)]"),
					)
			};
			View()
				.class("grid grid-cols-4 gap-3 max-sm:grid-cols-2")
				.child(figure("Visitors", format::count(result.visitors.value)))
				.child(figure("Pageviews", format::count(result.pageviews.value)))
				.child(figure("Sessions", format::count(result.sessions.value)))
				.child(figure(
					"Bounce rate",
					format::share(result.bounce_rate.value),
				))
		}
		Ok(_) => Text("This kind of insight has no card yet.").class(style::MUTED),
		Err(failure) => components::status_word(Tone::Bad, failure.clone()),
	};
	let built = components::card(card.name.clone())
		.aside(card.kind.clone())
		.test_id("dashboard-tile")
		.child(body)
		.child(components::link("Open →", card.href.clone()))
		.build();

	if card.wide {
		View()
			.class("col-span-2 flex-col max-lg:col-span-1")
			.child(built)
	} else {
		built
	}
}

/// A dashboard. It reads itself again as events arrive.
pub(crate) fn dashboard(dashboard: &Dashboard) -> Element {
	let mut grid = View()
		.class("grid grid-cols-2 gap-4 max-lg:grid-cols-1")
		.test_id("grid-tiles")
		.children(dashboard.cards.iter().map(card));

	if dashboard.cards.is_empty() {
		grid = grid.child(
			Text("No cards yet. Open any insight and press Add to dashboard.").class(style::MUTED),
		);
	}

	let count = dashboard.cards.len();

	View()
		.class("flex-col gap-5")
		.test_id("live-refresh")
		.child(components::link(
			"← Library",
			format!("{BASE}/analytics/library?tab=dashboards"),
		))
		.child(components::header(
			&dashboard.name,
			&format!(
				"dashboard · {count} card{} · updates live",
				if count == 1 { "" } else { "s" }
			),
			Vec::new(),
		))
		.child(grid)
}

fn actor(actor: &Actor) -> Element {
	Pressable(actor.name.clone())
		.href(actor.href.clone())
		.class("cursor-pointer rounded-lg hover:bg-[var(--admin-sunken)]")
		.test_id("actor")
		.child(
			View()
				.class(
					"flex-row items-center gap-3 border-b border-[var(--admin-line)] px-3 py-2.5",
				)
				.child(components::avatar(&actor.name, "h-8 w-8 text-xs leading-8"))
				.child(
					View()
						.class("min-w-0 flex-1 flex-col")
						.child(
							Text(actor.name.clone())
								.class("truncate text-sm font-semibold text-[var(--admin-ink)]"),
						)
						.child(Text(actor.summary.clone()).class(style::LABEL)),
				)
				.child(Text("→").class("text-sm text-[var(--admin-ink-3)]")),
		)
}

/// The people behind one point of a chart.
pub(crate) fn actors(actors: &Actors) -> Element {
	let mut list = View().class("flex-col").test_id("actors");
	let count = match &actors.people {
		Ok(people) => {
			list = list.children(people.iter().map(actor));

			if people.is_empty() {
				list = list.child(Text("Nobody here.").class(style::MUTED));
			}

			people.len()
		}
		Err(failure) => {
			list = list.child(components::status_word(Tone::Bad, failure.clone()));
			0
		}
	};
	let footer = View().class("flex-row gap-3").children(
		actors
			.next
			.as_ref()
			.map(|next| components::link_button(Kind::Secondary, "Next page", next.clone())),
	);

	View()
		.class("flex-col gap-5")
		.child(components::link("← Back to the chart", actors.back.clone()))
		.child(components::header(
			&actors.title,
			"Actors: the people behind one point of a query",
			Vec::new(),
		))
		.child(
			components::card(format!(
				"{count} {}",
				if count == 1 { "person" } else { "people" }
			))
			.aside(if actors.next.is_some() {
				"more on the next page"
			} else {
				""
			})
			.workings("actors", "Query::Actors")
			.child(list)
			.child(footer)
			.build(),
		)
}

/// What an analytics page says when it has nothing to read.
pub(crate) fn notice(notice: &Notice) -> Element {
	let title = Heading(1, notice.title.clone()).class(style::PAGE_TITLE);

	match &notice.why {
		Why::NotConfigured => {
			View()
				.class("flex-col gap-5")
				.test_id("analytics-not-configured")
				.child(title)
				.child(components::empty_state(
					"Connect the admin to your analytics",
					"The admin is its own app, so it reads your site's analytics the way your \
					 site's server does: from the analytics service, signed with the service's \
					 token. It hasn't been given either yet. Set both where the admin runs, then \
					 reload:",
					Some((CONNECT, "shell")),
					"The token is the app's: with it the admin can read every event, so keep it \
					 in the admin's environment, never in a page. AdminBuilder::analytics does \
					 the same from Rust.",
				))
				.child(
					components::card("Then, from your app")
						.aside("once the service runs")
						.child(
							Text(
								"Your app sends events with a Rust type for each, so a misspelled \
								 property fails when it compiles rather than in a chart:",
							)
							.class(style::SECONDARY),
						)
						.child(components::paper_code(FIRST_EVENT, "Rust"))
						.build(),
				)
		}
		Why::Unreachable(failure) => {
			View()
				.class("flex-col gap-5")
				.test_id("analytics-unreachable")
				.child(title)
				.child(
					components::card("The analytics service didn't answer")
						.child(components::status_word(Tone::Bad, "Not connected"))
						.child(Text(failure.clone()).class(style::BODY))
						.child(
							Text(
								"Nothing on this page is guessed, so it shows nothing until the \
								 service answers. Reload once it's back.",
							)
							.class(style::MUTED),
						)
						.build(),
				)
		}
		Why::Listening { shows } => {
			let check = |done: bool, text: &str| {
				View()
					.class("flex-row items-center gap-2")
					.child(if done {
						components::status_word(Tone::Good, "")
					} else {
						View()
							.class("h-2.5 w-2.5 rounded-full bg-[var(--admin-mint)] animate-pulse")
							.aria_hidden(true)
					})
					.child(Text(text.to_owned()).class(style::BODY))
			};

			View()
				.class("flex-col gap-5")
				.test_id("live-refresh")
				.child(title)
				.child(components::empty_state(
					"Waiting for your first event",
					&format!(
						"{shows} Your app sends events with a Rust type for each, so a misspelled \
						 property fails when it compiles rather than in a chart. Add \
						 leptoskit_analytics with the web and derive features, then send one:"
					),
					Some((FIRST_EVENT, "Rust")),
					"This page listens: it draws in by itself when the first event arrives.",
				))
				.child(
					components::card("Listening")
						.test_id("checklist")
						.child(check(true, "The admin reaches your analytics service"))
						.child(check(false, "Capture an event … waiting"))
						.build(),
				)
		}
	}
}
