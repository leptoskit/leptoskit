//! Usage access state, ranges, typed events, sessions and summaries.

use leptoskit_bridge::Error;

use crate::wire::Reader;
use crate::wire::Writer;

/// Whether the special Usage Access screen has granted this app the system's
/// usage data. iOS reports granted: nothing gates the lock and unlock
/// notifications it supports, and there is no usage-access concept to check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageAccessStatus {
	/// The platform cannot answer yet.
	Unknown = 0,
	/// Usage data is readable, or the supported events need no grant.
	Granted = 1,
	/// The user has not granted usage data access.
	Denied = 2,
}

impl UsageAccessStatus {
	/// Decodes the status reply.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		let mut reader = Reader::new(payload);
		let status = match reader.u32()? {
			0 => Self::Unknown,
			1 => Self::Granted,
			2 => Self::Denied,
			_ => return Err(Error::InvalidArgument),
		};

		reader.finish()?;
		Ok(status)
	}

	/// Encodes the status reply, for handlers and tests.
	pub fn encode(self) -> Vec<u8> {
		Writer::default().u32(self as u32).finish()
	}
}

/// The closed interval a historical query runs over, in Unix milliseconds.
///
/// Android clips every event, session and total to these edges. iOS ignores
/// the range: it has no history to query, so its events are the ones
/// buffered while the app is alive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsageRange {
	/// First millisecond included, which must not precede the epoch.
	pub since_ms: i64,
	/// Last millisecond included, which must not precede `since_ms`.
	pub until_ms: i64,
}

impl UsageRange {
	/// Rejects an empty, negative or reversed range before any platform
	/// call.
	pub fn new(since_ms: i64, until_ms: i64) -> Result<Self, Error> {
		if since_ms < 0 || until_ms < since_ms {
			return Err(Error::InvalidArgument);
		}

		Ok(Self { since_ms, until_ms })
	}

	/// Encodes the range every query carries.
	pub fn encode(self) -> [u8; 16] {
		let mut bytes = [0; 16];
		bytes[..8].copy_from_slice(&self.since_ms.to_le_bytes());
		bytes[8..].copy_from_slice(&self.until_ms.to_le_bytes());
		bytes
	}

	/// Decodes and validates the range a platform handler received.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		if payload.len() != 16 {
			return Err(Error::InvalidArgument);
		}

		Self::new(
			i64::from_le_bytes(
				payload[..8]
					.try_into()
					.map_err(|_| Error::InvalidArgument)?,
			),
			i64::from_le_bytes(
				payload[8..]
					.try_into()
					.map_err(|_| Error::InvalidArgument)?,
			),
		)
	}
}

/// One typed usage event. Android reports every kind; iOS reports only
/// [`EventKind::Locked`] and [`EventKind::Unlocked`] while the app is
/// alive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageEvent {
	/// Which system transition happened.
	pub kind: EventKind,
	/// When it happened, in Unix milliseconds.
	pub timestamp_ms: i64,
	/// The app the transition belongs to. Only the activity events carry
	/// one; every other event is device-wide.
	pub package: Option<String>,
}

/// The typed transitions the plugin reports, in wire order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum EventKind {
	/// An app's activity came to the foreground.
	ActivityResumed = 0,
	/// An app's activity left the foreground.
	ActivityPaused = 1,
	/// The screen became interactive.
	ScreenInteractive = 2,
	/// The screen stopped being interactive.
	ScreenNonInteractive = 3,
	/// The lock screen was shown.
	KeyguardShown = 4,
	/// The lock screen was dismissed.
	KeyguardHidden = 5,
	/// The device finished booting.
	DeviceStartup = 6,
	/// The device began shutting down.
	DeviceShutdown = 7,
	/// iOS: protected data is about to become unavailable, the device is
	/// locking.
	Locked = 8,
	/// iOS: protected data became available again, the device unlocked.
	Unlocked = 9,
}

impl EventKind {
	/// Decodes an event-kind code.
	pub fn from_code(code: u32) -> Result<Self, Error> {
		Ok(match code {
			0 => Self::ActivityResumed,
			1 => Self::ActivityPaused,
			2 => Self::ScreenInteractive,
			3 => Self::ScreenNonInteractive,
			4 => Self::KeyguardShown,
			5 => Self::KeyguardHidden,
			6 => Self::DeviceStartup,
			7 => Self::DeviceShutdown,
			8 => Self::Locked,
			9 => Self::Unlocked,
			_ => return Err(Error::InvalidArgument),
		})
	}

	/// Whether this transition belongs to one app, and so carries a
	/// package.
	#[must_use]
	pub const fn carries_package(self) -> bool {
		matches!(self, Self::ActivityResumed | Self::ActivityPaused)
	}
}

impl UsageEvent {
	/// Encodes one event, for handlers and tests.
	pub fn encode(&self) -> Result<Vec<u8>, Error> {
		let package = self.package.as_deref();
		if self.kind.carries_package() != package.is_some() {
			return Err(Error::InvalidArgument);
		}

		if package.is_some_and(|package| package.len() > MAX_PACKAGE_BYTES) {
			return Err(Error::TooLarge);
		}

		if self.timestamp_ms < 0 {
			return Err(Error::InvalidArgument);
		}

		Ok(Writer::default()
			.u32(self.kind as u32)
			.i64(self.timestamp_ms)
			.optional(package)?
			.finish())
	}

	/// Decodes one event, holding activity transitions to their package and
	/// every other kind to none.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		let mut reader = Reader::new(payload);
		let event = Self::read(&mut reader)?;
		reader.finish()?;
		Ok(event)
	}

	/// Reads one event from an open batch, without consuming its end.
	pub(crate) fn read(reader: &mut Reader<'_>) -> Result<Self, Error> {
		let kind = EventKind::from_code(reader.u32()?)?;
		let timestamp_ms = reader.i64()?;
		let package = reader.optional()?.map(str::to_owned);

		if kind.carries_package() != package.is_some() || timestamp_ms < 0 {
			return Err(Error::InvalidArgument);
		}

		if package
			.as_deref()
			.is_some_and(|package| package.len() > MAX_PACKAGE_BYTES)
		{
			return Err(Error::TooLarge);
		}

		Ok(Self {
			kind,
			timestamp_ms,
			package,
		})
	}
}

/// One app's continuous stretch in the foreground, clipped to the queried
/// range's edges.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageSession {
	/// The app that was in the foreground.
	pub package: String,
	/// The label the system's launcher shows for that app.
	pub label: String,
	/// When the app came to the foreground, in Unix milliseconds, clipped
	/// to the range's start when the session straddles it.
	pub start_ms: i64,
	/// When the app left the foreground, in Unix milliseconds, clipped to
	/// the range's end when it was still open there.
	pub end_ms: i64,
}

impl UsageSession {
	/// Encodes one session, for handlers and tests.
	pub fn encode(&self) -> Result<Vec<u8>, Error> {
		if self.start_ms < 0 || self.end_ms < self.start_ms || self.package.is_empty() {
			return Err(Error::InvalidArgument);
		}

		if self.package.len() > MAX_PACKAGE_BYTES || self.label.len() > MAX_LABEL_BYTES {
			return Err(Error::TooLarge);
		}

		Ok(Writer::default()
			.str(&self.package)?
			.str(&self.label)?
			.i64(self.start_ms)
			.i64(self.end_ms)
			.finish())
	}

	/// Decodes one session.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		let mut reader = Reader::new(payload);
		let session = Self::read(&mut reader)?;
		reader.finish()?;
		Ok(session)
	}

	/// Reads one session from an open batch, without consuming its end.
	pub(crate) fn read(reader: &mut Reader<'_>) -> Result<Self, Error> {
		let package = reader.str()?.to_owned();
		let label = reader.str()?.to_owned();
		let start_ms = reader.i64()?;
		let end_ms = reader.i64()?;

		if package.is_empty() || package.len() > MAX_PACKAGE_BYTES || label.len() > MAX_LABEL_BYTES
		{
			return Err(Error::InvalidArgument);
		}

		if start_ms < 0 || end_ms < start_ms {
			return Err(Error::InvalidArgument);
		}

		Ok(Self {
			package,
			label,
			start_ms,
			end_ms,
		})
	}
}

/// A range's screen time and per-app totals, built from the same events.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UsageSummary {
	/// Total milliseconds the screen was interactive, clipped to the range.
	pub screen_on_ms: i64,
	/// How many times the lock screen was dismissed.
	pub unlocks: u32,
	/// Foreground milliseconds per app, clipped to the range, sorted by
	/// package.
	pub apps: Vec<AppTotal>,
}

/// One app's foreground total inside a queried range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppTotal {
	/// The app that was in the foreground.
	pub package: String,
	/// Its foreground milliseconds inside the range.
	pub total_ms: i64,
}

impl UsageSummary {
	/// Encodes a summary, for handlers and tests.
	pub fn encode(&self) -> Result<Vec<u8>, Error> {
		if self.screen_on_ms < 0 || self.apps.iter().any(|app| app.total_ms < 0) {
			return Err(Error::InvalidArgument);
		}

		let count = u32::try_from(self.apps.len())
			.ok()
			.filter(|count| *count <= MAX_SESSIONS)
			.ok_or(Error::TooLarge)?;
		let mut writer = Writer::default();
		writer.i64(self.screen_on_ms).u32(self.unlocks).u32(count);

		for app in &self.apps {
			if app.package.is_empty() {
				return Err(Error::InvalidArgument);
			}

			if app.package.len() > MAX_PACKAGE_BYTES {
				return Err(Error::TooLarge);
			}

			writer.str(&app.package)?.i64(app.total_ms);
		}

		Ok(writer.finish())
	}

	/// Decodes a summary, rejecting negative totals and duplicate or
	/// unsorted packages.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		let mut reader = Reader::new(payload);
		let screen_on_ms = reader.i64()?;
		let unlocks = reader.u32()?;
		let count = reader.u32()?;

		if count > MAX_SESSIONS {
			return Err(Error::InvalidArgument);
		}

		let mut apps = Vec::new();

		for _ in 0..count {
			let package = reader.str()?.to_owned();
			let total_ms = reader.i64()?;

			if package.is_empty() || package.len() > MAX_PACKAGE_BYTES || total_ms < 0 {
				return Err(Error::InvalidArgument);
			}

			apps.push(AppTotal { package, total_ms });
		}

		reader.finish()?;

		if screen_on_ms < 0
			|| apps
				.windows(2)
				.any(|pair| pair[0].package >= pair[1].package)
		{
			return Err(Error::InvalidArgument);
		}

		Ok(Self {
			screen_on_ms,
			unlocks,
			apps,
		})
	}
}

/// Longest accepted package name, in bytes.
pub const MAX_PACKAGE_BYTES: usize = 256;
/// Longest accepted app label, in bytes.
pub const MAX_LABEL_BYTES: usize = 256;
/// Most events one query may return. A longer range reports its events in
/// pages.
pub const MAX_EVENTS: u32 = 1024;
/// Most sessions one query may return, and most apps one summary may list; a
/// range that yields more sessions asks for a narrower one.
pub const MAX_SESSIONS: u32 = 4096;

#[cfg(test)]
mod tests {
	use super::*;

	/// A range boundary a real query would use.
	const SINCE: i64 = 1_700_000_000_000;
	const UNTIL: i64 = 1_700_000_360_000;

	#[test]
	fn statuses_decode_exactly() {
		for (code, status) in [
			(0u32, UsageAccessStatus::Unknown),
			(1u32, UsageAccessStatus::Granted),
			(2u32, UsageAccessStatus::Denied),
		] {
			assert_eq!(UsageAccessStatus::decode(&code.to_le_bytes()), Ok(status));
		}

		assert_eq!(
			UsageAccessStatus::decode(&3_u32.to_le_bytes()),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			UsageAccessStatus::decode(&[1, 0, 0, 0, 0]),
			Err(Error::InvalidArgument)
		);
		assert_eq!(UsageAccessStatus::Granted.encode(), vec![1, 0, 0, 0]);
	}

	#[test]
	fn ranges_round_trip_and_reject_bad_bounds() {
		let range = UsageRange::new(SINCE, UNTIL).unwrap();
		assert_eq!(UsageRange::decode(&range.encode()), Ok(range));

		for (since, until) in [(-1, UNTIL), (UNTIL + 1, SINCE), (5, 4)] {
			assert_eq!(UsageRange::new(since, until), Err(Error::InvalidArgument));
		}

		let empty = UsageRange::new(7, 7).unwrap();
		assert_eq!(UsageRange::decode(&empty.encode()), Ok(empty));
		assert_eq!(
			UsageRange::decode(&range.encode()[..15]),
			Err(Error::InvalidArgument)
		);
	}

	#[test]
	fn events_pair_kinds_with_packages_and_round_trip() {
		let activity = UsageEvent {
			kind: EventKind::ActivityResumed,
			timestamp_ms: SINCE,
			package: Some("com.example.chat".to_owned()),
		};
		assert_eq!(
			UsageEvent::decode(&activity.encode().unwrap()),
			Ok(activity)
		);

		let device = UsageEvent {
			kind: EventKind::Unlocked,
			timestamp_ms: UNTIL,
			package: None,
		};
		assert_eq!(UsageEvent::decode(&device.encode().unwrap()), Ok(device));

		for kind in [
			EventKind::ActivityResumed,
			EventKind::ActivityPaused,
			EventKind::ScreenInteractive,
			EventKind::ScreenNonInteractive,
			EventKind::KeyguardShown,
			EventKind::KeyguardHidden,
			EventKind::DeviceStartup,
			EventKind::DeviceShutdown,
			EventKind::Locked,
			EventKind::Unlocked,
		] {
			assert_eq!(
				kind.carries_package(),
				kind == EventKind::ActivityResumed || kind == EventKind::ActivityPaused
			);
		}

		assert_eq!(EventKind::from_code(10), Err(Error::InvalidArgument));
	}

	#[test]
	fn events_reject_mismatched_packages_and_bad_values() {
		let activity = UsageEvent {
			kind: EventKind::ActivityResumed,
			timestamp_ms: SINCE,
			package: None,
		};
		assert_eq!(activity.encode(), Err(Error::InvalidArgument));

		let device = UsageEvent {
			kind: EventKind::KeyguardHidden,
			timestamp_ms: SINCE,
			package: Some("com.example.chat".to_owned()),
		};
		assert_eq!(device.encode(), Err(Error::InvalidArgument));

		let negative = UsageEvent {
			kind: EventKind::Locked,
			timestamp_ms: -1,
			package: None,
		};
		assert_eq!(negative.encode(), Err(Error::InvalidArgument));

		let oversized = UsageEvent {
			kind: EventKind::ActivityResumed,
			timestamp_ms: SINCE,
			package: Some("x".repeat(MAX_PACKAGE_BYTES + 1)),
		};
		assert_eq!(oversized.encode(), Err(Error::TooLarge));

		let payload = UsageEvent {
			kind: EventKind::Locked,
			timestamp_ms: SINCE,
			package: None,
		}
		.encode()
		.unwrap();
		let mut trailing = payload.clone();
		trailing.push(0);
		assert_eq!(UsageEvent::decode(&trailing), Err(Error::InvalidArgument));
	}

	fn session() -> UsageSession {
		UsageSession {
			package: "com.example.chat".to_owned(),
			label: "Chat".to_owned(),
			start_ms: SINCE,
			end_ms: UNTIL,
		}
	}

	#[test]
	fn sessions_round_trip_and_reject_bad_values() {
		let valid = session();
		assert_eq!(UsageSession::decode(&valid.encode().unwrap()), Ok(valid));

		let oversized_package = UsageSession {
			package: "x".repeat(MAX_PACKAGE_BYTES + 1),
			..session()
		};
		assert_eq!(oversized_package.encode(), Err(Error::TooLarge));

		let bad = [
			UsageSession {
				start_ms: -1,
				..session()
			},
			UsageSession {
				end_ms: SINCE - 1,
				..session()
			},
			UsageSession {
				package: String::new(),
				..session()
			},
		];

		for session in bad {
			assert_eq!(session.encode(), Err(Error::InvalidArgument));
		}

		let oversized_label = UsageSession {
			label: "x".repeat(MAX_LABEL_BYTES + 1),
			..session()
		};
		assert_eq!(oversized_label.encode(), Err(Error::TooLarge));
	}

	#[test]
	fn summaries_round_trip_and_require_sorted_unique_apps() {
		let summary = UsageSummary {
			screen_on_ms: 1_234_567,
			unlocks: 3,
			apps: vec![
				AppTotal {
					package: "com.example.chat".to_owned(),
					total_ms: 1_000,
				},
				AppTotal {
					package: "com.example.game".to_owned(),
					total_ms: 234,
				},
			],
		};
		assert_eq!(
			UsageSummary::decode(&summary.encode().unwrap()),
			Ok(summary.clone())
		);

		let empty = UsageSummary::default();
		assert_eq!(UsageSummary::decode(&empty.encode().unwrap()), Ok(empty));

		let duplicate = UsageSummary {
			apps: vec![
				AppTotal {
					package: "a".to_owned(),
					total_ms: 1,
				},
				AppTotal {
					package: "a".to_owned(),
					total_ms: 1,
				},
			],
			..UsageSummary::default()
		};
		assert_eq!(
			UsageSummary::decode(&duplicate.encode().unwrap()),
			Err(Error::InvalidArgument)
		);

		let reversed = UsageSummary {
			apps: vec![
				AppTotal {
					package: "b".to_owned(),
					total_ms: 1,
				},
				AppTotal {
					package: "a".to_owned(),
					total_ms: 1,
				},
			],
			..UsageSummary::default()
		};
		assert_eq!(
			UsageSummary::decode(&reversed.encode().unwrap()),
			Err(Error::InvalidArgument)
		);

		let negative = UsageSummary {
			apps: vec![AppTotal {
				package: "a".to_owned(),
				total_ms: -1,
			}],
			..UsageSummary::default()
		};
		assert_eq!(negative.encode(), Err(Error::InvalidArgument));

		let mut oversized = 0_i64.to_le_bytes().to_vec();
		oversized.extend_from_slice(&0_u32.to_le_bytes());
		oversized.extend_from_slice(&(MAX_SESSIONS + 1).to_le_bytes());
		assert_eq!(
			UsageSummary::decode(&oversized),
			Err(Error::InvalidArgument)
		);

		let payload = summary.encode().unwrap();
		let mut truncated = payload.clone();
		truncated.pop();
		assert_eq!(
			UsageSummary::decode(&truncated),
			Err(Error::InvalidArgument)
		);
	}
}
