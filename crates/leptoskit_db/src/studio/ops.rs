//! What the studio does to one table, whatever its Rust type: each table
//! registers these functions, instantiated for it, so every read and write
//! goes through the table's own queries, policies, hooks, and audit log.

use std::future::Future;
use std::pin::Pin;

use leptoskit_db_schema::ColumnType;
use leptoskit_db_schema::Dialect;

use crate::Action;
use crate::Db;
use crate::DbError;
use crate::Expr;
use crate::Order;
use crate::Query;
use crate::Result;
use crate::Statement;
use crate::Table;
use crate::Value;
use crate::expr::Comparison;
use crate::policy::Access;
use crate::sql::Sql;

/// A future the studio's handlers can hold across awaits.
pub(crate) type Boxed<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// A future that holds a row across awaits, which needn't be `Send`: the
/// studio runs it on a thread of its own.
pub(crate) type Local<T> = Pin<Box<dyn Future<Output = Result<T>>>>;

/// What a viewer may do to a table, action by action: the statement the
/// database layer adds, or `None` when nothing restricts it.
pub(crate) type Rules = Vec<(Action, Option<Statement>)>;

/// What the studio browses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
	Table,
	View,
	/// A view that keeps its rows until refreshed.
	Materialized,
}

impl Kind {
	pub(crate) fn label(self) -> &'static str {
		match self {
			Self::Table => "table",
			Self::View => "view",
			Self::Materialized => "materialized view",
		}
	}
}

/// One table's or view's operations, as plain function pointers.
#[derive(Clone, Copy)]
pub(crate) struct TableOps {
	pub(crate) kind: Kind,
	/// A page of rows, with `private` values shown only when the second
	/// argument, `--reveal`, is set.
	pub(crate) read: for<'a> fn(&'a Db, &'a Browse, bool) -> Boxed<'a, Rows>,
	pub(crate) rules: fn(&Access, Dialect) -> Result<Rules>,
	/// The `private` columns, which the audit log's page hides too.
	pub(crate) private: &'static [&'static str],
	/// How it's written to; a view never is.
	pub(crate) writes: Option<Writes>,
}

/// How the studio writes to a table.
#[derive(Clone, Copy)]
pub(crate) struct Writes {
	pub(crate) edit: fn(Db, Edit) -> Local<()>,
	pub(crate) remove: fn(Db, Vec<String>) -> Local<()>,
	pub(crate) insert: fn(Db, Insert) -> Local<Vec<String>>,
	/// The columns an insert provides, which the insert form asks for.
	pub(crate) provided: fn() -> Vec<leptoskit_db_schema::Column>,
}

impl TableOps {
	pub(crate) const fn of<T: Table>() -> Self {
		Self {
			kind: Kind::Table,
			read: read::<T>,
			rules: rules::<T>,
			private: T::PRIVATE,
			writes: Some(Writes {
				edit: edit::<T>,
				remove: remove::<T>,
				insert: insert::<T>,
				provided: provided::<T>,
			}),
		}
	}

	/// A view's: it's read like a table, through its policies, and never
	/// written to.
	pub(crate) const fn view<V: crate::View>() -> Self {
		Self {
			kind: if V::MATERIALIZED {
				Kind::Materialized
			} else {
				Kind::View
			},
			read: read::<V>,
			rules: rules::<V>,
			private: V::PRIVATE,
			writes: None,
		}
	}
}

/// A comparison a filter makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Op {
	Equal,
	NotEqual,
	Greater,
	Less,
	Contains,
	IsNull,
	IsNotNull,
}

impl Op {
	pub(crate) const ALL: [Self; 7] = [
		Self::Equal,
		Self::NotEqual,
		Self::Greater,
		Self::Less,
		Self::Contains,
		Self::IsNull,
		Self::IsNotNull,
	];

	pub(crate) fn name(self) -> &'static str {
		match self {
			Self::Equal => "eq",
			Self::NotEqual => "ne",
			Self::Greater => "gt",
			Self::Less => "lt",
			Self::Contains => "contains",
			Self::IsNull => "null",
			Self::IsNotNull => "not_null",
		}
	}

	pub(crate) fn label(self) -> &'static str {
		match self {
			Self::Equal => "=",
			Self::NotEqual => "≠",
			Self::Greater => ">",
			Self::Less => "<",
			Self::Contains => "contains",
			Self::IsNull => "is null",
			Self::IsNotNull => "is not null",
		}
	}

	pub(crate) fn parse(name: &str) -> Option<Self> {
		Self::ALL.into_iter().find(|op| op.name() == name)
	}
}

/// One filter on a column.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Filter {
	pub(crate) column: String,
	pub(crate) op: Op,
	pub(crate) value: String,
}

/// What a table page asks for.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Browse {
	pub(crate) filter: Option<Filter>,
	pub(crate) order: Option<(String, bool)>,
	pub(crate) limit: u64,
	pub(crate) offset: u64,
	pub(crate) with_deleted: bool,
}

/// A page of rows, as text, and how many rows match in all.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Rows {
	pub(crate) columns: Vec<&'static str>,
	pub(crate) key: Vec<usize>,
	/// The columns whose values the page hides, which it doesn't filter or
	/// sort by either.
	pub(crate) hidden: Vec<usize>,
	pub(crate) cells: Vec<Vec<Cell>>,
	pub(crate) total: u64,
}

/// One value, as the studio shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Cell {
	Null,
	Secret,
	/// A `private` value, while the studio doesn't reveal them.
	Private,
	Text(String),
}

impl Cell {
	pub(crate) fn text(&self) -> &str {
		match self {
			Self::Null => "NULL",
			Self::Secret => "‹secret›",
			Self::Private => "‹private›",
			Self::Text(text) => text,
		}
	}

	/// Whether the page hides the value.
	pub(crate) fn hidden(&self) -> bool {
		matches!(self, Self::Secret | Self::Private)
	}
}

/// A change to one column of one row.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Edit {
	/// The row's key, one text per key column.
	pub(crate) key: Vec<String>,
	pub(crate) column: String,
	/// The new value, or `None` for `NULL`.
	pub(crate) value: Option<String>,
}

/// A new row, as the insert form sends it: each column an insert provides,
/// with its text, or `None` for `NULL`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Insert {
	pub(crate) values: Vec<(String, Option<String>)>,
}

fn invalid(message: impl Into<String>) -> DbError {
	crate::expr::invalid(&message.into())
}

/// The table's own name for a column, or an error naming the table's
/// columns.
fn column_of<T: Table>(name: &str) -> Result<&'static str> {
	T::COLUMNS
		.iter()
		.copied()
		.find(|column| *column == name)
		.ok_or_else(|| {
			invalid(format!(
				"`{}` has no column `{name}`; it has {}",
				T::flat_name(),
				T::COLUMNS.join(", ")
			))
		})
}

/// Whether a page hides `column` of `T`: a `secret` one always, a
/// `private` one unless the studio reveals them.
fn hides<T: Table>(column: &str, reveal: bool) -> bool {
	T::SECRET.contains(&column) || (!reveal && T::PRIVATE.contains(&column))
}

/// The column a filter or ordering names, when the page shows it: matching
/// or sorting a hidden column's values would tell what they are.
fn shown_column<T: Table>(name: &str, reveal: bool) -> Result<&'static str> {
	let column = column_of::<T>(name)?;

	if T::SECRET.contains(&column) {
		Err(invalid(format!(
			"`{column}` is secret, so the studio never filters or sorts by it"
		)))
	} else if hides::<T>(column, reveal) {
		Err(invalid(format!(
			"`{column}` is private: start the studio with `--reveal` to filter or sort by it"
		)))
	} else {
		Ok(column)
	}
}

/// A column's type and whether it takes `NULL`.
fn column_type<T: Table>(column: &str) -> Result<(ColumnType, bool)> {
	T::definition()
		.columns
		.into_iter()
		.find(|candidate| candidate.name == column)
		.map(|column| (column.ty, column.nullable))
		.ok_or_else(|| invalid(format!("`{}` has no column `{column}`", T::flat_name())))
}

/// The value a form gives `column` of `T`: its text parsed for the column's
/// type, or `NULL` where the column takes it.
fn form_value<T: Table>(column: &str, text: Option<&str>) -> Result<Value> {
	let (ty, nullable) = column_type::<T>(column)?;

	match text {
		None if nullable => Ok(Value::Null),
		None => Err(invalid(format!("`{column}` can't be NULL"))),
		Some(text) => parse_value(&ty, text).map_err(invalid),
	}
}

/// The value `text` means in a column of type `ty`.
pub(crate) fn parse_value(ty: &ColumnType, text: &str) -> std::result::Result<Value, String> {
	let integer = |min: i64, max: i64| {
		text.trim()
			.parse::<i64>()
			.map_err(|error| format!("`{text}` isn't a whole number: {error}"))
			.and_then(|number| {
				if (min..=max).contains(&number) {
					Ok(Value::Int(number))
				} else {
					Err(format!("{number} is outside {min} to {max}"))
				}
			})
	};

	match ty {
		ColumnType::Bool => {
			match text.trim() {
				"true" => Ok(Value::Bool(true)),
				"false" => Ok(Value::Bool(false)),
				other => Err(format!("`{other}` isn't `true` or `false`")),
			}
		}
		ColumnType::Int16 => integer(i16::MIN.into(), i16::MAX.into()),
		ColumnType::Int32 => integer(i32::MIN.into(), i32::MAX.into()),
		ColumnType::Int64 => integer(i64::MIN, i64::MAX),
		ColumnType::Float32 | ColumnType::Float64 => {
			text.trim()
				.parse::<f64>()
				.map(Value::Float)
				.map_err(|error| format!("`{text}` isn't a number: {error}"))
		}
		ColumnType::Text { .. } => Ok(Value::Text(text.to_string())),
		ColumnType::Uuid => {
			uuid::Uuid::parse_str(text.trim())
				.map(Value::Uuid)
				.map_err(|error| format!("`{text}` isn't a UUID: {error}"))
		}
		ColumnType::Timestamp => {
			chrono::DateTime::parse_from_rfc3339(text.trim())
				.map(|instant| Value::Timestamp(instant.to_utc()))
				.map_err(|error| format!("`{text}` isn't an RFC 3339 instant: {error}"))
		}
		ColumnType::Date => {
			text.trim()
				.parse()
				.map(Value::Date)
				.map_err(|error| format!("`{text}` isn't a date, such as 2026-10-30: {error}"))
		}
		ColumnType::Json => {
			serde_json::from_str(text)
				.map(Value::Json)
				.map_err(|error| format!("that isn't JSON: {error}"))
		}
		other => {
			Err(format!(
				"the studio can't read or write `{other:?}` values yet"
			))
		}
	}
}

/// How the studio shows a value.
pub(crate) fn display(value: &Value) -> Cell {
	match value {
		Value::Null => Cell::Null,
		Value::Bool(value) => Cell::Text(value.to_string()),
		Value::Int(value) => Cell::Text(value.to_string()),
		Value::Float(value) => Cell::Text(value.to_string()),
		Value::Text(text) => Cell::Text(text.clone()),
		Value::Bytes(bytes) => {
			// SQLite stores UUIDs as 16 bytes.
			match uuid::Uuid::from_slice(bytes) {
				Ok(uuid) if bytes.len() == 16 => Cell::Text(uuid.to_string()),
				_ => Cell::Text(format!("{} bytes", bytes.len())),
			}
		}
		Value::Uuid(uuid) => Cell::Text(uuid.to_string()),
		Value::Timestamp(instant) => Cell::Text(instant.to_rfc3339()),
		Value::Date(date) => Cell::Text(date.to_string()),
		Value::Json(json) => Cell::Text(json.to_string()),
	}
}

/// The condition a filter makes on `T`.
fn filter_expr<T: Table>(filter: &Filter, reveal: bool) -> Result<Expr> {
	let column = shown_column::<T>(&filter.column, reveal)?;

	let comparison = match filter.op {
		Op::IsNull | Op::IsNotNull => {
			return Ok(Expr::Null {
				column,
				negated: filter.op == Op::IsNotNull,
			});
		}
		Op::Contains => {
			return Ok(Expr::Contains {
				column,
				needle: filter.value.clone(),
			});
		}
		Op::Equal => Comparison::Equal,
		Op::NotEqual => Comparison::NotEqual,
		Op::Greater => Comparison::Greater,
		Op::Less => Comparison::Less,
	};
	let (ty, _) = column_type::<T>(column)?;
	let value = parse_value(&ty, &filter.value).map_err(invalid)?;

	Ok(Expr::Compare {
		column,
		comparison,
		value,
	})
}

/// The query a page reads: its filter, and soft-deleted rows when asked.
fn browse_query<T: Table>(browse: &Browse, reveal: bool) -> Result<Query<T>> {
	let mut query = Query::<T>::new();

	if let Some(filter) = &browse.filter {
		let condition = filter_expr::<T>(filter, reveal)?;
		query = query.filter(|_| condition);
	}

	if browse.with_deleted {
		query = query.with_deleted();
	}

	Ok(query)
}

/// A page of `T`'s rows as `db` may read them, `secret` columns hidden,
/// and `private` ones unless `reveal`.
fn read<'a, T: Table>(db: &'a Db, browse: &'a Browse, reveal: bool) -> Boxed<'a, Rows> {
	Box::pin(async move {
		let total = browse_query::<T>(browse, reveal)?.count(db).await?;
		let mut query = browse_query::<T>(browse, reveal)?;
		// A view has no key, so it's ordered by its first column.
		let order = match &browse.order {
			Some((column, descending)) => {
				Some(Order::by(shown_column::<T>(column, reveal)?, *descending))
			}
			None => {
				T::KEY
					.first()
					.or(T::COLUMNS.first())
					.map(|column| Order::by(column, false))
			}
		};

		if let Some(order) = order {
			query = query.order_by(|_| order);
		}

		query = query.limit(browse.limit).offset(browse.offset);

		let cells = query
			.all(db)
			.await?
			.into_iter()
			.map(|row| {
				Ok(T::COLUMNS
					.iter()
					.zip(row.to_values()?)
					.map(|(column, value)| {
						if T::SECRET.contains(column) {
							Cell::Secret
						} else if hides::<T>(column, reveal) {
							Cell::Private
						} else {
							display(&value)
						}
					})
					.collect())
			})
			.collect::<Result<Vec<Vec<Cell>>>>()?;
		let key = T::KEY
			.iter()
			.map(|key| {
				T::COLUMNS
					.iter()
					.position(|column| column == key)
					.expect("a key is a column")
			})
			.collect();

		let hidden = T::COLUMNS
			.iter()
			.enumerate()
			.filter(|(_, column)| hides::<T>(column, reveal))
			.map(|(position, _)| position)
			.collect();

		Ok(Rows {
			columns: T::COLUMNS.to_vec(),
			key,
			hidden,
			cells,
			total,
		})
	})
}

/// The row of `T` with this key, as `db` may read it, soft-deleted or not.
async fn find<T: Table>(db: &Db, key: &[String]) -> Result<T> {
	if key.len() != T::KEY.len() {
		return Err(invalid(format!(
			"`{}` is keyed by {}",
			T::flat_name(),
			T::KEY.join(", ")
		)));
	}

	let mut condition = Expr::always();

	for (column, text) in T::KEY.iter().zip(key) {
		let (ty, _) = column_type::<T>(column)?;
		condition = condition
			& Expr::Compare {
				column,
				comparison: Comparison::Equal,
				value: parse_value(&ty, text).map_err(invalid)?,
			};
	}

	Query::<T>::new()
		.filter(|_| condition)
		.with_deleted()
		.one(db)
		.await
}

/// Changes one column of one row through `save`, with the table's hooks,
/// rules, policies, and audit log.
fn edit<T: Table>(db: Db, edit: Edit) -> Local<()> {
	Box::pin(async move {
		let column = column_of::<T>(&edit.column)?;

		if T::KEY.contains(&column) || T::GENERATED.contains(&column) {
			return Err(invalid(format!(
				"`{column}` can't change: it's part of the key or computed by the database"
			)));
		}

		let value = form_value::<T>(column, edit.value.as_deref())?;
		let row = find::<T>(&db, &edit.key).await?;
		let mut values = row.to_values()?;
		let position = T::COLUMNS
			.iter()
			.position(|candidate| *candidate == column)
			.expect("the column is the table's");
		values[position] = value;
		let mut row = T::from_values(values)?;
		crate::write::save(&db, &mut row).await
	})
}

/// Inserts a row through `insert`, with the table's hooks, rules, policies,
/// and audit log, from the columns an insert provides; the rest are filled
/// in as the table's `New…` type fills them. Returns the new row's key.
fn insert<T: Table>(db: Db, insert: Insert) -> Local<Vec<String>> {
	Box::pin(async move {
		let values = T::PROVIDED
			.iter()
			.map(|column| {
				let (_, text) = insert
					.values
					.iter()
					.find(|(name, _)| name == column)
					.ok_or_else(|| invalid(format!("the form has no value for `{column}`")))?;
				form_value::<T>(column, text.as_deref())
			})
			.collect::<Result<Vec<Value>>>()?;
		let stored = crate::write::insert(&db, T::from_provided(values)?).await?;
		let values = stored.to_values()?;
		Ok(T::KEY
			.iter()
			.filter_map(|key| T::COLUMNS.iter().position(|column| column == key))
			.map(|position| display(&values[position]).text().to_string())
			.collect())
	})
}

/// The columns an insert into `T` provides, in order.
fn provided<T: Table>() -> Vec<leptoskit_db_schema::Column> {
	let columns = T::definition().columns;
	T::PROVIDED
		.iter()
		.filter_map(|name| columns.iter().find(|column| column.name == *name).cloned())
		.collect()
}

/// Deletes one row through `delete`: soft-deletes it when the table can.
fn remove<T: Table>(db: Db, key: Vec<String>) -> Local<()> {
	Box::pin(async move {
		let mut row = find::<T>(&db, &key).await?;
		crate::write::delete(&db, &mut row).await
	})
}

/// What `access` may do to `T`'s rows, action by action, as SQL with its
/// values bound: `None` when nothing restricts it.
fn rules<T: Table>(access: &Access, dialect: Dialect) -> Result<Rules> {
	Action::ALL
		.iter()
		// Nothing writes to a view, so only its reads have rules.
		.filter(|action| !T::READ_ONLY || **action == Action::Select)
		.map(|action| {
			let statement = crate::policy::condition::<T>(access, *action)
				.map(|condition| {
					let mut sql = Sql::new(dialect);
					sql.push("SELECT * FROM ").table::<T>().push(" WHERE ");
					condition.write(&mut sql)?;
					Ok::<_, DbError>(sql.finish())
				})
				.transpose()?;
			Ok((*action, statement))
		})
		.collect()
}
