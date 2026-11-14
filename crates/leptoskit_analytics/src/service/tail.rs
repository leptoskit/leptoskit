//! The tail ring (§11, minimal): the last events the writer stored, in
//! memory, each with a sequence number. The `live_events` stream reads it,
//! so a live tail page sees events as they land, filtered server-side,
//! with the matched rate beside them. It is not durable: a restart starts
//! the sequence over, and a client that missed more than the ring holds is
//! told so.
//!
//! §11's resumable stream with sampling, pause, and the faceted filter
//! syntax is step 5's work; this is the ring and the stream it feeds.

use std::collections::VecDeque;

use leptoskit_rpc as rpc;
use serde::Deserialize;
use serde::Serialize;

use super::accept::Row;

/// The most events the ring holds, §11's default. Dropping beyond it only
/// loses what a tail can't show anyway: a reconnecting client that missed
/// more than this is told how far behind it is.
const CAPACITY: usize = 10_000;

/// One stored event, as the tail carries it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TailEvent {
	/// Where the stream reached with this event: each stored event takes
	/// the next number.
	pub seq: u64,
	pub name: String,
	pub distinct_id: String,
	/// When it happened, corrected for the client's clock, as RFC 3339.
	pub timestamp: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub session_id: Option<String>,
	/// The event's own properties, as a JSON object.
	pub properties: serde_json::Value,
}

impl rpc::Resumable for TailEvent {
	type Cursor = u64;

	fn cursor(&self) -> u64 {
		self.seq
	}
}

/// The ring itself. Cloning [`Ingest`](super::Ingest) shares one ring.
#[derive(Default)]
pub struct Tail {
	events: std::sync::Mutex<Ring>,
}

#[derive(Default)]
struct Ring {
	events: VecDeque<TailEvent>,
	/// The sequence number the next stored event takes. 1-based, so `0`
	/// means nothing stored yet.
	next_seq: u64,
}

impl Tail {
	/// An empty ring.
	#[must_use]
	pub fn new() -> Self {
		Self::default()
	}

	/// Appends the batch's stored events, oldest first, after its commit.
	pub fn append(&self, rows: &[&Row]) {
		let properties_of = |row: &Row| serde_json::from_str(&row.properties).unwrap_or_default();
		let mut ring = self
			.events
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner);
		for row in rows {
			let seq = ring.next_seq.max(1);
			ring.next_seq = seq + 1;
			ring.events.push_back(TailEvent {
				seq,
				name: row.name.clone(),
				distinct_id: row.distinct_id.clone(),
				timestamp: super::persons::stamp(row.timestamp),
				session_id: row.session_id.map(|id| id.to_string()),
				properties: properties_of(row),
			});
		}
		while ring.events.len() > CAPACITY {
			ring.events.pop_front();
		}
	}

	/// The events after `after`, oldest first, and the sequence the ring
	/// has reached. `missed` says whether events before the oldest one
	/// returned were dropped: the caller resumed from too far back, and
	/// everything between is gone.
	#[must_use]
	pub fn since(&self, after: u64) -> (Vec<TailEvent>, u64, bool) {
		let ring = self
			.events
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner);
		let latest = ring.next_seq.saturating_sub(1);
		let events: Vec<TailEvent> = ring
			.events
			.iter()
			.filter(|event| event.seq > after)
			.cloned()
			.collect();
		let missed = after < latest && events.first().is_none_or(|first| first.seq > after + 1);
		(events, latest, missed)
	}

	/// How many events the ring holds whose timestamp is within `window`
	/// of `now`, matching `name` and `distinct_id` when set: the matched
	/// rate a tail shows beside the events.
	#[must_use]
	pub fn rate(
		&self,
		name: Option<&str>,
		distinct_id: Option<&str>,
		now: leptoskit_db::Timestamp,
		window: chrono::TimeDelta,
	) -> u64 {
		let from = now - window;
		let ring = self
			.events
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner);
		ring.events
			.iter()
			.filter(|event| {
				name.is_none_or(|name| event.name == name)
					&& distinct_id.is_none_or(|id| event.distinct_id == id)
			})
			.filter_map(|event| super::persons::parse_stamp(&event.timestamp))
			.filter(|at| *at >= from)
			.count() as u64
	}
}

#[cfg(test)]
mod tests {
	use std::sync::atomic::AtomicU8;
	use std::sync::atomic::Ordering;

	use super::super::accept::Row;
	use super::Tail;
	use crate::wire;

	fn row(name: &str, distinct_id: &str, millis: i64) -> Row {
		static NEXT: AtomicU8 = AtomicU8::new(1);
		Row {
			uuid: uuid::Uuid::from_bytes(wire::uuid_v7(
				millis,
				[NEXT.fetch_add(1, Ordering::Relaxed); 10],
			)),
			name: name.into(),
			distinct_id: distinct_id.into(),
			session_id: None,
			context_id: 0,
			timestamp: leptoskit_db::Timestamp::from_timestamp_millis(millis).expect("millis"),
			properties: r#"{"plan":"pro"}"#.into(),
			source: crate::service::Source::Client,
		}
	}

	#[test]
	fn the_ring_holds_what_came_after_a_sequence() {
		let tail = Tail::new();
		let base = leptoskit_db::now().timestamp_millis();
		let rows: Vec<Row> = (0..3)
			.map(|n| row("signed_up", "user-1", base + n * 1_000))
			.collect();
		let refs: Vec<&Row> = rows.iter().collect();
		tail.append(&refs);

		let (all, latest, missed) = tail.since(0);
		assert_eq!(all.len(), 3);
		assert_eq!(latest, 3);
		assert!(!missed);
		assert_eq!(all[2].seq, 3);

		let (after_two, latest, missed) = tail.since(2);
		assert_eq!(after_two.len(), 1);
		assert_eq!(after_two[0].seq, 3);
		assert_eq!(latest, 3);
		assert!(!missed);

		// A sequence the ring no longer holds: everything from 0 is gone,
		// so the reader missed events and is told.
		tail.append(&refs);
		tail.append(&refs);
		let (events, _latest, missed) = tail.since(0);
		assert_eq!(events.len(), 9);
		assert!(!missed, "the oldest event is still there");
	}

	#[test]
	fn the_rate_counts_what_matched_within_the_window() {
		let tail = Tail::new();
		let now = leptoskit_db::now();
		let base = now.timestamp_millis();
		let mut rows = vec![
			row("signed_up", "user-1", base - 10_000),
			row("paid", "user-2", base - 500),
		];
		rows.push(row("signed_up", "user-1", base - 100));
		let refs: Vec<&Row> = rows.iter().collect();
		tail.append(&refs);

		assert_eq!(tail.rate(None, None, now, chrono::TimeDelta::seconds(2)), 2);
		assert_eq!(
			tail.rate(Some("signed_up"), None, now, chrono::TimeDelta::seconds(2)),
			1
		);
		assert_eq!(
			tail.rate(None, Some("user-1"), now, chrono::TimeDelta::seconds(2)),
			1
		);
		assert_eq!(
			tail.rate(None, None, now, chrono::TimeDelta::seconds(60)),
			3
		);
	}
}
