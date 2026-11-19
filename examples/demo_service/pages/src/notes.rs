//! Notes, signed in: the person's private notebook, the way to the real
//! admin, and the account menu.
//!
//! The server draws the page from who the request is signed in as. The
//! notes themselves are read once the page is open, through the `notes`
//! operation, and saved through `save_note`: the server answers only for the
//! session's own account, whatever the page sends. A note is always drawn as
//! text.

use std::sync::Arc;
use std::time::Duration;

use leptos::prelude::Effect;
use leptoskit::aria::Menu;
use leptoskit::auth::ErrorResponse;
use leptoskit::auth::client::sign_out_mutation;
use leptoskit::native::KeyPress;
use leptoskit::native::LinkTarget;
use leptoskit::native::Placement;
use leptoskit::native::Role;
use leptoskit::prelude::*;
use leptoskit::query::MutationHandle;
use leptoskit::query::use_mutation;
use leptoskit::rpc::FnError;

use crate::Shell;
use crate::chrome;
use crate::chrome::Tone;
use crate::model::Welcome;
use crate::model::utc_time;
use crate::ops;
use crate::ops::Note;

/// Where the sign-in that brings a visitor back here starts.
const SIGN_IN_AGAIN: &str = "/sign-in?return_to=%2Fnotes";

/// A card on the page's paper.
const CARD: &str =
	"flex-col gap-3 rounded-xl border border-[var(--admin-line)] bg-[var(--admin-surface)] p-4";

/// How long a message that has nowhere else to go stays on screen.
const TOAST_FOR: Duration = Duration::from_secs(8);

/// What reading the notebook has come to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Listing {
	/// The page has asked, and nothing has answered yet.
	Loading,
	/// The notes on screen are the server's.
	Ready,
	/// The read failed with nothing on screen.
	Failed,
	/// The visit is no longer signed in.
	SignedOut,
}

/// What the last save, or the last read that went wrong, came to.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Outcome {
	/// Saved, and analytics took its event.
	Saved { words: u32, actor: String },
	/// Saved, and analytics did not confirm its event.
	Unconfirmed,
	/// Something stopped the save: the text is still in the box.
	Problem {
		title: &'static str,
		body: String,
		/// Whether to offer the way back to signing in.
		sign_in: bool,
	},
}

/// What Notes keeps while it is open.
#[derive(Clone, Copy)]
pub(crate) struct State {
	notes: RwSignal<Vec<Note>>,
	listing: RwSignal<Listing>,
	/// What is typed in the field.
	draft: RwSignal<String>,
	/// A problem with the draft, said at the field.
	field_error: RwSignal<Option<String>>,
	saving: RwSignal<bool>,
	outcome: RwSignal<Option<Outcome>>,
	/// The note this page just saved, which comes in with a little motion.
	fresh: RwSignal<Option<String>>,
	/// Whether the account menu is open.
	menu_open: RwSignal<bool>,
	/// A message with nowhere else to go, such as a failed sign-out.
	toast: RwSignal<Option<String>>,
	/// Whether the account is on its way out.
	leaving: RwSignal<bool>,
	sign_out: MutationHandle<(), (), ErrorResponse>,
	to_field: FocusRequest,
}

impl State {
	pub(crate) fn new() -> Self {
		Self {
			notes: RwSignal::new(Vec::new()),
			listing: RwSignal::new(Listing::Loading),
			draft: RwSignal::new(String::new()),
			field_error: RwSignal::new(None),
			saving: RwSignal::new(false),
			outcome: RwSignal::new(None),
			fresh: RwSignal::new(None),
			menu_open: RwSignal::new(false),
			toast: RwSignal::new(None),
			leaving: RwSignal::new(false),
			sign_out: use_mutation(sign_out_mutation()),
			to_field: FocusRequest::new(),
		}
	}
}

/// Characters as the server counts them: Unicode scalar values, trimmed.
fn length(text: &str) -> usize {
	text.trim().chars().count()
}

fn plural(number: usize, word: &str) -> String {
	format!("{number} {word}{}", if number == 1 { "" } else { "s" })
}

/// Everything an action on this page needs.
#[derive(Clone)]
struct Page {
	shell: Shell,
	welcome: Arc<Welcome>,
	state: State,
}

impl Page {
	fn full(&self) -> bool {
		self.state.notes.with_untracked(Vec::len) >= self.welcome.max_notes
	}

	fn problem(&self, title: &'static str, body: &str) {
		self.state.outcome.set(Some(Outcome::Problem {
			title,
			body: body.to_owned(),
			sign_in: false,
		}));
	}

	/// The visit is not what the page was drawn for: the day ended, or the
	/// sign-in did.
	fn gone(&self, code: &str) {
		if code == ops::NEW_DAY {
			self.problem(
				"The demo reset",
				"Your notebook belonged to yesterday's demo. What you typed is still in the box.",
			);
			self.shell.reset_open.set(true);
		} else {
			self.state.outcome.set(Some(Outcome::Problem {
				title: "You're signed out",
				body: "Sign in again to keep writing. What you typed is still in the box."
					.to_owned(),
				sign_in: true,
			}));
		}
	}

	/// Reads the notebook. A failed read of a list already on screen stays
	/// quiet.
	async fn load(&self) {
		let state = self.state;

		match leptoskit::rpc::client::call(ops::Notes {}).await {
			Ok(notebook) => {
				state.notes.set(notebook.notes);
				state.listing.set(Listing::Ready);
			}
			Err(FnError::Refused { code, .. })
				if code == ops::SIGNED_OUT || code == ops::NO_PASS || code == ops::NEW_DAY =>
			{
				self.gone(&code);
				state.listing.set(Listing::SignedOut);
			}
			Err(_) => {
				if state.notes.with_untracked(Vec::is_empty) {
					state.listing.set(Listing::Failed);
				}
			}
		}
	}

	/// What is wrong with the draft before it is sent.
	fn draft_problem(&self) -> Option<String> {
		let used = length(&self.state.draft.get_untracked());
		let max = self.welcome.max_chars;

		if used == 0 {
			Some("Write something first.".to_owned())
		} else if used > max {
			Some(format!(
				"That's {} over the {max} limit.",
				plural(used - max, "character")
			))
		} else {
			None
		}
	}

	/// Says a problem with the draft at the field, and takes the visitor
	/// there.
	fn field_problem(&self, problem: String) {
		self.shell.announce(&problem);
		self.state.field_error.set(Some(problem));
		self.state.to_field.request();
	}

	/// Saves the draft, and says what became of it and of its event.
	fn save(&self) {
		let state = self.state;

		if state.saving.get_untracked() || self.full() {
			return;
		}

		if let Some(problem) = self.draft_problem() {
			self.field_problem(problem);
			return;
		}

		state.field_error.set(None);
		state.outcome.set(None);
		state.saving.set(true);
		let page = self.clone();
		let text = state.draft.get_untracked();
		crate::spawn(async move {
			let saved = leptoskit::rpc::client::call(ops::SaveNote { text }).await;
			state.saving.set(false);

			match saved {
				Ok(saved) => page.saved(saved),
				Err(FnError::Network(_)) => {
					page.problem(
						"Couldn't reach Notes",
						"Your note may not have saved. Check Saved notes before trying again; \
						 what you typed is still in the box.",
					);
					page.load().await;
				}
				Err(FnError::Refused { code, .. }) => page.refused(&code).await,
				Err(_) => {
					page.problem(
						"That didn't save",
						"Something went wrong on our side. What you typed is still in the box, so \
						 you can try again.",
					);
				}
			}
		});
	}

	/// A saved note: in the list, with what became of its event.
	fn saved(&self, saved: ops::Saved) {
		let state = self.state;
		let max = self.welcome.max_notes;
		state.fresh.set(Some(saved.note.id.clone()));
		state.notes.update(|notes| {
			notes.insert(0, saved.note);
			notes.truncate(max);
		});
		state.draft.set(String::new());

		if saved.analytics == "confirmed" {
			state.outcome.set(Some(Outcome::Saved {
				words: saved.words,
				actor: saved.actor,
			}));
			self.shell
				.announce("Note saved. Its note_created event reached analytics.");
		} else {
			state.outcome.set(Some(Outcome::Unconfirmed));
			self.shell
				.announce("Note saved. Analytics delivery could not be confirmed.");
		}
	}

	/// A save the server turned down: each keeps what was typed.
	async fn refused(&self, code: &str) {
		match code {
			ops::SIGNED_OUT | ops::NO_PASS | ops::NEW_DAY => self.gone(code),
			ops::NOTES_FULL => {
				self.problem(
					"Your notebook is full",
					&format!(
						"{} notes is the limit for a demo day, and everything resets at 00:00 \
						 UTC. What you typed is still in the box.",
						self.welcome.max_notes
					),
				);
				self.load().await;
			}
			ops::EMPTY_NOTE => self.field_problem("Write something first.".to_owned()),
			ops::NOTE_TOO_LONG => {
				self.field_problem(format!(
					"That's over the {} character limit.",
					self.welcome.max_chars
				));
			}
			_ => {
				self.problem(
					"That didn't save",
					"Something went wrong on our side. What you typed is still in the box, so you \
					 can try again.",
				);
			}
		}
	}

	/// Shows `message` for a while where nothing else can.
	fn toast(&self, message: &str) {
		let toast = self.state.toast;
		toast.set(Some(message.to_owned()));
		crate::spawn(async move {
			leptoskit::query::sleep(TOAST_FOR).await;
			toast.try_set(None);
		});
	}

	/// Signs out, then runs `then`. A failed sign-out stays here and says so.
	fn leave(&self, failed: &'static str, then: impl FnOnce(Self) + 'static) {
		let state = self.state;

		if state.leaving.get_untracked() {
			return;
		}

		state.leaving.set(true);
		let page = self.clone();
		crate::spawn(async move {
			if state.sign_out.mutate_async(()).await.is_ok() {
				then(page);
			} else {
				state.leaving.set(false);
				page.toast(failed);
			}
		});
	}

	fn sign_out(&self) {
		self.leave("Couldn't sign out. Try again in a moment.", |_| {
			crate::go("/?signed_out=1");
		});
	}

	/// Switching is signing out, then choosing the other person.
	fn switch(&self) {
		self.leave("Couldn't switch account. Try again in a moment.", |page| {
			let other = page.welcome.person.persona.other();
			crate::spawn(async move {
				match leptoskit::rpc::client::call(ops::Mint {
					persona: other,
					return_to: "/notes".to_owned(),
				})
				.await
				{
					Ok(minted) => crate::go(&minted.next),
					Err(_) => crate::go("/"),
				}
			});
		});
	}
}

/// Reads the notebook once the page is open. The server never puts the
/// notes in the page.
pub(crate) fn open(shell: &Shell, welcome: &Arc<Welcome>, state: &State) {
	let page = Page {
		shell: shell.clone(),
		welcome: Arc::clone(welcome),
		state: *state,
	};
	Effect::new(move |_| {
		let page = page.clone();
		crate::spawn(async move {
			page.load().await;
		});
	});
}

fn avatar_class(ada: bool) -> &'static str {
	if ada {
		"bg-[var(--admin-ember)] text-[#fffdf8] border border-[var(--admin-ember)]"
	} else {
		"bg-[var(--admin-sunken)] text-[var(--admin-ink)] border border-[var(--admin-line-strong)]"
	}
}

/// The words a link to the admin uses: Ada opens it, Grace tries it.
const fn admin_words(ada: bool) -> (&'static str, &'static str) {
	if ada {
		("Open the admin", "Open the admin ↗")
	} else {
		("Try the admin", "Try the admin ↗")
	}
}

/// The account menu: who is signed in, the admin, a switch, and sign out.
fn account_menu(page: &Page) -> Element {
	let state = page.state;
	let person = &page.welcome.person;
	let ada = person.is_ada();
	let other = person.persona.other();
	let initial = person
		.first_name()
		.chars()
		.next()
		.map(String::from)
		.unwrap_or_default();
	let (admin_label, admin_text) = admin_words(ada);
	let admin_url = page.welcome.context.admin_url.clone();
	let switch = {
		let page = page.clone();
		move || page.switch()
	};
	let sign_out = {
		let page = page.clone();
		move || page.sign_out()
	};
	let item = |text: &str, class: &str| {
		view! {
			<View class=format!("{class} w-full justify-start")>
				<Text>{text.to_owned()}</Text>
			</View>
		}
	};

	let menu = Menu::new("me", state.menu_open.get())
		.button(view! {
			<Button
				test_id="menu-toggle"
				aria_label=format!(
					"Signed in as {} #{}. Open the account menu.",
					person.first_name(),
					person.tag
				)
				class=format!(
					"h-[44px] w-[44px] items-center justify-center rounded-full text-[13px] \
					 font-bold cursor-pointer {}",
					avatar_class(ada)
				)
			>
				{initial}
			</Button>
		})
		.item(admin_label, move || crate::open_tab(&admin_url))
		.content(item(admin_text, chrome::BUTTON_QUIET))
		.item(format!("Switch to {}", other.first_name()), switch)
		.content(item(
			&format!("Switch to {}", other.first_name()),
			chrome::BUTTON_QUIET,
		))
		.item("Sign out of Notes", sign_out)
		.content(item("Sign out of Notes", chrome::BUTTON_SECONDARY))
		.placement(Placement::BelowEnd)
		.on_toggle(move |open| state.menu_open.set(open));

	// The menu names its parts by id; the page's styles and tests know them
	// by test id, and the list opens with who is signed in.
	let mut menu = Element::from(menu);
	if let Some(list) = menu.children.get_mut(1) {
		let mut panel = taken(list).test_id("menu-panel").class(
			"w-[300px] flex-col gap-1 rounded-xl border border-[var(--admin-line)] \
			 bg-[var(--admin-surface)] p-3",
		);
		for (each, id) in
			panel
				.children
				.iter_mut()
				.zip(["open-admin", "switch-account", "sign-out"])
		{
			*each = taken(each).test_id(id);
		}
		panel.children.insert(
			0,
			view! {
				<View
					class="flex-col gap-0.5 pb-2 mb-1 border-b border-[var(--admin-line)]"
					aria_hidden=true
				>
					<Text class=chrome::LABEL>"Signed in as"</Text>
					<View class="flex-row items-center gap-2">
						<Text class="font-bold text-[var(--admin-ink)]">{person.first_name()}</Text>
						<Text class="font-[var(--admin-font-mono)] text-xs text-[var(--admin-ink-3)]">
							{format!("#{}", person.tag)}
						</Text>
					</View>
					<Text class="font-[var(--admin-font-mono)] text-xs text-[var(--admin-ink-3)]">
						{person.email.clone()}
					</Text>
				</View>
			},
		);
		*list = panel;
	}

	view! { <View class="relative" test_id="menu-wrap">{menu}</View> }
}

/// The element in `slot`, leaving an empty view there until it is put back.
fn taken(slot: &mut Element) -> Element {
	std::mem::replace(slot, view! { <View /> })
}

/// A link to the admin, which opens beside this page so both stay in view.
fn admin_link(href: String, test_id: &str, label: &str, text: &str, class: String) -> Element {
	view! {
		<Pressable
			href=href
			target=LinkTarget::NewTab
			test_id=test_id.to_owned()
			aria_label=label.to_owned()
			class=class
		>
			<Text>{text.to_owned()}</Text>
		</Pressable>
	}
}

/// The admin hand-off card: the real next step.
fn admin_card(welcome: &Welcome) -> Element {
	let admin_url = welcome.context.admin_url.clone();

	let body = if welcome.person.is_ada() {
		view! {
			<View class="flex-col gap-3">
				<Text class=chrome::SECONDARY>
					"Ada has the admin role, so the admin will let her in. Open it in a new tab \
					 and keep this page beside it: Live shows each note you save as a real \
					 note_created event, and the rest is analytics, saved insights, and a \
					 simulated infrastructure."
				</Text>
				{chrome::raised(
					admin_link(
						admin_url,
						"open-admin",
						"Open the admin",
						"Open the admin ↗",
						format!("{} w-full", chrome::BUTTON_PRIMARY),
					),
					false,
				)}
			</View>
		}
	} else {
		view! {
			<View class="flex-col gap-3">
				{margin_note(
					"Grace has no admin role. Try the admin anyway: it'll turn her away and show \
					 you how that's decided.",
				)}
				{admin_link(
					admin_url,
					"open-admin",
					"Try the admin as Grace",
					"Try the admin as Grace ↗",
					chrome::BUTTON_SECONDARY.to_owned(),
				)}
				<Text class=chrome::MUTED>
					"To get in, switch to Ada from the account menu. You'll sign in once more with a \
					 new emailed code."
				</Text>
			</View>
		}
	};

	view! {
		<View
			class=format!("{} gap-3.5 p-5", CARD)
			role=Role::Region
			aria_labelled_by=["admin-title"]
			test_id="admin-card"
		>
			<Heading
				aria_level=2
				id="admin-title"
				class="text-base font-semibold text-[var(--admin-ink)]"
			>
				"The admin"
			</Heading>
			{body}
		</View>
	}
}

/// A short aside in the margin's hand.
fn margin_note(text: &str) -> Element {
	view! {
		<View class="flex-row items-start gap-3 rounded-[10px] border-l-[3px] border-[var(--admin-mint)] \
			       bg-[var(--admin-mint-wash)] px-3 py-2">
			<Text class="flex-1 font-[var(--admin-font-note)] text-[16px] leading-snug \
				       text-[var(--admin-ink-2)]">
				{text.to_owned()}
			</Text>
		</View>
	}
}

/// A save that went through. It carries no live role: the page announces
/// each result once through its own status region.
fn result(tone: Tone, test_id: &str, title: &str, body: &str, children: Vec<Element>) -> Element {
	let (border, glyph) = match tone {
		Tone::Good => ("border-[var(--admin-good)] text-[var(--admin-good)]", "✓"),
		_ => ("border-[var(--admin-warn)] text-[var(--admin-warn)]", "!"),
	};

	view! {
		<View
			class=format!(
				"flex-row items-start gap-2.5 rounded-[10px] border bg-[var(--admin-sunken)] px-3 \
				 py-2.5 {border}"
			)
			test_id=test_id.to_owned()
		>
			<Text class="w-4 shrink-0 text-center text-sm font-extrabold" aria_hidden=true>
				{glyph}
			</Text>
			<View class="min-w-0 flex-1 flex-col gap-2">
				<Text
					class="text-sm font-semibold leading-normal text-[var(--admin-ink)]"
					test_id="status-title"
				>
					{title.to_owned()}
				</Text>
				<Text class=chrome::SECONDARY test_id="status-body">{body.to_owned()}</Text>
				{children}
			</View>
		</View>
	}
}

/// What a confirmed save adds under its message: the person's analytics
/// actor, which is what to look for in the admin, and the way to Live.
fn saved_extras(welcome: &Welcome, actor: &str) -> Vec<Element> {
	let mut extras = vec![
		view! {
			<View class="flex-col gap-1">
				<Text class=chrome::LABEL>"Your actor in analytics"</Text>
				<Text
					class="font-[var(--admin-font-mono)] text-xs text-[var(--admin-ink)]"
					test_id="note-actor"
				>
					{actor.to_owned()}
				</Text>
				<Text class=chrome::MUTED>
					"Live lists events by name: filter for note_created, then look for the row whose \
					 actor starts like this one."
				</Text>
			</View>
		},
		admin_link(
			live_url(welcome),
			"open-admin-live",
			"Open Live in the admin",
			"Open Live in the admin ↗",
			format!("{} self-start", chrome::BUTTON_SECONDARY),
		),
	];

	if !welcome.person.is_ada() {
		extras.push(margin_note(
			"Grace has no admin role, so Live will turn her away. Switch to Ada from the account \
			 menu to watch this arrive.",
		));
	}

	extras
}

/// The admin's Live page, from configuration.
fn live_url(welcome: &Welcome) -> String {
	format!("{}analytics/live", welcome.context.admin_url)
}

/// What the last save came to, if anything.
fn outcome(page: &Page) -> Option<Element> {
	let welcome = &page.welcome;

	Some(match page.state.outcome.get()? {
		Outcome::Saved { words, actor } => {
			result(
				Tone::Good,
				"note-saved",
				"Saved. Analytics heard about it.",
				&format!(
					"Your note is private to this account. Its note_created event ({}, none of \
					 your text) reached analytics.",
					plural(usize::try_from(words).unwrap_or(usize::MAX), "word")
				),
				saved_extras(welcome, &actor),
			)
		}
		Outcome::Unconfirmed => {
			result(
				Tone::Warn,
				"note-unconfirmed",
				"Saved. Analytics didn't confirm.",
				"Your note is saved in your notebook. We couldn't confirm that its note_created \
				 event reached analytics, so it may not show in the admin. Nothing will resend it.",
				Vec::new(),
			)
		}
		Outcome::Problem {
			title,
			body,
			sign_in,
		} => {
			let way_back = sign_in.then(|| {
				view! {
					<Pressable
						href=SIGN_IN_AGAIN
						test_id="note-sign-in"
						aria_label="Sign in again"
						class="self-start text-sm font-semibold text-[var(--admin-ink)]"
					>
						<Text>"Sign in again"</Text>
					</Pressable>
				}
			});

			chrome::status(
				Tone::Bad,
				"note-alert",
				title,
				&body,
				way_back.into_iter().collect(),
			)
		}
	})
}

/// One saved note, as text.
fn note(each: &Note, fresh: bool) -> Element {
	let item = view! {
		<View
			role=Role::ListItem
			test_id="note"
			class="flex-col gap-1 rounded-[10px] border border-[var(--admin-line)] \
			       bg-[var(--admin-surface)] px-3 py-2.5"
		>
			<Text class="text-[15px] leading-relaxed text-[var(--admin-ink)]" test_id="note-body">
				{each.text.clone()}
			</Text>
			<Text class="font-[var(--admin-font-mono)] text-xs text-[var(--admin-ink-3)]">
				{format!("{} UTC", utc_time(each.created_at))}
			</Text>
		</View>
	};

	// The note this page just saved comes in with a little motion, which
	// `assets/demo.css` gives it by this id and stops for reduced motion.
	if fresh { item.id("note-new") } else { item }
}

/// What is saved, newest first, or why there is nothing to show.
fn saved_notes(state: State, notes: &[Note]) -> Element {
	let empty = match state.listing.get() {
		Listing::Loading => Some("Loading your notes…"),
		Listing::Failed => Some("Couldn't load your notes. Reload the page to try again."),
		Listing::SignedOut if notes.is_empty() => Some("Sign in again to see your notes."),
		Listing::Ready if notes.is_empty() => {
			Some("Nothing saved yet. Your first note will be here, and only you can read it.")
		}
		Listing::Ready | Listing::SignedOut => None,
	};
	let fresh = state.fresh.get();
	let items = notes
		.iter()
		.map(|each| note(each, fresh.as_deref() == Some(each.id.as_str())))
		.collect::<Vec<_>>();

	view! {
		<View class="flex-col gap-2 border-t border-[var(--admin-line)] pt-4">
			<Heading
				aria_level=3
				id="note-list-title"
				class="text-sm font-semibold text-[var(--admin-ink)]"
			>
				"Saved notes"
			</Heading>
			{empty.map(|text| {
				view! { <Text class=chrome::MUTED test_id="note-empty">{text}</Text> }
			})}
			<View
				role=Role::List
				aria_labelled_by=["note-list-title"]
				class="flex-col gap-2"
				test_id="note-list"
			>
				{items}
			</View>
		</View>
	}
}

/// The notebook: write a note, see what happened, and read what is saved.
fn notebook(page: &Page) -> Element {
	let state = page.state;
	let max_chars = page.welcome.max_chars;
	let max_notes = page.welcome.max_notes;
	let notes = state.notes.get();
	let draft = state.draft.get();
	let used = length(&draft);
	let over = used > max_chars;
	let full = notes.len() >= max_notes;
	let saving = state.saving.get();
	let invalid = state.field_error.with(Option::is_some);
	let error = state.field_error.get().unwrap_or_else(|| {
		if full {
			format!("Your notebook is full: {max_notes} notes is the limit for a demo day.")
		} else {
			String::new()
		}
	});
	let described = if error.is_empty() {
		vec!["note-help"]
	} else {
		vec!["note-help", "note-error"]
	};
	let border = if invalid {
		"border-2 border-[var(--admin-bad)]"
	} else {
		"border border-[var(--admin-line-strong)]"
	};
	let chars = if over {
		"text-[12.5px] tabular-nums font-bold text-[var(--admin-bad)]"
	} else {
		"text-[12.5px] tabular-nums text-[var(--admin-ink-3)]"
	};
	let change = move |value: &str| {
		state.draft.set(value.to_owned());
		state.field_error.set(None);
	};
	let keys = {
		let page = page.clone();
		move |key: &KeyPress| {
			if key.key == "Enter" && (key.ctrl || key.meta) {
				page.save();
				EventOutcome::Handled
			} else {
				EventOutcome::Unhandled
			}
		}
	};
	let save = {
		let page = page.clone();
		move || page.save()
	};

	view! {
		<View
			class=format!("{} gap-4 p-5", CARD)
			role=Role::Region
			aria_labelled_by=["notebook-title"]
			test_id="notebook"
		>
			<View class="flex-row flex-wrap items-center justify-between gap-x-3 gap-y-1">
				<Heading
					aria_level=2
					id="notebook-title"
					class="text-xl font-semibold text-[var(--admin-ink)]"
				>
					"Your notebook"
				</Heading>
				<Text class=chrome::MUTED test_id="notes-count">
					{format!("{} of {max_notes} notes", notes.len())}
				</Text>
			</View>
			<Text class=chrome::SECONDARY>
				"Only this account can read what you write here. Analytics, the admin, and the \
				 infrastructure are shared by everyone today, so a save sends one real event \
				 carrying a word count and never your text."
			</Text>
			<View class="flex-col gap-3" test_id="note-form">
				<View class="flex-col gap-1.5">
					<Text id="note-label" class=chrome::LABEL>"Write a note"</Text>
					<TextArea
						id="note-text"
						test_id="note-text"
						value=draft
						enabled=!full
						read_only=saving
						aria_invalid=invalid
						aria_labelled_by=["note-label"]
						aria_described_by=described
						focus_on=state.to_field
						on:change=change
						on:key=keys
						class=format!(
							"min-h-[120px] w-full rounded-lg bg-[var(--admin-sunken)] px-3 py-2.5 \
							 text-[15px] leading-relaxed text-[var(--admin-ink)] {border}"
						)
					/>
					<View class="flex-row flex-wrap items-center justify-between gap-x-3">
						<Text
							id="note-help"
							class="text-[12.5px] leading-normal text-[var(--admin-ink-3)]"
						>
							{format!("Up to {max_chars} characters. Ctrl or ⌘ + Enter saves.")}
						</Text>
						<Text class=chars test_id="note-chars">
							{format!("{used} / {max_chars}")}
						</Text>
					</View>
					<Text
						id="note-error"
						class="text-[13px] font-semibold leading-normal text-[var(--admin-bad)]"
						test_id="note-error"
					>
						{error}
					</Text>
				</View>
				{chrome::raised(
					view! {
						<Button
							test_id="note-save"
							class=chrome::BUTTON_PRIMARY
							enabled=!full
							on:press=save
						>
							{if saving { "Saving…" } else { "Save note" }}
						</Button>
					},
					false,
				)}
			</View>
			{outcome(page)}
			{saved_notes(state, &notes)}
		</View>
	}
}

/// How the visitor got here: the five steps of a real sign-in.
fn how_you_got_here(ada: bool) -> Element {
	let how = if ada {
		"Your emailed code came through a real sign-in, and the admin will check your ID token's \
		 roles for admin."
	} else {
		"Your emailed code came through a real sign-in. The admin will check your ID token's roles \
		 and find no admin."
	};
	let steps = ["Send", "Mail", "Read", "Verify", "Session"]
		.map(|step| {
			view! {
				<View
					role=Role::ListItem
					class="rounded-full border border-[#5ef2c2]/60 px-2.5 py-px"
				>
					<Text class="text-xs font-semibold text-[#5ef2c2]">{format!("✓ {step}")}</Text>
				</View>
			}
		})
		.to_vec();

	view! {
		<View
			class="flex-col gap-2 rounded-xl border border-[var(--admin-underside-line)] \
			       bg-[var(--admin-underside)] p-4"
			test_id="how-you-got-here"
		>
			<View class="flex-row items-center gap-2.5">
				{chrome::piece()}
				<Text class="text-[15px] font-semibold text-[#f4efe8]">"How you got here"</Text>
			</View>
			<Text class="text-[13.5px] leading-relaxed text-[#c9c0d0]">{how}</Text>
			<View
				class="flex-row flex-wrap gap-1.5 pt-1"
				role=Role::List
				aria_label="The five steps, all done"
			>
				{steps}
			</View>
		</View>
	}
}

/// A message the page could put nowhere else.
fn toast(page: &Page) -> Option<Element> {
	let message = page.state.toast.get()?;

	Some(view! {
		<View
			role=Role::Alert
			test_id="toast"
			class="rounded-[10px] border border-[var(--admin-bad)] bg-[var(--admin-surface)] \
			       px-3.5 py-3"
		>
			<Text class="text-sm text-[var(--admin-ink)]">{message}</Text>
		</View>
	})
}

/// The Notes page.
pub(crate) fn page(shell: &Shell, welcome: &Arc<Welcome>, state: &State) -> Element {
	let page = Page {
		shell: shell.clone(),
		welcome: Arc::clone(welcome),
		state: *state,
	};
	let person = &welcome.person;
	let ada = person.is_ada();
	let (admin_label, admin_text) = admin_words(ada);
	let right = vec![
		admin_link(
			welcome.context.admin_url.clone(),
			"open-admin-bar",
			admin_label,
			admin_text,
			format!("{} max-md:hidden", chrome::BUTTON_SECONDARY),
		),
		account_menu(&page),
	];

	view! {
		<View class=chrome::PAGE>
			{chrome::skip_link()} {chrome::band(shell)} {chrome::app_bar(right)}
			<View
				id="main"
				role=Role::Main
				tab_index=-1
				class=format!("{} flex-col gap-6 py-8 md:py-10", chrome::COLUMN)
			>
				<View class="grid grid-cols-1 items-start gap-6 md:grid-cols-[minmax(0,1fr)_340px]">
					<View class="min-w-0 flex-col gap-5">
						<View class="flex-col gap-1.5">
							<Heading
								aria_level=1
								id="notes-title"
								tab_index=-1
								class="text-3xl font-bold leading-tight text-[var(--admin-ink)]"
								test_id="welcome-title"
							>
								{format!("Welcome, {}.", person.first_name())}
							</Heading>
							<Text class=chrome::MUTED test_id="welcome-sub">
								{format!("#{} is yours until 00:00 UTC.", person.tag)}
							</Text>
						</View>
						{notebook(&page)}
						{how_you_got_here(ada)}
					</View>
					{admin_card(welcome)}
				</View>
			</View>
			{toast(&page)} {chrome::reset_dialog(shell)} {chrome::announcer(shell)}
		</View>
	}
}

/// What a visitor without scripts gets on Notes: the page reads and saves
/// notes through its client, and says so.
#[must_use]
pub fn noscript() -> String {
	"<noscript><p class=\"plain-mint\">Writing and reading notes needs scripts. The rest of this \
	 page works without them.</p></noscript>"
		.to_owned()
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::model::Persona;

	#[test]
	fn the_server_counts_characters_the_way_the_page_does() {
		assert_eq!(length("  héllo  "), 5);
		assert_eq!(length("👋🏽"), 2);
		assert_eq!(length("\n\t"), 0);
	}

	#[test]
	fn counts_are_worded_in_the_plural_when_they_should_be() {
		assert_eq!(plural(1, "word"), "1 word");
		assert_eq!(plural(0, "word"), "0 words");
		assert_eq!(plural(5, "character"), "5 characters");
	}

	#[test]
	fn the_words_for_the_admin_follow_the_role() {
		assert_eq!(admin_words(true).0, "Open the admin");
		assert_eq!(admin_words(false).0, "Try the admin");
		assert_eq!(Persona::Ada.other(), Persona::Grace);
	}
}
