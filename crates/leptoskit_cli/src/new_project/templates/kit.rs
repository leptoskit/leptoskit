//! `/kit`: a few of the controls every platform draws.

use leptoskit::prelude::ActivityIndicator;
use leptoskit::prelude::Checkbox;
use leptoskit::prelude::Element;
use leptoskit::prelude::Get;
use leptoskit::prelude::LinearProgressIndicator;
use leptoskit::prelude::Progress;
use leptoskit::prelude::ProgressValue;
use leptoskit::prelude::RwSignal;
use leptoskit::prelude::Set;
use leptoskit::prelude::Show;
use leptoskit::prelude::Slider;
use leptoskit::prelude::SliderState;
use leptoskit::prelude::Switch;
use leptoskit::prelude::Text;
use leptoskit::prelude::TextInput;
use leptoskit::prelude::View;
use leptoskit::prelude::component;
use leptoskit_router::Route;

/// Names the route `Kit`.
#[derive(Route, Clone, Debug, PartialEq)]
pub struct Kit;

/// Shows the shared controls. Each one is a real widget on the host.
#[component]
pub fn Page() -> Element {
	let reminders = RwSignal::new(true);
	let choice = RwSignal::new(false);
	let note = RwSignal::new(String::new());
	let volume = RwSignal::new(40.0_f64);

	View()
		.class("flex-1 gap-6 bg-slate-950 p-6")
		.child(Text("The kit").class("text-3xl font-bold text-white"))
		.child(
			Text("These controls are the same component on the web, phones, and desktop.")
				.class("text-base text-slate-300"),
		)
		.child(row(
			"Reminders",
			Show(
				move || {
					let _ = reminders.get();
					true
				},
				move || Switch(reminders.get()).on_toggle(move |checked| reminders.set(checked)),
			),
		))
		.child(row(
			"Done",
			Show(
				move || {
					let _ = choice.get();
					true
				},
				move || Checkbox(choice.get()).on_toggle(move |checked| choice.set(checked)),
			),
		))
		.child(
			TextInput()
				.class("rounded-xl bg-slate-900 px-4 py-3 text-white")
				.placeholder("Write something")
				.on_change(move |text| note.set(text.to_owned())),
		)
		.child(
			Text(move || {
				let text = note.get();

				if text.is_empty() {
					"Nothing written yet".to_owned()
				} else {
					text
				}
			})
			.class("text-base text-white"),
		)
		.child(row(
			"Level",
			Show(
				move || {
					let _ = volume.get();
					true
				},
				move || {
					Slider(
						SliderState::new(0.0, 100.0, volume.get(), None)
							.expect("the starter slider stays inside its range"),
					)
					.on_value_change(move |next| volume.set(next))
				},
			),
		))
		.child(
			LinearProgressIndicator(Progress::Determinate(
				ProgressValue::new(0.6).expect("0.6 is a completion fraction"),
			))
			.class("w-full"),
		)
		.child(ActivityIndicator(true))
}

fn row(label: &str, control: impl Into<Element>) -> Element {
	View()
		.class("flex-row items-center justify-between gap-4 rounded-2xl bg-slate-900 p-4")
		.child(Text(label).class("text-base text-white"))
		.child(control.into())
}
