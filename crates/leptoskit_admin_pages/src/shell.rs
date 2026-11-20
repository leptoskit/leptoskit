//! The frame around every screen: top bar, rail, content, and on phones a
//! bottom tab bar.
//!
//! The rail and the tab bar are links, the same on every page. The top bar
//! is where the admin's chrome does something: [`Shell`] holds what it
//! changes (the theme, Workings mode, the account menu, and search) and
//! builds the frame from it, on the server and again in the browser.

use std::sync::Arc;

use leptos::context::provide_context;
use leptoskit::aria::Menu;
use leptoskit::auth::ErrorResponse;
use leptoskit::auth::client::sign_out_mutation;
use leptoskit::native::FocusRequest;
use leptoskit::native::NativeEventSlot;
use leptoskit::native::Placement;
use leptoskit::native::Role;
use leptoskit::native::vector::Paint;
use leptoskit::native::vector::PathData;
use leptoskit::native::vector::Stroke;
use leptoskit::native::vector::VectorPath;
use leptoskit::native::vector::ViewBox;
use leptoskit::prelude::*;
use leptoskit::query::MutationHandle;
use leptoskit::query::use_mutation;
use leptoskit::shortcut::Shortcut;
use leptoskit::shortcut::use_shortcut;

use crate::BASE;
use crate::analytics::tail::Tail;
use crate::components;
use crate::confirm::Confirm;
use crate::model::Model;
use crate::model::Page;
use crate::model::Section;
use crate::model::Viewer;
use crate::style;
use crate::workings;
use crate::workings::Workings;

/// An icon, drawn as strokes in the current text colour.
pub fn icon(path: &str) -> Element {
	let view_box = ViewBox::new(0.0, 0.0, 24.0, 24.0).expect("a valid view box");
	let mut vector = Vector(view_box)
		.class("h-[18px] w-[18px] shrink-0")
		.aria_hidden(true);

	for part in path.split('|') {
		if let (Ok(data), Ok(stroke)) = (
			PathData::parse(part),
			Stroke::new(Paint::CurrentColor, 1.75),
		) {
			vector = vector.path(VectorPath::new(data).stroke(stroke.round()));
		}
	}

	vector
}

const ICON_OVERVIEW: &str = "M4 4h6v6H4z|M14 4h6v6h-6z|M4 14h6v6H4z|M14 14h6v6h-6z";
const ICON_LIVE: &str = "M3 12h4l3-7 4 14 3-7h4";
const ICON_TRENDS: &str = "M3 17l6-6 4 4 8-8";
const ICON_FUNNELS: &str = "M4 5h16l-6 8v6l-4-2v-4z";
const ICON_RETENTION: &str = "M4 6h16|M4 12h11|M4 18h6";
const ICON_LIBRARY: &str = "M4 4h16v16H4z|M4 10h16|M10 10v10";
const ICON_PEOPLE: &str = "M12 12a4 4 0 1 0 0-8 4 4 0 0 0 0 8z|M4 21c0-4 4-6 8-6s8 2 8 6";
const ICON_SERVER: &str = "M4 4h16v7H4z|M4 13h16v7H4z|M8 7.5h.01|M8 16.5h.01";
const ICON_HEALTH: &str = "M12 20s-7-4.5-7-10a4 4 0 0 1 7-2.6A4 4 0 0 1 19 10c0 5.5-7 10-7 10z";
const ICON_CLOCK: &str = "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18z|M12 7v5l3 2";
const ICON_LEARN: &str = "M2 9l10-5 10 5-10 5z|M6 11v5c3 2 9 2 12 0v-5";
const ICON_BOOK: &str = "M5 4h12a2 2 0 0 1 2 2v14H7a2 2 0 0 1-2-2z|M5 18a2 2 0 0 1 2-2h12";
pub const ICON_SEARCH: &str = "M11 18a7 7 0 1 0 0-14 7 7 0 0 0 0 14z|M20 20l-4-4";

/// The leptoskit mark: two ember boxes and the mint piece that arrives over
/// the air, tilted −12°.
pub fn mark(size: f32) -> Element {
	let unit = size / 28.0;
	let px = |value: f32| format!("{:.1}px", value * unit);
	View()
		.class("relative shrink-0")
		.class(format!("h-[{}] w-[{}]", px(28.0), px(28.0)))
		.aria_hidden(true)
		.child(
			View()
				.class("absolute rounded-[3px] bg-[var(--admin-ember)]")
				.class(format!(
					"left-0 top-[{}] h-[{}] w-[{}]",
					px(4.0),
					px(24.0),
					px(11.0)
				)),
		)
		.child(
			View()
				.class("absolute rounded-[3px] bg-[var(--admin-ember-warm)]")
				.class(format!(
					"left-[{}] top-[{}] h-[{}] w-[{}]",
					px(14.0),
					px(17.0),
					px(11.0),
					px(11.0)
				)),
		)
		.child(
			View()
				.class("absolute rounded-[3px] bg-[var(--admin-mint)] -rotate-12")
				.class(format!(
					"left-[{}] top-0 h-[{}] w-[{}]",
					px(15.0),
					px(11.0),
					px(11.0)
				)),
		)
}

struct RailItem {
	label: String,
	href: String,
	icon: &'static str,
	count: Option<String>,
	selected: bool,
	indent: bool,
}

fn rail_item(item: RailItem) -> Element {
	let mut row = View()
		.class(
			"relative flex-row items-center gap-2.5 rounded-lg px-2.5 py-[7px] \
			 max-lg:justify-center",
		)
		.class(if item.selected {
			"bg-[var(--admin-sunken)] text-[var(--admin-ink)]"
		} else {
			"text-[var(--admin-ink-2)]"
		});

	if item.selected {
		row = row.child(
			View()
				.class(
					"absolute -left-2.5 top-1.5 bottom-1.5 w-[3px] rounded-[3px] \
					 bg-[var(--admin-ember)]",
				)
				.aria_hidden(true),
		);
	}

	if item.indent {
		row = row.child(
			View()
				.class("w-4 shrink-0 max-lg:hidden")
				.test_id("rail-label"),
		);
	}

	row = row
		.child(icon(item.icon).class(if item.selected {
			"text-[var(--admin-ink)]"
		} else {
			"text-[var(--admin-ink-3)]"
		}))
		.child(
			Text(item.label.clone())
				.class("min-w-0 flex-1 truncate text-sm max-lg:hidden")
				.class(if item.selected { "font-semibold" } else { "" })
				.test_id("rail-label"),
		);

	if let Some(count) = item.count {
		row = row.child(
			Text(count)
				.class("text-[11px] tabular-nums text-[var(--admin-ink-3)] max-lg:hidden")
				.test_id("rail-label"),
		);
	}

	let mut link = Pressable(item.label)
		.href(item.href)
		.class("cursor-pointer rounded-lg hover:bg-[var(--admin-sunken)]")
		.child(row);

	if item.selected {
		link = link.aria_current(leptoskit::native::Current::Page);
	}

	link
}

fn group(label: &str) -> Element {
	Text(label.to_uppercase())
		.class(style::GROUP_LABEL)
		.class("mx-2.5 mb-1.5 mt-3.5 max-lg:hidden")
		.test_id("rail-label")
}

pub fn rail(viewer: &Viewer, section: &Section) -> Element {
	let item = |label: &str, href: String, icon, selected: bool| {
		rail_item(RailItem {
			label: label.to_owned(),
			href,
			icon,
			count: None,
			selected,
			indent: false,
		})
	};

	let mut rail = View()
		.class(
			"w-60 shrink-0 flex-col gap-0.5 border-r border-[var(--admin-line)] \
			 bg-[var(--admin-surface)] px-2.5 py-3.5 overflow-scroll max-lg:w-16 max-lg:px-2 \
			 max-sm:hidden",
		)
		.test_id("rail")
		.role(Role::Navigation)
		.aria_label("Admin")
		.child(group("Analytics"))
		.child(item(
			"Overview",
			format!("{BASE}/analytics/overview"),
			ICON_OVERVIEW,
			*section == Section::Overview,
		))
		.child(item(
			"Live",
			format!("{BASE}/analytics/live"),
			ICON_LIVE,
			*section == Section::Live,
		))
		.child(item(
			"Trends",
			format!("{BASE}/analytics/trends"),
			ICON_TRENDS,
			*section == Section::Trends,
		))
		.child(item(
			"Funnels",
			format!("{BASE}/analytics/funnels"),
			ICON_FUNNELS,
			*section == Section::Funnels,
		))
		.child(item(
			"Retention",
			format!("{BASE}/analytics/retention"),
			ICON_RETENTION,
			*section == Section::Retention,
		))
		.child(item(
			"Library",
			format!("{BASE}/analytics/library"),
			ICON_LIBRARY,
			*section == Section::Library,
		))
		.child(item(
			"People",
			format!("{BASE}/analytics/people"),
			ICON_PEOPLE,
			*section == Section::People,
		))
		.child(group("Infrastructure"));

	if viewer.projects.is_empty() {
		rail = rail.child(item(
			"Projects",
			format!("{BASE}/infra/projects"),
			ICON_SERVER,
			*section == Section::Projects,
		));
	}

	for project in &viewer.projects {
		let base = format!("{BASE}/infra/projects/{project}");
		rail = rail.child(item(
			project,
			format!("{base}/overview"),
			ICON_SERVER,
			*section == Section::Project(project.clone()),
		));
		let current = matches!(section, Section::Project(p) | Section::ProjectHealth(p) | Section::ProjectOperations(p) if p == project);

		if current || viewer.projects.len() == 1 {
			rail = rail
				.child(rail_item(RailItem {
					label: "Health".into(),
					href: format!("{base}/health"),
					icon: ICON_HEALTH,
					count: None,
					selected: *section == Section::ProjectHealth(project.clone()),
					indent: true,
				}))
				.child(rail_item(RailItem {
					label: "Operations".into(),
					href: format!("{base}/operations"),
					icon: ICON_CLOCK,
					count: None,
					selected: *section == Section::ProjectOperations(project.clone()),
					indent: true,
				}));
		}
	}

	rail.child(group("Learn"))
		.child(rail_item(RailItem {
			label: "Your progress".into(),
			href: format!("{BASE}/learn"),
			icon: ICON_LEARN,
			count: Some(format!("{}/{}", viewer.progress.0, viewer.progress.1)),
			selected: *section == Section::Learn,
			indent: false,
		}))
		.child(item(
			"Glossary",
			format!("{BASE}/learn/glossary"),
			ICON_BOOK,
			*section == Section::Glossary,
		))
}

/// The phone's bottom bar: the three areas, and `search`, the control that
/// opens search.
pub fn tab_bar(section: &Section, viewer: &Viewer, search: Element) -> Element {
	let area = |label: &str, href: String, icon_path, on: bool| {
		let mut link = Pressable(label.to_owned())
			.href(href)
			.class("flex-1 cursor-pointer")
			.child(
				View()
					.class("flex-col items-center gap-0.5")
					.class(if on {
						"text-[var(--admin-ink)]"
					} else {
						"text-[var(--admin-ink-3)]"
					})
					.child(icon(icon_path))
					.child(Text(label.to_owned()).class("text-[11px] font-semibold")),
			);

		if on {
			link = link.aria_current(leptoskit::native::Current::Page);
		}

		link
	};
	let analytics = matches!(
		section,
		Section::Overview
			| Section::Live
			| Section::Trends
			| Section::Funnels
			| Section::Retention
			| Section::Library
			| Section::People
	);
	let learn = matches!(section, Section::Learn | Section::Glossary);
	let infra_href = match viewer.projects.first() {
		Some(project) if viewer.projects.len() == 1 => {
			format!("{BASE}/infra/projects/{project}/overview")
		}
		_ => format!("{BASE}/infra/projects"),
	};

	View()
		.class(
			"shrink-0 flex-row border-t border-[var(--admin-line)] bg-[var(--admin-surface)] px-2 \
			 pt-1.5 pb-safe-offset-1.5 sm:hidden",
		)
		.role(Role::Navigation)
		.aria_label("Areas")
		.test_id("tab-bar")
		.child(area(
			"Analytics",
			format!("{BASE}/analytics/overview"),
			ICON_TRENDS,
			analytics,
		))
		.child(area("Infra", infra_href, ICON_SERVER, !analytics && !learn))
		.child(area("Learn", format!("{BASE}/learn"), ICON_LEARN, learn))
		.child(search)
}

/// The phone's control that opens search.
#[must_use]
pub fn search_open() -> Element {
	Button("⌘K")
		.aria_label("Find a screen")
		.class("flex-1 text-[11px] font-semibold text-[var(--admin-ink-3)] cursor-pointer")
		.test_id("search-open")
}

/// The app's mark and name, linking to the admin's home.
#[must_use]
pub fn brand(viewer: &Viewer) -> Element {
	Pressable(format!("{} admin home", viewer.app_name))
		.href(format!("{BASE}/"))
		.class("flex-row items-center gap-2 cursor-pointer")
		.child(mark(24.0))
		.child(
			Text(viewer.app_name.clone())
				.class("text-[15px] font-semibold text-[var(--admin-ink)]"),
		)
		.child(
			Text("admin").class("text-[15px] font-medium text-[var(--admin-ink-3)] max-sm:hidden"),
		)
}

/// The project switcher, on a project's screens when there is more than
/// one project.
#[must_use]
pub fn switcher(viewer: &Viewer, section: &Section) -> Option<Element> {
	let (Section::Project(project)
	| Section::ProjectHealth(project)
	| Section::ProjectOperations(project)) = section
	else {
		return None;
	};

	(viewer.projects.len() > 1).then(|| {
		Pressable("Switch project")
			.href(format!("{BASE}/infra/projects"))
			.class(style::CHIP)
			.class("cursor-pointer max-md:hidden")
			.child(Text(format!("{project} ▾")))
	})
}

/// The live events indicator: how fast analytics events arrive, linking to
/// the Live screen.
#[must_use]
pub fn pulse(viewer: &Viewer) -> Element {
	pulse_of(viewer.pulse.as_ref())
}

/// The live events indicator for `pulse`, or that analytics isn't
/// connected.
#[must_use]
pub fn pulse_of(pulse: Option<&crate::model::Pulse>) -> Element {
	const CLASS: &str = "flex-row items-center gap-2 rounded-full border \
	                     border-[var(--admin-line)] px-2.5 py-1 cursor-pointer \
	                     hover:bg-[var(--admin-sunken)] max-sm:hidden";

	let Some(pulse) = pulse else {
		return Pressable("Live events: analytics isn't connected yet")
			.href(format!("{BASE}/analytics/live"))
			.class(CLASS)
			.test_id("live-pulse")
			.child(View().class("h-[7px] w-[7px] rounded-full bg-[var(--admin-ink-3)]"))
			.child(Text("Not listening").class("text-xs text-[var(--admin-ink-2)] max-md:hidden"));
	};

	#[expect(clippy::cast_precision_loss, reason = "counts for drawing")]
	let seconds: Vec<f64> = pulse.seconds.iter().map(|count| *count as f64).collect();
	Pressable(format!("Live events: {:.1} a second", pulse.per_second))
		.href(format!("{BASE}/analytics/live"))
		.class(CLASS)
		.test_id("live-pulse")
		.child(
			crate::charts::trend_sparkline(&seconds, "h-4 w-14 max-md:hidden")
				.test_id("pulse-spark"),
		)
		.child(
			View()
				.class("h-[7px] w-[7px] rounded-full")
				.class(if pulse.live {
					"bg-[var(--admin-ember)]"
				} else {
					"bg-[var(--admin-ink-3)]"
				})
				.test_id("pulse-dot"),
		)
		.child(
			Text(format!("{:.0}/s", pulse.per_second))
				.class("text-xs tabular-nums text-[var(--admin-ink-2)]")
				.test_id("pulse-rate"),
		)
}

/// One place search can take the person.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
	/// What the person reads and types.
	pub label: String,
	/// Where it leads.
	pub href: String,
}

/// Every place search knows: the rail's screens, then the Learn pages.
#[must_use]
pub fn search_targets(viewer: &Viewer) -> Vec<Target> {
	let target = |label: &str, path: &str| {
		Target {
			label: label.to_owned(),
			href: format!("{BASE}{path}"),
		}
	};
	let mut targets = vec![
		target("Overview", "/analytics/overview"),
		target("Live", "/analytics/live"),
		target("Trends", "/analytics/trends"),
		target("Funnels", "/analytics/funnels"),
		target("Retention", "/analytics/retention"),
		target("Library", "/analytics/library"),
		target("People", "/analytics/people"),
	];

	if viewer.projects.is_empty() {
		targets.push(target("Projects", "/infra/projects"));
	}

	for project in &viewer.projects {
		targets.push(target(
			project,
			&format!("/infra/projects/{project}/overview"),
		));
		targets.push(Target {
			label: format!("{project} health"),
			href: format!("{BASE}/infra/projects/{project}/health"),
		});
		targets.push(Target {
			label: format!("{project} operations"),
			href: format!("{BASE}/infra/projects/{project}/operations"),
		});
	}

	targets.push(target("Your progress", "/learn"));
	targets.push(target("Glossary", "/learn/glossary"));
	targets
}

/// The targets whose label holds `query`, ignoring case: at most eight.
#[must_use]
pub fn matching(targets: &[Target], query: &str) -> Vec<Target> {
	let query = query.trim().to_lowercase();

	if query.is_empty() {
		return Vec::new();
	}

	targets
		.iter()
		.filter(|target| target.label.to_lowercase().contains(&query))
		.take(8)
		.cloned()
		.collect()
}

/// What the chrome around every page changes, and the page it frames.
#[derive(Clone)]
pub struct Shell {
	/// The page's model, as the server sent it.
	pub model: Arc<Model>,
	/// The page shown now: the model's, until a builder changes in place.
	pub(crate) page: RwSignal<Page>,
	/// Workings mode and the page's drawers.
	pub workings: Workings,
	/// `light` or `dark` once the person chose one.
	appearance: RwSignal<Option<String>>,
	/// Whether the account menu is open.
	menu_open: RwSignal<bool>,
	/// What the person typed into search.
	query: RwSignal<String>,
	/// Whether search shows on a phone, where it starts hidden.
	search_open: RwSignal<bool>,
	/// Takes focus to the search field.
	search: FocusRequest,
	/// What the notice above the content says.
	pub(crate) notice: RwSignal<String>,
	/// The confirm sheet a page's actions ask through.
	pub(crate) confirm: Confirm,
	/// Signing out at the main site.
	sign_out: MutationHandle<(), (), ErrorResponse>,
	/// The analytics stream: the pulse, and on Live, each event.
	pub(crate) tail: Tail,
}

impl Shell {
	/// The chrome for `model`'s page, with its keyboard shortcuts: ⌘K (Ctrl+K
	/// elsewhere) finds a screen and Shift+W turns Workings mode on or off.
	#[must_use]
	pub fn new(model: Model) -> Self {
		let tail = Tail::open(
			model.viewer.pulse.clone(),
			matches!(model.page, Page::Live(_)),
		);
		let shell = Self {
			workings: Workings::new(
				model.viewer.workings_mode,
				model.open.clone(),
				model.page.drawers().first().map(|drawer| drawer.id.clone()),
			),
			appearance: RwSignal::new(model.viewer.appearance.clone()),
			page: RwSignal::new(model.page.clone()),
			model: Arc::new(model),
			menu_open: RwSignal::new(false),
			query: RwSignal::new(String::new()),
			search_open: RwSignal::new(false),
			search: FocusRequest::new(),
			notice: RwSignal::new(String::new()),
			confirm: Confirm::new(),
			sign_out: use_mutation(sign_out_mutation()),
			tail,
		};

		let find = shell.clone();
		use_shortcut(Shortcut::primary("k"), move || find.find());
		let workings = shell.clone();
		use_shortcut(Shortcut::key("w").shift().unless_typing(), move || {
			workings.toggle_workings();
		});
		let drawers = shell.workings;
		let confirm = shell.confirm;
		provide_context(drawers);
		provide_context(confirm);
		provide_context(tail);
		provide_context(components::Notes::new());
		// While the confirm sheet asks, no shortcut runs: its keys are its
		// own.
		use_shortcut(Shortcut::key("w").unless_typing(), move || {
			drawers.toggle_first();
		});
		use_shortcut(Shortcut::key("Escape"), move || drawers.hide());

		shell
	}

	/// Shows search and moves focus into it.
	fn find(&self) {
		self.search_open.set(true);
		self.search.request();
	}

	/// Turns Workings mode on or off, and remembers it for the person.
	fn toggle_workings(&self) {
		let on = !self.workings.mode.get_untracked();
		self.workings.mode.set(on);
		browser::post(
			&format!("{BASE}/api/preferences"),
			&format!(r#"{{"kind":"workings_mode","on":{on}}}"#),
		);
	}

	/// Shows light if the page is dark and dark if it is light, and
	/// remembers the choice in a cookie the server reads.
	fn toggle_theme(&self) {
		let dark = match self.appearance.get_untracked().as_deref() {
			Some("dark") => true,
			Some("light") => false,
			_ => browser::prefers_dark(),
		};
		let (next, appearance) = if dark {
			("light", leptoskit::appearance::Appearance::Light)
		} else {
			("dark", leptoskit::appearance::Appearance::Dark)
		};
		self.appearance.set(Some(next.to_owned()));
		leptoskit::appearance::set(appearance);
		browser::set_cookie(&format!(
			"admin_theme={next}; path=/; max-age=31536000; samesite=lax"
		));
	}

	/// Signs out at the main site, then goes to the admin's sign-in page.
	fn leave(&self) {
		let sign_out = self.sign_out;
		crate::spawn(async move {
			// Signed out or not, the sign-in page says where the person
			// stands.
			let _ = sign_out.mutate_async(()).await;
			crate::go(&format!("{BASE}/sign-in"));
		});
	}

	/// The whole screen around `content`.
	#[must_use]
	pub fn frame(&self, content: Element) -> Element {
		let viewer = &self.model.viewer;
		let section = self.page.with(Page::section);
		let main = View()
			.class("min-w-0 flex-1 overflow-scroll")
			.role(Role::Main)
			.test_id("main")
			.child(
				View()
					.class(
						"mx-auto w-full max-w-[1200px] flex-col gap-5 px-7 pb-12 pt-6 max-sm:px-4 \
						 max-sm:pt-4",
					)
					.child(
						Status(self.notice.get())
							.class("text-sm font-semibold text-[var(--admin-bad)]")
							.test_id("notice"),
					)
					.child(content),
			);
		let find = self.clone();

		View()
			.class("h-screen flex-col")
			.class(style::PAGE)
			.child(self.top_bar())
			.child(
				View()
					.class("relative min-h-0 flex-1 flex-row overflow-hidden")
					.child(rail(viewer, &section))
					.child(main)
					.children({
						self.page.with(|page| {
							let drawers = page.drawers();
							(!drawers.is_empty()).then(|| workings::aside(drawers))
						})
					}),
			)
			.child(tab_bar(
				&section,
				viewer,
				search_open().on_press(move || find.find()),
			))
			.children(
				self.page
					.with(|page| {
						matches!(page, Page::Project(_) | Page::Person(_) | Page::Insight(_))
					})
					.then(|| self.confirm.view()),
			)
	}

	fn top_bar(&self) -> Element {
		let viewer = &self.model.viewer;
		let section = self.page.with(Page::section);

		View()
			.class(
				"relative h-14 shrink-0 flex-row items-center gap-3 border-b \
				 border-[var(--admin-line)] bg-[var(--admin-surface)] px-4 max-sm:gap-1.5 \
				 max-sm:px-3",
			)
			.role(Role::Banner)
			.test_id("top-bar")
			.child(brand(viewer))
			.child(components::env_pill(viewer.environment))
			.children(viewer.simulated.then(components::simulated_word))
			.children(switcher(viewer, &section))
			.child(self.search_field())
			.child(View().class("flex-1"))
			.child(pulse_of(self.tail.pulse().as_ref()))
			.child(self.workings_switch())
			.child(self.theme_button())
			.child(self.account_menu())
			.children(self.search_results())
	}

	fn search_field(&self) -> Element {
		let query = self.query;
		let go_first = {
			let viewer = self.model.viewer.clone();
			move |_: &NativeEvent| {
				if let Some(first) =
					matching(&search_targets(&viewer), &query.get_untracked()).first()
				{
					crate::go(&first.href);
				}
				EventOutcome::Handled
			}
		};

		View()
			.class(
				"max-w-[360px] flex-1 flex-row items-center gap-2 rounded-lg border \
				 border-[var(--admin-line)] bg-[var(--admin-sunken)] px-2.5 py-1",
			)
			.class(if self.search_open.get() {
				""
			} else {
				"max-lg:hidden"
			})
			.child(icon(ICON_SEARCH).class("text-[var(--admin-ink-3)]"))
			.child(
				TextInput()
					.placeholder("Find screens, projects, lessons…")
					.aria_label("Find screens, projects, and lessons")
					.class("min-w-0 flex-1 bg-transparent py-0.5 text-sm text-[var(--admin-ink)]")
					.test_id("search")
					.value(query.get())
					.focus_on(self.search)
					.on_change(move |text| query.set(text.to_owned()))
					.on(NativeEventSlot::Submit, go_first),
			)
			.child(Text("⌘K").class(
				"rounded border border-[var(--admin-line-strong)] px-1.5 text-xs \
				 text-[var(--admin-ink-3)]",
			))
	}

	/// The places that match what the person typed, under the search field.
	fn search_results(&self) -> Option<Element> {
		let query = self.query.get();

		if query.trim().is_empty() {
			return None;
		}

		let found = matching(&search_targets(&self.model.viewer), &query);
		let mut results = View()
			.class(
				"absolute left-1/2 top-[52px] w-[360px] -translate-x-1/2 flex-col rounded-xl \
				 border border-[var(--admin-line)] bg-[var(--admin-surface)] p-1.5 text-sm \
				 max-sm:left-4 max-sm:right-4 max-sm:w-auto max-sm:translate-x-0",
			)
			.role(Role::ListBox)
			.aria_label("Screens that match")
			.test_id("search-results");

		if found.is_empty() {
			results = results.child(
				Text(
					"Nothing matches. Analytics screens and saved insights arrive with Analytics.",
				)
				.class("px-2.5 py-2 text-[var(--admin-ink-3)]"),
			);
		}

		for target in found {
			results = results.child(
				Pressable(target.label.clone())
					.href(target.href)
					.role(Role::Option)
					.class("rounded-lg px-2.5 py-2 cursor-pointer hover:bg-[var(--admin-sunken)]")
					.child(Text(target.label).class("text-[var(--admin-ink)]")),
			);
		}

		Some(results)
	}

	/// The Show workings switch: the track and the words are one button, so
	/// the words can go on a phone and the switch still reads "Show
	/// workings" to a screen reader.
	fn workings_switch(&self) -> Element {
		let on = self.workings.mode.get();
		let shell = self.clone();

		Pressable("Show workings")
			.aria_pressed(on)
			.aria_label("Show workings (Shift+W)")
			.class(
				"flex-row items-center gap-2 rounded-lg px-1.5 py-1 cursor-pointer \
				 hover:bg-[var(--admin-sunken)]",
			)
			.test_id("workings-mode")
			.on_press(move || shell.toggle_workings())
			.child(
				View()
					.class("h-[18px] w-[30px] rounded-full relative")
					.class(if on {
						"bg-[var(--admin-mint)]"
					} else {
						"bg-[var(--admin-line-strong)]"
					})
					.test_id("workings-track")
					.aria_hidden(true)
					.child(
						View()
							.class(
								"absolute left-0.5 top-0.5 h-3.5 w-3.5 rounded-full \
								 bg-[var(--admin-surface)] transition-transform duration-200",
							)
							.class(if on { "translate-x-3" } else { "" })
							.test_id("workings-knob"),
					),
			)
			.child(
				Text("Show workings")
					.class("text-[13px] text-[var(--admin-ink-2)] max-sm:hidden")
					.aria_hidden(true),
			)
	}

	fn theme_button(&self) -> Element {
		let shell = self.clone();

		Button("◐")
			.aria_label("Switch between light and dark")
			.class(
				"h-8 w-8 items-center justify-center rounded-lg border border-[var(--admin-line)] \
				 bg-[var(--admin-surface)] text-base text-[var(--admin-ink)] cursor-pointer \
				 hover:bg-[var(--admin-sunken)]",
			)
			.test_id("theme")
			.on_press(move || shell.toggle_theme())
	}

	/// The avatar, which opens who is signed in and Sign out.
	fn account_menu(&self) -> Element {
		let viewer = &self.model.viewer;
		let menu_open = self.menu_open;
		let leave = {
			let shell = self.clone();
			move || shell.leave()
		};
		let menu = Menu::new("account", menu_open.get())
			.button(
				Button(viewer.initials())
					.aria_label(format!("Signed in as {}", viewer.who))
					.class(
						"h-[30px] w-[30px] items-center justify-center rounded-full border \
						 border-[var(--admin-line-strong)] bg-[var(--admin-sunken)] text-xs \
						 font-semibold text-[var(--admin-ink)] cursor-pointer",
					)
					.test_id("avatar"),
			)
			.item("Sign out", leave)
			.content(
				View()
					.class(components::Kind::Secondary.class_name())
					.class("w-full")
					.child(Text("Sign out")),
			)
			.placement(Placement::BelowEnd)
			.on_toggle(move |open| menu_open.set(open));

		// The menu names its parts by id; the admin's tests know them by test
		// id, and the list opens with who is signed in.
		let mut menu = Element::from(menu);

		if let Some(list) = menu.children.get_mut(1) {
			let mut panel = std::mem::replace(list, View())
				.test_id("avatar-menu")
				.class(
					"w-64 flex-col gap-2 rounded-xl border border-[var(--admin-line)] \
					 bg-[var(--admin-surface)] p-3",
				);

			if let Some(item) = panel.children.last_mut() {
				*item = std::mem::replace(item, View()).test_id("sign-out");
			}

			panel.children.insert(
				0,
				View()
					.class("flex-col gap-0.5")
					.aria_hidden(true)
					.child(Text("Signed in as").class(style::LABEL))
					.child(
						Text(viewer.who.clone())
							.class("text-sm font-semibold text-[var(--admin-ink)]"),
					),
			);
			*list = panel;
		}

		menu
	}
}

/// The stylesheet of a page this crate builds: the faces, the colour
/// tokens, and the little the class compiler can't say.
#[must_use]
pub fn stylesheet() -> String {
	format!(
		"@font-face{{font-family:\"Skribble Sketch\";src:url({BASE}/assets/sketch.woff2) \
		 format(\"woff2\");font-display:swap}}@font-face{{font-family:\"Skribble \
		 Mono\";src:url({BASE}/assets/mono.woff2) \
		 format(\"woff2\");font-display:swap}}{tokens}html,body{{margin:0;height:100%;background:\
		 var(--admin-paper)}}body{{font-family:var(--admin-font-sans);-webkit-font-smoothing:\
		 antialiased}}:focus-visible{{outline:2px solid \
		 var(--admin-ember);outline-offset:2px}}[data-testid=top-bar]{{z-index:\
		 30}}[data-testid=search-results]{{z-index:45}}@media \
		 (prefers-reduced-motion:reduce){{*{{animation:none!important;transition:none!\
		 important}}}}",
		tokens = style::tokens_css()
	)
}

/// What only a browser can do. On the server, each is nothing.
/// Remembers a preference for the signed-in admin, and lets it go.
pub(crate) fn preference(body: &str) {
	browser::post(&format!("{BASE}/api/preferences"), body);
}

mod browser {
	/// Whether the device prefers dark.
	pub(super) fn prefers_dark() -> bool {
		// `hydrate` is also on for a host `--all-features` build, which has
		// no window. These calls exist only in the browser build.
		#[cfg(all(feature = "hydrate", target_arch = "wasm32"))]
		{
			web_sys::window()
				.and_then(|window| {
					window
						.match_media("(prefers-color-scheme: dark)")
						.ok()
						.flatten()
				})
				.is_some_and(|list| list.matches())
		}
		#[cfg(not(all(feature = "hydrate", target_arch = "wasm32")))]
		false
	}

	/// Sets a cookie on this page's site.
	pub(super) fn set_cookie(cookie: &str) {
		#[cfg(all(feature = "hydrate", target_arch = "wasm32"))]
		{
			use wasm_bindgen::JsCast as _;

			if let Some(document) = web_sys::window()
				.and_then(|window| window.document())
				.and_then(|document| document.dyn_into::<web_sys::HtmlDocument>().ok())
			{
				let _ = document.set_cookie(cookie);
			}
		}
		#[cfg(not(all(feature = "hydrate", target_arch = "wasm32")))]
		let _ = cookie;
	}

	/// Sends `body`, as JSON, to `path` on this site, and lets it go: a
	/// preference that doesn't save is asked for again next time.
	pub(super) fn post(path: &str, body: &str) {
		#[cfg(all(feature = "hydrate", target_arch = "wasm32"))]
		{
			let init = web_sys::RequestInit::new();
			init.set_method("POST");
			init.set_credentials(web_sys::RequestCredentials::SameOrigin);
			init.set_body(&wasm_bindgen::JsValue::from_str(body));
			let Ok(request) = web_sys::Request::new_with_str_and_init(path, &init) else {
				return;
			};
			let _ = request.headers().set("content-type", "application/json");
			let Some(window) = web_sys::window() else {
				return;
			};
			let sent = wasm_bindgen_futures::JsFuture::from(window.fetch_with_request(&request));
			crate::spawn(async move {
				let _ = sent.await;
			});
		}
		#[cfg(not(all(feature = "hydrate", target_arch = "wasm32")))]
		let _ = (path, body);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn viewer(projects: &[&str]) -> Viewer {
		Viewer {
			app_name: "Notes".to_owned(),
			environment: crate::Environment::Production,
			simulated: false,
			who: "ada.lovelace@example.com".to_owned(),
			workings_mode: false,
			appearance: None,
			projects: projects.iter().map(|&project| project.to_owned()).collect(),
			progress: (1, 9),
			pulse: None,
		}
	}

	#[test]
	fn search_finds_screens_and_projects_by_any_part_of_their_name() {
		let targets = search_targets(&viewer(&["notes-production"]));
		let labels = |query: &str| {
			matching(&targets, query)
				.into_iter()
				.map(|target| target.label)
				.collect::<Vec<_>>()
		};

		assert_eq!(labels("GLOSS"), ["Glossary"]);
		assert_eq!(
			labels("production"),
			[
				"notes-production",
				"notes-production health",
				"notes-production operations"
			]
		);
		assert_eq!(
			labels("  "),
			Vec::<String>::new(),
			"nothing typed, nothing shown"
		);
		assert_eq!(labels("zebra"), Vec::<String>::new());
		assert_eq!(
			matching(&targets, "trends")[0].href,
			format!("{BASE}/analytics/trends")
		);
	}

	#[test]
	fn every_class_the_shell_uses_compiles() {
		let owner = leptos::prelude::Owner::new();
		owner.with(|| {
			leptoskit::query::QueryClient::new().provide();
			let shell = Shell::new(Model {
				viewer: Viewer {
					pulse: Some(crate::model::Pulse {
						per_second: 1.5,
						per_minute: 3,
						seconds: vec![0, 2, 1],
						live: true,
						reason: None,
						latest: 7,
					}),
					..viewer(&["notes-production", "notes-staging"])
				},
				page: Page::Glossary,
				open: None,
			});
			// Search with and without matches, and the account menu open.
			for query in ["", "trend", "zebra"] {
				shell.query.set(query.to_owned());
				shell.menu_open.set(!query.is_empty());
				let frame = shell.frame(crate::pages::content(&shell.model.page));
				assert!(frame.validate().is_ok(), "{query}");
			}
		});
	}

	#[test]
	fn initials_come_from_the_name_before_the_at() {
		assert_eq!(viewer(&[]).initials(), "AL");
	}
}
