//! From what a client sent to rows: validation, clock correction, and
//! contexts. Runs on the request's task, before anything is queued, so a
//! refused entry never costs the writer anything.

use chrono::DateTime;
use chrono::TimeDelta;
use chrono::Utc;
use leptoskit_db::Timestamp;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Map;
use serde_json::Value as Json;
use sha2::Digest as _;
use sha2::Sha256;
use uuid::Uuid;

use crate::wire;
use crate::wire::Body;

/// The longest event name or distinct id, in bytes.
pub const MAX_NAME_BYTES: usize = 200;

/// The largest an event's own properties may be, encoded as JSON.
pub const MAX_PROPERTIES_BYTES: usize = 64 * 1024;

/// How far ahead of the service's clock an event may claim to be, after
/// correction, before it's recorded as having happened when it arrived.
pub const MAX_FUTURE: TimeDelta = TimeDelta::hours(23);

/// Properties that describe the device and the library rather than the
/// event. A JSON client sends them with every event; the service moves them
/// into the event's context, as the binary format's clients do themselves.
pub const CONTEXT_PROPERTIES: &[&str] = &[
	"$lib",
	"$lib_version",
	"$os",
	"$os_version",
	"$browser",
	"$browser_version",
	"$device_type",
	"$screen_width",
	"$screen_height",
	"$viewport_width",
	"$viewport_height",
	"$locale",
	"$timezone",
	"$app_version",
	"$lk_release",
	"$device_id",
];

/// Which kind of client sent an event, from how it arrived, never from
/// anything the client says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i16)]
pub enum Source {
	/// A browser or a native app, through `v1/batch`.
	Client = 1,
	/// The app's server, signed with the service's token.
	Server = 2,
	/// JSON through the `capture` operation.
	Http = 3,
}

/// One row for the `events` table.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
	pub uuid: Uuid,
	pub name: String,
	pub distinct_id: String,
	pub timestamp: Timestamp,
	pub session_id: Option<Uuid>,
	pub context_id: i64,
	/// A JSON object.
	pub properties: String,
	pub source: Source,
}

/// One row for the `contexts` table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextRow {
	pub id: i64,
	/// A JSON object, with its keys sorted, so the same context always has
	/// the same id.
	pub properties: String,
}

/// An entry the service refused, by its place in the batch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Refused {
	pub index: u32,
	/// A short code: `invalid_uuid`, `invalid_name`, `invalid_distinct_id`,
	/// `properties_too_large`.
	pub reason: String,
}

/// What a batch became.
#[derive(Debug, Default)]
pub struct Accepted {
	pub rows: Vec<Row>,
	pub contexts: Vec<ContextRow>,
	pub refused: Vec<Refused>,
	/// Entries recorded with their time changed, because the client's
	/// clock was too far ahead.
	pub clamped: u32,
}

impl Accepted {
	/// Roughly how much memory the rows hold, for the queue's byte limit.
	pub fn bytes(&self) -> usize {
		self.rows
			.iter()
			.map(|row| 96 + row.name.len() + row.distinct_id.len() + row.properties.len())
			.sum::<usize>()
			+ self
				.contexts
				.iter()
				.map(|context| 16 + context.properties.len())
				.sum::<usize>()
	}
}

/// Accepts a binary batch received at `received_at`.
pub fn batch(batch: wire::Batch, received_at: Timestamp, source: Source) -> Accepted {
	let mut accepted = Accepted::default();
	let contexts: Vec<ContextRow> = batch
		.contexts
		.into_iter()
		.map(|context| {
			context_row(
				context
					.into_iter()
					.map(|(name, value)| (name, json(value)))
					.collect(),
			)
		})
		.collect();
	let empty = context_row(Vec::new());
	let skew = clock_skew(batch.sent_at, received_at);
	let mut used = vec![false; contexts.len()];
	let mut used_empty = false;

	for (index, entry) in batch.entries.into_iter().enumerate() {
		let index = u32::try_from(index).unwrap_or(u32::MAX);
		let Some(millis) = wire::uuid_v7_millis(&entry.uuid) else {
			accepted.refused.push(refused(index, "invalid_uuid"));
			continue;
		};
		let context_id = if let Some(context) = contexts.get(entry.context as usize) {
			used[entry.context as usize] = true;
			context.id
		} else {
			used_empty = true;
			empty.id
		};
		let (name, properties) = body(entry.body);
		let timestamp = corrected(millis, skew, received_at, &mut accepted.clamped);
		match row(
			Uuid::from_bytes(entry.uuid),
			name,
			entry.distinct_id,
			timestamp,
			entry.session_id.map(Uuid::from_bytes),
			context_id,
			&properties,
			source,
		) {
			Ok(row) => accepted.rows.push(row),
			Err(reason) => accepted.refused.push(refused(index, reason)),
		}
	}

	accepted.contexts = contexts
		.into_iter()
		.zip(used)
		.filter_map(|(context, used)| used.then_some(context))
		.collect();
	if used_empty {
		accepted.contexts.push(empty);
	}
	accepted
}

/// A batch as JSON, for clients without the binary format.
#[derive(Clone, Debug, Default, Serialize, Deserialize, schemars::JsonSchema)]
pub struct JsonBatch {
	/// The client's clock when it sent the batch, in RFC 3339, to correct
	/// each event's time with.
	#[serde(default)]
	pub sent_at: Option<String>,
	pub batch: Vec<JsonEvent>,
}

/// One event as JSON. Identity changes are events with reserved names:
/// `$identify` (with `$anon_distinct_id`), `$create_alias` (with `alias`),
/// `$set`, and `$groupidentify`, with `$set`, `$set_once`, and `$unset` in
/// their properties.
#[derive(Clone, Debug, Default, Serialize, Deserialize, schemars::JsonSchema)]
pub struct JsonEvent {
	pub event: String,
	pub distinct_id: String,
	/// A UUID v7 made when the event happened, so a retry is stored once.
	/// The service makes one when it's missing, and then can't recognise a
	/// retry.
	#[serde(default)]
	pub uuid: Option<Uuid>,
	/// When it happened, in RFC 3339. Defaults to the UUID's time, then to
	/// when the service received it.
	#[serde(default)]
	pub timestamp: Option<String>,
	#[serde(default)]
	pub session_id: Option<Uuid>,
	#[serde(default)]
	pub properties: Map<String, Json>,
}

/// Accepts a JSON batch received at `received_at`.
pub fn json_batch(batch: JsonBatch, received_at: Timestamp, source: Source) -> Accepted {
	let mut accepted = Accepted::default();
	let sent_at = batch
		.sent_at
		.as_deref()
		.and_then(|sent_at| DateTime::parse_from_rfc3339(sent_at).ok())
		.map(|sent_at| sent_at.timestamp_millis());
	let skew = sent_at.map_or(TimeDelta::zero(), |sent_at| {
		clock_skew(sent_at, received_at)
	});

	for (index, event) in batch.batch.into_iter().enumerate() {
		let index = u32::try_from(index).unwrap_or(u32::MAX);
		let given = event
			.timestamp
			.as_deref()
			.map(DateTime::parse_from_rfc3339)
			.transpose();
		let Ok(given) = given else {
			accepted.refused.push(refused(index, "invalid_timestamp"));
			continue;
		};
		let (uuid, millis) = if let Some(uuid) = event.uuid {
			let Some(millis) = wire::uuid_v7_millis(uuid.as_bytes()) else {
				accepted.refused.push(refused(index, "invalid_uuid"));
				continue;
			};
			(uuid, given.map_or(millis, |given| given.timestamp_millis()))
		} else {
			let millis = given.map_or(received_at.timestamp_millis(), |given| {
				given.timestamp_millis()
			});
			(
				Uuid::from_bytes(wire::uuid_v7(millis, random_tail())),
				millis,
			)
		};

		let mut properties = event.properties;
		let context: Vec<(String, Json)> = CONTEXT_PROPERTIES
			.iter()
			.filter_map(|name| {
				properties
					.remove(*name)
					.map(|value| ((*name).to_owned(), value))
			})
			.collect();
		let context = context_row(context);
		let timestamp = corrected(millis, skew, received_at, &mut accepted.clamped);
		let properties = Json::Object(properties);
		match row(
			uuid,
			event.event,
			event.distinct_id,
			timestamp,
			event.session_id,
			context.id,
			&properties,
			source,
		) {
			Ok(row) => {
				accepted.rows.push(row);
				if !accepted.contexts.contains(&context) {
					accepted.contexts.push(context);
				}
			}
			Err(reason) => accepted.refused.push(refused(index, reason)),
		}
	}

	accepted
}

#[allow(clippy::too_many_arguments)]
fn row(
	uuid: Uuid,
	name: String,
	distinct_id: String,
	timestamp: Timestamp,
	session_id: Option<Uuid>,
	context_id: i64,
	properties: &Json,
	source: Source,
) -> Result<Row, &'static str> {
	if !valid_name(&name) {
		return Err("invalid_name");
	}
	if !valid_name(&distinct_id) {
		return Err("invalid_distinct_id");
	}
	let properties = serde_json::to_string(properties).map_err(|_| "invalid_properties")?;
	if properties.len() > MAX_PROPERTIES_BYTES {
		return Err("properties_too_large");
	}
	Ok(Row {
		uuid,
		name,
		distinct_id,
		timestamp,
		session_id,
		context_id,
		properties,
		source,
	})
}

/// 1 to [`MAX_NAME_BYTES`] bytes, with no control characters.
fn valid_name(name: &str) -> bool {
	!name.trim().is_empty() && name.len() <= MAX_NAME_BYTES && !name.chars().any(char::is_control)
}

fn refused(index: u32, reason: &str) -> Refused {
	Refused {
		index,
		reason: reason.to_owned(),
	}
}

/// How far the client's clock is ahead of the service's: `sent_at` was
/// read from the client's clock as it sent, and arrived at `received_at`.
fn clock_skew(sent_at: i64, received_at: Timestamp) -> TimeDelta {
	if sent_at <= 0 {
		return TimeDelta::zero();
	}
	TimeDelta::milliseconds(sent_at.saturating_sub(received_at.timestamp_millis()))
}

/// The client's time `millis`, corrected for `skew`, and clamped to
/// `received_at` when it's still more than [`MAX_FUTURE`] ahead.
fn corrected(millis: i64, skew: TimeDelta, received_at: Timestamp, clamped: &mut u32) -> Timestamp {
	let timestamp = DateTime::<Utc>::from_timestamp_millis(millis).map(|time| time - skew);
	match timestamp {
		Some(timestamp) if timestamp <= received_at + MAX_FUTURE => timestamp,
		_ => {
			*clamped += 1;
			received_at
		}
	}
}

/// An identity change, written as the event `PostHog`'s model has for it.
fn body(body: Body) -> (String, Json) {
	let mut properties = Map::new();
	let name = match body {
		Body::Event {
			name,
			properties: given,
		} => {
			return (name, object(given));
		}
		Body::Identify {
			anon_distinct_id,
			set,
			set_once,
		} => {
			properties.insert("$anon_distinct_id".into(), Json::String(anon_distinct_id));
			put_object(&mut properties, "$set", set);
			put_object(&mut properties, "$set_once", set_once);
			"$identify"
		}
		Body::Alias { alias } => {
			properties.insert("alias".into(), Json::String(alias));
			"$create_alias"
		}
		Body::Set {
			set,
			set_once,
			unset,
		} => {
			put_object(&mut properties, "$set", set);
			put_object(&mut properties, "$set_once", set_once);
			if !unset.is_empty() {
				properties.insert(
					"$unset".into(),
					Json::Array(unset.into_iter().map(Json::String).collect()),
				);
			}
			"$set"
		}
		Body::GroupIdentify {
			group_type,
			group_key,
			set,
		} => {
			properties.insert("$group_type".into(), Json::String(group_type));
			properties.insert("$group_key".into(), Json::String(group_key));
			put_object(&mut properties, "$group_set", set);
			"$groupidentify"
		}
	};
	(name.to_owned(), Json::Object(properties))
}

fn put_object(properties: &mut Map<String, Json>, name: &str, values: wire::Properties) {
	if !values.is_empty() {
		properties.insert(name.into(), object(values));
	}
}

fn object(values: wire::Properties) -> Json {
	Json::Object(
		values
			.into_iter()
			.map(|(name, value)| (name, json(value)))
			.collect(),
	)
}

/// A wire value as JSON: an instant becomes RFC 3339 text, and a float JSON
/// can't hold becomes `null`.
fn json(value: wire::Value) -> Json {
	match value {
		wire::Value::Null => Json::Null,
		wire::Value::Bool(value) => Json::Bool(value),
		wire::Value::Int(value) => Json::from(value),
		wire::Value::Float(value) => {
			serde_json::Number::from_f64(value).map_or(Json::Null, Json::Number)
		}
		wire::Value::Text(value) => Json::String(value),
		wire::Value::Time(millis) => {
			DateTime::<Utc>::from_timestamp_millis(millis).map_or(Json::Null, |time| {
				Json::String(time.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
			})
		}
		wire::Value::List(items) => Json::Array(items.into_iter().map(json).collect()),
		wire::Value::Map(map) => object(map),
	}
}

/// A context's row: its properties sorted by name, and an id from their
/// hash, so every replica gives the same context the same id without
/// asking the database.
fn context_row(mut properties: Vec<(String, Json)>) -> ContextRow {
	properties.sort_by(|a, b| a.0.cmp(&b.0));
	properties.dedup_by(|a, b| a.0 == b.0);
	let properties = Json::Object(properties.into_iter().collect()).to_string();
	let digest = Sha256::digest(properties.as_bytes());
	let id = i64::from_be_bytes(digest[..8].try_into().expect("8 bytes"));
	ContextRow { id, properties }
}

fn random_tail() -> [u8; 10] {
	let random = Uuid::new_v4().into_bytes();
	random[..10].try_into().expect("10 bytes")
}

#[cfg(test)]
mod tests {
	use super::*;

	fn at(millis: i64) -> Timestamp {
		DateTime::<Utc>::from_timestamp_millis(millis).unwrap()
	}

	#[test]
	fn a_fast_clock_is_corrected_by_how_far_ahead_it_was_when_sending() {
		// The phone's clock is an hour ahead: it recorded the event at 13:00
		// by its clock and sent it at 13:05, which was 12:05 here.
		let hour = 3_600_000;
		let received = at(12 * hour + 5 * 60_000);
		let skew = clock_skew(13 * hour + 5 * 60_000, received);
		let mut clamped = 0;
		assert_eq!(
			corrected(13 * hour, skew, received, &mut clamped),
			at(12 * hour)
		);
		assert_eq!(clamped, 0);
	}

	#[test]
	fn an_event_still_far_in_the_future_is_recorded_when_it_arrived() {
		let received = at(1_000_000_000_000);
		let mut clamped = 0;
		let future = received.timestamp_millis() + MAX_FUTURE.num_milliseconds() + 1;
		assert_eq!(
			corrected(future, TimeDelta::zero(), received, &mut clamped),
			received
		);
		assert_eq!(clamped, 1);
	}

	#[test]
	fn the_same_context_in_any_order_has_one_id() {
		let a = context_row(vec![
			("$os".into(), Json::from("iOS")),
			("$lib".into(), Json::from("lk")),
		]);
		let b = context_row(vec![
			("$lib".into(), Json::from("lk")),
			("$os".into(), Json::from("iOS")),
		]);
		assert_eq!(a, b);
		assert_eq!(a.properties, r#"{"$lib":"lk","$os":"iOS"}"#);
	}

	#[test]
	fn names_are_bounded_and_printable() {
		assert!(valid_name("signed_up"));
		assert!(!valid_name(""));
		assert!(!valid_name("  "));
		assert!(!valid_name("a\nb"));
		assert!(!valid_name(&"x".repeat(MAX_NAME_BYTES + 1)));
	}
}
