//! The library: saved insights and dashboards, one list with two tabs.

use leptoskit::prelude::*;

use crate::BASE;
use crate::components;
use crate::components::Kind;
use crate::model::library::Item;
use crate::model::library::Library;
use crate::style;

fn item(item: &Item, test_id: &str) -> Element {
	Pressable(item.title.clone())
		.href(item.href.clone())
		.class("cursor-pointer rounded-lg hover:bg-[var(--admin-sunken)]")
		.test_id(test_id.to_owned())
		.child(
			View()
				.class(
					"flex-row items-center gap-3 border-b border-[var(--admin-line)] px-3 py-2.5",
				)
				.child(
					View()
						.class("min-w-0 flex-1 flex-col gap-0.5")
						.child(
							Text(item.title.clone())
								.class("truncate text-sm font-semibold text-[var(--admin-ink)]"),
						)
						.child(Text(item.detail.clone()).class(style::LABEL)),
				)
				.child(Text("→").class("text-sm text-[var(--admin-ink-3)]")),
		)
}

/// The library, on its insights or dashboards tab.
pub(crate) fn page(library: &Library) -> Element {
	let tabs = components::tabs(
		&[
			(
				"Insights".to_owned(),
				format!("{BASE}/analytics/library"),
				Some(library.insights.to_string()),
			),
			(
				"Dashboards".to_owned(),
				format!("{BASE}/analytics/library?tab=dashboards"),
				Some(library.dashboards.to_string()),
			),
		],
		usize::from(library.dashboards_tab),
	);
	let test_id = if library.dashboards_tab {
		"library-dashboard"
	} else {
		"library-insight"
	};
	let mut list = View()
		.class("flex-col")
		.test_id("library")
		.children(library.items.iter().map(|row| item(row, test_id)));

	if library.items.is_empty() {
		list = list.child(
			Text(if library.dashboards_tab {
				"No dashboards yet. Open any insight and press Add to dashboard."
			} else {
				"Nothing saved yet. Build a trend, a funnel, or a retention grid, then press Save."
			})
			.class(style::MUTED)
			.class("px-3 py-2"),
		);
	}

	let actions = vec![
		components::link_button(
			Kind::Secondary,
			"New trend",
			format!("{BASE}/analytics/trends"),
		),
		components::link_button(
			Kind::Secondary,
			"New funnel",
			format!("{BASE}/analytics/funnels"),
		),
	];

	View()
		.class("flex-col gap-5")
		.child(components::header(
			"Library",
			"saved insights and the dashboards they sit on",
			actions,
		))
		.child(tabs)
		.child(View().class(style::CARD).child(list))
}
