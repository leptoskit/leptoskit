//! The Workings drawer: the underside of a card, as five stations joined by
//! a mint rail.
//!
//! Analytics cards read Emit, Send, Store, Ask, Draw; infrastructure
//! actions read Declare, Generate, Plan, Apply, Check. Every station starts
//! with one plain sentence of why, and says so when it can't explain
//! something rather than guessing.

use leptos::context::use_context;
use leptos::prelude::StoredValue;
use leptos::prelude::UpdateValue as _;
use leptos::prelude::WithValue as _;
use leptoskit::native::FocusRequest;
use leptoskit::native::Reveal;
use leptoskit::prelude::*;
use serde::Deserialize;
use serde::Serialize;

use crate::components;
use crate::components::Line;
use crate::components::Tone;
use crate::style;

/// One drawer: the underside of one card.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Drawer {
	/// The card it belongs to, used in `workings-tab:<id>` and
	/// `#workings=<id>`.
	pub id: String,
	/// What the card is, in a few words.
	pub title: String,
	/// The stations, top to bottom.
	pub stations: Vec<Station>,
}

/// One station on the rail.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Station {
	/// The station's name: `Declare`, `Plan`, …
	pub name: String,
	/// Where its content comes from: `leptoskit.toml`, `what OpenTofu will do`.
	pub source: String,
	/// One plain sentence of why.
	pub why: String,
	/// What the station shows.
	pub body: Body,
}

/// What a station shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Body {
	/// Code, with Copy as the given format.
	Code(Vec<Line>, String),
	/// A short checklist, each item with its state.
	Checks(Vec<(Tone, String)>),
	/// Nothing to show, and why: "can't explain" states are honest.
	Missing(String),
	/// The same thing several ways, one tab each, such as the Ask station's
	/// Rust, SQL, and HTTP. The root's `data-admin-ask` picks the tab.
	Tabs(Vec<Tab>),
}

/// One tab of [`Body::Tabs`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tab {
	/// `Rust`, `SQL`, `HTTP`; lowercased, its test identifier.
	pub name: String,
	pub lines: Vec<Line>,
	pub copy_as: String,
}

impl Station {
	/// Whether the last builder change changed any of its lines.
	#[must_use]
	pub fn changed(&self) -> bool {
		self.lines().any(|line| line.changed)
	}

	/// Every line of its code, tab after tab.
	pub fn lines(&self) -> impl Iterator<Item = &Line> {
		let (lines, tabs): (&[Line], &[Tab]) = match &self.body {
			Body::Code(lines, _) => (lines, &[]),
			Body::Tabs(tabs) => (&[], tabs),
			_ => (&[], &[]),
		};

		lines
			.iter()
			.chain(tabs.iter().flat_map(|tab| tab.lines.iter()))
	}

	/// A station.
	pub fn new(
		name: impl Into<String>,
		source: impl Into<String>,
		why: impl Into<String>,
		body: Body,
	) -> Self {
		Self {
			name: name.into(),
			source: source.into(),
			why: why.into(),
			body,
		}
	}
}

/// What a page's drawers are doing, while the page runs in the browser.
///
/// The shell provides one for a page this crate builds; the pieces that
/// open and fill drawers (the mint tab, cards, code blocks, the panel) read
/// it from context. A page `leptoskit_admin` still draws on its own has
/// none, and those pieces render as they always have, for its script.
#[derive(Clone, Copy, Debug)]
pub struct Workings {
	/// Whether mint tabs show their sources.
	pub mode: RwSignal<bool>,
	/// The open drawer's id.
	pub open: RwSignal<Option<String>>,
	/// The Ask stations' tab: `rust`, `sql`, or `http`.
	pub ask: RwSignal<String>,
	/// The stations opened on a phone, as `drawer|station`.
	stations: RwSignal<Vec<String>>,
	/// The text a Copy button just copied, while it says so.
	copied: RwSignal<Option<String>>,
	/// The open drawer's Close button.
	close: FocusRequest,
	/// The control that opened the drawer, which gets focus back.
	opener: RwSignal<Option<FocusRequest>>,
	/// The first drawer on the page, which W opens.
	first: RwSignal<Option<String>>,
	/// Each drawer's mint tab, which gets focus back when nothing else
	/// opened it.
	tabs: StoredValue<Vec<(String, FocusRequest)>>,
	/// The `[infra]` key a plan row linked, whose lines the plan's drawer
	/// marks.
	linked: RwSignal<Option<String>>,
	/// Scrolls the first linked line into view.
	reveal: Reveal,
	/// What the last builder change changed, as the open drawer says it:
	/// `Ask changed: interval: Interval::Week`.
	change: RwSignal<String>,
	/// The text of the changed line the drawer shows.
	change_target: RwSignal<Option<String>>,
	/// Scrolls that line into the drawer's view.
	change_reveal: Reveal,
}

/// How long a Copy button says "Copied".
const COPIED_FOR: std::time::Duration = std::time::Duration::from_millis(1200);

impl Workings {
	/// The drawers of a page, with `open` open and Workings mode `mode`.
	#[must_use]
	pub fn new(mode: bool, open: Option<String>, first: Option<String>) -> Self {
		Self {
			mode: RwSignal::new(mode),
			open: RwSignal::new(open),
			ask: RwSignal::new("rust".to_owned()),
			stations: RwSignal::new(Vec::new()),
			copied: RwSignal::new(None),
			close: FocusRequest::new(),
			opener: RwSignal::new(None),
			first: RwSignal::new(first),
			tabs: StoredValue::new(Vec::new()),
			linked: RwSignal::new(None),
			reveal: Reveal::new(),
			change: RwSignal::new(String::new()),
			change_target: RwSignal::new(None),
			change_reveal: Reveal::new(),
		}
	}

	/// Notes drawer `id`'s mint tab, which gets focus back when the drawer
	/// closes and nothing else opened it.
	pub(crate) fn register_tab(&self, id: &str, tab: FocusRequest) {
		self.tabs.update_value(|tabs| {
			match tabs.iter_mut().find(|(known, _)| known == id) {
				Some(known) => known.1 = tab,
				None => tabs.push((id.to_owned(), tab)),
			}
		});
	}

	/// The page's drawers, when the page runs as this crate's.
	#[must_use]
	pub fn current() -> Option<Self> {
		use_context::<Self>()
	}

	/// Whether drawer `id` is open. Reading it rebuilds the screen when that
	/// changes.
	#[must_use]
	pub fn is_open(&self, id: &str) -> bool {
		self.open.with(|open| open.as_deref() == Some(id))
	}

	/// Opens drawer `id` and moves focus to its Close button; `from` gets
	/// focus back when it closes.
	pub fn show(&self, id: &str, from: Option<FocusRequest>) {
		self.linked.set(None);
		self.open.set(Some(id.to_owned()));
		self.opener.set(from);
		self.close.request();
	}

	/// Opens the plan's drawer with the lines that declare `key` marked, and
	/// scrolls to them; `from` gets focus back when it closes.
	pub fn link(&self, key: &str, from: Option<FocusRequest>) {
		self.show("plan", from);
		self.linked.set(Some(key.to_owned()));
		self.reveal.request();
	}

	/// Whether a line or plan row for `key` is the one a plan row linked:
	/// the key itself, or one of its parts (`backups` links
	/// `backups.enabled`).
	#[must_use]
	pub fn is_linked(&self, key: &str) -> bool {
		self.linked.with(|linked| {
			linked.as_deref().is_some_and(|linked| {
				key == linked
					|| key
						.strip_prefix(linked)
						.is_some_and(|rest| rest.starts_with('.'))
			})
		})
	}

	/// Scrolls the first linked line into view.
	#[must_use]
	pub fn reveal(&self) -> Reveal {
		self.reveal
	}

	/// Says what a builder change changed, and scrolls the line `target`
	/// into the open drawer's view; `None` says nothing.
	pub(crate) fn announce_change(&self, status: Option<String>, target: Option<String>) {
		self.change.set(status.unwrap_or_default());
		let reveal = target.is_some();
		self.change_target.set(target);

		if reveal {
			self.change_reveal.request();
		}
	}

	/// What the last builder change changed, while a drawer is open.
	#[must_use]
	pub fn change(&self) -> String {
		self.change.get()
	}

	/// Whether `text` is the changed line the drawer shows.
	#[must_use]
	pub fn is_change_target(&self, text: &str) -> bool {
		self.change_target
			.with(|target| target.as_deref() == Some(text))
	}

	/// Scrolls the changed line into the drawer's view.
	#[must_use]
	pub fn change_reveal(&self) -> Reveal {
		self.change_reveal
	}

	/// Opens a station on a phone, where stations start as headers.
	pub(crate) fn open_station(&self, key: &str) {
		if !self
			.stations
			.with_untracked(|open| open.iter().any(|station| station == key))
		{
			self.stations.update(|open| open.push(key.to_owned()));
		}
	}

	/// Closes the open drawer and gives focus back to what opened it.
	pub fn hide(&self) {
		if self.open.get_untracked().is_none() {
			return;
		}

		let open = self.open.get_untracked();
		self.open.set(None);
		self.linked.set(None);
		self.change.set(String::new());
		self.change_target.set(None);

		let remembered = browser::take_focus();

		if let Some(opener) = self.opener.get_untracked() {
			opener.request();
		} else if !remembered.is_some_and(browser::Focus::restore) {
			let tab = self.tabs.with_value(|tabs| {
				tabs.iter()
					.find(|(id, _)| Some(id) == open.as_ref())
					.map(|(_, tab)| *tab)
			});

			if let Some(tab) = tab {
				tab.request();
			}
		}

		self.opener.set(None);
	}

	/// Opens drawer `id`, or closes it when it is the one open.
	pub fn toggle(&self, id: &str, from: Option<FocusRequest>) {
		if self.open.with_untracked(|open| open.as_deref() == Some(id)) {
			self.hide();
		} else {
			self.show(id, from);
		}
	}

	/// W: closes the open drawer, or opens the drawer of the card that has
	/// focus, or else the page's first. What had focus gets it back.
	pub fn toggle_first(&self) {
		if self.open.get_untracked().is_some() {
			self.hide();
			return;
		}

		let card = browser::focused_card();
		let Some(id) = card.or_else(|| self.first.get_untracked()) else {
			return;
		};
		// Before the drawer takes focus.
		browser::remember_focus();
		self.show(&id, None);
	}

	/// Whether a station is open on a phone, where stations start as headers.
	fn station_open(&self, key: &str) -> bool {
		self.stations
			.with(|open| open.iter().any(|station| station == key))
	}

	fn toggle_station(&self, key: &str) {
		self.stations.update(|open| {
			if let Some(index) = open.iter().position(|station| station == key) {
				open.remove(index);
			} else {
				open.push(key.to_owned());
			}
		});
	}

	/// Shows `tab` in every Ask station, and notes that SQL was read.
	fn select_ask(&self, tab: &str) {
		self.ask.set(tab.to_owned());

		if tab == "sql" {
			crate::shell::preference(r#"{"kind":"use_note","note":"ask-sql"}"#);
		}
	}

	/// Whether a Copy button for `text` says "Copied" now.
	#[must_use]
	pub fn copied(&self, text: &str) -> bool {
		self.copied.with(|copied| copied.as_deref() == Some(text))
	}

	/// Copies `text`, then says so for a moment. When the browser refuses,
	/// the notice says to select the text instead.
	pub fn copy(&self, text: String, notice: Option<RwSignal<String>>) {
		copy(text, self.copied, notice);
	}
}

#[cfg(feature = "hydrate")]
fn copy(text: String, copied: RwSignal<Option<String>>, notice: Option<RwSignal<String>>) {
	crate::spawn(async move {
		if leptoskit_plugin_clipboard::web::write_text(&text)
			.await
			.is_err()
		{
			if let Some(notice) = notice {
				notice.set("The browser didn't allow copying. Select the text instead.".to_owned());
			}
			return;
		}

		copied.set(Some(text.clone()));
		let reset = move || {
			if copied.with_untracked(|copied| copied.as_deref() == Some(text.as_str())) {
				copied.set(None);
			}
		};

		if leptos::prelude::set_timeout(reset, COPIED_FOR).is_err() {
			copied.set(None);
		}
	});
}

/// Off the browser there's no clipboard: the press only marks the text
/// copied, which tests read.
#[cfg(not(feature = "hydrate"))]
fn copy(text: String, copied: RwSignal<Option<String>>, notice: Option<RwSignal<String>>) {
	let _ = (notice, COPIED_FOR);
	copied.set(Some(text));
}

/// The text Copy gives for `lines`.
#[must_use]
pub fn text_of(lines: &[Line]) -> String {
	lines
		.iter()
		.map(|line| line.text.as_str())
		.collect::<Vec<_>>()
		.join("\n")
}

/// What Copy all gives for `drawer`: every code block, a blank line apart.
fn all_text(drawer: &Drawer) -> String {
	let mut blocks = Vec::new();

	for station in &drawer.stations {
		match &station.body {
			Body::Code(lines, _) => blocks.push(text_of(lines)),
			Body::Tabs(tabs) => blocks.extend(tabs.iter().map(|tab| text_of(&tab.lines))),
			Body::Checks(_) | Body::Missing(_) => {}
		}
	}

	blocks.join("\n\n")
}

/// The drawer's panel, hidden until its card's tab opens it.
pub fn panel(drawer: &Drawer) -> Element {
	let workings = Workings::current();
	let mut close = Button("Close")
		.aria_label("Close workings (W)")
		.class(
			"shrink-0 rounded-lg border border-[#4a3f58] px-2.5 py-1 text-xs text-[#c9c0d0] \
			 cursor-pointer hover:text-white",
		)
		.test_id("workings-close");

	if let Some(workings) = workings {
		close = close
			.focus_on(workings.close)
			.on_press(move || workings.hide());
	}

	let header = View()
		.class(
			"shrink-0 flex-row items-start gap-3 border-b border-[var(--admin-underside-line)] \
			 px-5 py-4",
		)
		.child(
			View()
				.class("mt-1 h-3.5 w-3.5 shrink-0 rounded-[3px] bg-[#5ef2c2] -rotate-12")
				.aria_hidden(true),
		)
		.child(
			View()
				.class("min-w-0 flex-1 flex-col gap-0.5")
				.child(Heading(2, "Workings").class("text-base font-semibold text-[#f4efe8]"))
				.child(Text(drawer.title.clone()).class("text-xs text-[#9d93a8]"))
				// What a builder change just changed.
				.child(
					Status(match workings {
						Some(workings) if workings.is_open(&drawer.id) => workings.change(),
						_ => String::new(),
					})
					.class("text-xs font-semibold text-[#5ef2c2]")
					.test_id("drawer-changed"),
				),
		)
		.child(close);

	let count = drawer.stations.len();
	let analytics = drawer
		.stations
		.first()
		.is_some_and(|station| station.name == "Emit");
	let mut rail = View().class("flex-col px-5 pb-6 pt-1.5");

	for (index, station) in drawer.stations.iter().enumerate() {
		rail = rail.child(station_view(&drawer.id, station, index, count));
	}

	let text = all_text(drawer);
	let mut copy_all = Button(if workings.is_some_and(|workings| workings.copied(&text)) {
		"Copied"
	} else {
		"Copy all"
	})
	.class(
		"rounded-lg border border-[#5ef2c2] bg-[#5ef2c2] px-3 py-1.5 text-xs font-semibold \
		 text-[#0f0b16] cursor-pointer hover:opacity-90",
	)
	.test_id("copy-all");

	if let Some(workings) = workings {
		copy_all = copy_all.on_press(move || workings.copy(text.clone(), None));
	}

	// The header stays put; the stations scroll beneath it.
	View()
		.class("h-full flex-col")
		.class(match workings {
			Some(workings) if !workings.is_open(&drawer.id) => "hidden",
			_ => "",
		})
		.test_id(format!("drawer:{}", drawer.id))
		.aria_label(format!("Workings: {}", drawer.title))
		.child(header)
		.child(
			View()
				.class("min-h-0 flex-1 flex-col overflow-scroll")
				.test_id("drawer-scroll")
				.child(rail)
				.child(
					View()
						.class("flex-row flex-wrap gap-2 px-5 pb-6 pl-12")
						.child(copy_all)
						.child(
							Pressable(if analytics {
								"Open the product analytics guide"
							} else {
								"Open the infrastructure guide"
							})
							.href(if analytics {
								"https://leptoskit.dev/docs/guide/40-product-analytics.html"
							} else {
								"https://leptoskit.dev/docs/guide/24-deploying.html"
							})
							.class(
								"rounded-lg border border-[#4a3f58] px-3 py-1.5 cursor-pointer \
								 hover:border-[#c9c0d0]",
							)
							.child(Text("Open docs").class("text-xs font-semibold text-[#c9c0d0]")),
						),
				),
		)
}

fn station_view(drawer: &str, station: &Station, index: usize, count: usize) -> Element {
	let workings = Workings::current();
	let key = format!("{drawer}|{}", station.name.to_lowercase());
	let open = workings.is_some_and(|workings| workings.station_open(&key));
	// The rail runs from the first dot to the last.
	let line = match (index, count) {
		(_, 1) => None,
		(0, _) => Some("absolute left-[6px] top-6 bottom-0 w-0.5 bg-[#14bf9b]"),
		(last, count) if last + 1 == count => {
			Some("absolute left-[6px] top-0 h-6 w-0.5 bg-[#5ef2c2]")
		}
		_ => Some("absolute left-[6px] top-0 bottom-0 w-0.5 bg-[#14bf9b]"),
	};

	let mut view = View()
		.class("relative flex-col gap-2 pb-1.5 pl-7 pt-4")
		.test_id(format!("station:{}", station.name.to_lowercase()));

	// A station whose code the last builder change changed says so.
	if station.changed() {
		view = view.aria_description("changed");
	}

	if let Some(line) = line {
		view = view.child(View().class(line).aria_hidden(true));
	}

	// On phones the drawer is a bottom sheet and stations collapse to their
	// headers; this covers the header so the whole row opens its station.
	let mut toggle = Button(match (workings, open) {
		(None, _) => "",
		(Some(_), true) => "▴",
		(Some(_), false) => "▾",
	})
	.aria_label(format!("{} station", station.name))
	.aria_expanded(open)
	.class("absolute inset-x-0 top-2 h-9 rounded-lg cursor-pointer sm:hidden")
	.test_id("station-toggle");

	if let Some(workings) = workings {
		toggle = toggle
			.class("flex-row items-center justify-end pr-2 text-sm text-[#9d93a8]")
			.on_press(move || workings.toggle_station(&key));
	}

	view = view
		.child(
			View()
				.class(
					"absolute left-0 top-[18px] h-3.5 w-3.5 rounded-full border-[3px] \
					 border-[#5ef2c2] bg-[var(--admin-underside)]",
				)
				.aria_hidden(true)
				.test_id("station-dot"),
		)
		.child(
			View()
				.class("flex-row flex-wrap items-end gap-2 max-sm:pr-8")
				.test_id("station-head")
				.child(
					Heading(3, station.name.clone()).class("text-sm font-semibold text-[#f4efe8]"),
				)
				.child(Text(station.source.clone()).class("text-xs text-[#9d93a8]")),
		)
		.child(toggle);

	let body = View()
		.class("flex-col gap-2")
		.class(if workings.is_some() && !open {
			"max-sm:hidden"
		} else {
			""
		})
		.test_id("station-body")
		.child(components::prose(&station.why).class("text-[13px] leading-normal text-[#c9c0d0]"));

	view.child(station_body(drawer, station, body))
}

/// A station's explanation and what it shows.
fn station_body(drawer: &str, station: &Station, view: Element) -> Element {
	match &station.body {
		Body::Code(lines, copy_as) => view.child(components::code_block(lines, copy_as)),
		Body::Checks(items) => {
			let mut list = View().class("flex-col gap-1.5");

			for (tone, item) in items {
				let color = match tone {
					Tone::Good => "text-[#5fd08a]",
					Tone::Warn => "text-[#e3b341]",
					Tone::Bad => "text-[#ff8a9e]",
					Tone::Info => "text-[#c9c0d0]",
				};
				let icon = match tone {
					Tone::Good => "✓",
					Tone::Warn => "!",
					Tone::Bad => "✕",
					Tone::Info => "•",
				};
				list = list.child(
					View()
						.class("flex-row gap-2")
						.child(
							Text(icon)
								.class("w-4 shrink-0 text-sm font-semibold")
								.class(color),
						)
						.child(
							components::prose(item)
								.class("min-w-0 flex-1 text-[13px] text-[#f4efe8]"),
						),
				);
			}

			view.child(list)
		}
		Body::Tabs(tabs) => view.children(ask_tabs(drawer, station, tabs)),
		Body::Missing(reason) => {
			view.child(components::prose(reason).class(
				"rounded-[10px] border border-[var(--admin-underside-line)] px-3 py-2 text-[13px] \
				 text-[#9d93a8]",
			))
		}
	}
}

/// The Ask station's tabs and their panels. Every Ask station on the page
/// shows the same tab; without [`Workings`], the first shows until the
/// script picks another.
fn ask_tabs(drawer: &str, station: &Station, tabs: &[Tab]) -> [Element; 2] {
	let workings = Workings::current();

	if let Some(workings) = workings
		&& !tabs.is_empty()
	{
		return live_ask_tabs(workings, drawer, station, tabs);
	}

	let selected = |index: usize, tab: &Tab| {
		match workings {
			Some(workings) => workings.ask.with(|ask| *ask == tab.name.to_lowercase()),
			None => index == 0,
		}
	};
	let mut row = View()
		.class("flex-row gap-1.5")
		.role(leptoskit::native::Role::TabList)
		.aria_label(format!("{} as", station.name));
	let id = |part: &str, tab: &Tab| {
		format!("{part}-{drawer}-{}", tab.name.to_lowercase()).replace(
			|character: char| !character.is_ascii_alphanumeric() && character != '-',
			"-",
		)
	};

	for (index, tab) in tabs.iter().enumerate() {
		let on = selected(index, tab);
		let mut button = Button(tab.name.clone())
			.role(leptoskit::native::Role::Tab)
			.id(id("ask-tab", tab))
			.aria_controls([id("ask-panel", tab)])
			.aria_selected(on)
			.tab_index(if on { 0 } else { -1 })
			.class(
				"rounded-lg border px-3 py-1 text-[13px] font-semibold cursor-pointer \
				 hover:text-white",
			)
			.class(if workings.is_some() && on {
				"border-[#5ef2c2] text-[#5ef2c2]"
			} else {
				"border-[#4a3f58] text-[#c9c0d0]"
			})
			.test_id(format!("ask-tab:{}", tab.name.to_lowercase()));

		if let Some(workings) = workings {
			let name = tab.name.to_lowercase();
			button = button.on_press(move || workings.select_ask(&name));
		}

		row = row.child(button);
	}

	let mut panels = View().class("flex-col");

	for (index, tab) in tabs.iter().enumerate() {
		panels = panels.child(
			View()
				.class("flex-col")
				.class(if workings.is_some() && !selected(index, tab) {
					"hidden"
				} else {
					""
				})
				.role(leptoskit::native::Role::TabPanel)
				.id(id("ask-panel", tab))
				.aria_labelled_by([id("ask-tab", tab)])
				.test_id(format!("ask-panel:{}", tab.name.to_lowercase()))
				.child(components::code_block(&tab.lines, &tab.copy_as)),
		);
	}

	[row, panels]
}

/// The Ask tabs on a page this crate runs: a tab set, so the arrow keys,
/// Home and End move between them, and one tab picks every Ask station's.
fn live_ask_tabs(
	workings: Workings,
	drawer: &str,
	station: &Station,
	tabs: &[Tab],
) -> [Element; 2] {
	let names: Vec<String> = tabs.iter().map(|tab| tab.name.to_lowercase()).collect();
	let selected = workings
		.ask
		.with(|ask| names.iter().position(|name| name == ask))
		.unwrap_or(0);
	let set = leptoskit::aria::Tabs::new(
		format!("ask-{drawer}-{}", station.name.to_lowercase()).replace(
			|character: char| !character.is_ascii_alphanumeric() && character != '-',
			"-",
		),
		tabs.len(),
		selected,
	);
	let mut row = set
		.list({
			let names = names.clone();
			move |index| {
				if let Some(name) = names.get(index) {
					workings.select_ask(name);
				}
			}
		})
		.class("flex-row gap-1.5")
		.aria_label(format!("{} as", station.name));
	let mut panels = View().class("flex-col");

	for (index, (tab, name)) in tabs.iter().zip(&names).enumerate() {
		row = row.child(
			set.tab(index, tab.name.clone())
				.class(
					"rounded-lg border px-3 py-1 text-[13px] font-semibold cursor-pointer \
					 hover:text-white",
				)
				.class(if index == selected {
					"border-[#5ef2c2] text-[#5ef2c2]"
				} else {
					"border-[#4a3f58] text-[#c9c0d0]"
				})
				.test_id(format!("ask-tab:{name}")),
		);
		panels = panels.child(
			set.panel(index)
				.class("flex-col")
				.test_id(format!("ask-panel:{name}"))
				.child(components::code_block(&tab.lines, &tab.copy_as)),
		);
	}

	[row, panels]
}

/// The drawer column: every panel on the page, of which the open one shows.
pub fn aside(drawers: &[Drawer]) -> Element {
	let hidden = Workings::current().is_some_and(|workings| workings.open.with(Option::is_none));

	View()
		.class(style::UNDERSIDE)
		.class(
			"absolute inset-y-0 right-0 w-[440px] max-w-full shrink-0 border-l border-black \
			 overflow-scroll xl:relative max-sm:inset-x-0 max-sm:top-[15vh] max-sm:w-full \
			 max-sm:rounded-2xl transition duration-200 ease-out starting:translate-x-6 \
			 starting:opacity-40",
		)
		.class(if hidden { "hidden" } else { "" })
		.test_id("drawer-aside")
		.role(leptoskit::native::Role::Complementary)
		.aria_label("Workings")
		.children(drawers.iter().map(panel))
}

/// What had focus, and the card it is in, in the browser. W asks: the
/// framework has no question for "what has focus", so this reads the page.
mod browser {
	#[cfg(feature = "hydrate")]
	thread_local! {
		static REMEMBERED: std::cell::RefCell<Option<web_sys::HtmlElement>> =
			const { std::cell::RefCell::new(None) };
	}

	/// An element that had focus.
	pub(super) struct Focus(#[cfg(feature = "hydrate")] web_sys::HtmlElement);

	impl Focus {
		/// Gives it focus back, when it is still on the page.
		pub(super) fn restore(self) -> bool {
			#[cfg(feature = "hydrate")]
			{
				if !self.0.is_connected() {
					return false;
				}
				self.0.focus().is_ok()
			}
			#[cfg(not(feature = "hydrate"))]
			{
				let _ = self;
				false
			}
		}
	}

	#[cfg(feature = "hydrate")]
	fn active() -> Option<web_sys::HtmlElement> {
		use wasm_bindgen::JsCast as _;

		web_sys::window()?
			.document()?
			.active_element()?
			.dyn_into::<web_sys::HtmlElement>()
			.ok()
	}

	/// The drawer id of the card that holds focus.
	pub(super) fn focused_card() -> Option<String> {
		#[cfg(feature = "hydrate")]
		{
			let card = active()?.closest("[data-testid^='card:']").ok()??;
			card.get_attribute("data-testid")?
				.strip_prefix("card:")
				.map(str::to_owned)
		}
		#[cfg(not(feature = "hydrate"))]
		None
	}

	/// Remembers what has focus, unless nothing does.
	pub(super) fn remember_focus() {
		#[cfg(feature = "hydrate")]
		{
			let body = web_sys::window()
				.and_then(|window| window.document())
				.and_then(|document| document.body());
			let element = active().filter(|element| Some(element) != body.as_ref());
			REMEMBERED.with(|remembered| *remembered.borrow_mut() = element);
		}
	}

	/// What [`remember_focus`] remembered, forgetting it.
	pub(super) fn take_focus() -> Option<Focus> {
		#[cfg(feature = "hydrate")]
		{
			REMEMBERED.with(|remembered| remembered.borrow_mut().take().map(Focus))
		}
		#[cfg(not(feature = "hydrate"))]
		None
	}
}
