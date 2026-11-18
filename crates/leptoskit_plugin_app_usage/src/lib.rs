//! <!-- {=p_appUsageSummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_app_usage` as a direct dependency and select the
//! `app-usage` plugin in `native.plugins`; supported generated hosts discover
//! it from Cargo metadata. Generated Windows, Linux, and terminal hosts
//! currently omit runtime plugins, so use explicit registration in a custom
//! host there. Guests enable the `guest` feature and reuse `guest::AppUsage`
//! rather than resolving the plugin on every call. Android reports typed
//! usage events, foreground sessions and screen-time summaries from Usage
//! Access, which the app's owner grants on the system's own screen; iOS and
//! macOS give third-party apps no per-app usage, so sessions and summaries are
//! unavailable and events are the lock and unlock transitions buffered while
//! the app is alive.
//!
//! <!-- {/p_appUsageSummary} -->

use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;

use crate::wire::Reader;

mod types;
mod wire;

pub use types::*;

/// Stable native registration name.
pub const NAME: &str = "app-usage";
/// Usage Access state, empty request; never opens the system's screen.
pub const USAGE_ACCESS_STATUS: MethodId = MethodId(0);
/// Opens the system's Usage Access screen, empty request; empty reply.
pub const OPEN_USAGE_ACCESS_SETTINGS: MethodId = MethodId(1);
/// A [`UsageRange`] and a `u32` event cap: typed events inside the range.
/// On iOS a cancellable wait for buffered lock and unlock events.
pub const EVENTS: MethodId = MethodId(2);
/// A [`UsageRange`]: foreground [`UsageSession`]s clipped to its edges.
/// Unavailable on iOS.
pub const SESSIONS: MethodId = MethodId(3);
/// A [`UsageRange`]: screen-on time, unlock count and per-app totals.
/// Unavailable on iOS.
pub const SUMMARY: MethodId = MethodId(4);
/// Exported independently grantable operations.
pub const METHODS: &[MethodId] = &[
	USAGE_ACCESS_STATUS,
	OPEN_USAGE_ACCESS_SETTINGS,
	EVENTS,
	SESSIONS,
	SUMMARY,
];

/// Decodes an events reply: a little-endian count, then that many events.
pub fn decode_events(payload: &[u8]) -> Result<Vec<UsageEvent>, Error> {
	let mut reader = Reader::new(payload);
	let count = reader.u32()?;

	if count > MAX_EVENTS {
		return Err(Error::InvalidArgument);
	}

	let mut events = Vec::with_capacity(count as usize);

	for _ in 0..count {
		events.push(UsageEvent::read(&mut reader)?);
	}

	reader.finish()?;
	Ok(events)
}

/// Decodes a sessions reply: a little-endian count, then that many sessions.
pub fn decode_sessions(payload: &[u8]) -> Result<Vec<UsageSession>, Error> {
	let mut reader = Reader::new(payload);
	let count = reader.u32()?;

	if count > MAX_SESSIONS {
		return Err(Error::InvalidArgument);
	}

	let mut sessions = Vec::with_capacity(count as usize);

	for _ in 0..count {
		sessions.push(UsageSession::read(&mut reader)?);
	}

	reader.finish()?;
	Ok(sessions)
}

/// Per-app usage does not exist in the browser: a page cannot see other
/// tabs, let alone other apps, and its own lifetime is all it can measure.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
#[expect(
	clippy::unused_async,
	clippy::unused_async_trait_impl,
	reason = "the stubs stay async so browser and native callers share one API"
)]
pub mod web {
	use leptoskit_bridge::Error;

	use super::UsageAccessStatus;
	use super::UsageEvent;
	use super::UsageRange;
	use super::UsageSession;
	use super::UsageSummary;

	/// Browser-side app-usage client; every operation reports
	/// [`Error::Unavailable`].
	#[derive(Debug, Clone, Copy, Default)]
	pub struct AppUsage;
	impl AppUsage {
		/// App usage does not exist in the browser.
		pub const fn new() -> Self {
			Self
		}

		/// Unavailable: the browser has no usage data.
		pub async fn usage_access_status(self) -> Result<UsageAccessStatus, Error> {
			Err(Error::Unavailable)
		}

		/// Unavailable: the browser has no usage-access screen.
		pub async fn open_usage_access_settings(self) -> Result<(), Error> {
			Err(Error::Unavailable)
		}

		/// Unavailable: the browser has no usage events.
		pub async fn events(self, _range: UsageRange, _max: u32) -> Result<Vec<UsageEvent>, Error> {
			Err(Error::Unavailable)
		}

		/// Unavailable: the browser has no foreground sessions.
		pub async fn sessions(self, _range: UsageRange) -> Result<Vec<UsageSession>, Error> {
			Err(Error::Unavailable)
		}

		/// Unavailable: the browser has no screen-time summaries.
		pub async fn summary(self, _range: UsageRange) -> Result<UsageSummary, Error> {
			Err(Error::Unavailable)
		}
	}
}

#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest;

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn methods_are_dense_and_in_order() {
		for (index, method) in METHODS.iter().enumerate() {
			assert_eq!(method.0 as usize, index);
		}
	}

	#[cfg(not(target_arch = "wasm32"))]
	#[test]
	fn every_method_has_a_declared_constant() {
		assert_eq!(USAGE_ACCESS_STATUS, MethodId(0));
		assert_eq!(OPEN_USAGE_ACCESS_SETTINGS, MethodId(1));
		assert_eq!(EVENTS, MethodId(2));
		assert_eq!(SESSIONS, MethodId(3));
		assert_eq!(SUMMARY, MethodId(4));
	}

	fn event(kind: EventKind, timestamp_ms: i64, package: Option<&str>) -> UsageEvent {
		UsageEvent {
			kind,
			timestamp_ms,
			package: package.map(str::to_owned),
		}
	}

	/// Builds a batch reply the way the handlers do.
	fn batch(entries: &[&[u8]]) -> Vec<u8> {
		let mut payload = (entries.len() as u32).to_le_bytes().to_vec();

		for entry in entries {
			payload.extend_from_slice(entry);
		}

		payload
	}

	#[test]
	fn event_batches_round_trip_in_order() {
		let entries: Vec<_> = [
			event(EventKind::ActivityResumed, 1, Some("com.example.chat")),
			event(EventKind::KeyguardShown, 2, None),
			event(EventKind::ActivityPaused, 3, Some("com.example.chat")),
		]
		.iter()
		.map(UsageEvent::encode)
		.collect::<Result<Vec<_>, _>>()
		.unwrap();
		let references: Vec<_> = entries.iter().map(Vec::as_slice).collect();
		let decoded = decode_events(&batch(&references)).unwrap();

		assert_eq!(
			decoded,
			vec![
				event(EventKind::ActivityResumed, 1, Some("com.example.chat")),
				event(EventKind::KeyguardShown, 2, None),
				event(EventKind::ActivityPaused, 3, Some("com.example.chat")),
			]
		);
		assert_eq!(decode_events(&[0, 0, 0, 0]), Ok(Vec::new()));
	}

	#[test]
	fn event_batches_reject_oversized_counts_and_bad_framing() {
		let entry = event(EventKind::Unlocked, 1, None).encode().unwrap();

		for count in [(MAX_EVENTS + 1).to_le_bytes(), u32::MAX.to_le_bytes()] {
			let mut oversized = count.to_vec();
			oversized.extend_from_slice(&entry);
			assert_eq!(decode_events(&oversized), Err(Error::InvalidArgument));
		}

		for length in 0..4 {
			assert_eq!(decode_events(&entry[..length]), Err(Error::InvalidArgument));
		}

		let mut trailing = batch(&[&entry]);
		trailing.push(0);
		assert_eq!(decode_events(&trailing), Err(Error::InvalidArgument));
	}

	#[test]
	fn session_batches_round_trip_in_order() {
		let session = UsageSession {
			package: "com.example.chat".to_owned(),
			label: "Chat".to_owned(),
			start_ms: 1,
			end_ms: 2,
		};
		let entry = session.encode().unwrap();
		let decoded = decode_sessions(&batch(&[&entry, &entry])).unwrap();

		assert_eq!(decoded, vec![session.clone(), session]);
		assert_eq!(decode_sessions(&[0, 0, 0, 0]), Ok(Vec::new()));

		let mut truncated = batch(&[&entry]);
		truncated.pop();
		assert_eq!(decode_sessions(&truncated), Err(Error::InvalidArgument));
	}
}
