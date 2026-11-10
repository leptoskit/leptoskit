//! Fieldnotes, Markdown, type, and text flow.

use leptoskit::prelude::*;

use crate::frame::Card;
use crate::frame::Choice;
use crate::frame::Frame;
use crate::frame::Note;

const NOTES: [(&str, bool); 3] = [("Pier 42", true), ("Reading list", false), ("Keys", true)];

/// A local notebook: pins, and a filter that keeps them.
#[component]
pub fn Fieldnotes() -> Element {
	let pins = remember(|| true);
	let pinned = pins.get();
	let show = move || pins.set(!pins.get_untracked());
	let filter = if pinned {
		"Showing pins"
	} else {
		"Showing all"
	};
	view! {
		<Frame title="Fieldnotes">
			<Note text="Notes kept on the device. Pins stay when you ask for them." />
			<Choice label=filter on_press=show />
			{NOTES
				.into_iter()
				.filter(|(_, pin)| !pinned || *pin)
				.map(|(title, pin)| {
					let badge = if pin { "Pinned" } else { "Loose" };
					view! {
						<Card>
							<Text class="text-sm font-semibold text-slate-900">{title}</Text>
							<Text class="text-xs text-slate-500">{badge}</Text>
						</Card>
					}
				})
				.collect::<Fragment>()}
		</Frame>
	}
}

/// A Markdown reader for one short document.
#[component]
pub fn Markdown() -> Element {
	view! {
		<Frame title="Markdown">
			<Note text="Opens a file and shows it. This one is the sample chapter." />
			<Card>
				<Text class="text-base font-semibold text-slate-900">"Chapter 1"</Text>
				<Text class="text-sm text-slate-700">"Write the view once."</Text>
				<Text class="text-xs font-semibold text-slate-500">"let ready = true;"</Text>
			</Card>
		</Frame>
	}
}

const FACES: [&str; 3] = ["News", "Text", "Display"];

/// Phone screens as a type specimen.
#[component]
pub fn TypeTour() -> Element {
	let face = remember(|| 0_usize);
	let index = face.get();
	view! {
		<Frame title="Type tour">
			<Note text="Three faces, set as a specimen. No script runs this screen." />
			<View class="flex-row flex-wrap gap-2">
				<Choice label="News" on_press=move || face.set(0) />
				<Choice label="Text" on_press=move || face.set(1) />
				<Choice label="Display" on_press=move || face.set(2) />
			</View>
			<Card>
				<Text class="text-2xl font-bold text-slate-900">{FACES[index]}</Text>
				<Text class="text-sm text-slate-600">"The quick brown fox"</Text>
			</Card>
		</Frame>
	}
}

/// Text beside shapes, the way `shape-outside` would wrap it.
#[component]
pub fn Textflow() -> Element {
	let orb = remember(|| 0_usize);
	let index = orb.get();
	view! {
		<Frame title="Textflow">
			<Note text="Text beside a shape. Nudge the orb and the column stays." />
			<View class="flex-row gap-3 items-center">
				<View class="w-[72px] h-[72px] rounded-full bg-indigo-500" />
				<Text class="flex-1 text-sm text-slate-700">
					"The sentence keeps its place while the orb moves."
				</Text>
			</View>
			<Text class="text-sm font-semibold text-slate-900">{format!("Orb {index}")}</Text>
			<Choice label="Nudge orb" on_press=move || orb.update(|value| *value += 1) />
		</Frame>
	}
}

const STUDIES: [&str; 4] = [
	"Text around balls",
	"A dragon in the margin",
	"Masonry of captions",
	"A magazine spread",
];

/// Typography studies: pick one and read what it is about.
#[component]
pub fn Reflow() -> Element {
	let study = remember(|| 0_usize);
	let index = study.get();
	view! {
		<Frame title="Reflow">
			<Note text="Seven studies of type. Four of them fit this phone." />
			<View class="flex-row flex-wrap gap-2">
				<Choice label="Balls" on_press=move || study.set(0) />
				<Choice label="Dragon" on_press=move || study.set(1) />
				<Choice label="Masonry" on_press=move || study.set(2) />
				<Choice label="Spread" on_press=move || study.set(3) />
			</View>
			<Card>
				<Text class="text-sm font-semibold text-slate-900">{STUDIES[index]}</Text>
			</Card>
		</Frame>
	}
}
