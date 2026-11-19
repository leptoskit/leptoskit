//! Notes, signed in: the person's private notebook, and the door to the
//! real admin.
//!
//! The notebook's shell is drawn here as `view!` markup; `assets/demo.js`
//! fills it from `GET /demo/notes` and saves through `POST /demo/notes`,
//! putting every note in with `textContent`. The web renderer has no
//! multi-line field, so the script swaps the page's `TextInput` for a
//! `<textarea>` that carries every attribute the markup gave it. The
//! pretend checkout is a later slice and is not drawn.

use leptoskit::native::HasPopup;
use leptoskit::native::Role;
use leptoskit::prelude::*;
use serde_json::json;

use super::Context;
use super::Person;
use super::chrome;
use super::chrome::Tone;
use crate::notes::MAX_CHARS;
use crate::notes::MAX_NOTES;

/// What the welcome is rendered from: the account the request is really
/// signed in as, and the person this browser's pass minted for it.
#[derive(Clone, Debug)]
pub(crate) struct Welcome {
	pub context: Context,
	pub person: Person,
}

fn avatar_class(ada: bool) -> &'static str {
	if ada {
		"bg-[var(--admin-ember)] text-[#fffdf8] border border-[var(--admin-ember)]"
	} else {
		"bg-[var(--admin-sunken)] text-[var(--admin-ink)] border border-[var(--admin-line-strong)]"
	}
}

/// The account menu: who is signed in, the admin, a switch, and sign out.
fn account_menu(welcome: &Welcome) -> Element {
	let person = &welcome.person;
	let other = if person.is_ada() { "Grace" } else { "Ada" };
	let initial = person
		.first_name()
		.chars()
		.next()
		.map(String::from)
		.unwrap_or_default();

	view! {
		<View class="relative" test_id="menu-wrap">
			<Button
				id="me"
				test_id="menu-toggle"
				aria_has_popup=HasPopup::Menu
				aria_expanded=false
				aria_controls=["me-menu"]
				aria_label=format!(
					"Signed in as {}. Open the account menu.",
					person.first_name()
				)
				class=format!(
					"h-[44px] w-[44px] items-center justify-center rounded-full text-[13px] \
					 font-bold cursor-pointer {}",
					avatar_class(person.is_ada())
				)
			>
				{initial}
			</Button>
			<View
				id="me-menu"
				test_id="menu-panel"
				role=Role::Menu
				aria_labelled_by=["me"]
				class="absolute right-0 top-12 w-[300px] flex-col gap-2 \
				       rounded-xl border border-[var(--admin-line)] bg-[var(--admin-surface)] p-3"
			>
				<Text class=chrome::LABEL>"Signed in as"</Text>
				<View class="flex-row items-center gap-2">
					<Text class="font-bold text-[var(--admin-ink)]">{person.first_name()}</Text>
					<Text class="font-[var(--admin-font-mono)] text-xs text-[var(--admin-ink-3)]">
						{format!("#{}", person.tag)}
					</Text>
				</View>
				<Text class="font-[var(--admin-font-mono)] text-xs text-[var(--admin-ink-3)] ">
					{person.email.clone()}
				</Text>
				<View class="my-1 border-t border-[var(--admin-line)]" aria_hidden=true />
				<Pressable
					href=welcome.context.admin_url.clone()
					role=Role::MenuItem
					test_id="open-admin"
					aria_label=if person.is_ada() {
						"Open the admin"
					} else {
						"Try the admin"
					}
					class=format!("{} w-full justify-start", chrome::BUTTON_QUIET)
				>
					<Text>
						{if person.is_ada() {
							"Open the admin ↗"
						} else {
							"Try the admin ↗"
						}}
					</Text>
				</Pressable>
				<Button
					role=Role::MenuItem
					test_id="switch-account"
					class=format!("{} w-full justify-start", chrome::BUTTON_QUIET)
				>
					{format!("Switch to {other}")}
				</Button>
				<Button
					role=Role::MenuItem
					test_id="sign-out"
					class=format!("{} w-full", chrome::BUTTON_SECONDARY)
				>
					"Sign out of Notes"
				</Button>
			</View>
		</View>
	}
}

/// The admin hand-off card: the real next step.
fn admin_card(welcome: &Welcome) -> Element {
	let person = &welcome.person;
	let link_class = |primary: bool| {
		if primary {
			format!("{} w-full", chrome::BUTTON_PRIMARY)
		} else {
			chrome::BUTTON_SECONDARY.to_owned()
		}
	};

	let body = if person.is_ada() {
		view! {
			<View class="flex-col gap-3">
				<Text class=chrome::SECONDARY>
					"Ada has the admin role, so the admin will let her in. Open it in a new tab \
					 and keep this page beside it: Live shows each note you save as a real \
					 note_created event, and the rest is analytics, saved insights, and a \
					 simulated infrastructure."
				</Text>
				{chrome::raised(
					view! {
						<Pressable
							href=welcome.context.admin_url.clone()
							test_id="open-admin"
							aria_label="Open the admin"
							class=link_class(true)
						>
							<Text>"Open the admin ↗"</Text>
						</Pressable>
					},
					false,
				)}
			</View>
		}
	} else {
		view! {
			<View class="flex-col gap-3">
				{chrome::margin_note(
					"Grace has no admin role. Try the admin anyway: it'll turn her away and show \
					 you how that's decided.",
				)}
				<Pressable
					href=welcome.context.admin_url.clone()
					test_id="open-admin"
					aria_label="Try the admin as Grace"
					class=link_class(false)
				>
					<Text>"Try the admin as Grace ↗"</Text>
				</Pressable>
				<Text class=chrome::MUTED>
					"To get in, switch to Ada from the account menu. You'll sign in once more with a \
					 new emailed code."
				</Text>
			</View>
		}
	};

	view! {
		<View
			class=format!("{} gap-3.5 p-5", chrome::CARD)
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

/// A result the notebook shows after a save, hidden until the script
/// marks it `data-shown`. It carries no live role: the script announces
/// each result once through the page's own status region.
fn result(tone: Tone, test_id: &str, children: Vec<Element>) -> Element {
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
				/>
				<Text class=chrome::SECONDARY test_id="status-body" />
				{children}
			</View>
		</View>
	}
}

/// The admin's Live page, from configuration.
fn live_url(welcome: &Welcome) -> String {
	format!("{}analytics/live", welcome.context.admin_url)
}

/// What a confirmed save adds under its message: the person's analytics
/// actor, which is what to look for in the admin, and the way to Live.
fn saved_extras(welcome: &Welcome) -> Vec<Element> {
	let mut extras = vec![
		view! {
			<View class="flex-col gap-1">
				<Text class=chrome::LABEL>"Your actor in analytics"</Text>
				<Text
					class="font-[var(--admin-font-mono)] text-xs text-[var(--admin-ink)]"
					test_id="note-actor"
				/>
				<Text class=chrome::MUTED>
					"Live lists events by name: filter for note_created, then look for the row whose \
					 actor starts like this one."
				</Text>
			</View>
		},
		view! {
			<Pressable
				href=live_url(welcome)
				test_id="open-admin-live"
				aria_label="Open Live in the admin"
				class=format!("{} self-start", chrome::BUTTON_SECONDARY)
			>
				<Text>"Open Live in the admin ↗"</Text>
			</Pressable>
		},
	];

	if !welcome.person.is_ada() {
		extras.push(chrome::margin_note(
			"Grace has no admin role, so Live will turn her away. Switch to Ada from the account \
			 menu to watch this arrive.",
		));
	}

	extras
}

/// The notebook: write a note, see what happened, and read what is saved.
fn notebook(welcome: &Welcome) -> Element {
	view! {
		<View
			class=format!("{} gap-4 p-5", chrome::CARD)
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
					{format!("0 of {MAX_NOTES} notes")}
				</Text>
			</View>
			<Text class=chrome::SECONDARY>
				"Only this account can read what you write here. Analytics, the admin, and the \
				 infrastructure are shared by everyone today, so a save sends one real event \
				 carrying a word count and never your text."
			</Text>
			<Text class=chrome::MUTED test_id="note-needs-js">
				"Writing notes needs scripts. The rest of this page works without them."
			</Text>
			<View class="flex-col gap-3" test_id="note-form">
				<View class="flex-col gap-1.5">
					<Text id="note-label" class=chrome::LABEL>"Write a note"</Text>
					<TextArea
						id="note-text"
						test_id="note-text"
						aria_labelled_by=["note-label"]
						aria_described_by=["note-help"]
						class="min-h-[120px] w-full rounded-lg border border-[var(--admin-line-strong)] \
						       bg-[var(--admin-sunken)] px-3 py-2.5 text-[15px] leading-relaxed \
						       text-[var(--admin-ink)]"
					/>
					<View class="flex-row flex-wrap items-center justify-between gap-x-3">
						<Text
							id="note-help"
							class="text-[12.5px] leading-normal text-[var(--admin-ink-3)]"
						>
							{format!("Up to {MAX_CHARS} characters. Ctrl or ⌘ + Enter saves.")}
						</Text>
						<Text
							class="text-[12.5px] tabular-nums text-[var(--admin-ink-3)]"
							test_id="note-chars"
						>
							{format!("0 / {MAX_CHARS}")}
						</Text>
					</View>
					<Text
						id="note-error"
						class="text-[13px] font-semibold leading-normal text-[var(--admin-bad)]"
						test_id="note-error"
					/>
				</View>
				{chrome::raised(
					view! {
						<Button test_id="note-save" class=chrome::BUTTON_PRIMARY>
							"Save note"
						</Button>
					},
					false,
				)}
			</View>
			{chrome::status(Tone::Bad, "note-alert", "", "", Vec::new())}
			{result(Tone::Good, "note-saved", saved_extras(welcome))}
			{result(Tone::Warn, "note-unconfirmed", Vec::new())}
			<View class="flex-col gap-2 border-t border-[var(--admin-line)] pt-4">
				<Heading
					aria_level=3
					id="note-list-title"
					class="text-sm font-semibold text-[var(--admin-ink)]"
				>
					"Saved notes"
				</Heading>
				<Text class=chrome::MUTED test_id="note-empty">"Loading your notes…"</Text>
				<View
					role=Role::List
					aria_labelled_by=["note-list-title"]
					class="flex-col gap-2"
					test_id="note-list"
				/>
			</View>
		</View>
	}
}

fn body(welcome: &Welcome) -> Element {
	let person = &welcome.person;
	let ada = person.is_ada();
	let how = if ada {
		"Your emailed code came through a real sign-in, and the admin will check your ID token's \
		 roles for admin."
	} else {
		"Your emailed code came through a real sign-in. The admin will check your ID token's roles \
		 and find no admin."
	};

	view! {
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
					{notebook(welcome)}
					<View
						class="flex-col gap-2 rounded-xl border border-[var(--admin-underside-line)] \
						       bg-[var(--admin-underside)] p-4"
						test_id="how-you-got-here"
					>
						<View class="flex-row items-center gap-2.5">
							{chrome::piece()}
							<Text class="text-[15px] font-semibold text-[#f4efe8]">
								"How you got here"
							</Text>
						</View>
						<Text class="text-[13.5px] leading-relaxed text-[#c9c0d0]">{how}</Text>
						<View
							class="flex-row flex-wrap gap-1.5 pt-1"
							role=Role::List
							aria_label="The five steps, all done"
						>
							{["Send", "Mail", "Read", "Verify", "Session"]
								.map(|step| {
									view! {
										<View
											role=Role::ListItem
											class="rounded-full border border-[#5ef2c2]/60 px-2.5 py-px"
										>
											<Text class="text-xs font-semibold text-[#5ef2c2]">
												{format!("✓ {step}")}
											</Text>
										</View>
									}
								})
								.to_vec()}
						</View>
					</View>
				</View>
				{admin_card(welcome)}
			</View>
		</View>
	}
}

/// Notes' welcome as a document.
pub(super) fn page(welcome: &Welcome) -> String {
	let person = &welcome.person;
	let right = vec![
		view! {
			<Pressable
				href=welcome.context.admin_url.clone()
				test_id="open-admin-bar"
				aria_label=if person.is_ada() {
					"Open the admin"
				} else {
					"Try the admin"
				}
				class=format!("{} max-md:hidden", chrome::BUTTON_SECONDARY)
			>
				<Text>
					{if person.is_ada() {
						"Open the admin ↗"
					} else {
						"Try the admin ↗"
					}}
				</Text>
			</Pressable>
		},
		account_menu(welcome),
	];

	let tree = view! {
		<View class=chrome::PAGE>
			{chrome::skip_link()} {chrome::band(&welcome.context)} {chrome::app_bar(right)}
			{body(welcome)} {chrome::reset_dialog()}
		</View>
	};

	let mut data = super::base_state("notes", &welcome.context, "/notes");
	data["identity"] = json!(welcome.person);
	data["notebook"] = json!({ "maxChars": MAX_CHARS, "maxNotes": MAX_NOTES });

	super::document(
		&format!("{}'s notes · Notes", person.first_name()),
		" data-page=\"notes\"",
		&data,
		tree,
		"",
	)
}
