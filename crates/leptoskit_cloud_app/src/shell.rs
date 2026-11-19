//! The two frames every page sits in (`docs/agents/cloud-design.md` §3).
//!
//! The desk holds the sign-in, welcome and device sheets: paper and nothing
//! else. The console is the signed-in frame: a top bar, a rail of projects
//! on tablets and desktops, and a tab bar on phones. Both are one tree whose
//! classes change at the breakpoints, so phones and desktops get the same
//! components in a different arrangement.

use leptoskit::native::Current;
use leptoskit::native::Role;
use leptoskit::prelude::*;
use leptoskit_cloud::account::OrganizationSummary;
use leptoskit_cloud::account::WhoAmI;
use leptoskit_cloud::projects::ProjectSummary;
use leptoskit_router::Routable as _;

use crate::components::CostMeter;
use crate::components::ICON_ACCOUNT;
use crate::components::ICON_APPS;
use crate::components::ICON_BILLING;
use crate::components::ICON_PLUS;
use crate::components::ICON_SEARCH;
use crate::components::ICON_SWITCH;
use crate::components::OfflineChip;
use crate::components::ShippingChip;
use crate::components::focus_ring;
use crate::components::icon;
use crate::components::mark;
use crate::console::Console;
use crate::console::Load;
use crate::console::Who;
use crate::console::first_organization;
use crate::routes::Navigation;
use crate::routes::Route;
use crate::screens;
use crate::style;

/// The desk: a sheet centred on paper, scrolling when it's taller than the
/// window.
pub fn desk(children: Children) -> Element {
	ScrollView()
		.class("h-screen w-full bg-[var(--cloud-paper)]")
		.child(
			View()
				.class(style::DESK)
				.role(Role::Main)
				.children(children),
		)
}

/// The organisation the console is showing: the one in the address, or the
/// person's own on pages that belong to no organisation.
#[must_use]
pub fn current_organization<'a>(route: &Route, me: &'a WhoAmI) -> Option<&'a OrganizationSummary> {
	match route {
		Route::Org(crate::routes::Org { org })
		| Route::OrgBilling(crate::routes::OrgBilling { org }) => {
			me.organizations
				.iter()
				.find(|organization| &organization.slug == org)
		}
		_ => first_organization(me),
	}
}

/// What the shell shows around a page, worked out from the console's state.
#[derive(Clone, Debug, PartialEq)]
pub enum Frame {
	/// The session hasn't loaded.
	Checking,
	/// No one is signed in.
	SignedOut {
		/// The page, to come back to.
		here: String,
	},
	/// The session couldn't be read.
	SessionFailed(String),
	/// Signed in.
	SignedIn(Box<Chrome>),
}

/// The signed-in shell's contents.
#[derive(Clone, Debug, PartialEq)]
pub struct Chrome {
	/// The person, once loaded.
	pub me: Load<WhoAmI>,
	/// The organisation on screen, by slug, once known.
	pub organization: Option<OrganizationSummary>,
	/// Its projects, for the rail.
	pub projects: Load<Vec<ProjectSummary>>,
	/// The route, for what's current.
	pub route: Route,
	/// The rail's filter.
	pub filter: String,
}

impl Frame {
	/// The frame the console's state calls for on `route`.
	#[must_use]
	pub fn of(console: &Console, route: &Route) -> Self {
		match console.who() {
			Who::Checking => Self::Checking,
			Who::SignedOut => {
				Self::SignedOut {
					here: route.href().to_string(),
				}
			}
			Who::Unknown(error) => Self::SessionFailed(error),
			Who::SignedIn(_) => {
				let me = console.me();
				let organization = match &me {
					Load::Ready(me) => current_organization(route, me).cloned(),
					_ => None,
				};
				Self::SignedIn(Box::new(Chrome {
					me,
					organization,
					projects: Load::of(&console.projects, crate::console::rpc_problem),
					route: route.clone(),
					filter: console.filter.get(),
				}))
			}
		}
	}
}

/// The console around `children`.
pub fn console(children: Children, route: &Route, navigation: &Navigation) -> Element {
	let console = Console::get();
	let frame = Frame::of(&console, route);

	if let Frame::SignedOut { here } = &frame {
		screens::redirect(navigation, &screens::sign_in_href(here));
	}

	build(&console, frame, children)
}

/// The shell for `frame`, holding `children` when someone is signed in.
pub fn build(console: &Console, frame: Frame, children: Children) -> Element {
	match frame {
		Frame::Checking => {
			desk(Fragment::new().with(screens::loading(
				"Loading your session",
				&["h-64 w-full max-w-md"],
			)))
		}
		Frame::SignedOut { here } => desk(Fragment::new().with(screens::signed_out(&here))),
		Frame::SessionFailed(error) => {
			desk(Fragment::new().with(screens::session_failed(*console, &error)))
		}
		Frame::SignedIn(chrome) => signed_in(console, &chrome, children),
	}
}

fn signed_in(console: &Console, chrome: &Chrome, children: Children) -> Element {
	let console = *console;
	let turned = console.turned;
	let find = console.find;
	let typing = console.typing;

	View()
		.class(style::SHELL)
		.test_id("console")
		.on_key(move |key| {
			// While someone types in a field, every key is the field's.
			if typing.get_untracked() {
				return EventOutcome::Unhandled;
			}

			if key.is("/", false, false, false) {
				find.set(true);
				return EventOutcome::Handled;
			}

			if key.is("w", false, false, false) || key.is("W", false, false, false) {
				return if turned.turn_focused() {
					EventOutcome::Handled
				} else {
					EventOutcome::Unhandled
				};
			}

			if key.is("W", false, false, true) || key.is("w", false, false, true) {
				turned.turn_all();
				return EventOutcome::Handled;
			}

			if key.is("Escape", false, false, false) && turned.turn_back_all() {
				return EventOutcome::Handled;
			}

			EventOutcome::Unhandled
		})
		.child(top_bar(&console, chrome))
		.child(View().class(style::RULE_ACROSS).aria_hidden(true))
		.child(
			View()
				.class("flex-1 flex-row min-h-0")
				.child(rail(&console, chrome))
				.child(
					View()
						.class("hidden sm:flex")
						.class(style::RULE_DOWN)
						.aria_hidden(true),
				)
				.child(
					ScrollView().class("flex-1").child(
						View()
							.class(style::CONTENT)
							.role(Role::Main)
							.children(children),
					),
				),
		)
		.child(
			View()
				.class("flex sm:hidden")
				.class(style::RULE_ACROSS)
				.aria_hidden(true),
		)
		.child(tab_bar(chrome))
}

fn top_bar(console: &Console, chrome: &Chrome) -> Element {
	let services = console.services();
	let organization = chrome.organization.as_ref();
	let home = organization.map_or_else(
		|| "/organizations".to_owned(),
		|org| format!("/{}", org.slug),
	);
	let shipping = organization.and_then(|org| {
		ShippingChip {
			in_flight: services.deployments.in_flight(&org.slug),
		}
		.build()
	});
	let cost = organization.map(|org| {
		CostMeter {
			plan: org.plan.clone(),
			spend: services.billing.spend(&org.slug),
			href: None,
		}
		.build()
	});
	let offline = OfflineChip {
		connection: console.connection.get(),
	}
	.build();
	let crumb = organization.map(|org| {
		Pressable(format!("{}: projects", org.name))
			.href(home.clone())
			.class("min-h-11 flex-row items-center cursor-pointer")
			.child(Text(org.name.clone()).class("text-sm font-semibold truncate"))
	});
	let email = match &chrome.me {
		Load::Ready(me) => me.email.clone(),
		_ => None,
	};

	View()
		.class(style::TOP_BAR)
		.role(Role::Banner)
		.child(
			Pressable("LeptosKit Cloud: your projects")
				.href(home)
				.class(style::TARGET)
				.child(mark(24.0)),
		)
		.children(crumb)
		.child(View().class("flex-1"))
		.children(offline)
		.children(shipping)
		.children(cost.map(|cost| View().class("hidden sm:flex").child(cost)))
		.child(avatar(email.as_deref(), &chrome.route))
}

/// The account's initial in a circle, to `/account`.
fn avatar(email: Option<&str>, route: &Route) -> Element {
	let initial = email
		.and_then(|email| email.chars().next())
		.map_or_else(|| "·".to_owned(), |c| c.to_uppercase().collect());
	let mut link = Pressable("Your account")
		.href("/account")
		.class(style::TARGET)
		.child(
			View()
				.class("h-8 w-8 items-center justify-center rounded-full bg-[var(--cloud-sunken)]")
				.child(
					Text(initial)
						.class("text-sm font-semibold")
						.aria_hidden(true),
				),
		);

	if *route == Route::Account {
		link = link.aria_current(Current::Page);
	}

	link
}

fn rail(console: &Console, chrome: &Chrome) -> Element {
	let organizations = match &chrome.me {
		Load::Ready(me) => me.organizations.len(),
		_ => 0,
	};
	let Some(organization) = &chrome.organization else {
		return View()
			.class(style::RAIL)
			.role(Role::Navigation)
			.accessibility_label("Console")
			.children(matches!(chrome.me, Load::Loading).then(|| {
				screens::loading(
					"Loading your organisations",
					&["h-11 w-full", "h-11 w-full"],
				)
			}));
	};
	let projects_href = format!("/{}", organization.slug);
	let on_projects = matches!(&chrome.route, Route::Org(route) if route.org == organization.slug);
	let switch = (organizations > 1).then(|| {
		rail_item(
			console.focus,
			"rail-switch",
			"Switch organisation",
			"Switch",
			ICON_SWITCH,
			"/organizations",
			chrome.route == Route::Organizations,
		)
	});

	View()
		.class(style::RAIL)
		.role(Role::Navigation)
		.accessibility_label("Console")
		.child(
			View()
				.class("hidden lg:flex flex-row items-center gap-2 px-3 min-h-11")
				.child(
					Text(organization.name.clone())
						.class(style::CARD_TITLE)
						.class("flex-1 truncate"),
				),
		)
		.children(switch)
		.child(filter(console))
		.child(rail_item(
			console.focus,
			"rail-projects",
			"Projects",
			"Projects",
			ICON_APPS,
			&projects_href,
			on_projects,
		))
		// The group's label shows only over projects; an organisation with
		// none has the empty state in the page instead.
		.children(
			(!matches!(&chrome.projects, Load::Ready(projects) if projects.is_empty())).then(
				|| {
					View()
						.class("hidden lg:flex px-3 pt-3")
						.child(Text("PROJECTS").class(style::GROUP_LABEL).aria_hidden(true))
				},
			),
		)
		.child(rail_projects(console, chrome, &organization.slug))
		.child(rail_item(
			console.focus,
			"rail-new",
			"New project",
			"New project",
			ICON_PLUS,
			&format!("/{}/new", organization.slug),
			false,
		))
		.child(rail_item(
			console.focus,
			"rail-billing",
			"Billing",
			"Billing",
			ICON_BILLING,
			&format!("/{}/settings/billing", organization.slug),
			matches!(&chrome.route, Route::OrgBilling(route) if route.org == organization.slug),
		))
}

fn filter(console: &Console) -> Element {
	let filter = console.filter;
	let find = console.find;
	let typing = console.typing;
	let input = TextInput()
		.value(filter.get())
		.placeholder("Filter")
		.focused(find.get())
		.on_change(move |text: &str| filter.set(text.to_owned()))
		.on(NativeEventSlot::Focus, move |_| {
			typing.set(true);
			EventOutcome::Unhandled
		})
		.on(NativeEventSlot::Blur, move |_| {
			typing.set(false);
			find.set(false);
			EventOutcome::Unhandled
		})
		.on_key(move |key| {
			if key.is("Escape", false, false, false) {
				filter.set(String::new());
				find.set(false);
				return EventOutcome::Handled;
			}

			EventOutcome::Unhandled
		});

	View()
		.class("hidden lg:flex flex-row items-center gap-2 px-1")
		.child(icon(ICON_SEARCH))
		.child(
			crate::components::field("rail-filter", "Filter projects", input, None).class("flex-1"),
		)
		.child(
			Text("/")
				.class(style::LABEL)
				.class("rounded border border-[var(--cloud-line)] px-1.5")
				.aria_hidden(true),
		)
}

fn rail_projects(console: &Console, chrome: &Chrome, organization: &str) -> Element {
	// Named for screen readers, which skip the caption above it.
	let list = View()
		.class("flex-col gap-1")
		.role(Role::List)
		.accessibility_label("Projects");

	match &chrome.projects {
		Load::Loading => {
			list.child(screens::loading(
				"Loading projects",
				&["h-11 w-full", "h-11 w-full"],
			))
		}
		Load::Failed(error) => {
			list.child(
				Text(format!("✕ Projects didn't load: {error}"))
					.class(style::FIELD_ERROR)
					.class("hidden lg:flex px-3"),
			)
		}
		Load::Ready(projects) => {
			let needle = chrome.filter.trim().to_lowercase();
			let shown: Vec<&ProjectSummary> = projects
				.iter()
				.filter(|project| {
					needle.is_empty()
						|| project.name.to_lowercase().contains(&needle)
						|| project.slug.contains(&needle)
				})
				.collect();

			// No projects at all: no empty list for a screen reader to
			// count; the page has the empty state.
			if projects.is_empty() {
				return View();
			}

			if shown.is_empty() && !needle.is_empty() {
				return list.child(
					Status(format!("No projects match “{}”.", chrome.filter.trim()))
						.class(style::MUTED)
						.class("hidden lg:flex px-3"),
				);
			}

			list.children(shown.into_iter().map(|project| {
				let initial: String = project
					.name
					.chars()
					.next()
					.map(|c| c.to_uppercase().collect())
					.unwrap_or_default();
				let href = format!("/{organization}/{}", project.slug);
				let id = format!("rail-project-{}", project.slug);
				let item = Pressable(project.name.clone())
					.href(href)
					.class(style::RAIL_ITEM)
					.child(
						View()
							.class(
								"h-[18px] w-[18px] shrink-0 items-center justify-center \
								 rounded-[4px] bg-[var(--cloud-sunken)]",
							)
							.aria_hidden(true)
							.child(Text(initial).class("text-[11px] font-semibold")),
					)
					.child(Text(project.name.clone()).class(style::RAIL_WORDS));

				View()
					.role(Role::ListItem)
					.child(focus_ring(console.focus, &id, item).class("w-full"))
			}))
		}
	}
}

fn rail_item(
	focus: RwSignal<Option<String>>,
	id: &str,
	label: &str,
	words: &str,
	path: &str,
	href: &str,
	current: bool,
) -> Element {
	let mut item = Pressable(label)
		.href(href)
		.class(if current {
			style::RAIL_ITEM_CURRENT
		} else {
			style::RAIL_ITEM
		})
		.child(icon(path))
		.child(Text(words).class(style::RAIL_WORDS));

	if current {
		item = item.aria_current(Current::Page);
	}

	focus_ring(focus, id, item).class("w-full")
}

fn tab_bar(chrome: &Chrome) -> Element {
	let apps = chrome.organization.as_ref().map_or_else(
		|| "/organizations".to_owned(),
		|org| format!("/{}", org.slug),
	);
	let on_apps = matches!(chrome.route, Route::Org(_) | Route::OrgBilling(_));
	let on_you = matches!(chrome.route, Route::Account | Route::Organizations);

	View()
		.class(style::TAB_BAR)
		.role(Role::Navigation)
		.accessibility_label("Sections")
		.child(tab("Apps", ICON_APPS, &apps, on_apps))
		.child(tab("You", ICON_ACCOUNT, "/account", on_you))
}

fn tab(label: &str, path: &str, href: &str, current: bool) -> Element {
	let mut tab = Pressable(label)
		.href(href)
		.class(if current {
			style::TAB_CURRENT
		} else {
			style::TAB
		})
		.child(View().class(if current {
			style::TAB_BAR_MARK
		} else {
			style::TAB_BAR_SPACE
		}))
		.child(icon(path))
		.child(Text(label).aria_hidden(true));

	if current {
		tab = tab.aria_current(Current::Page);
	}

	tab
}
