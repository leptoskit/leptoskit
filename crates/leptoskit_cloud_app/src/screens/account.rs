//! `/account`: how you sign in, where you're signed in, and how the console
//! looks (`docs/agents/cloud-design.md` §14.5).

use leptoskit::appearance::Appearance;
use leptoskit::auth::Device;
use leptoskit::auth::IdentityInfo;
use leptoskit::auth::PasskeyInfo;
use leptoskit::auth::ProviderInfo;
use leptoskit::auth::StartProvider;
use leptoskit::native::Role;
use leptoskit::native::Tristate;
use leptoskit::prelude::*;
use leptoskit::query::MutateCallbacks;

use crate::components::Kind;
use crate::components::Tone;
use crate::components::focus_ring;
use crate::components::plain_button;
use crate::components::problem;
use crate::components::status_word;
use crate::console::Console;
use crate::console::Load;
use crate::console::auth_problem;
use crate::routes::Navigation;
use crate::style;

/// A date as the console writes it: `18 Nov 2026`, in UTC, so the server's
/// page and the browser's agree.
#[must_use]
pub fn date(at: &leptoskit::db::Timestamp) -> String {
	at.format("%-d %b %Y").to_string()
}

/// Everything the account page shows.
#[derive(Clone, Debug, PartialEq)]
pub struct AccountPage {
	/// The address the account signs in with.
	pub email: Option<String>,
	/// The providers the server set up.
	pub providers: Vec<ProviderInfo>,
	/// The providers connected to the account.
	pub identities: Load<Vec<IdentityInfo>>,
	/// Its passkeys.
	pub passkeys: Load<Vec<PasskeyInfo>>,
	/// Where it's signed in.
	pub devices: Load<Vec<Device>>,
	/// Whether this browser can add a passkey.
	pub can_add_passkey: bool,
	/// The console's appearance.
	pub appearance: Appearance,
	/// The last action's error.
	pub problem: Option<String>,
	/// Whether signing out is running.
	pub signing_out: bool,
}

impl AccountPage {
	/// The page the console's state calls for.
	#[must_use]
	pub fn read(console: &Console) -> Self {
		let account = console.account;
		let email = match console.me() {
			Load::Ready(me) => me.email,
			_ => None,
		};

		Self {
			email,
			providers: console.providers.data().unwrap_or_default(),
			identities: Load::of(&account.identities, auth_problem),
			passkeys: Load::of(&account.passkeys, auth_problem),
			devices: Load::of(&account.devices, auth_problem),
			can_add_passkey: console.mounted.get() && cfg!(target_arch = "wasm32"),
			appearance: leptoskit::appearance::get(),
			problem: account
				.add_passkey
				.error()
				.or_else(|| account.remove_passkey.error())
				.or_else(|| account.revoke_device.error())
				.or_else(|| account.link_provider.error())
				.or_else(|| account.sign_out.error())
				.map(|error| auth_problem(&error)),
			signing_out: account.sign_out.is_pending(),
		}
	}
}

/// `/account`.
pub fn page(navigation: &Navigation) -> Element {
	let console = Console::get();
	view(&console, navigation, &AccountPage::read(&console))
}

/// The page.
pub fn view(console: &Console, navigation: &Navigation, page: &AccountPage) -> Element {
	let console = *console;
	let account = console.account;
	let sign_out = {
		let navigation = navigation.clone();
		move || {
			let navigation = navigation.clone();
			account.sign_out.mutate_with(
				(),
				MutateCallbacks::new().on_success(move |_| leave(&navigation)),
			);
		}
	};

	View()
		.class("flex-col gap-6 w-full max-w-[720px]")
		.test_id("account")
		.child(Heading(1, "Account").class(style::PAGE_TITLE))
		.children(page.problem.as_ref().map(|text| problem(text.clone())))
		.child(card(
			"You",
			[Text(
				page.email
					.clone()
					.unwrap_or_else(|| "No email address on this account.".to_owned()),
			)
			.class(style::BODY)],
		))
		.child(card("Sign-in methods", sign_in_methods(&console, page)))
		.child(card("Signed in on", devices(&console, &page.devices)))
		.child(card(
			"Appearance",
			[
				appearance(&console, page.appearance),
				// leptoskit doesn't keep the choice yet (cloud-design.md §19
				// gap 14), so the card doesn't promise it will.
				Text("Lasts until the page reloads.").class(style::MUTED),
			],
		))
		.child(focus_ring(
			console.focus,
			"account-sign-out",
			plain_button(
				Kind::Danger,
				"Sign out",
				page.signing_out.then_some("Signing out…"),
				sign_out,
			),
		))
}

/// After signing out: a fresh page in the browser, so nothing one account
/// loaded is still in memory for the next, or the sign-in sheet elsewhere.
fn leave(navigation: &Navigation) {
	#[cfg(all(target_arch = "wasm32", feature = "hydrate"))]
	{
		let _ = navigation;

		if let Some(window) = web_sys::window() {
			let _ = window.location().assign("/sign-in");
		}
	}

	#[cfg(not(all(target_arch = "wasm32", feature = "hydrate")))]
	if let Some(route) = crate::console::route_of("/sign-in") {
		navigation.navigate(route);
	}
}

fn card(title: &str, body: impl IntoIterator<Item = Element>) -> Element {
	View()
		.class(style::CARD)
		.role(Role::Region)
		.accessibility_label(title)
		.child(Heading(2, title).class(style::CARD_TITLE))
		.children(body)
}

fn sign_in_methods(console: &Console, page: &AccountPage) -> Vec<Element> {
	let console = *console;
	let account = console.account;
	let mut rows = vec![method_row(
		"Email codes",
		page.email.clone(),
		Some(status_word(Tone::Good, "On")),
	)];

	match &page.identities {
		Load::Loading => rows.push(super::loading("Loading sign-in methods", &["h-11 w-full"])),
		Load::Failed(error) => {
			let identities = account.identities;
			rows.push(super::load_failed(
				"your sign-in methods",
				error,
				move || {
					drop(identities.refetch());
				},
			));
		}
		Load::Ready(identities) => {
			for provider in &page.providers {
				let linked = identities
					.iter()
					.find(|identity| identity.provider == provider.id);
				let action = if linked.is_some() {
					status_word(Tone::Good, "Linked")
				} else {
					let id = provider.id.clone();
					focus_ring(
						console.focus,
						&format!("account-link-{}", provider.id),
						plain_button(
							Kind::Secondary,
							format!("Link {}", provider.name),
							account.link_provider.is_pending().then_some("Opening…"),
							move || {
								account.link_provider.mutate(StartProvider {
									provider: id.clone(),
									redirect_to: Some("/account".to_owned()),
									prompt: None,
								});
							},
						),
					)
				};
				let detail = linked
					.and_then(|identity| identity.email.clone().or_else(|| identity.name.clone()));
				rows.push(method_row(&provider.name, detail, Some(action)));
			}
		}
	}

	rows.push(View().class(style::RULE_ACROSS).aria_hidden(true));
	rows.push(Heading(3, "Passkeys").class("text-sm font-semibold"));

	match &page.passkeys {
		Load::Loading => rows.push(super::loading("Loading passkeys", &["h-11 w-full"])),
		Load::Failed(error) => {
			let passkeys = account.passkeys;
			rows.push(super::load_failed("your passkeys", error, move || {
				drop(passkeys.refetch());
			}));
		}
		Load::Ready(passkeys) if passkeys.is_empty() => {
			rows.push(
				Text(
					"No passkeys yet. A passkey signs you in with your fingerprint, face or \
					 screen lock.",
				)
				.class(style::SECONDARY),
			);
		}
		Load::Ready(passkeys) => {
			rows.push(
				View()
					.class("flex-col gap-2")
					.role(Role::List)
					.accessibility_label("Passkeys")
					.children(passkeys.iter().map(|passkey| {
						let id = passkey.id;
						let used = passkey.last_used_at.as_ref().map_or_else(
							|| "never used".to_owned(),
							|at| format!("last used {}", date(at)),
						);
						View()
							.class("flex-row flex-wrap items-center gap-3")
							.role(Role::ListItem)
							.child(
								View()
									.class("flex-1 flex-col")
									.child(Text(passkey.name.clone()).class(style::BODY))
									.child(
										Text(format!(
											"Added {}, {used}",
											date(&passkey.created_at)
										))
										.class(style::MUTED),
									),
							)
							.child(focus_ring(
								console.focus,
								&format!("account-passkey-{id}"),
								plain_button(
									Kind::DangerQuiet,
									format!("Remove {}", passkey.name),
									None,
									move || account.remove_passkey.mutate(id),
								),
							))
					})),
			);
		}
	}

	if page.can_add_passkey {
		rows.push(focus_ring(
			console.focus,
			"account-add-passkey",
			plain_button(
				Kind::Secondary,
				"Add a passkey",
				account
					.add_passkey
					.is_pending()
					.then_some("Waiting for your passkey…"),
				move || account.add_passkey.mutate(None),
			),
		));
	}

	rows
}

fn method_row(name: &str, detail: Option<String>, action: Option<Element>) -> Element {
	View()
		.class("flex-row flex-wrap items-center gap-3 min-h-11")
		.child(
			View()
				.class("flex-1 flex-col")
				.child(Text(name).class(style::BODY).class("font-semibold"))
				.children(detail.map(|detail| Text(detail).class(style::MUTED))),
		)
		.children(action)
}

fn devices(console: &Console, devices: &Load<Vec<Device>>) -> Vec<Element> {
	let console = *console;
	let account = console.account;

	match devices {
		Load::Loading => {
			vec![super::loading(
				"Loading where you're signed in",
				&["h-11 w-full", "h-11 w-full"],
			)]
		}
		Load::Failed(error) => {
			let devices = account.devices;
			vec![super::load_failed(
				"where you're signed in",
				error,
				move || {
					drop(devices.refetch());
				},
			)]
		}
		Load::Ready(devices) => {
			vec![
				View()
					.class("flex-col gap-2")
					.role(Role::List)
					.accessibility_label("Where you're signed in")
					.children(devices.iter().map(|device| {
						let session = device.session.id;
						let action = if device.current {
							status_word(Tone::Info, "This device")
						} else {
							focus_ring(
								console.focus,
								&format!("account-device-{session}"),
								plain_button(
									Kind::DangerQuiet,
									format!("Sign out {}", device.session.device),
									None,
									move || account.revoke_device.mutate(session),
								),
							)
						};

						View()
							.class("flex-row flex-wrap items-center gap-3 min-h-11")
							.role(Role::ListItem)
							.child(
								View()
									.class("flex-1 flex-col")
									.child(Text(device.session.device.clone()).class(style::BODY))
									.child(
										Text(format!(
											"Used {}",
											date(&device.session.last_seen_at)
										))
										.class(style::MUTED),
									),
							)
							.child(action)
					})),
			]
		}
	}
}

fn appearance(console: &Console, chosen: Appearance) -> Element {
	let console = *console;
	let choices = [
		(Appearance::System, "System", "account-appearance-system"),
		(Appearance::Light, "Light", "account-appearance-light"),
		(Appearance::Dark, "Dark", "account-appearance-dark"),
	];

	View()
		.class("flex-row flex-wrap gap-2")
		.role(Role::RadioGroup)
		.accessibility_label("Appearance")
		.children(choices.into_iter().map(|(appearance, label, id)| {
			let selected = appearance == chosen;
			let choice = Pressable(label)
				.role(Role::Radio)
				.aria_checked(if selected {
					Tristate::True
				} else {
					Tristate::False
				})
				.class(if selected {
					"min-h-11 flex-row items-center gap-2 rounded-lg border-2 \
					 border-[var(--cloud-ink)] bg-[var(--cloud-surface)] px-4 text-sm font-semibold"
				} else {
					"min-h-11 flex-row items-center gap-2 rounded-lg border \
					 border-[var(--cloud-control-line)] bg-[var(--cloud-surface)] px-4 text-sm \
					 text-[var(--cloud-ink-2)]"
				})
				.on_press(move || leptoskit::appearance::set(appearance))
				.children(selected.then(|| Text("✓").aria_hidden(true)))
				.child(Text(label));

			focus_ring(console.focus, id, choice)
		}))
}
