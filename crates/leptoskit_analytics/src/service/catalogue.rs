//! The event catalogue: each typed event's schema ([`crate::Event`]), by
//! schema id, so the service can name a typed event's values.
//!
//! The app registers its schemas as the app (the `register_events`
//! operation), from its server: `client::server::Analytics` registers what
//! the server was built with when it starts. A registration adds versions
//! and never removes one, so an event an older client recorded, perhaps
//! offline, still reads. The service checks each schema id against its name
//! and fields, so a schema can't be registered under another's id.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::PoisonError;
use std::sync::RwLock;

use leptoskit_db::Connection as _;
use leptoskit_db::Db;
use leptoskit_db::Read as _;
use leptoskit_db::Statement;
use leptoskit_db::Value as DbValue;
use serde::Deserialize;
use serde::Serialize;

use super::ingest::Insert;
use crate::wire::Properties;
use crate::wire::Value;

/// The most schemas one registration may hold.
pub const MAX_SCHEMAS: usize = 1_000;

/// The most fields one schema may have.
pub const MAX_FIELDS: usize = 256;

/// The most bytes a field's name or type may have.
const MAX_FIELD_BYTES: usize = 200;

/// A schema, as the app registers it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SchemaInput {
	pub schema_id: u64,
	pub name: String,
	/// Each property's name and type as spelled.
	pub fields: Vec<(String, String)>,
}

/// What a registration came to.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Registered {
	/// Schemas the catalogue didn't have.
	pub added: u32,
	/// Schemas it had already.
	pub known: u32,
}

#[derive(Clone, Debug)]
struct Schema {
	name: String,
	fields: Vec<(String, String)>,
}

/// The catalogue, cached in memory and read through to the database, so a
/// schema another replica registered is found too. Clones share it.
#[derive(Clone)]
pub struct Catalogue {
	db: Db,
	schemas: Arc<RwLock<HashMap<u64, Schema>>>,
}

impl Catalogue {
	/// An empty cache of the catalogue in `db`.
	pub fn new(db: Db) -> Self {
		Self {
			db,
			schemas: Arc::default(),
		}
	}

	/// Makes sure every schema id in `ids` that's in the database is cached.
	pub async fn load(&self, ids: &[u64]) -> leptoskit_db::Result<()> {
		let db = &self.db;
		let missing: Vec<u64> = {
			let schemas = self.schemas.read().unwrap_or_else(PoisonError::into_inner);
			let mut missing: Vec<u64> = ids
				.iter()
				.copied()
				.filter(|id| !schemas.contains_key(id))
				.collect();
			missing.sort_unstable();
			missing.dedup();
			missing
		};
		if missing.is_empty() {
			return Ok(());
		}
		let placeholders: Vec<String> = (1..=missing.len())
			.map(|number| {
				match db.dialect() {
					leptoskit_db::Dialect::Postgres => format!("${number}"),
					leptoskit_db::Dialect::Sqlite => format!("?{number}"),
				}
			})
			.collect();
		let rows = db
			.fetch(Statement {
				sql: format!(
					"SELECT \"schema_id\", \"name\", \"fields\" FROM \"event_schemas\" WHERE \
					 \"schema_id\" IN ({})",
					placeholders.join(", ")
				),
				args: missing.iter().map(|id| DbValue::Int(*id as i64)).collect(),
			})
			.await?;
		let mut schemas = self.schemas.write().unwrap_or_else(PoisonError::into_inner);
		for row in rows {
			let (Some(DbValue::Int(id)), Some(DbValue::Text(name)), Some(DbValue::Text(fields))) =
				(row.first(), row.get(1), row.get(2))
			else {
				continue;
			};
			let Ok(fields) = serde_json::from_str(fields) else {
				continue;
			};
			schemas.insert(
				*id as u64,
				Schema {
					name: name.clone(),
					fields,
				},
			);
		}
		Ok(())
	}

	/// Names a typed event's values: the event's properties, its values by
	/// their field names followed by `extra`, or why it can't be read:
	/// `unknown_schema`, which the client answers by sending it again with
	/// names, or `schema_mismatch`.
	pub fn resolve(
		&self,
		name: &str,
		schema_id: u64,
		values: Vec<Value>,
		extra: Properties,
	) -> Result<Properties, &'static str> {
		let schemas = self.schemas.read().unwrap_or_else(PoisonError::into_inner);
		let Some(schema) = schemas.get(&schema_id) else {
			return Err("unknown_schema");
		};
		if schema.name != name
			|| schema.fields.len() != values.len()
			|| !schema
				.fields
				.iter()
				.zip(&values)
				.all(|((_, ty), value)| fits(ty, value))
		{
			return Err("schema_mismatch");
		}
		let mut properties: Properties = schema
			.fields
			.iter()
			.map(|(field, _)| field.clone())
			.zip(values)
			.collect();
		for (key, value) in extra {
			if !properties.iter().any(|(have, _)| *have == key) {
				properties.push((key, value));
			}
		}
		Ok(properties)
	}

	/// Adds `schemas` to the catalogue, as the app. Refused whole when one
	/// isn't valid, so a build registers all of its events or none.
	pub async fn register(&self, schemas: Vec<SchemaInput>) -> Result<Registered, String> {
		let db = &self.db;
		if schemas.len() > MAX_SCHEMAS {
			return Err(format!("at most {MAX_SCHEMAS} schemas at once"));
		}
		for schema in &schemas {
			validate(schema)?;
		}
		let now = leptoskit_db::now();
		let tx = db.begin().await.map_err(|error| error.to_string())?;
		let mut added = 0;
		for chunk in schemas.chunks(200) {
			let mut insert = Insert::new(
				db.dialect(),
				"event_schemas",
				&[
					"schema_id",
					"name",
					"fields",
					"first_registered",
					"last_registered",
				],
			);
			for schema in chunk {
				insert.row(vec![
					DbValue::Int(schema.schema_id as i64),
					DbValue::Text(schema.name.clone()),
					DbValue::Text(serde_json::to_string(&schema.fields).unwrap_or_default()),
					DbValue::Timestamp(now),
					DbValue::Timestamp(now),
				]);
			}
			let mut statement = insert.on_conflict_do_nothing("schema_id");
			statement.sql.push_str(" RETURNING \"schema_id\"");
			added += tx
				.fetch(statement)
				.await
				.map_err(|error| error.to_string())?
				.len() as u32;
		}
		let ids: Vec<DbValue> = schemas
			.iter()
			.map(|schema| DbValue::Int(schema.schema_id as i64))
			.collect();
		for chunk in ids.chunks(500) {
			let placeholders: Vec<String> = (2..chunk.len() + 2)
				.map(|number| {
					match db.dialect() {
						leptoskit_db::Dialect::Postgres => format!("${number}"),
						leptoskit_db::Dialect::Sqlite => format!("?{number}"),
					}
				})
				.collect();
			let mut args = vec![DbValue::Timestamp(now)];
			args.extend(chunk.iter().cloned());
			tx.execute(Statement {
				sql: format!(
					"UPDATE \"event_schemas\" SET \"last_registered\" = {} WHERE \"schema_id\" IN \
					 ({})",
					match db.dialect() {
						leptoskit_db::Dialect::Postgres => "$1",
						leptoskit_db::Dialect::Sqlite => "?1",
					},
					placeholders.join(", ")
				),
				args,
			})
			.await
			.map_err(|error| error.to_string())?;
		}
		tx.commit().await.map_err(|error| error.to_string())?;

		let mut cached = self.schemas.write().unwrap_or_else(PoisonError::into_inner);
		for schema in schemas {
			cached.insert(
				schema.schema_id,
				Schema {
					name: schema.name,
					fields: schema.fields,
				},
			);
		}
		Ok(Registered {
			added,
			known: u32::try_from(ids.len()).unwrap_or(u32::MAX) - added,
		})
	}
}

fn validate(schema: &SchemaInput) -> Result<(), String> {
	let name = &schema.name;
	if name.trim().is_empty()
		|| name.len() > 200
		|| name.chars().any(char::is_control)
		|| name.starts_with('$')
	{
		return Err(format!("{name:?} isn't a valid event name"));
	}
	if schema.fields.len() > MAX_FIELDS {
		return Err(format!("`{name}` has more than {MAX_FIELDS} fields"));
	}
	for (field, ty) in &schema.fields {
		let valid = |text: &str| {
			!text.is_empty() && text.len() <= MAX_FIELD_BYTES && !text.chars().any(char::is_control)
		};
		if !valid(field)
			|| !valid(ty)
			|| schema
				.fields
				.iter()
				.filter(|(other, _)| other == field)
				.count()
				> 1
		{
			return Err(format!("`{name}` has a field that isn't valid: {field:?}"));
		}
	}
	let fields: Vec<(&str, &str)> = schema
		.fields
		.iter()
		.map(|(field, ty)| (field.as_str(), ty.as_str()))
		.collect();
	if crate::schema_id(name, &fields) != schema.schema_id {
		return Err(format!(
			"`{name}`'s schema id doesn't match its name and fields"
		));
	}
	Ok(())
}

/// Whether `value` can be a value of the type spelled `ty`. Types are known
/// only by their spelling (see [`crate::Event::SCHEMA_ID`]), so the check
/// covers the spellings `Value` converts from and accepts anything else.
fn fits(ty: &str, value: &Value) -> bool {
	if let Some(inner) = ty
		.strip_prefix("Option<")
		.and_then(|rest| rest.strip_suffix('>'))
	{
		return matches!(value, Value::Null) || fits(inner, value);
	}
	if ty.starts_with("Vec<") {
		return matches!(value, Value::List(_));
	}
	match ty {
		"String" | "&str" | "&String" | "&'staticstr" => matches!(value, Value::Text(_)),
		"bool" => matches!(value, Value::Bool(_)),
		"i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" => matches!(value, Value::Int(_)),
		"f32" | "f64" => matches!(value, Value::Float(_)),
		_ => true,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	async fn catalogue(fields: &[(&str, &str)]) -> (Catalogue, u64) {
		let catalogue =
			Catalogue::new(leptoskit_db::testing::sqlite_migrated(super::super::MIGRATIONS).await);
		let id = crate::schema_id("checkout", fields);
		catalogue.schemas.write().unwrap().insert(
			id,
			Schema {
				name: "checkout".into(),
				fields: fields
					.iter()
					.map(|(field, ty)| ((*field).into(), (*ty).into()))
					.collect(),
			},
		);
		(catalogue, id)
	}

	#[tokio::test]
	async fn a_known_schema_names_the_values() {
		let (catalogue, id) = catalogue(&[("plan", "String"), ("coupon", "Option<String>")]).await;
		let properties = catalogue
			.resolve(
				"checkout",
				id,
				vec![Value::Text("pro".into()), Value::Null],
				vec![("flavour".into(), Value::Text("beta".into()))],
			)
			.unwrap();
		assert_eq!(
			properties,
			[
				("plan".to_owned(), Value::Text("pro".into())),
				("coupon".to_owned(), Value::Null),
				("flavour".to_owned(), Value::Text("beta".into())),
			]
		);
	}

	#[tokio::test]
	async fn unknown_ids_counts_and_types_are_refused() {
		let (catalogue, id) = catalogue(&[("plan", "String"), ("seats", "u32")]).await;
		let resolve = |id, values| catalogue.resolve("checkout", id, values, Vec::new());
		assert_eq!(resolve(id ^ 1, vec![]), Err("unknown_schema"));
		assert_eq!(
			resolve(id, vec![Value::Text("pro".into())]),
			Err("schema_mismatch")
		);
		assert_eq!(
			resolve(id, vec![Value::Text("pro".into()), Value::Text("3".into())]),
			Err("schema_mismatch")
		);
		assert_eq!(
			catalogue.resolve(
				"renamed",
				id,
				vec![Value::Text("pro".into()), Value::Int(3)],
				Vec::new()
			),
			Err("schema_mismatch")
		);
		assert!(resolve(id, vec![Value::Text("pro".into()), Value::Int(3)]).is_ok());
	}

	#[test]
	fn a_schema_must_match_its_id_and_rules() {
		let good = SchemaInput {
			schema_id: crate::schema_id("checkout", &[("plan", "String")]),
			name: "checkout".into(),
			fields: vec![("plan".into(), "String".into())],
		};
		assert!(validate(&good).is_ok());
		let mut wrong_id = good.clone();
		wrong_id.schema_id ^= 1;
		assert!(validate(&wrong_id).unwrap_err().contains("doesn't match"));
		let mut reserved = good.clone();
		reserved.name = "$pageview".into();
		assert!(validate(&reserved).is_err());
		let mut twice = good;
		twice.fields.push(("plan".into(), "String".into()));
		assert!(validate(&twice).is_err());
	}
}
