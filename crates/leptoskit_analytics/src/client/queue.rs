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
	/// The most entries held, waiting or on their way; past it, the oldest
	/// waiting are dropped.
	pub queue_entries: usize,
	/// The most the queue may hold, roughly: its entries, waiting or on
	/// their way, and the contexts they name. An entry larger than this,
	/// or than [`batch_bytes`](Self::batch_bytes), is dropped when it's
	/// queued, since it could never be sent.
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
	/// The batch will never be accepted: it's dropped. `400` can retry
	/// compact events with names when the `typed` feature is enabled.
	Rejected,
	/// A bad request: typed events may need to be sent with names to an older
	/// service.
	#[cfg(feature = "typed")]
	BadRequest,
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
			#[cfg(feature = "typed")]
			400 => Self::BadRequest,
			413 => Self::TooLarge,
			408 | 429 | 500..=599 => {
				Self::Retry {
					after: retry_after
						.and_then(|seconds| seconds.trim_ascii().parse::<i64>().ok())
						.map(|seconds| seconds.saturating_mul(1_000)),
				}
			}
			_ => Self::Rejected,
		}
	}
}

/// The first byte of a [`Queue::snapshot`] file.
const SNAPSHOT_VERSION: u8 = 1;

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
	/// The last generation handed out, by `set_context` or a restore.
	last_generation: u32,
	/// Changes whenever what a snapshot would hold changes.
	version: u64,
	waiting: VecDeque<Queued>,
	/// The entries of the batch on its way, oldest first.
	sending: Vec<Queued>,
	/// What the waiting entries take.
	bytes: usize,
	/// What the entries on their way take.
	sending_bytes: usize,
	/// What the kept contexts take.
	context_bytes: usize,
	dropped: u64,
	/// How many of `dropped` the batch on its way reports.
	reporting: u64,
	/// Shrinks when the service says a batch is too large.
	batch_entries: usize,
	failures: u32,
	retry_at: i64,
	/// The field names of each typed event's schema, by schema id, to send
	/// it with its names when the service doesn't know the schema.
	#[cfg(feature = "typed")]
	schemas: Vec<(u64, &'static [(&'static str, &'static str)])>,
}

impl Queue {
	/// An empty queue, whose entries are sent with `context`: what's the
	/// same for all of them, such as the library and the device.
	pub fn new(limits: Limits, context: Properties) -> Self {
		let batch_entries = limits.batch_entries.max(1);
		let context_bytes = properties_size(&context);
		Self {
			limits,
			contexts: vec![(0, context)],
			generation: 0,
			last_generation: 0,
			version: 0,
			waiting: VecDeque::new(),
			sending: Vec::new(),
			bytes: 0,
			sending_bytes: 0,
			context_bytes,
			dropped: 0,
			reporting: 0,
			batch_entries,
			failures: 0,
			retry_at: i64::MIN,
			#[cfg(feature = "typed")]
			schemas: Vec::new(),
		}
	}

	/// Remembers a typed event's fields, for [`settle_with_refusals`].
	///
	/// [`settle_with_refusals`]: Self::settle_with_refusals
	#[cfg(feature = "typed")]
	pub fn remember_schema(
		&mut self,
		schema_id: u64,
		fields: &'static [(&'static str, &'static str)],
	) {
		if !self.schemas.iter().any(|(have, _)| *have == schema_id) {
			self.schemas.push((schema_id, fields));
		}
	}

	/// [`settle`](Self::settle) with [`Outcome::Stored`], given the entries
	/// the service refused, by their index in the batch and the reason. A
	/// typed event refused as `unknown_schema` is queued again with its
	/// property names, keeping its id, so it's stored once all the same.
	#[cfg(feature = "typed")]
	pub fn settle_with_refusals(&mut self, refused: &[(u32, &str)], now: i64, jitter: f64) {
		let mut again = Vec::new();
		let mut dropped = 0_u64;
		for (index, reason) in refused {
			if *reason != "unknown_schema" {
				continue;
			}
			let Some(queued) = self.sending.get(*index as usize) else {
				continue;
			};
			let Some(body) = keyed_body(&queued.entry.body, &self.schemas) else {
				continue;
			};
			let mut entry = queued.entry.clone();
			entry.body = body;
			let bytes = size(&entry);
			if bytes > self.limits.queue_bytes.min(self.limits.batch_bytes) {
				dropped += 1;
				continue;
			}
			again.push(Queued {
				bytes,
				entry,
				at: queued.at,
			});
		}
		// Keep the entries visible while settling prunes unused contexts.
		for queued in again.into_iter().rev() {
			self.bytes += queued.bytes;
			self.waiting.push_front(queued);
		}
		self.settle(Outcome::Stored, now, jitter);
		self.dropped = self.dropped.saturating_add(dropped);
		self.enforce_limits();
	}

	/// Settles named events; permanent per-event refusals need no retry.
	#[cfg(not(feature = "typed"))]
	pub fn settle_with_refusals(&mut self, _refused: &[(u32, &str)], now: i64, jitter: f64) {
		self.settle(Outcome::Stored, now, jitter);
	}

	/// Entries queued from now on are sent with `context`; those already
	/// queued keep theirs.
	pub fn set_context(&mut self, context: Properties) {
		self.last_generation = self.last_generation.wrapping_add(1);
		self.generation = self.last_generation;
		self.context_bytes += properties_size(&context);
		self.contexts.push((self.generation, context));
		self.forget_unused_contexts();
		self.enforce_limits();
	}

	/// Queues `entry`, recorded at `now`. Its `context` is replaced with the
	/// current context.
	pub fn push(&mut self, mut entry: Entry, now: i64) {
		entry.context = self.generation;
		self.push_with_context(entry, now);
	}

	fn push_with_context(&mut self, entry: Entry, at: i64) {
		self.append_with_context(entry, at);
		self.enforce_limits();
	}

	fn append_with_context(&mut self, entry: Entry, at: i64) {
		self.version += 1;
		let bytes = size(&entry);
		if bytes > self.limits.queue_bytes.min(self.limits.batch_bytes) {
			self.dropped += 1;
			return;
		}
		self.waiting.push_back(Queued { entry, bytes, at });
		self.bytes += bytes;
	}

	/// Drops the oldest waiting entries until the queue is within its
	/// limits, counting a batch on its way and the contexts. Entries on
	/// their way are never dropped here: they settle first.
	#[inline(never)]
	fn enforce_limits(&mut self) {
		while !self.waiting.is_empty()
			&& (self.waiting.len() + self.sending.len() > self.limits.queue_entries
				|| self.bytes + self.sending_bytes + self.context_bytes > self.limits.queue_bytes)
		{
			self.drop_oldest();
			self.forget_unused_contexts();
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
		self.version += 1;
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
			self.sending_bytes += next.bytes;
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
		self.reporting = self.dropped;
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

	/// Takes a batch with explicit property names, for delivery such as
	/// `sendBeacon` that cannot read per-event refusals from the response.
	#[cfg(feature = "typed")]
	pub fn take_named(&mut self, now: i64, random: [u8; 10]) -> Option<Pending> {
		if self.is_sending() || now < self.retry_at {
			return None;
		}
		if name_entries(self.waiting.iter_mut(), &self.schemas) {
			self.version += 1;
			self.bytes = self.waiting.iter().map(|entry| entry.bytes).sum();
			self.enforce_limits();
		}
		if self
			.waiting
			.iter()
			.any(|entry| matches!(entry.entry.body, Body::TypedEvent { .. }))
		{
			return None;
		}
		self.take(now, random)
	}

	/// Takes a batch; all entries already carry their property names.
	#[cfg(not(feature = "typed"))]
	pub fn take_named(&mut self, now: i64, random: [u8; 10]) -> Option<Pending> {
		self.take(now, random)
	}

	/// Settles the batch on its way with what the service did with it, at
	/// `now`. `jitter`, from 0 to 1, spreads retries so clients that failed
	/// together don't all retry together.
	pub fn settle(&mut self, outcome: Outcome, now: i64, jitter: f64) {
		self.version += 1;
		match outcome {
			#[cfg(feature = "typed")]
			Outcome::BadRequest => {
				if name_entries(self.sending.iter_mut(), &self.schemas) {
					self.sending_bytes = self.sending.iter().map(|entry| entry.bytes).sum();
					self.put_back();
					self.failures = 0;
					self.retry_at = now;
				} else {
					self.settle(Outcome::Rejected, now, jitter);
				}
			}
			Outcome::Stored | Outcome::Rejected => {
				if outcome == Outcome::Rejected {
					self.dropped += self.sending.len() as u64;
				} else {
					// Drops counted while the batch was on its way are
					// reported by the next one.
					self.dropped -= self.reporting.min(self.dropped);
				}
				self.reporting = 0;
				self.sending.clear();
				self.sending_bytes = 0;
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
					self.sending_bytes = 0;
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

	/// A number that changes whenever what [`snapshot`](Self::snapshot)
	/// would return changes, so a platform saves only when it must.
	pub fn version(&self) -> u64 {
		self.version
	}

	/// Everything the queue holds, waiting or on its way, as files of at
	/// most `max_bytes` each, for a platform to save and
	/// [`restore`](Self::restore) after the app restarts. Each is the
	/// batch format behind a short header, typed events include their property
	/// names, so another app build can restore them without the old schema
	/// metadata. An empty queue is no files.
	pub fn snapshot(&self, max_bytes: usize) -> Vec<Vec<u8>> {
		let all: Vec<&Queued> = self.sending.iter().chain(&self.waiting).collect();
		let mut segments = Vec::new();
		let mut start = 0;
		let mut bytes = 0;
		// Estimates first, at three quarters of the limit; then each is
		// encoded and halved until it fits.
		for (index, queued) in all.iter().enumerate() {
			if index > start && bytes + queued.bytes > max_bytes / 4 * 3 {
				self.encode_segments(&all[start..index], max_bytes, &mut segments);
				start = index;
				bytes = 0;
			}
			bytes += queued.bytes;
		}
		if start < all.len() {
			self.encode_segments(&all[start..], max_bytes, &mut segments);
		}
		if let Some(first) = segments.first_mut() {
			first[1..9].copy_from_slice(&self.dropped.to_le_bytes());
		}
		segments
	}

	fn encode_segments(&self, entries: &[&Queued], max_bytes: usize, out: &mut Vec<Vec<u8>>) {
		let mut used = Vec::new();
		let mut batch = Batch::default();
		for queued in entries {
			let mut entry = queued.entry.clone();
			#[cfg(feature = "typed")]
			if let Some(body) = keyed_body(&entry.body, &self.schemas) {
				entry.body = body;
			}
			entry.context = index_of(&mut used, entry.context);
			batch.entries.push(entry);
		}
		batch.contexts = used
			.iter()
			.map(|generation| self.context(*generation))
			.collect();
		let mut segment = vec![SNAPSHOT_VERSION];
		segment.extend_from_slice(&0_u64.to_le_bytes());
		segment.extend_from_slice(&wire::encode(&batch));
		if segment.len() <= max_bytes || entries.len() <= 1 {
			// A lone entry larger than a file can hold is still saved whole;
			// the platform decides what to do with a file too large.
			out.push(segment);
		} else {
			let (first, second) = entries.split_at(entries.len() / 2);
			self.encode_segments(first, max_bytes, out);
			self.encode_segments(second, max_bytes, out);
		}
	}

	fn context(&self, generation: u32) -> Properties {
		self.contexts
			.iter()
			.find(|(have, _)| *have == generation)
			.map(|(_, context)| context.clone())
			.unwrap_or_default()
	}

	/// Adds the entries of a saved [`snapshot`](Self::snapshot) file, in
	/// order, behind those already queued. They keep their contexts and
	/// their ids, so the service stores each once even if it was sent
	/// before the app stopped, and the time they were recorded, which
	/// [`Limits::max_age`] is measured from.
	pub fn restore(&mut self, segment: &[u8]) -> Result<(), wire::DecodeError> {
		let refused = |reason| wire::DecodeError { offset: 0, reason };
		let (&version, rest) = segment.split_first().ok_or(refused("an empty file"))?;
		if version != SNAPSHOT_VERSION || rest.len() < 8 {
			return Err(refused("not a saved analytics queue"));
		}
		let dropped = u64::from_le_bytes(rest[..8].try_into().expect("8 bytes"));
		let batch = wire::decode(&rest[8..], &wire::Limits::default())?;

		let generations: Vec<u32> = batch
			.contexts
			.into_iter()
			.map(|context| {
				if let Some((generation, _)) =
					self.contexts.iter().find(|(_, have)| *have == context)
				{
					return *generation;
				}
				self.last_generation = self.last_generation.wrapping_add(1);
				self.contexts.push((self.last_generation, context));
				self.last_generation
			})
			.collect();
		self.dropped += dropped;
		for mut entry in batch.entries {
			let Some(generation) = generations.get(entry.context as usize) else {
				continue;
			};
			entry.context = *generation;
			let at = wire::uuid_v7_millis(&entry.uuid).unwrap_or(0);
			self.append_with_context(entry, at);
		}
		// Every restored entry must be present before dropping older ones:
		// otherwise pruning contexts can forget a later entry's context.
		self.forget_unused_contexts();
		self.enforce_limits();
		Ok(())
	}

	fn put_back(&mut self) {
		for queued in self.sending.drain(..).rev() {
			self.bytes += queued.bytes;
			self.waiting.push_front(queued);
		}
		self.sending_bytes = 0;
		self.enforce_limits();
	}

	fn drop_oldest(&mut self) {
		self.version += 1;
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
	#[inline(never)]
	fn forget_unused_contexts(&mut self) {
		let generation = self.generation;
		let (waiting, sending) = (&self.waiting, &self.sending);
		self.contexts.retain(|(have, _)| {
			*have == generation
				|| waiting.iter().any(|queued| queued.entry.context == *have)
				|| sending.iter().any(|queued| queued.entry.context == *have)
		});
		self.context_bytes = self
			.contexts
			.iter()
			.map(|(_, context)| properties_size(context))
			.sum();
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

/// Known field names used for compact events' named fallback.
#[cfg(feature = "typed")]
type Schema = (u64, &'static [(&'static str, &'static str)]);

#[cfg(feature = "typed")]
fn keyed_body(body: &Body, schemas: &[Schema]) -> Option<Body> {
	let Body::TypedEvent {
		name,
		schema_id,
		values,
		properties,
	} = body
	else {
		return None;
	};
	let (_, fields) = schemas.iter().find(|(have, _)| have == schema_id)?;
	if fields.len() != values.len() {
		return None;
	}
	let mut keyed: Properties = fields
		.iter()
		.zip(values)
		.map(|((field, _), value)| ((*field).to_owned(), value.clone()))
		.collect();
	for (key, value) in properties {
		if !keyed.iter().any(|(have, _)| have == key) {
			keyed.push((key.clone(), value.clone()));
		}
	}
	Some(Body::Event {
		name: name.clone(),
		properties: keyed,
	})
}

#[cfg(feature = "typed")]
fn name_entries<'a>(entries: impl Iterator<Item = &'a mut Queued>, schemas: &[Schema]) -> bool {
	let mut named = false;
	for queued in entries {
		if let Some(body) = keyed_body(&queued.entry.body, schemas) {
			queued.entry.body = body;
			queued.bytes = size(&queued.entry);
			named = true;
		}
	}
	named
}

/// Roughly what an entry takes encoded: strings, values and fixed fields.
fn size(entry: &Entry) -> usize {
	let body = match &entry.body {
		Body::Event { name, properties } => name.len() + properties_size(properties),
		#[cfg(feature = "typed")]
		Body::TypedEvent {
			name,
			values,
			properties,
			..
		} => {
			name.len()
				+ 8
				+ values.iter().map(value_size).sum::<usize>()
				+ properties_size(properties)
		}
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

	#[cfg(feature = "typed")]
	fn typed(n: u8, fields: &'static [(&'static str, &'static str)]) -> Entry {
		let mut entry = event(n);
		entry.body = Body::TypedEvent {
			name: "typed".into(),
			schema_id: crate::schema_id("typed", fields),
			values: vec![Value::Text("pro".into())],
			properties: Vec::new(),
		};
		entry
	}

	#[cfg(feature = "typed")]
	const LONG_FIELDS: &[(&str, &str)] = &[(
		concat!(
			"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
			"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
			"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
			"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
		),
		"String",
	)];

	#[cfg(feature = "typed")]
	#[test]
	fn typed_beacon_delivery_uses_names_and_respects_queue_limits() {
		let mut queue = queue(Limits::default());
		queue.remember_schema(crate::schema_id("typed", LONG_FIELDS), LONG_FIELDS);
		queue.push(typed(0, LONG_FIELDS), 0);
		let pending = queue.take_named(0, [0; 10]).unwrap();
		let batch = wire::decode(&pending.body, &wire::Limits::default()).unwrap();
		assert!(matches!(batch.entries[0].body, Body::Event { .. }));
		queue.settle(Outcome::Stored, 0, 0.0);
		queue.limits.queue_bytes = 150;
		queue.push(typed(1, LONG_FIELDS), 0);
		assert!(
			queue.take_named(0, [0; 10]).is_none(),
			"expanded names stay within the byte bound"
		);
		assert_eq!(queue.dropped(), 1);
	}

	#[cfg(feature = "typed")]
	#[test]
	fn typed_snapshot_keeps_names_without_the_old_build_s_schema_metadata() {
		let mut source = queue(Limits::default());
		source.remember_schema(crate::schema_id("typed", LONG_FIELDS), LONG_FIELDS);
		let entry = typed(0, LONG_FIELDS);
		let uuid = entry.uuid;
		source.push(entry, 0);
		let mut restored = queue(Limits::default());
		for segment in source.snapshot(1 << 20) {
			restored.restore(&segment).unwrap();
		}
		let pending = restored.take(0, [0; 10]).unwrap();
		let batch = wire::decode(&pending.body, &wire::Limits::default()).unwrap();
		assert_eq!(batch.entries[0].uuid, uuid);
		assert!(
			matches!(&batch.entries[0].body, Body::Event { properties, .. }
			if properties[0].0 == LONG_FIELDS[0].0)
		);
	}

	#[cfg(feature = "typed")]
	#[test]
	fn typed_old_service_bad_request_retries_the_whole_batch_with_names() {
		let mut queue = queue(Limits::default());
		queue.remember_schema(crate::schema_id("typed", LONG_FIELDS), LONG_FIELDS);
		queue.push(event(1), 0);
		queue.push(typed(0, LONG_FIELDS), 0);
		let before = queue.take(0, [0; 10]).unwrap();
		queue.settle(Outcome::from_status(400, None), 0, 0.0);
		assert_eq!(queue.len(), 2, "the old service accepted neither entry");
		let after = queue.take(0, [0; 10]).unwrap();
		let before = wire::decode(&before.body, &wire::Limits::default()).unwrap();
		let after = wire::decode(&after.body, &wire::Limits::default()).unwrap();
		assert_eq!(after.entries[0], before.entries[0]);
		assert_eq!(after.entries[1].uuid, before.entries[1].uuid);
		assert!(matches!(after.entries[1].body, Body::Event { .. }));
		queue.settle(Outcome::from_status(400, None), 0, 0.0);
		assert_eq!(queue.len(), 0, "a keyed bad request does not retry forever");
	}

	#[cfg(feature = "typed")]
	#[test]
	fn typed_fallback_keeps_the_original_context_and_field_precedence() {
		static FIELDS: &[(&str, &str)] = &[("plan", "String")];
		let old = vec![("$lib_version".into(), Value::Text("old".into()))];
		let mut queue = Queue::new(Limits::default(), old.clone());
		queue.remember_schema(crate::schema_id("typed", FIELDS), FIELDS);
		let mut entry = typed(0, FIELDS);
		let uuid = entry.uuid;
		if let Body::TypedEvent { properties, .. } = &mut entry.body {
			properties.extend([
				("plan".into(), Value::Text("shadow".into())),
				("registered".into(), Value::Bool(true)),
			]);
		}
		queue.push(entry, 0);
		queue.take(0, [0; 10]).unwrap();
		queue.set_context(vec![("$lib_version".into(), Value::Text("new".into()))]);
		queue.settle_with_refusals(&[(0, "unknown_schema")], 0, 0.0);
		let resent = queue.take(0, [0; 10]).unwrap();
		let batch = wire::decode(&resent.body, &wire::Limits::default()).unwrap();
		assert_eq!(batch.entries[0].uuid, uuid);
		assert_eq!(batch.contexts[batch.entries[0].context as usize], old);
		assert_eq!(
			batch.entries[0].body,
			Body::Event {
				name: "typed".into(),
				properties: vec![
					("plan".into(), Value::Text("pro".into())),
					("registered".into(), Value::Bool(true)),
				],
			}
		);
	}

	#[cfg(feature = "typed")]
	#[test]
	fn typed_fallback_drops_an_entry_that_expands_past_either_byte_limit() {
		for limits in [
			Limits {
				queue_bytes: 128,
				..Limits::default()
			},
			Limits {
				batch_bytes: 128,
				..Limits::default()
			},
		] {
			let mut queue = Queue::new(limits, Vec::new());
			queue.remember_schema(crate::schema_id("typed", LONG_FIELDS), LONG_FIELDS);
			queue.push(typed(0, LONG_FIELDS), 0);
			queue.take(0, [0; 10]).unwrap();
			queue.settle_with_refusals(&[(0, "unknown_schema")], 0, 0.0);
			assert!(queue.is_empty());
			assert_eq!(queue.dropped(), 1);
		}
	}

	#[cfg(feature = "typed")]
	#[test]
	fn typed_fallback_enforces_the_total_waiting_byte_limit() {
		let mut queue = Queue::new(
			Limits {
				batch_entries: 1,
				queue_bytes: 280,
				batch_bytes: 280,
				..Limits::default()
			},
			Vec::new(),
		);
		queue.remember_schema(crate::schema_id("typed", LONG_FIELDS), LONG_FIELDS);
		queue.push(typed(0, LONG_FIELDS), 0);
		queue.take(0, [0; 10]).unwrap();
		queue.push(event(1), 0);
		queue.settle_with_refusals(&[(0, "unknown_schema")], 0, 0.0);
		assert_eq!(queue.len(), 1);
		assert_eq!(queue.dropped(), 1);
		assert_eq!(
			names(&queue.take(0, [0; 10]).unwrap()),
			["event 1", "$lk_dropped"]
		);
	}

	#[cfg(feature = "typed")]
	#[test]
	fn typed_fallback_counts_the_retained_context_in_its_byte_limit() {
		let mut queue = Queue::new(
			Limits {
				batch_entries: 1,
				queue_bytes: 400,
				batch_bytes: 400,
				..Limits::default()
			},
			vec![("padding".into(), Value::Text("x".repeat(140)))],
		);
		queue.remember_schema(crate::schema_id("typed", LONG_FIELDS), LONG_FIELDS);
		queue.push(typed(0, LONG_FIELDS), 0);
		queue.take(0, [0; 10]).unwrap();
		queue.push(event(1), 0);
		queue.settle_with_refusals(&[(0, "unknown_schema")], 0, 0.0);
		assert_eq!(queue.len(), 1);
		assert_eq!(queue.dropped(), 1);
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
	fn an_entry_too_large_to_send_is_dropped_when_queued() {
		let mut queue = queue(Limits {
			batch_bytes: 200,
			..Limits::default()
		});
		let mut large = event(0);
		large.body = Body::Event {
			name: "large".into(),
			properties: vec![("text".into(), Value::Text("x".repeat(500)))],
		};
		queue.push(large, 0);
		assert_eq!(queue.len(), 0);
		assert_eq!(queue.dropped(), 1);
	}

	#[test]
	fn a_batch_on_its_way_counts_against_the_limits() {
		let mut queue = queue(Limits {
			batch_entries: 3,
			queue_entries: 4,
			..Limits::default()
		});
		for n in 0..3 {
			queue.push(event(n), 0);
		}
		queue.take(0, [0; 10]).unwrap();
		for n in 3..6 {
			queue.push(event(n), 0);
		}
		assert_eq!(queue.len(), 1, "three on their way, one waiting");
		assert_eq!(queue.dropped(), 2);
		queue.settle(Outcome::Retry { after: None }, 0, 0.0);
		assert_eq!(queue.len(), 4);
	}

	#[test]
	fn contexts_count_against_the_byte_limit() {
		let small = size(&event(0));
		let mut queue = queue(Limits {
			queue_bytes: 3 * small + 40,
			..Limits::default()
		});
		for n in 0..3 {
			queue.push(event(n), 0);
		}
		assert_eq!(queue.len(), 3);
		queue.set_context(vec![("$lib".into(), Value::Text("x".repeat(small)))]);
		assert!(
			queue.len() < 3,
			"a larger context leaves room for fewer entries"
		);
		let held = queue.bytes + queue.sending_bytes + queue.context_bytes;
		assert!(held <= 3 * small + 40);
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
	fn restoring_with_a_smaller_limit_keeps_the_last_entry_s_context() {
		let mut source = queue(Limits::default());
		for number in 0..3 {
			source.set_context(vec![("version".into(), Value::Int(number))]);
			source.push(event(number as u8), 1_000);
		}
		let files = source.snapshot(1 << 20);
		let mut restored = queue(Limits {
			queue_entries: 1,
			..Limits::default()
		});
		for file in files {
			restored.restore(&file).unwrap();
		}
		let pending = restored.take(1_000, [0; 10]).unwrap();
		let batch = wire::decode(&pending.body, &wire::Limits::default()).unwrap();
		assert_eq!(batch.entries[0].body, event(2).body);
		assert_eq!(
			batch.contexts[batch.entries[0].context as usize],
			vec![("version".into(), Value::Int(2))]
		);
		assert_eq!(restored.dropped(), 2);
		restored.settle(Outcome::Stored, 1_000, 0.0);
		assert_eq!(
			restored.dropped(),
			0,
			"the successful batch acknowledges the drops"
		);
	}

	#[test]
	fn a_snapshot_restores_every_entry_in_order_with_its_context() {
		let mut queue = queue(Limits {
			batch_entries: 2,
			queue_entries: 3,
			..Limits::default()
		});
		for n in 0..4 {
			queue.push(event(n), 1_000);
		}
		queue.set_context(vec![("$lib".into(), Value::Text("second".into()))]);
		queue.push(event(4), 1_000);
		queue.take(1_000, [0; 10]).unwrap();
		let before = queue.version();
		let files = queue.snapshot(1 << 20);
		assert_eq!(files.len(), 1);
		assert_eq!(queue.version(), before, "a snapshot changes nothing");

		let mut restored = Queue::new(
			Limits::default(),
			vec![("$lib".into(), Value::Text("new".into()))],
		);
		for file in &files {
			restored.restore(file).unwrap();
		}
		assert_eq!(restored.len(), 3);
		assert_eq!(restored.dropped(), 2, "the drops are still reported");
		let pending = restored.take(1_000, [0; 10]).unwrap();
		assert_eq!(
			names(&pending),
			["event 2", "event 3", "event 4", "$lk_dropped"]
		);
		let batch = wire::decode(&pending.body, &wire::Limits::default()).unwrap();
		assert_eq!(batch.contexts.len(), 2);
		assert_eq!(
			batch.contexts[batch.entries[2].context as usize][0].1,
			Value::Text("second".into())
		);

		restored.settle(Outcome::Stored, 1_000, 0.0);
		restored.push(event(9), 1_000);
		let batch = wire::decode(
			&restored.take(1_000, [0; 10]).unwrap().body,
			&wire::Limits::default(),
		)
		.unwrap();
		assert_eq!(
			batch.contexts,
			[vec![("$lib".to_owned(), Value::Text("new".into()))]],
			"new entries use the current context"
		);
	}

	#[test]
	fn a_large_queue_is_saved_in_files_under_the_limit() {
		let mut queue = queue(Limits::default());
		for n in 0..200 {
			let mut entry = event(n);
			entry.body = Body::Event {
				name: format!("event {n}"),
				properties: vec![("text".into(), Value::Text(format!("{n:0>200}")))],
			};
			queue.push(entry, 1_000);
		}
		let files = queue.snapshot(8 << 10);
		assert!(files.len() > 3);
		assert!(files.iter().all(|file| file.len() <= 8 << 10));
		let mut restored = queue_with_default();
		for file in &files {
			restored.restore(file).unwrap();
		}
		assert_eq!(restored.len(), 200);
	}

	#[test]
	fn a_file_that_isn_t_a_snapshot_is_refused() {
		let mut queue = queue_with_default();
		assert!(queue.restore(b"").is_err());
		assert!(queue.restore(b"\x02whatever").is_err());
		assert!(queue.restore(b"\x01\0\0\0\0\0\0\0\0garbage").is_err());
		assert!(queue.is_empty());
	}

	fn queue_with_default() -> Queue {
		queue(Limits::default())
	}

	#[test]
	fn statuses_mean_what_http_says() {
		assert_eq!(Outcome::from_status(200, None), Outcome::Stored);
		#[cfg(feature = "typed")]
		assert_eq!(Outcome::from_status(400, None), Outcome::BadRequest);
		#[cfg(not(feature = "typed"))]
		assert_eq!(Outcome::from_status(400, None), Outcome::Rejected);
		assert_eq!(Outcome::from_status(401, None), Outcome::Rejected);
		assert_eq!(Outcome::from_status(413, None), Outcome::TooLarge);
		assert_eq!(
			Outcome::from_status(503, Some("2")),
			Outcome::Retry { after: Some(2_000) }
		);
		assert_eq!(
			Outcome::from_status(429, Some(" \t2 \t")),
			Outcome::Retry { after: Some(2_000) }
		);
		assert_eq!(
			Outcome::from_status(429, Some("\u{a0}2")),
			Outcome::Retry { after: None }
		);
		assert_eq!(
			Outcome::from_status(429, Some("soon")),
			Outcome::Retry { after: None }
		);
	}
}
