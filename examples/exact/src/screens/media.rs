//! Recorder, photo, video, motion, canvas, and SVG.

use leptoskit::prelude::*;

use crate::frame::Card;
use crate::frame::Choice;
use crate::frame::Frame;
use crate::frame::Note;

/// A microphone screen: ready, then recording.
#[component]
pub fn Recorder() -> Element {
	let recording = remember(|| false);
	let on = recording.get();
	let label = if on { "Stop" } else { "Record" };
	let status = if on { "Recording" } else { "Ready" };
	view! {
		<Frame title="Recorder">
			<Note text="The microphone, as a button. Nothing is written to disk." />
			<Card>
				<Text class="text-base font-semibold text-slate-900">{status}</Text>
				<View class="flex-row gap-1 items-end h-8">
					<View class="w-2 h-3 bg-indigo-500" />
					<View class="w-2 h-6 bg-indigo-500" />
					<View class="w-2 h-4 bg-indigo-500" />
					<View class="w-2 h-7 bg-indigo-500" />
				</View>
				<Choice label=label on_press=move || recording.set(!on) />
			</Card>
		</Frame>
	}
}

/// Rotate and crop, as choices rather than a native gesture module.
#[component]
pub fn PhotoEditor() -> Element {
	let turns = remember(|| 0_u32);
	let crop = remember(|| "Free");
	let quarter = turns.get() % 4;
	let frame = crop.get();
	view! {
		<Frame title="Photo editor">
			<Note text="Rotate, then pick a crop. The picture is a stand-in." />
			<View class="h-[180px] items-center justify-center rounded-xl bg-slate-200">
				<Text class="text-sm text-slate-900">{format!("Turn {quarter} · {frame}")}</Text>
			</View>
			<View class="flex-row flex-wrap gap-2">
				<Choice label="Rotate" on_press=move || turns.update(|value| *value += 1) />
				<Choice label="Square" on_press=move || crop.set("Square") />
				<Choice label="Wide" on_press=move || crop.set("Wide") />
			</View>
		</Frame>
	}
}

/// A clip that shrinks when the keyboard is open.
#[component]
pub fn VideoPlayer() -> Element {
	let playing = remember(|| false);
	let keyboard = remember(|| false);
	let play = playing.get();
	let keys = keyboard.get();
	let stage = if keys { "h-[96px]" } else { "h-[200px]" };
	let play_label = if play { "Pause" } else { "Play" };
	view! {
		<Frame title="Video player">
			<Note text="The clip shrinks when the keyboard opens." />
			<View class=format!("{stage} items-center justify-center rounded-xl bg-slate-900")>
				<Text class="text-sm text-white">{if play { "Playing" } else { "Paused" }}</Text>
			</View>
			<View class="flex-row gap-2">
				<Choice label=play_label on_press=move || playing.set(!play) />
				<Choice label="Keyboard" on_press=move || keyboard.set(!keys) />
			</View>
		</Frame>
	}
}

/// Animated images and a keyframe you can start.
#[component]
pub fn MotionGallery() -> Element {
	let playing = remember(|| false);
	let on = playing.get();
	view! {
		<Frame title="Motion gallery">
			<Note text="GIF, WebP, and a CSS keyframe, compared by playing one." />
			<Card>
				<Text class="text-sm text-slate-900">"GIF · a looping frame"</Text>
				<Text class="text-sm text-slate-900">"WebP · one still, then motion"</Text>
				<Text class="text-sm font-semibold text-slate-900">
					{if on {
						"Keyframe running"
					} else {
						"Keyframe resting"
					}}
				</Text>
				<Choice label="Play keyframe" on_press=move || playing.set(true) />
			</Card>
		</Frame>
	}
}

const CANVAS_PAGES: [&str; 3] = ["Filled rects", "An arc", "Filled text"];

/// Canvas 2D pages. The marks are views, not a bitmap.
#[component]
pub fn CanvasGallery() -> Element {
	let page = remember(|| 0_usize);
	let index = page.get();
	view! {
		<Frame title="Canvas gallery">
			<Note text="Rects, an arc, and text: the canvas pages, as views." />
			<View class="flex-row flex-wrap gap-2">
				<Choice label="Rects" on_press=move || page.set(0) />
				<Choice label="Arc" on_press=move || page.set(1) />
				<Choice label="Text" on_press=move || page.set(2) />
			</View>
			<Card>
				<Text class="text-sm font-semibold text-slate-900">{CANVAS_PAGES[index]}</Text>
				<View class="flex-row gap-2">
					<View class="w-8 h-8 bg-indigo-500" />
					<View class="w-8 h-8 rounded-full bg-slate-900" />
				</View>
			</Card>
		</Frame>
	}
}

const SVG_PAGES: [&str; 3] = ["Rectangle", "Circle", "Path"];

/// SVG pages, one feature at a time.
#[component]
pub fn SvgGallery() -> Element {
	let page = remember(|| 0_usize);
	let index = page.get();
	view! {
		<Frame title="SVG gallery">
			<Note text="One page per SVG feature. These three are the start." />
			<View class="flex-row flex-wrap gap-2">
				<Choice label="Rectangle" on_press=move || page.set(0) />
				<Choice label="Circle" on_press=move || page.set(1) />
				<Choice label="Path" on_press=move || page.set(2) />
			</View>
			<Card>
				<Text class="text-sm font-semibold text-slate-900">{SVG_PAGES[index]}</Text>
			</Card>
		</Frame>
	}
}
