//! The wall clock the lifecycle watches.
//!
//! The day is read from the UTC wall clock at most every
//! [`watch_interval`], so suspend, NTP steps, and startup catch-up all
//! resolve the same way: the date differs, the image re-executes. In debug
//! builds the process tests drive the same clock from a file, so a reset
//! is deterministic instead of waiting for real midnight.

use std::time::Duration;

use chrono::DateTime;
use chrono::Utc;

/// The most time between two UTC-date checks.
const WATCH_INTERVAL: Duration = Duration::from_secs(30);

/// Now, in Unix milliseconds: the wall clock, or in a debug build the
/// `DEMO_TEST_CLOCK` file's milliseconds when that file parses. The test
/// hook is compiled out of release builds, so a release process never
/// reads it.
#[must_use]
pub fn now_millis() -> i64 {
	#[cfg(debug_assertions)]
	if let Some(millis) = test_clock() {
		return millis;
	}

	std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map_or(0, |since| {
			i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
		})
}

#[cfg(debug_assertions)]
fn test_clock() -> Option<i64> {
	let path = std::env::var_os("DEMO_TEST_CLOCK")?;
	let text = std::fs::read_to_string(path).ok()?;
	text.trim().parse().ok()
}

/// The UTC calendar day of a Unix-millisecond instant: `YYYY-MM-DD`.
#[must_use]
pub fn day_of(millis: i64) -> String {
	DateTime::from_timestamp_millis(millis)
		.unwrap_or(DateTime::UNIX_EPOCH)
		.with_timezone(&Utc)
		.format("%Y-%m-%d")
		.to_string()
}

/// How often the day watch compares the UTC date with the generation's.
/// Debug builds may shorten it with `DEMO_TEST_CLOCK_TICK_MS`, clamped to
/// at most [`WATCH_INTERVAL`] and at least 10 ms so a bad value can't
/// spin.
#[must_use]
pub fn watch_interval() -> Duration {
	#[cfg(debug_assertions)]
	if let Some(millis) = std::env::var("DEMO_TEST_CLOCK_TICK_MS")
		.ok()
		.and_then(|millis| millis.trim().parse::<u64>().ok())
	{
		return Duration::from_millis(millis.clamp(10, WATCH_INTERVAL.as_millis() as u64));
	}

	WATCH_INTERVAL
}

/// How long a process whose initialization failed keeps the fallback
/// before it re-executes for a fresh try. Debug builds may shorten it with
/// `DEMO_TEST_RETRY_MS`.
#[must_use]
pub fn retry_delay() -> Duration {
	const DELAY: Duration = Duration::from_secs(60);

	#[cfg(debug_assertions)]
	if let Some(millis) = std::env::var("DEMO_TEST_RETRY_MS")
		.ok()
		.and_then(|millis| millis.trim().parse::<u64>().ok())
	{
		return Duration::from_millis(millis.clamp(100, DELAY.as_millis() as u64));
	}

	DELAY
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn days_format_as_utc_dates() {
		// 2026-11-15T00:00:00Z and a moment either side of it.
		assert_eq!(day_of(1_794_700_800_000), "2026-11-15");
		assert_eq!(day_of(1_794_700_799_999), "2026-11-14");
		assert_eq!(day_of(1_794_700_800_001), "2026-11-15");
		// The epoch itself, where some clocks start.
		assert_eq!(day_of(0), "1970-01-01");
	}
}
