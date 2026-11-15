//! Sign-in: this browser's own person, the real emailed code, and the
//! demo inbox drawn as the page's underside.
//!
//! The server decides what it can: which person is chosen, what the
//! sign-in is for, and whether this process still holds mail. The page's
//! script runs the rest against Auth's real routes, and reads the inbox
//! only after a send or when asked.

use leptoskit::native::Live;
use leptoskit::native::Role;
use leptoskit::prelude::*;
use serde_json::json;

use super::Context;
use super::Person;
use super::chrome;
use super::chrome::Tone;
use crate::return_to;

/// Why the visitor is signing in, which the page says in one line.
#[derive(Clone, Debug)]
pub(crate) enum Flow {
	/// Straight from Notes.
	Plain,
	/// The admin sent a signed-out visitor to sign in.
	Admin,
	/// The admin asked Notes who is signing in, and someone already is.
	Switch(Person),
}

/// What mail this process holds for the chosen person, without the mail:
/// the page's script reads the messages themselves from the private
/// inbox.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Mail {
	/// How many messages this process still holds for the person.
	pub held: usize,
	/// When a code the person was sent can no longer be shown, as Unix
	/// milliseconds: the process that held it has gone, or dropped it.
	pub lost_at_ms: Option<i64>,
}

/// What sign-in is rendered from.
#[derive(Clone, Debug)]
pub(crate) struct SignIn {
	pub context: Context,
	/// The validated local path to end at.
	pub return_to: String,
	pub flow: Flow,
	/// The person signing in, once chosen; `None` shows the choice.
	pub identity: Option<Person>,
	/// Who the choice starts on, when the visitor already leans one way.
	pub preselect: Option<&'static str>,
	/// The pass was from an earlier day and has been cleared.
	pub new_day: bool,
	pub mail: Mail,
}

/// The stations, in order: name, and the real call each stands for.
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
		"GET /demo/inbox (pass cookie)",
		"You read the code here, as you would in your email app. The inbox answers only for this \
		 browser's pass, never for an address in a URL.",
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

/// The initial state the root carries, which the script then drives.
fn initial_state(view: &SignIn) -> &'static str {
	if view.identity.is_none() {
		"choose"
	} else if view.mail.lost_at_ms.is_some() {
		"lost"
	} else if view.mail.held > 0 {
		"sent"
	} else {
		"ready"
	}
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

fn choice(persona: &'static str, preselected: bool) -> Element {
	let (name, blurb, avatar) = if persona == "ada" {
		(
			"Ada",
			"Has the admin role. The admin lets her in.",
			"bg-[var(--admin-ember)] text-[#fffdf8]",
		)
	} else {
		(
			"Grace",
			"No admin role. The admin turns her away.",
			"bg-[var(--admin-sunken)] border border-[var(--admin-line-strong)] \
			 text-[var(--admin-ink)]",
		)
	};
	view! {
		<Pressable
			role=Role::Radio
			aria_checked=preselected
			tab_index=if preselected || persona == "ada" {
				0
			} else {
				-1
			}
			test_id=format!("choose:{persona}")
			aria_label=format!("{name}. {blurb}")
			class="min-h-[64px] w-full flex-row items-center gap-3 rounded-xl border-[1.5px] \
			       border-[var(--admin-line-strong)] bg-[var(--admin-surface)] p-3 text-start \
			       cursor-pointer hover:bg-[var(--admin-sunken)]"
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
				class="h-[18px] w-[18px] shrink-0 items-center justify-center rounded-full border-[1.5px] \
				       border-[var(--admin-line-strong)]"
				test_id="choice-radio"
				aria_hidden=true
			>
				<View
					class="h-[9px] w-[9px] rounded-full bg-[var(--admin-ink)]"
					test_id="choice-radio-dot"
				/>
			</View>
		</Pressable>
	}
}

fn choose_pane(view: &SignIn) -> Element {
	let preselect = view.preselect;
	let go = if let Some(persona) = preselect {
		format!(
			"Continue as {}",
			if persona == "ada" { "Ada" } else { "Grace" }
		)
	} else {
		"Continue".to_owned()
	};

	view! {
		<View class="flex-col gap-3" test_id="pane-choose">
			<Text id="choose-label" class=chrome::LABEL>"Who's signing in?"</Text>
			<View
				class="flex-col gap-2.5"
				role=Role::RadioGroup
				aria_labelled_by=["choose-label"]
				test_id="choose-group"
			>
				{choice("ada", preselect == Some("ada"))}
				{choice("grace", preselect == Some("grace"))}
			</View>
			{chrome::raised(
				view! {
					<Button
						class=format!("{} w-full text-base", chrome::BUTTON_PRIMARY)
						test_id="choose-go"
						enabled=preselect.is_some()
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

fn identity_pane(view: &SignIn) -> Element {
	let (initial, name, tag, role, ada, email) = match &view.identity {
		Some(person) => {
			(
				person
					.first_name()
					.chars()
					.next()
					.map(String::from)
					.unwrap_or_default(),
				person.first_name().to_owned(),
				format!("#{}", person.tag),
				if person.is_ada() {
					"admin role"
				} else {
					"no admin role"
				},
				person.is_ada(),
				person.email.clone(),
			)
		}
		None => {
			(
				String::new(),
				String::new(),
				String::new(),
				"",
				true,
				String::new(),
			)
		}
	};
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
	let swap = match view.identity.as_ref().map(|person| person.persona) {
		Some("grace") => "Be Ada instead",
		_ => "Be Grace instead",
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
					<Text class="text-sm font-bold">{initial}</Text>
				</View>
				<Text class="text-[15px] font-bold text-[var(--admin-ink)]" test_id="who-name">
					{name}
				</Text>
				<Text
					class="font-[var(--admin-font-mono)] text-xs text-[var(--admin-ink-3)]"
					test_id="who-tag"
				>
					{tag}
				</Text>
				<View
					class=format!(
						"rounded-full border bg-[var(--admin-surface)] px-2.5 py-px {chip}"
					)
					test_id="who-role"
				>
					<Text class="text-xs">{role}</Text>
				</View>
				<View class="flex-1" />
				<Button
					class="min-h-[44px] flex-row items-center px-1 text-[13.5px] font-semibold  \
					       text-[var(--admin-ink-2)] cursor-pointer"
					test_id="swap"
				>
					{swap}
				</Button>
			</View>
			<View class="flex-col gap-1.5">
				<Text id="email-label" class=chrome::LABEL>"Email"</Text>
				<TextInput
					id="email"
					test_id="email"
					value=email
					aria_read_only=true
					aria_labelled_by=["email-label"]
					aria_described_by=["email-help"]
					class="min-h-[44px] w-full rounded-lg border  \
					       border-[var(--admin-line-strong)] bg-[var(--admin-sunken)] px-3 py-2.5 \
					       font-[var(--admin-font-mono)] text-[13.5px] text-[var(--admin-ink-2)]"
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

fn code_pane() -> Element {
	view! {
		<View class="flex-col gap-3" test_id="pane-code">
			<View class="flex-col gap-1.5">
				<Text id="code-label" class=chrome::LABEL>"Code"</Text>
				<TextInput
					id="code"
					test_id="code"
					placeholder="XXXX-XXXX"
					autocomplete="one-time-code"
					aria_labelled_by=["code-label"]
					aria_described_by=["code-help"]
					class="min-h-[48px] w-full rounded-lg border border-[var(--admin-line-strong)] \
					       bg-[var(--admin-sunken)] px-3 py-2 font-[var(--admin-font-mono)] text-[22px] \
					        tracking-widest text-[var(--admin-ink)]"
				/>
				<View class="flex-row flex-wrap items-center gap-x-1.5" id="code-help">
					<Text
						class="text-[12.5px] leading-normal text-[var(--admin-ink-3)]"
						test_id="code-help-wide"
					>
						"In your demo inbox →"
					</Text>
					<Pressable
						href="#inbox-title"
						test_id="peek"
						aria_label="Open your demo inbox"
						class="min-h-[28px] text-[12.5px] font-semibold  text-[var(--admin-ink-2)] \
						       cursor-pointer"
					>
						<Text>"Open your demo inbox ↓"</Text>
					</Pressable>
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
					>
						"Sign in"
					</Button>
				},
				true,
			)}
			<View class="flex-row flex-wrap items-center gap-2">
				<Button class=chrome::BUTTON_QUIET test_id="resend">"Send a new code"</Button>
				<Text class=chrome::MUTED test_id="cooldown" aria_live=Live::Polite />
			</View>
		</View>
	}
}

fn send_pane() -> Element {
	view! {
		<View class="flex-col gap-3" test_id="pane-send">
			{chrome::raised(
				view! {
					<Button
						class=format!("{} w-full text-base", chrome::BUTTON_PRIMARY)
						test_id="send"
					>
						"Send a code"
					</Button>
				},
				true,
			)}
			<Text class=chrome::MUTED test_id="send-help">
				"Notes emails a one-time code. Here, the email waits in your demo inbox."
			</Text>
		</View>
	}
}

fn card(view: &SignIn) -> Element {
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
				{if view.new_day {
					Some(chrome::status(
						Tone::Info,
						"notice-new-day",
						"A new demo day started at 00:00 UTC.",
						"Yesterday's people were cleared, including your Ada or Grace. Pick who \
						 to be today.",
						Vec::new(),
					))
				} else {
					None
				}}
				{choose_pane(view)}
				{identity_pane(view)}
				{chrome::status(Tone::Bad, "notice-alert", "", "", Vec::new())}
				{chrome::status(Tone::Warn, "notice-status", "", "", Vec::new())}
				{send_pane()}
				{code_pane()}
			</View>
		</View>
	}
}

fn station(index: usize, name: &str, selected: bool) -> Element {
	view! {
		<Pressable
			role=Role::Tab
			aria_selected=selected
			tab_index=if selected { 0 } else { -1 }
			aria_controls=["station-panel"]
			test_id=format!("station:{name}")
			id=format!("station-tab-{name}")
			aria_label=format!("{}, step {} of 5", capitalize(name), index + 1)
			class="min-h-[44px] flex-1 min-w-0 flex-col items-center gap-1.5 text-xs font-semibold \
			       text-[#9d93a8] cursor-pointer"
		>
			<View
				class="h-4 w-4 shrink-0 rounded-full border-2 border-[#4a3f58] bg-[#16111f]"
				test_id="station-dot"
				aria_hidden=true
			/>
			<Text aria_hidden=true>{capitalize(name)}</Text>
		</Pressable>
	}
}

fn capitalize(word: &str) -> String {
	let mut chars = word.chars();
	chars
		.next()
		.map(|first| first.to_uppercase().chain(chars).collect())
		.unwrap_or_default()
}

fn inbox(view: &SignIn) -> Element {
	let waiting = if view.identity.is_some() {
		"Waiting for mail. Press Send a code and watch it land here."
	} else {
		"Choose who's signing in. Their mail will land here."
	};
	let (first_sig, first_text) = (STATIONS[0].1, STATIONS[0].2);

	view! {
		<View
			class="min-w-0 flex-col gap-4 rounded-2xl border border-[var(--admin-underside-line)] \
			       bg-[var(--admin-underside)] p-4 text-[#f4efe8] md:p-5"
			role=Role::Complementary
			aria_labelled_by=["inbox-title"]
			test_id="inbox"
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
			<View
				class="flex-row"
				role=Role::TabList
				aria_label="How this sign-in works, step by step. Use the arrow keys to read each step."
				test_id="stations"
			>
				{STATIONS
					.iter()
					.enumerate()
					.map(|(index, (name, ..))| station(index, name, index == 0))
					.collect::<Vec<_>>()}
			</View>
			<View
				id="station-panel"
				test_id="station-panel"
				role=Role::TabPanel
				aria_labelled_by=["station-tab-send"]
				aria_live=Live::Polite
				class="flex-col gap-1 rounded-[10px] border border-[#30283c] bg-[#0b0811] px-3 py-2.5"
			>
				<Text
					class="font-[var(--admin-font-mono)] text-xs text-[#5ef2c2] "
					test_id="explain-sig"
				>
					{first_sig}
				</Text>
				<Text class="text-[13px] leading-normal text-[#c9c0d0]" test_id="explain-text">
					{first_text}
				</Text>
			</View>
			<View
				class="flex-col gap-2.5"
				id="mailbox"
				test_id="mailbox"
				aria_label="Mail in your demo inbox"
			>
				<View
					class="flex-row items-center gap-2.5 rounded-[10px] border  \
					       border-[#4a3f58] px-3.5 py-4"
					test_id="listening"
				>
					<View
						class="h-[9px] w-[9px] shrink-0 rounded-full bg-[#5ef2c2]"
						test_id="pulse"
						aria_hidden=true
					/>
					<Text class="flex-1 text-[13.5px] leading-normal text-[#c9c0d0]">
						{waiting}
					</Text>
				</View>
			</View>
			<View class="flex-row flex-wrap items-center justify-between gap-2 border-t border-[#30283c] pt-3">
				<Text class="min-w-0 flex-1 text-[12.5px] leading-normal text-[#9d93a8]">
					{format!(
						"Keeps your last 3 messages for 10 minutes, in this server's memory. Tied \
						 to this browser by a cookie only {} can read.",
						super::host_only(&view.context.site_host)
					)}
				</Text>
				<Button
					test_id="inbox-refresh"
					class="min-h-[44px] flex-row items-center rounded-lg border border-[#4a3f58] px-3 \
					       py-2 text-[13px] font-semibold text-[#f4efe8] cursor-pointer hover:bg-[#251e30]"
				>
					"Check for mail"
				</Button>
			</View>
		</View>
	}
}

/// The sign-in page as a document.
pub(super) fn page(view: &SignIn) -> String {
	let tree = view! {
		<View class=chrome::PAGE>
			{chrome::skip_link()} {chrome::band(&view.context)} {chrome::app_bar(Vec::new())}
			<View
				id="main"
				role=Role::Main
				tab_index=-1
				class=format!("{} py-8 md:py-10", chrome::COLUMN)
			>
				<View class="grid grid-cols-1 items-start gap-7 lg:grid-cols-[minmax(0,1fr)_minmax(0,1.05fr)]">
					{card(view)} {inbox(view)}
				</View>
			</View>
			{chrome::reset_dialog()}
		</View>
	};

	let flow = match view.flow {
		Flow::Plain => "plain",
		Flow::Admin => "admin",
		Flow::Switch(_) => "switch",
	};
	let state = initial_state(view);
	let mut data = super::base_state("sign-in", &view.context, &view.return_to);
	data["flow"] = json!(flow);
	data["identity"] = json!(view.identity);
	data["preselect"] = json!(view.preselect);
	data["mail"] = json!({ "held": view.mail.held, "lostAtMs": view.mail.lost_at_ms });
	data["stations"] = json!(
		STATIONS
			.iter()
			.map(|(name, sig, text)| json!({ "name": name, "sig": sig, "text": text }))
			.collect::<Vec<_>>()
	);
	data["signInTarget"] = json!(sign_in_target(view));

	super::document(
		"Sign in · Notes",
		&format!(" data-page=\"sign-in\" data-flow=\"{flow}\" data-state=\"{state}\""),
		&data,
		tree,
		"<noscript><p class=\"plain-mint\">Signing in sends and checks an emailed code, which \
		 needs scripts on. Turn them on and reload this page.</p></noscript>",
	)
}

/// The sign-in path a swap or a new person starts again from, with the
/// validated return target.
fn sign_in_target(view: &SignIn) -> String {
	format!(
		"/sign-in?return_to={}",
		return_to::encoded_query_value(&view.return_to)
	)
}
