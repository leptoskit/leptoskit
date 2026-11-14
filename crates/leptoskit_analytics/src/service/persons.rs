//! Person processing, run by the batch writer for each batch (§8 of the
//! design).
//!
//! Every newly stored event's `distinct_id` resolves to a person: an id the
//! service hasn't seen creates one when the setting allows, and the
//! identity entries apply their changes. `$set`, `$set_once`, and `$unset`
//! update properties last-write-wins **by event time per property**, so an
//! offline client's late batch converges instead of overwriting newer
//! values, which `PostHog` gets wrong by applying them in arrival order.
//! `$identify` with `$anon_distinct_id` moves the anonymous person's
//! distinct ids onto the identified person once and records an override
//! for each moved id, so queries apply the merge to past events without
//! rewriting them. Merging two identified persons is refused and counted
//! as a `merge_refused` ingestion warning, as `PostHog`'s merge restrictions
//! are. `$create_alias` follows the same rules.
//!
//! Duplicates never reach this module: the writer processes the rows a
//! batch newly inserted only, so a retried batch applies nothing twice.
//! On Postgres, merges take an advisory lock per person, so two replicas
//! processing the same id serialise rather than race.

use std::collections::HashMap;
use std::collections::HashSet;

use chrono::SecondsFormat;
use leptoskit_db::Connection as _;
use leptoskit_db::Dialect;
use leptoskit_db::Read as _;
use leptoskit_db::Statement;
use leptoskit_db::Timestamp;
use leptoskit_db::Tx;
use leptoskit_db::Value;
use serde_json::Value as Json;
use uuid::Uuid;

use super::accept::Row;
use super::ingest::Insert;
use super::ingest::Warning;

/// Who gets a person profile. `PostHog`'s `person_profiles`, kept as a
/// service setting: the writer runs it for every batch alike.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PersonProfiles {
	/// Only identified ids and events that set person properties get a
	/// person; other anonymous events keep `person_id` null and still
	/// count by `distinct_id`. The default, as `PostHog`'s is.
	#[default]
	IdentifiedOnly,
	/// Every event's distinct id gets a person, as before anonymous
	/// profiles were made opt-in.
	All,
	/// Nobody: `person_id` stays null everywhere, and identity entries
	/// store as events only.
	Never,
}

impl PersonProfiles {
	/// The names `LEPTOSKIT_ANALYTICS_PERSON_PROFILES` and the
	/// configuration spell it with.
	pub fn parse(value: &str) -> Option<Self> {
		match value.trim().to_ascii_lowercase().as_str() {
			"identified_only" | "identifiedonly" | "identified" => Some(Self::IdentifiedOnly),
			"all" => Some(Self::All),
			"never" | "none" | "false" => Some(Self::Never),
			_ => None,
		}
	}
}

/// Distinct ids that mean "nobody in particular": generic placeholders
/// never create a person and never merge one, however they arrive.
#[must_use]
pub fn is_placeholder(distinct_id: &str) -> bool {
	matches!(
		distinct_id.trim(),
		"" | "0"
			| "anonymous"
			| "distinct_id"
			| "guest"
			| "not_authenticated"
			| "null"
			| "undefined"
			| "unknown"
	)
}

/// What a batch's person processing came to.
pub(crate) struct BatchPersons {
	/// The person of each processed row, in order.
	pub person_ids: Vec<Option<Uuid>>,
	/// Warnings to record, such as refused merges.
	pub warnings: Vec<Warning>,
}

/// Processes `rows` (the entries a batch newly inserted, in arrival
/// order) against the persons tables inside `tx`.
pub(crate) async fn process(
	tx: &Tx,
	dialect: Dialect,
	rows: &[&Row],
	profiles: PersonProfiles,
	now: Timestamp,
) -> Result<BatchPersons, String> {
	let mut persons = Persons::new(profiles, now);
	// Every id the batch may resolve, including the anonymous ids and
	// aliases its identity entries name: a batch with only `$identify`
	// still needs the anonymous id's person to merge it.
	let mut needed: Vec<String> = Vec::new();
	for row in rows {
		needed.push(row.distinct_id.clone());
		if let Some(properties) = identity_properties(row) {
			for key in ["$anon_distinct_id", "alias"] {
				if let Some(id) = properties.get(key).and_then(Json::as_str) {
					needed.push(id.to_owned());
				}
			}
		}
	}
	needed.sort();
	needed.dedup();
	persons.load(tx, dialect, &needed).await?;

	let mut person_ids = Vec::with_capacity(rows.len());
	for row in rows {
		let properties = identity_properties(row);
		let person = match row.name.as_str() {
			"$identify" => persons.identify(row, properties.as_ref()),
			"$create_alias" => persons.alias(row, properties.as_ref()),
			_ => {
				persons.resolve(
					&row.distinct_id,
					creates_person(profiles, properties.as_ref()),
				)
			}
		};
		if let Some(person) = person {
			persons.apply_changes(person, properties.as_ref(), row.timestamp);
		}
		person_ids.push(person);
	}

	persons.write_back(tx, dialect).await?;
	Ok(BatchPersons {
		person_ids,
		warnings: persons.warnings,
	})
}

/// Whether an entry with `properties` creates a person for an id the
/// service hasn't seen: the identity entries do, and everything does under
/// [`PersonProfiles::All`].
fn creates_person(profiles: PersonProfiles, properties: Option<&Json>) -> bool {
	match profiles {
		PersonProfiles::Never => false,
		PersonProfiles::All => true,
		PersonProfiles::IdentifiedOnly => properties.is_some(),
	}
}

/// The properties of an identity entry, parsed once per row: only those
/// entries carry changes, so the rest of a batch pays no parsing.
fn identity_properties(row: &Row) -> Option<Json> {
	match row.name.as_str() {
		"$identify" | "$create_alias" | "$set" => serde_json::from_str(&row.properties).ok(),
		_ => None,
	}
}

/// A person being processed, in memory until the batch's write-back.
struct PersonState {
	id: Uuid,
	properties: Json,
	updated_at: HashMap<String, String>,
	created_at: Timestamp,
	is_identified: bool,
	version: i64,
	/// Whether the row must be written: it's new, or something changed.
	changed: bool,
}

impl PersonState {
	fn new(id: Uuid, now: Timestamp) -> Self {
		Self {
			id,
			properties: Json::Object(serde_json::Map::new()),
			updated_at: HashMap::new(),
			created_at: now,
			is_identified: false,
			version: 1,
			changed: true,
		}
	}
}

/// One batch's persons: what the tables hold for the ids it touches, plus
/// what the batch changed, in memory until the write-back.
struct Persons {
	mappings: HashMap<String, Uuid>,
	/// The distinct ids [`Self::mappings`] was loaded with, so the
	/// write-back inserts only the new ones.
	loaded: HashSet<String>,
	states: HashMap<Uuid, PersonState>,
	profiles: PersonProfiles,
	now: Timestamp,
	/// Merges, in order: the absorbed person, then the survivor.
	moved: Vec<(Uuid, Uuid)>,
	warnings: Vec<Warning>,
}

impl Persons {
	fn new(profiles: PersonProfiles, now: Timestamp) -> Self {
		Self {
			mappings: HashMap::new(),
			loaded: HashSet::new(),
			states: HashMap::new(),
			profiles,
			now,
			moved: Vec::new(),
			warnings: Vec::new(),
		}
	}

	/// Reads the distinct ids' mappings and their persons' rows.
	async fn load(&mut self, tx: &Tx, dialect: Dialect, ids: &[String]) -> Result<(), String> {
		if self.profiles == PersonProfiles::Never || ids.is_empty() {
			return Ok(());
		}
		let mut wanted: Vec<&str> = ids
			.iter()
			.map(String::as_str)
			.filter(|id| !is_placeholder(id) && !self.loaded.contains(*id))
			.collect();
		wanted.sort_unstable();
		wanted.dedup();
		for chunk in wanted.chunks(super::ingest::ROWS_PER_STATEMENT) {
			let sql = format!(
				"SELECT \"distinct_id\", \"person_id\" FROM \"person_distinct_ids\" WHERE \
				 \"distinct_id\" IN ({})",
				placeholders(dialect, chunk.len())
			);
			let rows = tx
				.fetch(Statement {
					sql,
					args: chunk
						.iter()
						.map(|id| Value::Text((*id).to_owned()))
						.collect(),
				})
				.await
				.map_err(|error| error.to_string())?;
			for pair in rows {
				let (Some(Value::Text(distinct_id)), Some(id)) = (pair.first(), pair.get(1)) else {
					continue;
				};
				let Some(person) = uuid_of(id) else {
					continue;
				};
				self.loaded.insert(distinct_id.clone());
				self.mappings.insert(distinct_id.clone(), person);
			}
		}
		let mut wanted: Vec<Uuid> = self.mappings.values().copied().collect();
		wanted.sort();
		wanted.dedup();
		for chunk in wanted.chunks(super::ingest::ROWS_PER_STATEMENT) {
			let sql = format!(
				"SELECT \"id\", \"properties\", \"properties_updated_at\", \"created_at\", \
				 \"is_identified\", \"version\" FROM \"persons\" WHERE \"id\" IN ({})",
				placeholders(dialect, chunk.len()),
			);
			let rows = tx
				.fetch(Statement {
					sql,
					args: chunk.iter().map(|id| Value::Uuid(*id)).collect(),
				})
				.await
				.map_err(|error| error.to_string())?;
			for row in rows {
				let (Some(id), Some(properties), Some(updated_at), Some(created_at)) =
					(row.first(), row.get(1), row.get(2), row.get(3))
				else {
					continue;
				};
				let Some(id) = uuid_of(id) else {
					continue;
				};
				let Some(created_at) = timestamp_of(created_at) else {
					continue;
				};
				let (Value::Text(properties), Value::Text(updated_at)) = (properties, updated_at)
				else {
					continue;
				};
				let is_identified = matches!(row.get(4), Some(Value::Bool(true) | Value::Int(1)));
				let version = match row.get(5) {
					Some(Value::Int(version)) => *version,
					_ => 1,
				};
				self.states.insert(
					id,
					PersonState {
						id,
						properties: serde_json::from_str(properties)
							.unwrap_or(Json::Object(serde_json::Map::default())),
						updated_at: serde_json::from_str(updated_at).unwrap_or_default(),
						created_at,
						is_identified,
						version,
						changed: false,
					},
				);
			}
		}
		Ok(())
	}

	/// The person `distinct_id` belongs to, creating one when `create` says
	/// so. Placeholders never get one.
	fn resolve(&mut self, distinct_id: &str, create: bool) -> Option<Uuid> {
		if self.profiles == PersonProfiles::Never || is_placeholder(distinct_id) {
			return None;
		}
		if let Some(person) = self.mappings.get(distinct_id) {
			return Some(*person);
		}
		if !create {
			return None;
		}
		let person = Uuid::new_v4();
		self.states
			.insert(person, PersonState::new(person, self.now));
		self.mappings.insert(distinct_id.to_owned(), person);
		Some(person)
	}

	/// `$identify`: the row's id is now the person's known id, marked
	/// identified, and the anonymous id's person merges into it.
	fn identify(&mut self, row: &Row, properties: Option<&Json>) -> Option<Uuid> {
		let person = self.resolve(&row.distinct_id, true)?;
		self.mark_identified(person);
		let anon = properties
			.as_ref()
			.and_then(|properties| properties.get("$anon_distinct_id").and_then(Json::as_str));
		if let Some(anon) = anon
			&& anon != row.distinct_id
		{
			self.merge(anon, person, "$identify");
		}
		Some(person)
	}

	/// `$create_alias`: `alias` becomes another id of the same person,
	/// under the same merge rules as [`Self::identify`].
	fn alias(&mut self, row: &Row, properties: Option<&Json>) -> Option<Uuid> {
		let person = self.resolve(&row.distinct_id, true)?;
		let alias = properties
			.as_ref()
			.and_then(|properties| properties.get("alias"))
			.and_then(Json::as_str);
		if let Some(alias) = alias
			&& alias != row.distinct_id
		{
			self.merge(alias, person, "$create_alias");
		}
		Some(person)
	}

	/// Merges the person of `from_id` into `to`, once: their distinct ids
	/// move, an override is recorded for each, and their properties join
	/// the survivor's by their write times. Two identified persons refuse.
	fn merge(&mut self, from_id: &str, to: Uuid, entry: &str) {
		if is_placeholder(from_id) {
			return;
		}
		let Some(from) = self.mappings.get(from_id).copied() else {
			// The anonymous id has no person (perhaps nothing was stored
			// under it): record it as another id of the person, so later
			// events under it attach. There's nothing to move: its events,
			// if any, have no person.
			self.mappings.insert(from_id.to_owned(), to);
			return;
		};
		if from == to {
			return;
		}
		let both_identified = self
			.states
			.get(&from)
			.is_some_and(|state| state.is_identified)
			&& self
				.states
				.get(&to)
				.is_some_and(|state| state.is_identified);
		if both_identified {
			self.warnings.push(Warning {
				reason: "merge_refused".into(),
				count: 1,
				detail: format!("{entry} would merge two identified persons: {from_id}"),
			});
			return;
		}
		let from_state = self.states.remove(&from);
		let to_state = self.states.get_mut(&to);
		if let (Some(from_state), Some(to_state)) = (from_state, to_state) {
			for (key, value) in object(&from_state.properties) {
				let from_seen = from_state.updated_at.get(&key);
				let to_seen = to_state.updated_at.get(&key);
				if to_seen.is_none()
					|| from_seen
						.is_some_and(|from| from.as_str() > to_seen.expect("checked").as_str())
				{
					to_state.properties[&key] = value;
					if let Some(seen) = from_seen {
						to_state.updated_at.insert(key.clone(), seen.clone());
					}
				}
			}
			to_state.changed = true;
		}
		// Anything pointing at the absorbed person points at the survivor
		// now, including ids this batch resolved earlier.
		for person in self.mappings.values_mut() {
			if *person == from {
				*person = to;
			}
		}
		self.states.remove(&from);
		self.moved.push((from, to));
	}

	fn mark_identified(&mut self, person: Uuid) {
		if let Some(state) = self.states.get_mut(&person)
			&& !state.is_identified
		{
			state.is_identified = true;
			state.changed = true;
		}
	}

	/// Applies an identity entry's `$set`, `$set_once`, and `$unset` to
	/// `person`, each property last written when this entry is newer.
	fn apply_changes(&mut self, person: Uuid, properties: Option<&Json>, timestamp: Timestamp) {
		let Some(properties) = properties else {
			return;
		};
		let Some(state) = self.states.get_mut(&person) else {
			return;
		};
		let when = stamp(timestamp);
		for (key, value) in object_of(properties, "$set") {
			if newer(&state.updated_at, &key, &when) {
				state.properties[&key] = value;
				state.updated_at.insert(key, when.clone());
				state.changed = true;
			}
		}
		for (key, value) in object_of(properties, "$set_once") {
			if !state
				.properties
				.as_object()
				.is_some_and(|object| object.contains_key(&key))
			{
				state.properties[key.as_str()] = value;
				state.updated_at.insert(key, when.clone());
				state.changed = true;
			}
		}
		for key in properties
			.get("$unset")
			.and_then(Json::as_array)
			.into_iter()
			.flatten()
			.filter_map(Json::as_str)
		{
			if newer(&state.updated_at, key, &when) {
				if let Some(object) = state.properties.as_object_mut() {
					object.remove(key);
				}
				state.updated_at.remove(key);
				state.changed = true;
			}
		}
	}

	/// Writes everything the batch changed, inside the writer's
	/// transaction.
	async fn write_back(&mut self, tx: &Tx, dialect: Dialect) -> Result<(), String> {
		self.lock_merges(tx, dialect).await?;
		let overrides = self.overrides_of_moved(tx, dialect).await?;
		self.upsert_persons(tx, dialect).await?;
		self.delete_absorbed(tx, dialect).await?;
		self.write_overrides(tx, dialect, overrides).await?;
		self.move_distinct_ids(tx, dialect).await?;
		self.insert_new_mappings(tx, dialect).await?;
		Ok(())
	}

	/// Postgres: one advisory lock per merged person, taken in a sorted
	/// order so two replicas merging the same pair can't deadlock.
	async fn lock_merges(&self, tx: &Tx, dialect: Dialect) -> Result<(), String> {
		if dialect != Dialect::Postgres {
			return Ok(());
		}
		let mut locks: Vec<Uuid> = self
			.moved
			.iter()
			.flat_map(|(from, to)| [*from, *to])
			.collect();
		locks.sort();
		locks.dedup();
		for id in locks {
			tx.execute(Statement {
				sql: "SELECT pg_advisory_xact_lock($1)".into(),
				args: vec![Value::Int(i64::from_be_bytes(
					id.as_bytes()[..8].try_into().expect("8 bytes"),
				))],
			})
			.await
			.map_err(|error| error.to_string())?;
		}
		Ok(())
	}

	/// Which distinct ids each merge moves, read before the move: the rows
	/// still say the old person.
	async fn overrides_of_moved(
		&self,
		tx: &Tx,
		dialect: Dialect,
	) -> Result<Vec<(String, Uuid)>, String> {
		let mut overrides: Vec<(String, Uuid)> = Vec::new();
		for (from, to) in &self.moved {
			let rows = tx
				.fetch(Statement {
					sql: format!(
						"SELECT \"distinct_id\" FROM \"person_distinct_ids\" WHERE \"person_id\" \
						 = {}",
						placeholder(dialect, 1)
					),
					args: vec![Value::Uuid(*from)],
				})
				.await
				.map_err(|error| error.to_string())?;
			for row in rows {
				if let Some(Value::Text(distinct_id)) = row.first() {
					overrides.push((distinct_id.clone(), *to));
				}
			}
		}
		Ok(overrides)
	}

	/// Writes the new and changed person rows.
	async fn upsert_persons(&self, tx: &Tx, dialect: Dialect) -> Result<(), String> {
		let mut living: Vec<&PersonState> =
			self.states.values().filter(|state| state.changed).collect();
		living.sort_by_key(|state| state.id);
		for chunk in living.chunks(super::ingest::ROWS_PER_STATEMENT) {
			let mut insert = Insert::new(
				dialect,
				"persons",
				&[
					"id",
					"properties",
					"properties_updated_at",
					"created_at",
					"is_identified",
					"version",
				],
			);
			for state in chunk {
				insert.row(vec![
					Value::Uuid(state.id),
					Value::Text(state.properties.to_string()),
					Value::Text(serde_json::to_string(&state.updated_at).unwrap_or_default()),
					Value::Timestamp(state.created_at),
					Value::Bool(state.is_identified),
					Value::Int(state.version.max(1)),
				]);
			}
			let mut statement = insert.finish();
			statement.sql.push_str(
				" ON CONFLICT (\"id\") DO UPDATE SET \"properties\" = excluded.\"properties\", \
				 \"properties_updated_at\" = excluded.\"properties_updated_at\", \
				 \"is_identified\" = excluded.\"is_identified\", \"version\" = \
				 \"persons\".\"version\" + 1",
			);
			tx.execute(statement)
				.await
				.map_err(|error| error.to_string())?;
		}
		Ok(())
	}

	/// Removes the persons a merge absorbed.
	async fn delete_absorbed(&self, tx: &Tx, dialect: Dialect) -> Result<(), String> {
		let absorbed: Vec<Value> = self
			.moved
			.iter()
			.map(|(from, _)| Value::Uuid(*from))
			.collect();
		for chunk in absorbed.chunks(super::ingest::ROWS_PER_STATEMENT) {
			tx.execute(Statement {
				sql: format!(
					"DELETE FROM \"persons\" WHERE \"id\" IN ({})",
					placeholders(dialect, chunk.len())
				),
				args: chunk.to_vec(),
			})
			.await
			.map_err(|error| error.to_string())?;
		}
		Ok(())
	}

	/// Records, for each moved id, that its events belong to the survivor
	/// now.
	async fn write_overrides(
		&self,
		tx: &Tx,
		dialect: Dialect,
		overrides: Vec<(String, Uuid)>,
	) -> Result<(), String> {
		for chunk in overrides.chunks(super::ingest::ROWS_PER_STATEMENT) {
			let mut insert = Insert::new(
				dialect,
				"person_overrides",
				&["distinct_id", "person_id", "created_at"],
			);
			for (distinct_id, person) in chunk {
				insert.row(vec![
					Value::Text(distinct_id.clone()),
					Value::Uuid(*person),
					Value::Timestamp(self.now),
				]);
			}
			let mut statement = insert.finish();
			statement.sql.push_str(
				" ON CONFLICT (\"distinct_id\") DO UPDATE SET \"person_id\" = \
				 excluded.\"person_id\"",
			);
			tx.execute(statement)
				.await
				.map_err(|error| error.to_string())?;
		}
		Ok(())
	}

	/// Points every stored distinct id of an absorbed person at its
	/// survivor.
	async fn move_distinct_ids(&self, tx: &Tx, dialect: Dialect) -> Result<(), String> {
		for (from, to) in &self.moved {
			tx.execute(Statement {
				sql: format!(
					"UPDATE \"person_distinct_ids\" SET \"person_id\" = {} WHERE \"person_id\" = \
					 {}",
					placeholder(dialect, 1),
					placeholder(dialect, 2)
				),
				args: vec![Value::Uuid(*to), Value::Uuid(*from)],
			})
			.await
			.map_err(|error| error.to_string())?;
		}
		Ok(())
	}

	/// Records the batch's new id-to-person mappings.
	async fn insert_new_mappings(&self, tx: &Tx, dialect: Dialect) -> Result<(), String> {
		let mut new_ids: Vec<(&String, &Uuid)> = self
			.mappings
			.iter()
			.filter(|(distinct_id, _)| !self.loaded.contains(*distinct_id))
			.collect();
		new_ids.sort_by(|a, b| a.0.cmp(b.0));
		for chunk in new_ids.chunks(super::ingest::ROWS_PER_STATEMENT) {
			let mut insert = Insert::new(
				dialect,
				"person_distinct_ids",
				&["distinct_id", "person_id", "created_at"],
			);
			for (distinct_id, person) in chunk {
				insert.row(vec![
					Value::Text((*distinct_id).clone()),
					Value::Uuid(**person),
					Value::Timestamp(self.now),
				]);
			}
			tx.execute(insert.on_conflict_do_nothing("distinct_id"))
				.await
				.map_err(|error| error.to_string())?;
		}
		Ok(())
	}
}

/// An instant from either dialect: a `Timestamp` bound as written, or the
/// text SQLite returns, in RFC 3339.
pub(crate) fn timestamp_of(value: &Value) -> Option<Timestamp> {
	match value {
		Value::Timestamp(time) => Some(*time),
		Value::Text(text) => {
			chrono::DateTime::parse_from_rfc3339(text)
				.ok()
				.map(|time| time.to_utc())
		}
		_ => None,
	}
}

/// Whether a property's last write, as `updated_at` holds it, is older
/// than this entry: a missing write is always older, and equal times keep
/// what's stored, so a retried identity entry changes nothing.
fn newer(updated_at: &HashMap<String, String>, key: &str, when: &str) -> bool {
	updated_at.get(key).is_none_or(|seen| seen.as_str() < when)
}

/// An instant written so text order is time order.
pub(crate) fn stamp(time: Timestamp) -> String {
	time.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn object(properties: &Json) -> serde_json::Map<String, Json> {
	properties.as_object().cloned().unwrap_or_default()
}

/// The object under `name`, when it is one.
fn object_of(properties: &Json, name: &str) -> Vec<(String, Json)> {
	properties
		.get(name)
		.and_then(Json::as_object)
		.map(|object| {
			object
				.iter()
				.map(|(key, value)| (key.clone(), value.clone()))
				.collect()
		})
		.unwrap_or_default()
}

/// A `Value` that holds a UUID, as either dialect returns one.
pub(crate) fn uuid_of(value: &Value) -> Option<Uuid> {
	match value {
		Value::Uuid(uuid) => Some(*uuid),
		Value::Bytes(bytes) => Uuid::from_slice(bytes).ok(),
		_ => None,
	}
}

/// The `n`th placeholder for the dialect.
pub(crate) fn placeholder(dialect: Dialect, number: usize) -> String {
	match dialect {
		Dialect::Postgres => format!("${number}"),
		Dialect::Sqlite => format!("?{number}"),
	}
}

/// `count` placeholders, comma-separated.
pub(crate) fn placeholders(dialect: Dialect, count: usize) -> String {
	(1..=count)
		.map(|number| placeholder(dialect, number))
		.collect::<Vec<_>>()
		.join(", ")
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn placeholders_never_get_a_person() {
		for id in [
			"",
			"0",
			"anonymous",
			"guest",
			"null",
			"undefined",
			"  ",
			"not_authenticated",
		] {
			assert!(is_placeholder(id), "{id:?} is a placeholder");
		}
		for id in [
			"user-1",
			"email@example.com",
			"019a5f3c-0000-7000-8000-0000000000ff",
		] {
			assert!(!is_placeholder(id), "{id:?} isn't");
		}
	}

	#[test]
	fn the_setting_parses_its_names() {
		assert_eq!(
			PersonProfiles::parse("identified_only"),
			Some(PersonProfiles::IdentifiedOnly)
		);
		assert_eq!(PersonProfiles::parse("all"), Some(PersonProfiles::All));
		assert_eq!(PersonProfiles::parse("never"), Some(PersonProfiles::Never));
		assert_eq!(PersonProfiles::parse("sometimes"), None);
		assert_eq!(PersonProfiles::default(), PersonProfiles::IdentifiedOnly);
	}

	#[test]
	fn an_entry_without_properties_creates_a_person_only_when_asked() {
		assert!(!creates_person(PersonProfiles::IdentifiedOnly, None));
		assert!(creates_person(
			PersonProfiles::IdentifiedOnly,
			Some(&Json::Null)
		));
		assert!(creates_person(PersonProfiles::All, None));
		assert!(!creates_person(PersonProfiles::Never, Some(&Json::Null)));
	}
}
