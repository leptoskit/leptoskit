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
use leptoskit::prelude::*;
pub(crate) use leptoskit_admin_pages::model::Section;
pub(crate) use leptoskit_admin_pages::model::Viewer;
use leptoskit_admin_pages::shell::rail;
use leptoskit_admin_pages::shell::tab_bar;

use crate::BASE;
use crate::components;
use crate::style;
use crate::workings;
use crate::workings::Drawer;

/// One screen: its title, rail place, content, and drawers.
pub(crate) struct Screen {
	pub title: String,
	pub section: Section,
	pub content: Element,
	pub drawers: Vec<Drawer>,
	/// The drawer open when the page loads.
	pub open: Option<String>,
}

fn top_bar(viewer: &Viewer, section: &Section) -> Element {
	let search = View()
		.class(
			"max-w-[360px] flex-1 flex-row items-center gap-2 rounded-lg border \
			 border-[var(--admin-line)] bg-[var(--admin-sunken)] px-2.5 py-1 max-lg:hidden",
		)
		.child(
			leptoskit_admin_pages::shell::icon(leptoskit_admin_pages::shell::ICON_SEARCH)
				.class("text-[var(--admin-ink-3)]"),
		)
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
			 border-[var(--admin-line)] bg-[var(--admin-surface)] px-4 max-sm:gap-1.5 max-sm:px-3",
		)
		.role(Role::Banner)
		.test_id("top-bar")
		.child(leptoskit_admin_pages::shell::brand(viewer))
		.child(components::env_pill(viewer.environment))
		.children(viewer.simulated.then(components::simulated_word))
		.children(leptoskit_admin_pages::shell::switcher(viewer, section))
		.child(search)
		.child(View().class("flex-1"))
		.child(leptoskit_admin_pages::shell::pulse(viewer))
		.child(switch)
		.child(theme)
		.child(avatar)
		.child(menu)
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
		.child(tab_bar(
			&screen.section,
			viewer,
			leptoskit_admin_pages::shell::search_open(),
		))
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

	// The drawers: the Ask station's tabs, the lines a builder change
	// washed and the station it changed, stations collapsing to their
	// headers on phones, and Copy's row staying put while code scrolls
	// sideways. Then the live tail's filter and typed badge. A raw string,
	// so rustfmt leaves its quotes alone.
	css.push_str(
		r#"[data-changed]{background:rgba(94,242,194,.22)!important;box-shadow:inset 3px 0 #5ef2c2}
[data-changed-station]>[data-testid=station-dot]{background:#5ef2c2!important}
[data-changed-station]>[data-testid=station-head]::after{content:"changed";align-self:center;padding:0 6px;border-radius:999px;background:#5ef2c2;color:#0f0b16;font-size:11px;font-weight:600;line-height:16px}
[data-testid=code-head]{position:sticky;left:0}
[data-testid=station-toggle]::after{content:"\25BE";position:absolute;right:6px;top:50%;transform:translateY(-50%);color:#9d93a8;font-size:14px;transition:transform .15s}
[data-testid=station-toggle][aria-expanded=true]::after{transform:translateY(-50%) rotate(180deg)}
@media (max-width:639px){[data-testid^="station:"]:not([data-open])>[data-testid=station-body]{display:none!important}}
[data-testid^="ask-panel:"]{display:none!important}
html:not([data-admin-ask]) [data-testid="ask-panel:rust"],html[data-admin-ask=rust] [data-testid="ask-panel:rust"],html[data-admin-ask=sql] [data-testid="ask-panel:sql"],html[data-admin-ask=http] [data-testid="ask-panel:http"]{display:flex!important}
[data-testid^="ask-tab:"][aria-selected=true]{border-color:#5ef2c2!important;color:#5ef2c2!important}
[data-testid=live-event][data-filtered]{display:none!important}
[data-testid=struct-badge][aria-hidden=true]{display:none!important}
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
