//! Signing in: one button that goes to the main site, and the screen for
//! someone the main site signed in who isn't an admin.

use leptoskit::native::TextSpan;
use leptoskit::prelude::*;

use crate::Inner;
use crate::components;
use crate::shell;
use crate::style;

/// What the sign-in screen says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
	/// Nobody's signed in yet.
	Ready,
	/// The main site signed them in, but they don't have an admin role. The
	/// name is theirs when the admin knows it.
	Refused(Option<String>),
	/// Sign-in didn't finish, with the code the callback gave.
	Failed(String),
}

/// The main site's host, for "You'll sign in on notes.example".
fn host(issuer: &str) -> &str {
	issuer
		.split_once("://")
		.map_or(issuer, |(_, rest)| rest)
		.trim_end_matches('/')
}

/// The card, raised on an ink block like the primary button.
fn frame(card: Element) -> Element {
	View()
		.class("min-h-screen items-center justify-center px-4 py-12")
		.class(style::PAGE)
		.role(leptoskit::native::Role::Main)
		.child(
			View()
				.class("relative w-full max-w-md")
				.child(
					View()
						.class(
							"absolute inset-0 translate-x-2 translate-y-2 rounded-2xl \
							 bg-[var(--admin-ink)]",
						)
						.aria_hidden(true),
				)
				.child(
					View()
						.class(
							"relative w-full flex-col gap-5 rounded-2xl border \
							 border-[var(--admin-line)] bg-[var(--admin-surface)] p-6 md:p-8",
						)
						.child(card),
				),
		)
}

fn heading(inner: &Inner) -> Element {
	View()
		.class("flex-col gap-4")
		.child(
			View()
				.class("flex-row items-center gap-3")
				.child(shell::mark(40.0))
				.child(components::env_pill(inner.environment))
				.children(
					inner
						.simulated_infrastructure
						.then(components::simulated_word),
				),
		)
		.child(
			Heading(1, format!("{} admin", inner.app_name))
				.class("text-2xl font-bold text-[var(--admin-ink)]"),
		)
}

/// "Analytics, infrastructure, and lessons for **notes.example**."
fn purpose(site: &str) -> Element {
	Text(String::new())
		.spans([
			TextSpan::new("Analytics, infrastructure, and lessons for "),
			TextSpan::new(site).bold(),
			TextSpan::new("."),
		])
		.class("text-base leading-normal text-[var(--admin-ink-2)]")
}

const ROLES_HOOK: &str = "impl auth::Roles for User {
    async fn roles(db: &Db, account: Id<Account>) -> auth::Result<Vec<String>> {
        match User::find(db, account).await? {
            Some(user) if user.admin => Ok(vec![\"admin\".into()]),
            _ => Ok(Vec::new()),
        }
    }
}";

fn how_admins_are_chosen(inner: &Inner) -> Element {
	let roles = inner.roles.join(" or ");
	let where_ = host(&inner.main_site.issuer).to_owned();

	View()
		.class("flex-col gap-3")
		.child(
			Button("How admins are chosen")
				.aria_expanded(false)
				.class(
					"self-start flex-row items-center text-sm font-semibold \
					 text-[var(--admin-ink-2)] cursor-pointer hover:text-[var(--admin-ink)]",
				)
				.test_id("how-toggle"),
		)
		.child(
			View()
				.class("flex-col gap-3")
				.test_id("how-panel")
				.child(
					Text(String::new())
						.spans([
							TextSpan::new(format!(
								"The admin lets in accounts that {where_} gives the "
							)),
							TextSpan::new(roles).bold(),
							TextSpan::new(format!(
								" role. {where_} decides who has it with its Roles hook, and \
								 sends the roles along when you sign in:"
							)),
						])
						.class(style::SECONDARY),
				)
				.child(components::paper_code(ROLES_HOOK, "Rust"))
				.child(
					Text(
						"Give the account the role there, then sign in again. The admin reads \
						 roles at sign-in, so a change applies from the next one.",
					)
					.class(style::MUTED),
				),
		)
}

/// The sign-in screen.
pub(crate) fn screen(inner: &Inner, outcome: &Outcome) -> Element {
	let site = host(&inner.main_site.issuer).to_owned();

	let card = match outcome {
		Outcome::Ready | Outcome::Failed(_) => {
			let mut card = View()
				.class("flex-col gap-5")
				.test_id("sign-in")
				.child(heading(inner));

			if let Outcome::Failed(code) = outcome {
				card = card.child(
					Status(format!(
						"Sign-in didn't finish ({code}). Nothing changed; try again, and if it \
						 keeps happening, check that {site} lists this admin's callback."
					))
					.class("text-sm text-[var(--admin-bad)]")
					.test_id("sign-in-error"),
				);
			}

			let roles = inner
				.roles
				.iter()
				.map(String::as_str)
				.collect::<Vec<_>>()
				.join(" or ");

			card.child(purpose(&site))
				.child(
					View()
						.class("relative w-full")
						.child(View().class(style::BUTTON_PRIMARY_BLOCK).aria_hidden(true))
						.child(
							Button(format!("Continue with {}", inner.button))
								.class(style::BUTTON_PRIMARY)
								.class("w-full py-3 text-base")
								.test_id("continue"),
						),
				)
				.child(
					Text(String::new())
						.spans([
							TextSpan::new(format!(
								"You'll sign in on {site} and come straight back here. Only \
								 accounts with the "
							)),
							TextSpan::new(roles).bold(),
							TextSpan::new(" role get in."),
						])
						.class(style::SECONDARY),
				)
				.child(how_admins_are_chosen(inner))
		}
		Outcome::Refused(who) => {
			let sentence = match who {
				Some(who) => format!("You're signed in as {who}, but this account isn't an admin."),
				None => {
					format!("The account you signed in with on {site} isn't an admin here.")
				}
			};
			View()
				.class("flex-col gap-5")
				.test_id("refusal")
				.child(heading(inner))
				.child(Text(sentence).class("text-base font-semibold text-[var(--admin-ink)]"))
				.child(components::raised(
					Button("Switch account")
						.class(style::BUTTON_PRIMARY)
						.test_id("switch-account"),
				))
				.child(Text(format!("{site} will ask who you are again.")).class(style::MUTED))
				.child(how_admins_are_chosen(inner))
		}
	};

	frame(card)
}

/// The sign-in screen as a document.
pub(crate) fn page(inner: &Inner, outcome: &Outcome, appearance: Option<&str>) -> String {
	let title = match outcome {
		Outcome::Refused(_) => format!("Not an admin · {} admin", inner.app_name),
		_ => format!("Sign in · {} admin", inner.app_name),
	};
	let css = shell::shell_css(&[]);
	let root = shell::root_attributes(appearance, false, None);
	shell::document(&title, &root, &css, screen(inner, outcome))
}
