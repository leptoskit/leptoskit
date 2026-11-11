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
