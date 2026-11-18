//! Cached native app-usage client.
use leptoskit_bridge::Error;

use super::*;

/// Resolves the host once, then uses numeric method identifiers.
#[derive(Debug, Clone, Copy)]
pub struct AppUsage(leptoskit_bridge::guest::Client);
impl AppUsage {
	/// Resolves an explicitly installed host plugin.
	pub fn new() -> Result<Self, Error> {
		leptoskit_bridge::guest::Client::new(NAME).map(Self)
	}

	/// Reports whether the system's Usage Access screen has granted usage
	/// data. iOS answers granted: the lock and unlock events it supports
	/// need no grant.
	pub async fn usage_access_status(self) -> Result<UsageAccessStatus, Error> {
		UsageAccessStatus::decode(&self.0.call(USAGE_ACCESS_STATUS, &[])?.await?)
	}

	/// Opens the system's Usage Access screen so the app's owner can grant
	/// access themselves. Device Admin does not grant usage data; this is
	/// the right screen. Unavailable on iOS.
	pub async fn open_usage_access_settings(self) -> Result<(), Error> {
		self.empty(OPEN_USAGE_ACCESS_SETTINGS).await
	}

	/// Typed events inside `range`, at most `max` (1..=1024) of them,
	/// oldest first. Android queries the system's usage log; when the log
	/// holds more than `max` events in the range, the reply is capped and
	/// the app narrows the range or pages through it. On iOS this is a
	/// cancellable wait: it returns the lock and unlock transitions
	/// buffered while the app has been alive, waiting for the next one when
	/// the buffer is empty, and the range is ignored.
	pub async fn events(
		self,
		range: UsageRange,
		max: u32,
	) -> Result<Vec<UsageEvent>, Error> {
		if !(1..=MAX_EVENTS).contains(&max) {
			return Err(Error::InvalidArgument);
		}

		let mut payload = range.encode().to_vec();
		payload.extend_from_slice(&max.to_le_bytes());
		decode_events(&self.0.call(EVENTS, &payload)?.await?)
	}

	/// Per-app foreground sessions inside `range`, clipped to its edges and
	/// oldest first. Unavailable on iOS, which withholds per-app usage from
	/// third-party apps.
	pub async fn sessions(self, range: UsageRange) -> Result<Vec<UsageSession>, Error> {
		decode_sessions(&self.0.call(SESSIONS, &range.encode())?.await?)
	}

	/// The range's screen-on time, unlock count and per-app foreground
	/// totals. Unavailable on iOS.
	pub async fn summary(self, range: UsageRange) -> Result<UsageSummary, Error> {
		UsageSummary::decode(&self.0.call(SUMMARY, &range.encode())?.await?)
	}

	async fn empty(self, method: MethodId) -> Result<(), Error> {
		if self.0.call(method, &[])?.await?.is_empty() {
			Ok(())
		} else {
			Err(Error::InvalidArgument)
		}
	}
}
