//! The sign-in screen: an email address, then the code emailed to it. The
//! same two steps sign up an address that has no account yet, so there's no
//! separate sign-up screen. An account with two-step verification turned on
//! gets a third step: a passkey, a code from its authenticator app, or a
//! recovery code.
//!
//! A passkey signs in on its own: from the button, or, in a browser that
//! offers them, from the email field's suggestions as soon as the screen
//! opens. Each provider the server set up gets a "Continue with …" button,
//! and, when the server lets wallets sign in, each Solana wallet app in the
//! browser gets a "Sign in with …" button. Native apps answer `unsupported`
//! to passkeys and providers for now, and the screen says so only when the
//! person asked; it shows no wallet buttons where there are no wallet apps.

use std::time::Duration;

use leptoskit::auth::ErrorResponse;
use leptoskit::auth::Factor;
use leptoskit::auth::ProviderInfo;
use leptoskit::auth::ProviderOutcome;
use leptoskit::auth::PublicSession;
use leptoskit::auth::StartProvider;
use leptoskit::auth::WalletSettings;
use leptoskit::auth::client::EmailCode;
use leptoskit::auth::client::PasskeyMode;
use leptoskit::auth::client::SecondFactorPrompt;
use leptoskit::auth::client::SignInStep;
use leptoskit::auth::client::WalletApp;
use leptoskit::auth::client::cancel_passkey_sign_in;
use leptoskit::auth::client::complete_sign_in;
use leptoskit::auth::client::complete_sign_in_with_passkey;
use leptoskit::auth::client::continue_provider_sign_up;
use leptoskit::auth::client::passkey_sign_in;
use leptoskit::auth::client::provider_outcome;
use leptoskit::auth::client::send_code;
use leptoskit::auth::client::sign_in_with_provider;
use leptoskit::auth::client::sign_out_mutation;
use leptoskit::auth::client::use_auth;
use leptoskit::auth::client::use_providers;
use leptoskit::auth::client::use_wallet_settings;
use leptoskit::auth::client::verify_code;
use leptoskit::auth::client::wallet_apps;
use leptoskit::auth::client::wallet_sign_in;
use leptoskit::leptos::prelude::Effect;
use leptoskit::leptos::prelude::Get;
use leptoskit::leptos::prelude::GetUntracked;
use leptoskit::leptos::prelude::RwSignal;
use leptoskit::leptos::prelude::Set;
use leptoskit::leptos::prelude::Update;
use leptoskit::prelude::*;
use leptoskit::query::ErrorArgs;
use leptoskit::query::MutateCallbacks;
use leptoskit::query::MutationHandle;
use leptoskit::query::QueryHandle;
use leptoskit::query::QueryObserverOptions;
use leptoskit::query::SuccessArgs;
use leptoskit::query::use_mutation;
use leptoskit::query::use_query_with;

/// The sign-in screen's state. It's `Copy`, so handlers capture it freely.
#[derive(Clone, Copy)]
pub struct SignIn {
	email: RwSignal<String>,
	code: RwSignal<String>,
	/// The address the last code went to. The screen asks for the code once
	/// it's set.
	sent_to: RwSignal<Option<String>>,
	/// The code from the authenticator app, or a recovery code.
	factor_code: RwSignal<String>,
	/// The second step, once the emailed code asked for one.
	second: RwSignal<Option<SecondFactorPrompt>>,
	/// What's wrong with what was typed, found before anything is sent.
	problem: RwSignal<Option<String>>,
	/// Seconds until the server takes another attempt, after `rate_limited`.
	wait: RwSignal<u64>,
	/// How a third-party sign-in that just came back ended.
	outcome: RwSignal<Option<ProviderOutcome>>,
	/// Whether the person pressed the passkey button, so its failure is worth
	/// saying. The autofill request fails quietly.
	asked_for_passkey: RwSignal<bool>,
	/// Whether the screen has mounted in the browser or a native app. Until
	/// then it says it's checking, so a page rendered on the server matches
	/// the browser's first render, however quickly the session arrives.
	mounted: RwSignal<bool>,
	session: QueryHandle<Option<PublicSession>, ErrorResponse>,
	send: MutationHandle<String, (), ErrorResponse>,
	verify: MutationHandle<EmailCode, SignInStep, ErrorResponse>,
	finish: MutationHandle<String, PublicSession, ErrorResponse>,
	passkey: MutationHandle<PasskeyMode, PublicSession, ErrorResponse>,
	passkey_second: MutationHandle<(), PublicSession, ErrorResponse>,
	providers: QueryHandle<Vec<ProviderInfo>, ErrorResponse>,
	provider: MutationHandle<StartProvider, (), ErrorResponse>,
	separately: MutationHandle<(), PublicSession, ErrorResponse>,
	sign_out: MutationHandle<(), (), ErrorResponse>,
	wallet_settings: QueryHandle<WalletSettings, ErrorResponse>,
	wallet_apps: QueryHandle<Vec<WalletApp>, ErrorResponse>,
	wallet: MutationHandle<String, SignInStep, ErrorResponse>,
}

impl SignIn {
	/// Creates the state in the current reactive owner, which needs a
	/// `QueryClient`.
	pub fn new() -> Self {
		let mounted = RwSignal::new(false);
		let outcome = RwSignal::new(None);
		let passkey = use_mutation(passkey_sign_in());
		let second = RwSignal::new(None);
		// Effects never run while the server renders. On mount, the screen
		// reads how a provider sign-in ended, and offers saved passkeys in
		// the email field's suggestions, where the browser can.
		Effect::new(move |_| {
			mounted.set(true);
			show_outcome(outcome, second);
			passkey.mutate(PasskeyMode::Autofill);
		});

		let wallet_settings = use_wallet_settings();
		// The wallet apps are asked for only when wallets may sign in.
		let wallets_sign_in = move || {
			wallet_settings
				.data()
				.is_some_and(|settings| settings.sign_in)
		};

		Self {
			email: RwSignal::new(String::new()),
			code: RwSignal::new(String::new()),
			sent_to: RwSignal::new(None),
			factor_code: RwSignal::new(String::new()),
			second,
			problem: RwSignal::new(None),
			wait: RwSignal::new(0),
			outcome,
			asked_for_passkey: RwSignal::new(false),
			mounted,
			session: use_auth(),
			send: use_mutation(send_code()),
			verify: use_mutation(verify_code()),
			finish: use_mutation(complete_sign_in()),
			passkey,
			passkey_second: use_mutation(complete_sign_in_with_passkey()),
			providers: use_providers(),
			provider: use_mutation(sign_in_with_provider()),
			separately: use_mutation(continue_provider_sign_up()),
			sign_out: use_mutation(sign_out_mutation()),
			wallet_settings,
			wallet_apps: use_query_with(
				wallet_apps(),
				|| (),
				QueryObserverOptions::default().with_enabled(wallets_sign_in),
			),
			wallet: use_mutation(wallet_sign_in()),
		}
	}

	/// The current session, or `None` while signed out or still checking.
	pub fn session(&self) -> Option<PublicSession> {
		self.mounted
			.get()
			.then(|| self.session.data().flatten())
			.flatten()
	}

	/// Whether someone is signed in.
	pub fn is_signed_in(&self) -> bool {
		self.session().is_some()
	}

	/// Whether someone is signed in, once the screen has asked the server:
	/// `None` while it's still checking.
	pub fn known(&self) -> Option<bool> {
		self.mounted
			.get()
			.then(|| self.session.data().map(|session| session.is_some()))
			.flatten()
	}

	/// Signs out of this device.
	pub fn sign_out(&self) {
		self.sign_out.mutate(());
	}

	/// Whether signing out is under way.
	pub fn is_signing_out(&self) -> bool {
		self.sign_out.is_pending()
	}

	/// The whole screen: a card with the step the person is on.
	pub fn screen(&self) -> Element {
		let session = self.mounted.get().then(|| self.session.data()).flatten();
		let step = match session {
			Some(Some(_)) => self.signed_in(),
			Some(None) => {
				match (self.second.get(), self.sent_to.get(), self.outcome.get()) {
					(Some(prompt), ..) => self.second_step(&prompt),
					(None, Some(email), _) => self.code_step(&email),
					(None, None, Some(ProviderOutcome::EmailInUse)) => self.email_in_use(),
					(None, None, _) => self.email_step(),
				}
			}
			None => {
				match self.session.error().filter(|_| self.mounted.get()) {
					Some(error) => self.unreachable(&error),
					None => Status("Checking whether you're signed in…").class("text-slate-300"),
				}
			}
		};

		View()
			.class("min-h-screen items-center justify-center p-6 bg-slate-950 text-white")
			.child(
				View()
					.class("w-full max-w-[420px] gap-4 rounded-2xl bg-slate-900 p-6")
					.child(Heading(1, "Sign in").class("text-2xl font-bold"))
					.child(step),
			)
	}

	/// Step one: the address to send a code to.
	fn email_step(&self) -> Element {
		let this = *self;
		let email = self.email;
		let sending = self.send.is_pending();
		let label = match self.wait.get() {
			0 if sending => "Sending a code…".to_owned(),
			0 => "Email me a code".to_owned(),
			seconds => format!("Try again in {seconds} s"),
		};

		View()
			.class("gap-3")
			.child(
				Text(
					"Enter your email address and we'll send you a code. If you're new, this \
					 creates your account.",
				)
				.class("text-sm text-slate-300"),
			)
			.child(Text("Email address").class("text-sm font-semibold"))
			.child(
				TextInput()
					.id("sign-in-email")
					.aria_label("Email address")
					.aria_required(true)
					// Offers saved passkeys in the field's suggestions.
					.autocomplete("username webauthn")
					.placeholder("you@example.com")
					.value(email.get_untracked())
					.on_change(move |text| email.set(text.to_owned()))
					.class("h-11 rounded-lg px-3 bg-white text-slate-900"),
			)
			.children(self.message(self.send.error()))
			.children(self.came_back())
			.child(
				Button(label)
					.enabled(!sending && self.wait.get() == 0)
					.aria_busy(sending)
					.on_press(move || this.send_code())
					.class("rounded-lg bg-indigo-500 px-4 py-3 font-semibold"),
			)
			.child(self.other_ways())
	}

	/// Signing in without email: a passkey, or a provider's account.
	fn other_ways(&self) -> Element {
		let this = *self;
		let signing_in = self.passkey.is_pending() && self.asked_for_passkey.get();
		let mut ways = View()
			.class("gap-2")
			.child(
				Button(if signing_in {
					"Waiting for your passkey…"
				} else {
					"Sign in with a passkey"
				})
				.enabled(!signing_in)
				.aria_busy(signing_in)
				.on_press(move || this.sign_in_with_passkey())
				.class("rounded-lg bg-slate-700 px-4 py-3 font-semibold"),
			)
			.children(
				self.passkey
					.error()
					.filter(|_| self.asked_for_passkey.get())
					.map(|error| Alert(passkey_problem(&error)).class("text-sm text-rose-300")),
			);

		for provider in self.providers.data().unwrap_or_default() {
			let id = provider.id.clone();
			ways = ways.child(
				Button(format!("Continue with {}", provider.name))
					.enabled(!self.provider.is_pending())
					.on_press(move || this.continue_with(&id))
					.class("rounded-lg bg-slate-700 px-4 py-3 font-semibold"),
			);
		}

		let wallets_sign_in = self
			.wallet_settings
			.data()
			.is_some_and(|settings| settings.sign_in);

		if wallets_sign_in {
			for app in self.wallet_apps.data().unwrap_or_default() {
				let name = app.name.clone();
				let signing = self.wallet.is_pending();
				ways = ways.child(
					Button(if signing {
						"Waiting for your wallet…".to_owned()
					} else {
						format!("Sign in with {}", app.name)
					})
					.enabled(!signing)
					.aria_busy(signing)
					.on_press(move || this.sign_in_with_wallet(&name))
					.class("rounded-lg bg-slate-700 px-4 py-3 font-semibold"),
				);
			}
		}

		ways.children(
			self.provider
				.error()
				.map(|error| Alert(passkey_problem(&error)).class("text-sm text-rose-300")),
		)
		.children(
			self.wallet
				.error()
				.map(|error| Alert(wallet_problem(&error)).class("text-sm text-rose-300")),
		)
	}

	fn sign_in_with_wallet(&self, wallet: &str) {
		cancel_passkey_sign_in();
		let this = *self;
		self.wallet.mutate_with(
			wallet.to_owned(),
			MutateCallbacks::new().on_success(move |args: SuccessArgs<SignInStep, String, ()>| {
				if let SignInStep::SecondFactor(prompt) = args.data {
					this.second.set(Some(prompt));
				}
			}),
		);
	}

	/// What a provider sign-in that came back with a problem says.
	fn came_back(&self) -> Option<Element> {
		match self.outcome.get()? {
			ProviderOutcome::Failed(code) => {
				Some(Alert(provider_failure(&code)).class("text-sm text-rose-300"))
			}
			_ => None,
		}
	}

	/// The provider's address is another account's: sign in to that one, or
	/// make a separate account.
	fn email_in_use(&self) -> Element {
		let this = *self;
		let creating = self.separately.is_pending();

		View()
			.class("gap-3")
			.child(
				Status(
					"An account with that email address already exists. Sign in to it with your \
					 email to connect this sign-in from its security settings, or continue with a \
					 separate account.",
				)
				.class("text-sm text-slate-300"),
			)
			.children(
				self.separately
					.error()
					.map(|error| Alert(sentence(&error.message)).class("text-sm text-rose-300")),
			)
			.child(
				Button("Sign in with my email")
					.on_press(move || this.outcome.set(None))
					.class("rounded-lg bg-indigo-500 px-4 py-3 font-semibold"),
			)
			.child(
				Button(if creating {
					"Creating…"
				} else {
					"Continue with a separate account"
				})
				.enabled(!creating)
				.aria_busy(creating)
				.on_press(move || this.separately.mutate(()))
				.class("rounded-lg bg-slate-700 px-4 py-3 font-semibold"),
			)
	}

	/// Step two: the code from the email.
	fn code_step(&self, email: &str) -> Element {
		let this = *self;
		let code = self.code;
		let verifying = self.verify.is_pending();
		let label = match self.wait.get() {
			0 if verifying => "Signing in…".to_owned(),
			0 => "Sign in".to_owned(),
			seconds => format!("Try again in {seconds} s"),
		};

		View()
			.class("gap-3")
			.child(
				Status(format!(
					"Check {email} for a code. It works for 10 minutes."
				))
				.class("text-sm text-slate-300"),
			)
			.child(Text("Code").class("text-sm font-semibold"))
			.child(
				TextInput()
					.id("sign-in-code")
					.aria_label("Code")
					.aria_required(true)
					.placeholder("ABCD-EFGH")
					.value(code.get_untracked())
					.on_change(move |text| code.set(text.to_owned()))
					.class("h-11 rounded-lg px-3 bg-white text-slate-900"),
			)
			.children(self.message(self.verify.error().or_else(|| self.send.error())))
			.child(
				Button(label)
					.enabled(!verifying && self.wait.get() == 0)
					.aria_busy(verifying)
					.on_press(move || this.verify_code())
					.class("rounded-lg bg-indigo-500 px-4 py-3 font-semibold"),
			)
			.child(
				View()
					.class("flex-row gap-3")
					.child(
						Button("Send a new code")
							.enabled(!self.send.is_pending() && self.wait.get() == 0)
							.on_press(move || this.send_code())
							.class("flex-1 rounded-lg px-4 py-2 text-sm text-indigo-300"),
					)
					.child(
						Button("Use a different address")
							.on_press(move || this.start_over())
							.class("flex-1 rounded-lg px-4 py-2 text-sm text-indigo-300"),
					),
			)
	}

	/// Step three, for an account with two-step verification: a code from
	/// the authenticator app, or a recovery code.
	fn second_step(&self, prompt: &SecondFactorPrompt) -> Element {
		let this = *self;
		let code = self.factor_code;
		let finishing = self.finish.is_pending();
		let label = match self.wait.get() {
			0 if finishing => "Checking…".to_owned(),
			0 => "Continue".to_owned(),
			seconds => format!("Try again in {seconds} s"),
		};
		let passkey = prompt.factors.contains(&Factor::Passkey);
		let using_passkey = self.passkey_second.is_pending();
		let hint = if !prompt.factors.contains(&Factor::Totp) {
			"Use your passkey, or enter one of your recovery codes."
		} else if prompt.factors.contains(&Factor::RecoveryCode) {
			"Enter the 6-digit code from your authenticator app. Lost your phone? Enter one of \
			 your recovery codes instead."
		} else {
			"Enter the 6-digit code from your authenticator app."
		};

		View()
			.class("gap-3")
			.child(Status(hint).class("text-sm text-slate-300"))
			.child(Text("Code").class("text-sm font-semibold"))
			.child(
				TextInput()
					.id("sign-in-second-factor")
					.aria_label("Authenticator or recovery code")
					.aria_required(true)
					.placeholder("123456")
					.value(code.get_untracked())
					.on_change(move |text| code.set(text.to_owned()))
					.class("h-11 rounded-lg px-3 bg-white text-slate-900"),
			)
			.children(self.message(self.finish.error()))
			.child(
				Button(label)
					.enabled(!finishing && self.wait.get() == 0)
					.aria_busy(finishing)
					.on_press(move || this.complete())
					.class("rounded-lg bg-indigo-500 px-4 py-3 font-semibold"),
			)
			.children(passkey.then(|| {
				Button(if using_passkey {
					"Waiting for your passkey…"
				} else {
					"Use a passkey"
				})
				.enabled(!using_passkey)
				.aria_busy(using_passkey)
				.on_press(move || this.complete_with_passkey())
				.class("rounded-lg bg-slate-700 px-4 py-3 font-semibold")
			}))
			.children(
				self.passkey_second
					.error()
					.map(|error| Alert(passkey_problem(&error)).class("text-sm text-rose-300")),
			)
			.child(
				Button("Start again")
					.on_press(move || this.start_over())
					.class("rounded-lg px-4 py-2 text-sm text-indigo-300"),
			)
	}

	/// What a signed-in person sees here.
	fn signed_in(&self) -> Element {
		let this = *self;
		let signing_out = self.is_signing_out();

		View()
			.class("gap-3")
			.child(Status("You're signed in.").class("text-slate-300"))
			.children(self.message(self.sign_out.error()))
			.child(
				Button(if signing_out {
					"Signing out…"
				} else {
					"Sign out"
				})
				.enabled(!signing_out)
				.aria_busy(signing_out)
				.on_press(move || this.sign_out())
				.class("rounded-lg bg-slate-700 px-4 py-3 font-semibold"),
			)
	}

	/// The session couldn't be read, such as when the server is down.
	fn unreachable(&self, error: &ErrorResponse) -> Element {
		let session = self.session;

		View()
			.class("gap-3")
			.child(Alert(sentence(&error.message)).class("text-sm text-rose-300"))
			.child(
				Button("Try again")
					.on_press(move || drop(session.refetch()))
					.class("rounded-lg bg-slate-700 px-4 py-3 font-semibold"),
			)
	}

	/// What went wrong, if anything: a problem with what was typed first,
	/// then the server's answer.
	fn message(&self, error: Option<ErrorResponse>) -> Option<Element> {
		let text = match (self.problem.get(), error) {
			(Some(problem), _) => problem,
			// The button counts down; once it reaches zero, the refusal no
			// longer applies.
			(None, Some(error)) if retry_after(&error).is_some() && self.wait.get() == 0 => {
				return None;
			}
			(None, Some(error)) => sentence(&error.message),
			(None, None) => return None,
		};

		Some(Alert(text).class("text-sm text-rose-300"))
	}

	fn sign_in_with_passkey(&self) {
		self.asked_for_passkey.set(true);
		self.passkey.mutate(PasskeyMode::Prompt);
	}

	/// Signs in with a provider. A browser leaves for its page and reads
	/// the outcome when it comes back; a native app gets it once the system
	/// browser closes.
	fn continue_with(&self, provider: &str) {
		cancel_passkey_sign_in();
		let this = *self;
		self.provider.mutate_with(
			StartProvider {
				provider: provider.to_owned(),
				redirect_to: None,
			},
			MutateCallbacks::new().on_success(move |_| show_outcome(this.outcome, this.second)),
		);
	}

	fn complete_with_passkey(&self) {
		let this = *self;
		self.passkey_second.mutate_with(
			(),
			MutateCallbacks::new()
				.on_success(move |_| this.second.set(None))
				.on_error(move |failed: ErrorArgs<ErrorResponse, (), ()>| {
					if failed.error.error == "sign_in_expired" {
						this.start_over();
					}
				}),
		);
	}

	fn send_code(&self) {
		cancel_passkey_sign_in();
		let email = self.email.get_untracked().trim().to_owned();

		if !looks_like_email(&email) {
			self.problem.set(Some(
				"Enter an email address, such as ada@example.com.".into(),
			));
			return;
		}

		self.problem.set(None);
		self.verify.reset();
		let sent_to = self.sent_to;
		let wait = self.wait;
		self.send.mutate_with(
			email.clone(),
			MutateCallbacks::new()
				.on_success(move |_| sent_to.set(Some(email)))
				.on_error(move |failed| hold_off(wait, &failed.error)),
		);
	}

	fn verify_code(&self) {
		let Some(email) = self.sent_to.get_untracked() else {
			return;
		};
		let code = self.code.get_untracked().trim().to_owned();

		if code.is_empty() {
			self.problem
				.set(Some("Enter the code from the email.".into()));
			return;
		}

		self.problem.set(None);
		let this = *self;
		self.verify.mutate_with(
			EmailCode { email, code },
			MutateCallbacks::new()
				// Signed in, the session query holds the new session, so the
				// screen moves on by itself; otherwise it asks for the second
				// factor.
				.on_success(move |success| {
					this.code.set(String::new());
					this.sent_to.set(None);

					if let SignInStep::SecondFactor(prompt) = &success.data {
						this.second.set(Some(prompt.clone()));
					}
				})
				.on_error(move |failed| hold_off(this.wait, &failed.error)),
		);
	}

	fn complete(&self) {
		let code = self.factor_code.get_untracked().trim().to_owned();

		if code.is_empty() {
			self.problem
				.set(Some("Enter the code from your authenticator app.".into()));
			return;
		}

		self.problem.set(None);
		let this = *self;
		self.finish.mutate_with(
			code,
			MutateCallbacks::new()
				.on_success(move |_| {
					this.factor_code.set(String::new());
					this.second.set(None);
				})
				.on_error(move |failed: ErrorArgs<ErrorResponse, String, ()>| {
					if failed.error.error == "sign_in_expired" {
						this.start_over();
						this.problem.set(Some(
							"That took too long. Enter your email address to start again.".into(),
						));
					} else {
						hold_off(this.wait, &failed.error);
					}
				}),
		);
	}

	fn start_over(&self) {
		self.code.set(String::new());
		self.factor_code.set(String::new());
		self.sent_to.set(None);
		self.second.set(None);
		self.problem.set(None);
		self.outcome.set(None);
		self.send.reset();
		self.verify.reset();
		self.finish.reset();
		self.passkey_second.reset();
	}
}

impl Default for SignIn {
	fn default() -> Self {
		Self::new()
	}
}

/// After a `rate_limited` answer, counts `wait` down a second at a time from
/// the server's `retry_after`. The buttons stay disabled until it reaches
/// zero.
fn hold_off(wait: RwSignal<u64>, error: &ErrorResponse) {
	let Some(seconds) = retry_after(error) else {
		return;
	};
	let counting = wait.get_untracked() > 0;
	wait.set(seconds);

	if counting {
		return;
	}

	any_spawner::Executor::spawn_local(async move {
		while wait.try_get_untracked().is_some_and(|left| left > 0) {
			leptoskit::query::sleep(Duration::from_secs(1)).await;
			wait.try_update(|left| *left = left.saturating_sub(1));
		}
	});
}

/// How many seconds to wait before trying again, when the server refused
/// because of too many attempts.
pub fn retry_after(error: &ErrorResponse) -> Option<u64> {
	if error.error == "rate_limited" {
		Some(error.retry_after.unwrap_or(1).max(1))
	} else {
		None
	}
}

/// What a passkey or provider failure says to the person.
pub fn passkey_problem(error: &ErrorResponse) -> String {
	match error.error.as_str() {
		"cancelled" => "No passkey was used.".to_owned(),
		"unsupported" => "This device can't use that yet. Use your email instead.".to_owned(),
		_ => sentence(&error.message),
	}
}

/// What a wallet sign-in that failed says to the person.
pub fn wallet_problem(error: &ErrorResponse) -> String {
	match error.error.as_str() {
		"cancelled" => "Nothing was signed.".to_owned(),
		"unknown_wallet" => {
			"That wallet can't sign in here. Sign in another way, then let it sign in from \
			 Security."
				.to_owned()
		}
		"wallet_failed" => "That wallet's signature didn't work. Try again.".to_owned(),
		"no_wallet" => "That wallet isn't available here anymore.".to_owned(),
		_ => passkey_problem(error),
	}
}

/// What a provider sign-in that came back failed says to the person.
pub fn provider_failure(code: &str) -> String {
	match code {
		"cancelled" => "Signing in was cancelled.",
		"sign_in_expired" => "That took too long. Try again.",
		"disabled" => "This account is disabled.",
		"locked" => "This account is locked for a while after too many attempts.",
		"unknown_provider" => "That sign-in isn't set up.",
		_ => "Signing in with that account didn't work. Try again.",
	}
	.to_owned()
}

/// Whether `email` is worth sending to the server, which checks it properly.
pub fn looks_like_email(email: &str) -> bool {
	email
		.split_once('@')
		.is_some_and(|(name, domain)| !name.is_empty() && domain.contains('.'))
		&& !email.contains(char::is_whitespace)
}

/// Shows how the last provider sign-in ended, with its second step when it
/// asked for one.
fn show_outcome(
	outcome: RwSignal<Option<ProviderOutcome>>,
	second: RwSignal<Option<SecondFactorPrompt>>,
) {
	let ended = provider_outcome();

	if let Some(ProviderOutcome::SecondFactor(factors)) = &ended {
		second.set(Some(SecondFactorPrompt {
			factors: factors.clone(),
			expires_in: 300,
		}));
	}

	outcome.set(ended);
}

/// A message as a sentence: a capital letter first and a full stop last.
pub fn sentence(message: &str) -> String {
	let message = message.trim();
	let mut characters = message.chars();
	let Some(first) = characters.next() else {
		return String::new();
	};
	let mut sentence = first.to_uppercase().collect::<String>();
	sentence.push_str(characters.as_str());

	if !sentence.ends_with(['.', '!', '?']) {
		sentence.push('.');
	}

	sentence
}

#[cfg(test)]
mod tests {
	use super::*;

	fn refusal(error: &str, retry_after: Option<u64>) -> ErrorResponse {
		ErrorResponse {
			error: error.into(),
			message: "too many attempts; try again in 30 seconds".into(),
			retry_after,
		}
	}

	#[test]
	fn only_a_rate_limit_asks_to_wait() {
		assert_eq!(retry_after(&refusal("rate_limited", Some(30))), Some(30));
		assert_eq!(retry_after(&refusal("rate_limited", None)), Some(1));
		assert_eq!(retry_after(&refusal("invalid_code", Some(30))), None);
	}

	#[test]
	fn addresses_are_checked_loosely() {
		assert!(looks_like_email("ada@example.com"));
		assert!(!looks_like_email("ada"));
		assert!(!looks_like_email("@example.com"));
		assert!(!looks_like_email("ada@localhost"));
		assert!(!looks_like_email("ada @example.com"));
	}

	#[test]
	fn failures_are_explained_in_words() {
		assert_eq!(
			passkey_problem(&refusal("cancelled", None)),
			"No passkey was used."
		);
		assert!(passkey_problem(&refusal("unsupported", None)).contains("Use your email"));
		assert_eq!(
			passkey_problem(&refusal("passkey_failed", None)),
			"Too many attempts; try again in 30 seconds."
		);
		assert_eq!(provider_failure("cancelled"), "Signing in was cancelled.");
		assert!(provider_failure("provider").contains("didn't work"));
	}

	#[test]
	fn wallet_failures_are_explained_in_words() {
		for (code, words) in [
			("cancelled", "Nothing was signed."),
			("unknown_wallet", "That wallet can't sign in here."),
			("wallet_failed", "That wallet's signature didn't work."),
			("no_wallet", "That wallet isn't available here anymore."),
			("unsupported", "This device can't use that yet."),
		] {
			assert!(
				wallet_problem(&refusal(code, None)).starts_with(words),
				"{code}"
			);
		}
	}

	#[test]
	fn messages_read_as_sentences() {
		assert_eq!(
			sentence("that code didn't work; ask for a new one"),
			"That code didn't work; ask for a new one."
		);
		assert_eq!(sentence("Done!"), "Done!");
		assert_eq!(sentence("  "), "");
	}

	// The states only the screen can reach: the steps after the email, the
	// two-step prompts, and what each refusal says. Their inputs live in
	// private signals, so these tests drive the same screen the app mounts
	// from this module. They share the binary with the room's tokio test, so
	// they run on the same executor instead of installing their own.

	use leptoskit::leptos::prelude::Owner;
	use leptoskit::query::QueryClient;
	use leptoskit::query::QueryDefaults;
	use leptoskit::query::RetryPolicy;
	use leptoskit_testing::Finder;
	use leptoskit_testing::Queries;
	use leptoskit_testing::WidgetTester;

	fn fixture() -> (Owner, SignIn, QueryClient) {
		let owner = Owner::new();
		let (sign_in, client) = owner.with(|| {
			let client = QueryClient::new()
				.with_defaults(QueryDefaults::new().with_retry(RetryPolicy::Never));
			client.provide();
			(SignIn::default(), client)
		});
		(owner, sign_in, client)
	}

	fn mount(owner: &Owner, sign_in: &SignIn) -> WidgetTester {
		let owner = owner.clone();
		let sign_in = *sign_in;
		WidgetTester::new(move || owner.with(|| sign_in.screen()), 800.0, 2400.0)
			.expect("the sign-in screen mounts")
	}

	fn prompt(factors: Vec<Factor>) -> SecondFactorPrompt {
		SecondFactorPrompt {
			factors,
			expires_in: 300,
		}
	}

	/// The refusals a call can leave on screen; which one depends on the
	/// features this test binary was built with alongside other packages.
	fn refused(screen: &WidgetTester) -> bool {
		[
			"This build can't reach the server: turn on leptoskit_auth's `web` feature for the \
			 browser, or `guest` for a native app.",
			"Signing in and out runs in the browser or the app, not while the server renders.",
			"Something went wrong; try again.",
		]
		.iter()
		.any(|text| !screen.find_all(&Finder::text(*text)).is_empty())
	}

	/// Runs every queued effect and call to rest.
	async fn settle() {
		for _ in 0..8 {
			any_spawner::Executor::tick().await;
			tokio::task::yield_now().await;
		}
	}

	#[tokio::test]
	async fn the_code_step_checks_and_sends_again() {
		let _ = any_spawner::Executor::init_tokio();
		tokio::task::LocalSet::new()
			.run_until(async {
				let (owner, sign_in, client) = fixture();
				client.set_query_data(&leptoskit::auth::client::session(), (), None);
				sign_in.mounted.set(true);
				sign_in.email.set("ada@example.com".into());
				sign_in.sent_to.set(Some("ada@example.com".into()));
				settle().await;
				let sign_in_button = Finder::role(NativeKind::Button, "Sign in");
				let mut screen = mount(&owner, &sign_in);

				screen
					.get(&Finder::text(
						"Check ada@example.com for a code. It works for 10 minutes.",
					))
					.unwrap();
				screen.get(&Finder::label("Code")).unwrap();
				screen.get(&Finder::text("Send a new code")).unwrap();
				screen
					.get(&Finder::text("Use a different address"))
					.unwrap();

				// An empty code is refused here.
				screen.tap(&sign_in_button).unwrap();
				settle().await;
				screen.pump();
				screen
					.get(&Finder::text("Enter the code from the email."))
					.unwrap();

				// A typed one reaches the client, which has no server here.
				screen
					.enter_text(&Finder::label("Code"), "ABCD-EFGH")
					.unwrap();
				screen.tap(&sign_in_button).unwrap();
				settle().await;
				screen.pump();
				assert!(refused(&screen), "the code's refusal is on screen");

				// Sending another code without an address asks for one first.
				sign_in.email.set(String::new());
				screen.pump();
				screen.tap(&Finder::text("Send a new code")).unwrap();
				settle().await;
				screen.pump();
				screen
					.get(&Finder::text(
						"Enter an email address, such as ada@example.com.",
					))
					.unwrap();

				// Starting over returns to the address step.
				screen
					.tap(&Finder::text("Use a different address"))
					.unwrap();
				settle().await;
				screen.pump();
				screen.get(&Finder::label("Email address")).unwrap();

				drop(screen);
				owner.cleanup();
			})
			.await;
	}

	#[tokio::test]
	async fn the_second_step_names_every_factor_it_offers() {
		let _ = any_spawner::Executor::init_tokio();
		tokio::task::LocalSet::new()
			.run_until(async {
				let (owner, sign_in, client) = fixture();
				client.set_query_data(&leptoskit::auth::client::session(), (), None);
				sign_in.mounted.set(true);
				sign_in
					.second
					.set(Some(prompt(vec![Factor::Totp, Factor::RecoveryCode])));
				settle().await;
				let mut screen = mount(&owner, &sign_in);

				screen
					.get(&Finder::text(
						"Enter the 6-digit code from your authenticator app. Lost your phone? \
						 Enter one of your recovery codes instead.",
					))
					.unwrap();
				assert!(
					screen.query_by_text("Use a passkey").unwrap().is_none(),
					"no passkey was offered"
				);

				// The code is checked before it is sent, then the call fails.
				screen.tap(&Finder::text("Continue")).unwrap();
				settle().await;
				screen.pump();
				screen
					.get(&Finder::text("Enter the code from your authenticator app."))
					.unwrap();
				screen
					.enter_text(&Finder::label("Authenticator or recovery code"), "123456")
					.unwrap();
				screen.tap(&Finder::text("Continue")).unwrap();
				settle().await;
				screen.pump();
				assert!(refused(&screen), "the code's refusal is on screen");

				// Without a recovery code the hint only names the app.
				sign_in.second.set(Some(prompt(vec![Factor::Totp])));
				settle().await;
				screen.pump();
				screen
					.get(&Finder::text(
						"Enter the 6-digit code from your authenticator app.",
					))
					.unwrap();

				// A passkey alone offers the passkey button, and its refusal
				// shows.
				sign_in.second.set(Some(prompt(vec![Factor::Passkey])));
				settle().await;
				screen.pump();
				screen
					.get(&Finder::text(
						"Use your passkey, or enter one of your recovery codes.",
					))
					.unwrap();
				screen.tap(&Finder::text("Use a passkey")).unwrap();
				settle().await;
				screen.pump();
				assert!(refused(&screen), "the passkey's refusal is on screen");

				// Starting again clears every step.
				screen.tap(&Finder::text("Start again")).unwrap();
				settle().await;
				screen.pump();
				screen.get(&Finder::label("Email address")).unwrap();
				assert!(sign_in.second.get_untracked().is_none());

				drop(screen);
				owner.cleanup();
			})
			.await;
	}

	#[tokio::test]
	async fn an_email_already_in_use_offers_both_ways_forward() {
		let _ = any_spawner::Executor::init_tokio();
		tokio::task::LocalSet::new()
			.run_until(async {
				let (owner, sign_in, client) = fixture();
				client.set_query_data(&leptoskit::auth::client::session(), (), None);
				sign_in.mounted.set(true);
				// The mount effect reads how a provider sign-in ended and
				// would clear a value set before it ran.
				settle().await;
				sign_in.outcome.set(Some(ProviderOutcome::EmailInUse));
				let mut screen = mount(&owner, &sign_in);

				screen
					.get(&Finder::text(
						"An account with that email address already exists. Sign in to it with \
						 your email to connect this sign-in from its security settings, or \
						 continue with a separate account.",
					))
					.unwrap();

				// Signing in with the address goes back to step one.
				screen.tap(&Finder::text("Sign in with my email")).unwrap();
				settle().await;
				screen.pump();
				screen.get(&Finder::label("Email address")).unwrap();

				// A separate account runs the mutation, which fails here.
				sign_in.outcome.set(Some(ProviderOutcome::EmailInUse));
				settle().await;
				screen.pump();
				screen
					.tap(&Finder::text("Continue with a separate account"))
					.unwrap();
				settle().await;
				screen.pump();
				assert!(refused(&screen), "the refusal is on screen");

				drop(screen);
				owner.cleanup();
			})
			.await;
	}

	#[tokio::test]
	async fn a_returning_provider_says_how_it_ended() {
		let _ = any_spawner::Executor::init_tokio();
		tokio::task::LocalSet::new()
			.run_until(async {
				let (owner, sign_in, client) = fixture();
				client.set_query_data(&leptoskit::auth::client::session(), (), None);
				sign_in.mounted.set(true);
				settle().await;
				sign_in
					.outcome
					.set(Some(ProviderOutcome::Failed("locked".into())));
				let mut screen = mount(&owner, &sign_in);

				screen
					.get(&Finder::text(
						"This account is locked for a while after too many attempts.",
					))
					.unwrap();

				// A signed-in return says nothing here: the session query
				// moves the screen on by itself.
				sign_in.outcome.set(Some(ProviderOutcome::SignedIn));
				settle().await;
				screen.pump();
				assert!(
					screen
						.query_by_text(
							"This account is locked for a while after too many attempts."
						)
						.unwrap()
						.is_none()
				);

				drop(screen);
				owner.cleanup();
			})
			.await;
	}

	#[tokio::test]
	async fn a_waiting_screen_counts_down_and_says_so() {
		let _ = any_spawner::Executor::init_tokio();
		tokio::task::LocalSet::new()
			.run_until(async {
				let (owner, sign_in, client) = fixture();
				client.set_query_data(&leptoskit::auth::client::session(), (), None);
				sign_in.mounted.set(true);
				sign_in.wait.set(30);
				settle().await;
				let mut screen = mount(&owner, &sign_in);
				screen.get(&Finder::text("Try again in 30 s")).unwrap();

				sign_in.sent_to.set(Some("ada@example.com".into()));
				settle().await;
				screen.pump();
				screen.get(&Finder::text("Try again in 30 s")).unwrap();

				sign_in.second.set(Some(prompt(vec![Factor::Totp])));
				settle().await;
				screen.pump();
				screen.get(&Finder::text("Try again in 30 s")).unwrap();

				// A rate limit starts the countdown at its own second, and
				// asking again while it runs doesn't start a second one.
				let error = refusal("rate_limited", Some(3));
				sign_in.wait.set(0);
				hold_off(sign_in.wait, &error);
				assert_eq!(sign_in.wait.get_untracked(), 3);
				hold_off(sign_in.wait, &error);
				assert_eq!(sign_in.wait.get_untracked(), 3);

				drop(screen);
				owner.cleanup();
			})
			.await;
	}

	#[tokio::test]
	async fn an_unmounted_screen_does_not_claim_to_know() {
		let _ = any_spawner::Executor::init_tokio();
		tokio::task::LocalSet::new()
			.run_until(async {
				let (owner, sign_in, client) = fixture();
				let session: PublicSession = serde_json::from_value(serde_json::json!({
					"id": "0192f3c4-0000-7000-8000-000000000090",
					"account_id": "0192f3c4-0000-7000-8000-000000000091",
					"aal": "aal1",
					"authenticated_at": "2026-11-01T10:00:00Z",
					"mfa_at": null,
					"last_seen_at": "2026-11-01T10:00:00Z",
					"idle_expires_at": "2026-11-02T10:00:00Z",
					"expires_at": "2026-12-01T10:00:00Z",
					"device": "the test",
					"revoked_at": null,
					"created_at": "2026-11-01T10:00:00Z"
				}))
				.expect("a session");
				client.set_query_data(&leptoskit::auth::client::session(), (), Some(session));

				// Until the mount effect runs, the answer is unknown on
				// purpose, so a server render and the browser's first render
				// agree.
				assert!(sign_in.known().is_none());
				assert!(!sign_in.is_signed_in());
				let checking = mount(&owner, &sign_in);
				checking
					.get(&Finder::text("Checking whether you're signed in…"))
					.unwrap();
				drop(checking);

				sign_in.mounted.set(true);
				settle().await;
				assert_eq!(sign_in.known(), Some(true));
				assert!(sign_in.session().is_some());
				assert!(sign_in.is_signed_in());
				assert!(!sign_in.is_signing_out());
				sign_in.sign_out();
				settle().await;
				assert!(!sign_in.is_signing_out());

				let screen = mount(&owner, &sign_in);
				screen.get(&Finder::text("You're signed in.")).unwrap();
				drop(screen);

				client.set_query_data(&leptoskit::auth::client::session(), (), None);
				settle().await;
				assert_eq!(sign_in.known(), Some(false));

				owner.cleanup();
			})
			.await;
	}
}
