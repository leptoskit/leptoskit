//! Sign-in: this browser's own person, the real emailed code, and the demo
//! inbox drawn as the page's underside.
//!
//! The server decides what it can: which person is chosen, what the sign-in
//! is for, and whether it still holds mail. The page does the rest against
//! Auth's real routes, and reads the inbox only after a send or when asked.

use std::sync::Arc;
use std::time::Duration;

use leptos::prelude::Effect;
use leptos::prelude::Memo;
use leptoskit::aria::RadioGroup;
use leptoskit::aria::Tabs;
use leptoskit::auth::ErrorResponse;
use leptoskit::auth::client::EmailCode;
use leptoskit::auth::client::SignInStep;
use leptoskit::auth::client::send_code;
use leptoskit::auth::client::verify_code;
use leptoskit::native::Capitalize;
use leptoskit::native::KeyPress;
use leptoskit::native::Keyboard;
use leptoskit::native::Live;
use leptoskit::native::Role;
use leptoskit::native::SubmitLabel;
use leptoskit::prelude::*;
use leptoskit::query::MutationHandle;
use leptoskit::query::use_mutation;
use leptoskit::rpc::FnError;

use crate::Shell;
use crate::chrome;
use crate::chrome::Tone;
use crate::front;
use crate::model::Flow;
use crate::model::Person;
use crate::model::Persona;
use crate::model::SignIn;
use crate::model::code_in;
use crate::model::query_value;
use crate::model::utc_clock;
use crate::model::utc_time;
use crate::model::wait_clock;
use crate::ops;
use crate::ops::Letter;

/// How long a code works for, which is also how long the inbox keeps it.
const CODE_LIFETIME_MS: i64 = 600_000;

/// How long Auth makes an address wait between codes.
const RESEND_WAIT: Duration = Duration::from_secs(30);

/// The stations, in order: name, the real call each stands for, and what
/// that call does.
const STATIONS: [(&str, &str, &str); 5] = [
	(
		"send",
		"POST /auth/email/send",
		"Notes makes a one-time code and keeps only its hash.",
	),
	(
		"mail",
		"Mailer::send(Mail { to, subject, text })",
		"Notes hands the email to its Mailer. A live site's Mailer calls an email provider; this \
		 demo's keeps a small private inbox.",
	),
	(
		"read",
		"rpc: inbox (pass cookie)",
		"You read the code here, as you would in your email app. The inbox answers only for this \
		 browser's pass, never for an address in a request.",
	),
	(
		"verify",
		"POST /auth/email/verify",
		"Notes checks the code against the hash. It works once, for 10 minutes.",
	),
	(
		"session",
		"Set-Cookie: a session cookie",
		"Notes sets a session cookie that only this site can read. The admin gets its own, on its \
		 own host.",
	),
];

const SEND: usize = 0;
const MAIL: usize = 1;
const READ: usize = 2;
const VERIFY: usize = 3;
const SESSION: usize = 4;

/// Where the sign-in has got to, which decides the panes that show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
	/// Nobody is chosen yet.
	Choose,
	/// A person is chosen and no code is out.
	Ready,
	/// A code is in the inbox.
	Sent,
	/// A code was sent that this server no longer holds.
	Lost,
}

/// What the page says above the form about the last thing it tried.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Notice {
	/// An alert interrupts; anything else is said politely.
	alert: bool,
	title: String,
	body: String,
	/// Whether to offer the way back to choosing a person.
	start_again: bool,
}

/// How far the stations are lit: every station up to `done` is behind the
/// visitor, and `now` is where the sign-in is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Lit {
	done: Option<usize>,
	now: Option<usize>,
}

/// What sign-in keeps while it is open.
#[derive(Clone, Copy)]
pub(crate) struct State {
	stage: RwSignal<Stage>,
	/// This person's mail, oldest first.
	letters: RwSignal<Vec<Letter>>,
	lost_at_ms: RwSignal<Option<i64>>,
	/// What is typed in the code field.
	code: RwSignal<String>,
	/// Whether the code field is marked wrong.
	invalid: RwSignal<bool>,
	/// Whether a request is on its way.
	busy: RwSignal<bool>,
	/// Whether the pass is gone, so nothing here can work.
	blocked: RwSignal<bool>,
	/// Whether the day is full, so choosing cannot work.
	full: RwSignal<bool>,
	/// When another code may be sent, on the page's own clock.
	cooldown_until: RwSignal<Duration>,
	/// Whether that time is still ahead.
	waiting: Memo<bool>,
	notice: RwSignal<Option<Notice>>,
	/// Who the choice is on.
	chosen: RwSignal<Option<Persona>>,
	/// The station whose explanation shows.
	station: RwSignal<usize>,
	lit: RwSignal<Lit>,
	send: MutationHandle<String, (), ErrorResponse>,
	verify: MutationHandle<EmailCode, SignInStep, ErrorResponse>,
	to_code: FocusRequest,
	to_verify: FocusRequest,
	to_mail_action: FocusRequest,
	to_refresh: FocusRequest,
	show_verify: Reveal,
	show_inbox: Reveal,
}

impl State {
	/// Sign-in as the server drew it.
	pub(crate) fn new(shell: &Shell, view: &SignIn) -> Self {
		let first = if view.identity.is_none() {
			Stage::Choose
		} else if view.mail.lost_at_ms.is_some() {
			Stage::Lost
		} else if view.mail.held > 0 {
			Stage::Sent
		} else {
			Stage::Ready
		};
		let cooldown_until = RwSignal::new(Duration::ZERO);
		let clock = shell.clock;
		let state = Self {
			stage: RwSignal::new(first),
			letters: RwSignal::new(Vec::new()),
			lost_at_ms: RwSignal::new(view.mail.lost_at_ms),
			code: RwSignal::new(String::new()),
			invalid: RwSignal::new(false),
			busy: RwSignal::new(false),
			blocked: RwSignal::new(false),
			full: RwSignal::new(false),
			cooldown_until,
			waiting: Memo::new(move |_| clock.get() < cooldown_until.get()),
			notice: RwSignal::new(None),
			chosen: RwSignal::new(view.preselect),
			station: RwSignal::new(SEND),
			lit: RwSignal::new(Lit {
				done: None,
				now: Some(SEND),
			}),
			send: use_mutation(send_code()),
			verify: use_mutation(verify_code()),
			to_code: FocusRequest::new(),
			to_verify: FocusRequest::new(),
			to_mail_action: FocusRequest::new(),
			to_refresh: FocusRequest::new(),
			show_verify: Reveal::new(),
			show_inbox: Reveal::new(),
		};
		state.light_for(first);

		state
	}

	/// Lights the stations, and shows the explanation of the one the
	/// sign-in is at.
	fn light(self, done: Option<usize>, now: Option<usize>) {
		self.lit.set(Lit { done, now });

		if let Some(now) = now {
			self.station.set(now);
		}
	}

	/// Lights the stations the way a stage leaves them.
	fn light_for(self, stage: Stage) {
		match stage {
			Stage::Choose | Stage::Ready => self.light(None, Some(SEND)),
			Stage::Lost => self.light(Some(SEND), Some(MAIL)),
			Stage::Sent if self.code.get_untracked().is_empty() => {
				self.light(Some(MAIL), Some(READ));
			}
			Stage::Sent => self.light(Some(READ), Some(VERIFY)),
		}
	}

	fn set_stage(self, stage: Stage) {
		self.stage.set(stage);
		self.light_for(stage);
	}

	fn alert(self, title: &str, body: &str) {
		self.notice.set(Some(Notice {
			alert: true,
			title: title.to_owned(),
			body: body.to_owned(),
			start_again: false,
		}));
	}

	fn status(self, title: &str, body: &str) {
		self.notice.set(Some(Notice {
			alert: false,
			title: title.to_owned(),
			body: body.to_owned(),
			start_again: false,
		}));
	}

	fn clear_notices(self) {
		self.notice.set(None);
		self.invalid.set(false);
	}
}

/// Everything an action on this page needs.
#[derive(Clone)]
struct Page {
	shell: Shell,
	view: Arc<SignIn>,
	state: State,
}

impl Page {
	/// The server's clock now, in Unix milliseconds.
	fn now_ms(&self) -> i64 {
		let elapsed = self.shell.clock.get_untracked();

		self.shell
			.context
			.now_ms
			.saturating_add(i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
	}

	/// The pass under this page is gone: nothing here can work until the
	/// visitor chooses a person again.
	fn pass_gone(&self, new_day: bool) {
		let state = self.state;
		state.set_stage(if self.view.identity.is_some() {
			Stage::Ready
		} else {
			Stage::Choose
		});
		state.blocked.set(true);
		state.invalid.set(false);
		state.notice.set(Some(Notice {
			alert: true,
			title: if new_day {
				"A new demo day started.".to_owned()
			} else {
				"This browser's pass is gone.".to_owned()
			},
			body: "Choose who to be again to get a new person. Cleared cookies and the daily \
			       reset both do this."
				.to_owned(),
			start_again: true,
		}));
	}

	/// Reads this pass's inbox. `true` when it answered, `false` when it
	/// could not, with the problem already on the page.
	async fn load_inbox(&self) -> bool {
		let state = self.state;
		let Some(me) = &self.view.identity else {
			return false;
		};

		let mailbox = match leptoskit::rpc::client::call(ops::Inbox {
			persona: me.persona,
		})
		.await
		{
			Ok(mailbox) => mailbox,
			Err(FnError::Network(_)) => {
				state.alert(
					"Couldn't reach Notes.",
					"Check your connection, then check for mail again.",
				);
				return false;
			}
			Err(FnError::Refused { code, .. }) if code == ops::NO_PASS || code == ops::NEW_DAY => {
				self.pass_gone(code == ops::NEW_DAY);
				return false;
			}
			Err(_) => {
				state.alert(
					"The inbox didn't answer.",
					"Try Check for mail again in a few seconds.",
				);
				return false;
			}
		};

		let any = !mailbox.letters.is_empty();
		state.letters.set(mailbox.letters);
		state.lost_at_ms.set(mailbox.lost_at_ms);

		// A code another process sent that this one no longer holds is the
		// current truth; older mail it still shows is replaced.
		if let Some(lost_at_ms) = mailbox.lost_at_ms {
			state.set_stage(Stage::Lost);
			state.status(
				"Your last code isn't here any more.",
				&format!(
					"This server no longer has the code it sent at {} UTC. Send a new code; your \
					 account and any session are untouched.",
					utc_time(lost_at_ms)
				),
			);
		} else if any {
			state.set_stage(Stage::Sent);
		} else {
			state.set_stage(Stage::Ready);
		}

		true
	}

	/// Says why Auth turned a request down.
	fn refusal(&self, error: &ErrorResponse, doing: &str) {
		let state = self.state;

		match error.error.as_str() {
			"network" => {
				state.alert(
					"Couldn't reach Notes.",
					"Check your connection and try again.",
				);
			}
			"not_your_identity" => self.pass_gone(false),
			"rate_limited" => {
				let newest = state
					.letters
					.with_untracked(|letters| letters.last().map(|letter| letter.sent_at_ms));
				let still = newest
					.map(|sent| sent.saturating_add(CODE_LIFETIME_MS))
					.filter(|until| *until > self.now_ms())
					.map(|until| {
						format!(
							" Your newest code still works until {} UTC, if it's in your inbox.",
							utc_time(until)
						)
					})
					.unwrap_or_default();
				let wait = error.retry_after.map(Duration::from_secs);
				state.status(
					"That's enough codes for now.",
					&format!("Try again {}.{still}", front::wait_words(wait)),
				);

				if let Some(wait) = wait {
					self.start_cooldown(wait);
				}
			}
			"invalid_code" => {
				state.alert(
					"That code didn't work.",
					"It may be mistyped, already used, or expired. Check your demo inbox, or send \
					 a new code.",
				);
				state.invalid.set(true);
				state.to_code.request();
			}
			"server" => {
				state.alert(
					"Notes is busy or restarting.",
					&format!("Try {doing} again in a few seconds."),
				);
			}
			_ => {
				state.alert(
					"That didn't go through.",
					&format!("Try {doing} again. If it keeps happening, reload the page."),
				);
			}
		}
	}

	fn start_cooldown(&self, wait: Duration) {
		self.state
			.cooldown_until
			.set(self.shell.clock.get_untracked().saturating_add(wait));
	}

	/// Asks Auth to email a code, then reads the inbox it lands in.
	fn send(&self) {
		let state = self.state;
		let Some(me) = &self.view.identity else {
			return;
		};

		if state.busy.get_untracked()
			|| state.waiting.get_untracked()
			|| state.blocked.get_untracked()
		{
			return;
		}

		state.busy.set(true);
		state.clear_notices();
		state.light(None, Some(SEND));
		let page = self.clone();
		let email = me.email.clone();
		crate::spawn(async move {
			let sent = state.send.mutate_async(email).await;
			state.busy.set(false);

			if let Err(error) = sent {
				page.refusal(&error, "sending");

				if error.error == "rate_limited" {
					page.load_inbox().await;
				}

				return;
			}

			page.start_cooldown(RESEND_WAIT);
			let before = state.letters.with_untracked(|letters| {
				letters
					.iter()
					.map(|letter| letter.receipt.clone())
					.collect::<Vec<_>>()
			});

			if !page.load_inbox().await || state.stage.get_untracked() == Stage::Lost {
				return;
			}

			let fresh = state.letters.with_untracked(|letters| {
				letters
					.iter()
					.any(|letter| !before.contains(&letter.receipt))
			});

			if fresh {
				state.code.set(String::new());
				state.light_for(Stage::Sent);
				page.shell.announce("Code sent. It is in your demo inbox.");
				state.to_code.request();
			} else {
				state.alert(
					"The code is on its way, but the inbox has nothing yet.",
					"Press Check for mail in a moment.",
				);
			}
		});
	}

	/// Signs in with the typed code, then leaves for where the visitor was
	/// going.
	fn verify(&self) {
		let state = self.state;
		let Some(me) = &self.view.identity else {
			return;
		};

		if state.busy.get_untracked() || state.blocked.get_untracked() {
			return;
		}

		let typed = state.code.get_untracked().trim().to_owned();

		if typed.is_empty() {
			state.alert(
				"Type the code from your demo inbox.",
				"Or press Fill in code on the newest message.",
			);
			state.invalid.set(true);
			state.to_code.request();
			return;
		}

		state.busy.set(true);
		state.clear_notices();
		state.light(Some(READ), Some(VERIFY));
		let page = self.clone();
		let input = EmailCode {
			email: me.email.clone(),
			code: typed,
		};
		crate::spawn(async move {
			match state.verify.mutate_async(input).await {
				Ok(SignInStep::SignedIn(_)) => {
					state.light(Some(SESSION), None);
					state.station.set(SESSION);
					page.shell.announce("Signed in.");
					// Only a path the server validated for this page.
					crate::go(&page.view.return_to);
				}
				Ok(SignInStep::SecondFactor(_)) => {
					// The demo's people have no second factor.
					state.busy.set(false);
					state.alert(
						"That didn't go through.",
						"Try signing in again. If it keeps happening, reload the page.",
					);
				}
				Err(error) => {
					state.busy.set(false);
					page.refusal(&error, "signing in");
				}
			}
		});
	}

	/// Puts the inbox's code in the form and carries the visitor to Sign in.
	fn fill(&self, code: &str) {
		let state = self.state;
		state.code.set(code.to_owned());
		state.clear_notices();
		state.light(Some(READ), Some(VERIFY));
		self.shell.announce("Code filled in. Press Sign in.");
		state.show_verify.request();
		state.to_verify.request();
	}

	/// Reads the inbox because the visitor asked.
	fn check_mail(&self) {
		let state = self.state;
		state.clear_notices();

		if self.view.identity.is_none() {
			return;
		}

		let page = self.clone();
		crate::spawn(async move {
			if page.load_inbox().await {
				page.shell
					.announce(if state.letters.with_untracked(Vec::is_empty) {
						"Nothing new in your inbox."
					} else {
						"Inbox checked."
					});
			}
		});
	}

	/// Asks the server for `persona`, then opens the sign-in it names.
	fn choose(&self, persona: Persona) {
		let state = self.state;

		if state.busy.get_untracked() || state.full.get_untracked() {
			return;
		}

		state.busy.set(true);
		state.clear_notices();
		let page = self.clone();
		crate::spawn(async move {
			match leptoskit::rpc::client::call(ops::Mint {
				persona,
				return_to: page.view.return_to.clone(),
			})
			.await
			{
				Ok(minted) => crate::go(&minted.next),
				Err(error) => {
					let problem = front::problem_of(&error, (page.shell.seconds_to_reset())());
					state.busy.set(false);
					state.full.set(problem.full);
					state.notice.set(Some(Notice {
						alert: problem.tone == Tone::Bad,
						title: problem.title.to_owned(),
						body: problem.body,
						start_again: false,
					}));
				}
			}
		});
	}

	/// On a narrow screen the inbox sits below the form: go to it, and to
	/// the newest message's action when it has one.
	fn peek(&self) {
		let state = self.state;
		state.show_inbox.request();

		let has_action = match state.stage.get_untracked() {
			Stage::Lost => true,
			Stage::Sent => {
				state.letters.with_untracked(|letters| {
					letters
						.last()
						.is_some_and(|letter| code_in(&letter.subject).is_some())
				})
			}
			Stage::Choose | Stage::Ready => false,
		};

		if has_action {
			state.to_mail_action.request();
		} else {
			state.to_refresh.request();
		}
	}
}

/// Reads the inbox once the page is open, when the server said it holds
/// mail or has lost some. The server never puts the mail in the page.
pub(crate) fn open(shell: &Shell, view: &Arc<SignIn>, state: &State) {
	if view.identity.is_none() || (view.mail.held == 0 && view.mail.lost_at_ms.is_none()) {
		return;
	}

	let page = Page {
		shell: shell.clone(),
		view: Arc::clone(view),
		state: *state,
	};
	// `new_sync` runs on the multi-thread server runtime. `Effect::new`
	// calls `spawn_local`, which panics there.
	Effect::new_sync(move |_| {
		let page = page.clone();
		crate::spawn(async move {
			page.load_inbox().await;
		});
	});
}

fn context_line(view: &SignIn) -> Option<Element> {
	let line = match (&view.flow, &view.identity) {
		(Flow::Plain, _) => return None,
		(Flow::Admin | Flow::Switch(_), Some(_)) | (Flow::Admin, None) => {
			chrome::rich(
				chrome::SECONDARY,
				&[("Then you'll go straight back to the Notes admin.", false)],
			)
		}
		(Flow::Switch(current), None) => {
			chrome::rich(
				chrome::SECONDARY,
				&[
					(
						"The admin asked Notes who's signing in. Right now that's ",
						false,
					),
					(&format!("{} #{}", current.first_name(), current.tag), true),
					("; pick Ada to get in.", false),
				],
			)
		}
	};

	Some(view! { <View test_id="flow-line">{line}</View> })
}

/// What one choice shows: an avatar, the name, what the admin does with
/// them, and a radio mark.
fn choice(persona: Persona, checked: bool) -> Element {
	let (blurb, avatar) = match persona {
		Persona::Ada => {
			(
				"Has the admin role. The admin lets her in.",
				"bg-[var(--admin-ember)] text-[#fffdf8]",
			)
		}
		Persona::Grace => {
			(
				"No admin role. The admin turns her away.",
				"bg-[var(--admin-sunken)] border border-[var(--admin-line-strong)] \
				 text-[var(--admin-ink)]",
			)
		}
	};
	let name = persona.first_name();
	let (frame, mark) = if checked {
		(
			"border-[var(--admin-ink)] bg-[var(--admin-sunken)]",
			"border-[var(--admin-ink)]",
		)
	} else {
		(
			"border-[var(--admin-line-strong)]",
			"border-[var(--admin-line-strong)]",
		)
	};

	view! {
		<View
			class=format!(
				"min-h-[64px] w-full flex-row items-center gap-3 rounded-xl border-[1.5px] \
				 bg-[var(--admin-surface)] p-3 text-start cursor-pointer \
				 hover:bg-[var(--admin-sunken)] {frame}"
			)
			test_id=format!("choose:{}", persona.slug())
		>
			<View
				class=format!("h-8 w-8 shrink-0 items-center justify-center rounded-[9px] {avatar}")
				aria_hidden=true
			>
				<Text class="text-sm font-bold">
					{name.chars().next().map(String::from).unwrap_or_default()}
				</Text>
			</View>
			<View class="min-w-0 flex-1 flex-col gap-px">
				<Text class="text-[15px] font-semibold text-[var(--admin-ink)]">{name}</Text>
				<Text class="text-[13px] text-[var(--admin-ink-2)]">{blurb}</Text>
			</View>
			<View
				class=format!(
					"h-[18px] w-[18px] shrink-0 items-center justify-center rounded-full \
					 border-[1.5px] {mark}"
				)
				aria_hidden=true
			>
				{checked.then(|| {
					view! { <View class="h-[9px] w-[9px] rounded-full bg-[var(--admin-ink)]" /> }
				})}
			</View>
		</View>
	}
}

fn choose_pane(page: &Page) -> Element {
	let state = page.state;
	let chosen = state.chosen.get();
	let selected = chosen.and_then(|persona| Persona::ALL.iter().position(|each| *each == persona));
	let group = RadioGroup::new(
		"choose",
		[
			"Ada. Has the admin role. The admin lets her in.",
			"Grace. No admin role. The admin turns her away.",
		],
		selected,
	)
	.option(0, choice(Persona::Ada, chosen == Some(Persona::Ada)))
	.option(1, choice(Persona::Grace, chosen == Some(Persona::Grace)))
	.on_change(move |index| state.chosen.set(Persona::ALL.get(index).copied()));
	let go = chosen.map_or_else(
		|| "Continue".to_owned(),
		|persona| format!("Continue as {}", persona.first_name()),
	);
	let press = {
		let page = page.clone();
		move || {
			if let Some(persona) = state.chosen.get_untracked() {
				page.choose(persona);
			}
		}
	};
	let enabled = chosen.is_some() && !state.busy.get() && !state.full.get();

	view! {
		<View class="flex-col gap-3" test_id="pane-choose">
			<Text id="choose-label" class=chrome::LABEL>"Who's signing in?"</Text>
			{Element::from(group)
				.class("flex-col gap-2.5")
				.aria_labelled_by(["choose-label"])
				.test_id("choose-group")}
			{chrome::raised(
				view! {
					<Button
						class=format!("{} w-full text-base", chrome::BUTTON_PRIMARY)
						test_id="choose-go"
						enabled=enabled
						on:press=press
					>
						{go}
					</Button>
				},
				true,
			)}
			<Text class=chrome::MUTED>
				"You get your own made-up person. Only this browser can sign in to them."
			</Text>
		</View>
	}
}

fn identity_pane(page: &Page, me: &Person) -> Element {
	let state = page.state;
	let ada = me.is_ada();
	let avatar = if ada {
		"bg-[var(--admin-ember)] text-[#fffdf8]"
	} else {
		"bg-[var(--admin-sunken)] border border-[var(--admin-line-strong)] text-[var(--admin-ink)]"
	};
	let chip = if ada {
		"border-[var(--admin-ink)] text-[var(--admin-ink)] font-semibold"
	} else {
		"border-[var(--admin-line-strong)] text-[var(--admin-ink-2)] font-medium"
	};
	let other = me.persona.other();
	let swap = {
		let page = page.clone();
		move || page.choose(other)
	};

	view! {
		<View class="flex-col gap-4" test_id="pane-identity">
			<View
				class="flex-row flex-wrap items-center gap-x-2.5 gap-y-1 rounded-[10px] border \
				       border-[var(--admin-line)] bg-[var(--admin-paper)] px-3 py-2.5"
				test_id="who"
			>
				<View
					class=format!(
						"h-8 w-8 shrink-0 items-center justify-center rounded-[9px] {avatar}"
					)
					aria_hidden=true
					test_id="who-avatar"
				>
					<Text class="text-sm font-bold">
						{me
							.first_name()
							.chars()
							.next()
							.map(String::from)
							.unwrap_or_default()}
					</Text>
				</View>
				<Text class="text-[15px] font-bold text-[var(--admin-ink)]" test_id="who-name">
					{me.first_name()}
				</Text>
				<Text
					class="font-[var(--admin-font-mono)] text-xs text-[var(--admin-ink-3)]"
					test_id="who-tag"
				>
					{format!("#{}", me.tag)}
				</Text>
				<View
					class=format!(
						"rounded-full border bg-[var(--admin-surface)] px-2.5 py-px {chip}"
					)
					test_id="who-role"
				>
					<Text class="text-xs">{if ada { "admin role" } else { "no admin role" }}</Text>
				</View>
				<View class="flex-1" />
				<Button
					class="min-h-[44px] flex-row items-center px-1 text-[13.5px] font-semibold \
					       text-[var(--admin-ink-2)] cursor-pointer"
					test_id="swap"
					enabled=!state.busy.get() && !state.full.get()
					on:press=swap
				>
					{format!("Be {} instead", other.first_name())}
				</Button>
			</View>
			<View class="flex-col gap-1.5">
				<Text id="email-label" class=chrome::LABEL>"Email"</Text>
				<TextInput
					id="email"
					test_id="email"
					value=me.email.clone()
					keyboard=Keyboard::Email
					read_only=true
					aria_labelled_by=["email-label"]
					aria_described_by=["email-help"]
					class="min-h-[44px] w-full rounded-lg border border-[var(--admin-line-strong)] \
					       bg-[var(--admin-sunken)] px-3 py-2.5 font-[var(--admin-font-mono)] \
					       text-[13.5px] text-[var(--admin-ink-2)]"
				/>
				<Text
					id="email-help"
					class="text-[12.5px] leading-normal text-[var(--admin-ink-3)]"
				>
					"Made up for you. Mail to .invalid can never be delivered, so it lands in your \
					 demo inbox instead."
				</Text>
			</View>
		</View>
	}
}

/// How long until another code may be sent, in whole seconds.
fn cooldown_seconds(page: &Page) -> impl Fn() -> u64 + Clone + Send + Sync + 'static {
	let clock = page.shell.clock;
	let until = page.state.cooldown_until;

	move || {
		until
			.get()
			.saturating_sub(clock.get())
			.as_secs_f64()
			.ceil()
			.max(0.0) as u64
	}
}

fn code_pane(page: &Page) -> Element {
	let state = page.state;
	let invalid = state.invalid.get();
	let busy = state.busy.get() || state.blocked.get();
	let waiting = state.waiting.get();
	let change = move |value: &str| {
		state.code.set(value.to_owned());
		state.invalid.set(false);

		if state
			.notice
			.with_untracked(|notice| notice.as_ref().is_some_and(|notice| notice.alert))
		{
			state.notice.set(None);
		}

		if state.stage.get_untracked() == Stage::Sent {
			state.light_for(Stage::Sent);
		}
	};
	let keys = {
		let page = page.clone();
		move |key: &KeyPress| {
			if key.key == "Enter" {
				page.verify();
				EventOutcome::Handled
			} else {
				EventOutcome::Unhandled
			}
		}
	};
	let verify = {
		let page = page.clone();
		move || page.verify()
	};
	let resend = {
		let page = page.clone();
		move || page.send()
	};
	let peek = {
		let page = page.clone();
		move || page.peek()
	};
	let seconds = cooldown_seconds(page);
	let described = if invalid {
		vec!["code-error", "code-help"]
	} else {
		vec!["code-help"]
	};
	let border = if invalid {
		"border-2 border-[var(--admin-bad)]"
	} else {
		"border border-[var(--admin-line-strong)]"
	};

	view! {
		<View class="flex-col gap-3" test_id="pane-code">
			<View class="flex-col gap-1.5">
				<Text id="code-label" class=chrome::LABEL>"Code"</Text>
				<TextInput
					id="code"
					test_id="code"
					value=state.code.get()
					placeholder="XXXX-XXXX"
					autocomplete="one-time-code"
					max_length=9
					capitalize=Capitalize::Characters
					autocorrect=false
					submit_label=SubmitLabel::Go
					aria_invalid=invalid
					aria_labelled_by=["code-label"]
					aria_described_by=described
					focus_on=state.to_code
					on:change=change
					on:key=keys
					class=format!(
						"min-h-[48px] w-full rounded-lg bg-[var(--admin-sunken)] px-3 py-2 \
						 font-[var(--admin-font-mono)] text-[22px] tracking-widest \
						 text-[var(--admin-ink)] {border}"
					)
				/>
				<View class="flex-row flex-wrap items-center gap-x-1.5" id="code-help">
					<Text
						class="text-[12.5px] leading-normal text-[var(--admin-ink-3)] max-lg:hidden"
						test_id="code-help-wide"
					>
						"In your demo inbox →"
					</Text>
					<Button
						test_id="peek"
						aria_label="Open your demo inbox"
						class="min-h-[28px] text-[12.5px] font-semibold text-[var(--admin-ink-2)] \
						       cursor-pointer lg:hidden"
						on:press=peek
					>
						"Open your demo inbox ↓"
					</Button>
					<Text class="text-[12.5px] leading-normal text-[var(--admin-ink-3)]">
						"It works once, for 10 minutes."
					</Text>
				</View>
			</View>
			{chrome::raised(
				view! {
					<Button
						class=format!("{} w-full text-base", chrome::BUTTON_PRIMARY)
						test_id="verify"
						enabled=!busy
						focus_on=state.to_verify
						reveal_on=(state.show_verify, ScrollAlign::Center)
						on:press=verify
					>
						"Sign in"
					</Button>
				},
				true,
			)}
			<View class="flex-row flex-wrap items-center gap-2">
				<Button
					class=chrome::BUTTON_QUIET
					test_id="resend"
					enabled=!busy && !waiting
					on:press=resend
				>
					"Send a new code"
				</Button>
				<Text class=chrome::MUTED test_id="cooldown" aria_live=Live::Polite>
					{move || {
						match seconds() {
							0 => String::new(),
							seconds => format!("available in {}", wait_clock(seconds)),
						}
					}}
				</Text>
			</View>
		</View>
	}
}

fn send_pane(page: &Page) -> Element {
	let state = page.state;
	let enabled = !state.busy.get() && !state.blocked.get() && !state.waiting.get();
	let lost = state.stage.get() == Stage::Lost;
	let press = {
		let page = page.clone();
		move || page.send()
	};
	let seconds = cooldown_seconds(page);

	view! {
		<View class="flex-col gap-3" test_id="pane-send">
			{chrome::raised(
				view! {
					<Button
						class=format!("{} w-full text-base", chrome::BUTTON_PRIMARY)
						test_id="send"
						enabled=enabled
						on:press=press
					>
						{if lost {
							"Send a new code"
						} else {
							"Send a code"
						}}
					</Button>
				},
				true,
			)}
			<Text class=chrome::MUTED test_id="send-help">
				{move || {
					match seconds() {
						0 => {
							"Notes emails a one-time code. Here, the email waits in your demo \
							 inbox."
								.to_owned()
						}
						seconds => format!("Another code is available in {}.", wait_clock(seconds)),
					}
				}}
			</Text>
		</View>
	}
}

/// The alert or status above the form, when the page has something to say.
fn notice(page: &Page) -> Option<Element> {
	let notice = page.state.notice.get()?;
	let children = if notice.start_again {
		vec![view! {
			<Pressable
				href=format!("/sign-in?return_to={}", query_value(&page.view.return_to))
				test_id="start-again"
				aria_label="Start again"
				class="self-start text-sm font-semibold text-[var(--admin-ink-2)] \
				       cursor-pointer"
			>
				<Text>"Start again"</Text>
			</Pressable>
		}]
	} else {
		Vec::new()
	};

	Some(if notice.alert {
		chrome::status(
			Tone::Bad,
			"notice-alert",
			&notice.title,
			&notice.body,
			children,
		)
		.id("code-error")
	} else {
		chrome::status(
			Tone::Warn,
			"notice-status",
			&notice.title,
			&notice.body,
			children,
		)
	})
}

fn card(page: &Page) -> Element {
	let view = &page.view;
	let stage = page.state.stage.get();
	let new_day = view.new_day.then(|| {
		chrome::status(
			Tone::Info,
			"notice-new-day",
			"A new demo day started at 00:00 UTC.",
			"Yesterday's people were cleared, including your Ada or Grace. Pick who to be today.",
			Vec::new(),
		)
	});
	let panes = match (&view.identity, stage) {
		(Some(me), Stage::Sent) => vec![identity_pane(page, me), code_pane(page)],
		(Some(me), _) => vec![identity_pane(page, me), send_pane(page)],
		(None, _) => vec![choose_pane(page)],
	};
	// The notice sits between who is signing in and what they do next.
	let (first, rest) = panes.split_at(1);

	view! {
		<View class="relative w-full min-w-0">
			<View
				class="absolute inset-0 translate-x-1.5 translate-y-1.5 rounded-2xl \
				       bg-[var(--admin-ink)] md:translate-x-2 md:translate-y-2"
				aria_hidden=true
			/>
			<View class="relative w-full flex-col gap-[18px] rounded-2xl border border-[var(--admin-line)] \
				       bg-[var(--admin-surface)] p-[18px] md:p-[30px]">
				<View class="flex-col gap-2">
					<Heading
						aria_level=1
						id="signin-title"
						tab_index=-1
						class="text-2xl font-semibold leading-tight text-[var(--admin-ink)]"
					>
						"Sign in to Notes"
					</Heading>
					{context_line(view)}
				</View>
				{new_day}
				{first.to_vec()}
				{notice(page)}
				{rest.to_vec()}
			</View>
		</View>
	}
}

fn capitalize(word: &str) -> String {
	let mut chars = word.chars();
	chars
		.next()
		.map(|first| first.to_uppercase().chain(chars).collect())
		.unwrap_or_default()
}

/// The rail above the station names: a track, how much of it is behind the
/// visitor, and a dot for each station.
fn rail(lit: Lit) -> Element {
	let fill = match lit.done {
		None | Some(SEND) => "w-0",
		Some(MAIL) => "w-[20%]",
		Some(READ) => "w-[40%]",
		Some(VERIFY) => "w-[60%]",
		Some(_) => "w-[80%]",
	};
	let dots = (0..STATIONS.len())
		.map(|index| {
			let done = lit.done.is_some_and(|done| index <= done);
			let now = lit.now == Some(index);
			let dot = match (done, now) {
				(true, _) => "border-[#5ef2c2] bg-[#5ef2c2]",
				(false, true) => "border-[#5ef2c2] bg-[#16111f]",
				(false, false) => "border-[#4a3f58] bg-[#16111f]",
			};
			view! {
				<View class="flex-1 min-w-0 items-center">
					<View
						class=format!(
							"h-4 w-4 shrink-0 rounded-full border-2 transition-colors {dot}"
						)
						test_id=format!("station-dot:{}", STATIONS[index].0)
					/>
				</View>
			}
		})
		.collect::<Vec<_>>();

	view! {
		<View class="relative flex-row" aria_hidden=true test_id="station-rail">
			<View class="absolute left-[10%] right-[10%] top-[7px] h-[2px] bg-[#30283c]" />
			<View class=format!(
				"absolute left-[10%] top-[7px] h-[2px] bg-[#5ef2c2] transition-all duration-500 \
				 {fill}"
			) />
			{dots}
		</View>
	}
}

/// The stations and the explanation of the selected one.
fn stations(state: &State) -> Vec<Element> {
	let selected = state.station.get();
	let state = *state;
	let progress = state.lit.get();
	let tabs = Tabs::new("station", STATIONS.len(), selected);
	let list = tabs
		.list(move |index| state.station.set(index))
		.class("flex-row")
		.aria_label("How this sign-in works, step by step. Use the arrow keys to read each step.")
		.test_id("stations")
		.children(STATIONS.iter().enumerate().map(|(index, (name, ..))| {
			let colour = if index == selected {
				"text-[#5ef2c2]"
			} else if progress.done.is_some_and(|done| index <= done) {
				"text-[#f4efe8]"
			} else {
				"text-[#9d93a8]"
			};
			tabs.tab(index, capitalize(name))
				.test_id(format!("station:{name}"))
				.aria_label(format!("{}, step {} of 5", capitalize(name), index + 1))
				.class(format!(
					"min-h-[36px] flex-1 min-w-0 items-center justify-center text-xs \
					 font-semibold cursor-pointer {colour}"
				))
		}));
	let panels = STATIONS.iter().enumerate().map(|(index, (_, sig, text))| {
		let shown = index == selected;
		let sig_id = if shown {
			"explain-sig".to_owned()
		} else {
			format!("explain-sig:{index}")
		};
		let text_id = if shown {
			"explain-text".to_owned()
		} else {
			format!("explain-text:{index}")
		};
		tabs.panel(index)
			.aria_live(Live::Polite)
			.class("flex-col gap-1 rounded-[10px] border border-[#30283c] bg-[#0b0811] px-3 py-2.5")
			.test_id(if shown {
				"station-panel".to_owned()
			} else {
				format!("station-panel:{index}")
			})
			.child(view! {
				<Text class="font-[var(--admin-font-mono)] text-xs text-[#5ef2c2]" test_id=sig_id>
					{(*sig).to_owned()}
				</Text>
			})
			.child(view! {
				<Text class="text-[13px] leading-normal text-[#c9c0d0]" test_id=text_id>
					{(*text).to_owned()}
				</Text>
			})
	});

	std::iter::once(rail(progress))
		.chain(std::iter::once(list))
		.chain(panels)
		.collect()
}

/// The row that says the inbox is waiting.
fn listening(text: &str) -> Element {
	view! {
		<View
			class="flex-row items-center gap-2.5 rounded-[10px] border border-[#4a3f58] \
			       px-3.5 py-4"
			test_id="listening"
		>
			<View
				class="h-[9px] w-[9px] shrink-0 rounded-full bg-[#5ef2c2]"
				test_id="pulse"
				aria_hidden=true
			/>
			<Text class="flex-1 text-[13.5px] leading-normal text-[#c9c0d0]">
				{text.to_owned()}
			</Text>
		</View>
	}
}

/// The paper button the inbox's own actions use.
const BUTTON_PAPER: &str = "min-h-[44px] rounded-lg border border-[#f4efe8] bg-[#f4efe8] px-4 \
                            py-2 text-sm font-semibold text-[#16111f] cursor-pointer";

/// One line of a message's header.
fn meta(term: &str, value: &str, mono: bool) -> Element {
	let face = if mono {
		"font-[var(--admin-font-mono)] text-xs"
	} else {
		"text-[12.5px]"
	};

	view! {
		<View class="flex-row gap-2.5">
			<Text class="w-9 shrink-0 text-[12.5px] text-[#9d93a8]">{term.to_owned()}</Text>
			<Text class=format!("min-w-0 flex-1 text-[#c9c0d0] {face}")>{value.to_owned()}</Text>
		</View>
	}
}

/// One message. The newest shows its text and offers its code; an older one
/// is struck through, because a newer code stops it working.
fn letter(page: &Page, letter: &Letter, current: bool) -> Element {
	let code = code_in(&letter.subject).map(|code| {
		let action = if current {
			let page = page.clone();
			let filled = code.to_owned();
			view! {
				<Button
					class=BUTTON_PAPER
					test_id="fill"
					focus_on=page.state.to_mail_action
					on:press=move || page.fill(&filled)
				>
					"Fill in code"
				</Button>
			}
		} else {
			view! {
				<Text class="text-[12.5px] text-[#9d93a8]">
					"Replaced: a newer code stops this one working."
				</Text>
			}
		};
		let face = if current {
			"text-[26px] text-[#f4efe8]"
		} else {
			"text-lg text-[#f4efe8]"
		};

		view! {
			<View class="flex-row flex-wrap items-center justify-between gap-3 border-t border-[#30283c] pt-2.5">
				<Text
					class=format!("font-[var(--admin-font-mono)] tracking-widest {face}")
					test_id=if current {
						"mail-code"
					} else {
						"mail-code-old"
					}
				>
					{code.to_owned()}
				</Text>
				{action}
			</View>
		}
	});

	view! {
		<View
			class=format!(
				"flex-col gap-2.5 rounded-xl border border-[#30283c] bg-[#0b0811] p-3.5 \
				 text-[#c9c0d0] {}",
				if current { "" } else { "opacity-60" }
			)
			role=Role::Article
			aria_label=if current {
				"Newest message"
			} else {
				"Older message, replaced by a newer code"
			}
			test_id=if current { "mail" } else { "mail-old" }
		>
			<View class="flex-col gap-0.5">
				{meta("From", "Notes", false)} {meta("To", &letter.to, true)}
				{meta(
					"At",
					&format!("{} UTC", utc_clock(letter.sent_at_ms)),
					false,
				)}
			</View>
			<Text class="text-sm font-semibold leading-snug text-[#f4efe8]">
				{letter.subject.clone()}
			</Text>
			{current.then(|| {
				view! {
					<Text class="text-[13px] leading-normal" test_id="mail-text">
						{letter.text.clone()}
					</Text>
				}
			})}
			{code}
		</View>
	}
}

/// What the inbox holds, newest first.
fn mailbox(page: &Page) -> Vec<Element> {
	let state = page.state;
	let now = state.stage.get();

	if now == Stage::Choose || page.view.identity.is_none() {
		return vec![listening(
			"Choose who's signing in. Their mail will land here.",
		)];
	}

	let letters = state.letters.get();
	let lost = now == Stage::Lost;
	let mut rows = Vec::new();

	if lost {
		let send = {
			let page = page.clone();
			move || page.send()
		};
		let at = state.lost_at_ms.get().map(utc_time).unwrap_or_default();
		rows.push(view! {
			<View
				class="flex-col gap-2.5 rounded-xl border border-[#30283c] p-3.5 \
				       text-[#c9c0d0]"
				role=Role::Article
				aria_label="A code this server no longer has"
				test_id="mail-gone"
			>
				<Text class="text-sm font-semibold leading-snug text-[#f4efe8]">
					{format!("A code was sent at {at} UTC, but this server no longer has it.")}
				</Text>
				<Text class="text-[13px] leading-normal">
					"Codes live only in the memory of the server that sent them, so this one is \
					 gone from here. A new release can do that. Your account and any session are \
					 fine."
				</Text>
				<View class="flex-row">
					<Button
						class=BUTTON_PAPER
						test_id="mail-send"
						enabled=!state.busy.get() && !state.blocked.get() && !state.waiting.get()
						focus_on=state.to_mail_action
						on:press=send
					>
						"Send a new code"
					</Button>
				</View>
			</View>
		});
	} else if letters.is_empty() {
		return vec![listening(
			"Waiting for mail. Press Send a code and watch it land here.",
		)];
	}

	let newest = letters.len().saturating_sub(1);
	rows.extend(
		letters
			.iter()
			.enumerate()
			.rev()
			.map(|(index, each)| letter(page, each, !lost && index == newest)),
	);

	rows
}

fn inbox(page: &Page) -> Element {
	let state = page.state;
	let refresh = {
		let page = page.clone();
		move || page.check_mail()
	};
	let host = page
		.shell
		.context
		.site_host
		.rsplit_once(':')
		.filter(|(_, port)| port.bytes().all(|byte| byte.is_ascii_digit()))
		.map_or(page.shell.context.site_host.as_str(), |(host, _)| host)
		.to_owned();

	view! {
		<View
			class="min-w-0 flex-col gap-4 rounded-2xl border border-[var(--admin-underside-line)] \
			       bg-[var(--admin-underside)] p-4 text-[#f4efe8] md:p-5"
			role=Role::Complementary
			aria_labelled_by=["inbox-title"]
			test_id="inbox"
			reveal_on=(state.show_inbox, ScrollAlign::Start)
		>
			<View class="flex-row flex-wrap items-center gap-2.5">
				{chrome::piece()}
				<Heading
					aria_level=2
					id="inbox-title"
					class="text-base font-semibold text-[#f4efe8]"
				>
					"Demo inbox"
				</Heading>
				<View class="rounded-full border border-[#4a3f58] px-2.5 py-px">
					<Text class="text-xs font-medium text-[#c9c0d0]">"only this browser"</Text>
				</View>
			</View>
			<Text class="text-[13.5px] leading-normal text-[#c9c0d0]">
				"Sign-in really sends an email. The demo's Mailer keeps it here instead, so nothing \
				 leaves the server."
			</Text>
			{stations(&state)}
			<View
				class="flex-col gap-2.5"
				id="mailbox"
				test_id="mailbox"
				role=Role::Group
				aria_label="Mail in your demo inbox"
			>
				{mailbox(page)}
			</View>
			<View class="flex-row flex-wrap items-center justify-between gap-2 border-t border-[#30283c] pt-3">
				<Text class="min-w-0 flex-1 text-[12.5px] leading-normal text-[#9d93a8]">
					{format!(
						"Keeps your last 3 messages for 10 minutes, in this server's memory. Tied \
						 to this browser by a cookie only {host} can read."
					)}
				</Text>
				<Button
					test_id="inbox-refresh"
					class="min-h-[44px] flex-row items-center rounded-lg border border-[#4a3f58] px-3 \
					       py-2 text-[13px] font-semibold text-[#f4efe8] cursor-pointer \
					       hover:bg-[#251e30]"
					focus_on=state.to_refresh
					on:press=refresh
				>
					"Check for mail"
				</Button>
			</View>
		</View>
	}
}

/// The sign-in page.
pub(crate) fn page(shell: &Shell, view: &Arc<SignIn>, state: &State) -> Element {
	let page = Page {
		shell: shell.clone(),
		view: Arc::clone(view),
		state: *state,
	};

	view! {
		<View class=chrome::PAGE>
			{chrome::skip_link()} {chrome::band(shell)} {chrome::app_bar(Vec::new())}
			<View
				id="main"
				role=Role::Main
				tab_index=-1
				class=format!("{} py-8 md:py-10", chrome::COLUMN)
			>
				<View class="grid grid-cols-1 items-start gap-7 lg:grid-cols-[minmax(0,1fr)_minmax(0,1.05fr)]">
					{card(&page)} {inbox(&page)}
				</View>
			</View>
			{chrome::reset_dialog(shell)} {chrome::announcer(shell)}
		</View>
	}
}

/// What a visitor without scripts gets on sign-in. Choosing is a plain form
/// post, as on the front door; sending and checking a code needs the page's
/// client, and says so.
#[must_use]
pub fn noscript(view: &SignIn) -> String {
	if view.identity.is_none() {
		return front::noscript_form(&view.return_to);
	}

	"<noscript><p class=\"plain-mint\">Signing in sends and checks an emailed code, which needs \
	 scripts on. Turn them on and reload this page.</p></noscript>"
		.to_owned()
}
