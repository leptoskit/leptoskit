//! Product analytics for leptoskit apps.
//!
//! Without features, this crate is the client apps record events with
//! ([`client`]) and the batch format it sends them in ([`wire`]), and
//! depends on nothing, so it adds little to an app's bundle. With the `service`
//! feature, it's the analytics service itself: a process of its own, with its
//! own database, which `lk` runs when an app declares `[services.analytics]`.
//!
//! The design, and what's built so far, is in the repository's
//! `docs/agents/analytics-service.md`.

pub mod client;
pub mod wire;

#[cfg(feature = "service")]
pub mod service;

/// The service's name: the `[services.analytics]` table in the app's
/// `leptoskit.toml` must use it, because every operation is addressed to it.
pub const SERVICE: &str = "analytics";

/// An event declared as a Rust type, so its name and properties are checked
/// when the app compiles: a renamed or retyped property is a compile error,
/// not a gap in the data.
///
/// Derive it with the `derive` feature:
///
/// ```ignore
/// #[derive(leptoskit_analytics::Event)]
/// #[event(name = "checkout_completed")]
/// struct CheckoutCompleted {
/// 	plan: String,
/// 	amount_cents: u32,
/// }
///
/// client.capture_event(CheckoutCompleted { plan: "pro".into(), amount_cents: 1999 });
/// ```
///
/// Without `name`, the event is named after the struct, in `snake_case`. The
/// name follows the service's rules: not blank, at most 200 bytes, no control
/// characters, and no leading `$`, which leptoskit's own events use.
/// Each field becomes a property of the same name (a field written `r#type`
/// is the property `type`); its type must convert into [`wire::Value`].
pub trait Event {
	/// The event's name.
	const NAME: &'static str;
	/// Each property's name and type, as declared.
	const FIELDS: &'static [(&'static str, &'static str)];
	/// A hash of the name and the fields' names and types: the same
	/// declaration always has the same id, and any change to it changes it.
	/// Types are hashed as spelled, so `String` and `std::string::String`
	/// differ.
	const SCHEMA_ID: u64;

	/// The event's properties, in field order.
	fn into_properties(self) -> wire::Properties;

	/// The event's values, in field order, without their names: what a
	/// compact event sends when the `typed` feature is enabled.
	fn into_values(self) -> Vec<wire::Value>;
}

/// The schema id of an event named `name` with `fields` (each property's
/// name and its type as spelled, without whitespace): FNV-1a over each,
/// NUL-separated. What `#[derive(Event)]` computes at compile time, and
/// what the service checks a registered schema against.
pub fn schema_id(name: &str, fields: &[(&str, &str)]) -> u64 {
	let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
	let mut eat = |text: &str| {
		for byte in text.bytes().chain([0]) {
			hash ^= u64::from(byte);
			hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
		}
	};
	eat(name);
	for (field, ty) in fields {
		eat(field);
		eat(ty);
	}
	hash
}

#[cfg(feature = "derive")]
pub use leptoskit_analytics_macros::Event;

/// A typed event's schema, as `#[derive(Event)]` records it in a server
/// build: what the app registers in the service's event catalogue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EventSchema {
	pub name: &'static str,
	pub schema_id: u64,
	pub fields: &'static [(&'static str, &'static str)],
}

#[cfg(all(feature = "derive", not(target_arch = "wasm32")))]
inventory::collect!(EventSchema);

/// Every typed event this program was built with, outside wasm: what the
/// app's server registers in the service's event catalogue when it starts.
/// Browser and native builds don't collect them, so their bundles don't
/// grow.
#[cfg(all(feature = "derive", not(target_arch = "wasm32")))]
pub fn compiled_events() -> impl Iterator<Item = &'static EventSchema> {
	inventory::iter::<EventSchema>.into_iter()
}

#[cfg(all(feature = "derive", not(target_arch = "wasm32")))]
#[doc(hidden)]
pub use inventory as __inventory;
