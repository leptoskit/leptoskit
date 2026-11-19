//! The analytics area's drawing: numbers and times as they read, a query
//! as an address, the charts, and each builder. The admin's server runs the
//! queries; what it gets back is drawn here, on the server and, for a page
//! that hydrates, in the browser too.

pub mod draw;
pub mod format;
pub mod insights;
pub mod params;
pub mod swap;

use leptoskit::native::Placement;
use leptoskit::prelude::*;
use serde::Deserialize;
use serde::Serialize;

use crate::components;
use crate::style;

/// An event the service has stored, as the builders offer it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Definition {
	pub name: String,
	/// How many have been stored.
	pub volume: u64,
	/// Each field and its type, when a Rust type sends it.
	pub fields: Option<Vec<(String, String)>>,
	/// The keys the latest few carried, each with its JSON type.
	pub properties: Vec<(String, String)>,
}

impl Definition {
	/// Whether a Rust type sends it.
	#[must_use]
	pub fn typed(&self) -> bool {
		self.fields.is_some()
	}
}

/// How a card's rows became its chart.
pub struct Drawn {
	/// One or two sentences: how many rows, what the chart does with them.
	pub sentence: String,
	/// The rows as a small table, header first.
	pub rows: Vec<String>,
}

/// Pads a table's cells into columns.
pub fn table(rows: &[Vec<String>]) -> Vec<String> {
	let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
	let widths: Vec<usize> = (0..columns)
		.map(|column| {
			rows.iter()
				.filter_map(|row| row.get(column))
				.map(|cell| cell.chars().count())
				.max()
				.unwrap_or(0)
		})
		.collect();
	rows.iter()
		.map(|row| {
			row.iter()
				.enumerate()
				.map(|(column, cell)| {
					if column == 0 {
						format!("{cell:<width$}", width = widths[column])
					} else {
						format!("{cell:>width$}", width = widths[column])
					}
				})
				.collect::<Vec<_>>()
				.join("  ")
				.trim_end()
				.to_owned()
		})
		.collect()
}

/// A pill that links somewhere: `Last 30 days`, `Compare previous period`.
pub fn chip(label: &str, value: &str, href: String, on: bool) -> Element {
	let mut content = View().class(style::CHIP).class("cursor-pointer");

	if !label.is_empty() {
		content = content.child(Text(label.to_owned()).class("text-[var(--admin-ink-2)]"));
	}

	content = content.child(Text(value.to_owned()).class("font-semibold text-[var(--admin-ink)]"));
	let mut link = Pressable(format!("{label} {value}").trim().to_owned())
		.href(href.clone())
		.class("rounded-full hover:bg-[var(--admin-sunken)]")
		.child(content);

	if on {
		link = link.aria_current(leptoskit::native::Current::Page);
	}

	swap::link(link, href)
}

/// A row of links where one is chosen: `Day | Week`.
pub fn segmented(label: &str, options: &[(String, String, bool)]) -> Element {
	let mut row = View()
		.class(
			"flex-row self-start overflow-hidden rounded-lg border \
			 border-[var(--admin-line-strong)]",
		)
		.role(leptoskit::native::Role::Group)
		.aria_label(label.to_owned());

	for (name, href, on) in options {
		let mut link = Pressable(name.clone())
			.href(href.clone())
			.class("cursor-pointer px-3 py-1.5")
			.class(if *on {
				"bg-[var(--admin-ink)]"
			} else {
				"hover:bg-[var(--admin-sunken)]"
			})
			.child(Text(name.clone()).class(if *on {
				"text-sm font-semibold text-[var(--admin-paper)]"
			} else {
				"text-sm text-[var(--admin-ink-2)]"
			}));

		if *on {
			link = link.aria_current(leptoskit::native::Current::Page);
		}

		row = row.child(swap::link(link, href.clone()));
	}

	View()
		.class("flex-col gap-1.5")
		.child(Text(label.to_owned()).class("text-sm font-semibold text-[var(--admin-ink-2)]"))
		.child(row)
}

/// One choice in a [`menu`]: a link to the page it leads to.
pub struct MenuItem {
	pub label: String,
	/// What sits at the item's end: `struct · 4`.
	pub detail: Option<String>,
	pub href: String,
}

impl MenuItem {
	#[must_use]
	pub fn new(label: impl Into<String>, detail: Option<String>, href: String) -> Self {
		Self {
			label: label.into(),
			detail,
			href,
		}
	}
}

/// A control that opens a list over the content beneath it: `+ Add
/// series`, `Breakdown`. It is a [`Menu`](leptoskit::aria::Menu): one open
/// at a time, the arrow keys move through it, and Escape or a press
/// elsewhere closes it. Each item is a link the builder follows in place.
pub fn menu(label: &str, id: &str, items: Vec<MenuItem>) -> Element {
	let swap = swap::Swap::current();
	let open = swap.is_some_and(|swap| swap.is_menu_open(id));
	let mut menu = leptoskit::aria::Menu::new(format!("menu-{id}"), open)
		.button(
			Button(label.to_owned())
				.class(
					"self-start rounded-lg px-1 py-1 text-sm font-semibold \
					 text-[var(--admin-ink)] cursor-pointer hover:bg-[var(--admin-sunken)]",
				)
				.test_id("menu-toggle")
				.aria_description(id.to_owned()),
		)
		.placement(Placement::BelowStart);
	let hrefs: Vec<String> = items.iter().map(|item| item.href.clone()).collect();

	for item in items {
		let mut row = View()
			.class("flex-row items-center gap-2 px-2 py-1.5")
			.child(
				Text(item.label.clone())
					.class("min-w-0 flex-1 truncate text-sm text-[var(--admin-ink)]"),
			);

		if let Some(detail) = &item.detail {
			row = row.child(Text(detail.clone()).class(style::LABEL));
		}

		let href = item.href;
		menu = menu
			.item(item.label, move || {
				if let Some(swap) = swap {
					swap.go(href.clone(), true);
				}
			})
			.content(row);
	}

	if let Some(swap) = swap {
		let id = id.to_owned();
		menu = menu.on_toggle(move |open| swap.set_menu(&id, open));
	}

	// The menu names its parts by id; the tests know them by test id, and
	// each item keeps its address, which works without the page's Wasm.
	let mut menu = Element::from(menu).class("relative flex-col");

	if let Some(list) = menu.children.get_mut(1) {
		let mut panel = std::mem::replace(list, View()).test_id("menu-panel").class(
			"max-h-72 w-72 flex-col gap-0.5 overflow-scroll rounded-xl border \
			 border-[var(--admin-line)] bg-[var(--admin-surface)] p-1.5",
		);

		for (item, href) in panel.children.iter_mut().zip(hrefs) {
			*item = std::mem::replace(item, View())
				.href(href)
				.class("cursor-pointer rounded-lg hover:bg-[var(--admin-sunken)]")
				.test_id("menu-item");
		}

		*list = panel;
	}

	menu
}

/// A stat tile: label, hero number, a sparkline, and the change.
pub struct Stat<'a> {
	pub label: &'a str,
	pub value: String,
	pub delta: Option<String>,
	/// Whether a rise is good: bounce rate rising isn't.
	pub up_is_good: bool,
	pub spark: Option<Vec<f64>>,
	pub drawer: &'a str,
	pub source: &'a str,
	/// What a change is measured against, without its article: `previous
	/// 30 days`, `period before`.
	pub versus: String,
}

pub fn stat(stat: Stat<'_>) -> Element {
	let tone = match &stat.delta {
		Some(delta) if delta.starts_with('▲') == stat.up_is_good && !delta.starts_with('=') => {
			"text-[var(--admin-good)]"
		}
		Some(delta) if !delta.starts_with('=') => "text-[var(--admin-bad)]",
		_ => "text-[var(--admin-ink-3)]",
	};
	let mut card = components::card(stat.label)
		.workings(stat.drawer, stat.source)
		.child(
			Text(stat.value)
				.class(style::HERO_NUMBER)
				.test_id("stat-value"),
		);

	card = card.child(match stat.spark {
		Some(values) if values.len() > 1 => draw::sparkline(&values, "h-7 w-32"),
		_ => {
			Text("No daily series yet")
				.class("h-7 truncate text-xs leading-7 text-[var(--admin-ink-3)]")
		}
	});

	card.child(
		Text(match stat.delta {
			Some(delta) => format!("{delta} vs the {}", stat.versus),
			None => format!("Nothing in the {} to compare", stat.versus),
		})
		.class("text-[13px] tabular-nums")
		.class(tone),
	)
	.build()
	.test_id(format!(
		"stat:{}",
		stat.label.to_lowercase().replace(' ', "-")
	))
}
