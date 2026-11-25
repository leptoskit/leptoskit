//! Counting a tenant's pushes: the rows of its acceptance ledger, by when
//! its providers accepted them. The ledger names no person, so an app that
//! forgets one, or prunes its deliveries, still has every push counted.

use std::time::Duration;

use leptoskit_db::Db;
use leptoskit_db::Timestamp;
use leptoskit_notifications::push::PushAcceptance;

/// How far behind now a count stops: a delivery accepted just before a
/// count may still be being written, so it is counted in the next one.
pub const LAG: Duration = Duration::from_mins(1);

/// Where a count ends and the next starts: an instant, written
/// `accepted:<milliseconds since the Unix epoch>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Cursor(i64);

impl Cursor {
	const PREFIX: &'static str = "accepted:";

	/// The cursor at `at`.
	#[must_use]
	pub fn at(at: Timestamp) -> Self {
		Self(at.timestamp_millis())
	}

	/// `text` read back, or `None` when it isn't one of these.
	#[must_use]
	pub fn parse(text: &str) -> Option<Self> {
		let millis = text.strip_prefix(Self::PREFIX)?;

		if millis.is_empty() || !millis.bytes().all(|byte| byte.is_ascii_digit()) {
			return None;
		}

		millis.parse().ok().map(Self)
	}

	fn instant(self) -> Option<Timestamp> {
		chrono::DateTime::from_timestamp_millis(self.0)
	}
}

impl std::fmt::Display for Cursor {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(formatter, "{}{}", Self::PREFIX, self.0)
	}
}

/// The pushes `db`'s providers accepted after `after` (from the beginning
/// when `None`) and at or before `until`.
///
/// # Errors
///
/// When the database can't be read, or `until` isn't after `after`.
pub async fn count(db: &Db, after: Option<Cursor>, until: Cursor) -> Result<u64, String> {
	if after.is_some_and(|after| after >= until) {
		return Err("the count would end before it starts".to_owned());
	}

	let until = until
		.instant()
		.ok_or_else(|| "the count ends at no instant".to_owned())?;
	let after = match after {
		Some(after) => {
			Some(
				after
					.instant()
					.ok_or_else(|| "the count starts at no instant".to_owned())?,
			)
		}
		None => None,
	};
	let db = db.system("the hub counts a tenant's accepted pushes");

	PushAcceptance::query()
		.filter(move |row| {
			let before = row.accepted_at.lte(until);

			match after {
				Some(after) => before & row.accepted_at.gt(after),
				None => before,
			}
		})
		.count(&db)
		.await
		.map_err(|error| format!("its pushes couldn't be counted: {error}"))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_cursor_reads_back_as_written() {
		let cursor = Cursor(1_764_097_200_000);
		assert_eq!(cursor.to_string(), "accepted:1764097200000");
		assert_eq!(Cursor::parse(&cursor.to_string()), Some(cursor));

		for wrong in [
			"",
			"accepted:",
			"accepted:-1",
			"accepted:12a",
			"sent:12",
			"accepted:99999999999999999999999",
		] {
			assert_eq!(Cursor::parse(wrong), None, "{wrong}");
		}
	}
}
