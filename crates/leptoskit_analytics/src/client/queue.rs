//! The queue every client sends from: entries wait here until a batch
//! holding them is stored, are retried with backoff when it isn't, and are
//! dropped, oldest first and counted, when there are too many.
//!
//! The queue does no I/O and reads no clock: callers pass the time in, take
//! a [`Pending`] batch, send it however their platform sends, and
//! [`settle`](Queue::settle) it with the answer. That keeps it the same on
//! the web, in a native app, and on a server, and testable without any of
//! them.

use std::collections::VecDeque;

use crate::wire;
use crate::wire::Batch;
use crate::wire::Body;
use crate::wire::Entry;
use crate::wire::Properties;
use crate::wire::Value;

/// How much a queue holds and sends at once.
#[derive(Clone, Debug)]
pub struct Limits {
	/// The most entries in one batch.
	pub batch_entries: usize,
	/// The most a batch's entries may take, roughly, before encoding.
	pub batch_bytes: usize,
	/// The most entries waiting; past it, the oldest are dropped.
	pub queue_entries: usize,
	/// The most the waiting entries may take, roughly.
	pub queue_bytes: usize,
	/// Entries older than this, in milliseconds, are dropped rather than
	/// sent: the service only deduplicates retries within its window.
	pub max_age: i64,
}

impl Default for Limits {
	fn default() -> Self {
		Self {
			batch_entries: 100,
			batch_bytes: 1 << 20,
			queue_entries: 10_000,
			queue_bytes: 5 << 20,
			max_age: 7 * 24 * 60 * 60 * 1_000,
		}
	}
}

/// The first retry waits this long, in milliseconds, and each after it
/// twice as long as the one before, up to [`MAX_BACKOFF`].
pub const FIRST_BACKOFF: i64 = 1_000;

/// The longest a retry waits, in milliseconds.
pub const MAX_BACKOFF: i64 = 5 * 60 * 1_000;

/// What the service did with a batch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
	/// Stored. Entries the service refused are in its answer; they'll never
	/// be accepted, so they're done too.
	Stored,
	/// The batch will never be accepted (a `4xx` other than `408`, `413`
	/// and `429`): it's dropped.
	Rejected,
	/// The batch was too large: it's sent again in smaller batches.
	TooLarge,
	/// Try again later: no answer, a `5xx`, `408` or `429`. `after` is the
	/// `Retry-After` the service asked for, in milliseconds.
	Retry { after: Option<i64> },
}

impl Outcome {
	/// The outcome an HTTP status means, with the response's `Retry-After`
	/// header, in seconds, if it had one.
	pub fn from_status(status: u16, retry_after: Option<&str>) -> Self {
		match status {
			200..=299 => Self::Stored,
			413 => Self::TooLarge,
			408 | 429 | 500..=599 => {
				Self::Retry {
					after: retry_after
						.and_then(|seconds| seconds.trim().parse::<i64>().ok())
						.map(|seconds| seconds.saturating_mul(1_000)),
				}
			}
			_ => Self::Rejected,
		}
	}
}

/// A batch on its way, encoded.
#[derive(Debug)]
pub struct Pending {
	/// The batch, in [`wire`]'s format: the request's body.
	pub body: Vec<u8>,
	/// How many of the queue's entries it holds.
	pub entries: usize,
}

struct Queued {
	entry: Entry,
	/// Roughly what the entry takes, for the byte limits.
	bytes: usize,
	/// When it was queued, for [`Limits::max_age`].
	at: i64,
}

/// Entries waiting to be sent.
pub struct Queue {
	limits: Limits,
	/// Each context the waiting entries name, by the generation an entry's
	/// `context` field holds.
	contexts: Vec<(u32, Properties)>,
	generation: u32,
	waiting: VecDeque<Queued>,
	/// The entries of the batch on its way, oldest first.
	sending: Vec<Queued>,
	bytes: usize,
	dropped: u64,
	/// Shrinks when the service says a batch is too large.
	batch_entries: usize,
	failures: u32,
	retry_at: i64,
}

impl Queue {
	/// An empty queue, whose entries are sent with `context`: what's the
	/// same for all of them, such as the library and the device.
	pub fn new(limits: Limits, context: Properties) -> Self {
		let batch_entries = limits.batch_entries.max(1);
		Self {
			limits,
			contexts: vec![(0, context)],
			generation: 0,
			waiting: VecDeque::new(),
			sending: Vec::new(),
			bytes: 0,
			dropped: 0,
			batch_entries,
			failures: 0,
			retry_at: i64::MIN,
		}
	}

	/// Entries queued from now on are sent with `context`; those already
	/// queued keep theirs.
	pub fn set_context(&mut self, context: Properties) {
		self.generation = self.generation.wrapping_add(1);
		self.contexts.push((self.generation, context));
		self.forget_unused_contexts();
	}

	/// Queues `entry`, recorded at `now`. Its `context` is replaced with the
	/// current context.
	pub fn push(&mut self, mut entry: Entry, now: i64) {
		entry.context = self.generation;
		let bytes = size(&entry);
		self.waiting.push_back(Queued {
			entry,
			bytes,
			at: now,
		});
		self.bytes += bytes;
		while self.waiting.len() > self.limits.queue_entries
			|| (self.bytes > self.limits.queue_bytes && self.waiting.len() > 1)
		{
			self.drop_oldest();
		}
	}

	/// How many entries are waiting, not counting a batch on its way.
	pub fn len(&self) -> usize {
		self.waiting.len()
	}

	/// Whether nothing is waiting.
	pub fn is_empty(&self) -> bool {
		self.waiting.is_empty()
	}

	/// Whether a batch is on its way.
	pub fn is_sending(&self) -> bool {
		!self.sending.is_empty()
	}

	/// How many entries have been dropped and not yet reported.
	pub fn dropped(&self) -> u64 {
		self.dropped
	}

	/// When the next batch may be sent, after a failure: milliseconds since
	/// the epoch, or `None` when it may be sent now.
	pub fn retry_at(&self) -> Option<i64> {
		(self.failures > 0).then_some(self.retry_at)
	}

	/// Drops every waiting entry, as opting out does. A batch on its way
	/// still settles.
	pub fn clear(&mut self) {
		self.waiting.clear();
		self.bytes = 0;
		self.dropped = 0;
		self.forget_unused_contexts();
	}

	/// The next batch to send, if there's one to send at `now`: none while
	/// a batch is on its way, while waiting out a retry, or when nothing
	/// waits. `random` fills the id of the `$lk_dropped` event that reports
	/// dropped entries, when there are some.
	pub fn take(&mut self, now: i64, random: [u8; 10]) -> Option<Pending> {
		if self.is_sending() || now < self.retry_at {
			return None;
		}
		self.drop_expired(now);
		if self.waiting.is_empty() {
			return None;
		}

		let mut bytes = 0;
		while let Some(next) = self.waiting.front() {
			let full = self.sending.len() >= self.batch_entries
				|| (bytes + next.bytes > self.limits.batch_bytes && !self.sending.is_empty());
			if full {
				break;
			}
			bytes += next.bytes;
			self.bytes -= next.bytes;
			self.sending.extend(self.waiting.pop_front());
		}

		let mut batch = Batch {
			sent_at: now,
			contexts: Vec::new(),
			entries: Vec::with_capacity(self.sending.len() + 1),
		};
		let mut used: Vec<u32> = Vec::new();
		for queued in &self.sending {
			let mut entry = queued.entry.clone();
			entry.context = index_of(&mut used, entry.context);
			batch.entries.push(entry);
		}
		if self.dropped > 0 {
			let first = &self.sending[0].entry;
			batch.entries.push(Entry {
				uuid: wire::uuid_v7(now, random),
				distinct_id: first.distinct_id.clone(),
				session_id: None,
				context: 0,
				body: Body::Event {
					name: "$lk_dropped".into(),
					properties: vec![(
						"count".into(),
						Value::Int(i64::try_from(self.dropped).unwrap_or(i64::MAX)),
					)],
				},
			});
		}
		batch.contexts = used
			.iter()
			.map(|generation| {
				self.contexts
					.iter()
					.find(|(have, _)| have == generation)
					.map(|(_, context)| context.clone())
					.unwrap_or_default()
			})
			.collect();

		Some(Pending {
			body: wire::encode(&batch),
			entries: self.sending.len(),
		})
	}

	/// Settles the batch on its way with what the service did with it, at
	/// `now`. `jitter`, from 0 to 1, spreads retries so clients that failed
	/// together don't all retry together.
	pub fn settle(&mut self, outcome: Outcome, now: i64, jitter: f64) {
		match outcome {
			Outcome::Stored | Outcome::Rejected => {
				if outcome == Outcome::Rejected {
					self.dropped += self.sending.len() as u64;
				} else {
					self.dropped = 0;
				}
				self.sending.clear();
				self.failures = 0;
				self.retry_at = i64::MIN;
				self.batch_entries = self.limits.batch_entries.max(1);
				self.forget_unused_contexts();
			}
			Outcome::TooLarge => {
				if self.sending.len() <= 1 {
					// Even alone it's too large: it never will be accepted.
					self.dropped += self.sending.len() as u64;
					self.sending.clear();
					self.forget_unused_contexts();
				} else {
					self.batch_entries = (self.sending.len() / 2).max(1);
					self.put_back();
				}
			}
			Outcome::Retry { after } => {
				self.failures = self.failures.saturating_add(1);
				let doubling = FIRST_BACKOFF.saturating_mul(1 << (self.failures - 1).min(20));
				let wait = doubling.min(MAX_BACKOFF);
				let jittered = wait / 2 + (wait as f64 / 2.0 * jitter.clamp(0.0, 1.0)) as i64;
				self.retry_at = now.saturating_add(jittered.max(after.unwrap_or(0)));
				self.put_back();
			}
		}
	}

	fn put_back(&mut self) {
		for queued in self.sending.drain(..).rev() {
			self.bytes += queued.bytes;
			self.waiting.push_front(queued);
		}
		while self.waiting.len() > self.limits.queue_entries {
			self.drop_oldest();
		}
	}

	fn drop_oldest(&mut self) {
		if let Some(oldest) = self.waiting.pop_front() {
			self.bytes -= oldest.bytes;
			self.dropped += 1;
		}
	}

	fn drop_expired(&mut self, now: i64) {
		while self
			.waiting
			.front()
			.is_some_and(|oldest| now.saturating_sub(oldest.at) > self.limits.max_age)
		{
			self.drop_oldest();
		}
	}

	/// Keeps only the current context and those a queued entry names.
	fn forget_unused_contexts(&mut self) {
		let generation = self.generation;
		let (waiting, sending) = (&self.waiting, &self.sending);
		self.contexts.retain(|(have, _)| {
			*have == generation
				|| waiting.iter().any(|queued| queued.entry.context == *have)
				|| sending.iter().any(|queued| queued.entry.context == *have)
		});
	}
}

/// `generation`'s index in `used`, adding it when it's new.
fn index_of(used: &mut Vec<u32>, generation: u32) -> u32 {
	let index = used
		.iter()
		.position(|have| *have == generation)
		.unwrap_or_else(|| {
			used.push(generation);
			used.len() - 1
		});
	index as u32
}

/// Roughly what `entry` takes encoded: its strings, and a little for each
/// value and the entry's fixed fields.
fn size(entry: &Entry) -> usize {
	let body = match &entry.body {
		Body::Event { name, properties } => name.len() + properties_size(properties),
		Body::Identify {
			anon_distinct_id,
			set,
			set_once,
		} => anon_distinct_id.len() + properties_size(set) + properties_size(set_once),
		Body::Alias { alias } => alias.len(),
		Body::Set {
			set,
			set_once,
			unset,
		} => {
			properties_size(set)
				+ properties_size(set_once)
				+ unset.iter().map(String::len).sum::<usize>()
		}
		Body::GroupIdentify {
			group_type,
			group_key,
			set,
		} => group_type.len() + group_key.len() + properties_size(set),
	};
	36 + entry.distinct_id.len() + body
}

fn properties_size(properties: &Properties) -> usize {
	properties
		.iter()
		.map(|(name, value)| name.len() + 2 + value_size(value))
		.sum()
}

fn value_size(value: &Value) -> usize {
	match value {
		Value::Text(text) => text.len() + 2,
		Value::List(items) => 2 + items.iter().map(value_size).sum::<usize>(),
		Value::Map(entries) => 2 + properties_size(entries),
		_ => 9,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn event(n: u8) -> Entry {
		Entry {
			uuid: wire::uuid_v7(1_000, [n; 10]),
			distinct_id: "someone".into(),
			session_id: None,
			context: 0,
			body: Body::Event {
				name: format!("event {n}"),
				properties: Vec::new(),
			},
		}
	}

	fn names(pending: &Pending) -> Vec<String> {
		wire::decode(&pending.body, &wire::Limits::default())
			.unwrap()
			.entries
			.into_iter()
			.map(|entry| {
				match entry.body {
					Body::Event { name, .. } => name,
					_ => String::new(),
				}
			})
			.collect()
	}

	fn queue(limits: Limits) -> Queue {
		Queue::new(limits, vec![("$lib".into(), Value::Text("test".into()))])
	}

	#[test]
	fn batches_go_one_at_a_time_and_stored_ones_are_done() {
		let mut queue = queue(Limits {
			batch_entries: 2,
			..Limits::default()
		});
		for n in 0..3 {
			queue.push(event(n), 0);
		}
		let first = queue.take(0, [0; 10]).unwrap();
		assert_eq!(names(&first), ["event 0", "event 1"]);
		assert!(queue.take(0, [0; 10]).is_none(), "one batch at a time");
		queue.settle(Outcome::Stored, 0, 0.0);
		let second = queue.take(0, [0; 10]).unwrap();
		assert_eq!(names(&second), ["event 2"]);
		queue.settle(Outcome::Stored, 0, 0.0);
		assert!(queue.take(0, [0; 10]).is_none());
	}

	#[test]
	fn failures_back_off_doubling_and_keep_the_entries_in_order() {
		let mut queue = queue(Limits::default());
		queue.push(event(0), 0);
		queue.take(0, [0; 10]).unwrap();
		queue.settle(Outcome::Retry { after: None }, 0, 1.0);
		assert_eq!(queue.retry_at(), Some(1_000));
		queue.push(event(1), 10);
		assert!(queue.take(999, [0; 10]).is_none(), "still waiting");
		queue.take(1_000, [0; 10]).unwrap();
		queue.settle(Outcome::Retry { after: None }, 1_000, 1.0);
		assert_eq!(queue.retry_at(), Some(3_000));
		queue.take(3_000, [0; 10]).unwrap();
		queue.settle(
			Outcome::Retry {
				after: Some(60_000),
			},
			3_000,
			1.0,
		);
		assert_eq!(queue.retry_at(), Some(63_000), "Retry-After wins");

		let pending = queue.take(63_000, [0; 10]).unwrap();
		assert_eq!(names(&pending), ["event 0", "event 1"]);
		queue.settle(Outcome::Stored, 63_000, 0.0);
		assert_eq!(queue.retry_at(), None);
	}

	#[test]
	fn backoff_stops_growing_at_five_minutes() {
		let mut queue = queue(Limits::default());
		queue.push(event(0), 0);
		let mut now = 0;
		let mut waits = Vec::new();
		for _ in 0..12 {
			queue.take(now, [0; 10]).unwrap();
			queue.settle(Outcome::Retry { after: None }, now, 1.0);
			let next = queue.retry_at().unwrap();
			waits.push(next - now);
			now = next;
		}
		assert_eq!(
			waits,
			[
				1_000, 2_000, 4_000, 8_000, 16_000, 32_000, 64_000, 128_000, 256_000, 300_000,
				300_000, 300_000
			]
		);
	}

	#[test]
	fn too_large_halves_the_batch_and_a_lone_entry_is_dropped() {
		let mut queue = queue(Limits::default());
		for n in 0..4 {
			queue.push(event(n), 0);
		}
		queue.take(0, [0; 10]).unwrap();
		queue.settle(Outcome::TooLarge, 0, 0.0);
		let half = queue.take(0, [0; 10]).unwrap();
		assert_eq!(half.entries, 2);
		queue.settle(Outcome::TooLarge, 0, 0.0);
		assert_eq!(queue.take(0, [0; 10]).unwrap().entries, 1);
		queue.settle(Outcome::TooLarge, 0, 0.0);
		assert_eq!(queue.dropped(), 1);
		assert_eq!(queue.len(), 3);
	}

	#[test]
	fn the_oldest_are_dropped_and_reported_in_the_next_batch() {
		let mut queue = queue(Limits {
			queue_entries: 2,
			..Limits::default()
		});
		for n in 0..5 {
			queue.push(event(n), 0);
		}
		assert_eq!(queue.dropped(), 3);
		let pending = queue.take(0, [7; 10]).unwrap();
		assert_eq!(names(&pending), ["event 3", "event 4", "$lk_dropped"]);
		let batch = wire::decode(&pending.body, &wire::Limits::default()).unwrap();
		assert_eq!(
			batch.entries[2].body,
			Body::Event {
				name: "$lk_dropped".into(),
				properties: vec![("count".into(), Value::Int(3))],
			}
		);
		queue.settle(Outcome::Stored, 0, 0.0);
		assert_eq!(queue.dropped(), 0);
	}

	#[test]
	fn entries_past_the_maximum_age_are_dropped() {
		let mut queue = queue(Limits {
			max_age: 100,
			..Limits::default()
		});
		queue.push(event(0), 0);
		queue.push(event(1), 50);
		let pending = queue.take(120, [0; 10]).unwrap();
		assert_eq!(names(&pending), ["event 1", "$lk_dropped"]);
	}

	#[test]
	fn each_entry_keeps_the_context_it_was_queued_with() {
		let mut queue = queue(Limits::default());
		queue.push(event(0), 0);
		queue.set_context(vec![("$lib".into(), Value::Text("second".into()))]);
		queue.push(event(1), 0);
		let batch = wire::decode(
			&queue.take(0, [0; 10]).unwrap().body,
			&wire::Limits::default(),
		)
		.unwrap();
		assert_eq!(batch.contexts.len(), 2);
		assert_eq!(batch.entries[0].context, 0);
		assert_eq!(batch.entries[1].context, 1);
		assert_eq!(
			batch.contexts[1],
			vec![("$lib".to_owned(), Value::Text("second".into()))]
		);
		queue.settle(Outcome::Stored, 0, 0.0);
		assert_eq!(queue.contexts.len(), 1, "the first context is forgotten");
	}

	#[test]
	fn statuses_mean_what_http_says() {
		assert_eq!(Outcome::from_status(200, None), Outcome::Stored);
		assert_eq!(Outcome::from_status(400, None), Outcome::Rejected);
		assert_eq!(Outcome::from_status(401, None), Outcome::Rejected);
		assert_eq!(Outcome::from_status(413, None), Outcome::TooLarge);
		assert_eq!(
			Outcome::from_status(503, Some("2")),
			Outcome::Retry { after: Some(2_000) }
		);
		assert_eq!(
			Outcome::from_status(429, Some("soon")),
			Outcome::Retry { after: None }
		);
	}
}
