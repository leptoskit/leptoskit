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

/// One window of an application-performance metric, as `leptoskit_apm`
/// exports it over OTLP: a delta histogram point, such as how long one
/// route took over one minute. Keyed by a hash of its series and start, so
/// a retried export stores each window once.
#[table(name = "apm_windows", index(metric, start), index(operation, start))]
pub struct ApmWindow {
	#[db(key)]
	pub id: i64,
	/// The metric's name, such as `leptoskit.operation.duration`.
	#[db(min_len = 1, max_len = 200)]
	pub metric: String,
	#[db(max_len = 200)]
	pub unit: String,
	/// The resource's `service.name`, `service.instance.id`,
	/// `service.version` and `deployment.environment.name`.
	#[db(max_len = 200)]
	pub service: String,
	#[db(max_len = 200)]
	pub instance: String,
	#[db(max_len = 200)]
	pub release: String,
	#[db(max_len = 200)]
	pub environment: String,
	/// The point's `leptoskit.operation`, such as a route.
	#[db(max_len = 400)]
	pub operation: String,
	/// The point's other attributes, as a JSON object with sorted keys.
	pub attributes: String,
	pub start: Timestamp,
	pub end: Timestamp,
	pub count: i64,
	pub sum: Option<f64>,
	pub max: Option<f64>,
	/// The histogram's counts, as a JSON array, one more than its bounds.
	pub bucket_counts: String,
	/// The histogram's upper bounds, as a JSON array.
	pub bounds: String,
}

/// One version of a typed event's schema ([`crate::Event`]): what a typed
/// event's values are called, by schema id. Versions are kept, never
/// replaced, so events from clients built against an older version still
/// read correctly.
#[table(name = "event_schemas", index(name))]
pub struct EventSchema {
	/// The schema id, as the two's-complement `i64` of the `u64`.
	#[db(key)]
	pub schema_id: i64,
	#[db(min_len = 1, max_len = 200)]
	pub name: String,
	/// Each property's name and type as spelled, as a JSON array of pairs.
	pub fields: String,
	pub first_registered: Timestamp,
	pub last_registered: Timestamp,
}

/// A person: the properties known about someone, and whether the app has
/// identified them (§8 of the design). Their distinct ids live in
/// [`PersonDistinctId`].
#[table(name = "persons", index(created_at))]
pub struct Person {
	/// Made up when the person was created; events point at it.
	#[db(key)]
	pub id: Uuid,
	/// The person's properties, as a JSON object.
	pub properties: String,
	/// Each property's last write, as a JSON object of name to instant, so
	/// an offline client's late events don't overwrite newer ones: `$set`
	/// and `$set_once` win by event time per property, not by arrival.
	pub properties_updated_at: String,
	/// When the person was created.
	pub created_at: Timestamp,
	/// Whether the app identified this person with `$identify`.
	pub is_identified: bool,
	/// Bumped every time the row changes, so a reader can tell a newer
	/// properties answer from an older one.
	pub version: i64,
}

/// One distinct id that belongs to a person: the id a client records
/// events under. Keyed by the id, so resolving an event's person is one
/// indexed read.
#[table(name = "person_distinct_ids")]
pub struct PersonDistinctId {
	#[db(key, min_len = 1, max_len = 200)]
	pub distinct_id: String,
	/// The person the id belongs to.
	pub person_id: Uuid,
	/// When the id was first recorded.
	pub created_at: Timestamp,
}

/// A recorded merge (§8): events stored under `distinct_id` now belong to
/// the person `person_id` holds, found by queries through this row instead
/// of rewriting every stored event.
#[table(name = "person_overrides")]
pub struct PersonOverride {
	/// The distinct id that belonged to another person before a merge.
	#[db(key, min_len = 1, max_len = 200)]
	pub distinct_id: String,
	/// The person its events belong to now.
	pub person_id: Uuid,
	/// When the merge happened.
	pub created_at: Timestamp,
}

/// A group events are about besides the person, such as a company (§8),
/// with the properties `$groupidentify` set on it.
#[table(name = "groups")]
pub struct Group {
	/// A hash of the type's index and the key, so the writer can upsert
	/// without reading first.
	#[db(key)]
	pub id: i64,
	/// The [`GroupType`]'s index.
	pub type_index: i16,
	#[db(min_len = 1, max_len = 200)]
	pub key: String,
	/// The group's properties, as a JSON object.
	pub properties: String,
	/// Each property's last write, as [`Person::properties_updated_at`].
	pub properties_updated_at: String,
	pub created_at: Timestamp,
}

/// A kind of group the project uses, such as `company` or `team`. At most
/// five per project, as `PostHog`'s are; the first `$groupidentify` for a
/// new name takes the next index.
#[table(name = "group_types")]
pub struct GroupType {
	#[db(key, min_len = 1, max_len = 40)]
	pub name: String,
	/// The index in an event's `$groups` and in [`Group::type_index`].
	pub index: i16,
	pub created_at: Timestamp,
}

/// One visit (§8): the batch writer maintains one row per session id,
/// upserting it with every batch that carries the session, so web
/// analytics reads sessions rather than every event.
#[table(
	name = "sessions",
	index(first_timestamp),
	index(person_id),
	index(distinct_id, last_timestamp)
)]
pub struct Session {
	/// The client's session id, a UUID v7.
	#[db(key)]
	pub id: Uuid,
	/// The id the session's events are recorded under.
	#[db(min_len = 1, max_len = 200)]
	pub distinct_id: String,
	/// The person, once person processing has run. Null for sessions
	/// without a person.
	pub person_id: Option<Uuid>,
	/// The session's first event.
	pub first_timestamp: Timestamp,
	/// The session's last event so far.
	pub last_timestamp: Timestamp,
	/// `last_timestamp - first_timestamp`, in milliseconds.
	pub duration_ms: i64,
	/// The first page's address, as `$current_url` held it.
	#[db(max_len = 2_000)]
	pub entry_url: String,
	#[db(max_len = 400)]
	pub entry_pathname: String,
	/// The latest page's address.
	#[db(max_len = 2_000)]
	pub exit_url: String,
	#[db(max_len = 400)]
	pub exit_pathname: String,
	/// How many of the session's events are stored.
	pub event_count: i64,
	/// How many were `$pageview`s.
	pub pageview_count: i64,
	/// A `$pageview` and nothing else: one page, no other event.
	pub is_bounce: bool,
	/// How the visit arrived, from the referrer and the UTM parameters:
	/// `Direct`, `Organic Search`, `Paid Search`, `Organic Social`,
	/// `Paid Social`, `Referral`, `Email`, `Affiliate`, `Audio`, `Mobile
	/// Push`, `SMS`, `Video`, or `Unknown` (§8).
	#[db(max_len = 40)]
	pub channel: String,
	/// The entry pageview's referrer's host, without `www.`.
	#[db(max_len = 200)]
	pub referrer_domain: String,
	#[db(max_len = 200)]
	pub utm_source: String,
	#[db(max_len = 200)]
	pub utm_medium: String,
	#[db(max_len = 200)]
	pub utm_campaign: String,
	/// From the session's context: `Desktop`, `Mobile`, or `Tablet`.
	#[db(max_len = 100)]
	pub device_type: String,
	#[db(max_len = 100)]
	pub browser: String,
	#[db(max_len = 100)]
	pub os: String,
}

leptoskit_db::export_schema!();

/// Every migration, embedded for the service to apply when it starts.
pub static MIGRATIONS: &[leptoskit_db::Migration] = leptoskit_db::migrations!("migrations");

/// The same migrations written for Postgres.
#[cfg(feature = "postgres")]
pub static POSTGRES_MIGRATIONS: &[leptoskit_db::Migration] =
	leptoskit_db::migrations!("migrations-postgres");
