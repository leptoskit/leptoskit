//! The SQL fragments the planner composes, per dialect: how a JSON
//! property is read, how a condition is written, and how an instant
//! buckets into an hour, a day, a week, or a month.
//!
//! Everything a query's caller controls arrives as a bound parameter or is
//! checked against an allow-list before it reaches this file, so nothing
//! here builds SQL out of untrusted text: property names are validated by
//! [`checked_property`], session columns come from [`SESSION_COLUMNS`],
//! and every value is a `?N` or `$N`.

use leptoskit_db::Dialect;

use crate::query::Operator;

/// An operator's name, as the JSON holds it, for messages.
pub fn operator_name(operator: Operator) -> &'static str {
	match operator {
		Operator::Equals => "equals",
		Operator::NotEquals => "not_equals",
		Operator::Contains => "contains",
		Operator::Regex => "regex",
		Operator::GreaterThan => "greater_than",
		Operator::LessThan => "less_than",
		Operator::IsSet => "is_set",
		Operator::IsNotSet => "is_not_set",
	}
}

/// The most characters a property name may have.
pub const MAX_PROPERTY_BYTES: usize = 200;

/// The session columns a filter may name, and their kinds. Anything else
/// is refused, so a session filter can't reach another table.
pub const SESSION_COLUMNS: &[(&str, SessionColumn)] = &[
	("channel", SessionColumn::Text),
	("referrer_domain", SessionColumn::Text),
	("utm_source", SessionColumn::Text),
	("utm_medium", SessionColumn::Text),
	("utm_campaign", SessionColumn::Text),
	("device_type", SessionColumn::Text),
	("browser", SessionColumn::Text),
	("os", SessionColumn::Text),
	("entry_pathname", SessionColumn::Text),
	("exit_pathname", SessionColumn::Text),
	("entry_url", SessionColumn::Text),
	("exit_url", SessionColumn::Text),
	("duration_ms", SessionColumn::Number),
	("event_count", SessionColumn::Number),
	("pageview_count", SessionColumn::Number),
	("is_bounce", SessionColumn::Bool),
];

/// What a session column holds. Sessions use the empty string for a
/// missing text column, and never store null.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionColumn {
	Text,
	Number,
	Bool,
}

/// Validates a property name, and answers the JSON path or the key that
/// reads it, bound as a parameter: SQLite takes a path such as
/// `$."plan"`, Postgres takes the bare key. A name with a control
/// character, or over [`MAX_PROPERTY_BYTES`], is refused; everything else
/// is safe, because the name travels as a value, not as SQL.
///
/// # Errors
///
/// [`PlanError::Property`](super::PlanError::Property) naming what's wrong.
pub fn checked_property(key: &str) -> Result<PropertyKey, PlanError> {
	if key.is_empty() {
		return Err(PlanError::Property(
			"a property's name can't be empty".into(),
		));
	}
	if key.len() > MAX_PROPERTY_BYTES {
		return Err(PlanError::Property(format!(
			"a property's name is at most {MAX_PROPERTY_BYTES} bytes"
		)));
	}
	if key.bytes().any(|byte| byte < 0x20) {
		return Err(PlanError::Property(format!(
			"the property name {key:?} has a control character in it"
		)));
	}
	Ok(PropertyKey {
		path: format!(r#"$."{}""#, key.replace('"', "\"\"")),
		key: key.to_owned(),
	})
}

/// A property name, as each dialect reads it.
#[derive(Clone, Debug)]
pub struct PropertyKey {
	/// The SQLite JSON path, bound as a parameter.
	pub path: String,
	/// The bare key, bound as a parameter on Postgres.
	pub key: String,
}

/// Why a query couldn't be planned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlanError {
	/// A property name a query can't safely or sensibly read.
	Property(String),
	/// A session column that isn't one of [`SESSION_COLUMNS`].
	Column(String),
	/// A combination the engine doesn't support, with the reason.
	Unsupported(String),
	/// A range or interval that doesn't hold together.
	Range(String),
}

impl std::fmt::Display for PlanError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Property(why) => write!(f, "the property can't be read: {why}"),
			Self::Column(what) => write!(f, "the session column can't be filtered: {what}"),
			Self::Unsupported(why) => write!(f, "not supported: {why}"),
			Self::Range(why) => write!(f, "the range doesn't hold together: {why}"),
		}
	}
}

impl std::error::Error for PlanError {}

/// The SQL fragments for one dialect.
pub struct DialectSql {
	pub dialect: Dialect,
}

impl DialectSql {
	/// Fragments for `dialect`.
	#[must_use]
	pub fn new(dialect: Dialect) -> Self {
		Self { dialect }
	}

	/// The `n`th placeholder.
	#[must_use]
	pub fn placeholder(&self, n: usize) -> String {
		match self.dialect {
			Dialect::Postgres => format!("${n}"),
			Dialect::Sqlite => format!("?{n}"),
		}
	}

	/// Reads one property of a JSON text column as text, or SQL `NULL`
	/// when it's absent: SQLite's `json_extract` of the bound path,
	/// Postgres's `#>>` of the bound key. `<column>` must already be a
	/// trusted fragment, such as `"events"."properties"`.
	///
	/// `at` is the placeholder number of the property's name; the caller
	/// pushes the argument at the same index.
	#[must_use]
	pub fn property_text(&self, column: &str, at: usize) -> String {
		match self.dialect {
			Dialect::Postgres => {
				format!(
					"({column}::jsonb #>> ARRAY[{at_scalar}::text])",
					at_scalar = self.placeholder(at)
				)
			}
			Dialect::Sqlite => format!("json_extract({column}, {})", self.placeholder(at)),
		}
	}

	/// Reads one property of a JSON text column, keeping its JSON type:
	/// SQLite returns what `json_extract` returns (numbers as numbers,
	/// `true` as `1`); Postgres returns `jsonb`, compared with
	/// `IS DISTINCT FROM` a bound `jsonb`. Used for `equals`.
	#[must_use]
	pub fn property_value(&self, column: &str, at: usize) -> String {
		match self.dialect {
			Dialect::Postgres => format!("({column}::jsonb -> {})", self.placeholder(at)),
			Dialect::Sqlite => format!("json_extract({column}, {})", self.placeholder(at)),
		}
	}

	/// Writes one condition on a JSON property's text, with `value`'s
	/// placeholder at `value_at` and the property's name's at `name_at`.
	/// The caller binds the name first, then the value, in that order.
	///
	/// # Errors
	///
	/// [`PlanError::Unsupported`] when the operator needs what the dialect
	/// doesn't have, such as regular expressions on SQLite.
	pub fn condition(
		&self,
		operator: Operator,
		column: &str,
		name_at: usize,
		value_at: usize,
	) -> Result<String, PlanError> {
		let sql = self;
		let text = sql.property_text(column, name_at);
		match operator {
			Operator::Equals => {
				Ok(format!(
					"({}) = {}",
					sql.property_value(column, name_at),
					sql.equals_operand(value_at)
				))
			}
			Operator::NotEquals => {
				Ok(format!(
					"({}) IS NOT {}",
					sql.property_value(column, name_at),
					sql.equals_operand(value_at)
				))
			}
			Operator::Contains => {
				Ok(match self.dialect {
					Dialect::Postgres => {
						format!(
							"strpos(lower({text}), lower({})) > 0",
							sql.placeholder(value_at),
							text = text
						)
					}
					Dialect::Sqlite => {
						format!(
							"instr(lower({text}), lower({})) > 0",
							sql.placeholder(value_at),
							text = text
						)
					}
				})
			}
			Operator::Regex => {
				match sql.dialect {
					Dialect::Postgres => Ok(format!("({text}) ~ {}", sql.placeholder(value_at))),
					Dialect::Sqlite => {
						Err(PlanError::Unsupported(
							"regular expressions need Postgres; SQLite has none to offer".into(),
						))
					}
				}
			}
			Operator::GreaterThan | Operator::LessThan => {
				let comparison = if operator == Operator::GreaterThan {
					">"
				} else {
					"<"
				};
				Ok(match sql.dialect {
					Dialect::Postgres => {
						format!(
							"(({text})::numeric {comparison} {}::numeric)",
							sql.placeholder(value_at),
							text = text
						)
					}
					Dialect::Sqlite => {
						format!(
							"(CAST({text} AS NUMERIC) {comparison} {})",
							sql.placeholder(value_at),
							text = text
						)
					}
				})
			}
			Operator::IsSet => Ok(format!("({text}) IS NOT NULL")),
			Operator::IsNotSet => Ok(format!("({text}) IS NULL")),
		}
	}

	/// The right-hand side of an `equals` or `IS NOT`: on Postgres the
	/// bound value cast to `jsonb`, which compares with `->`'s `jsonb`
	/// exactly, type and all; on SQLite the bound value itself, which
	/// `json_extract` matches the same way.
	#[must_use]
	fn equals_operand(&self, value_at: usize) -> String {
		match self.dialect {
			Dialect::Postgres => format!("{}::jsonb", self.placeholder(value_at)),
			Dialect::Sqlite => self.placeholder(value_at),
		}
	}

	/// Writes one condition on a session column, whose name is already one
	/// of [`SESSION_COLUMNS`]. `value_at` is the value's placeholder.
	///
	/// # Errors
	///
	/// [`PlanError::Column`] when the operator doesn't fit the column's
	/// kind, such as `contains` on a number.
	pub fn session_condition(
		&self,
		column: &str,
		kind: SessionColumn,
		operator: Operator,
		value_at: usize,
	) -> Result<String, PlanError> {
		let placeholder = self.placeholder(value_at);
		let at = format!(r#""s"."{column}""#);
		match operator {
			Operator::Equals | Operator::NotEquals => {
				let comparison = if operator == Operator::Equals {
					"="
				} else {
					"<>"
				};
				Ok(match kind {
					SessionColumn::Text | SessionColumn::Number => {
						format!("{at} {comparison} {placeholder}")
					}
					SessionColumn::Bool => format!("{at} {comparison} ({placeholder})"),
				})
			}
			Operator::Contains => {
				match kind {
					SessionColumn::Text => {
						Ok(format!("instr(lower({at}), lower({placeholder})) > 0"))
					}
					_ => {
						Err(PlanError::Column(format!(
							"`contains` filters a text column, and `{column}` holds {}",
							match kind {
								SessionColumn::Number => "a number",
								SessionColumn::Bool => "a boolean",
								SessionColumn::Text => "text",
							}
						)))
					}
				}
			}
			Operator::Regex => {
				Err(PlanError::Column(
					"session columns don't take regular expressions; use `contains`".into(),
				))
			}
			Operator::GreaterThan | Operator::LessThan => {
				let comparison = if operator == Operator::GreaterThan {
					">"
				} else {
					"<"
				};
				match kind {
					SessionColumn::Number => Ok(format!("{at} {comparison} {placeholder}")),
					SessionColumn::Bool => Ok(format!("{at} = ({placeholder})")),
					SessionColumn::Text => {
						Err(PlanError::Column(format!(
							"`{column}` is text: order it with `equals` or `contains`"
						)))
					}
				}
			}
			Operator::IsSet | Operator::IsNotSet => {
				if kind != SessionColumn::Text {
					return Err(PlanError::Column(format!(
						"`{column}` is never missing, so `is_set` and `is_not_set` have nothing \
						 to ask it"
					)));
				}
				let set = if operator == Operator::IsSet {
					format!("{at} <> ''")
				} else {
					format!("{at} = ''")
				};
				Ok(set)
			}
		}
	}

	/// The bound form of a filter's value, for the dialect and operator.
	/// Equals and not-equals compare JSON values exactly, so Postgres
	/// takes the value as its JSON text and casts to `jsonb`; SQLite's
	/// `json_extract` already returns the value typed, so the value binds
	/// as itself. Everything else compares text or numbers.
	///
	/// # Errors
	///
	/// [`PlanError::Unsupported`] when the operator needs a value kind the
	/// filter doesn't carry.
	pub fn condition_value(
		&self,
		operator: Operator,
		value: Option<&serde_json::Value>,
	) -> Result<Vec<leptoskit_db::Value>, PlanError> {
		use leptoskit_db::Value;
		match operator {
			Operator::IsSet | Operator::IsNotSet => Ok(Vec::new()),
			Operator::Equals | Operator::NotEquals => {
				let Some(value) = value else {
					return Err(PlanError::Unsupported(format!(
						"`{}` names a value to compare",
						operator_name(operator)
					)));
				};
				Ok(vec![match self.dialect {
					Dialect::Postgres => Value::Text(value.to_string()),
					Dialect::Sqlite => {
						match value {
							serde_json::Value::Bool(bool) => Value::Bool(*bool),
							serde_json::Value::Number(number) => {
								number.as_i64().map_or_else(
									|| Value::Float(number.as_f64().unwrap_or_default()),
									Value::Int,
								)
							}
							serde_json::Value::String(text) => Value::Text(text.clone()),
							other => Value::Text(other.to_string()),
						}
					}
				}])
			}
			Operator::Contains | Operator::Regex => {
				let Some(value) = value.and_then(|value| value.as_str()) else {
					return Err(PlanError::Unsupported(format!(
						"`{}` needs text to match",
						operator_name(operator)
					)));
				};
				Ok(vec![Value::Text(value.to_owned())])
			}
			Operator::GreaterThan | Operator::LessThan => {
				let Some(number) = value.and_then(serde_json::Value::as_f64) else {
					return Err(PlanError::Unsupported(format!(
						"`{}` needs a number",
						operator_name(operator)
					)));
				};
				Ok(vec![Value::Float(number)])
			}
		}
	}

	/// How wide an interval is, in seconds, in wall-clock terms. A day is
	/// 86,400, a week 604,800: DST is a fixed offset's problem, not the
	/// engine's.
	#[must_use]
	pub fn interval_seconds(interval: crate::query::Interval) -> i64 {
		match interval {
			crate::query::Interval::Hour => 3_600,
			crate::query::Interval::Day => 86_400,
			crate::query::Interval::Week => 604_800,
			crate::query::Interval::Month => 2_629_800,
		}
	}

	/// The expression that buckets the events table's `timestamp` into an
	/// interval, as wall-clock text in `offset_seconds` east of UTC, such
	/// as `2026-11-14T00:00:00`. `shift_at` is the placeholder of the
	/// shift, bound as `"0 seconds"` or `"7200 seconds"`; the caller binds
	/// it.
	///
	/// # Errors
	///
	/// Nothing today; kept fallible so a new interval can refuse cleanly.
	pub fn bucket(
		&self,
		interval: crate::query::Interval,
		shift_at: usize,
	) -> Result<String, PlanError> {
		let shift = self.placeholder(shift_at);
		Ok(match self.dialect {
			Dialect::Postgres => {
				let part = match interval {
					crate::query::Interval::Hour => "hour",
					crate::query::Interval::Day => "day",
					crate::query::Interval::Week => "week",
					crate::query::Interval::Month => "month",
				};
				format!(
					"to_char(date_trunc('{part}', \"events\".\"timestamp\" + {shift}::interval), \
					 'YYYY-MM-DD\"T\"HH24:MI:SS')"
				)
			}
			Dialect::Sqlite => {
				let format = match interval {
					crate::query::Interval::Hour => "%Y-%m-%dT%H:00:00",
					crate::query::Interval::Month => "%Y-%m-01T00:00:00",
					// A day and a week start at the same midnight; the
					// week's Monday shift rides below.
					_ => "%Y-%m-%dT00:00:00",
				};
				// The Monday that starts the week: step back six days,
				// then forward to the next-or-same Monday, which lands on
				// the week's own Monday whatever day it was.
				let week = if interval == crate::query::Interval::Week {
					", '-6 days', 'weekday 1'"
				} else {
					""
				};
				format!("strftime('{format}', \"events\".\"timestamp\", {shift}{week})")
			}
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn property_names_travel_as_values_or_are_refused() {
		let key = checked_property(r"plan'); DROP TABLE events;--").expect("safe: bound");
		assert_eq!(key.key, r"plan'); DROP TABLE events;--");
		assert!(key.path.contains(r#""plan'); DROP TABLE events;--""#));

		assert!(checked_property("").is_err());
		assert!(checked_property("a\x00b").is_err());
		let long = "x".repeat(MAX_PROPERTY_BYTES + 1);
		assert!(checked_property(&long).is_err());
	}

	#[test]
	fn a_condition_is_written_with_placeholders_only() {
		let sql = DialectSql::new(Dialect::Sqlite);
		let key = checked_property("plan").unwrap();
		let _ = key;
		let condition = sql
			.condition(Operator::Contains, r#""events"."properties""#, 3, 4)
			.unwrap();
		assert!(condition.contains("?3"));
		assert!(condition.contains("?4"));
		assert!(!condition.contains("plan"));

		let postgres = DialectSql::new(Dialect::Postgres);
		let condition = postgres
			.condition(Operator::Equals, r#""events"."properties""#, 1, 2)
			.unwrap();
		assert!(condition.contains("$1"));
		assert!(condition.contains("$2::jsonb"));

		assert!(postgres.condition(Operator::Regex, "", 1, 2).is_ok());
		assert!(sql.condition(Operator::Regex, "", 1, 2).is_err());
	}

	#[test]
	fn session_filters_refuse_what_cannot_match() {
		let sql = DialectSql::new(Dialect::Sqlite);
		assert!(
			sql.session_condition("browser", SessionColumn::Text, Operator::Contains, 1)
				.is_ok()
		);
		assert!(
			sql.session_condition("event_count", SessionColumn::Number, Operator::Contains, 1)
				.is_err()
		);
		assert!(
			sql.session_condition(
				"duration_ms",
				SessionColumn::Number,
				Operator::GreaterThan,
				1
			)
			.is_ok()
		);
		assert!(
			sql.session_condition("browser", SessionColumn::Text, Operator::GreaterThan, 1)
				.is_err()
		);
		assert!(
			sql.session_condition("channel", SessionColumn::Text, Operator::IsSet, 1)
				.is_ok()
		);
		assert!(
			sql.session_condition("event_count", SessionColumn::Number, Operator::IsSet, 1)
				.is_err()
		);
	}

	#[test]
	fn buckets_are_wall_clock_text() {
		let sql = DialectSql::new(Dialect::Sqlite);
		let day = sql.bucket(crate::query::Interval::Day, 2).unwrap();
		assert!(day.contains("?2"));
		assert!(day.contains("%Y-%m-%dT00:00:00"));
		let week = sql.bucket(crate::query::Interval::Week, 2).unwrap();
		assert!(week.contains("weekday 1"));
		let postgres = DialectSql::new(Dialect::Postgres);
		let hour = postgres.bucket(crate::query::Interval::Hour, 1).unwrap();
		assert!(hour.contains("date_trunc('hour'"));
		assert!(hour.contains("$1::interval"));
	}
}
