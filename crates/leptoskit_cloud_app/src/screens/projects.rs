//! `/<org>`: every project in the organisation, and the first-run sheet when
//! there are none (`docs/agents/cloud-design.md` §10.1).
//!
//! Each project is a `TurnCard`: its front is what `projects::projects`
//! says, and its back says so. The environment row, the production URL and
//! the month's cost on each card need operations the control plane doesn't
//! have yet (§20 #3, #4, #9), so they aren't drawn rather than guessed.

use leptoskit::native::Role;
use leptoskit::prelude::*;
use leptoskit_cloud::account::OrganizationSummary;
use leptoskit_cloud::projects::ProjectSummary;

use crate::components::Kind;
use crate::components::TurnCard;
use crate::components::focus_ring;
use crate::components::link_button;
use crate::components::sheet;
use crate::console::Console;
use crate::console::Load;
use crate::console::rpc_problem;
use crate::data::PlanTerms;
use crate::routes::Navigation;
use crate::routes::Org;
use crate::screens::account::date;
use crate::shell::current_organization;
use crate::style;

/// Everything the projects page shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProjectsPage {
	/// The organisations haven't loaded.
	Loading,
	/// The person's organisations didn't load.
	Failed(String),
	/// No organisation by this slug that the person can see.
	NotFound {
		/// The slug in the address.
		slug: String,
	},
	/// The organisation.
	Organization {
		/// It.
		organization: OrganizationSummary,
		/// Its projects.
		projects: Load<Vec<ProjectSummary>>,
		/// What its plan includes, once `billing::plans` exists.
		plan: PlanTerms,
	},
}

impl ProjectsPage {
	/// The page the console's state calls for on `route`.
	#[must_use]
	pub fn read(console: &Console, route: &Org) -> Self {
		match console.me() {
			Load::Loading => Self::Loading,
			Load::Failed(error) => Self::Failed(error),
			Load::Ready(me) => {
				let current = crate::routes::Route::Org(route.clone());
				match current_organization(&current, &me) {
					None => {
						Self::NotFound {
							slug: route.org.clone(),
						}
					}
					Some(organization) => {
						Self::Organization {
							plan: console.services().billing.plan(&organization.plan),
							organization: organization.clone(),
							projects: Load::of(&console.projects, rpc_problem),
						}
					}
				}
			}
		}
	}
}

/// `/<org>`.
pub fn page(route: &Org, navigation: &Navigation) -> Element {
	let console = Console::get();
	let page = ProjectsPage::read(&console, route);

	if matches!(page, ProjectsPage::NotFound { .. }) {
		super::respond_with(404);
	}

	view(&console, navigation, &page)
}

/// The page.
pub fn view(console: &Console, _navigation: &Navigation, page: &ProjectsPage) -> Element {
	let console = *console;

	let body = match page {
		ProjectsPage::Loading => {
			vec![
				header(&console, None),
				super::loading("Loading projects", &["h-28 w-full", "h-28 w-full"]),
			]
		}
		ProjectsPage::Failed(error) => {
			let whoami = console.whoami;
			vec![
				header(&console, None),
				super::load_failed("your organisations", error, move || {
					drop(whoami.refetch());
				}),
			]
		}
		ProjectsPage::NotFound { slug } => {
			vec![
				View()
					.class("flex-col items-start gap-3")
					.test_id("organization-not-found")
					.child(Heading(1, "Not here.").class(style::PAGE_TITLE))
					.child(
						Text(format!(
							"There's no organisation at {slug}, or you can't see it."
						))
						.class(style::SECONDARY),
					)
					.child(crate::components::link(
						"Your organisations →",
						"/organizations",
					)),
			]
		}
		ProjectsPage::Organization {
			organization,
			projects,
			plan,
		} => {
			match projects {
				Load::Loading => {
					vec![
						header(&console, Some(organization)),
						super::loading("Loading projects", &["h-28 w-full", "h-28 w-full"]),
					]
				}
				Load::Failed(error) => {
					let projects = console.projects;
					vec![
						header(&console, Some(organization)),
						super::load_failed("the projects", error, move || {
							drop(projects.refetch());
						}),
					]
				}
				Load::Ready(projects) if projects.is_empty() => {
					vec![empty(&console, organization, plan)]
				}
				Load::Ready(projects) => {
					vec![
						header(&console, Some(organization)),
						View()
							.class("grid grid-cols-1 lg:grid-cols-2 gap-4")
							.role(Role::List)
							.accessibility_label("Projects")
							.children(projects.iter().enumerate().map(|(index, project)| {
								View().role(Role::ListItem).child(card(
									&console,
									organization,
									project,
									index,
								))
							})),
					]
				}
			}
		}
	};

	View()
		.class("flex-col gap-6 w-full")
		.test_id("projects")
		.children(body)
}

fn header(console: &Console, organization: Option<&OrganizationSummary>) -> Element {
	let console = *console;
	View()
		.class("flex-row flex-wrap items-center gap-3")
		.child(
			Heading(1, "Projects")
				.class(style::PAGE_TITLE)
				.class("flex-1"),
		)
		.children(organization.map(|organization| {
			focus_ring(
				console.focus,
				"projects-new",
				link_button(
					Kind::Primary,
					"New project",
					format!("/{}/new", organization.slug),
				),
			)
		}))
}

fn card(
	console: &Console,
	organization: &OrganizationSummary,
	project: &ProjectSummary,
	index: usize,
) -> Element {
	let console = *console;
	// The card's title is the project's name. It links nowhere until the
	// project's overview exists (UI 2).
	let front = vec![
		Text(format!("{}/{}", organization.slug, project.slug))
			.class(style::CODE)
			.class("text-[var(--cloud-ink-2)]"),
		Text(format!(
			"Region {} · since {}",
			project.region,
			date(&project.created_at)
		))
		.class(style::MUTED),
	];
	let back = vec![
		fact("Slug", &project.slug),
		fact("Region", &project.region),
		fact("Created", &date(&project.created_at)),
		fact("ID", &project.id.to_string()),
	];

	TurnCard {
		id: format!("project-{}", project.slug),
		title: project.name.clone(),
		source: "projects::projects".to_owned(),
		index,
		front,
		back,
	}
	.build(console.turned)
}

fn fact(label: &str, value: &str) -> Element {
	View()
		.class("flex-row flex-wrap items-center gap-2")
		.child(Text(label).class("text-xs font-medium text-[#c9c0b6] w-16"))
		.child(
			Text(value.to_owned())
				.class(style::UNDERSIDE_TEXT)
				.class("font-[var(--cloud-font-mono)]"),
		)
}

/// No projects yet: a sheet with the three ways in.
fn empty(console: &Console, organization: &OrganizationSummary, plan: &PlanTerms) -> Element {
	let console = *console;
	let org = &organization.slug;
	let ways = [
		(
			"From your terminal",
			"For an app you already have.",
			format!("/{org}/new?path=terminal"),
		),
		(
			"A starter, in one press",
			"No code yet? Ship one of ours and change it later.",
			format!("/{org}/new?path=starter"),
		),
		(
			"From GitHub",
			"Ship a repository you already have on GitHub.",
			format!("/{org}/new?path=github"),
		),
	];
	let mut lines = vec![
		Heading(1, "Ship your first app").class(style::DISPLAY),
		Text("Three ways in. Each ends the same way: a signed release sheet and a live URL.")
			.class(style::SECONDARY),
		View()
			.class("flex-col gap-2")
			.role(Role::List)
			.accessibility_label("Ways to ship")
			.children(
				ways.into_iter()
					.enumerate()
					.map(|(index, (title, detail, href))| {
						View().role(Role::ListItem).child(
							focus_ring(
								console.focus,
								&format!("projects-way-{index}"),
								Pressable(title)
									.href(href)
									.class(
										"min-h-11 flex-row items-center gap-3 rounded-lg px-3 \
										 py-2 hover:bg-[var(--cloud-sunken)] cursor-pointer",
									)
									.child(
										View()
											.class("flex-1 flex-col")
											.child(Text(title).class(style::CARD_TITLE))
											.child(Text(detail).class(style::SECONDARY)),
									)
									.child(Text("→").class(style::SECONDARY).aria_hidden(true)),
							)
							.class("w-full"),
						)
					}),
			),
	];

	if let PlanTerms::Known { name, includes } = plan {
		lines.push(Text(format!("{name}: {includes}.")).class(style::MUTED));
	}

	View()
		.class("flex-col items-start w-full max-w-xl")
		.test_id("projects-empty")
		.child(sheet(lines))
}
