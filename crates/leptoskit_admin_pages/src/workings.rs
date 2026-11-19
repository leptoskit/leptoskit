//! The Workings drawer: the underside of a card, as five stations joined by
//! a mint rail.
//!
//! Analytics cards read Emit, Send, Store, Ask, Draw; infrastructure
//! actions read Declare, Generate, Plan, Apply, Check. Every station starts
//! with one plain sentence of why, and says so when it can't explain
//! something rather than guessing.

use leptos::context::use_context;
use leptoskit::native::FocusRequest;
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
		}
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
		self.open.set(Some(id.to_owned()));
		self.opener.set(from);
		self.close.request();
	}

	/// Closes the open drawer and gives focus back to what opened it.
	pub fn hide(&self) {
		if self.open.get_untracked().is_none() {
			return;
		}

		self.open.set(None);

		if let Some(opener) = self.opener.get_untracked() {
			opener.request();
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

	/// W: closes the open drawer, or opens the page's first.
	pub fn toggle_first(&self) {
		if self.open.get_untracked().is_some() {
			self.hide();
		} else if let Some(first) = self.first.get_untracked() {
			self.show(&first, None);
		}
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
					Status("")
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
