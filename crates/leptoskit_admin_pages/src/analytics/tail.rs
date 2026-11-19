//! The admin's analytics stream, on any page: the pulse in the top bar,
//! and on Live, each event as it arrives.
//!
//! The admin's server follows the analytics service's `live_events` tail
//! once and shares it; a page opens [`analytics_live`](crate::ops) on the
//! admin's own live connection and reads its share from there.

use leptoskit::prelude::*;

use crate::model::Pulse;
use crate::model::live::Arrival;

/// The stream, as a page reads it. Without the page's Wasm, or with no
/// analytics service, it holds what the server drew and never changes.
#[derive(Clone, Copy)]
pub struct Tail {
	/// How fast events arrive, when the admin is connected.
	pulse: RwSignal<Option<Pulse>>,
	/// How many times the pulse came, changed or not: about once a second.
	#[cfg_attr(
		not(feature = "hydrate"),
		expect(dead_code, reason = "only the browser re-reads a builder")
	)]
	beats: RwSignal<u64>,
	/// Events that arrived and no page has taken yet, oldest first.
	arrived: RwSignal<Vec<Arrival>>,
}

impl Tail {
	/// Follows the stream when the admin is connected (`pulse` is the
	/// server's reading), with each event when `events`.
	#[must_use]
	pub(crate) fn open(pulse: Option<Pulse>, events: bool) -> Self {
		let connected = pulse.is_some();
		let tail = Self {
			pulse: RwSignal::new(pulse),
			beats: RwSignal::new(0),
			arrived: RwSignal::new(Vec::new()),
		};

		if connected {
			tail.follow(events);
		}

		tail
	}

	#[cfg(feature = "hydrate")]
	fn follow(self, events: bool) {
		use leptos::prelude::Effect;

		use crate::ops::Live;

		crate::live_connection();
		let stream = leptoskit::rpc::use_stream(move || crate::ops::AnalyticsLive { events });

		Effect::new(move |_| {
			let items = stream.items().get();

			if items.is_empty() {
				return;
			}

			// The handle keeps every item; this page needs each one once.
			stream.clear();

			for item in items {
				match item {
					Live::Pulse(pulse) => {
						self.beats.update(|beats| *beats += 1);

						if self
							.pulse
							.with_untracked(|shown| shown.as_ref() != Some(&pulse))
						{
							self.pulse.set(Some(pulse));
						}
					}
					Live::Event(event) => self.arrived.update(|arrived| arrived.push(event)),
				}
			}
		});
	}

	#[cfg(not(feature = "hydrate"))]
	fn follow(self, events: bool) {
		let _ = (self, events);
	}

	/// How fast events arrive now. Reading it rebuilds the screen when it
	/// changes.
	#[must_use]
	pub fn pulse(&self) -> Option<Pulse> {
		self.pulse.get()
	}

	/// The pulse's newest event number, without tracking it.
	#[must_use]
	#[cfg_attr(
		not(feature = "hydrate"),
		expect(dead_code, reason = "only the browser re-reads a builder")
	)]
	pub(crate) fn latest_untracked(&self) -> Option<u64> {
		self.pulse
			.with_untracked(|pulse| pulse.as_ref().map(|pulse| pulse.latest))
	}

	/// How many pulses have come. An effect that reads it runs about once a
	/// second while the stream flows.
	#[must_use]
	#[cfg_attr(
		not(feature = "hydrate"),
		expect(dead_code, reason = "only the browser re-reads a builder")
	)]
	pub(crate) fn beats(&self) -> u64 {
		self.beats.get()
	}

	/// The events that arrived since the last call, oldest first. An effect
	/// that calls it runs again when more arrive.
	pub(crate) fn take(&self) -> Vec<Arrival> {
		let arrived = self.arrived.get();

		if !arrived.is_empty() {
			self.arrived.set(Vec::new());
		}

		arrived
	}

	/// The page's tail, when the shell opened one.
	#[must_use]
	pub fn current() -> Option<Self> {
		leptos::context::use_context::<Self>()
	}
}
