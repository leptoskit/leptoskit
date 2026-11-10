//! The service's tables. The batch writer fills them with SQL of its own
//! (one statement per table per batch), so these declarations exist for the
//! migrations and for reading.

use leptoskit_db::Timestamp;
use leptoskit_db::table;
use uuid::Uuid;

/// One event, or one of the identity changes that are entries of their own
/// in a batch (`$identify`, `$create_alias`, `$set`, `$groupidentify`).
#[table(
	name = "events",
	index(timestamp),
	index(name, timestamp),
	index(distinct_id, timestamp),
	index(session_id)
)]
pub struct Event {
	/// The UUID v7 the client made when the event happened: a retried batch
	/// writes it once.
	#[db(key)]
	pub uuid: Uuid,
	#[db(min_len = 1, max_len = 200)]
	pub name: String,
	#[db(min_len = 1, max_len = 200)]
	pub distinct_id: String,
	/// The person, once person processing has run. Null until then, and
	/// for events without a person.
	pub person_id: Option<Uuid>,
	/// When it happened, corrected for the client's clock.
	pub timestamp: Timestamp,
	/// When the service accepted it.
	pub received_at: Timestamp,
	pub session_id: Option<Uuid>,
	/// The [`EventContext`] it was sent with.
	pub context_id: i64,
	/// The event's own properties, as a JSON object.
	pub properties: String,
	/// Which kind of client sent it: see [`Source`](crate::service::Source).
	pub source: i16,
}

/// What's the same for many events from one device: the library, the
/// operating system, the screen. Stored once, keyed by a hash of its
/// properties.
#[table(name = "contexts")]
pub struct EventContext {
	#[db(key)]
	pub id: i64,
	/// The properties, as a JSON object.
	pub properties: String,
	pub first_seen: Timestamp,
}

/// Every event name the service has seen.
#[table(name = "event_definitions")]
pub struct EventDefinition {
	#[db(key, min_len = 1, max_len = 200)]
	pub name: String,
	pub first_seen: Timestamp,
	pub last_seen: Timestamp,
	/// How many have been stored.
	pub volume: i64,
}

/// Entries the service refused or changed, counted by reason, so nothing is
/// dropped without a trace.
#[table(name = "ingestion_warnings", index(at))]
pub struct IngestionWarning {
	#[db(key)]
	pub id: Uuid,
	pub at: Timestamp,
	/// A short code, such as `invalid` or `clock_clamped`.
	#[db(min_len = 1, max_len = 40)]
	pub reason: String,
	/// How many entries it covers.
	pub count: i64,
	/// An example of what was wrong.
	#[db(max_len = 400)]
	pub detail: String,
}

leptoskit_db::export_schema!();

/// Every migration, embedded for the service to apply when it starts.
pub static MIGRATIONS: &[leptoskit_db::Migration] = leptoskit_db::migrations!("migrations");

/// The same migrations written for Postgres.
#[cfg(feature = "postgres")]
pub static POSTGRES_MIGRATIONS: &[leptoskit_db::Migration] =
	leptoskit_db::migrations!("migrations-postgres");
