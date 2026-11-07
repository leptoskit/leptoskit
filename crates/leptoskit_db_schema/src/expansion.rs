//! Structural expansion checks, distinct from data-loss classification.

use crate::Column;
use crate::ColumnType;
use crate::Dialect;
use crate::RequireHints;
use crate::Schema;
use crate::Step;

/// Why a generated migration cannot participate in an expansion window.
#[derive(Debug, thiserror::Error)]
pub enum ExpansionError {
	/// A snapshot or its relation to the next snapshot is invalid.
	#[error("invalid expansion snapshots: {0}")]
	Schema(String),
	/// The change needs a separately reviewed rollout.
	#[error("not a supported online expansion: {0}")]
	Unsupported(String),
	/// The SQL is not exactly what these snapshots generate.
	#[error(
		"expansion SQL differs from the generated migration; edited SQL requires a separately \
		 reviewed rollout"
	)]
	EditedSql,
}

/// Check the structural expansion subset against the exact migration bytes.
///
/// This accepts new plain tables, nullable columns without defaults or
/// generated expressions, and comments. Existing objects cannot be removed,
/// renamed or constrained. Extension types, custom expressions and changes
/// to triggers, policies or views need a separate rollout. Pack membership
/// must be known and cannot shrink.
///
/// This is a contract for applications that name columns in reads and
/// writes. It does not prove that arbitrary SQL, positional `INSERT`,
/// `SELECT *`, triggers or application business logic tolerate a new shape.
/// PostgreSQL still takes DDL locks; the caller must bound those waits.
///
/// # Errors
/// Rejects unsupported shapes, unknown pack membership, or any SQL edit,
/// including an edit which leaves the snapshot unchanged.
pub fn verify_expansion(
	previous: &Schema,
	target: &Schema,
	sql: &str,
) -> Result<(), ExpansionError> {
	if previous.dialect != Dialect::Postgres || target.dialect != Dialect::Postgres {
		return Err(ExpansionError::Unsupported("PostgreSQL is required".into()));
	}
	if previous.format != crate::FORMAT_VERSION || target.format != crate::FORMAT_VERSION {
		return Err(ExpansionError::Unsupported(
			"current schema snapshot format is required".into(),
		));
	}
	let plan = crate::diff(previous, target, &mut RequireHints)
		.map_err(|error| ExpansionError::Schema(error.to_string()))?;
	let old_packs = previous
		.packs
		.as_ref()
		.ok_or_else(|| ExpansionError::Unsupported("baseline pack membership is unknown".into()))?;
	let new_packs = target
		.packs
		.as_ref()
		.ok_or_else(|| ExpansionError::Unsupported("target pack membership is unknown".into()))?;
	for old in old_packs {
		if !new_packs.iter().any(|new| {
			new.name == old.name
				&& new.builtin == old.builtin
				&& old.tables.iter().all(|table| new.tables.contains(table))
		}) {
			return Err(ExpansionError::Unsupported(format!(
				"schema pack {} lost membership",
				old.name
			)));
		}
	}
	for step in &plan.steps {
		let supported = match step {
			Step::AddColumn { column, .. } => column.nullable && plain(column),
			Step::CreateTable(table) => {
				table.columns.iter().all(plain)
					&& table.indexes.is_empty()
					&& table.foreign_keys.is_empty()
					&& table.checks.is_empty()
					&& table.partition.is_none()
					&& table.search.is_none()
					&& table.row_security.is_none()
					&& table.geo_indexes.is_empty()
					&& table.vectors.is_empty()
			}
			Step::SetTableComment { .. } | Step::SetColumnComment { .. } => true,
			_ => false,
		};
		if !supported {
			return Err(ExpansionError::Unsupported(step.to_string()));
		}
	}
	if crate::render(&plan, previous, target).sql() != sql {
		return Err(ExpansionError::EditedSql);
	}
	Ok(())
}

fn plain(column: &Column) -> bool {
	column.default.is_none()
		&& column.generated.is_none()
		&& column.renamed_from.is_none()
		&& builtin(&column.ty)
}

fn builtin(ty: &ColumnType) -> bool {
	match ty {
		ColumnType::Sequence
		| ColumnType::Geometry { .. }
		| ColumnType::Geography { .. }
		| ColumnType::Vector { .. } => false,
		ColumnType::Array { element } => builtin(element),
		ColumnType::Bool
		| ColumnType::Int16
		| ColumnType::Int32
		| ColumnType::Int64
		| ColumnType::Float32
		| ColumnType::Float64
		| ColumnType::Decimal { .. }
		| ColumnType::Text { .. }
		| ColumnType::Bytes
		| ColumnType::Uuid
		| ColumnType::Timestamp
		| ColumnType::Date
		| ColumnType::LocalTimestamp
		| ColumnType::LocalTime
		| ColumnType::Duration
		| ColumnType::Json => true,
	}
}
