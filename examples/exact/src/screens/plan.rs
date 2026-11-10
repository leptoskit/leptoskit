//! Calendar, carousel, Still, and Update Lab.

use leptoskit::prelude::*;
use leptoskit::virtual_list::Axis;
use leptoskit::virtual_list::VirtualList;
use leptoskit::virtual_list::VirtualOptions;
use leptoskit::virtual_list::Virtualizer;

use crate::frame::Card;
use crate::frame::Choice;
use crate::frame::Frame;
use crate::frame::Note;

/// A month. Pick a day and read its agenda.
#[component]
pub fn Calendar() -> Element {
	let day = remember(|| 16_u32);
	let selected = day.get();
	view! {
		<Frame title="Calendar">
			<Note text="September 2026. Pick a day. Dragging lives in the full app." />
			<View class="flex-row flex-wrap gap-2">
				<Choice label="16" on_press=move || day.set(16) />
				<Choice label="17" on_press=move || day.set(17) />
				<Choice label="18" on_press=move || day.set(18) />
			</View>
			<Card>
				<Text class="text-sm font-semibold text-slate-900">
					{format!("{selected} September")}
				</Text>
				<Text class="text-sm text-slate-600">"Design review · 10:00"</Text>
			</Card>
		</Frame>
	}
}

/// Twenty-five thousand cards, with only the visible window built.
#[component]
pub fn Carousel() -> Element {
	let offset = remember(|| 0.0_f64);
	let at = offset.get();
	let mut virtualizer = Virtualizer::new(VirtualOptions::fixed(25_000, 168.0).with_overscan(1));
	virtualizer.set_viewport(358.0);
	virtualizer.scroll_to(at);
	let follow = offset;
	let list = VirtualList(
		&virtualizer,
		Axis::Horizontal,
		move |next| follow.set(next),
		|item| card(item.index),
	)
	.expect("a fixed card width is a valid estimate");
	let advance = move || offset.set((at + 168.0).min(168.0 * 24_990.0));
	view! {
		<Frame title="Carousel">
			<Note text="25,000 cards in a horizontal list. Only the window is built." />
			<View class="h-[120px]">{list}</View>
			<Choice label="Next cards" on_press=advance />
		</Frame>
	}
}

fn card(index: usize) -> Element {
	let label = format!("Card {}", index + 1);
	view! {
		<View class="w-[160px] h-[96px] m-1 justify-center rounded-xl bg-white px-3">
			<Text class="text-sm font-semibold text-slate-900">{label}</Text>
		</View>
	}
}

const RECORDS: [&str; 3] = ["North", "Harbour", "Studio"];

/// Drag-to-reorder, shown as moving a record up the list.
#[component]
pub fn Still() -> Element {
	let order = remember(|| vec![0_usize, 1, 2]);
	let ids = order.get();
	let lift = move || {
		order.update(|ids| {
			if ids.len() > 1 {
				ids.swap(0, 1);
			}
		});
	};
	view! {
		<Frame title="Still">
			<Note text="Reorder, then a sheet of records. This list has three of them." />
			<Choice label="Move first down" on_press=lift />
			{ids
				.into_iter()
				.map(|id| {
					let name = RECORDS[id];
					view! {
						<Card>
							<Text class="text-sm text-slate-900">{name}</Text>
						</Card>
					}
				})
				.collect::<Fragment>()}
			<Text class="text-xs text-slate-500">"100 records in the full sheet"</Text>
		</Frame>
	}
}

/// A generation counter for a live replacement.
#[component]
pub fn UpdateLab() -> Element {
	let generation = remember(|| 1_u32);
	let current = generation.get();
	view! {
		<Frame title="Update lab">
			<Note text="Replace the running screen. The generation is the proof." />
			<Card>
				<Text class="text-sm text-slate-900">{format!("Generation {current}")}</Text>
				<Choice label="Replace" on_press=move || generation.update(|value| *value += 1) />
			</Card>
		</Frame>
	}
}
