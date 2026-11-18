//! `/sign-in` and `/sign-in/email` (`docs/agents/cloud-design.md` §7.1–7.2).
//!
//! One sheet for signing in and signing up: the server knows whether an
//! account exists, so the person never has to. GitHub comes first, an
//! emailed code second, then a quiet row of the other ways that can work
//! here. A way the server hasn't set up never shows.

use leptoskit::auth::ErrorResponse;
use leptoskit::auth::ProviderOutcome;
use leptoskit::auth::StartProvider;
use leptoskit::auth::client::EmailCode;
use leptoskit::auth::client::PasskeyMode;
use leptoskit::auth::client::SignInStep;
use leptoskit::native::Role;
use leptoskit::prelude::*;
use leptoskit::query::MutateCallbacks;
use leptoskit_router::Routable as _;

use super::masthead;
use super::redirect;
use crate::components::Connection;
use crate::components::Kind;
use crate::components::block_button;
use crate::components::button;
use crate::components::field;
use crate::components::focus_ring;
use crate::components::link;
use crate::components::margin_note;
use crate::components::notice;
use crate::components::plain_button;
use crate::components::problem;
use crate::components::sheet;
use crate::components::sheet_rule;
use crate::console::Console;
use crate::console::Who;
use crate::console::local_path;
use crate::routes::Navigation;
use crate::routes::Route;
use crate::routes::SignIn;
use crate::routes::SignInEmail;
use crate::style;

/// How long before another code can be sent.
pub const RESEND_AFTER: u32 = 30;

/// The ways to sign in that can work here.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Methods {
	/// The server set GitHub up.
	pub github: bool,
	/// The server set Google up.
	pub google: bool,
	/// This browser can use passkeys.
	pub passkey: bool,
	/// A Solana wallet this browser offers, by name, when the server lets
	/// wallets sign in.
	pub wallet: Option<String>,
}

/// Which way to sign in is running.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Busy {
	/// None.
	#[default]
	Idle,
	/// Going to GitHub.
	GitHub,
	/// Going to Google.
	Google,
	/// Sending a code.
	Email,
	/// Waiting for a passkey.
	Passkey,
	/// Waiting for a wallet.
	Wallet,
}

/// Everything the sign-in sheet shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignInSheet {
	/// Where to go after, on this site.
	pub next: Option<String>,
	/// The ways that can work here; `None` while the server's list loads.
	pub methods: Option<Methods>,
	/// Whether a terminal sent the person here.
	pub from_terminal: bool,
	/// What went wrong last: a provider's answer, or the last action's error.
	pub problem: Option<String>,
	/// What's wrong with the address, found before anything is sent.
	pub email_problem: Option<String>,
	/// The account asked for a second step after a provider signed in.
	pub second_step: bool,
	/// What's running.
	pub busy: Busy,
	/// The console can't reach the cloud.
	pub offline: bool,
}

impl SignInSheet {
	/// The sheet the console's state calls for on `route`.
	#[must_use]
	pub fn read(console: &Console, route: &SignIn) -> Self {
		let flow = console.sign_in;
		let next = local_path(route.next.as_deref());
		let outcome = outcome(route);
		let methods = console.providers.data().map(|providers| {
			let has = |id: &str| providers.iter().any(|provider| provider.id == id);
			let wallet_allowed = console
				.wallet_settings
				.data()
				.is_some_and(|settings| settings.enabled && settings.sign_in);
			Methods {
				github: has("github"),
				google: has("google"),
				passkey: console.mounted.get() && cfg!(target_arch = "wasm32"),
				wallet: wallet_allowed
					.then(|| {
						console.wallet_apps.data().and_then(|apps| {
							apps.into_iter().find(|app| app.sign_in).map(|app| app.name)
						})
					})
					.flatten(),
			}
		});
		let busy = if flow.provider.is_pending() {
			match flow.opening.get().as_deref() {
				Some("google") => Busy::Google,
				_ => Busy::GitHub,
			}
		} else if flow.send.is_pending() {
			Busy::Email
		} else if flow.passkey.is_pending() {
			Busy::Passkey
		} else if flow.wallet.is_pending() {
			Busy::Wallet
		} else {
			Busy::Idle
		};
		let action_error = flow
			.provider
			.error()
			.or_else(|| flow.send.error())
			.or_else(|| flow.passkey.error())
			.or_else(|| flow.wallet.error())
			.or_else(|| flow.finish.error())
			.map(|error| sign_in_problem(&error));
		let second_step = matches!(outcome, Some(ProviderOutcome::SecondFactor(_)))
			|| matches!(flow.wallet.data(), Some(SignInStep::SecondFactor(_)));
		let provider = route.via.as_deref().unwrap_or("github");

		Self {
			from_terminal: next
				.as_deref()
				.is_some_and(|next| next.starts_with("/device")),
			next,
			methods,
			problem: action_error.or_else(|| {
				outcome
					.as_ref()
					.and_then(|outcome| outcome_problem(outcome, provider))
			}),
			email_problem: flow.problem.get(),
			second_step,
			busy,
			offline: console.connection.get() == Connection::Offline,
		}
	}
}

/// How a provider's sign-in ended, from the address it sent the person back
/// to.
fn outcome(route: &SignIn) -> Option<ProviderOutcome> {
	let mut query = Vec::new();

	if let Some(auth) = &route.auth {
		query.push(format!("auth={auth}"));
	}

	if let Some(error) = &route.auth_error {
		query.push(format!("auth_error={error}"));
	}

	if let Some(factors) = &route.auth_factors {
		query.push(format!("auth_factors={factors}"));
	}

	ProviderOutcome::from_query(&query.join("&"))
}

/// A provider's name, from its ID.
fn provider_name(id: &str) -> &'static str {
	match id {
		"google" => "Google",
		_ => "GitHub",
	}
}

/// What to say about a provider's answer, when it needs saying.
#[must_use]
pub fn outcome_problem(outcome: &ProviderOutcome, provider: &str) -> Option<String> {
	let name = provider_name(provider);

	match outcome {
		ProviderOutcome::Failed(code) => {
			let why = match code.as_str() {
				"cancelled" | "access_denied" => "the request was cancelled".to_owned(),
				"provider" => format!("{name} couldn't be reached"),
				other => format!("it answered `{other}`"),
			};
			Some(format!(
				"{name} didn't sign you in: {why}. Try again, or use another way."
			))
		}
		ProviderOutcome::EmailInUse => {
			Some(format!(
				"An account with this email already signs in another way. Sign in that way to add \
				 {name} to it."
			))
		}
		ProviderOutcome::SignedIn
		| ProviderOutcome::SecondFactor(_)
		| ProviderOutcome::Linked
		| ProviderOutcome::AlreadyLinked
		| ProviderOutcome::Merge => None,
	}
}

/// What to say when signing in failed, from the server's error.
#[must_use]
pub fn sign_in_problem(error: &ErrorResponse) -> String {
	match error.error.as_str() {
		"invalid_email" => "That doesn't look like an email address.".to_owned(),
		"invalid_code" => {
			"That code didn't match, or it has expired. Use the newest email from LeptosKit Cloud, \
			 or send another code."
				.to_owned()
		}
		"rate_limited" => {
			format!(
				"Too many tries from this network. {}",
				retry_sentence(error.retry_after)
			)
		}
		"sign_in_expired" => "That took too long. Start again from the email.".to_owned(),
		"cancelled" => "The request was cancelled. Try again, or use another way.".to_owned(),
		"passkey_failed" => {
			"Your passkey didn't sign you in. Try again, or use another way.".to_owned()
		}
		"wallet_failed" => {
			"Your wallet didn't sign you in. Try again, or use another way.".to_owned()
		}
		_ => sentence(&error.message),
	}
}

/// "Try again in 12 minutes.", from the server's 720 seconds.
fn retry_sentence(seconds: Option<u64>) -> String {
	match seconds {
		Some(seconds) if seconds >= 90 => format!("Try again in {} minutes.", seconds.div_ceil(60)),
		Some(seconds) => {
			format!(
				"Try again in {seconds} second{}.",
				if seconds == 1 { "" } else { "s" }
			)
		}
		None => "Try again later.".to_owned(),
	}
}

/// The server's message as a sentence: a capital and a full stop.
fn sentence(message: &str) -> String {
	let mut chars = message.trim().chars();
	let mut sentence: String = chars
		.next()
		.map(|first| first.to_uppercase().chain(chars).collect())
		.unwrap_or_default();

	if !sentence.ends_with('.') {
		sentence.push('.');
	}

	sentence
}

/// Whether `email` could be an address: something, an `@`, and a domain
/// with a dot. The server checks it properly.
#[must_use]
pub fn looks_like_email(email: &str) -> bool {
	let email = email.trim();
	let Some((local, domain)) = email.split_once('@') else {
		return false;
	};

	!local.is_empty()
		&& domain.contains('.')
		&& !domain.starts_with('.')
		&& !domain.ends_with('.')
		&& !email.contains(char::is_whitespace)
}

/// `/sign-in`.
pub fn page(route: &SignIn, navigation: &Navigation) -> Element {
	let console = Console::get();
	let next = local_path(route.next.as_deref());

	if let Who::SignedIn(_) = console.who() {
		let to = next.unwrap_or_else(|| "/welcome".to_owned());
		redirect(navigation, &to);
		return continuing(&to);
	}

	// A sent code goes on to its own sheet.
	if console.sign_in.send.is_success()
		&& let Some(to) = console.sign_in.sent_to.get()
	{
		redirect(navigation, &email_href(&to, next.as_deref()));
	}

	view(&console, &SignInSheet::read(&console, route))
}

/// Signed in already: on to where they were going.
fn continuing(to: &str) -> Element {
	sheet([
		masthead(),
		Heading(1, "You're signed in.").class(style::DISPLAY),
		link("Continue →", to.to_owned()),
	])
}

/// `/sign-in/email?to=…&next=…`.
fn email_href(to: &str, next: Option<&str>) -> String {
	Route::SignInEmail(SignInEmail {
		to: Some(to.to_owned()),
		next: next.map(str::to_owned),
	})
	.href()
	.to_string()
}

/// `/sign-in?via=<provider>&next=…`: where a provider sends the person back
/// to, so its answer can be named.
fn return_href(provider: &str, next: Option<&str>) -> String {
	Route::SignIn(SignIn {
		next: next.map(str::to_owned),
		via: Some(provider.to_owned()),
		auth: None,
		auth_error: None,
		auth_factors: None,
	})
	.href()
	.to_string()
}

/// The sign-in sheet.
pub fn view(console: &Console, state: &SignInSheet) -> Element {
	let console = *console;
	let flow = console.sign_in;
	let ring = |id: &str, control: Element| focus_ring(console.focus, id, control);
	let busy = state.busy;
	let disabled = state.offline || busy != Busy::Idle;
	let next = state.next.clone();

	let mut lines = vec![
		masthead(),
		View()
			.class("flex-col gap-2")
			.child(Heading(1, "Sign here.").class(style::DISPLAY))
			.child(
				Text(
					"One account for your apps' servers, databases, over-the-air updates and \
					 store releases.",
				)
				.class(style::SECONDARY),
			),
	];

	if state.from_terminal {
		lines.push(margin_note(
			"Your terminal is waiting. Sign in, then approve it.",
		));
	}

	lines.push(sheet_rule());
	lines.push(
		View()
			.class("flex-col gap-2")
			.role(Role::List)
			.accessibility_label("What you get")
			.children(
				[
					"A server and database for one app, free",
					"Every release can be rolled back",
					"A spend cap from the first minute",
				]
				.into_iter()
				.map(|line| {
					View()
						.class("flex-row items-start gap-2")
						.role(Role::ListItem)
						.child(
							Text("✓")
								.class("text-sm font-semibold text-[var(--cloud-good)]")
								.aria_hidden(true),
						)
						.child(Text(line).class(style::BODY).class("flex-1"))
				}),
			),
	);
	lines.push(sheet_rule());

	if let Some(problem_text) = &state.problem {
		lines.push(problem(problem_text.clone()));
	}

	if state.offline {
		lines.push(problem("You're offline. Signing in needs a connection."));
	}

	if state.second_step {
		lines.push(second_step(&console));
	}

	match &state.methods {
		None => {
			lines.push(super::loading(
				"Loading the ways to sign in",
				&["h-11 w-full"],
			));
		}
		Some(methods) => {
			if methods.github {
				let next = next.clone();
				lines.push(
					ring(
						"sign-in-github",
						block_button(
							"Continue with GitHub",
							(busy == Busy::GitHub).then_some("Opening GitHub…"),
							move || {
								flow.opening.set(Some("github".to_owned()));
								flow.provider.mutate(StartProvider {
									provider: "github".to_owned(),
									redirect_to: Some(return_href("github", next.as_deref())),
								});
							},
						)
						.class("w-full"),
					)
					.class("w-full"),
				);
			}

			lines.push(email_row(&console, state, disabled));

			let quiet = quiet_ways(&console, methods, state, disabled);

			if !quiet.is_empty() {
				lines.push(
					View()
						.class("flex-row flex-wrap items-center gap-x-1")
						.role(Role::Group)
						.accessibility_label("Other ways to sign in")
						.children(quiet),
				);
			}
		}
	}

	lines.push(sheet_rule());
	lines.push(
		Text("By continuing you accept the Terms and the Privacy notice.")
			.class("text-xs leading-normal text-[var(--cloud-ink-3)]"),
	);

	sheet(lines).test_id("sign-in")
}

/// The address field and its button, which wraps under it on phones.
fn email_row(console: &Console, state: &SignInSheet, disabled: bool) -> Element {
	let console = *console;
	let flow = console.sign_in;
	let typing = console.typing;
	let send = move || {
		let email = flow.email.get_untracked().trim().to_owned();

		if !looks_like_email(&email) {
			flow.problem
				.set(Some("That doesn't look like an email address.".to_owned()));
			return;
		}

		flow.problem.set(None);
		flow.sent_to.set(None);
		let sent = email.clone();
		let resend_in = flow.resend_in;
		let sent_to = flow.sent_to;
		flow.send.mutate_with(
			email,
			MutateCallbacks::new().on_success(move |_| {
				sent_to.set(Some(sent));
				countdown(resend_in, RESEND_AFTER);
			}),
		);
	};
	let input = TextInput()
		.value(flow.email.get())
		.placeholder("you@example.com")
		.autocomplete("email")
		.enabled(!disabled)
		.on_change(move |text: &str| flow.email.set(text.to_owned()))
		.on(NativeEventSlot::Focus, move |_| {
			typing.set(true);
			EventOutcome::Unhandled
		})
		.on(NativeEventSlot::Blur, move |_| {
			typing.set(false);
			EventOutcome::Unhandled
		})
		.on_key(move |key| {
			if key.is("Enter", false, false, false) {
				send();
				return EventOutcome::Handled;
			}

			EventOutcome::Unhandled
		});
	let mut action = plain_button(
		Kind::Secondary,
		"Email me a code",
		(state.busy == Busy::Email).then_some("Sending…"),
		send,
	);

	if disabled && state.busy != Busy::Email {
		action = action.enabled(false);
	}

	View()
		.class("flex-row flex-wrap items-end gap-2 w-full")
		.child(View().class("flex-1 min-w-[200px]").child(field(
			"sign-in-email",
			"Email",
			input,
			state.email_problem.as_deref(),
		)))
		// The ring hugs its button (`self-start` in its own column); this
		// box puts the pair on the field's baseline in the row.
		.child(
			View()
				.class("self-end")
				.child(focus_ring(console.focus, "sign-in-send", action)),
		)
}

/// Passkey, Google and a Solana wallet: each only when it can work here.
fn quiet_ways(
	console: &Console,
	methods: &Methods,
	state: &SignInSheet,
	disabled: bool,
) -> Vec<Element> {
	let console = *console;
	let flow = console.sign_in;
	let busy = state.busy;
	let mut ways = Vec::new();
	let quiet = |label: &str, busy_label: Option<&str>, action: Box<dyn Fn() + Send + Sync>| {
		let mut control = plain_button(Kind::Quiet, label, busy_label, action);

		if disabled && busy_label.is_none() {
			control = control.enabled(false);
		}

		control
	};

	if methods.passkey {
		ways.push(quiet(
			"Passkey",
			(busy == Busy::Passkey).then_some("Waiting for your passkey…"),
			Box::new(move || flow.passkey.mutate(PasskeyMode::Prompt)),
		));
	}

	if methods.google {
		let next = state.next.clone();
		ways.push(quiet(
			"Google",
			(busy == Busy::Google).then_some("Opening Google…"),
			Box::new(move || {
				flow.opening.set(Some("google".to_owned()));
				flow.provider.mutate(StartProvider {
					provider: "google".to_owned(),
					redirect_to: Some(return_href("google", next.as_deref())),
				});
			}),
		));
	}

	if let Some(wallet) = &methods.wallet {
		let wallet = wallet.clone();
		ways.push(quiet(
			"Solana wallet",
			(busy == Busy::Wallet).then_some("Waiting for your wallet…"),
			Box::new(move || flow.wallet.mutate(wallet.clone())),
		));
	}

	let count = ways.len();
	ways.into_iter()
		.enumerate()
		.flat_map(|(index, way)| {
			let separator = (index + 1 < count).then(|| {
				Text("·")
					.class("text-sm text-[var(--cloud-ink-3)]")
					.aria_hidden(true)
			});
			std::iter::once(way).chain(separator)
		})
		.collect()
}

/// The second step, for an account with two-step verification.
fn second_step(console: &Console) -> Element {
	let console = *console;
	let flow = console.sign_in;
	let finish = move || {
		flow.finish
			.mutate(flow.factor_code.get_untracked().trim().to_owned());
	};
	let input = TextInput()
		.value(flow.factor_code.get())
		.autocomplete("one-time-code")
		.focused(true)
		.on_change(move |text: &str| flow.factor_code.set(text.to_owned()))
		.on_key(move |key| {
			if key.is("Enter", false, false, false) {
				finish();
				return EventOutcome::Handled;
			}

			EventOutcome::Unhandled
		});

	View()
		.class("flex-col gap-3")
		.role(Role::Group)
		.accessibility_label("Two-step verification")
		.child(
			Text(
				"This account has two-step verification. Enter the code from your authenticator \
				 app, or a recovery code.",
			)
			.class(style::BODY),
		)
		.child(field(
			"sign-in-factor",
			"Authenticator or recovery code",
			input,
			None,
		))
		.child(focus_ring(
			console.focus,
			"sign-in-finish",
			button(
				Kind::Primary,
				"Continue",
				flow.finish.is_pending().then_some("Checking…"),
				finish,
			),
		))
}

/// Counts `seconds` down to zero, a second at a time, in the browser. Off
/// the browser it only sets the start, which the tests read.
fn countdown(remaining: RwSignal<u32>, seconds: u32) {
	remaining.set(seconds);

	#[cfg(all(target_arch = "wasm32", feature = "hydrate"))]
	{
		use std::cell::Cell;
		use std::rc::Rc;

		let handle = Rc::new(Cell::new(None::<leptos::prelude::IntervalHandle>));
		let held = Rc::clone(&handle);
		let started = leptos::prelude::set_interval(
			move || {
				let left = remaining.get_untracked().saturating_sub(1);
				remaining.set(left);

				if left == 0
					&& let Some(handle) = held.take()
				{
					handle.clear();
				}
			},
			std::time::Duration::from_secs(1),
		);

		if let Ok(started) = started {
			handle.set(Some(started));
		}
	}
}

/// `0:24`.
#[must_use]
pub fn clock(seconds: u32) -> String {
	format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// Everything the code sheet shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmailSheet {
	/// Where the code went.
	pub to: String,
	/// Where to go after, on this site.
	pub next: Option<String>,
	/// Whether the account asked for a second step.
	pub second_step: bool,
	/// What went wrong last.
	pub problem: Option<String>,
	/// What the sheet is waiting for.
	pub waiting: EmailWait,
	/// Whether another code went out.
	pub resent: bool,
	/// Seconds until another code can be sent.
	pub resend_in: u32,
}

/// What the code sheet is waiting for: one press at a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmailWait {
	/// Nothing.
	Nothing,
	/// The code to be checked.
	Checking,
	/// Another code to go out.
	Sending,
}

/// `/sign-in/email`.
pub fn email_page(route: &SignInEmail, navigation: &Navigation) -> Element {
	let console = Console::get();
	let flow = console.sign_in;
	let next = local_path(route.next.as_deref());

	if let Who::SignedIn(_) = console.who() {
		let to = next.unwrap_or_else(|| "/welcome".to_owned());
		redirect(navigation, &to);
		return continuing(&to);
	}

	let Some(to) = route.to.clone().filter(|to| looks_like_email(to)) else {
		let back = next
			.as_deref()
			.map_or_else(|| "/sign-in".to_owned(), super::sign_in_href);
		redirect(navigation, &back);
		return continuing(&back);
	};

	let state = EmailSheet {
		second_step: matches!(flow.verify.data(), Some(SignInStep::SecondFactor(_))),
		problem: flow
			.verify
			.error()
			.or_else(|| flow.finish.error())
			.or_else(|| flow.send.error())
			.map(|error| sign_in_problem(&error)),
		waiting: if flow.verify.is_pending() {
			EmailWait::Checking
		} else if flow.send.is_pending() {
			EmailWait::Sending
		} else {
			EmailWait::Nothing
		},
		resent: flow.send.is_success() && flow.sent_to.get().as_deref() == Some(to.as_str()),
		resend_in: flow.resend_in.get(),
		to,
		next,
	};

	email_view(&console, navigation, &state)
}

/// The code sheet.
pub fn email_view(console: &Console, navigation: &Navigation, state: &EmailSheet) -> Element {
	let console = *console;
	let flow = console.sign_in;
	let to = state.to.clone();
	let typing = console.typing;
	let verify = {
		let to = to.clone();
		move || {
			flow.verify.mutate(EmailCode {
				email: to.clone(),
				code: flow.code.get_untracked().trim().to_owned(),
			});
		}
	};
	let input = TextInput()
		.value(flow.code.get())
		.autocomplete("one-time-code")
		.focused(!state.second_step)
		.on_change(move |text: &str| flow.code.set(text.to_owned()))
		.on(NativeEventSlot::Focus, move |_| {
			typing.set(true);
			EventOutcome::Unhandled
		})
		.on(NativeEventSlot::Blur, move |_| {
			typing.set(false);
			EventOutcome::Unhandled
		})
		.on_key({
			let verify = verify.clone();
			move |key| {
				if key.is("Enter", false, false, false) {
					verify();
					return EventOutcome::Handled;
				}

				EventOutcome::Unhandled
			}
		});
	let different = {
		let navigation = navigation.clone();
		let back = super::sign_in_href(state.next.as_deref().unwrap_or("/"));
		move || {
			flow.send.reset();
			flow.verify.reset();
			flow.code.set(String::new());

			if let Some(route) = crate::console::route_of(&back) {
				navigation.navigate(route);
			}
		}
	};
	let resend = {
		let to = to.clone();
		move || {
			let resend_in = flow.resend_in;
			let sent_to = flow.sent_to;
			let sent = to.clone();
			flow.send.mutate_with(
				to.clone(),
				MutateCallbacks::new().on_success(move |_| {
					sent_to.set(Some(sent));
					countdown(resend_in, RESEND_AFTER);
				}),
			);
		}
	};
	let resend_label = if state.resend_in > 0 {
		format!("Send another code in {}", clock(state.resend_in))
	} else {
		"Send another code".to_owned()
	};
	let mut resend_button = plain_button(
		Kind::Quiet,
		resend_label,
		(state.waiting == EmailWait::Sending).then_some("Sending…"),
		resend,
	);

	if state.resend_in > 0 {
		resend_button = resend_button.enabled(false);
	}

	let mut lines = vec![
		masthead(),
		View()
			.class("flex-col gap-2")
			.child(Heading(1, "Check your email.").class(style::DISPLAY))
			.child(Text(format!("We sent a code to {}.", state.to)).class(style::SECONDARY)),
	];

	if let Some(problem_text) = &state.problem {
		lines.push(problem(problem_text.clone()));
	}

	if state.second_step {
		lines.push(second_step(&console));
	} else {
		lines.push(field("sign-in-code", "Code from the email", input, None));
		lines.push(
			focus_ring(
				console.focus,
				"sign-in-verify",
				block_button(
					"Continue",
					(state.waiting == EmailWait::Checking).then_some("Checking…"),
					verify,
				),
			)
			.class("w-full"),
		);
	}

	if state.resent && state.resend_in > 0 {
		lines.push(notice(format!("We sent another code to {}.", state.to)));
	}

	lines.push(
		View()
			.class("flex-row flex-wrap items-center gap-2")
			.child(focus_ring(
				console.focus,
				"sign-in-different",
				plain_button(Kind::Quiet, "Use a different email", None, different),
			))
			.child(focus_ring(console.focus, "sign-in-resend", resend_button)),
	);

	sheet(lines).test_id("sign-in-email")
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn addresses_are_checked_loosely_before_sending() {
		assert!(looks_like_email("ana@example.com"));
		assert!(looks_like_email(" ana@example.com "));
		assert!(!looks_like_email("ana"));
		assert!(!looks_like_email("ana@example"));
		assert!(!looks_like_email("@example.com"));
		assert!(!looks_like_email("ana @example.com"));
		assert!(!looks_like_email("ana@.com"));
	}

	#[test]
	fn the_wait_after_too_many_tries_is_read_from_the_server() {
		assert_eq!(retry_sentence(Some(720)), "Try again in 12 minutes.");
		assert_eq!(retry_sentence(Some(1)), "Try again in 1 second.");
		assert_eq!(retry_sentence(None), "Try again later.");
	}

	#[test]
	fn provider_answers_are_named() {
		assert_eq!(
			outcome_problem(&ProviderOutcome::Failed("cancelled".to_owned()), "github").as_deref(),
			Some(
				"GitHub didn't sign you in: the request was cancelled. Try again, or use another \
				 way."
			)
		);
		assert_eq!(
			outcome_problem(&ProviderOutcome::Failed("provider".to_owned()), "google").as_deref(),
			Some(
				"Google didn't sign you in: Google couldn't be reached. Try again, or use another \
				 way."
			)
		);
		assert_eq!(outcome_problem(&ProviderOutcome::SignedIn, "github"), None);
	}

	#[test]
	fn server_errors_become_sentences() {
		let error = |code: &str, message: &str| {
			ErrorResponse {
				error: code.to_owned(),
				message: message.to_owned(),
				retry_after: None,
			}
		};
		assert_eq!(
			sign_in_problem(&error("invalid_code", "that code didn't work")),
			"That code didn't match, or it has expired. Use the newest email from LeptosKit \
			 Cloud, or send another code."
		);
		assert_eq!(
			sign_in_problem(&error("disabled", "this account is disabled")),
			"This account is disabled."
		);
	}

	#[test]
	fn countdowns_read_as_a_clock() {
		assert_eq!(clock(24), "0:24");
		assert_eq!(clock(30), "0:30");
		assert_eq!(clock(61), "1:01");
	}
}
