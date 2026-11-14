//! What the app reads and manages: persons and their events, sessions,
//! deletion, and export. Every operation here is app-only, like
//! [`super::register_events`](super::register_events): the app's server
//! calls it with the service's token, so an admin page reads through the
//! app, which decides who may see what (§18).
//!
//! These are reads over what step 2 stores. The insights engine (§10)
//! supersedes them for dashboards; they exist so the app's admin can list,
//! inspect, delete, and export a person today.

use chrono::SecondsFormat;
use leptoskit_db::Connection as _;
use leptoskit_db::Dialect;
use leptoskit_db::RawRead;
use leptoskit_db::Read as _;
use leptoskit_db::Statement;
use leptoskit_db::Value;
use leptoskit_rpc as rpc;
use rpc::server::Identity;
use serde::Deserialize;
use serde::Serialize;
use uuid::Uuid;

use super::persons::placeholder;
use super::persons::timestamp_of;
use super::persons::uuid_of;
use super::tables;

/// The most rows one page answers with.
const MAX_PAGE: u32 = 200;

/// A person, as the list answers with.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PersonSummary {
	pub id: Uuid,
	/// The person's properties, as a JSON object.
	pub properties: serde_json::Value,
	pub created_at: String,
	/// Whether the app identified this person with `$identify`.
	pub is_identified: bool,
	/// Bumped every time the person changes.
	pub version: i64,
}

/// A person with everything belonging to them.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PersonView {
	#[serde(flatten)]
	pub person: PersonSummary,
	/// Every distinct id events were recorded under, including the
	/// anonymous ones a merge moved onto them.
	pub distinct_ids: Vec<String>,
}

/// One stored event.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct EventView {
	pub uuid: Uuid,
	pub name: String,
	pub distinct_id: String,
	pub timestamp: String,
	pub session_id: Option<Uuid>,
	/// The event's own properties, as a JSON object.
	pub properties: serde_json::Value,
}

/// One visit, as the sessions table holds it.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SessionView {
	pub id: Uuid,
	pub distinct_id: String,
	pub person_id: Option<Uuid>,
	pub first_timestamp: String,
	pub last_timestamp: String,
	pub duration_ms: i64,
	pub entry_url: String,
	pub entry_pathname: String,
	pub exit_url: String,
	pub exit_pathname: String,
	pub event_count: i64,
	pub pageview_count: i64,
	pub is_bounce: bool,
	pub channel: String,
	pub referrer_domain: String,
	pub utm_source: String,
	pub utm_medium: String,
	pub utm_campaign: String,
	pub device_type: String,
	pub browser: String,
	pub os: String,
}

/// What deleting a person removed.
#[derive(Clone, Debug, Default, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Deleted {
	pub events: u64,
	pub sessions: u64,
}

/// Lists persons, newest first, filtered and paginated. `search` matches a
/// substring of a distinct id or of the properties' JSON; `property` and
/// `value` together match one property's exact value, such as property
/// `plan` being `pro`.
#[rpc::query(service = "analytics")]
pub async fn persons(
	cx: &rpc::QueryCx,
	#[serde(default)] search: Option<String>,
	#[serde(default)] property: Option<String>,
	#[serde(default)] value: Option<String>,
	limit: u32,
	#[serde(default)] offset: u32,
) -> rpc::Result<Vec<PersonSummary>> {
	app_only(cx.identity())?;
	let db = cx.db();
	let dialect = db.dialect();
	let limit = i64::from(limit.clamp(1, MAX_PAGE));
	let offset = i64::from(offset);

	let mut where_clause = String::new();
	let mut args: Vec<Value> = Vec::new();
	if let (Some(property), Some(value)) = (property.as_deref(), value.as_deref()) {
		args.push(Value::Text(property.to_owned()));
		args.push(Value::Text(value.to_owned()));
		let last = args.len();
		where_clause = match dialect {
			Dialect::Postgres => {
				format!("WHERE \"properties\"::jsonb ->> ${} = ${last}", last - 1)
			}
			Dialect::Sqlite => {
				format!(
					"WHERE json_extract(\"properties\", '$.\"{}\"') = ?{last}",
					property.replace('"', "\"\""),
				)
			}
		};
	}
	if let Some(search) = search.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
		args.push(Value::Text(format!("%{search}%")));
		let like = match dialect {
			Dialect::Postgres => "ILIKE",
			Dialect::Sqlite => "LIKE",
		};
		let found = match where_clause.is_empty() {
			true => "WHERE",
			false => "AND",
		};
		let at = placeholder(dialect, args.len());
		where_clause = format!(
			"{where_clause} {found} (\"properties\" {like} {at} OR EXISTS (SELECT 1 FROM \
			 \"person_distinct_ids\" \"d\" WHERE \"d\".\"person_id\" = \"persons\".\"id\" AND \
			 \"d\".\"distinct_id\" {like} {at}))"
		);
	}
	let at = args.len();
	let sql = format!(
		"SELECT \"id\", \"properties\", \"created_at\", \"is_identified\", \"version\" FROM \
		 \"persons\" {where_clause} ORDER BY \"created_at\" DESC, \"id\" LIMIT {} OFFSET {}",
		placeholder(dialect, at + 1),
		placeholder(dialect, at + 2),
	);
	args.push(Value::Int(limit));
	args.push(Value::Int(offset));
	let rows = person_read(db, || Statement::new(sql, args).reads::<tables::Person>()).await?;
	Ok(rows.iter().filter_map(|row| person_summary(row)).collect())
}

/// One person with their distinct ids, or nothing when the id is unknown.
#[rpc::query(service = "analytics")]
pub async fn person(cx: &rpc::QueryCx, id: Uuid) -> rpc::Result<Option<PersonView>> {
	app_only(cx.identity())?;
	let db = cx.db();
	let dialect = db.dialect();
	let summary = person_row(db, dialect, id)
		.await
		.map_err(rpc::Error::server)?;
	let Some(summary) = summary else {
		return Ok(None);
	};
	let rows = person_read(db, || {
		Statement::new(
			"SELECT \"distinct_id\" FROM \"person_distinct_ids\" WHERE \"person_id\" = ?1 ORDER \
			 BY \"created_at\", \"distinct_id\""
				.replace("?1", &placeholder(dialect, 1)),
			vec![Value::Uuid(id)],
		)
		.reads::<tables::PersonDistinctId>()
	})
	.await?;
	let distinct_ids = rows
		.iter()
		.filter_map(|row| {
			match row.first() {
				Some(Value::Text(distinct_id)) => Some(distinct_id.clone()),
				_ => None,
			}
		})
		.collect();
	Ok(Some(PersonView {
		person: summary,
		distinct_ids,
	}))
}

/// The person's events, newest first: those stored with them as their
/// person, and those stored under any of their distinct ids.
#[rpc::query(service = "analytics")]
pub async fn person_events(
	cx: &rpc::QueryCx,
	id: Uuid,
	limit: u32,
	#[serde(default)] offset: u32,
) -> rpc::Result<Vec<EventView>> {
	app_only(cx.identity())?;
	let db = cx.db();
	let dialect = db.dialect();
	let limit = i64::from(limit.clamp(1, MAX_PAGE));
	let offset = i64::from(offset);
	let sql = format!(
		"SELECT \"uuid\", \"name\", \"distinct_id\", \"timestamp\", \"session_id\", \
		 \"properties\" FROM \"events\" WHERE \"person_id\" = {person} OR \"distinct_id\" IN \
		 (SELECT \"distinct_id\" FROM \"person_distinct_ids\" WHERE \"person_id\" = {person}) \
		 ORDER BY \"timestamp\" DESC, \"uuid\" LIMIT {at_limit} OFFSET {at_offset}",
		person = placeholder(dialect, 1),
		at_limit = placeholder(dialect, 2),
		at_offset = placeholder(dialect, 3),
	);
	let rows = person_read(db, || {
		Statement::new(
			sql,
			vec![Value::Uuid(id), Value::Int(limit), Value::Int(offset)],
		)
		.reads::<tables::Event>()
		.reads::<tables::PersonDistinctId>()
	})
	.await?;
	Ok(rows.iter().filter_map(|row| event_view(row)).collect())
}

/// The person's sessions, newest first.
#[rpc::query(service = "analytics")]
pub async fn person_sessions(
	cx: &rpc::QueryCx,
	id: Uuid,
	limit: u32,
	#[serde(default)] offset: u32,
) -> rpc::Result<Vec<SessionView>> {
	app_only(cx.identity())?;
	let db = cx.db();
	let dialect = db.dialect();
	let limit = i64::from(limit.clamp(1, MAX_PAGE));
	let offset = i64::from(offset);
	let sql = format!(
		"SELECT \"id\", \"distinct_id\", \"person_id\", \"first_timestamp\", \"last_timestamp\", \
		 \"duration_ms\", \"entry_url\", \"entry_pathname\", \"exit_url\", \"exit_pathname\", \
		 \"event_count\", \"pageview_count\", \"is_bounce\", \"channel\", \"referrer_domain\", \
		 \"utm_source\", \"utm_medium\", \"utm_campaign\", \"device_type\", \"browser\", \"os\" \
		 FROM \"sessions\" WHERE \"person_id\" = {person} OR \"distinct_id\" IN (SELECT \
		 \"distinct_id\" FROM \"person_distinct_ids\" WHERE \"person_id\" = {person}) ORDER BY \
		 \"last_timestamp\" DESC LIMIT {at_limit} OFFSET {at_offset}",
		person = placeholder(dialect, 1),
		at_limit = placeholder(dialect, 2),
		at_offset = placeholder(dialect, 3),
	);
	let rows = person_read(db, || {
		Statement::new(
			sql,
			vec![Value::Uuid(id), Value::Int(limit), Value::Int(offset)],
		)
		.reads::<tables::Session>()
		.reads::<tables::PersonDistinctId>()
	})
	.await?;
	Ok(rows.iter().filter_map(|row| session_view(row)).collect())
}

/// Deletes the person and everything stored about them, at once: their
/// distinct ids, overrides, events, and sessions (§17). Deleting in
/// bounded chunks by a job, and the replay and survey answers that step 7
/// and 9 bring, come later.
#[rpc::mutation(service = "analytics")]
pub async fn delete_person(cx: &rpc::MutationCx, id: Uuid) -> rpc::Result<Deleted> {
	app_only(cx.identity())?;
	let tx = cx.tx();
	let dialect = tx.dialect();
	let person = placeholder(dialect, 1);
	let mut deleted = Deleted::default();
	for (table, filter) in [
		(
			"events",
			format!(
				"\"person_id\" = {person} OR \"distinct_id\" IN (SELECT \"distinct_id\" FROM \
				 \"person_distinct_ids\" WHERE \"person_id\" = {person})"
			),
		),
		(
			"sessions",
			format!(
				"\"person_id\" = {person} OR \"distinct_id\" IN (SELECT \"distinct_id\" FROM \
				 \"person_distinct_ids\" WHERE \"person_id\" = {person})"
			),
		),
		("person_overrides", format!("\"person_id\" = {person}")),
		("person_distinct_ids", format!("\"person_id\" = {person}")),
		("persons", format!("\"id\" = {person}")),
	] {
		let removed = tx
			.execute(Statement {
				sql: format!("DELETE FROM \"{table}\" WHERE {filter}"),
				args: vec![Value::Uuid(id)],
			})
			.await
			.map_err(|error| rpc::Error::server(error.to_string()))?;
		match table {
			"events" => deleted.events = u64::try_from(removed).unwrap_or(u64::MAX),
			"sessions" => deleted.sessions = u64::try_from(removed).unwrap_or(u64::MAX),
			_ => {}
		}
	}
	Ok(deleted)
}

/// Everything stored about a person, as one JSON document: the person,
/// their distinct ids, their events, and their sessions (§17). When the
/// service declares an `exports` volume, the document is also written
/// there and its file named in the answer; a download link valid for an
/// hour comes with the admin shell (§18).
#[rpc::mutation(service = "analytics")]
pub async fn export_person(cx: &rpc::MutationCx, id: Uuid) -> rpc::Result<serde_json::Value> {
	app_only(cx.identity())?;
	let tx = cx.tx();
	let dialect = tx.dialect();
	let summary = person_row_of(tx, dialect, id)
		.await
		.map_err(rpc::Error::server)?;
	let Some(summary) = summary else {
		return Err(rpc::Error::invalid("no person with that id"));
	};
	let distinct_ids: Vec<String> = tx
		.fetch(Statement {
			sql: "SELECT \"distinct_id\" FROM \"person_distinct_ids\" WHERE \"person_id\" = ?1 \
			      ORDER BY \"created_at\""
				.replace("?1", &placeholder(dialect, 1)),
			args: vec![Value::Uuid(id)],
		})
		.await
		.map_err(|error| rpc::Error::server(error.to_string()))?
		.iter()
		.filter_map(|row| {
			match row.first() {
				Some(Value::Text(distinct_id)) => Some(distinct_id.clone()),
				_ => None,
			}
		})
		.collect();
	let events = tx
		.fetch(Statement {
			sql: format!(
				"SELECT \"uuid\", \"name\", \"distinct_id\", \"timestamp\", \"session_id\", \
				 \"properties\" FROM \"events\" WHERE \"person_id\" = {person} OR \"distinct_id\" \
				 IN (SELECT \"distinct_id\" FROM \"person_distinct_ids\" WHERE \"person_id\" = \
				 {person}) ORDER BY \"timestamp\" DESC",
				person = placeholder(dialect, 1),
			),
			args: vec![Value::Uuid(id)],
		})
		.await
		.map_err(|error| rpc::Error::server(error.to_string()))?;
	let sessions = tx
		.fetch(Statement {
			sql: format!(
				"SELECT \"id\", \"distinct_id\", \"person_id\", \"first_timestamp\", \
				 \"last_timestamp\", \"duration_ms\", \"entry_url\", \"entry_pathname\", \
				 \"exit_url\", \"exit_pathname\", \"event_count\", \"pageview_count\", \
				 \"is_bounce\", \"channel\", \"referrer_domain\", \"utm_source\", \"utm_medium\", \
				 \"utm_campaign\", \"device_type\", \"browser\", \"os\" FROM \"sessions\" WHERE \
				 \"person_id\" = {person} OR \"distinct_id\" IN (SELECT \"distinct_id\" FROM \
				 \"person_distinct_ids\" WHERE \"person_id\" = {person}) ORDER BY \
				 \"last_timestamp\" DESC",
				person = placeholder(dialect, 1),
			),
			args: vec![Value::Uuid(id)],
		})
		.await
		.map_err(|error| rpc::Error::server(error.to_string()))?;
	let mut document = serde_json::json!({
		"person": {
			"person": summary,
			"distinct_ids": distinct_ids,
		},
		"events": events.iter().filter_map(|row| event_view(row)).collect::<Vec<_>>(),
		"sessions": sessions.iter().filter_map(|row| session_view(row)).collect::<Vec<_>>(),
	});

	if let Ok(volume) = std::env::var("LEPTOSKIT_SERVICE_VOLUME_EXPORTS")
		&& directory(volume.trim()).is_some()
	{
		let file = directory(volume.trim()).expect("checked").join(format!(
			"person-{id}-{}.json",
			leptoskit_db::now().format("%Y%m%dT%H%M%S")
		));
		if std::fs::write(&file, document.to_string()).is_ok() {
			document["file"] = serde_json::Value::String(file.display().to_string());
		}
	}
	Ok(document)
}

/// Refuses anyone but the app, as `register_events` does.
fn app_only(identity: &Identity) -> Result<(), rpc::Error> {
	if identity.caller.as_deref() != Some(rpc::server::APP_CALLER) {
		return Err(rpc::Error::forbidden(
			"only the app reads and manages persons: call this from its server, with the \
			 service's token",
		));
	}
	Ok(())
}

/// The volume's directory, when it exists.
fn directory(volume: &str) -> Option<std::path::PathBuf> {
	let directory = std::path::PathBuf::from(volume);
	directory.is_dir().then_some(directory)
}

/// Runs a raw read over a `Tracked` connection: `declare` builds it, naming
/// every table its SQL touches, so the tracked connection records them.
async fn person_read(
	db: &leptoskit_db::Tracked<leptoskit_db::Db>,
	declare: impl FnOnce() -> RawRead,
) -> Result<Vec<Vec<Value>>, rpc::Error> {
	declare()
		.fetch(db)
		.await
		.map_err(|error| rpc::Error::server(error.to_string()))
}

/// The person's summary row, through either kind of connection.
async fn person_row<C: leptoskit_db::Read>(
	connection: &C,
	dialect: Dialect,
	id: Uuid,
) -> Result<Option<PersonSummary>, String> {
	let rows = Statement::new(
		"SELECT \"id\", \"properties\", \"created_at\", \"is_identified\", \"version\" FROM \
		 \"persons\" WHERE \"id\" = ?1"
			.replace("?1", &placeholder(dialect, 1)),
		vec![Value::Uuid(id)],
	)
	.reads::<tables::Person>()
	.fetch(connection)
	.await
	.map_err(|error| error.to_string())?;
	Ok(rows.first().and_then(|row| person_summary(row)))
}

/// The person's summary row, through a transaction, which records nothing.
async fn person_row_of<C: leptoskit_db::Read>(
	connection: &C,
	dialect: Dialect,
	id: Uuid,
) -> Result<Option<PersonSummary>, String> {
	let rows = connection
		.fetch(Statement::new(
			"SELECT \"id\", \"properties\", \"created_at\", \"is_identified\", \"version\" FROM \
			 \"persons\" WHERE \"id\" = ?1"
				.replace("?1", &placeholder(dialect, 1)),
			vec![Value::Uuid(id)],
		))
		.await
		.map_err(|error| error.to_string())?;
	Ok(rows.first().and_then(|row| person_summary(row)))
}

/// A person row as the summary, or nothing when a column is unreadable.
fn person_summary(row: &[Value]) -> Option<PersonSummary> {
	let id = uuid_of(row.first()?)?;
	let properties = match row.get(1)? {
		Value::Text(properties) => serde_json::from_str(properties).ok()?,
		_ => return None,
	};
	let created_at = timestamp_of(row.get(2)?)?;
	let is_identified = matches!(row.get(3), Some(Value::Bool(true)) | Some(Value::Int(1)));
	let version = match row.get(4) {
		Some(Value::Int(version)) => *version,
		_ => 1,
	};
	Some(PersonSummary {
		id,
		properties,
		created_at: stamp(created_at),
		is_identified,
		version,
	})
}

fn event_view(row: &[Value]) -> Option<EventView> {
	let uuid = uuid_of(row.first()?)?;
	let Value::Text(name) = row.get(1)? else {
		return None;
	};
	let Value::Text(distinct_id) = row.get(2)? else {
		return None;
	};
	let timestamp = timestamp_of(row.get(3)?)?;
	let session_id = row.get(4).and_then(uuid_of);
	let properties = match row.get(5)? {
		Value::Text(properties) => serde_json::from_str(properties).ok()?,
		_ => return None,
	};
	Some(EventView {
		uuid,
		name: name.clone(),
		distinct_id: distinct_id.clone(),
		timestamp: stamp(timestamp),
		session_id,
		properties,
	})
}

fn session_view(row: &[Value]) -> Option<SessionView> {
	let text = |index: usize| {
		match row.get(index) {
			Some(Value::Text(text)) => text.clone(),
			_ => String::new(),
		}
	};
	let int = |index: usize| {
		match row.get(index) {
			Some(Value::Int(int)) => *int,
			_ => 0,
		}
	};
	Some(SessionView {
		id: uuid_of(row.first()?)?,
		distinct_id: text(1),
		person_id: row.get(2).and_then(uuid_of),
		first_timestamp: stamp(timestamp_of(row.get(3)?)?),
		last_timestamp: stamp(timestamp_of(row.get(4)?)?),
		duration_ms: int(5),
		entry_url: text(6),
		entry_pathname: text(7),
		exit_url: text(8),
		exit_pathname: text(9),
		event_count: int(10),
		pageview_count: int(11),
		is_bounce: matches!(row.get(12), Some(Value::Bool(true)) | Some(Value::Int(1))),
		channel: text(13),
		referrer_domain: text(14),
		utm_source: text(15),
		utm_medium: text(16),
		utm_campaign: text(17),
		device_type: text(18),
		browser: text(19),
		os: text(20),
	})
}

/// An instant as text, in the one format every answer uses.
fn stamp(time: leptoskit_db::Timestamp) -> String {
	time.to_rfc3339_opts(SecondsFormat::Millis, true)
}
