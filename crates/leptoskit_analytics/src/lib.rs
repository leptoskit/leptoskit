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
/// Without `name`, the event is named after the struct, in `snake_case`.
/// Each field becomes a property of the same name; its type must convert
/// into [`wire::Value`].
pub trait Event {
	/// The event's name.
	const NAME: &'static str;
	/// Each property's name and type, as declared.
	const FIELDS: &'static [(&'static str, &'static str)];
	/// A hash of the name and the fields' names and types: the same
	/// declaration always has the same id, and any change to it changes it.
	const SCHEMA_ID: u64;

	/// The event's properties, in field order.
	fn into_properties(self) -> wire::Properties;
}

#[cfg(feature = "derive")]
pub use leptoskit_analytics_macros::Event;
