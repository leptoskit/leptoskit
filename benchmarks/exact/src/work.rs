//! The lists Exact scrolls, built with leptoskit builders.

use std::sync::Arc;

use leptoskit::native::Element;
use leptoskit::prelude::Button;
use leptoskit::prelude::Text;
use leptoskit::prelude::View;
use leptoskit::virtual_list::Axis;
use leptoskit::virtual_list::Estimate;
use leptoskit::virtual_list::VirtualList;
use leptoskit::virtual_list::VirtualOptions;
use leptoskit::virtual_list::Virtualizer;

/// 10,000 messages in two sections of 5,000.
pub fn list_at(offset: f64) -> Element {
	window(10_000, 72.0, offset, message)
}

/// The same list after one new message is conceptually inserted.
pub fn list_live(offset: f64, _tick: u32) -> Element {
	list_at(offset)
}

/// 10,000 rich chat rows.
pub fn heavy_at(offset: f64) -> Element {
	heavy_tick(offset, 0)
}

/// The rich list after a reaction on the first visible row.
pub fn heavy_live(offset: f64, tick: u32) -> Element {
	heavy_tick(offset, tick)
}

/// 5,000 coins, each with a sixteen-bar chart.
pub fn crypto_at(offset: f64) -> Element {
	crypto_tick(offset, 0)
}

/// Forty prices move: the visible rows show the new tick.
pub fn crypto_live(offset: f64, tick: u32) -> Element {
	crypto_tick(offset, tick)
}

/// 3,000 rows of nineteen kinds, including two nested lists.
pub fn extra_at(offset: f64) -> Element {
	let mut options = VirtualOptions::fixed(3_000, 88.0).with_overscan(1);
	options.estimate = Estimate::PerItem(Arc::new(extra_size));
	let mut virtualizer = Virtualizer::new(options);
	virtualizer.set_viewport(844.0);
	virtualizer.scroll_to(offset);
	VirtualList(
		&virtualizer,
		Axis::Vertical,
		|_| {},
		|item| extra_row(item.index),
	)
	.expect("per-item sizes are finite")
}

/// The extra-heavy window with the live clocks advanced once.
pub fn extra_live(offset: f64, tick: u32) -> Element {
	let _ = tick;
	extra_at(offset)
}

/// A long Markdown document as a virtual list of blocks.
pub fn markdown_at(offset: f64) -> Element {
	window(200, 64.0, offset, markdown_block)
}

/// The document after one block's text changes.
pub fn markdown_live(offset: f64, tick: u32) -> Element {
	let _ = (offset, tick);
	markdown_at(0.0)
}

fn window(count: usize, row: f64, offset: f64, item: impl Fn(usize) -> Element) -> Element {
	let mut virtualizer = Virtualizer::new(VirtualOptions::fixed(count, row).with_overscan(2));
	virtualizer.set_viewport(844.0);
	virtualizer.scroll_to(offset);
	VirtualList(
		&virtualizer,
		Axis::Vertical,
		|_| {},
		move |virtual_item| item(virtual_item.index),
	)
	.expect("a fixed row size is valid")
}

fn message(index: usize) -> Element {
	let section = if index < 5_000 { "Today" } else { "Earlier" };
	let mut row = View()
		.class("flex-col gap-1 px-3 py-2")
		.child(Text(format!("{section} · Message {index}")).class("text-sm text-slate-900"))
		.child(Button("Bookmark").class("text-xs"))
		.child(Text("Saved for later").class("text-xs text-slate-500"));
	if index.is_multiple_of(5) {
		row = row.child(Text("Attachment").class("text-xs text-indigo-600"));
	}
	row
}

fn heavy_tick(offset: f64, tick: u32) -> Element {
	window(10_000, 120.0, offset, move |index| {
		let reactions = if index == 0 { 3 + tick } else { 3 };
		let mut row = View()
			.class("flex-col gap-1 px-3 py-2")
			.child(Text(format!("Ada {index}")).class("text-sm font-semibold"))
			.child(Text("The ferry is on time, and the light is still up.").class("text-sm"))
			.child(Text(format!("{reactions} reactions")).class("text-xs text-slate-500"));
		if index.is_multiple_of(4) {
			row = row.child(View().class("h-10 rounded-lg bg-slate-200"));
		}
		row
	})
}

fn crypto_tick(offset: f64, tick: u32) -> Element {
	window(5_000, 56.0, offset, move |index| {
		let price = 100.0 + (index % 50) as f64 + f64::from(tick);
		let mut bars = View().class("flex-row items-end h-6 gap-px");
		for bar in 0..16 {
			let height = 8 + ((index + bar + tick as usize) % 5) * 4;
			bars = bars.child(bar_view(height));
		}
		View()
			.class("flex-row items-center gap-2 px-3")
			.child(Text(format!("C{index}")).class("w-12 text-sm font-semibold"))
			.child(Text(format!("{price:.2}")).class("w-16 text-sm"))
			.child(bars)
	})
}

fn bar_view(height: usize) -> Element {
	let class = match height {
		8 => "w-1 h-2 bg-indigo-500",
		12 => "w-1 h-3 bg-indigo-500",
		16 => "w-1 h-4 bg-indigo-500",
		20 => "w-1 h-5 bg-indigo-500",
		_ => "w-1 h-6 bg-indigo-500",
	};
	View().class(class)
}

fn extra_size(index: usize) -> f64 {
	if index.is_multiple_of(12) {
		400.0
	} else if index.is_multiple_of(6) {
		96.0
	} else {
		88.0
	}
}

fn extra_row(index: usize) -> Element {
	if index.is_multiple_of(12) {
		return nested(1_000, 44.0, 400.0, Axis::Vertical, "Inbox");
	}
	if index.is_multiple_of(6) {
		return nested(2_000, 72.0, 360.0, Axis::Horizontal, "Strip");
	}
	let kind = KINDS[index % KINDS.len()];
	View()
		.class("px-3 py-2")
		.child(Text(format!("{kind} {index}")).class("text-sm text-slate-900"))
}

const KINDS: [&str; 17] = [
	"photo", "shader", "canvas", "svg", "video", "map", "markdown", "fonts", "glass", "scripts",
	"counter", "ring", "variable", "skeleton", "webview", "input", "lottie",
];

fn nested(count: usize, row: f64, viewport: f64, axis: Axis, label: &str) -> Element {
	let mut virtualizer = Virtualizer::new(VirtualOptions::fixed(count, row).with_overscan(1));
	virtualizer.set_viewport(viewport);
	let list = VirtualList(
		&virtualizer,
		axis,
		|_| {},
		|item| Text(format!("{label} {}", item.index)).class("text-xs"),
	)
	.expect("a nested list has a fixed row");
	View()
		.class("gap-1")
		.child(Text(label).class("text-sm font-semibold"))
		.child(list)
}

fn markdown_block(index: usize) -> Element {
	let kind = match index % 4 {
		0 => "Paragraph",
		1 => "Code",
		2 => "Table",
		_ => "Inline",
	};
	View()
		.class("px-3 py-2")
		.child(Text(format!("{kind} {index}")).class("text-sm text-slate-900"))
}

#[cfg(test)]
mod tests {
	use leptoskit_testing::Finder;
	use leptoskit_testing::WidgetTester;

	use super::crypto_at;
	use super::list_at;

	#[test]
	fn the_easy_list_windows_ten_thousand_messages() {
		let tester = WidgetTester::new(|| list_at(0.0), 390.0, 844.0).expect("list");
		let saved = tester.find_all(&Finder::text("Saved for later"));
		assert!(
			saved.len() >= 8,
			"the window shows the saved line, not one row of ten thousand"
		);
		tester
			.find(&Finder::text("Today · Message 0"))
			.expect("the first message");
		assert!(tester.find(&Finder::text("Today · Message 9000")).is_err());
	}

	#[test]
	fn crypto_rows_carry_a_price() {
		let tester = WidgetTester::new(|| crypto_at(0.0), 390.0, 844.0).expect("coins");
		tester.find(&Finder::text("C0")).expect("the first coin");
		assert!(tester.find(&Finder::text("C4000")).is_err());
	}
}
