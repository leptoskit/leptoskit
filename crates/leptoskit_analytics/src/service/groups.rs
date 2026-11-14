//! Group processing (§8 of the design): the kinds of group a project
//! uses, at most five, and each group's properties, from `$groupidentify`
//! entries. Events carry their groups as `$groups`; these rows hold what
//! insights count when they count groups instead of persons.

use std::collections::HashMap;

use leptoskit_db::Connection as _;
use leptoskit_db::Dialect;
use leptoskit_db::Read as _;
use leptoskit_db::Statement;
use leptoskit_db::Timestamp;
use leptoskit_db::Tx;
use leptoskit_db::Value;
use serde_json::Value as Json;
use sha2::Digest as _;
use sha2::Sha256;

use super::accept::Row;
use super::ingest::Insert;
use super::ingest::Warning;
use super::persons::placeholders;
use super::persons::stamp;

/// The most group types one project may have, as `PostHog`'s five.
pub const MAX_GROUP_TYPES: usize = 5;

/// Processes the batch's `$groupidentify` entries inside `tx`.
pub(crate) async fn process(
	tx: &Tx,
	dialect: Dialect,
	rows: &[&Row],
	now: Timestamp,
) -> Result<Vec<Warning>, String> {
	let mut entries: Vec<(String, String)> = Vec::new();
	for row in rows {
		if row.name != "$groupidentify" {
			continue;
		}
		let Ok(properties) = serde_json::from_str::<Json>(&row.properties) else {
			continue;
		};
		let group_type = properties.get("$group_type").and_then(Json::as_str);
		let group_key = properties.get("$group_key").and_then(Json::as_str);
		if let Some((group_type, group_key)) = group_type.zip(group_key) {
			entries.push((group_type.to_owned(), group_key.to_owned()));
		}
	}
	if entries.is_empty() {
		return Ok(Vec::new());
	}

	// The registry: a name keeps the index it first took.
	let mut types: HashMap<String, i16> = HashMap::new();
	let mut highest = -1_i16;
	let registry = tx
		.fetch(Statement {
			sql: "SELECT \"name\", \"index\" FROM \"group_types\"".into(),
			args: Vec::new(),
		})
		.await
		.map_err(|error| error.to_string())?;
	for row in registry {
		let (Some(Value::Text(name)), Some(Value::Int(index))) = (row.first(), row.get(1)) else {
			continue;
		};
		let Ok(index) = i16::try_from(*index) else {
			continue;
		};
		highest = highest.max(index);
		types.insert(name.clone(), index);
	}

	let mut warnings = Vec::new();
	let mut new_types: Vec<(String, i16)> = Vec::new();
	for (group_type, _) in &entries {
		if types.contains_key(group_type) {
			continue;
		}
		if types.len() >= MAX_GROUP_TYPES {
			if !warnings
				.iter()
				.any(|warning: &Warning| warning.detail.contains(group_type.as_str()))
			{
				warnings.push(Warning {
					reason: "too_many_group_types".into(),
					count: 1,
					detail: format!(
						"$groupidentify for {group_type} ignored: a project has at most \
						 {MAX_GROUP_TYPES} group types"
					),
				});
			}
			continue;
		}
		highest += 1;
		types.insert(group_type.clone(), highest);
		new_types.push((group_type.clone(), highest));
	}
	if !new_types.is_empty() {
		let mut insert = Insert::new(dialect, "group_types", &["name", "index", "created_at"]);
		for (name, index) in &new_types {
			insert.row(vec![
				Value::Text(name.clone()),
				Value::Int(i64::from(*index)),
				Value::Timestamp(now),
			]);
		}
		tx.execute(insert.on_conflict_do_nothing("name"))
			.await
			.map_err(|error| error.to_string())?;
	}

	// What the batch's groups hold now, so `$group_set` merges per key
	// rather than replacing everything.
	let mut groups: HashMap<i64, (Json, HashMap<String, String>)> = HashMap::new();
	let ids: Vec<i64> = entries
		.iter()
		.filter_map(|(group_type, group_key)| {
			types
				.get(group_type)
				.map(|index| group_id(*index, group_key))
		})
		.collect();
	for chunk in ids.chunks(super::ingest::ROWS_PER_STATEMENT) {
		let existing = tx
			.fetch(Statement {
				sql: format!(
					"SELECT \"id\", \"properties\", \"properties_updated_at\" FROM \"groups\" \
					 WHERE \"id\" IN ({})",
					placeholders(dialect, chunk.len())
				),
				args: chunk.iter().map(|id| Value::Int(*id)).collect(),
			})
			.await
			.map_err(|error| error.to_string())?;
		for row in existing {
			let (Some(Value::Int(id)), Some(properties), Some(updated_at)) =
				(row.first(), row.get(1), row.get(2))
			else {
				continue;
			};
			let Value::Text(properties) = properties else {
				continue;
			};
			let Value::Text(updated_at) = updated_at else {
				continue;
			};
			groups.insert(
				*id,
				(
					serde_json::from_str(properties).unwrap_or(Json::Object(Default::default())),
					serde_json::from_str(updated_at).unwrap_or_default(),
				),
			);
		}
	}

	// Apply each entry's `$group_set` last-write-wins by event time, in
	// arrival order.
	let mut touched: Vec<(i64, i16, String, Json, HashMap<String, String>)> = Vec::new();
	for row in rows.iter().filter(|row| row.name == "$groupidentify") {
		let Ok(properties) = serde_json::from_str::<Json>(&row.properties) else {
			continue;
		};
		let Some(group_type) = properties.get("$group_type").and_then(Json::as_str) else {
			continue;
		};
		let Some(group_key) = properties.get("$group_key").and_then(Json::as_str) else {
			continue;
		};
		let Some(index) = types.get(group_type) else {
			continue;
		};
		let id = group_id(*index, group_key);
		let when = stamp(row.timestamp);
		if let Some(edit) = touched.iter_mut().find(|(touched, ..)| *touched == id) {
			merge_set(&mut edit.3, &mut edit.4, &properties, &when);
			continue;
		}
		let (mut merged, mut seen) = groups.get(&id).map_or(
			(Json::Object(serde_json::Map::new()), HashMap::new()),
			|(properties, seen)| (properties.clone(), seen.clone()),
		);
		merge_set(&mut merged, &mut seen, &properties, &when);
		touched.push((id, *index, group_key.to_owned(), merged, seen));
	}

	for chunk in touched.chunks(super::ingest::ROWS_PER_STATEMENT) {
		let mut insert = Insert::new(
			dialect,
			"groups",
			&[
				"id",
				"type_index",
				"key",
				"properties",
				"properties_updated_at",
				"created_at",
			],
		);
		for (id, index, key, properties, seen) in chunk {
			insert.row(vec![
				Value::Int(*id),
				Value::Int(i64::from(*index)),
				Value::Text(key.clone()),
				Value::Text(properties.to_string()),
				Value::Text(serde_json::to_string(seen).unwrap_or_default()),
				Value::Timestamp(now),
			]);
		}
		let mut statement = insert.finish();
		statement.sql.push_str(
			" ON CONFLICT (\"id\") DO UPDATE SET \"properties\" = excluded.\"properties\", \
			 \"properties_updated_at\" = excluded.\"properties_updated_at\"",
		);
		tx.execute(statement)
			.await
			.map_err(|error| error.to_string())?;
	}

	Ok(warnings)
}

/// Applies one entry's `$group_set` per key into `properties`, where
/// `seen` holds each key's last write: a key already written later than
/// this entry keeps what it has, so out-of-order arrival converges.
fn merge_set(properties: &mut Json, seen: &mut HashMap<String, String>, entry: &Json, when: &str) {
	let Some(set) = entry.get("$group_set").and_then(Json::as_object) else {
		return;
	};
	for (key, value) in set {
		if seen.get(key).is_none_or(|written| written.as_str() < when) {
			properties[key.clone()] = value.clone();
			seen.insert(key.clone(), when.to_owned());
		}
	}
}

/// A group's row id: a hash of the type's index and the key, so every
/// replica gives the same group the same id without asking the database.
#[must_use]
pub fn group_id(type_index: i16, key: &str) -> i64 {
	let digest = Sha256::digest(format!("{type_index}\n{key}").as_bytes());
	i64::from_be_bytes(digest[..8].try_into().expect("8 bytes"))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_same_group_always_hashes_the_same_id() {
		assert_eq!(group_id(0, "acme"), group_id(0, "acme"));
		assert_ne!(group_id(0, "acme"), group_id(1, "acme"));
		assert_ne!(group_id(0, "acme"), group_id(0, "acme-corp"));
		assert_ne!(
			group_id(0, "a\nb"),
			group_id(0, "a\n1b"),
			"the newline separator can't be faked"
		);
	}
}
