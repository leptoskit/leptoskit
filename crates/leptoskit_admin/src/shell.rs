//! The frame around every screen: top bar, rail, content, the Workings
//! drawer, and on phones a bottom tab bar.
//!
//! The shell is a full-height flex column, because the class compiler has
//! no `fixed` or `sticky`: pinned chrome comes from the layout. The few
//! states the script toggles (which drawer is open, Workings mode, the
//! avatar menu) are attributes on the root element, and a small stylesheet
//! here turns them into visibility, the way `dark:` follows
//! `data-lk-appearance`.

use std::fmt::Write as _;

use leptos::prelude::RenderHtml as _;
use leptoskit::native::Role;
use leptoskit::native::vector::Paint;
use leptoskit::native::vector::PathData;
use leptoskit::native::vector::Stroke;
use leptoskit::native::vector::VectorPath;
use leptoskit::native::vector::ViewBox;
use leptoskit::prelude::*;

use crate::BASE;
use crate::Environment;
use crate::components;
use crate::style;
use crate::workings;
use crate::workings::Drawer;

/// Who is looking, and how they like the admin.
#[derive(Clone, Debug)]
pub(crate) struct Viewer {
	/// The app's name: `Notes`.
	pub app_name: String,
	/// Which environment this admin acts on.
	pub environment: Environment,
	/// The signed-in admin's email, or their name at the main site.
	pub who: String,
	/// Whether mint tabs show their sources.
	pub workings_mode: bool,
	/// `light` or `dark` when the person chose one.
	pub appearance: Option<String>,
	/// Registered projects, for the rail and the switcher.
	pub projects: Vec<String>,
	/// Lessons finished and lessons there are.
	pub progress: (usize, usize),
	/// How fast analytics events arrive, when the admin is connected.
	pub pulse: Option<crate::analytics::live::Pulse>,
}

impl Viewer {
	fn initials(&self) -> String {
		let name = self.who.split('@').next().unwrap_or(&self.who);
		let mut initials: String = name
			.split(['.', '_', '-', ' '])
			.filter_map(|part| part.chars().next())
			.take(2)
			.collect();

		if initials.is_empty() {
			initials.push('?');
		}

		initials.to_uppercase()
	}
}

/// Where a screen sits in the rail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Section {
	Overview,
	Live,
	Trends,
	Funnels,
	Retention,
	Library,
	People,
	Projects,
	Project(String),
	ProjectHealth(String),
	ProjectOperations(String),
	Learn,
	Glossary,
}

/// One screen: its title, rail place, content, and drawers.
pub(crate) struct Screen {
	pub title: String,
	pub section: Section,
	pub content: Element,
	pub drawers: Vec<Drawer>,
	/// The drawer open when the page loads.
	pub open: Option<String>,
}

/// An icon, drawn as strokes in the current text colour.
fn icon(path: &str) -> Element {
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
const ICON_SEARCH: &str = "M11 18a7 7 0 1 0 0-14 7 7 0 0 0 0 14z|M20 20l-4-4";
const ICON_MOON: &str = "M20 14.5A8 8 0 0 1 9.5 4a8 8 0 1 0 10.5 10.5z";

/// The leptoskit mark: two ember boxes and the mint piece that arrives over
/// the air, tilted −12°.
pub(crate) fn mark(size: f32) -> Element {
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

#[expect(clippy::too_many_lines, reason = "the top bar, left to right")]
fn top_bar(viewer: &Viewer, section: &Section) -> Element {
	let brand = Pressable(format!("{} admin home", viewer.app_name))
		.href(format!("{BASE}/"))
		.class("flex-row items-center gap-2 cursor-pointer")
		.child(mark(24.0))
		.child(
			Text(viewer.app_name.clone())
				.class("text-[15px] font-semibold text-[var(--admin-ink)]"),
		)
		.child(
			Text("admin").class("text-[15px] font-medium text-[var(--admin-ink-3)] max-sm:hidden"),
		);

	let search = View()
		.class(
			"max-w-[360px] flex-1 flex-row items-center gap-2 rounded-lg border \
			 border-[var(--admin-line)] bg-[var(--admin-sunken)] px-2.5 py-1 max-lg:hidden",
		)
		.child(icon(ICON_SEARCH).class("text-[var(--admin-ink-3)]"))
		.child(
			TextInput()
				.placeholder("Find screens, projects, lessons…")
				.aria_label("Find screens, projects, and lessons")
				.class("min-w-0 flex-1 bg-transparent py-0.5 text-sm text-[var(--admin-ink)]")
				.test_id("search"),
		)
		.child(Text("⌘K").class(
			"rounded border border-[var(--admin-line-strong)] px-1.5 text-xs \
			 text-[var(--admin-ink-3)]",
		));

	let mut switcher = None;

	if let Section::Project(project)
	| Section::ProjectHealth(project)
	| Section::ProjectOperations(project) = section
		&& viewer.projects.len() > 1
	{
		switcher = Some(
			Pressable("Switch project")
				.href(format!("{BASE}/infra/projects"))
				.class(style::CHIP)
				.class("cursor-pointer max-md:hidden")
				.child(Text(format!("{project} ▾"))),
		);
	}

	let pulse = match &viewer.pulse {
		None => {
			Pressable("Live events: analytics isn't connected yet")
				.href(format!("{BASE}/analytics/live"))
				.class(
					"flex-row items-center gap-2 rounded-full border border-[var(--admin-line)] \
					 px-2.5 py-1 cursor-pointer hover:bg-[var(--admin-sunken)] max-sm:hidden",
				)
				.test_id("live-pulse")
				.child(View().class("h-[7px] w-[7px] rounded-full bg-[var(--admin-ink-3)]"))
				.child(
					Text("Not listening").class("text-xs text-[var(--admin-ink-2)] max-md:hidden"),
				)
		}
		Some(pulse) => {
			#[expect(clippy::cast_precision_loss, reason = "counts for drawing")]
			let seconds: Vec<f64> = pulse.seconds.iter().map(|count| *count as f64).collect();
			let live = pulse.status == crate::analytics::live::Status::Live;
			Pressable(format!("Live events: {:.1} a second", pulse.per_second))
				.href(format!("{BASE}/analytics/live"))
				.class(
					"flex-row items-center gap-2 rounded-full border border-[var(--admin-line)] \
					 px-2.5 py-1 cursor-pointer hover:bg-[var(--admin-sunken)] max-sm:hidden",
				)
				.test_id("live-pulse")
				.child(
					crate::analytics::draw::sparkline(&seconds, "h-4 w-14 max-md:hidden")
						.test_id("pulse-spark"),
				)
				.child(
					View()
						.class("h-[7px] w-[7px] rounded-full")
						.class(if live {
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
	};

	// The track and the words are one button, so the words can go on a
	// phone and the switch still reads "Show workings" to a screen reader.
	let switch = Pressable("Show workings")
		.aria_pressed(viewer.workings_mode)
		.aria_label("Show workings (Shift+W)")
		.class(
			"flex-row items-center gap-2 rounded-lg px-1.5 py-1 cursor-pointer \
			 hover:bg-[var(--admin-sunken)]",
		)
		.test_id("workings-mode")
		.child(
			View()
				.class("h-[18px] w-[30px] rounded-full bg-[var(--admin-line-strong)] relative")
				.test_id("workings-track")
				.aria_hidden(true)
				.child(
					View()
						.class(
							"absolute left-0.5 top-0.5 h-3.5 w-3.5 rounded-full \
							 bg-[var(--admin-surface)] transition-transform duration-200",
						)
						.test_id("workings-knob"),
				),
		)
		.child(
			Text("Show workings")
				.class("text-[13px] text-[var(--admin-ink-2)] max-sm:hidden")
				.aria_hidden(true),
		);

	let theme = Button("◐")
		.aria_label("Switch between light and dark")
		.class(
			"h-8 w-8 items-center justify-center rounded-lg border border-[var(--admin-line)] \
			 bg-[var(--admin-surface)] text-base text-[var(--admin-ink)] cursor-pointer \
			 hover:bg-[var(--admin-sunken)]",
		)
		.test_id("theme");
	let _ = ICON_MOON;

	let avatar = Button(viewer.initials())
		.aria_label(format!("Signed in as {}", viewer.who))
		.aria_has_popup(leptoskit::native::HasPopup::Menu)
		.class(
			"h-[30px] w-[30px] items-center justify-center rounded-full border \
			 border-[var(--admin-line-strong)] bg-[var(--admin-sunken)] text-xs font-semibold \
			 text-[var(--admin-ink)] cursor-pointer",
		)
		.test_id("avatar");

	let menu = View()
		.class(
			"absolute right-3 top-[52px] w-64 flex-col gap-2 rounded-xl border \
			 border-[var(--admin-line)] bg-[var(--admin-surface)] p-3",
		)
		.test_id("avatar-menu")
		.role(Role::Menu)
		.child(Text("Signed in as").class(style::LABEL))
		.child(Text(viewer.who.clone()).class("text-sm font-semibold text-[var(--admin-ink)]"))
		.child(
			Button("Sign out")
				.role(Role::MenuItem)
				.class(components::Kind::Secondary.class_name())
				.test_id("sign-out"),
		);

	View()
		.class(
			"relative h-14 shrink-0 flex-row items-center gap-3 border-b \
			 border-[var(--admin-line)] bg-[var(--admin-surface)] px-4 max-sm:gap-2 max-sm:px-3",
		)
		.role(Role::Banner)
		.test_id("top-bar")
		.child(brand)
		.child(components::env_pill(viewer.environment))
		.children(switcher)
		.child(search)
		.child(View().class("flex-1"))
		.child(pulse)
		.child(switch)
		.child(theme)
		.child(avatar)
		.child(menu)
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

fn rail(viewer: &Viewer, section: &Section) -> Element {
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

fn tab_bar(section: &Section, viewer: &Viewer) -> Element {
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
		.child(
			Button("⌘K")
				.aria_label("Find a screen")
				.class("flex-1 text-[11px] font-semibold text-[var(--admin-ink-3)] cursor-pointer")
				.test_id("search-open"),
		)
}

/// The whole screen as a component tree.
pub(crate) fn frame(viewer: &Viewer, screen: Screen) -> Element {
	let content = View()
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
					Status("")
						.class("text-sm font-semibold text-[var(--admin-bad)]")
						.test_id("notice"),
				)
				.child(screen.content),
		);

	let mut body = View()
		.class("relative min-h-0 flex-1 flex-row overflow-hidden")
		.child(rail(viewer, &screen.section))
		.child(content);

	if !screen.drawers.is_empty() {
		body = body.child(workings::aside(&screen.drawers));
	}

	View()
		.class("h-screen flex-col")
		.class(style::PAGE)
		.child(top_bar(viewer, &screen.section))
		.child(body)
		.child(tab_bar(&screen.section, viewer))
}

/// The stylesheet for the states the script toggles, and the fonts.
pub(crate) fn shell_css(drawers: &[Drawer]) -> String {
	let mut css = format!(
		"@font-face{{font-family:\"Skribble Sketch\";src:url({BASE}/assets/sketch.woff2) \
		 format(\"woff2\");font-display:swap}}@font-face{{font-family:\"Skribble \
		 Mono\";src:url({BASE}/assets/mono.woff2) \
		 format(\"woff2\");font-display:swap}}{tokens}html,body{{margin:0;height:100%;background:\
		 var(--admin-paper)}}body{{font-family:var(--admin-font-sans);-webkit-font-smoothing:\
		 antialiased}}:focus-visible{{outline:2px solid \
		 var(--admin-ember);outline-offset:2px}}[data-testid^=\"drawer:\"]{{display:none!\
		 important}}html:not([data-admin-drawer]) \
		 [data-testid=drawer-aside]{{display:none!important}}html:not([data-admin-mode=workings]) \
		 [data-testid=mint-source]{{display:none!important}}html:not([data-admin-mode=workings]) \
		 [data-testid^=\"workings-tab:\"]{{background:transparent!important;border-color:\
		 transparent!important}}html[data-admin-mode=workings] \
		 [data-testid=workings-track]{{background:var(--admin-mint)!\
		 important}}html[data-admin-mode=workings] \
		 [data-testid=workings-knob]{{transform:translateX(12px)}}[data-testid=top-bar]{{z-index:\
		 30}}[data-testid=drawer-aside]{{z-index:20}}[data-testid=avatar-menu]{{z-index:40;\
		 display:none!important}}html[data-admin-menu] \
		 [data-testid=avatar-menu]{{display:flex!important}}[data-testid=confirm]:\
		 not([data-shown]){{display:none!important}}[data-linked]{{background:rgba(94,242,194,.\
		 18)!important;box-shadow:inset 3px 0 \
		 #5ef2c2}}[data-testid=how-toggle][aria-expanded=false]~[data-testid=how-panel]{{display:\
		 none!important}}[data-testid=code]{{overflow-x:auto!\
		 important}}[data-testid^=code-line]{{min-width:\
		 max-content}}[data-testid=code-text]{{white-space:pre!important;flex:none!\
		 important}}[data-testid=how-toggle]::before{{content:\"\\25B8\";display:inline-block;\
		 margin-right:.4em;transition:transform \
		 .15s}}[data-testid=how-toggle][aria-expanded=true]::before{{transform:\
		 rotate(90deg)}}[data-testid=search-results]:empty{{display:none!important}}@media \
		 (min-width:1280px){{html[data-admin-drawer] \
		 [data-testid=rail-label]{{display:none!important}}html[data-admin-drawer] \
		 [data-testid=rail]{{width:4rem!important;padding-left:.5rem!important;padding-right:.\
		 5rem!important}}html[data-admin-drawer] \
		 [data-testid=grid-main]>*{{grid-column:1/-1!important}}html[data-admin-drawer] \
		 [data-testid=grid-tiles]{{grid-template-columns:repeat(2,minmax(0,1fr))!important}}}}@\
		 media (prefers-reduced-motion:reduce){{*{{animation:none!important;transition:none!\
		 important}}}}",
		tokens = style::tokens_css()
	);

	// The analytics drawers: the Ask station's tabs, the lines a builder
	// change washed, and the live tail's filter. A raw string, so rustfmt
	// leaves its quotes alone.
	css.push_str(
		r#"[data-changed]{background:rgba(94,242,194,.22)!important;box-shadow:inset 3px 0 #5ef2c2}
[data-testid^="ask-panel:"]{display:none!important}
html:not([data-admin-ask]) [data-testid="ask-panel:rust"],html[data-admin-ask=rust] [data-testid="ask-panel:rust"],html[data-admin-ask=sql] [data-testid="ask-panel:sql"],html[data-admin-ask=http] [data-testid="ask-panel:http"]{display:flex!important}
html:not([data-admin-ask]) [data-testid="ask-tab:rust"],html[data-admin-ask=rust] [data-testid="ask-tab:rust"],html[data-admin-ask=sql] [data-testid="ask-tab:sql"],html[data-admin-ask=http] [data-testid="ask-tab:http"]{border-color:#5ef2c2!important;color:#5ef2c2!important}
[data-testid=live-event][data-filtered]{display:none!important}
[data-testid=live-event] [data-testid=how-toggle]::before{display:none}
[data-testid=menu-panel]{z-index:10;box-shadow:0 12px 32px -12px rgba(15,11,22,.28)}
[data-testid=menu-toggle][aria-expanded=false]~[data-testid=menu-panel]{display:none!important}"#,
	);

	for drawer in drawers {
		let id = css_string(&drawer.id);
		// A raw string: rustfmt's string wrapping would split the escaped
		// quotes.
		let _ = write!(
			css,
			r#"html[data-admin-drawer="{id}"] [data-testid="drawer:{id}"]{{display:flex!important}}
html[data-admin-drawer="{id}"] [data-testid="card:{id}"]{{border-color:var(--admin-mint)!important;outline:1px solid var(--admin-mint)}}
html[data-admin-drawer="{id}"] [data-testid="workings-tab:{id}"]{{background:var(--admin-mint)!important;border-color:var(--admin-mint)!important;color:#0f0b16!important}}
html[data-admin-drawer="{id}"] [data-testid="workings-tab:{id}"] *{{color:#0f0b16!important;transform:none!important}}"#
		);
	}

	css
}

/// Keeps a value inside a quoted CSS attribute selector.
fn css_string(value: &str) -> String {
	value
		.chars()
		.filter(|character| {
			character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | ':')
		})
		.collect()
}

/// Escapes text for an HTML attribute or `<title>`.
pub(crate) fn escape(text: &str) -> String {
	let mut escaped = String::with_capacity(text.len());

	for character in text.chars() {
		match character {
			'&' => escaped.push_str("&amp;"),
			'<' => escaped.push_str("&lt;"),
			'>' => escaped.push_str("&gt;"),
			'"' => escaped.push_str("&quot;"),
			'\'' => escaped.push_str("&#39;"),
			_ => escaped.push(character),
		}
	}

	escaped
}

/// The root element's attributes: the person's appearance, Workings mode,
/// and the drawer open when the page loads.
pub(crate) fn root_attributes(
	appearance: Option<&str>,
	workings_mode: bool,
	open: Option<&str>,
) -> String {
	let mut attributes = String::new();

	if let Some(appearance @ ("light" | "dark")) = appearance {
		let _ = write!(attributes, " data-lk-appearance=\"{appearance}\"");
	}

	if workings_mode {
		attributes.push_str(" data-admin-mode=\"workings\"");
	}

	if let Some(open) = open {
		let _ = write!(attributes, " data-admin-drawer=\"{}\"", css_string(open));
	}

	attributes
}

/// A whole HTML document around `body`.
pub(crate) fn document(title: &str, root: &str, css: &str, body: Element) -> String {
	let body = leptoskit::web::render(body).to_html();
	format!(
		"<!doctype html><html lang=\"en\"{root}><head><meta charset=\"utf-8\"><meta \
		 name=\"viewport\" content=\"width=device-width, initial-scale=1, \
		 viewport-fit=cover\"><meta name=\"color-scheme\" content=\"light dark\"><meta \
		 name=\"robots\" content=\"noindex\"><title>{title}</title><style>{css}</style><script \
		 type=\"module\" \
		 src=\"{BASE}/assets/admin.mjs\"></script></head><body>{body}</body></html>",
		title = escape(title)
	)
}

/// A signed-in screen as a whole document.
pub(crate) fn page(viewer: &Viewer, screen: Screen) -> String {
	let title = format!("{} · {} admin", screen.title, viewer.app_name);
	let css = shell_css(&screen.drawers);
	let open = screen.open.clone();
	let root = root_attributes(
		viewer.appearance.as_deref(),
		viewer.workings_mode,
		open.as_deref(),
	);
	document(&title, &root, &css, frame(viewer, screen))
}
