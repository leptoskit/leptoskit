//! Caltrain, Weatherlight, and the map demo.

use leptoskit::prelude::*;

use crate::frame::Card;
use crate::frame::Choice;
use crate::frame::Frame;
use crate::frame::Note;

const STATIONS: [&str; 4] = ["San Francisco", "Palo Alto", "Mountain View", "San Jose"];

/// Nearby stations, a picked stop, and the next two trains.
#[component]
pub fn Caltrain() -> Element {
	let station = remember(|| 1_usize);
	let index = station.get();
	let name = STATIONS[index];
	view! {
		<Frame title="Caltrain">
			<Note text="A departure board for the peninsula. Search is a station." />
			<View class="flex-row flex-wrap gap-2">
				<Choice label="San Francisco" on_press=move || station.set(0) />
				<Choice label="Palo Alto" on_press=move || station.set(1) />
				<Choice label="Mountain View" on_press=move || station.set(2) />
				<Choice label="San Jose" on_press=move || station.set(3) />
			</View>
			<Card>
				<Text class="text-base font-semibold text-slate-900">
					{format!("{name} departures")}
				</Text>
				<Text class="text-sm text-slate-700">"Northbound · 12 min"</Text>
				<Text class="text-sm text-slate-700">"Southbound · 19 min"</Text>
			</Card>
		</Frame>
	}
}

const HOURS: [(&str, &str); 4] = [
	("Morning", "Clear, 14°"),
	("Noon", "Bright, 19°"),
	("Evening", "Wind off the bay, 16°"),
	("Night", "Fog, 13°"),
];

/// Weather for a chosen hour, without a network call.
#[component]
pub fn Weatherlight() -> Element {
	let hour = remember(|| 0_usize);
	let index = hour.get();
	view! {
		<Frame title="Weatherlight">
			<Note text="Oakland, for the hour you pick. The sky follows that hour." />
			<View class="flex-row flex-wrap gap-2">
				<Choice label="Morning" on_press=move || hour.set(0) />
				<Choice label="Noon" on_press=move || hour.set(1) />
				<Choice label="Evening" on_press=move || hour.set(2) />
				<Choice label="Night" on_press=move || hour.set(3) />
			</View>
			<Card>
				<Text class="text-base font-semibold text-slate-900">{HOURS[index].0}</Text>
				<Text class="text-sm text-slate-700">{HOURS[index].1}</Text>
			</Card>
		</Frame>
	}
}

const PINS: [&str; 3] = ["Ferry Building", "Dolores Park", "Lands End"];

/// Pins a native map would draw. This screen draws the list.
#[component]
pub fn MapDemo() -> Element {
	let pin = remember(|| 0_usize);
	let index = pin.get();
	view! {
		<Frame title="Map demo">
			<Note text="MapKit on Apple, a map on the web. Here, the same pins." />
			<View class="h-[180px] items-center justify-center rounded-xl bg-slate-200">
				<Text class="text-sm font-semibold text-slate-900">{PINS[index]}</Text>
			</View>
			<View class="flex-row flex-wrap gap-2">
				<Choice label="Ferry Building" on_press=move || pin.set(0) />
				<Choice label="Dolores Park" on_press=move || pin.set(1) />
				<Choice label="Lands End" on_press=move || pin.set(2) />
			</View>
		</Frame>
	}
}
