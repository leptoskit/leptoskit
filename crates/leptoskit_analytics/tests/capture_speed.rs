//! What one `capture` costs on the device: the client compiled natively,
//! recording a five-property event (§16 of the design: under 2 µs).
//!
//! Ignored by default; run it in release:
//! `cargo nextest run --release -p leptoskit_analytics --test capture_speed
//! --run-ignored only --no-capture`.

#![cfg(not(target_arch = "wasm32"))]

use std::cell::Cell;
use std::time::Instant;

use leptoskit_analytics::client::Client;
use leptoskit_analytics::client::Config;
use leptoskit_analytics::client::Limits;
use leptoskit_analytics::client::Outcome;
use leptoskit_analytics::client::Platform;
use leptoskit_analytics::props;

struct Clock(Cell<i64>);

impl Platform for Clock {
	fn now(&self) -> i64 {
		self.0.set(self.0.get() + 1);
		self.0.get()
	}

	fn fill_random(&self, bytes: &mut [u8]) {
		bytes.fill(self.0.get() as u8);
	}
}

#[test]
#[ignore = "a benchmark: run it in release"]
fn capture_takes_under_two_microseconds() {
	const EVENTS: u32 = 100_000;
	let config = Config {
		limits: Limits {
			queue_entries: EVENTS as usize,
			queue_bytes: usize::MAX,
			..Limits::default()
		},
		..Config::default()
	};
	let mut client = Client::new(
		Clock(Cell::new(1_762_646_400_000)),
		config,
		props! { "$lib" => "bench" },
	);
	client.register(props! { "flavour" => "beta" });

	let started = Instant::now();
	for n in 0..EVENTS {
		client.capture(
			"checkout_step",
			props! {
				"step" => n % 4,
				"plan" => "pro",
				"amount_cents" => 1999,
				"trial" => false,
				"$pathname" => "/checkout",
			},
		);
	}
	let per_capture = started.elapsed() / EVENTS;

	let started = Instant::now();
	let mut encoded = 0;
	while let Some(pending) = client.queue().take(0, [0; 10]) {
		encoded += pending.body.len();
		client.queue().settle(Outcome::Stored, 0, 0.0);
	}
	let per_event_encoded = started.elapsed() / EVENTS;

	println!(
		"capture: {per_capture:?} per event; encoding batches: {per_event_encoded:?} per event, \
		 {} bytes per event",
		encoded / EVENTS as usize
	);
	assert!(per_capture.as_nanos() < 2_000, "{per_capture:?}");
}
