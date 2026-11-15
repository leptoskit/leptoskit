//! Notes, signed in: a welcome, and the door to the real admin.
//!
//! Writing notes, the pretend checkout, and the event trail are the next
//! slice, so this page says so and does not draw controls for them.

use leptoskit::native::HasPopup;
use leptoskit::native::Role;
use leptoskit::prelude::*;
use serde_json::json;

use super::Context;
use super::Person;
use super::chrome;

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
					"Ada has the admin role, so the admin will let her in. Open it in a new tab, \
					 keep this page beside it, and look around: analytics, saved insights, and a \
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
					{chrome::margin_note(
						"Writing notes comes in the next slice. Until then, this page is the door \
						 to the real admin.",
					)}
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

	super::document(
		&format!("{}'s notes · Notes", person.first_name()),
		" data-page=\"notes\"",
		&data,
		tree,
		"",
	)
}
