//! Typed events: `#[derive(Event)]` names an event, lists its properties in
//! order, gives it a stable schema id, and the clients record it.

#![cfg(not(target_arch = "wasm32"))]

use leptoskit_analytics::Event;
use leptoskit_analytics::client::Client;
use leptoskit_analytics::client::Config;
use leptoskit_analytics::client::Outcome;
use leptoskit_analytics::client::Platform;
use leptoskit_analytics::props;
use leptoskit_analytics::wire;
use leptoskit_analytics::wire::Body;
use leptoskit_analytics::wire::Value;

#[derive(Event)]
#[event(name = "checkout_completed")]
struct CheckoutCompleted {
	plan: String,
	amount_cents: u32,
	coupon: Option<String>,
}

#[derive(Event)]
struct SignedUp;

#[test]
fn the_derive_names_the_event_and_lists_its_properties() {
	assert_eq!(CheckoutCompleted::NAME, "checkout_completed");
	assert_eq!(
		CheckoutCompleted::FIELDS,
		[
			("plan", "String"),
			("amount_cents", "u32"),
			("coupon", "Option<String>")
		]
	);
	assert_eq!(SignedUp::NAME, "signed_up", "named after the struct");
	assert_eq!(SignedUp::FIELDS.len(), 0);
	assert_ne!(CheckoutCompleted::SCHEMA_ID, SignedUp::SCHEMA_ID);
	// Stable across builds: the id is part of the wire format to come.
	assert_eq!(CheckoutCompleted::SCHEMA_ID, 0x9792_45b8_5425_9b97);

	let properties = CheckoutCompleted {
		plan: "pro".into(),
		amount_cents: 1999,
		coupon: None,
	}
	.into_properties();
	assert_eq!(
		properties,
		props! { "plan" => "pro", "amount_cents" => 1999_u32, "coupon" => Value::Null }
	);
}

struct Clock;

impl Platform for Clock {
	fn now(&self) -> i64 {
		1_762_646_400_000
	}

	fn fill_random(&self, bytes: &mut [u8]) {
		bytes.fill(7);
	}
}

#[test]
fn a_typed_event_is_recorded_like_any_other() {
	let mut client = Client::new(Clock, Config::default(), props! {});
	client.register(props! { "flavour" => "beta" });
	client.capture_event(CheckoutCompleted {
		plan: "team".into(),
		amount_cents: 4900,
		coupon: Some("SPRING".into()),
	});
	let pending = client.queue().take(0, [0; 10]).unwrap();
	client.queue().settle(Outcome::Stored, 0, 0.0);
	let batch = wire::decode(&pending.body, &wire::Limits::default()).unwrap();
	assert_eq!(
		batch.entries[0].body,
		Body::Event {
			name: "checkout_completed".into(),
			properties: props! {
				"plan" => "team",
				"amount_cents" => 4900_u32,
				"coupon" => "SPRING",
				"flavour" => "beta",
			},
		}
	);
}

#[test]
fn mistakes_are_compile_errors_that_say_what_to_write() {
	let cases = trybuild::TestCases::new();
	cases.compile_fail("tests/ui/*.rs");
}
