//! `/welcome`: the first thirty seconds (`docs/agents/cloud-design.md` §7.3).
//!
//! It shows once, to an account that has only its personal organisation and
//! no projects, and asks one question. Anyone else goes on to their
//! organisation.

use leptoskit::native::Role;
use leptoskit::prelude::*;

use super::masthead;
use super::redirect;
use crate::components::Kind;
use crate::components::focus_ring;
use crate::components::link_button;
use crate::components::sheet;
use crate::components::sheet_rule;
use crate::console::Console;
use crate::console::Load;
use crate::console::Who;
use crate::console::first_organization;
use crate::console::looks_new;
use crate::data::PlanTerms;
use crate::data::plan_name;
use crate::routes::Navigation;
use crate::style;

/// Everything the welcome sheet shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WelcomeSheet {
	/// Who to greet: the personal organisation's name.
	pub name: String,
	/// The organisation's slug.
	pub organization: String,
	/// Where the console is, such as `cloud.leptoskit.dev`.
	pub host: String,
	/// The plan's name and, once `billing::plans` exists, what it includes.
	pub plan: PlanTerms,
	/// The plan's slug, for its name until then.
	pub plan_slug: String,
}

/// `/welcome`.
pub fn page(navigation: &Navigation) -> Element {
	let console = Console::get();

	match console.who() {
		Who::SignedOut => {
			redirect(navigation, &super::sign_in_href("/welcome"));
			return super::signed_out("/welcome");
		}
		Who::Checking => return super::loading("Loading your account", &["h-80 w-full max-w-md"]),
		Who::Unknown(error) => return super::session_failed(console, &error),
		Who::SignedIn(_) => {}
	}

	let me = match console.me() {
		Load::Loading => return super::loading("Loading your account", &["h-80 w-full max-w-md"]),
		Load::Failed(error) => {
			let whoami = console.whoami;
			return sheet([super::load_failed("your account", &error, move || {
				drop(whoami.refetch());
			})]);
		}
		Load::Ready(me) => me,
	};
	let Some(organization) = first_organization(&me).cloned() else {
		redirect(navigation, "/organizations");
		return super::loading("Loading your account", &["h-80 w-full max-w-md"]);
	};
	let not_new = match Load::of(&console.projects, crate::console::rpc_problem) {
		Load::Ready(projects) => !looks_new(&me, Some(projects.as_slice())),
		// The projects come only once the page is live (`Console::new`):
		// until then the sheet shows, unless `whoami` already says the
		// account isn't new.
		Load::Loading => !looks_new(&me, Some(&[])),
		// Without them the console can't tell; the organisation's page says
		// what went wrong.
		Load::Failed(_) => true,
	};

	if not_new {
		redirect(navigation, &format!("/{}", organization.slug));
	}

	let billing = console.services().billing;
	view(
		&console,
		&WelcomeSheet {
			name: organization.name.clone(),
			host: console.host(),
			plan: billing.plan(&organization.plan),
			plan_slug: organization.plan.clone(),
			organization: organization.slug,
		},
	)
}

/// The plan's sentence: what it includes when the console knows, and always
/// that nothing charges without a card.
#[must_use]
pub fn plan_sentence(plan: &PlanTerms, slug: &str) -> String {
	match plan {
		PlanTerms::Known { name, includes } => {
			format!(
				"You're on {name}: {includes}, and nothing here can charge you until you add a \
				 card."
			)
		}
		PlanTerms::NotAvailable => {
			format!(
				"You're on {}, and nothing here can charge you until you add a card.",
				plan_name(slug)
			)
		}
	}
}

/// The welcome sheet.
pub fn view(console: &Console, state: &WelcomeSheet) -> Element {
	let org = &state.organization;

	sheet([
		masthead(),
		View()
			.class("flex-col gap-2")
			.child(Heading(1, format!("Hello, {}.", state.name)).class(style::DISPLAY))
			.child(
				Text(format!("Your projects live at {}/{org}.", state.host))
					.class(style::SECONDARY),
			)
			.child(Text(plan_sentence(&state.plan, &state.plan_slug)).class(style::SECONDARY)),
		sheet_rule(),
		Text("Do you have a leptoskit app already?").class(style::CARD_TITLE),
		View()
			.class("flex-col items-start gap-3")
			.role(Role::Group)
			.accessibility_label("Do you have a leptoskit app already?")
			.child(focus_ring(
				console.focus,
				"welcome-terminal",
				link_button(
					Kind::Primary,
					"Yes, ship it from my terminal",
					format!("/{org}/new?path=terminal"),
				),
			))
			.child(focus_ring(
				console.focus,
				"welcome-starter",
				link_button(
					Kind::Secondary,
					"Not yet, ship a starter",
					format!("/{org}/new?path=starter"),
				),
			)),
		focus_ring(
			console.focus,
			"welcome-look",
			link_button(Kind::Quiet, "Look around first", format!("/{org}")),
		),
	])
	.test_id("welcome")
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_plan_is_named_without_terms_the_cloud_has_not_given() {
		assert_eq!(
			plan_sentence(&PlanTerms::NotAvailable, "hobby"),
			"You're on Hobby, and nothing here can charge you until you add a card."
		);
		assert_eq!(
			plan_sentence(
				&PlanTerms::Known {
					name: "Hobby".to_owned(),
					includes: "free, one project".to_owned(),
				},
				"hobby"
			),
			"You're on Hobby: free, one project, and nothing here can charge you until you add a \
			 card."
		);
	}
}
