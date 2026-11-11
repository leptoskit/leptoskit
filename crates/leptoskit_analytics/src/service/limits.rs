//! Rate limits: a token bucket per `distinct_id`, in memory, so one hot
//! sender (a load test, a bot, a client stuck in a loop) can't fill the
//! writer's queue for everyone else. The app's own events, sent as the
//! server, aren't limited.
//!
//! The defaults are far above what a person's device sends, so a real
//! client never meets them. Events over the limit are dropped, counted in
//! the answer, and recorded as an ingestion warning.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::time::Instant;

use super::accept::Row;
use super::accept::Source;

/// A bucket's size and how fast it refills.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rate {
	/// Events a second, sustained.
	pub per_second: f64,
	/// Events at once, from a full bucket.
	pub burst: f64,
}

impl Rate {
	/// Each `distinct_id`'s default: 100 events a second, 2,000 at once.
	pub const PER_DISTINCT_ID: Self = Self {
		per_second: 100.0,
		burst: 2_000.0,
	};
}

struct Bucket {
	tokens: f64,
	at: Instant,
}

/// Buckets past this many are swept of those that have refilled, so ids
/// seen once don't hold memory forever.
const SWEEP_AT: usize = 100_000;

/// Token buckets by `distinct_id`.
pub struct Limiter {
	rate: Rate,
	buckets: Mutex<HashMap<String, Bucket>>,
}

impl Limiter {
	pub fn new(rate: Rate) -> Self {
		Self {
			rate,
			buckets: Mutex::new(HashMap::new()),
		}
	}

	/// Removes from `rows` those over their `distinct_id`'s limit at `now`,
	/// and returns how many it removed.
	pub fn admit(&self, rows: &mut Vec<Row>, now: Instant) -> u32 {
		let mut buckets = self.buckets.lock().unwrap_or_else(PoisonError::into_inner);
		if buckets.len() > SWEEP_AT {
			let full = self.rate.burst / self.rate.per_second.max(f64::MIN_POSITIVE);
			buckets.retain(|_, bucket| now.duration_since(bucket.at).as_secs_f64() < full);
		}
		let before = rows.len();
		rows.retain(|row| {
			if row.source == Source::Server {
				return true;
			}
			let bucket = buckets.entry(row.distinct_id.clone()).or_insert(Bucket {
				tokens: self.rate.burst,
				at: now,
			});
			let refill = now.duration_since(bucket.at).as_secs_f64() * self.rate.per_second;
			bucket.tokens = (bucket.tokens + refill).min(self.rate.burst);
			bucket.at = now;
			if bucket.tokens >= 1.0 {
				bucket.tokens -= 1.0;
				true
			} else {
				false
			}
		});
		(before - rows.len()) as u32
	}
}

#[cfg(test)]
mod tests {
	use std::time::Duration;

	use uuid::Uuid;

	use super::*;

	fn row(distinct_id: &str, source: Source) -> Row {
		Row {
			uuid: Uuid::nil(),
			name: "event".into(),
			distinct_id: distinct_id.into(),
			timestamp: leptoskit_db::now(),
			session_id: None,
			context_id: 0,
			properties: "{}".into(),
			source,
		}
	}

	fn rows(distinct_id: &str, count: usize) -> Vec<Row> {
		(0..count)
			.map(|_| row(distinct_id, Source::Client))
			.collect()
	}

	#[test]
	fn a_hot_id_is_limited_and_others_are_not() {
		let limiter = Limiter::new(Rate {
			per_second: 10.0,
			burst: 5.0,
		});
		let now = Instant::now();
		let mut hot = rows("bot", 8);
		assert_eq!(limiter.admit(&mut hot, now), 3);
		assert_eq!(hot.len(), 5);

		let mut person = rows("person", 2);
		assert_eq!(limiter.admit(&mut person, now), 0);

		let mut later = rows("bot", 3);
		assert_eq!(
			limiter.admit(&mut later, now + Duration::from_millis(200)),
			1,
			"0.2 s refills two"
		);
	}

	#[test]
	fn the_app_s_own_events_are_not_limited() {
		let limiter = Limiter::new(Rate {
			per_second: 1.0,
			burst: 1.0,
		});
		let mut server: Vec<Row> = (0..50).map(|_| row("system", Source::Server)).collect();
		assert_eq!(limiter.admit(&mut server, Instant::now()), 0);
	}
}
