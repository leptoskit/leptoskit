//! The ingest queue and the batch writer.
//!
//! Every request's rows go into one bounded queue. One task drains it in
//! batches and writes each batch in one transaction, then answers every
//! request whose rows were in it. Many requests share each commit, so the
//! database sees a few large transactions rather than many small ones,
//! while every `200` still means the rows are stored ("group commit").
//!
//! The queue is bounded in rows and in bytes, and it's the only buffer, so
//! memory is bounded by configuration rather than by load. When it's full,
//! [`Ingest::submit`] fails at once and the request answers `503`.

use std::fmt::Write as _;
use std::sync::Arc;
use std::time::Duration;

use leptoskit_db::Connection as _;
use leptoskit_db::Db;
use leptoskit_db::Dialect;
use leptoskit_db::Read as _;
use leptoskit_db::Statement;
use leptoskit_db::Timestamp;
use leptoskit_db::Value;
use tokio::sync::OwnedSemaphorePermit;
use tokio::sync::Semaphore;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::time::Instant;

use super::accept::Accepted;
use super::accept::ContextRow;
use super::accept::Row;
use super::limits::Limiter;
use super::limits::Rate;

/// How the queue and the writer are sized.
#[derive(Clone, Copy, Debug)]
pub struct IngestConfig {
	/// The most rows waiting to be written.
	pub queue_rows: usize,
	/// The most bytes waiting to be written, roughly.
	pub queue_bytes: usize,
	/// The most rows the writer puts in one transaction.
	pub batch_rows: usize,
	/// How long the writer waits for more rows before writing what it has.
	pub batch_wait: Duration,
	/// Each `distinct_id`'s rate limit, or `None` for none.
	pub per_distinct_id: Option<Rate>,
	/// When a request is answered.
	pub ack: Ack,
}

/// When a request that sent events is answered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Ack {
	/// Once its events are committed: every answer means they're stored.
	#[default]
	Committed,
	/// As soon as its events are queued: faster, and a crash loses at most
	/// what's queued. Duplicates aren't known yet, so none are reported.
	Queued,
}

impl Ack {
	/// `"queued"` or `"committed"`, as `LEPTOSKIT_ANALYTICS_ACK` holds it.
	pub fn parse(value: &str) -> Option<Self> {
		match value.trim() {
			"committed" => Some(Self::Committed),
			"queued" => Some(Self::Queued),
			_ => None,
		}
	}
}

impl Default for IngestConfig {
	fn default() -> Self {
		Self {
			queue_rows: 50_000,
			queue_bytes: 64 << 20,
			batch_rows: 2_000,
			batch_wait: Duration::from_millis(50),
			per_distinct_id: Some(Rate::PER_DISTINCT_ID),
			ack: Ack::Committed,
		}
	}
}

/// What a request's rows came to once written.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Committed {
	/// Rows written now.
	pub stored: u32,
	/// Rows already written by an earlier attempt, so not written again.
	pub duplicates: u32,
	/// Rows dropped because their `distinct_id` sent too many too fast.
	pub rate_limited: u32,
}

/// Why rows weren't written.
#[derive(Debug)]
pub enum IngestError {
	/// The queue is full: try again later.
	Full,
	/// The writer has stopped.
	Closed,
	/// The transaction failed: nothing in it was written.
	Database(String),
}

impl std::fmt::Display for IngestError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Full => f.write_str("the ingest queue is full"),
			Self::Closed => f.write_str("the batch writer has stopped"),
			Self::Database(error) => write!(f, "writing the batch failed: {error}"),
		}
	}
}

impl std::error::Error for IngestError {}

struct Submission {
	rows: Vec<Row>,
	contexts: Vec<ContextRow>,
	warnings: Vec<Warning>,
	received_at: Timestamp,
	done: oneshot::Sender<Result<Committed, String>>,
	_rows_permit: OwnedSemaphorePermit,
	_bytes_permit: OwnedSemaphorePermit,
}

/// A warning row for `ingestion_warnings`.
#[derive(Clone, Debug)]
pub struct Warning {
	pub reason: String,
	pub count: u32,
	pub detail: String,
}

/// The handle requests submit rows through. Cloning it is cheap.
#[derive(Clone)]
pub struct Ingest {
	queue: mpsc::UnboundedSender<Submission>,
	rows: Arc<Semaphore>,
	bytes: Arc<Semaphore>,
	config: IngestConfig,
	limiter: Option<Arc<Limiter>>,
}

/// The unit the byte limit counts in, so it fits a semaphore's permits.
const BYTE_UNIT: usize = 1024;

impl Ingest {
	/// Starts the batch writer on `db`, on the current Tokio runtime.
	pub fn start(db: Db, config: IngestConfig) -> Self {
		let (queue, receiver) = mpsc::unbounded_channel();
		tokio::spawn(write_batches(db, receiver, config));
		Self {
			queue,
			rows: Arc::new(Semaphore::new(config.queue_rows)),
			bytes: Arc::new(Semaphore::new(config.queue_bytes.div_ceil(BYTE_UNIT))),
			config,
			limiter: config
				.per_distinct_id
				.map(|rate| Arc::new(Limiter::new(rate))),
		}
	}

	/// Queues `accepted` and waits until it's written. Refused entries and
	/// clamped clocks are written as warnings in the same transaction.
	///
	/// # Errors
	///
	/// [`IngestError::Full`] at once when the queue has no room, and the
	/// others when the write fails.
	pub async fn submit(
		&self,
		mut accepted: Accepted,
		received_at: Timestamp,
	) -> Result<Committed, IngestError> {
		let mut warnings = Vec::new();
		let rate_limited = self.limiter.as_ref().map_or(0, |limiter| {
			limiter.admit(&mut accepted.rows, std::time::Instant::now())
		});
		if rate_limited > 0 {
			warnings.push(Warning {
				reason: "rate_limited".into(),
				count: rate_limited,
				detail: "a distinct id sent more than its rate limit".into(),
			});
		}
		if !accepted.refused.is_empty() {
			warnings.push(Warning {
				reason: "invalid".into(),
				count: accepted.refused.len() as u32,
				detail: accepted.refused[0].reason.clone(),
			});
		}
		if accepted.clamped > 0 {
			warnings.push(Warning {
				reason: "clock_clamped".into(),
				count: accepted.clamped,
				detail: "the client's clock was more than 23 hours ahead".into(),
			});
		}
		if accepted.rows.is_empty() && warnings.is_empty() {
			return Ok(Committed::default());
		}
		let finish = |committed: Committed| {
			Committed {
				rate_limited,
				..committed
			}
		};

		let rows = u32::try_from(accepted.rows.len().max(1)).unwrap_or(u32::MAX);
		let bytes = u32::try_from(accepted.bytes().div_ceil(BYTE_UNIT).max(1)).unwrap_or(u32::MAX);
		if rows as usize > self.config.queue_rows
			|| bytes as usize > self.config.queue_bytes.div_ceil(BYTE_UNIT)
		{
			return Err(IngestError::Full);
		}
		let rows_permit = Arc::clone(&self.rows)
			.try_acquire_many_owned(rows)
			.map_err(|_| IngestError::Full)?;
		let bytes_permit = Arc::clone(&self.bytes)
			.try_acquire_many_owned(bytes)
			.map_err(|_| IngestError::Full)?;

		let (done, wait) = oneshot::channel();
		let queued = accepted.rows.len() as u32;
		self.queue
			.send(Submission {
				rows: accepted.rows,
				contexts: accepted.contexts,
				warnings,
				received_at,
				done,
				_rows_permit: rows_permit,
				_bytes_permit: bytes_permit,
			})
			.map_err(|_| IngestError::Closed)?;

		if self.config.ack == Ack::Queued {
			// The writer still commits them; nobody waits to hear it.
			drop(wait);
			return Ok(finish(Committed {
				stored: queued,
				..Committed::default()
			}));
		}
		match wait.await {
			Ok(Ok(committed)) => Ok(finish(committed)),
			Ok(Err(error)) => Err(IngestError::Database(error)),
			Err(_) => Err(IngestError::Closed),
		}
	}

	/// How many rows are waiting to be written.
	pub fn queued_rows(&self) -> usize {
		self.config.queue_rows - self.rows.available_permits()
	}
}

async fn write_batches(
	db: Db,
	mut receiver: mpsc::UnboundedReceiver<Submission>,
	config: IngestConfig,
) {
	while let Some(first) = receiver.recv().await {
		let mut rows = first.rows.len();
		let mut submissions = vec![first];
		let deadline = Instant::now() + config.batch_wait;
		while rows < config.batch_rows {
			match tokio::time::timeout_at(deadline, receiver.recv()).await {
				Ok(Some(submission)) => {
					rows += submission.rows.len();
					submissions.push(submission);
				}
				Ok(None) | Err(_) => break,
			}
		}

		match write(&db, &submissions).await {
			Ok(stored) => {
				for (submission, committed) in submissions.into_iter().zip(stored) {
					let _ = submission.done.send(Ok(committed));
				}
			}
			Err(error) => {
				let message = error.to_string();
				for submission in submissions {
					let _ = submission.done.send(Err(message.clone()));
				}
			}
		}
	}
}

/// Rows per `INSERT`, which keeps every statement well under SQLite's and
/// Postgres's limits on bound values.
const ROWS_PER_STATEMENT: usize = 500;

/// Writes every submission in one transaction, and says what each came to.
async fn write(db: &Db, submissions: &[Submission]) -> leptoskit_db::Result<Vec<Committed>> {
	let dialect = db.dialect();
	let tx = db.begin().await?;

	let mut contexts: Vec<&ContextRow> = Vec::new();
	for context in submissions
		.iter()
		.flat_map(|submission| &submission.contexts)
	{
		if !contexts.iter().any(|known| known.id == context.id) {
			contexts.push(context);
		}
	}
	let now = submissions[0].received_at;
	for chunk in contexts.chunks(ROWS_PER_STATEMENT) {
		let mut statement = Insert::new(dialect, "contexts", &["id", "properties", "first_seen"]);
		for context in chunk {
			statement.row(vec![
				Value::Int(context.id),
				Value::Text(context.properties.clone()),
				Value::Timestamp(now),
			]);
		}
		tx.execute(statement.on_conflict_do_nothing("id")).await?;
	}

	// Which rows each submission wrote, by the names `RETURNING` gives back.
	let mut committed = vec![Committed::default(); submissions.len()];
	let mut volumes: Vec<(String, i64, Timestamp, Timestamp)> = Vec::new();
	for (index, submission) in submissions.iter().enumerate() {
		for chunk in submission.rows.chunks(ROWS_PER_STATEMENT) {
			let mut statement = Insert::new(
				dialect,
				"events",
				&[
					"uuid",
					"name",
					"distinct_id",
					"person_id",
					"timestamp",
					"received_at",
					"session_id",
					"context_id",
					"properties",
					"source",
				],
			);
			for row in chunk {
				statement.row(vec![
					Value::Uuid(row.uuid),
					Value::Text(row.name.clone()),
					Value::Text(row.distinct_id.clone()),
					Value::Null,
					Value::Timestamp(row.timestamp),
					Value::Timestamp(submission.received_at),
					row.session_id.map_or(Value::Null, Value::Uuid),
					Value::Int(row.context_id),
					Value::Text(row.properties.clone()),
					Value::Int(row.source as i64),
				]);
			}
			let mut statement = statement.on_conflict_do_nothing("uuid");
			statement.sql.push_str(" RETURNING \"name\", \"timestamp\"");
			let written = tx.fetch(statement).await?;
			let stored = written.len();
			committed[index].stored += stored as u32;
			committed[index].duplicates += (chunk.len() - stored) as u32;
			for row in written {
				let (Some(Value::Text(name)), Some(timestamp)) = (row.first().cloned(), row.get(1))
				else {
					continue;
				};
				let timestamp = timestamp_of(timestamp).unwrap_or(now);
				match volumes.iter_mut().find(|volume| volume.0 == name) {
					Some(volume) => {
						volume.1 += 1;
						volume.2 = volume.2.min(timestamp);
						volume.3 = volume.3.max(timestamp);
					}
					None => volumes.push((name, 1, timestamp, timestamp)),
				}
			}
		}
	}

	let greatest = match dialect {
		Dialect::Postgres => "GREATEST",
		Dialect::Sqlite => "MAX",
	};
	let least = match dialect {
		Dialect::Postgres => "LEAST",
		Dialect::Sqlite => "MIN",
	};
	for chunk in volumes.chunks(ROWS_PER_STATEMENT) {
		let mut statement = Insert::new(
			dialect,
			"event_definitions",
			&["name", "first_seen", "last_seen", "volume"],
		);
		for (name, volume, first, last) in chunk {
			statement.row(vec![
				Value::Text(name.clone()),
				Value::Timestamp(*first),
				Value::Timestamp(*last),
				Value::Int(*volume),
			]);
		}
		let mut statement = statement.finish();
		let _ = write!(
			statement.sql,
			" ON CONFLICT (\"name\") DO UPDATE SET \"first_seen\" = \
			 {least}(\"event_definitions\".\"first_seen\", excluded.\"first_seen\"), \
			 \"last_seen\" = {greatest}(\"event_definitions\".\"last_seen\", \
			 excluded.\"last_seen\"), \"volume\" = \"event_definitions\".\"volume\" + \
			 excluded.\"volume\""
		);
		tx.execute(statement).await?;
	}

	let warnings: Vec<&Warning> = submissions
		.iter()
		.flat_map(|submission| &submission.warnings)
		.collect();
	for chunk in warnings.chunks(ROWS_PER_STATEMENT) {
		let mut statement = Insert::new(
			dialect,
			"ingestion_warnings",
			&["id", "at", "reason", "count", "detail"],
		);
		for warning in chunk {
			let mut detail = warning.detail.clone();
			truncate(&mut detail, 400);
			statement.row(vec![
				Value::Uuid(uuid::Uuid::new_v4()),
				Value::Timestamp(now),
				Value::Text(warning.reason.clone()),
				Value::Int(i64::from(warning.count)),
				Value::Text(detail),
			]);
		}
		tx.execute(statement.finish()).await?;
	}

	tx.commit().await?;
	Ok(committed)
}

fn timestamp_of(value: &Value) -> Option<Timestamp> {
	match value {
		Value::Timestamp(timestamp) => Some(*timestamp),
		Value::Text(text) => {
			chrono::DateTime::parse_from_rfc3339(text)
				.ok()
				.map(|time| time.to_utc())
		}
		_ => None,
	}
}

fn truncate(text: &mut String, max: usize) {
	if text.len() > max {
		let mut end = max;
		while !text.is_char_boundary(end) {
			end -= 1;
		}
		text.truncate(end);
	}
}

/// A multi-row `INSERT`, numbering placeholders for the dialect.
struct Insert {
	dialect: Dialect,
	sql: String,
	args: Vec<Value>,
	rows: usize,
}

impl Insert {
	fn new(dialect: Dialect, table: &str, columns: &[&str]) -> Self {
		let columns = columns
			.iter()
			.map(|column| format!("\"{column}\""))
			.collect::<Vec<_>>()
			.join(", ");
		Self {
			dialect,
			sql: format!("INSERT INTO \"{table}\" ({columns}) VALUES "),
			args: Vec::new(),
			rows: 0,
		}
	}

	fn row(&mut self, values: Vec<Value>) {
		if self.rows > 0 {
			self.sql.push_str(", ");
		}
		self.sql.push('(');
		for (index, value) in values.into_iter().enumerate() {
			if index > 0 {
				self.sql.push_str(", ");
			}
			self.args.push(value);
			let number = self.args.len();
			let _ = match self.dialect {
				Dialect::Postgres => write!(self.sql, "${number}"),
				Dialect::Sqlite => write!(self.sql, "?{number}"),
			};
		}
		self.sql.push(')');
		self.rows += 1;
	}

	fn finish(self) -> Statement {
		Statement {
			sql: self.sql,
			args: self.args,
		}
	}

	fn on_conflict_do_nothing(self, key: &str) -> Statement {
		let mut statement = self.finish();
		let _ = write!(statement.sql, " ON CONFLICT (\"{key}\") DO NOTHING");
		statement
	}
}
