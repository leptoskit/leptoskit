//! The Workings drawer: the underside of a card, as five stations joined by
//! a mint rail.
//!
//! Analytics cards read Emit, Send, Store, Ask, Draw; infrastructure
//! actions read Declare, Generate, Plan, Apply, Check. Every station starts
//! with one plain sentence of why, and says so when it can't explain
//! something rather than guessing.

use leptoskit::prelude::*;

use crate::components;
use crate::components::Line;
use crate::components::Tone;
use crate::style;

/// One drawer: the underside of one card.
#[derive(Clone, Debug)]
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
#[derive(Clone, Debug)]
pub struct Station {
	/// The station's name: `Declare`, `Plan`, …
	pub name: &'static str,
	/// Where its content comes from: `leptoskit.toml`, `what OpenTofu will do`.
	pub source: String,
	/// One plain sentence of why.
	pub why: String,
	/// What the station shows.
	pub body: Body,
}

/// What a station shows.
#[derive(Clone, Debug)]
pub enum Body {
	/// Code, with Copy as the given format.
	Code(Vec<Line>, &'static str),
	/// A short checklist, each item with its state.
	Checks(Vec<(Tone, String)>),
	/// Nothing to show, and why: "can't explain" states are honest.
	Missing(String),
	/// The same thing several ways, one tab each, such as the Ask station's
	/// Rust, SQL, and HTTP. The root's `data-admin-ask` picks the tab.
	Tabs(Vec<Tab>),
}

/// One tab of [`Body::Tabs`].
#[derive(Clone, Debug)]
pub struct Tab {
	/// `Rust`, `SQL`, `HTTP`; lowercased, its test identifier.
	pub name: &'static str,
	pub lines: Vec<Line>,
	pub copy_as: &'static str,
}

impl Station {
	/// A station.
	pub fn new(
		name: &'static str,
		source: impl Into<String>,
		why: impl Into<String>,
		body: Body,
	) -> Self {
		Self {
			name,
			source: source.into(),
			why: why.into(),
			body,
		}
	}
}

/// The drawer's panel, hidden until its card's tab opens it.
pub fn panel(drawer: &Drawer) -> Element {
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
				.child(Text(drawer.title.clone()).class("text-xs text-[#9d93a8]")),
		)
		.child(
			Button("Close")
				.aria_label("Close workings (W)")
				.class(
					"shrink-0 rounded-lg border border-[#4a3f58] px-2.5 py-1 text-xs \
					 text-[#c9c0d0] cursor-pointer hover:text-white",
				)
				.test_id("workings-close"),
		);

	let count = drawer.stations.len();
	let analytics = drawer
		.stations
		.first()
		.is_some_and(|station| station.name == "Emit");
	let mut rail = View().class("flex-col px-5 pb-6 pt-1.5");

	for (index, station) in drawer.stations.iter().enumerate() {
		rail = rail.child(station_view(station, index, count));
	}

	// The header stays put; the stations scroll beneath it.
	View()
		.class("h-full flex-col")
		.test_id(format!("drawer:{}", drawer.id))
		.aria_label(format!("Workings: {}", drawer.title))
		.child(header)
		.child(
			View()
				.class("min-h-0 flex-1 flex-col overflow-scroll")
				.child(rail)
				.child(
					View()
						.class("flex-row flex-wrap gap-2 px-5 pb-6 pl-12")
						.child(
							Button("Copy all")
								.class(
									"rounded-lg border border-[#5ef2c2] bg-[#5ef2c2] px-3 py-1.5 \
									 text-xs font-semibold text-[#0f0b16] cursor-pointer \
									 hover:opacity-90",
								)
								.test_id("copy-all"),
						)
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

fn station_view(station: &Station, index: usize, count: usize) -> Element {
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

	view = view
		.child(
			View()
				.class(
					"absolute left-0 top-[18px] h-3.5 w-3.5 rounded-full border-[3px] \
					 border-[#5ef2c2] bg-[var(--admin-underside)]",
				)
				.aria_hidden(true),
		)
		.child(
			View()
				.class("flex-row flex-wrap items-end gap-2")
				.child(Heading(3, station.name).class("text-sm font-semibold text-[#f4efe8]"))
				.child(Text(station.source.clone()).class("text-xs text-[#9d93a8]")),
		)
		.child(components::prose(&station.why).class("text-[13px] leading-normal text-[#c9c0d0]"));

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
		Body::Tabs(tabs) => {
			let mut row = View()
				.class("flex-row gap-1.5")
				.role(leptoskit::native::Role::TabList)
				.aria_label(format!("{} as", station.name));

			for tab in tabs {
				row = row.child(
					Button(tab.name)
						.role(leptoskit::native::Role::Tab)
						.class(
							"rounded-lg border border-[#4a3f58] px-3 py-1 text-[13px] \
							 font-semibold text-[#c9c0d0] cursor-pointer hover:text-white",
						)
						.test_id(format!("ask-tab:{}", tab.name.to_lowercase())),
				);
			}

			let mut panels = View().class("flex-col");

			for tab in tabs {
				panels = panels.child(
					View()
						.class("flex-col")
						.role(leptoskit::native::Role::TabPanel)
						.test_id(format!("ask-panel:{}", tab.name.to_lowercase()))
						.child(components::code_block(&tab.lines, tab.copy_as)),
				);
			}

			view.child(row).child(panels)
		}
		Body::Missing(reason) => {
			view.child(components::prose(reason).class(
				"rounded-[10px] border border-[var(--admin-underside-line)] px-3 py-2 text-[13px] \
				 text-[#9d93a8]",
			))
		}
	}
}

/// The drawer column: every panel on the page, of which the script shows
/// the open one.
pub fn aside(drawers: &[Drawer]) -> Element {
	let mut aside = View()
		.class(style::UNDERSIDE)
		.class(
			"absolute inset-y-0 right-0 w-[440px] max-w-full shrink-0 border-l border-black \
			 overflow-scroll xl:relative max-sm:inset-x-0 max-sm:top-[15vh] max-sm:w-full \
			 max-sm:rounded-2xl transition duration-200 ease-out starting:translate-x-6 \
			 starting:opacity-40",
		)
		.test_id("drawer-aside")
		.role(leptoskit::native::Role::Complementary)
		.aria_label("Workings");

	for drawer in drawers {
		aside = aside.child(panel(drawer));
	}

	aside
}
