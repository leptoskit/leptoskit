//! Signing in: one button that goes to the main site, and the page for
//! someone the main site signed in who isn't an admin. It's the one page
//! without the shell, since nobody is signed in to see one.

use leptos::prelude::Get as _;
use leptos::prelude::RwSignal;
use leptos::prelude::Set as _;
use leptos::prelude::Update as _;
use leptoskit::auth::ErrorResponse;
use leptoskit::auth::Prompt;
use leptoskit::auth::StartProvider;
use leptoskit::auth::client::sign_in_with_provider;
use leptoskit::native::Role;
use leptoskit::native::TextSpan;
use leptoskit::prelude::*;
use leptoskit::query::MutationHandle;
use leptoskit::query::use_mutation;

use crate::BASE;
use crate::components;
use crate::model::Viewer;
use crate::model::signin::Outcome;
use crate::model::signin::SignIn;
use crate::shell;
use crate::style;

/// What a person can do on the page: go to the main site, and open How
/// admins are chosen.
#[derive(Clone, Copy)]
pub(crate) struct Start {
	provider: MutationHandle<StartProvider, (), ErrorResponse>,
	/// Why the main site couldn't be reached, once it couldn't.
	problem: RwSignal<String>,
	/// Whether How admins are chosen is open.
	how: RwSignal<bool>,
}

impl Start {
	pub(crate) fn new() -> Self {
		Self {
			provider: use_mutation(sign_in_with_provider()),
			problem: RwSignal::new(String::new()),
			how: RwSignal::new(false),
		}
	}

	/// Goes to the main site to sign in, and back to the sign-in page.
	/// Switching asks it to sign someone in again rather than reuse the
	/// account it has.
	fn go(self, switch: bool) {
		self.problem.set(String::new());
		crate::spawn(async move {
			let started = self
				.provider
				.mutate_async(StartProvider {
					provider: "main".into(),
					redirect_to: Some(format!("{BASE}/sign-in")),
					prompt: switch.then_some(Prompt::Login),
				})
				.await;

			// Started, the page is already on its way to the main site.
			if let Err(refused) = started {
				self.problem.set(if refused.error == "network" {
					"Couldn't reach the main site. Check the admin's connection to it and try \
					 again."
						.to_owned()
				} else {
					refused.message
				});
			}
		});
	}
}

/// The main site's host, for "You'll sign in on notes.example".
#[must_use]
pub fn host(issuer: &str) -> &str {
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
		.role(Role::Main)
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

fn heading(viewer: &Viewer) -> Element {
	View()
		.class("flex-col gap-4")
		.child(
			View()
				.class("flex-row items-center gap-3")
				.child(shell::mark(40.0))
				.child(components::env_pill(viewer.environment))
				.children(viewer.simulated.then(components::simulated_word)),
		)
		.child(
			Heading(1, format!("{} admin", viewer.app_name))
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

fn how_admins_are_chosen(sign_in: &SignIn, start: Option<Start>) -> Element {
	let roles = sign_in.roles.join(" or ");
	let site = &sign_in.site;
	let open = start.is_some_and(|start| start.how.get());
	let mut toggle = Button(format!(
		"{} How admins are chosen",
		if open { "▾" } else { "▸" }
	))
	.aria_expanded(open)
	.aria_label("How admins are chosen")
	.class(
		"self-start flex-row items-center text-sm font-semibold text-[var(--admin-ink-2)] \
		 cursor-pointer hover:text-[var(--admin-ink)]",
	)
	.test_id("how-toggle");

	if let Some(start) = start {
		toggle = toggle.on_press(move || start.how.update(|open| *open = !*open));
	}

	let mut how = View().class("flex-col gap-3").child(toggle);

	if open {
		how = how.child(
			View()
				.class("flex-col gap-3")
				.test_id("how-panel")
				.child(
					Text(String::new())
						.spans([
							TextSpan::new(format!(
								"The admin lets in accounts that {site} gives the "
							)),
							TextSpan::new(roles).bold(),
							TextSpan::new(format!(
								" role. {site} decides who has it with its Roles hook, and sends \
								 the roles along when you sign in:"
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
		);
	}

	how
}

/// Why the main site couldn't be reached, when it couldn't.
fn problem(start: Option<Start>) -> Option<Element> {
	let problem = start.map(|start| start.problem.get())?;

	(!problem.is_empty()).then(|| {
		Status(problem)
			.class("text-sm text-[var(--admin-bad)]")
			.test_id("sign-in-problem")
	})
}

/// The sign-in page.
pub(crate) fn page(viewer: &Viewer, sign_in: &SignIn, start: Option<Start>) -> Element {
	let site = &sign_in.site;
	let busy = start.is_some_and(|start| start.provider.is_pending());
	let go = move |button: Element, switch: bool| {
		match start {
			Some(start) => button.on_press(move || start.go(switch)),
			None => button,
		}
	};

	let card = match &sign_in.outcome {
		Outcome::Ready | Outcome::Failed(_) => {
			let mut card = View()
				.class("flex-col gap-5")
				.test_id("sign-in")
				.child(heading(viewer));

			if let Outcome::Failed(code) = &sign_in.outcome {
				card = card.child(
					Status(format!(
						"Sign-in didn't finish ({code}). Nothing changed; try again, and if it \
						 keeps happening, check that {site} lists this admin's callback."
					))
					.class("text-sm text-[var(--admin-bad)]")
					.test_id("sign-in-error"),
				);
			}

			card.child(purpose(site))
				.child(
					View()
						.class("relative w-full")
						.child(View().class(style::BUTTON_PRIMARY_BLOCK).aria_hidden(true))
						.child(go(
							Button(format!("Continue with {}", sign_in.button))
								.class(style::BUTTON_PRIMARY)
								.class("w-full py-3 text-base")
								.aria_busy(busy)
								.test_id("continue"),
							false,
						)),
				)
				.children(problem(start))
				.child(
					Text(String::new())
						.spans([
							TextSpan::new(format!(
								"You'll sign in on {site} and come straight back here. Only \
								 accounts with the "
							)),
							TextSpan::new(sign_in.roles.join(" or ")).bold(),
							TextSpan::new(" role get in."),
						])
						.class(style::SECONDARY),
				)
				.child(how_admins_are_chosen(sign_in, start))
		}
		Outcome::Refused(who) => {
			let sentence = match who {
				Some(who) => format!("You're signed in as {who}, but this account isn't an admin."),
				None => format!("The account you signed in with on {site} isn't an admin here."),
			};
			View()
				.class("flex-col gap-5")
				.test_id("refusal")
				.child(heading(viewer))
				.child(Text(sentence).class("text-base font-semibold text-[var(--admin-ink)]"))
				.child(components::raised(go(
					Button("Switch account")
						.class(style::BUTTON_PRIMARY)
						.aria_busy(busy)
						.test_id("switch-account"),
					true,
				)))
				.children(problem(start))
				.child(Text(format!("{site} will ask who you are again.")).class(style::MUTED))
				.child(how_admins_are_chosen(sign_in, start))
		}
	};

	frame(card)
}
