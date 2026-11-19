//! A builder changes in place.
//!
//! Every control on a builder is a link to the same page with one parameter
//! changed. In the browser, pressing one fetches that address's page as
//! JSON and puts it in place of this one: focus, the open drawer and its
//! scroll stay where they were, the address and the title follow, and the
//! drawer marks the lines of code the change changed. The browser's Back
//! and Forward come back the same way. Without the page's Wasm, the links
//! are plain links.
//!
//! A builder, a dashboard, and a page waiting for its first event also read
//! themselves again when the analytics stream says there are newer events.

use std::collections::HashSet;

use leptos::context::use_context;
#[cfg(feature = "hydrate")]
use leptos::prelude::Effect;
#[cfg(feature = "hydrate")]
use leptos::prelude::GetValue as _;
#[cfg(feature = "hydrate")]
use leptos::prelude::SetValue as _;
use leptos::prelude::StoredValue;
use leptos::prelude::WithValue as _;
use leptoskit::native::EventOutcome;
use leptoskit::native::NativeEventSlot;
use leptoskit::prelude::*;

use crate::components::Line;
use crate::model::Page;
use crate::workings::Body;
use crate::workings::Drawer;
use crate::workings::Workings;

/// A builder page that changes in place. The builder provides one as
/// context; the links and menus it draws read it.
#[derive(Clone, Copy)]
pub struct Swap {
	/// The page the shell frames.
	page: RwSignal<Page>,
	workings: Workings,
	/// The app's name, for the document's title.
	app: StoredValue<String>,
	/// Whether a page is on its way.
	busy: RwSignal<bool>,
	/// The open menu's id.
	menu: RwSignal<Option<String>>,
}

impl Swap {
	/// Swaps `page` in place, with `workings` marking what changed.
	#[must_use]
	pub(crate) fn new(page: RwSignal<Page>, workings: Workings, app: &str) -> Self {
		let swap = Self {
			page,
			workings,
			app: StoredValue::new(app.to_owned()),
			busy: RwSignal::new(false),
			menu: RwSignal::new(None),
		};

		// Back and Forward bring back the page the address names.
		#[cfg(feature = "hydrate")]
		{
			let handle = leptos::prelude::window_event_listener(leptos::ev::popstate, move |_| {
				if let Some(href) =
					web_sys::window().and_then(|window| window.location().href().ok())
				{
					swap.go(href, false);
				}
			});
			leptos::prelude::on_cleanup(move || handle.remove());
			swap.reread();
		}

		swap
	}

	/// Reads the page again when the stream has newer events, at most every
	/// ten seconds, unless the person is in the middle of something: an
	/// open menu or row, the confirm sheet, or a field.
	#[cfg(feature = "hydrate")]
	fn reread(self) {
		const EVERY_MS: f64 = 10_000.0;

		let Some(tail) = crate::analytics::tail::Tail::current() else {
			return;
		};
		let now = || {
			web_sys::window()
				.and_then(|window| window.performance())
				.map_or(0.0, |performance| performance.now())
		};
		let confirm = crate::confirm::Confirm::current();
		let seen = StoredValue::new(tail.latest_untracked());
		let read_at = StoredValue::new(now());

		Effect::new(move |_| {
			let _ = tail.beats();
			let Some(latest) = tail.latest_untracked() else {
				return;
			};
			let Some(before) = seen.get_value() else {
				seen.set_value(Some(latest));
				return;
			};

			if latest <= before
				|| !self.page.with_untracked(rereads)
				|| self.busy.get_untracked()
				|| now() - read_at.get_value() < EVERY_MS
			{
				return;
			}

			let Some(document) = web_sys::window().and_then(|window| window.document()) else {
				return;
			};
			let busy = self.menu.with_untracked(Option::is_some)
				|| confirm.is_some_and(|confirm| confirm.asking_untracked())
				|| document
					.query_selector(r#"[data-testid="how-toggle"][aria-expanded="true"]"#)
					.ok()
					.flatten()
					.is_some()
				|| document.active_element().is_some_and(|element| {
					matches!(element.tag_name().as_str(), "INPUT" | "TEXTAREA")
				});

			if busy {
				return;
			}

			seen.set_value(Some(latest));
			read_at.set_value(now());

			if let Some(href) = web_sys::window().and_then(|window| window.location().href().ok()) {
				self.go(href, false);
			}
		});
	}

	/// The builder's swap, when the page runs as this crate's.
	#[must_use]
	pub fn current() -> Option<Self> {
		use_context::<Self>()
	}

	/// The page's address, without its query.
	fn path(&self) -> Option<String> {
		self.page.with_untracked(|page| {
			match page {
				Page::Insight(insight) => Some(insight.path.clone()),
				Page::Web(_) => Some(crate::pages::web_path()),
				_ => None,
			}
		})
	}

	/// Whether `href` is this page with another query.
	fn same_page(&self, href: &str) -> bool {
		let path = href.split(['?', '#']).next().unwrap_or_default();
		self.path().is_some_and(|own| own == path)
	}

	/// Whether menu `id` is open. Reading it rebuilds the screen when that
	/// changes.
	#[must_use]
	pub fn is_menu_open(&self, id: &str) -> bool {
		self.menu.with(|open| open.as_deref() == Some(id))
	}

	/// Opens menu `id`, closing any other, or closes it.
	pub fn set_menu(&self, id: &str, open: bool) {
		self.menu.set(open.then(|| id.to_owned()));
	}

	/// Fetches `href`'s page and puts it in place. `chosen` is a change the
	/// person made: the address is pushed, and the open drawer shows what
	/// changed. A dashboard or a notice reading itself again is put in place
	/// too; anything else is loaded the usual way.
	pub fn go(self, href: String, chosen: bool) {
		if self.busy.get_untracked() {
			return;
		}

		self.busy.set(true);
		self.menu.set(None);

		crate::spawn(async move {
			let answer = crate::get_json(&href, true).await;
			self.busy.set(false);
			let next = answer
				.filter(|(ok, _)| *ok)
				.and_then(|(_, data)| serde_json::from_value::<Page>(data).ok());
			let mut next = match next {
				Some(Page::Insight(next)) => next,
				// The web overview, which changes in place too, or a page that
				// only reads itself again.
				Some(next @ (Page::Web(_) | Page::Dashboard(_) | Page::Notice(_)))
					if matches!(next, Page::Web(_)) || !chosen =>
				{
					let title = format!(
						"{} · {} admin",
						next.title(),
						self.app.with_value(Clone::clone)
					);
					self.page.set(next);
					browser::show(&href, &title, chosen);
					self.workings.announce_change(None, None);
					return;
				}
				_ => {
					crate::go(&href);
					return;
				}
			};

			let tab = self.workings.ask.get_untracked();
			let change = self.page.with_untracked(|page| {
				match page {
					Page::Insight(before) => mark(&before.drawer, &mut next.drawer, &tab),
					_ => None,
				}
			});
			let drawer = next.drawer.id.clone();
			let title = format!(
				"{} · {} admin",
				next.name,
				self.app.with_value(Clone::clone)
			);
			self.page.set(Page::Insight(next));
			browser::show(&href, &title, chosen);

			match change {
				Some(change)
					if chosen
						&& self
							.workings
							.open
							.with_untracked(|open| open.as_deref() == Some(drawer.as_str())) =>
				{
					self.workings
						.open_station(&format!("{drawer}|{}", change.station.to_lowercase()));
					self.workings
						.announce_change(Some(change.status()), change.line);
				}
				_ => self.workings.announce_change(None, None),
			}
		});
	}
}

/// Whether `page` reads itself again when newer events arrive.
#[cfg(feature = "hydrate")]
fn rereads(page: &Page) -> bool {
	match page {
		Page::Insight(_) | Page::Web(_) | Page::Dashboard(_) => true,
		Page::Notice(notice) => {
			matches!(notice.why, crate::model::analytics::Why::Listening { .. })
		}
		_ => false,
	}
}

/// `link`, which goes to `href`, changing the page in place when `href` is
/// the builder's own address.
#[must_use]
pub fn link(link: Element, href: String) -> Element {
	let Some(swap) = Swap::current().filter(|swap| swap.same_page(&href)) else {
		return link;
	};

	link.on(NativeEventSlot::Press, move |_| {
		swap.go(href.clone(), true);
		EventOutcome::Handled
	})
}

/// What a change changed, as the drawer shows it.
#[derive(Debug, PartialEq, Eq)]
struct Change {
	/// The first station with a changed line.
	station: String,
	/// That line's text, when the person can see it: a line in an Ask tab
	/// nobody is looking at can't be shown, though its station can.
	line: Option<String>,
}

impl Change {
	/// `Ask changed: interval: Interval::Week`.
	fn status(&self) -> String {
		let Some(line) = &self.line else {
			return format!("{} changed", self.station);
		};
		let text = line.trim();
		let text = text.strip_suffix([',', ';']).unwrap_or(text);
		let text = if text.chars().count() > 48 {
			format!("{}…", text.chars().take(47).collect::<String>())
		} else {
			text.to_owned()
		};

		format!("{} changed: {text}", self.station)
	}
}

/// Marks the lines of `after` that `before` didn't have, block by block,
/// and says where the first one is. `tab` is the Ask tab showing.
fn mark(before: &Drawer, after: &mut Drawer, tab: &str) -> Option<Change> {
	let mut first: Option<Change> = None;

	for station in &mut after.stations {
		let Some(old) = before.stations.iter().find(|old| old.name == station.name) else {
			continue;
		};
		let mut shown = None;

		match (&old.body, &mut station.body) {
			(Body::Code(old, _), Body::Code(lines, _)) => {
				shown = mark_lines(old, lines);
			}
			(Body::Tabs(old), Body::Tabs(tabs)) => {
				for each in tabs {
					let Some(old) = old.iter().find(|old| old.name == each.name) else {
						continue;
					};
					let changed = mark_lines(&old.lines, &mut each.lines);

					if each.name.to_lowercase() == tab {
						shown = shown.or(changed);
					}
				}
			}
			_ => {}
		}

		match &mut first {
			None if station.changed() => {
				first = Some(Change {
					station: station.name.clone(),
					line: shown,
				});
			}
			// A shown line beats a station whose lines are all hidden.
			Some(change) if change.line.is_none() && shown.is_some() => {
				*change = Change {
					station: station.name.clone(),
					line: shown,
				};
			}
			_ => {}
		}
	}

	first
}

/// Marks each line of `after` that isn't a line of `before`, and gives the
/// first one's text.
fn mark_lines(before: &[Line], after: &mut [Line]) -> Option<String> {
	let seen: HashSet<&str> = before.iter().map(|line| line.text.as_str()).collect();
	let mut first = None;

	for line in after {
		line.changed = !line.text.trim().is_empty() && !seen.contains(line.text.as_str());

		if line.changed && first.is_none() {
			first = Some(line.text.clone());
		}
	}

	first
}

mod browser {
	/// Shows `href` in the address bar, pushed as a new entry when `push`,
	/// and `title` as the document's.
	#[cfg(feature = "hydrate")]
	pub(super) fn show(href: &str, title: &str, push: bool) {
		let Some(window) = web_sys::window() else {
			return;
		};

		if let Some(document) = window.document() {
			document.set_title(title);
		}

		if push && let Ok(history) = window.history() {
			// The open drawer's link stays on the address.
			let hash = window.location().hash().unwrap_or_default();
			let _ = history.push_state_with_url(
				&wasm_bindgen::JsValue::NULL,
				"",
				Some(&format!("{href}{hash}")),
			);
		}
	}

	#[cfg(not(feature = "hydrate"))]
	pub(super) fn show(href: &str, title: &str, push: bool) {
		let _ = (href, title, push);
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::workings::Station;
	use crate::workings::Tab;

	fn drawer(ask: &[&str], sql: &[&str], store: &[&str]) -> Drawer {
		let lines = |texts: &[&str]| texts.iter().map(|text| Line::plain(*text)).collect();
		Drawer {
			id: "trends".to_owned(),
			title: String::new(),
			stations: vec![
				Station::new("Store", "", "", Body::Code(lines(store), "text".to_owned())),
				Station::new(
					"Ask",
					"",
					"",
					Body::Tabs(vec![
						Tab {
							name: "Rust".to_owned(),
							lines: lines(ask),
							copy_as: "Rust".to_owned(),
						},
						Tab {
							name: "SQL".to_owned(),
							lines: lines(sql),
							copy_as: "SQL".to_owned(),
						},
					]),
				),
			],
		}
	}

	#[test]
	fn a_change_marks_the_new_lines_and_names_the_one_showing() {
		let before = drawer(
			&["Trends {", "    interval: Interval::Day,", "}"],
			&["select day"],
			&["events"],
		);
		let mut after = drawer(
			&["Trends {", "    interval: Interval::Week,", "}"],
			&["select week"],
			&["events"],
		);
		let change = mark(&before, &mut after, "rust").unwrap();

		assert_eq!(change.status(), "Ask changed: interval: Interval::Week");
		let marked: Vec<&str> = after
			.stations
			.iter()
			.flat_map(Station::lines)
			.filter(|line| line.changed)
			.map(|line| line.text.as_str())
			.collect();
		assert_eq!(marked, ["    interval: Interval::Week,", "select week"]);
		assert!(!after.stations[0].changed(), "Store didn't change");
	}

	#[test]
	fn a_change_only_a_hidden_tab_shows_names_its_station() {
		let before = drawer(&["a"], &["select day"], &["events"]);
		let mut after = drawer(&["a"], &["select week"], &["events"]);

		let change = mark(&before, &mut after, "rust").unwrap();
		assert_eq!(change.line, None);
		assert_eq!(change.status(), "Ask changed");
	}

	#[test]
	fn nothing_changed_says_nothing() {
		let before = drawer(&["a"], &["b"], &["c"]);
		let mut after = before.clone();

		assert_eq!(mark(&before, &mut after, "rust"), None);
	}

	#[test]
	fn a_long_line_is_cut_short() {
		let change = Change {
			station: "Ask".to_owned(),
			line: Some(format!("    {};", "x".repeat(60))),
		};

		assert_eq!(change.status(), format!("Ask changed: {}…", "x".repeat(47)));
	}
}
