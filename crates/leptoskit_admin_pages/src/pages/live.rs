//! Live: events as they arrive, a few seconds after they happen. The page
//! starts with the latest events the server read, then adds each one the
//! admin's stream brings at the top.

#[cfg(all(feature = "hydrate", target_arch = "wasm32"))]
use leptos::prelude::Effect;
use leptos::prelude::GetValue as _;
use leptos::prelude::SetValue as _;
use leptos::prelude::StoredValue;
use leptoskit::native::Role;
use leptoskit::prelude::*;

use crate::analytics::format;
use crate::analytics::tail::Tail;
use crate::components;
use crate::components::Kind;
use crate::components::Tone;
use crate::model::Pulse;
use crate::model::live::Arrival;
use crate::model::live::Live;
use crate::style;

/// The most events the page holds.
const KEEP: usize = 200;

/// The list as it stands: what it shows, what waits while it's paused,
/// and what the person opened or typed.
#[derive(Clone, Copy)]
pub(crate) struct Watch {
	/// Each event with the key it keeps while it's listed, newest first.
	rows: RwSignal<Vec<(u64, Arrival)>>,
	/// Events that came while paused, oldest first.
	waiting: RwSignal<Vec<Arrival>>,
	paused: RwSignal<bool>,
	filter: RwSignal<String>,
	/// The keys of the rows whose properties show.
	expanded: RwSignal<Vec<u64>>,
	next: StoredValue<u64>,
}

impl Watch {
	/// The list the server drew, following the page's tail when it has one.
	pub(crate) fn new(live: &Live) -> Self {
		let watch = Self {
			rows: RwSignal::new(
				(0_u64..)
					.zip(live.events.iter().take(KEEP).cloned())
					.collect(),
			),
			waiting: RwSignal::new(Vec::new()),
			paused: RwSignal::new(false),
			filter: RwSignal::new(String::new()),
			expanded: RwSignal::new(Vec::new()),
			next: StoredValue::new(live.events.len() as u64),
		};

		// The list follows the stream in the browser. `Effect::new` calls
		// `spawn_local`, which panics on the server runtime, and `hydrate`
		// is also on for a host `--all-features` build.
		#[cfg(all(feature = "hydrate", target_arch = "wasm32"))]
		if let Some(tail) = Tail::current() {
			Effect::new(move |_| {
				let arrived = tail.take();

				if arrived.is_empty() {
					return;
				}

				if watch.paused.get_untracked() {
					watch.waiting.update(|waiting| waiting.extend(arrived));
				} else {
					watch.add(arrived);
				}
			});
		}

		watch
	}

	/// Puts `arrived`, oldest first, at the top of the list.
	fn add(self, arrived: Vec<Arrival>) {
		let mut key = self.next.get_value();

		self.rows.update(|rows| {
			for event in arrived {
				rows.insert(0, (key, event));
				key += 1;
			}

			rows.truncate(KEEP);
		});
		self.next.set_value(key);
	}

	/// Holds the list still, or lets what waited in.
	fn pause(self) {
		if self.paused.get_untracked() {
			let waited = self.waiting.try_update(std::mem::take).unwrap_or_default();
			self.paused.set(false);
			self.add(waited);
		} else {
			self.paused.set(true);
		}

		crate::shell::preference(r#"{"kind":"use_note","note":"live-pause"}"#);
	}

	fn toggle(self, key: u64) {
		self.expanded.update(|expanded| {
			if let Some(at) = expanded.iter().position(|open| *open == key) {
				expanded.remove(at);
			} else {
				expanded.push(key);
			}
		});
	}
}

/// One event's row: time, name, who, and its properties when opened.
fn row(event: &Arrival, expanded: bool, toggle: Option<Box<dyn Fn() + Send + Sync>>) -> Element {
	let at = format::instant(&event.timestamp)
		.map(|at| format::clock(&at) + &format!(":{:02}", chrono::Timelike::second(&at)))
		.unwrap_or_default();
	let detail = event
		.properties
		.get("$pathname")
		.map(format::value)
		.unwrap_or_default();
	let mut title = View()
		.class("min-w-0 flex-1 flex-row items-center gap-1.5")
		.child(
			Text(event.name.clone())
				.class(style::CODE)
				.class("truncate font-semibold"),
		);

	if event.typed {
		title = title.child(components::struct_badge().test_id("struct-badge"));
	}

	let mut pressable = Pressable(format!("{} from {} at {at}", event.name, event.distinct_id))
		.aria_expanded(expanded)
		.class("cursor-pointer hover:bg-[var(--admin-sunken)]")
		.test_id("how-toggle")
		.child(
			View()
				.class("flex-row items-center gap-3 px-3 py-2")
				.child(
					Text(at).class("w-20 shrink-0 text-xs tabular-nums text-[var(--admin-ink-3)]"),
				)
				.child(title)
				.child(
					Text(detail)
						.class("w-40 truncate text-xs text-[var(--admin-ink-2)] max-md:hidden"),
				)
				.child(Text(event.distinct_id.clone()).class(
					"w-44 truncate text-end text-xs text-[var(--admin-ink-3)] max-sm:hidden",
				)),
		);

	if let Some(toggle) = toggle {
		pressable = pressable.on_press(toggle);
	}

	let mut row = View()
		.class("flex-col border-b border-[var(--admin-line)]")
		.test_id("live-event")
		.aria_description(event.name.clone())
		.child(pressable);

	if expanded {
		row = row.child(View().class("px-3 pb-3").test_id("how-panel").child(
			components::paper_code(
				&serde_json::to_string_pretty(&event.properties).unwrap_or_default(),
				"JSON",
			),
		));
	}

	row
}

/// How fast events arrive, with the stream's state.
fn rate(pulse: Option<&Pulse>) -> Element {
	let (per_second, per_minute, seconds) = pulse.map_or((0.0, 0, &[][..]), |pulse| {
		(pulse.per_second, pulse.per_minute, pulse.seconds.as_slice())
	});
	let status = match pulse {
		Some(Pulse { live: true, .. }) => components::status_word(Tone::Good, "Listening"),
		Some(Pulse {
			reason: Some(reason),
			..
		}) => components::status_word(Tone::Warn, format!("Reconnecting: {reason}")),
		_ => components::status_word(Tone::Info, "Connecting"),
	};
	#[expect(clippy::cast_precision_loss, reason = "counts for drawing")]
	let seconds: Vec<f64> = seconds.iter().map(|count| *count as f64).collect();

	View()
		.class("flex-row flex-wrap items-center gap-4")
		.child(
			View()
				.class("flex-col")
				.child(
					Text(format!("{per_second:.1}/s"))
						.class(style::HERO_NUMBER)
						.test_id("live-rate"),
				)
				.child(Text("events per second, last 10 s").class(style::LABEL)),
		)
		.child(
			View()
				.class("flex-col")
				.child(
					Text(format::thousands(per_minute))
						.class("text-2xl font-semibold tabular-nums text-[var(--admin-ink)]")
						.test_id("live-minute"),
				)
				.child(Text("in the last minute").class(style::LABEL)),
		)
		.child(crate::analytics::draw::sparkline(&seconds, "h-10 w-48").test_id("live-spark"))
		.child(View().class("grow"))
		.child(status.test_id("live-status"))
}

/// The Live page. Without `watch`, the list the server read, still.
pub(crate) fn page(live: &Live, watch: Option<Watch>) -> Element {
	let pulse = Tail::current().and_then(|tail| tail.pulse());
	let filter = watch
		.map(|watch| watch.filter.get().trim().to_lowercase())
		.unwrap_or_default();
	let paused = watch.is_some_and(|watch| watch.paused.get());
	let waiting = watch.map_or(0, |watch| watch.waiting.with(Vec::len));
	let expanded = watch.map(|watch| watch.expanded.get()).unwrap_or_default();
	let rows: Vec<(u64, Arrival)> = watch.map_or_else(
		|| (0_u64..).zip(live.events.iter().cloned()).collect(),
		|watch| watch.rows.get(),
	);
	let list = View()
		.class("flex-col")
		.role(Role::Log)
		.aria_label("Events as they arrive")
		.test_id("live-events")
		.children(
			rows.iter()
				.filter(|(_, event)| {
					filter.is_empty() || event.name.to_lowercase().contains(&filter)
				})
				.map(|(key, event)| {
					let key = *key;
					row(
						event,
						expanded.contains(&key),
						watch.map(|watch| -> Box<dyn Fn() + Send + Sync> {
							Box::new(move || watch.toggle(key))
						}),
					)
				}),
		);
	let mut search = TextInput()
		.value(watch.map(|watch| watch.filter.get()).unwrap_or_default())
		.placeholder("Filter by event name")
		.aria_label("Filter by event name")
		.class(
			"min-w-0 flex-1 rounded-lg border border-[var(--admin-line-strong)] \
			 bg-[var(--admin-surface)] px-3 py-2 text-sm text-[var(--admin-ink)]",
		)
		.test_id("live-filter");
	let mut pause = Button(match (paused, waiting) {
		(false, _) => "Pause".to_owned(),
		(true, 0) => "Resume".to_owned(),
		(true, waiting) => format!("Resume ({waiting})"),
	})
	.aria_pressed(paused)
	.class(Kind::Secondary.class_name())
	.test_id("live-pause");

	if let Some(watch) = watch {
		search = search.on_change(move |text| watch.filter.set(text.to_owned()));
		pause = pause.on_press(move || watch.pause());
	}

	let controls = View()
		.class("flex-row flex-wrap items-center gap-3")
		.child(search)
		.child(pause);
	let mut card = components::card("Events")
		.aside("newest first")
		.workings("live", "live_events")
		.child(controls);

	if live.pause_note {
		card = card.child(components::margin_note(
			"live-pause",
			"Pause holds the list still while you read; new events wait and come in when you \
			 resume.",
		));
	}

	View()
		.class("flex-col gap-5")
		.test_id("live-page")
		.child(components::header(
			"Live",
			"events as they arrive, a few seconds after they happen",
			Vec::new(),
		))
		.child(View().class(style::CARD).child(rate(pulse.as_ref())))
		.child(card.child(list).build())
}
