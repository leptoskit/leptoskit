//! `/organizations`: your organisations and your role in each, and an
//! inline form to make another (`docs/agents/cloud-design.md` §14.1).

use leptoskit::native::Role;
use leptoskit::prelude::*;
use leptoskit::query::MutateCallbacks;
use leptoskit_cloud::access::RESERVED_ORGANIZATION_SLUGS;
use leptoskit_cloud::access::Role as OrganizationRole;
use leptoskit_cloud::account::CreateOrganization;
use leptoskit_cloud::account::OrganizationSummary;
use leptoskit_cloud::slug::Slug;

use crate::components::Kind;
use crate::components::button;
use crate::components::field;
use crate::components::focus_ring;
use crate::components::plain_button;
use crate::components::problem;
use crate::console::Console;
use crate::console::Load;
use crate::console::rpc_problem;
use crate::data::plan_name;
use crate::routes::Navigation;
use crate::style;

/// A role as the console names it.
#[must_use]
pub const fn role_name(role: OrganizationRole) -> &'static str {
	match role {
		OrganizationRole::Owner => "Owner",
		OrganizationRole::Admin => "Admin",
		OrganizationRole::Developer => "Developer",
		OrganizationRole::Billing => "Billing",
		OrganizationRole::Viewer => "Viewer",
	}
}

/// The new organisation's slug: the one typed, or else one made from the
/// name, and what's wrong with it, checked as it's typed.
#[must_use]
pub fn slug_for(name: &str, typed: Option<&str>) -> (String, Option<String>) {
	let slug = match typed {
		Some(typed) => typed.to_owned(),
		None => Slug::suggest(name).map_or_else(|_| String::new(), |slug| slug.as_str().to_owned()),
	};

	if slug.is_empty() {
		return (
			slug,
			(typed.is_some() || !name.trim().is_empty()).then(|| {
				"Choose a slug: 3 to 40 lower-case letters, digits and hyphens.".to_owned()
			}),
		);
	}

	if RESERVED_ORGANIZATION_SLUGS.contains(&slug.as_str()) {
		let problem = format!("“{slug}” is one of the console's own addresses; choose another.");
		return (slug, Some(problem));
	}

	// The slug's errors quote it in backticks, which read as stray marks on
	// a page: they become quotation marks.
	let problem = Slug::parse(&slug).err().map(|error| {
		let mut words = quoted(&error.to_string());
		words.push('.');
		words
	});
	(slug, problem)
}

/// `text` with each pair of backticks turned into quotation marks.
fn quoted(text: &str) -> String {
	let mut open = true;
	text.chars()
		.map(|c| {
			if c == '`' {
				let mark = if open { '“' } else { '”' };
				open = !open;
				mark
			} else {
				c
			}
		})
		.collect()
}

/// The form for a new organisation, as it stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Form {
	/// The name.
	pub name: String,
	/// The slug it will get.
	pub slug: String,
	/// What's wrong with the slug.
	pub slug_problem: Option<String>,
	/// Whether it's being made.
	pub creating: bool,
	/// Why making it failed.
	pub problem: Option<String>,
}

/// Everything the page shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrganizationsPage {
	/// The organisations.
	pub list: Load<Vec<OrganizationSummary>>,
	/// The form, while it's open.
	pub form: Option<Form>,
}

impl OrganizationsPage {
	/// The page the console's state calls for.
	#[must_use]
	pub fn read(console: &Console) -> Self {
		let new = console.new_organization;
		let form = new.open.get().then(|| {
			let name = new.name.get();
			let typed = new.slug.get();
			let (slug, slug_problem) = slug_for(&name, typed.as_deref());
			Form {
				name,
				slug,
				slug_problem,
				creating: new.create.is_pending(),
				problem: new.create.error().map(|error| {
					format!("The organisation wasn't created: {}.", rpc_problem(&error))
				}),
			}
		});

		Self {
			list: Load::of(&console.organizations, rpc_problem),
			form,
		}
	}
}

/// `/organizations`.
pub fn page(navigation: &Navigation) -> Element {
	let console = Console::get();
	view(&console, navigation, &OrganizationsPage::read(&console))
}

/// The page.
pub fn view(console: &Console, navigation: &Navigation, page: &OrganizationsPage) -> Element {
	let console = *console;
	let new = console.new_organization;
	let header = View()
		.class("flex-row flex-wrap items-center gap-3")
		.child(
			Heading(1, "Organisations")
				.class(style::PAGE_TITLE)
				.class("flex-1"),
		)
		.children(page.form.is_none().then(|| {
			focus_ring(
				console.focus,
				"organizations-create",
				button(Kind::Secondary, "Create an organisation", None, move || {
					new.create.reset();
					new.open.set(true);
				}),
			)
		}));
	let list = match &page.list {
		Load::Loading => super::loading("Loading organisations", &["h-20 w-full", "h-20 w-full"]),
		Load::Failed(error) => {
			let organizations = console.organizations;
			super::load_failed("your organisations", error, move || {
				drop(organizations.refetch());
			})
		}
		Load::Ready(organizations) => {
			View()
				.class("flex-col gap-3")
				.role(Role::List)
				.accessibility_label("Your organisations")
				.children(
					organizations
						.iter()
						.map(|organization| row(&console, organization)),
				)
		}
	};

	View()
		.class("flex-col gap-6 w-full max-w-[720px]")
		.test_id("organizations")
		.child(header)
		.children(
			page.form
				.as_ref()
				.map(|form| self::form(&console, navigation, form)),
		)
		.child(list)
}

fn row(console: &Console, organization: &OrganizationSummary) -> Element {
	let console = *console;
	let href = format!("/{}", organization.slug);
	let mut facts = vec![
		Text(role_name(organization.role)).class(style::SECONDARY),
		Text("·").class(style::MUTED).aria_hidden(true),
		Text(plan_name(&organization.plan)).class(style::SECONDARY),
	];

	if organization.personal {
		facts.push(Text("·").class(style::MUTED).aria_hidden(true));
		facts.push(Text("Personal").class(style::SECONDARY));
	}

	View()
		.class(style::CARD)
		.role(Role::ListItem)
		.child(
			View()
				.class(style::CARD_HEADER)
				.child(focus_ring(
					console.focus,
					&format!("organization-{}", organization.slug),
					Pressable(format!("{}: projects", organization.name))
						.href(href)
						.class("min-h-11 flex-row items-center cursor-pointer")
						.child(Text(organization.name.clone()).class(style::CARD_TITLE)),
				))
				.child(
					Text(format!("/{}", organization.slug))
						.class(style::CODE)
						.class("text-[var(--cloud-ink-3)]"),
				),
		)
		.child(
			View()
				.class("flex-row flex-wrap items-center gap-2")
				.children(facts),
		)
}

fn form(console: &Console, navigation: &Navigation, form: &Form) -> Element {
	let console = *console;
	let new = console.new_organization;
	let typing = console.typing;
	let focus_events = |input: Element| {
		input
			.on(NativeEventSlot::Focus, move |_| {
				typing.set(true);
				EventOutcome::Unhandled
			})
			.on(NativeEventSlot::Blur, move |_| {
				typing.set(false);
				EventOutcome::Unhandled
			})
	};
	let name = focus_events(
		TextInput()
			.value(form.name.clone())
			.placeholder("Acme")
			.autocomplete("organization")
			.focused(true)
			.on_change(move |text: &str| new.name.set(text.to_owned())),
	);
	let slug = focus_events(
		TextInput()
			.value(form.slug.clone())
			.placeholder("acme")
			.autocomplete("off")
			.on_change(move |text: &str| {
				new.slug
					.set((!text.is_empty()).then(|| text.trim().to_lowercase()));
			}),
	);
	let ready =
		!form.name.trim().is_empty() && form.slug_problem.is_none() && !form.slug.is_empty();
	let create = {
		let navigation = navigation.clone();
		let slug = form.slug.clone();
		let name = form.name.trim().to_owned();
		move || {
			let to = format!("/{slug}");
			let navigation = navigation.clone();
			new.create.mutate_with(
				CreateOrganization {
					slug: slug.clone(),
					name: name.clone(),
				},
				MutateCallbacks::new().on_success(move |_| {
					new.open.set(false);
					new.name.set(String::new());
					new.slug.set(None);

					if let Some(route) = crate::console::route_of(&to) {
						navigation.navigate(route);
					}
				}),
			);
		}
	};
	let mut create_button = plain_button(
		Kind::Primary,
		"Create organisation",
		form.creating.then_some("Creating…"),
		create,
	);

	if !ready {
		create_button = create_button.enabled(false);
	}

	View()
		.class(style::CARD)
		.role(Role::Form)
		.accessibility_label("New organisation")
		.child(Heading(2, "New organisation").class(style::CARD_TITLE))
		.child(field("organization-name", "Name", name, None))
		.child(field(
			"organization-slug",
			"Slug: the address of its projects",
			slug,
			form.slug_problem.as_deref(),
		))
		.child(Text(format!("Its projects will be at /{}.", form.slug)).class(style::MUTED))
		.children(form.problem.as_ref().map(|text| problem(text.clone())))
		.child(
			View()
				.class("flex-row flex-wrap items-center gap-3")
				.child(focus_ring(
					console.focus,
					"organization-create",
					crate::components::raised(create_button),
				))
				.child(focus_ring(
					console.focus,
					"organization-cancel",
					plain_button(Kind::Quiet, "Cancel", None, move || {
						new.open.set(false);
						new.create.reset();
					}),
				)),
		)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_slug_follows_the_name_until_it_is_typed() {
		assert_eq!(slug_for("Acme Labs", None), ("acme-labs".to_owned(), None));
		assert_eq!(
			slug_for("Acme Labs", Some("acme")),
			("acme".to_owned(), None)
		);
		assert_eq!(slug_for("", None), (String::new(), None));
	}

	#[test]
	fn console_addresses_and_broken_slugs_are_caught_as_they_are_typed() {
		let (_, problem) = slug_for("", Some("device"));
		assert_eq!(
			problem.as_deref(),
			Some("“device” is one of the console's own addresses; choose another.")
		);
		let (_, problem) = slug_for("", Some("ab"));
		assert_eq!(
			problem.as_deref(),
			Some("“ab” has 2 characters; a slug has 3 to 40.")
		);
		let (slug, problem) = slug_for("Go", None);
		assert_eq!(slug, "");
		assert_eq!(
			problem.as_deref(),
			Some("Choose a slug: 3 to 40 lower-case letters, digits and hyphens.")
		);
	}
}
