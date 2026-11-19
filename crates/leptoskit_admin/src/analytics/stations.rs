//! An analytics card's Workings: five stations from the struct in your app
//! to the chart on the page (`docs/agents/admin-design.md` §1).
//!
//! - **Emit**: the event's Rust type, from the service's catalogue, or a derive
//!   written from the properties it arrives with.
//! - **Send**: what one of these events weighs on the wire, typed and as JSON,
//!   encoded here from the latest one that arrived.
//! - **Store**: the tables and columns the query read.
//! - **Ask**: the typed `Query`, the SQL it became, and the HTTP call.
//! - **Draw**: the rows that came back and how they became the chart.

use std::sync::OnceLock;

use leptoskit_analytics::query::Answer;
use leptoskit_analytics::query::EventRow;
use leptoskit_analytics::query::Query;
use leptoskit_analytics::service::query::EventDefinitionView;
use leptoskit_analytics::wire;

use super::code;
use super::format;
use crate::components::Line;
use crate::workings::Body;
use crate::workings::Drawer;
use crate::workings::Station;
use crate::workings::Tab;

/// The event a query is mostly about: what Emit and Send explain.
pub(crate) fn focus_event(query: &Query) -> Option<String> {
	match query {
		Query::Trends(trends) => trends.series.first().map(|series| series.event.clone()),
		Query::Funnels(funnels) => funnels.steps.first().cloned(),
		Query::Retention(retention) => Some(retention.start_event.clone()),
		Query::WebOverview(_) => Some("$pageview".to_owned()),
		Query::Events(events) => events.event.clone(),
		Query::Actors(actors) => focus_event(&actors.query),
	}
}

/// How a card's rows became its chart.
pub(crate) struct Drawn {
	/// One or two sentences: how many rows, what the chart does with them.
	pub sentence: String,
	/// The rows as a small table, header first.
	pub rows: Vec<String>,
}

/// What a drawer is built from.
pub(crate) struct Source<'a> {
	pub id: &'a str,
	pub title: String,
	pub query: &'a Query,
	pub answer: Option<&'a Answer>,
	pub definitions: &'a [EventDefinitionView],
	/// The latest fetched event of [`focus_event`], or any event when the
	/// query has no event filter.
	pub sample: Option<&'a EventRow>,
	/// The service's origin, for the HTTP tab.
	pub url: &'a str,
	pub drawn: Drawn,
}

fn lines(text: impl IntoIterator<Item = String>) -> Vec<Line> {
	text.into_iter().map(Line::plain).collect()
}

/// The Rust type a JSON type would be written as.
fn rust_type(json: &str) -> &'static str {
	match json {
		"integer" => "i64",
		"number" => "f64",
		"boolean" => "bool",
		_ => "String",
	}
}

/// A field name as Rust spells it: `$pathname` can't be one.
fn field_name(key: &str) -> Option<String> {
	let name: String = key
		.chars()
		.map(|character| {
			if character.is_ascii_alphanumeric() {
				character.to_ascii_lowercase()
			} else {
				'_'
			}
		})
		.collect();
	let name = name.trim_matches('_').to_owned();

	if name.is_empty() || name.starts_with(|character: char| character.is_ascii_digit()) {
		return None;
	}

	Some(match name.as_str() {
		"type" | "struct" | "fn" | "match" | "ref" | "move" | "mod" | "use" | "self" => {
			format!("r#{name}")
		}
		_ => name,
	})
}

fn emit(event: &str, definition: Option<&EventDefinitionView>) -> Station {
	let struct_name = format::struct_name(event);
	let named = |lines: &mut Vec<String>| {
		if format::snake_case(&struct_name) != event {
			lines.push(format!("#[event(name = {})]", serde_json::json!(event)));
		}
	};

	if let Some(fields) = definition.and_then(|definition| definition.fields.as_ref()) {
		let mut code = vec!["#[derive(Event)]".to_owned()];
		named(&mut code);
		code.push(format!("struct {struct_name} {{"));
		code.extend(fields.iter().map(|(name, ty)| format!("    {name}: {ty},")));
		code.push("}".to_owned());
		return Station::new(
			"Emit",
			"the event catalogue",
			format!(
				"`{event}` is a Rust type in your app, so its properties are its fields and a \
				 misspelled one fails when it compiles. This is the shape the app last \
				 registered; the service doesn't record where in your code it's sent from."
			),
			Body::Code(lines(code), "Rust".to_owned()),
		);
	}

	if event.starts_with('$') {
		let properties = definition.map_or_else(Vec::new, |definition| {
			definition
				.properties
				.iter()
				.map(|(key, ty)| format!("{key}: {ty}"))
				.collect()
		});
		return Station::new(
			"Emit",
			"sent by the analytics client",
			format!(
				"`{event}` is one of the client's own events: the web client sends it by itself, \
				 with the properties below, once `web::init` runs."
			),
			if properties.is_empty() {
				Body::Missing("None have arrived yet, so there are no properties to show.".into())
			} else {
				Body::Code(lines(properties), "text".to_owned())
			},
		);
	}

	let mut code = vec![
		"// No Rust type yet. One written from the properties it arrives with:".to_owned(),
		"#[derive(Event)]".to_owned(),
	];
	named(&mut code);
	code.push(format!("struct {struct_name} {{"));

	for (key, ty) in definition.map_or(&[][..], |definition| &definition.properties[..]) {
		if let Some(name) = field_name(key) {
			code.push(format!("    {name}: {},", rust_type(ty)));
		}
	}

	code.push("}".to_owned());
	Station::new(
		"Emit",
		"no Rust type yet",
		format!(
			"`{event}` arrives with named properties and no Rust type, so a typo in a property \
			 name becomes a new property instead of an error. Declaring it as a struct fixes \
			 that, and makes each one smaller to send."
		),
		Body::Code(lines(code), "Rust".to_owned()),
	)
}

fn wire_value(value: &serde_json::Value) -> wire::Value {
	match value {
		serde_json::Value::Null => wire::Value::Null,
		serde_json::Value::Bool(flag) => wire::Value::Bool(*flag),
		serde_json::Value::Number(number) => {
			number.as_i64().map_or_else(
				|| wire::Value::Float(number.as_f64().unwrap_or(0.0)),
				wire::Value::Int,
			)
		}
		serde_json::Value::String(text) => wire::Value::Text(text.clone()),
		other => wire::Value::Text(other.to_string()),
	}
}

fn uuid_bytes(text: Option<&str>) -> Option<[u8; 16]> {
	text.and_then(|text| uuid::Uuid::parse_str(text).ok())
		.map(uuid::Uuid::into_bytes)
}

/// The bytes one event takes, typed and as JSON, from a real one.
pub(crate) fn weigh(sample: &EventRow, fields: Option<&[(String, String)]>) -> (usize, usize) {
	let properties = sample.properties.as_object().cloned().unwrap_or_default();
	let millis = format::instant(&sample.timestamp).map_or(0, |time| time.timestamp_millis());
	let body = match fields {
		Some(fields) => {
			let names: Vec<(&str, &str)> = fields
				.iter()
				.map(|(name, ty)| (name.as_str(), ty.as_str()))
				.collect();
			wire::Body::TypedEvent {
				name: sample.name.clone(),
				schema_id: leptoskit_analytics::schema_id(&sample.name, &names),
				values: fields
					.iter()
					.map(|(name, _)| properties.get(name).map_or(wire::Value::Null, wire_value))
					.collect(),
				properties: Vec::new(),
			}
		}
		None => {
			wire::Body::Event {
				name: sample.name.clone(),
				properties: properties
					.iter()
					.map(|(key, value)| (key.clone(), wire_value(value)))
					.collect(),
			}
		}
	};
	let batch = wire::Batch {
		sent_at: millis,
		contexts: vec![Vec::new()],
		entries: vec![wire::Entry {
			uuid: uuid_bytes(Some(&sample.uuid)).unwrap_or_else(|| wire::uuid_v7(millis, [0; 10])),
			distinct_id: sample.distinct_id.clone(),
			session_id: uuid_bytes(sample.session_id.as_deref()),
			context: 0,
			body,
		}],
	};
	let json = serde_json::json!({
		"event": sample.name,
		"distinct_id": sample.distinct_id,
		"uuid": sample.uuid,
		"timestamp": sample.timestamp,
		"session_id": sample.session_id,
		"properties": properties,
	});
	(
		wire::encode(&batch).len(),
		serde_json::to_vec(&json).map_or(0, |bytes| bytes.len()),
	)
}

fn send(
	event: &str,
	definition: Option<&EventDefinitionView>,
	sample: Option<&EventRow>,
) -> Station {
	let Some(sample) = sample else {
		return Station::new(
			"Send",
			"the wire",
			"What one event weighs is measured from a real one, and none has arrived yet.",
			Body::Missing(format!("Waiting for a `{event}` to weigh.")),
		);
	};
	let fields = definition.and_then(|definition| definition.fields.as_deref());
	let (binary, json) = weigh(sample, fields);
	#[expect(clippy::cast_precision_loss, reason = "a share for display")]
	let saving = 1.0 - binary as f64 / json.max(1) as f64;
	let label = if fields.is_some() {
		"typed, positional"
	} else {
		"binary, named"
	};
	let mut code = vec![
		format!("{label:<20} {binary:>5} bytes"),
		format!("{:<20} {json:>5} bytes", "JSON, named"),
	];

	if let Some(fields) = fields {
		let values: Vec<String> = fields
			.iter()
			.map(|(name, _)| {
				format::value(
					sample
						.properties
						.get(name)
						.unwrap_or(&serde_json::Value::Null),
				)
			})
			.collect();
		code.push(String::new());
		code.push(format!("// values in field order: {}", values.join(", ")));
	}

	Station::new(
		"Send",
		format!("the latest {event}, re-encoded here"),
		if fields.is_some() {
			format!(
				"Typed events send no property names, so this one is {binary} bytes instead of \
				 {json}: {} smaller. Measured as one event alone in a batch; a real batch shares \
				 its context and session between events, so it shrinks further.",
				format::share(saving)
			)
		} else {
			format!(
				"Without a type the names travel with every event: {binary} bytes in the binary \
				 format, {json} as JSON. Measured as one event alone in a batch."
			)
		},
		Body::Code(lines(code), "text".to_owned()),
	)
}

/// A table's name and its columns, each a name and an SQL type.
type Table = (String, Vec<(String, String)>);

/// Every table the service's migrations create, with its columns.
fn tables() -> &'static [Table] {
	static TABLES: OnceLock<Vec<Table>> = OnceLock::new();
	TABLES.get_or_init(|| {
		let mut tables = Vec::new();

		for migration in leptoskit_analytics::service::MIGRATIONS {
			let mut current: Option<Table> = None;

			for line in migration.sql.lines() {
				let line = line.trim();

				if let Some(rest) = line.strip_prefix("CREATE TABLE \"") {
					if let Some(done) = current.take() {
						tables.push(done);
					}
					current = rest
						.split('"')
						.next()
						.map(|name| (name.to_owned(), Vec::new()));
					continue;
				}

				let Some((_, columns)) = current.as_mut() else {
					continue;
				};

				if line.starts_with(')') {
					if let Some(done) = current.take() {
						tables.push(done);
					}
					continue;
				}

				if let Some(rest) = line.strip_prefix('"')
					&& let Some((name, rest)) = rest.split_once('"')
				{
					let kind = rest.split_whitespace().next().unwrap_or("").to_owned();
					columns.push((name.to_owned(), kind));
				}
			}

			if let Some(done) = current {
				tables.push(done);
			}
		}

		tables
	})
}

fn store(answer: Option<&Answer>) -> Station {
	let Some(answer) = answer else {
		return Station::new(
			"Store",
			"tables",
			"The tables come from the SQL a query ran, and this one didn't run.",
			Body::Missing("No SQL to read the tables from.".into()),
		);
	};
	let read: Vec<&(String, Vec<(String, String)>)> = tables()
		.iter()
		.filter(|(table, _)| {
			answer.info.statements.iter().any(|statement| {
				statement.sql.contains(&format!("FROM \"{table}\""))
					|| statement.sql.contains(&format!("JOIN \"{table}\""))
			})
		})
		.collect();
	let mut code = Vec::new();

	for (table, columns) in &read {
		if !code.is_empty() {
			code.push(String::new());
		}

		code.push(table.clone());

		for (name, kind) in columns {
			code.push(format!("  {name:<18} {kind}"));
		}
	}

	if code.is_empty() {
		code.push("-- the query read no tables by name".to_owned());
	}

	let names: Vec<&str> = read.iter().map(|(table, _)| table.as_str()).collect();
	Station::new(
		"Store",
		format!("{} on {}", names.join(", "), answer.info.dialect),
		"Events land in `events` as they arrive; sessions and persons are kept up to date beside \
		 them as each batch is stored, so a query reads tables, never a queue. These are the ones \
		 this query read.",
		Body::Code(lines(code), "text".to_owned()),
	)
}

fn ask(query: &Query, answer: Option<&Answer>, url: &str) -> Station {
	let sql = answer.map_or_else(
		|| vec!["-- the query didn't run, so there's no SQL".to_owned()],
		|answer| code::sql(&answer.info),
	);
	let timing = answer.map_or_else(String::new, |answer| {
		format!(
			" It ran {} statement{} on {} in {} ms.",
			answer.info.statements.len(),
			if answer.info.statements.len() == 1 {
				""
			} else {
				"s"
			},
			answer.info.dialect,
			answer.info.elapsed_ms
		)
	});
	Station::new(
		"Ask",
		"the query",
		format!(
			"Every insight is one value of the typed `Query` enum. The SQL is what it \
			 became.{timing} Change the builder and these lines change with it."
		),
		Body::Tabs(vec![
			Tab {
				name: "Rust".to_owned(),
				lines: lines(code::rust(query)),
				copy_as: "Rust".to_owned(),
			},
			Tab {
				name: "SQL".to_owned(),
				lines: lines(sql),
				copy_as: "SQL".to_owned(),
			},
			Tab {
				name: "HTTP".to_owned(),
				lines: lines(code::http(query, url)),
				copy_as: "shell".to_owned(),
			},
		]),
	)
}

fn draw(drawn: Drawn) -> Station {
	Station::new(
		"Draw",
		"rows → chart",
		drawn.sentence,
		if drawn.rows.is_empty() {
			Body::Missing("No rows came back.".into())
		} else {
			Body::Code(lines(drawn.rows), "text".to_owned())
		},
	)
}

/// A card's whole drawer.
pub(crate) fn drawer(source: Source<'_>) -> Drawer {
	let event = focus_event(source.query)
		.or_else(|| source.sample.map(|sample| sample.name.clone()))
		.unwrap_or_else(|| "$pageview".to_owned());
	let definition = source
		.definitions
		.iter()
		.find(|definition| definition.name == event);
	let sample = source.sample.filter(|sample| sample.name == event);
	Drawer {
		id: source.id.to_owned(),
		title: source.title,
		stations: vec![
			emit(&event, definition),
			send(&event, definition, sample),
			store(source.answer),
			ask(source.query, source.answer, source.url),
			draw(source.drawn),
		],
	}
}

/// Pads a table's cells into columns.
pub(crate) fn table(rows: &[Vec<String>]) -> Vec<String> {
	let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
	let widths: Vec<usize> = (0..columns)
		.map(|column| {
			rows.iter()
				.filter_map(|row| row.get(column))
				.map(|cell| cell.chars().count())
				.max()
				.unwrap_or(0)
		})
		.collect();
	rows.iter()
		.map(|row| {
			row.iter()
				.enumerate()
				.map(|(column, cell)| {
					if column == 0 {
						format!("{cell:<width$}", width = widths[column])
					} else {
						format!("{cell:>width$}", width = widths[column])
					}
				})
				.collect::<Vec<_>>()
				.join("  ")
				.trim_end()
				.to_owned()
		})
		.collect()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_migrations_name_the_event_columns() {
		let events = tables()
			.iter()
			.find(|(table, _)| table == "events")
			.expect("an events table");
		assert!(
			events
				.1
				.iter()
				.any(|(name, kind)| name == "properties" && kind == "TEXT")
		);
	}

	#[test]
	fn a_typed_event_weighs_less_than_its_json() {
		let sample = EventRow {
			uuid: uuid::Uuid::from_bytes(wire::uuid_v7(1_763_000_000_000, [7; 10])).to_string(),
			name: "checkout_started".into(),
			distinct_id: "ada@notes.example".into(),
			person_id: None,
			timestamp: "2026-11-13T10:00:00Z".into(),
			session_id: None,
			properties: serde_json::json!({ "plan": "pro", "seats": 3 }),
			source: None,
		};
		let fields = vec![
			("plan".to_owned(), "String".to_owned()),
			("seats".to_owned(), "u32".to_owned()),
		];
		let (typed, json) = weigh(&sample, Some(&fields));
		let (named, _) = weigh(&sample, None);
		assert!(typed < named, "{typed} vs {named}");
		assert!(named < json, "{named} vs {json}");
	}

	#[test]
	fn tables_line_up() {
		assert_eq!(
			table(&[
				vec!["day".into(), "count".into()],
				vec!["2026-11-12".into(), "8".into()],
			]),
			["day         count", "2026-11-12      8"]
		);
	}
}
